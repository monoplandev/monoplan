//! Monoplan FFI — the Rust↔Swift boundary for the Apple clients.
//!
//! What crosses: the profile's lifecycle (`ensure_local_workspace`,
//! `login`, `signup` and `logout`: the local-only bootstrap and the
//! bodies of the CLI commands of the same names), `Engine`, the daemon
//! loop embedded in the app's process (`spec/daemon-plan.md`, amended
//! 2026-10-27), and the app's client path into it: `Engine::request`
//! and `Engine::watch`, the socket's request and watch frames as
//! function calls (`spec/mac-window-plan.md`, phase 1), and the mirror
//! for the window: `Engine::subscribe` (a snapshot, a leased peer id, and
//! updates streamed to a `MirrorSink`) and `Engine::import` (the
//! window's ops applied as local), phase 2 of the same plan. Replacing
//! `request` / `watch` (`spec/swift-bindings-plan.md`): typed calls on
//! `Engine`, one per `monoplan_cli::api` function and returning the
//! records it defines, typed versions of the requests that are not doc
//! commands (`sync`, `reconnect`, `account`, `workspaces`,
//! `create_workspace`, `delete_workspace`), and `Engine::observe`, the
//! watch as an `EngineObserver`. iOS uses nothing else; `request`,
//! `watch` and the mirror stay until macOS is over too. The app
//! is a client of its own engine here and never over the socket, which
//! stays the CLI's. The earlier offline `MonoplanStore`
//! prototype (`spec/swift-ffi-plan.md`) was removed on 2026-10-27: it
//! booted a second doc under a fake doc id with a caller-held DEK, and
//! nothing but its smoke test used it.
//!
//! Everything here is proc-macro / library mode uniffi (no `.udl`):
//! `setup_scaffolding!` plus `#[uniffi::export]` / `#[derive(uniffi::…)]`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

uniffi::setup_scaffolding!();

use monoplan_cli::api::{
    self,
    agenda::{AgendaGroup, AgendaQuery},
    bin::BinEntry,
    find::FindHit,
    items::{Item, ItemInfo, ItemState, Place},
    lists::{ListFilter, ListInfo},
    places::{LocatedItem, PlaceSuggestion},
    starred::StarredItem,
};

// ---------------------------------------------------------------------
// Engine: the daemon loop, embedded (spec/daemon-plan.md, amended
// 2026-10-27). On macOS the app runs the engine whenever it runs: it runs
// `daemon::start` / `daemon::Serving` on its own thread and its UI is
// an in-process client of that loop (`daemon::LocalClient`), the same
// loop the CLI reaches over the socket. The headless
// `monoplan daemon` stays for CLI-only, Linux and servers, and fills in
// on a Mac only while the app is not running: `start` takes a headless
// daemon over, and the CLI's auto-spawn starts one again if the app
// has quit.
// ---------------------------------------------------------------------

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum EngineError {
    /// No `secrets.toml` in the profile dir: nothing to run.
    #[error("not logged in")]
    NotLoggedIn,
    /// Something `start` will not take over holds `engine.lock`:
    /// another app's engine, an engine still starting, or a CLI command
    /// rewriting the profile (`client::claim`). Try again later.
    #[error("another process holds this profile")]
    Busy,
    /// The loop has ended (`shutdown`, or a client's `Shutdown`): no
    /// request is answered and no watch is served until the app
    /// starts a new engine.
    #[error("the engine has stopped")]
    Stopped,
    #[error("{message}")]
    Failed { message: String },
}

impl From<monoplan_cli::api::ApiError> for EngineError {
    fn from(e: monoplan_cli::api::ApiError) -> Self {
        use monoplan_cli::api::ApiError;
        match e {
            ApiError::Stopped => EngineError::Stopped,
            ApiError::NoSuchWorkspace { message } | ApiError::Failed { message } => {
                EngineError::Failed { message }
            }
        }
    }
}

/// Where the embedded engine's `tracing` output goes: `engine.log` in
/// the profile dir, the file the headless daemon's stderr is appended
/// to, so there is one place to look on a Mac. A GUI process has no
/// terminal, and the engine's connect failures (`warn`) would otherwise
/// vanish. `RUST_LOG` overrides the level. Set once per process; the
/// file is reopened per write, so it survives a sign-out purge and
/// comes back with the next profile. Its size is capped at each engine
/// start (`daemon::start_with`).
fn init_log(profile_dir: &std::path::Path) {
    use tracing_subscriber::{EnvFilter, fmt::writer::MakeWriter};

    struct Append(PathBuf);
    impl<'a> MakeWriter<'a> for Append {
        type Writer = Box<dyn std::io::Write + 'a>;
        fn make_writer(&'a self) -> Self::Writer {
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.0)
            {
                Ok(f) => Box::new(f),
                Err(_) => Box::new(std::io::sink()),
            }
        }
    }

    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_ansi(false)
        .with_writer(Append(profile_dir.join(monoplan_cli::daemon::LOG_FILE)))
        .try_init();
}

/// Whether the embedded engine serves the CLI's Unix socket. Every
/// platform with a terminal, except iOS: no CLI runs there, nothing
/// else can hold the profile, and the app container's paths are longer
/// than a socket path may be (`daemon::start_with`).
const SERVE_SOCKET: bool = !cfg!(target_os = "ios");

fn runtime() -> Result<tokio::runtime::Runtime, EngineError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| EngineError::Failed {
            message: e.to_string(),
        })
}

