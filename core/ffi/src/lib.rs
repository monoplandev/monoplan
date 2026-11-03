//! Monoplan FFI — the Rust↔Swift boundary for the Apple clients.
//!
//! What crosses: the profile's lifecycle (`login` and `logout`, the
//! bodies of the CLI commands of the same names), `Host`, the daemon
//! loop embedded in the app's process (`spec/daemon-plan.md`, amended
//! 2026-10-27), and the app's client path into it: `Host::request`
//! and `Host::watch`, the socket's request and watch frames as
//! function calls (`spec/mac-window-plan.md`, phase 1), and the mirror
//! for the window: `Host::subscribe` (a snapshot, a leased peer id, and
//! updates streamed to a `MirrorSink`) and `Host::import` (the
//! window's ops applied as local), phase 2 of the same plan. The app
//! is a client of its own host here and never over the socket, which
//! stays the CLI's. The earlier offline `MonoplanStore`
//! prototype (`spec/swift-ffi-plan.md`) was removed on 2026-10-27: it
//! booted a second doc under a fake doc id with a caller-held DEK, and
//! nothing but its smoke test used it.
//!
//! Everything here is proc-macro / library mode uniffi (no `.udl`):
//! `setup_scaffolding!` plus `#[uniffi::export]` / `#[derive(uniffi::…)]`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

uniffi::setup_scaffolding!();

// ---------------------------------------------------------------------
// Host: the daemon loop, embedded (spec/daemon-plan.md, amended
// 2026-10-27). On macOS the app is the host whenever it runs: it runs
// `daemon::start` / `daemon::Serving` on its own thread and its UI is
// an in-process client of that loop (`daemon::LocalClient`), the same
// loop the CLI reaches over the socket. The headless
// `monoplan daemon` stays for CLI-only, Linux and servers, and fills in
// on a Mac only while the app is not running: `start` takes a headless
// daemon over, and the CLI's auto-spawn starts one again if the app
// has quit.
// ---------------------------------------------------------------------

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum HostError {
    /// No `secrets.toml` in the profile dir: nothing to host.
    #[error("not logged in")]
    NotLoggedIn,
    /// Something `start` will not take over holds `daemon.lock`:
    /// another app's host, a host still starting, or a CLI command
    /// rewriting the profile (`client::claim`). Try again later.
    #[error("another process holds this profile")]
    Busy,
    /// The loop has ended (`shutdown`, or a client's `Shutdown`): no
    /// request is answered and no watch is served until the app
    /// starts a new host.
    #[error("the host has stopped")]
    Stopped,
    #[error("{message}")]
    Failed { message: String },
}

/// Where the embedded host's `tracing` output goes: `daemon.log` in
/// the profile dir, the file the headless daemon's stderr is appended
/// to, so there is one place to look on a Mac. A GUI process has no
/// terminal, and the host's connect failures (`warn`) would otherwise
/// vanish. `RUST_LOG` overrides the level. Set once per process; the
/// file is reopened per write, so it survives a sign-out purge and
/// comes back with the next profile.
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
        .with_writer(Append(profile_dir.join("daemon.log")))
        .try_init();
}

/// Sign in: what `monoplan auth login` does once it has the credentials,
/// for the app's sign-in sheet. Blocks for the key derivation and two
/// round trips, so call it off the main thread. On success the profile
/// is written and released; the app's host controller then hosts it.
#[uniffi::export]
pub fn login(
    profile_dir: String,
    server: String,
    email: String,
    password: String,
    device_name: String,
) -> Result<(), HostError> {
    use monoplan_cli::{
        commands::login::{Credentials, login},
        config::Profile,
    };
    let profile =
        Profile::create_at(PathBuf::from(profile_dir)).map_err(|e| HostError::Failed {
            message: e.to_string(),
        })?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| HostError::Failed {
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
    .map_err(|e| HostError::Failed {
        message: format!("{e:#}"),
    })
}

/// Sign out: stop whatever hosts the profile, hold it, wipe it. What
/// `monoplan auth logout` does, for the app's Settings. The app stops its
/// own host first and keeps its host controller from restarting one
/// until this returns; `NotLoggedIn` when there was no profile.
#[uniffi::export]
pub fn logout(profile_dir: String) -> Result<(), HostError> {
    use monoplan_cli::{commands::logout, config::Profile};
    let profile = Profile::active_at(PathBuf::from(profile_dir)).ok_or(HostError::NotLoggedIn)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| HostError::Failed {
            message: e.to_string(),
        })?;
    rt.block_on(logout::logout(&profile))
        .map_err(|e| HostError::Failed {
            message: format!("{e:#}"),
        })
}