/// The profile at `profile_dir`, created as a local-only workspace if
/// there is none (`monoplan_cli::bootstrap`: a doc id and DEK minted
/// locally, the onboarding doc as the baseline, no device token). What
/// the first doc command does on a fresh CLI install, for the app to
/// do before it starts the engine: accountless use on a Mac is the web client's
/// anonymous session (`spec/auth.md` §Device pairing). Nothing to do
/// when a profile exists. Blocks briefly (the seed is written and the
/// profile held meanwhile), so call it off the main thread.
#[uniffi::export]
pub fn ensure_local_workspace(profile_dir: String) -> Result<(), EngineError> {
    use monoplan_cli::bootstrap::open_or_init_at;
    runtime()?
        .block_on(open_or_init_at(PathBuf::from(profile_dir)))
        .map(|_| ())
        .map_err(|e| EngineError::Failed {
            message: format!("{e:#}"),
        })
}

/// What the profile holds, for the app's menus and sheets: nothing,
/// a local-only workspace, or an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ProfileState {
    None,
    LocalOnly,
    Account,
}

#[uniffi::export]
pub fn profile_state(profile_dir: String) -> ProfileState {
    use monoplan_cli::config::Profile;
    match Profile::active_at(PathBuf::from(profile_dir)) {
        None => ProfileState::None,
        Some(p) => match p.has_account() {
            Ok(true) => ProfileState::Account,
            _ => ProfileState::LocalOnly,
        },
    }
}

/// What a signup gave back (`monoplan_cli::commands::signup::Outcome`).
#[derive(Debug, Clone, uniffi::Record)]
pub struct SignupOutcome {
    pub account_id: String,
    /// Shown once by the app; never persisted.
    pub recovery_code: String,
    /// The profile held a local-only workspace and the account now
    /// owns it: same doc id, same key, history intact.
    pub adopted: bool,
}

/// Create an account: what `monoplan auth signup` does once it has the
/// credentials, for the app's sheet. A local-only workspace in the
/// profile is adopted as the account's Home doc (`spec/auth.md` §Device
/// pairing). Blocks for the key derivation and the round trips, so
/// call it off the main thread. On success the profile is written and
/// released; the app's engine controller then starts an engine on it, and the first
/// connect pushes the local history.
#[uniffi::export]
pub fn signup(
    profile_dir: String,
    server: String,
    email: String,
    password: String,
    device_name: String,
) -> Result<SignupOutcome, EngineError> {
    use monoplan_cli::{
        commands::signup::{Credentials, signup},
        config::Profile,
    };
    use monoplan_protocol::KdfParams;
    let profile =
        Profile::create_at(PathBuf::from(profile_dir)).map_err(|e| EngineError::Failed {
            message: e.to_string(),
        })?;
    runtime()?
        .block_on(signup(
            &profile,
            Credentials {
                server,
                email,
                password,
                device_name,
                kdf_params: KdfParams::DEFAULT,
            },
        ))
        .map(|o| SignupOutcome {
            account_id: o.account_id,
            recovery_code: o.recovery_code,
            adopted: o.adopted,
        })
        .map_err(|e| EngineError::Failed {
            message: format!("{e:#}"),
        })
}

/// Sign in: what `monoplan auth login` does once it has the credentials,
/// for the app's sign-in sheet. Blocks for the key derivation and two
/// round trips, so call it off the main thread. On success the profile
/// is written and released; the app's engine controller then starts an engine on it.
/// A local-only workspace in the profile is replaced, as the CLI's
/// login does after its confirmation: the app asks first.
#[uniffi::export]
pub fn login(
    profile_dir: String,
    server: String,
    email: String,
    password: String,
    device_name: String,
) -> Result<(), EngineError> {
    use monoplan_cli::{
        commands::login::{Credentials, login},
        config::Profile,
    };
    let profile =
        Profile::create_at(PathBuf::from(profile_dir)).map_err(|e| EngineError::Failed {
            message: e.to_string(),
        })?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| EngineError::Failed {
            message: e.to_string(),
        })?;
    rt.block_on(login(
        &profile,
        Credentials {
            server,
            email,
            password,
            device_name,
        },
    ))
    .map(|_| ())
    .map_err(|e| EngineError::Failed {
        message: format!("{e:#}"),
    })
}

/// Whether `name` matches `query` the way the search index matches a
/// list's name (`spec/search.md`): every query token prefixes some
/// token of `name`, accents and case folded; an empty query matches
/// everything. For the labels a client owns (its built-in views), which
/// the index never sees.
#[uniffi::export]
pub fn matches_name(name: String, query: String) -> bool {
    monoplan_core::matches_name(&name, &query)
}

/// Sign out: stop whatever holds the profile, hold it, revoke this
/// device on the server, wipe it. What `monoplan auth logout` does, for
/// the app's Settings. The revoke is best-effort (an unreachable server
/// does not block the wipe) and its outcome is not surfaced here yet. The app stops its
/// own engine first and keeps its engine controller from restarting one
/// until this returns; `NotLoggedIn` when there was no profile.
#[uniffi::export]
pub fn logout(profile_dir: String) -> Result<(), EngineError> {
    use monoplan_cli::{commands::logout, config::Profile};
    let profile = Profile::active_at(PathBuf::from(profile_dir)).ok_or(EngineError::NotLoggedIn)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| EngineError::Failed {
            message: e.to_string(),
        })?;
    rt.block_on(logout::logout(&profile))
        .map(|_| ())
        .map_err(|e| EngineError::Failed {
            message: format!("{e:#}"),
        })
}

/// One running engine loop. Dropping it does *not* stop the loop; call
/// `shutdown` (clean exit: persist, fold, close the socket, release the
/// lock). A client's `Shutdown` request (`daemon stop`, `logout`,
/// `login`, `password`) ends the loop too, after which `is_running` is
/// false and the app starts a new one once the profile is free.
#[derive(uniffi::Object)]
pub struct Engine {
    shutdown: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// The in-process client path (`daemon::LocalClient`).
    client: monoplan_cli::daemon::LocalClient,
    /// The profile this engine runs, for `device_token`.
    profile_dir: PathBuf,
}

/// Where a watch's frames go: the app's object, called from the engine's
/// threads, so the app hops to its own queue inside. The first frame is
/// `Reply::Watching`, then one `Reply::Event` per event, each msgpack-
/// encoded exactly as the socket would carry it after the length prefix.
#[uniffi::export(with_foreign)]
pub trait EngineSink: Send + Sync {
    fn frame(&self, frame: Vec<u8>);
    /// The engine loop has ended. No frame follows.
    fn stopped(&self);
}

/// One live watch. Dropping it, or `cancel`, ends the forwarding; the
/// sink hears nothing more and no `stopped`.
#[derive(uniffi::Object)]
pub struct Watcher {
    cancelled: Arc<AtomicBool>,
}

#[uniffi::export]
impl Watcher {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The engine's sync state as the app shows it (`ipc::SyncStatus`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncState {
    /// Socket up and the doc open on it right now.
    pub online: bool,
    /// Unix millis of the last completed exchange with the server, on
    /// any run of the engine.
    pub last_synced_at: Option<i64>,
    /// Why the engine is offline, for a human, when it knows.
    pub offline_reason: Option<String>,
    /// No account: the engine never connects, so there is nothing to
    /// be offline from.
    pub local_only: bool,
    /// Local work the server has not confirmed yet.
    pub pending: bool,
}

impl From<monoplan_cli::ipc::SyncStatus> for SyncState {
    fn from(s: monoplan_cli::ipc::SyncStatus) -> Self {
        SyncState {
            online: s.online,
            last_synced_at: s.last_synced_at,
            offline_reason: s.offline_reason,
            local_only: s.local_only,
            pending: s.pending,
        }
    }
}

/// What a write does to the item's duration alongside its `when`
/// (`Engine::set_when`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DurationChange {
    /// Leave it to the core: a first timed `when` defaults it to 60
    /// minutes, and clearing `when` clears it.
    Keep,
    Clear,
    Minutes {
        minutes: u32,
    },
}

/// What a write does to the item's pending date suggestion alongside
/// its text (`Engine::set_text`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SuggestionChange {
    Keep,
    /// Drop it: the user's "no", or its phrase left the title.
    Clear,
    /// Raise one: a `when`-shaped value read out of the title.
    Set {
        when: String,
    },
}

/// Who is signed in on this device (`Engine::account`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AccountInfo {
    pub account_id: String,
    pub email: String,
    pub device_id: String,
    /// The server the engine syncs with.
    pub server_url: String,
}

/// One workspace the engine has open (`Engine::workspaces`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WorkspaceInfo {
    /// The doc's id: what a typed call's `workspace` takes, and what
    /// the rows of the reads that span workspaces are tagged with.
    pub doc_id: String,
    /// The doc's name; nil for Home and for an unnamed doc.
    pub name: Option<String>,
    /// The account's Home doc (the Personal workspace), or the one
    /// workspace of a local-only install.
    pub home: bool,
}

/// Where the engine's events go (`Engine::observe`): the app's object,
/// called from a thread of the engine's, so the app hops to its own
/// queue inside.
#[uniffi::export(with_foreign)]
pub trait EngineObserver: Send + Sync {
    /// A doc changed: a call or command committed, or remote ops were
    /// applied. Read again whatever is on screen.
    fn changed(&self);
    /// Online / offline flipped, pending work started or settled, or
    /// `last_synced_at` moved. Called once with the current state as
    /// `observe` starts.
    fn sync(&self, state: SyncState);
    /// The engine loop has ended. Nothing follows.
    fn stopped(&self);
}

/// Where a mirror's updates go (`spec/mac-window-plan.md` "Engine to
/// mirror"): the app's object, called on the engine loop's thread, so the
/// app hops to the webview's queue inside and never calls back into
/// `Engine` synchronously (the loop is busy delivering; an `import` from
/// inside `update` would wait on itself). Each `update` is Loro update
/// bytes since the previous one (or since the snapshot), to import into
/// the mirror doc tagged `remote`.
#[uniffi::export(with_foreign)]
pub trait MirrorSink: Send + Sync {
    fn update(&self, bytes: Vec<u8>);
    /// The engine loop has ended, or the doc was closed under the mirror.
    /// No update follows; subscribe again once there is an engine.
    fn stopped(&self);
}

/// Adapts the uniffi callback to the daemon's sink trait.
struct ForeignSink(Arc<dyn MirrorSink>);

impl monoplan_cli::daemon::MirrorSink for ForeignSink {
    fn update(&self, bytes: Vec<u8>) {
        self.0.update(bytes);
    }
    fn stopped(&self) {
        self.0.stopped();
    }
}

/// One live mirror of one doc (`spec/mac-window-plan.md`): the seed the
/// webview loads and the handle its ops go back through
/// (`Engine::import`). Dropping it, or `close`, ends the subscription and
/// releases the peer slot; the sink hears nothing more.
#[derive(uniffi::Object)]
pub struct Subscription {
    inner: Mutex<Option<monoplan_cli::daemon::Subscription>>,
    doc_id: String,
    peer_id: u64,
    snapshot: Vec<u8>,
}

#[uniffi::export]
impl Subscription {
    /// The doc being mirrored, as its UUID string.
    pub fn doc_id(&self) -> String {
        self.doc_id.clone()
    }

    /// The peer the mirror doc must write under for the life of this
    /// subscription (`Doc.loadWithPeer` on the wasm side).
    pub fn peer_id(&self) -> u64 {
        self.peer_id
    }

    /// Plaintext Loro snapshot of the doc at subscribe time; the sink's
    /// updates continue from exactly here.
    pub fn snapshot(&self) -> Vec<u8> {
        self.snapshot.clone()
    }

    /// End the subscription now rather than on release.
    pub fn close(&self) {
        self.inner.lock().unwrap().take();
    }
}