/// One running host loop. Dropping it does *not* stop the loop; call
/// `shutdown` (clean exit: persist, fold, close the socket, release the
/// lock). A client's `Shutdown` request (`daemon stop`, `logout`,
/// `login`, `password`) ends the loop too, after which `is_running` is
/// false and the app starts a new one once the profile is free.
#[derive(uniffi::Object)]
pub struct Host {
    shutdown: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// The in-process client path (`daemon::LocalClient`).
    client: monoplan_cli::daemon::LocalClient,
    /// The host thread's runtime, for the watch forwarding tasks.
    rt: tokio::runtime::Handle,
    /// The profile being hosted, for `device_token`.
    profile_dir: PathBuf,
}

/// Where a watch's frames go: the app's object, called from the host's
/// threads, so the app hops to its own queue inside. The first frame is
/// `Reply::Watching`, then one `Reply::Event` per event, each msgpack-
/// encoded exactly as the socket would carry it after the length prefix.
#[uniffi::export(with_foreign)]
pub trait HostSink: Send + Sync {
    fn frame(&self, frame: Vec<u8>);
    /// The host loop has ended. No frame follows.
    fn stopped(&self);
}

/// One live watch. Dropping it, or `cancel`, ends the forwarding; the
/// sink hears nothing more and no `stopped`.
#[derive(uniffi::Object)]
pub struct Watcher {
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

#[uniffi::export]
impl Watcher {
    pub fn cancel(&self) {
        if let Some(t) = self.task.lock().unwrap().take() {
            t.abort();
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Where a mirror's updates go (`spec/mac-window-plan.md` "Host to
/// mirror"): the app's object, called on the host loop's thread, so the
/// app hops to the webview's queue inside and never calls back into
/// `Host` synchronously (the loop is busy delivering; an `import` from
/// inside `update` would wait on itself). Each `update` is Loro update
/// bytes since the previous one (or since the snapshot), to import into
/// the mirror doc tagged `remote`.
#[uniffi::export(with_foreign)]
pub trait MirrorSink: Send + Sync {
    fn update(&self, bytes: Vec<u8>);
    /// The host loop has ended, or the doc was closed under the mirror.
    /// No update follows; subscribe again once there is a host.
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
/// (`Host::import`). Dropping it, or `close`, ends the subscription and
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

impl From<monoplan_cli::daemon::MirrorError> for HostError {
    fn from(e: monoplan_cli::daemon::MirrorError) -> Self {
        use monoplan_cli::daemon::MirrorError;
        match e {
            MirrorError::Stopped => HostError::Stopped,
            MirrorError::Failed(message) => HostError::Failed { message },
        }
    }
}

/// The profile as the window should see it (`spec/mac-window-plan.md`
/// "Swift side", the `session` request): what `Host::session` answers.
/// No credential: the device token stays with the app
/// (`Host::device_token`) and never reaches the webview.
#[derive(Debug, Clone, uniffi::Record)]
pub struct HostSession {
    pub account_id: String,
    pub email: String,
    pub device_id: String,
    pub primary_doc_id: String,
    /// The server the host syncs with: where the window's HTTP calls go.
    pub server_url: String,
    /// Every doc the host has open, Home first.
    pub docs: Vec<HostDoc>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct HostDoc {
    pub doc_id: String,
    /// The doc's name; nil for Home and for an unnamed doc.
    pub name: Option<String>,
}

fn encode(reply: &monoplan_cli::ipc::Reply) -> Vec<u8> {
    // `Reply` is plain data over serde; the only failure is a
    // msgpack bug, which the socket path would hit identically.
    monoplan_cli::ipc::encode_payload(reply).expect("reply encodes")
}

#[uniffi::export]
impl Host {
    /// Claim the profile (stopping a headless daemon that holds it),
    /// bind the socket, boot the doc, and serve on a background thread.
    /// Returns once the socket is listening (or the start failed), so
    /// call it off the main thread: booting a large doc takes a moment.
    #[uniffi::constructor]
    pub fn start(profile_dir: String) -> Result<Arc<Host>, HostError> {
        use monoplan_cli::{client, config::Profile, daemon};
        let profile =
            Profile::active_at(PathBuf::from(profile_dir)).ok_or(HostError::NotLoggedIn)?;
        init_log(&profile.dir);
        let profile_dir = profile.dir.clone();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        type Ready = Result<(daemon::LocalClient, tokio::runtime::Handle), HostError>;
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Ready>();
        let thread = std::thread::Builder::new()
            .name("monoplan-host".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(HostError::Failed {
                            message: e.to_string(),
                        }));
                        return;
                    }
                };
                rt.block_on(async move {
                    match client::take_over(&profile.dir).await {
                        Ok(true) => {}
                        Ok(false) => {
                            let _ = ready_tx.send(Err(HostError::Busy));
                            return;
                        }
                        Err(e) => {
                            let _ = ready_tx.send(Err(HostError::Failed {
                                message: format!("{e:#}"),
                            }));
                            return;
                        }
                    }
                    let started = match daemon::start(profile).await {
                        Ok(Some(s)) => s,
                        // Lost the lock to someone between the takeover
                        // and the claim; the app tries again later.
                        Ok(None) => {
                            let _ = ready_tx.send(Err(HostError::Busy));
                            return;
                        }
                        Err(e) => {
                            let _ = ready_tx.send(Err(HostError::Failed {
                                message: format!("{e:#}"),
                            }));
                            return;
                        }
                    };
                    let (serving, client) = daemon::Serving::new(started, true);
                    let _ = ready_tx.send(Ok((client, tokio::runtime::Handle::current())));
                    if let Err(e) = serving.run(shutdown_rx).await {
                        eprintln!("monoplan host: {e:#}");
                    }
                });
            })
            .map_err(|e| HostError::Failed {
                message: e.to_string(),
            })?;
        match ready_rx.recv() {
            Ok(Ok((client, rt))) => Ok(Arc::new(Host {
                shutdown: Mutex::new(Some(shutdown_tx)),
                thread: Mutex::new(Some(thread)),
                client,
                rt,
                profile_dir,
            })),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err(HostError::Failed {
                message: "host thread exited before reporting".into(),
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
    pub fn request(&self, frame: Vec<u8>) -> Result<Vec<u8>, HostError> {
        use monoplan_cli::ipc::{Request, decode_payload};
        if self.client.is_stopped() {
            return Err(HostError::Stopped);
        }
        let req: Request = decode_payload(&frame).map_err(|e| HostError::Failed {
            message: e.to_string(),
        })?;
        Ok(encode(&self.client.request_blocking(req)))
    }

    /// Subscribe to the loop's watch stream (`spec/daemon-plan.md`
    /// projection tier): `sink` gets `Watching` now, then one `Event`
    /// frame per change until the watcher is cancelled or dropped, or
    /// `stopped` once when the loop ends.
    pub fn watch(&self, sink: Arc<dyn HostSink>) -> Result<Arc<Watcher>, HostError> {
        use monoplan_cli::ipc::{Reply, WatchEvent};
        use tokio::sync::broadcast::error::RecvError;
        if self.client.is_stopped() {
            return Err(HostError::Stopped);
        }
        let (sync, mut rx) = self.client.watch();
        sink.frame(encode(&Reply::Watching { sync }));
        let task = self.rt.spawn(async move {
            loop {
                let ev = match rx.recv().await {
                    Ok(ev) => ev,
                    // Behind by more than the buffer: the backlog
                    // collapses to "something changed", as on the socket.
                    Err(RecvError::Lagged(_)) => WatchEvent::Changed,
                    Err(RecvError::Closed) => {
                        sink.stopped();
                        return;
                    }
                };
                sink.frame(encode(&Reply::Event(ev)));
            }
        });
        Ok(Arc::new(Watcher {
            task: Mutex::new(Some(task)),
        }))
    }

    /// Mirror a doc in the window (`spec/mac-window-plan.md`, phase 2):
    /// `workspace` is a `-w` selector or nil for Home. The returned
    /// subscription carries the snapshot and the leased peer id; from
    /// then on `sink.update` gets every change the host applies (a CLI
    /// command, a remote frame, another window's import) until the
    /// subscription is closed or released. Blocks for the snapshot
    /// export, so call it off the main thread.
    pub fn subscribe(
        &self,
        workspace: Option<String>,
        sink: Arc<dyn MirrorSink>,
    ) -> Result<Arc<Subscription>, HostError> {
        if self.client.is_stopped() {
            return Err(HostError::Stopped);
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
    /// to the host's doc: local from here, WALed on the next capture,
    /// pushed, and fanned out to any other mirror. Blocks until they
    /// are durable, so call it off the main thread. `Failed` after
    /// `close`, or for bytes Loro will not import.
    pub fn import(&self, subscription: Arc<Subscription>, bytes: Vec<u8>) -> Result<(), HostError> {
        // Held for the call, so a concurrent `close` waits for the
        // import to land rather than racing it.
        let guard = subscription.inner.lock().unwrap();
        let Some(sub) = guard.as_ref() else {
            return Err(HostError::Failed {
                message: "subscription closed".into(),
            });
        };
        Ok(sub.import_blocking(bytes)?)
    }

    /// The profile as the window should see it (`spec/mac-window-plan.md`
    /// "Swift side"): identity, server, and the open docs, Home first.
    /// Blocks for the loop's turn, so call it off the main thread.
    /// `Failed` for a local-only workspace.
    pub fn session(&self) -> Result<HostSession, HostError> {
        if self.client.is_stopped() {
            return Err(HostError::Stopped);
        }
        let info = self.client.session_blocking()?;
        Ok(HostSession {
            account_id: info.account_id,
            email: info.email,
            device_id: info.device_id,
            primary_doc_id: info.primary_doc_id.0.to_string(),
            server_url: info.server_url,
            docs: info
                .docs
                .into_iter()
                .map(|d| HostDoc {
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

    /// Still serving? False once a client's `Shutdown` ended the loop.
    pub fn is_running(&self) -> bool {
        self.thread
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|t| !t.is_finished())
    }
}