impl From<monoplan_cli::daemon::MirrorError> for EngineError {
    fn from(e: monoplan_cli::daemon::MirrorError) -> Self {
        use monoplan_cli::daemon::MirrorError;
        match e {
            MirrorError::Stopped => EngineError::Stopped,
            MirrorError::Failed(message) => EngineError::Failed { message },
        }
    }
}

/// The profile as the window should see it (`spec/mac-window-plan.md`
/// "Swift side", the `session` request): what `Engine::session` answers.
/// No credential: the device token stays with the app
/// (`Engine::device_token`) and never reaches the webview.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EngineSession {
    /// Empty for a local-only workspace, as are `email`, `device_id`
    /// and `server_url`.
    pub account_id: String,
    pub email: String,
    pub device_id: String,
    pub primary_doc_id: String,
    /// The server the engine syncs with: where the window's HTTP calls go.
    pub server_url: String,
    /// No account behind the profile: the window runs it as the web
    /// client runs an anonymous session (`spec/auth.md`).
    pub local_only: bool,
    /// Every doc the engine has open, Home first.
    pub docs: Vec<EngineDoc>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct EngineDoc {
    pub doc_id: String,
    /// The doc's name; nil for Home and for an unnamed doc.
    pub name: Option<String>,
}

fn unexpected(reply: monoplan_cli::ipc::Reply) -> EngineError {
    EngineError::Failed {
        message: format!("unexpected reply: {reply:?}"),
    }
}

impl Engine {
    /// One of the loop's own requests (not a doc command), with its
    /// failure as an `EngineError`.
    fn reply(
        &self,
        req: monoplan_cli::ipc::Request,
    ) -> Result<monoplan_cli::ipc::Reply, EngineError> {
        use monoplan_cli::ipc::Reply;
        if self.client.is_stopped() {
            return Err(EngineError::Stopped);
        }
        match self.client.request_blocking(req) {
            // The loop ended under the request.
            Reply::Err { .. } if self.client.is_stopped() => Err(EngineError::Stopped),
            Reply::Err { message } => Err(EngineError::Failed { message }),
            other => Ok(other),
        }
    }

    fn synced(&self, req: monoplan_cli::ipc::Request) -> Result<SyncState, EngineError> {
        match self.reply(req)? {
            monoplan_cli::ipc::Reply::Synced { sync } => Ok(sync.into()),
            other => Err(unexpected(other)),
        }
    }
}

/// The engine's build id (`ipc::build_id`): the crate version and the
/// binary's modification time, for the app's debug screen.
#[uniffi::export]
pub fn build_id() -> String {
    monoplan_cli::ipc::build_id()
}

fn encode(reply: &monoplan_cli::ipc::Reply) -> Vec<u8> {
    // `Reply` is plain data over serde; the only failure is a
    // msgpack bug, which the socket path would hit identically.
    monoplan_cli::ipc::encode_payload(reply).expect("reply encodes")
}

#[uniffi::export]
impl Engine {
    /// Claim the profile (stopping a headless daemon that holds it),
    /// bind the socket, boot the doc, and serve on a background thread.
    /// Returns once the socket is listening (or the start failed), so
    /// call it off the main thread: booting a large doc takes a moment.
    #[uniffi::constructor]
    pub fn start(profile_dir: String) -> Result<Arc<Engine>, EngineError> {
        use monoplan_cli::{client, config::Profile, daemon};
        let profile =
            Profile::active_at(PathBuf::from(profile_dir)).ok_or(EngineError::NotLoggedIn)?;
        init_log(&profile.dir);
        let profile_dir = profile.dir.clone();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        type Ready = Result<daemon::LocalClient, EngineError>;
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Ready>();
        let thread = std::thread::Builder::new()
            .name("monoplan-engine".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(EngineError::Failed {
                            message: e.to_string(),
                        }));
                        return;
                    }
                };
                rt.block_on(async move {
                    // On iOS nothing else can hold the profile (no CLI,
                    // no headless daemon, one process per container),
                    // and the engine serves no socket: the app is its
                    // only client, and the container paths are longer
                    // than `sun_path` allows anyway.
                    if SERVE_SOCKET {
                        match client::take_over(&profile.dir).await {
                            Ok(true) => {}
                            Ok(false) => {
                                let _ = ready_tx.send(Err(EngineError::Busy));
                                return;
                            }
                            Err(e) => {
                                let _ = ready_tx.send(Err(EngineError::Failed {
                                    message: format!("{e:#}"),
                                }));
                                return;
                            }
                        }
                    }
                    let started = match daemon::start_with(profile, SERVE_SOCKET).await {
                        Ok(Some(s)) => s,
                        // Lost the lock to someone between the takeover
                        // and the claim; the app tries again later.
                        Ok(None) => {
                            let _ = ready_tx.send(Err(EngineError::Busy));
                            return;
                        }
                        Err(e) => {
                            let _ = ready_tx.send(Err(EngineError::Failed {
                                message: format!("{e:#}"),
                            }));
                            return;
                        }
                    };
                    let (serving, client) = daemon::Serving::new(started, true);
                    let _ = ready_tx.send(Ok(client));
                    if let Err(e) = serving.run(shutdown_rx).await {
                        eprintln!("monoplan engine: {e:#}");
                    }
                });
            })
            .map_err(|e| EngineError::Failed {
                message: e.to_string(),
            })?;
        match ready_rx.recv() {
            Ok(Ok(client)) => Ok(Arc::new(Engine {
                shutdown: Mutex::new(Some(shutdown_tx)),
                thread: Mutex::new(Some(thread)),
                client,
                profile_dir,
            })),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(EngineError::Failed {
                message: "engine thread exited before reporting".into(),
            }),
        }
    }

    /// Clean exit; blocks until the loop has released the profile.
    pub fn shutdown(&self) {
        if let Some(tx) = self.shutdown.lock().unwrap().take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.lock().unwrap().take() {
            let _ = t.join();
        }
    }

    /// One IPC request as a function call: a msgpack `Request` in, the
    /// msgpack `Reply` out, the bytes the socket carries after its
    /// length prefix. Blocks until the loop has answered (a doc command
    /// runs inline there), so call it off the main thread. `Hello` is
    /// answered without the loop; `Watch` is `watch` below.
    pub fn request(&self, frame: Vec<u8>) -> Result<Vec<u8>, EngineError> {
        use monoplan_cli::ipc::{Request, decode_payload};
        if self.client.is_stopped() {
            return Err(EngineError::Stopped);
        }
        let req: Request = decode_payload(&frame).map_err(|e| EngineError::Failed {
            message: e.to_string(),
        })?;
        Ok(encode(&self.client.request_blocking(req)))
    }

    /// Subscribe to the loop's watch stream (`spec/daemon-plan.md`
    /// projection tier): `sink` gets `Watching` now, then one `Event`
    /// frame per event, each msgpack-encoded exactly as the socket
    /// would carry it after the length prefix, and `stopped` once when
    /// the loop ends.
    ///
    /// The forwarder is a plain thread, not a task on the engine's
    /// runtime: that runtime is dropped as soon as the loop returns,
    /// which cancels its tasks, and a forwarder cancelled between the
    /// channel closing and its next poll would never deliver the one
    /// event the app's client depends on to notice the engine is gone
    /// (a sign-in's claim or a terminal command ending it). A blocking
    /// receive sees `Closed` whatever happened to the runtime.
    pub fn watch(&self, sink: Arc<dyn EngineSink>) -> Result<Arc<Watcher>, EngineError> {
        use monoplan_cli::ipc::{Reply, WatchEvent};
        use tokio::sync::broadcast::error::RecvError;
        if self.client.is_stopped() {
            return Err(EngineError::Stopped);
        }
        let (sync, mut rx) = self.client.watch();
        sink.frame(encode(&Reply::Watching { sync }));
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        std::thread::Builder::new()
            .name("monoplan-watch".into())
            .spawn(move || {
                loop {
                    let ev = match rx.blocking_recv() {
                        Ok(ev) => ev,
                        // Behind by more than the buffer: the backlog
                        // collapses to "something changed", as on the socket.
                        Err(RecvError::Lagged(_)) => WatchEvent::Changed,
                        Err(RecvError::Closed) => {
                            if !flag.load(Ordering::SeqCst) {
                                sink.stopped();
                            }
                            return;
                        }
                    };
                    // A cancelled watch stays subscribed until the next
                    // event or the loop's end, and says nothing.
                    if flag.load(Ordering::SeqCst) {
                        return;
                    }
                    sink.frame(encode(&Reply::Event(ev)));
                }
            })
            .map_err(|e| EngineError::Failed {
                message: e.to_string(),
            })?;
        Ok(Arc::new(Watcher { cancelled }))
    }

    /// [`watch`](Self::watch) without the frames
    /// (`spec/swift-bindings-plan.md`): `observer.sync` now with the
    /// current state, then `changed` / `sync` per event and `stopped`
    /// once when the loop ends. The same forwarder thread, for the
    /// same reason.
    pub fn observe(&self, observer: Arc<dyn EngineObserver>) -> Result<Arc<Watcher>, EngineError> {
        use monoplan_cli::ipc::WatchEvent;
        use tokio::sync::broadcast::error::RecvError;
        if self.client.is_stopped() {
            return Err(EngineError::Stopped);
        }
        let (sync, mut rx) = self.client.watch();
        observer.sync(sync.into());
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        std::thread::Builder::new()
            .name("monoplan-observe".into())
            .spawn(move || {
                loop {
                    let ev = match rx.blocking_recv() {
                        Ok(ev) => ev,
                        Err(RecvError::Lagged(_)) => WatchEvent::Changed,
                        Err(RecvError::Closed) => {
                            if !flag.load(Ordering::SeqCst) {
                                observer.stopped();
                            }
                            return;
                        }
                    };
                    if flag.load(Ordering::SeqCst) {
                        return;
                    }
                    match ev {
                        WatchEvent::Changed => observer.changed(),
                        WatchEvent::Sync(s) => observer.sync(s.into()),
                    }
                }
            })
            .map_err(|e| EngineError::Failed {
                message: e.to_string(),
            })?;
        Ok(Arc::new(Watcher { cancelled }))
    }

    // ----- typed calls (`monoplan_cli::api`) -----
    //
    // Each runs on the engine loop against the doc `workspace` names
    // (a `-w` selector: name or doc id; nil is Home) and blocks for
    // its answer, so call them off the main thread. A write is durable
    // when it returns, and `EngineObserver::changed` follows.

    /// The workspace's lists, in order, each with its open count.
    pub fn lists(
        &self,
        workspace: Option<String>,
        filter: ListFilter,
    ) -> Result<Vec<ListInfo>, EngineError> {
        Ok(self
            .client
            .call_blocking(workspace, move |cx| api::lists::lists(cx, filter))?)
    }

    /// The items of `list` in order, binned ones left out; closed ones
    /// only with `include_done`.
    pub fn items(
        &self,
        workspace: Option<String>,
        list: String,
        include_done: bool,
    ) -> Result<Vec<Item>, EngineError> {
        Ok(self.client.call_blocking(workspace, move |cx| {
            api::items::items(cx, &list, include_done)
        })?)
    }

    /// Add an item to `list` (at the top with `top`, else the end) and
    /// return its id.
    pub fn add_item(
        &self,
        workspace: Option<String>,
        list: String,
        text: String,
        top: bool,
    ) -> Result<String, EngineError> {
        Ok(self.client.call_blocking(workspace, move |cx| {
            let mut ids = api::items::add_items(cx, &list, &[text], top)?;
            Ok(ids.remove(0))
        })?)
    }

    /// Tick or untick an item: `Done`, or back to `Backlog`.
    pub fn set_done(
        &self,
        workspace: Option<String>,
        id: String,
        done: bool,
    ) -> Result<(), EngineError> {
        Ok(self
            .client
            .call_blocking(workspace, move |cx| api::items::set_done(cx, &id, done))?)
    }

    /// One item, whatever its state, with its notes, its list's name
    /// and emoji and whether it is starred: an item's page.
    pub fn item(&self, workspace: Option<String>, id: String) -> Result<ItemInfo, EngineError> {
        Ok(self
            .client
            .call_blocking(workspace, move |cx| api::items::item(cx, &id))?)
    }

    /// Move an item to any of the six workflow states.
    pub fn set_state(
        &self,
        workspace: Option<String>,
        id: String,
        state: ItemState,
    ) -> Result<(), EngineError> {
        Ok(self
            .client
            .call_blocking(workspace, move |cx| api::items::set_state(cx, &id, state))?)
    }

    /// Retitle an item, and in the same call raise or drop the date
    /// suggestion the new title brings.
    pub fn set_text(
        &self,
        workspace: Option<String>,
        id: String,
        text: String,
        suggestion: SuggestionChange,
    ) -> Result<(), EngineError> {
        Ok(self.client.call_blocking(workspace, move |cx| {
            api::items::set_text(cx, &id, &text)?;
            match &suggestion {
                SuggestionChange::Keep => Ok(()),
                SuggestionChange::Clear => api::items::set_when_suggested(cx, &id, None),
                SuggestionChange::Set { when } => {
                    api::items::set_when_suggested(cx, &id, Some(when))
                }
            }
        })?)
    }

    /// Raise a date suggestion (`when`-shaped) on an item, or dismiss
    /// the pending one (nil). The yes is `set_when` with the suggested
    /// value: any date set clears the suggestion.
    pub fn set_when_suggested(
        &self,
        workspace: Option<String>,
        id: String,
        when: Option<String>,
    ) -> Result<(), EngineError> {
        Ok(self.client.call_blocking(workspace, move |cx| {
            api::items::set_when_suggested(cx, &id, when.as_deref())
        })?)
    }

    /// Set an item's planned date (`YYYY-MM-DD`, or `YYYY-MM-DDTHH:MM`
    /// floating) or clear it (nil), then apply `duration`.
    pub fn set_when(
        &self,
        workspace: Option<String>,
        id: String,
        when: Option<String>,
        duration: DurationChange,
    ) -> Result<(), EngineError> {
        Ok(self.client.call_blocking(workspace, move |cx| {
            api::items::set_when(cx, &id, when.as_deref())?;
            match duration {
                DurationChange::Keep => Ok(()),
                DurationChange::Clear => api::items::set_duration(cx, &id, None),
                DurationChange::Minutes { minutes } => {
                    api::items::set_duration(cx, &id, Some(minutes))
                }
            }
        })?)
    }

    /// Set an item's duration in whole minutes, or clear it (nil).
    pub fn set_duration(
        &self,
        workspace: Option<String>,
        id: String,
        minutes: Option<u32>,
    ) -> Result<(), EngineError> {
        Ok(self.client.call_blocking(workspace, move |cx| {
            api::items::set_duration(cx, &id, minutes)
        })?)
    }

    /// Set an item's deadline (`YYYY-MM-DD`), or clear it (nil).
    pub fn set_deadline(
        &self,
        workspace: Option<String>,
        id: String,
        deadline: Option<String>,
    ) -> Result<(), EngineError> {
        Ok(self.client.call_blocking(workspace, move |cx| {
            api::items::set_deadline(cx, &id, deadline.as_deref())
        })?)
    }

    /// Set an item's place, or clear it (nil). A place with
    /// coordinates or an address is written as given; a bare label
    /// takes a known place of that label whole unless `exact`.
    pub fn set_place(
        &self,
        workspace: Option<String>,
        id: String,
        place: Option<Place>,
        exact: bool,
    ) -> Result<(), EngineError> {
        Ok(self.client.call_blocking(workspace, move |cx| {
            api::items::set_place(cx, &id, place, exact)
        })?)
    }

    /// Move an item to another list of its workspace, at the end.
    pub fn move_item(
        &self,
        workspace: Option<String>,
        id: String,
        list: String,
    ) -> Result<(), EngineError> {
        Ok(self.client.call_blocking(workspace, move |cx| {
            api::items::move_item(cx, &id, Some(&list), None)
        })?)
    }

    /// Reorder an item within its own list, to the 1-based slot it
    /// ends up in among the list's open items.
    pub fn reorder_item(
        &self,
        workspace: Option<String>,
        id: String,
        pos: u32,
    ) -> Result<(), EngineError> {
        Ok(self.client.call_blocking(workspace, move |cx| {
            api::items::move_item(cx, &id, None, Some(pos as usize))
        })?)
    }

    /// Star an item (at the top of Starred) or unstar it. Starred is
    /// Home's, whatever `workspace` the item is in.
    pub fn set_starred(
        &self,
        workspace: Option<String>,
        id: String,
        starred: bool,
    ) -> Result<(), EngineError> {
        Ok(self.client.call_blocking(workspace, move |cx| {
            if starred {
                api::items::star(cx, &id, 0)
            } else {
                api::items::unstar(cx, &id)
            }
        })?)
    }

    /// Send an item to the bin, or restore it; its workflow state is
    /// kept under the mask.
    pub fn set_binned(
        &self,
        workspace: Option<String>,
        id: String,
        binned: bool,
    ) -> Result<(), EngineError> {
        Ok(self
            .client
            .call_blocking(workspace, move |cx| api::items::set_binned(cx, &id, binned))?)
    }

    /// Copy an item beside itself in its own list and return the
    /// copy's id. `star` also stars the copy of a starred item,
    /// directly below it.
    pub fn duplicate_item(
        &self,
        workspace: Option<String>,
        id: String,
        star: bool,
    ) -> Result<String, EngineError> {
        Ok(self
            .client
            .call_blocking(workspace, move |cx| api::items::duplicate(cx, &id, star))?)
    }

    /// Add a list at the end of the workspace's lists; its id.
    pub fn add_list(&self, workspace: Option<String>, name: String) -> Result<String, EngineError> {
        Ok(self
            .client
            .call_blocking(workspace, move |cx| api::lists::add_list(cx, &name))?)
    }

    /// Rename a user-created list.
    pub fn rename_list(
        &self,
        workspace: Option<String>,
        list: String,
        name: String,
    ) -> Result<(), EngineError> {
        Ok(self.client.call_blocking(workspace, move |cx| {
            api::lists::rename_list(cx, &list, &name)
        })?)
    }

    /// Set a user-created list's emoji, or clear it (nil).
    pub fn set_list_icon(
        &self,
        workspace: Option<String>,
        list: String,
        icon: Option<String>,
    ) -> Result<(), EngineError> {
        Ok(self.client.call_blocking(workspace, move |cx| {
            api::lists::set_list_icon(cx, &list, icon.as_deref())
        })?)
    }

    /// Archive a user-created list (its items are kept), or restore it.
    pub fn set_list_archived(
        &self,
        workspace: Option<String>,
        list: String,
        archived: bool,
    ) -> Result<(), EngineError> {
        Ok(self.client.call_blocking(workspace, move |cx| {
            api::lists::set_list_archived(cx, &list, archived)
        })?)
    }

    // The reads below span every open workspace: they take none, and
    // their rows carry the workspace each came from.

    /// The agenda over every workspace (minus those hidden from the
    /// calendar): Today first, always there; the Unscheduled group
    /// after it when there is one; then each non-empty day up to
    /// `days` past today.
    pub fn agenda(&self, days: u32) -> Result<Vec<AgendaGroup>, EngineError> {
        Ok(self.client.call_blocking(None, move |cx| {
            api::agenda::agenda(
                cx,
                &AgendaQuery {
                    days,
                    today: None,
                    everywhere: true,
                },
            )
        })?)
    }

    /// [`agenda`](Self::agenda) as the JSON `monoplan agenda --all
    /// --json` prints: the file format of the home-screen widget's
    /// snapshot, which the app leaves for it and never parses.
    pub fn agenda_json(&self, days: u32) -> Result<String, EngineError> {
        Ok(self.client.call_blocking(None, move |cx| {
            api::agenda::agenda_json(
                cx,
                &AgendaQuery {
                    days,
                    today: None,
                    everywhere: true,
                },
            )
        })?)
    }

    /// Starred in full: Home's curated order, each cross-workspace
    /// entry resolved, rows tagged with their workspace and list.
    pub fn starred(&self) -> Result<Vec<StarredItem>, EngineError> {
        Ok(self
            .client
            .call_blocking(None, api::starred::starred_everywhere)?)
    }

    /// Reorder a starred item to directly under `after`, another
    /// starred item, or to the top (nil). A neighbour, not a position:
    /// a position counts entries the resolved lens does not show.
    pub fn move_starred(&self, id: String, after: Option<String>) -> Result<(), EngineError> {
        Ok(self.client.call_blocking(None, move |cx| {
            api::starred::move_starred(cx, &id, None, after.as_deref())
        })?)
    }

    /// Every workspace's binned items, newest binned first.
    pub fn binned(&self) -> Result<Vec<BinEntry>, EngineError> {
        Ok(self
            .client
            .call_blocking(None, api::bin::binned_everywhere)?)
    }

    /// Search every workspace: lists above items, each by relevance,
    /// at most `limit`. An empty query finds nothing.
    pub fn find(&self, query: String, limit: u32) -> Result<Vec<FindHit>, EngineError> {
        Ok(self
            .client
            .call_blocking(None, move |cx| api::find::find(cx, &query, limit as usize))?)
    }

    /// The account's known places matching `query` (all when empty),
    /// most recently used first; at most `limit`, 0 being no limit.
    pub fn places(&self, query: String, limit: u32) -> Result<Vec<PlaceSuggestion>, EngineError> {
        Ok(self.client.call_blocking(None, move |cx| {
            api::places::places(cx, &query, limit as usize)
        })?)
    }

    /// The Open items that carry a place, every workspace.
    pub fn placed_items(&self) -> Result<Vec<LocatedItem>, EngineError> {
        Ok(self
            .client
            .call_blocking(None, |cx| api::places::placed_items(cx, "", 0))?)
    }

    // Back to one workspace.

    /// The workspace's newest `limit` Done items across every list,
    /// newest first by the moment each entered Done. A full `limit`
    /// back means there may be more.
    pub fn done(&self, workspace: Option<String>, limit: u32) -> Result<Vec<Item>, EngineError> {
        Ok(self.client.call_blocking(workspace, move |cx| {
            api::done::done(cx, None, Some(limit as usize))
        })?)
    }

    /// How many Done items the workspace holds in all.
    pub fn done_count(&self, workspace: Option<String>) -> Result<u32, EngineError> {
        Ok(self
            .client
            .call_blocking(workspace, |cx| api::done::done_count(cx, None))?)
    }

    /// Hard-delete one binned item. One not in the bin is refused.
    pub fn delete_binned(&self, workspace: Option<String>, id: String) -> Result<(), EngineError> {
        Ok(self
            .client
            .call_blocking(workspace, move |cx| api::bin::delete_binned(cx, &id))?)
    }

    /// Hard-delete every binned item of the workspace; how many went.
    pub fn empty_bin(&self, workspace: Option<String>) -> Result<u32, EngineError> {
        Ok(self.client.call_blocking(workspace, api::bin::empty_bin)?)
    }

    // ----- the engine itself -----

    /// Push now and report whether the server answers. Waits out a
    /// connect that is under way; a local-only workspace answers at
    /// once.
    pub fn sync(&self) -> Result<SyncState, EngineError> {
        self.synced(monoplan_cli::ipc::Request::Sync)
    }

    /// The network changed under the engine (path change, wake): drop
    /// the socket and connect again now. Answers with the state as of
    /// the drop; `EngineObserver::sync` reports the reconnect.
    pub fn reconnect(&self) -> Result<SyncState, EngineError> {
        self.synced(monoplan_cli::ipc::Request::Reconnect)
    }

    /// Who is signed in; nil for a local-only workspace.
    pub fn account(&self) -> Result<Option<AccountInfo>, EngineError> {
        let info = self.client.session_blocking()?;
        Ok((!info.local_only).then_some(AccountInfo {
            account_id: info.account_id,
            email: info.email,
            device_id: info.device_id,
            server_url: info.server_url,
        }))
    }

    /// Every workspace the engine has open, Home first.
    pub fn workspaces(&self) -> Result<Vec<WorkspaceInfo>, EngineError> {
        let info = self.client.session_blocking()?;
        Ok(info
            .docs
            .into_iter()
            .map(|d| WorkspaceInfo {
                home: d.doc_id == info.primary_doc_id,
                doc_id: d.doc_id.0.to_string(),
                name: d.name,
            })
            .collect())
    }

    /// Create a workspace the account owns, by name, and open it; its
    /// doc id. Needs an account.
    pub fn create_workspace(&self, name: String) -> Result<String, EngineError> {
        use monoplan_cli::ipc::{Reply, Request};
        match self.reply(Request::CreateWorkspace { name })? {
            Reply::WorkspaceCreated { doc_id, .. } => Ok(doc_id),
            other => Err(unexpected(other)),
        }
    }

    /// Delete a workspace the account owns, on the server and on every
    /// device. The server refuses one that is not ours or still has
    /// other members; Home is never deleted.
    pub fn delete_workspace(&self, doc_id: String) -> Result<(), EngineError> {
        use monoplan_cli::ipc::{Reply, Request};
        match self.reply(Request::DeleteWorkspace { workspace: doc_id })? {
            Reply::WorkspaceDeleted { .. } => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    /// Mirror a doc in the window (`spec/mac-window-plan.md`, phase 2):
    /// `workspace` is a `-w` selector or nil for Home. The returned
    /// subscription carries the snapshot and the leased peer id; from
    /// then on `sink.update` gets every change the engine applies (a CLI
    /// command, a remote frame, another window's import) until the
    /// subscription is closed or released. Blocks for the snapshot
    /// export, so call it off the main thread.
    pub fn subscribe(
        &self,
        workspace: Option<String>,
        sink: Arc<dyn MirrorSink>,
    ) -> Result<Arc<Subscription>, EngineError> {
        if self.client.is_stopped() {
            return Err(EngineError::Stopped);
        }
        let sub = self
            .client
            .subscribe_blocking(workspace, Box::new(ForeignSink(sink)))?;
        Ok(Arc::new(Subscription {
            doc_id: sub.doc_id.0.to_string(),
            peer_id: sub.peer_id,
            snapshot: sub.snapshot.clone(),
            inner: Mutex::new(Some(sub)),
        }))
    }

    /// Apply the window's ops (Loro update bytes since its last send)
    /// to the engine's doc: local from here, WALed on the next capture,
    /// pushed, and fanned out to any other mirror. Blocks until they
    /// are durable, so call it off the main thread. `Failed` after
    /// `close`, or for bytes Loro will not import.
    pub fn import(
        &self,
        subscription: Arc<Subscription>,
        bytes: Vec<u8>,
    ) -> Result<(), EngineError> {
        // Held for the call, so a concurrent `close` waits for the
        // import to land rather than racing it.
        let guard = subscription.inner.lock().unwrap();
        let Some(sub) = guard.as_ref() else {
            return Err(EngineError::Failed {
                message: "subscription closed".into(),
            });
        };
        Ok(sub.import_blocking(bytes)?)
    }

    /// The profile as the window should see it (`spec/mac-window-plan.md`
    /// "Swift side"): identity, server, and the open docs, Home first.
    /// Blocks for the loop's turn, so call it off the main thread.
    /// A local-only workspace answers with `local_only` set and empty
    /// identity fields.
    pub fn session(&self) -> Result<EngineSession, EngineError> {
        if self.client.is_stopped() {
            return Err(EngineError::Stopped);
        }
        let info = self.client.session_blocking()?;
        Ok(EngineSession {
            account_id: info.account_id,
            email: info.email,
            device_id: info.device_id,
            primary_doc_id: info.primary_doc_id.0.to_string(),
            server_url: info.server_url,
            local_only: info.local_only,
            docs: info
                .docs
                .into_iter()
                .map(|d| EngineDoc {
                    doc_id: d.doc_id.0.to_string(),
                    name: d.name,
                })
                .collect(),
        })
    }

    /// This device's server token, for the app to put on the window's
    /// HTTP calls (`Authorization: Bearer`, `spec/auth.md`). Read from
    /// the profile on each call; nil for a local-only workspace. The
    /// webview never sees it: the app's scheme handler adds it on the
    /// way out.
    pub fn device_token(&self) -> Option<String> {
        use monoplan_cli::config::Profile;
        Profile::active_at(self.profile_dir.clone())?
            .read_secrets()
            .ok()?
            .device_token
    }

    /// Still serving? False once the loop has ended (a client's
    /// `Shutdown`, a command's `claim`), from the moment it ends: the
    /// thread is still unwinding then, and a request to it would only
    /// get `Stopped`, so the app must not hand this engine to a new page
    /// or client meanwhile.
    pub fn is_running(&self) -> bool {
        !self.client.is_stopped()
            && self
                .thread
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|t| !t.is_finished())
    }
}
