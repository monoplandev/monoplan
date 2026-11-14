//! `monoplan daemon start` — the resident host (`spec/daemon-plan.md`).
//!
//! Hold a [`Engine`] open for the life of the process and drive its
//! sans-IO replicas (one per doc) from one `select!` loop over the WebSocket, client
//! requests arriving on a Unix socket, a persist tick, a liveness ping,
//! and shutdown signals.
//!
//! - **Ownership.** The daemon holds `engine.lock` (a kernel flock,
//!   released on any exit path) for its lifetime. `Engine::open`
//!   refuses while it is held, so a one-shot command can never write
//!   WAL rows the resident doc would not see.
//! - **Transport policy** (host-owned per `spec/architecture.md`):
//!   reconnect with capped exponential backoff, a ping every
//!   `PING_INTERVAL` whose missed pong counts as a disconnect (after
//!   laptop sleep the socket is usually dead before the OS says so),
//!   and the replica's `OpenDoc` handshake timeout.
//! - **Persist cadence** mirroring the web host: capture on a ~1 s
//!   tick and right after remote frames apply (so the Ack goes out
//!   against durable bytes), threshold fold on the hot path, full fold
//!   on clean exit.
//! - **Two client paths, one loop.** Socket connections and in-process
//!   clients ([`LocalClient`]: the Apple apps through `monoplan-ffi`)
//!   feed the same command channel and subscribe to the same watch
//!   broadcast; the loop never knows which it is serving. The socket is
//!   the CLI's path; an app only ever uses the local one, mostly as
//!   typed calls ([`LocalClient::call_blocking`], `crate::api`), which
//!   are [`HostCmd`] variants the socket codec cannot produce.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use monoplan_core::{DocId, Event, hello_frame, parse_hello_response};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;

use crate::api::{ApiError, Ctx};
use crate::commands::run_doc;
use crate::config::Profile;
use crate::ipc::{self, Reply, Request, SyncStatus, WatchEvent, read_frame, write_frame};
use crate::sync::{Engine, SyncError, WsStream, connect_ws, engine_options, send_outboxes};

/// Socket connect + version handshake budget. Longer than the one-shot
/// CLI's 2 s: the daemon is not holding up a command, and real networks
/// (captive portals, sleepy wifi) take longer than localhost.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Replica-level `OpenDoc` → `DocOpened` budget after the socket is up.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Reconnect backoff: doubles from `BACKOFF_MIN` to `BACKOFF_MAX` with
/// jitter; reset on a successful handshake. While the server is
/// unreachable this *is* the slow poll (`spec/daemon-plan.md`).
const BACKOFF_MIN: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// Liveness ping cadence; a ping sent while the previous pong is still
/// outstanding means the socket is dead.
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// Persist tick: WAL capture + threshold fold + replica timeout check.
const PERSIST_INTERVAL: Duration = Duration::from_secs(1);
/// Events buffered per watcher before it is deemed lagging. A lagging
/// watcher gets one `Changed` instead of the backlog, so this only has
/// to cover a burst between two of its reads.
const WATCH_BUFFER: usize = 64;

const LOCK_FILE: &str = "engine.lock";
/// The engine's log in the profile dir: the embedded host's `tracing`
/// output and a headless daemon's stderr, both appended.
pub const LOG_FILE: &str = "engine.log";
/// Past this size at engine start the log is cut back to its tail.
const LOG_CAP: u64 = 1024 * 1024;
/// How much of the tail a cut keeps.
const LOG_KEEP: u64 = 256 * 1024;

#[derive(Parser, Debug)]
#[command(subcommand_required = true, arg_required_else_help = true)]
pub struct DaemonArgs {
    #[command(subcommand)]
    pub sub: DaemonSub,
}

#[derive(Subcommand, Debug)]
pub enum DaemonSub {
    /// Run the resident process attached to the terminal. Refuses if
    /// one already holds the profile.
    Start {
        /// Started by a client: report readiness as one line on stdout
        /// (`ready`, `already-running`, or `error: …`) and keep running.
        /// Without it the daemon stays attached to the terminal.
        #[arg(long)]
        detach: bool,
    },
    /// Ask the running daemon to exit and wait until it has.
    Stop,
    /// Is a daemon running for this profile?
    Status,
    /// Run the daemon at login: a launchd agent (macOS) or systemd user
    /// unit (Linux) pointing at this executable. Idempotent; also
    /// (re)starts it now.
    Install,
    /// Remove the login agent / unit and stop the daemon it runs.
    Uninstall,
}

/// Exclusive claim that a daemon owns this profile. Same mechanism and
/// same rules as the peer lease (`crate::peer`): kernel flock, held via
/// the `File`, never unlinked.
pub struct DaemonLock {
    _lock: File,
}

fn lock_path(profile_dir: &Path) -> std::path::PathBuf {
    profile_dir.join(LOCK_FILE)
}

fn open_lock_file(profile_dir: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(lock_path(profile_dir))
}

/// Try to become the profile's daemon. `Ok(None)` when another process
/// already holds the lock.
pub fn try_acquire(profile_dir: &Path) -> std::io::Result<Option<DaemonLock>> {
    let file = open_lock_file(profile_dir)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(DaemonLock { _lock: file })),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

/// Is a daemon currently holding this profile? Probes by taking and
/// immediately releasing the lock, so it is a point-in-time answer:
/// callers that need to *act* on "not running" should `try_acquire`
/// and keep the lock instead.
pub fn is_running(profile_dir: &Path) -> std::io::Result<bool> {
    Ok(try_acquire(profile_dir)?.is_none())
}

/// Observable host state, published on every transition. Tests and
/// (later) `daemon status` read it; nothing in the sync path does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DaemonStatus {
    pub online: bool,
    pub last_synced_at: Option<i64>,
    pub offline_reason: Option<String>,
    /// No account: the host never connects (see `ipc::SyncStatus`).
    pub local_only: bool,
    /// Unconfirmed local work (see `ipc::SyncStatus`).
    pub pending: bool,
    /// Contiguous-prefix server seq the replica has applied in memory.
    pub last_contiguous_seq: u64,
    pub connects: u32,
}

impl DaemonStatus {
    pub fn sync(&self) -> SyncStatus {
        SyncStatus {
            online: self.online,
            last_synced_at: self.last_synced_at,
            offline_reason: self.offline_reason.clone(),
            local_only: self.local_only,
            pending: self.pending,
        }
    }
}

/// Control handle for an in-process host (tests, and later the tray /
/// GUI hosts that embed the loop instead of spawning it).
pub struct DaemonHandle {
    pub status: watch::Receiver<DaemonStatus>,
    /// The in-process client path, as the app uses it.
    pub client: LocalClient,
    shutdown: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<Result<(), SyncError>>,
}

impl DaemonHandle {
    /// Ask the host to exit cleanly and wait for it: persists, folds,
    /// closes the socket, releases the lock.
    pub async fn shutdown(mut self) -> Result<(), SyncError> {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        self.task
            .await
            .map_err(|e| SyncError::Engine(format!("daemon task: {e}")))?
    }
}

/// A host that owns the profile and is listening, but not yet serving:
/// the output of [`start`], the input of [`serve`]. Split so the same
/// loop runs as the headless daemon (`run`) and inside the macOS app
/// (`monoplan-ffi::Engine`, `spec/daemon-plan.md` amendment 2026-10-27).
pub struct Started {
    lock: DaemonLock,
    /// `None` for a host with no CLI to serve (the iOS app): the loop
    /// runs, the in-process [`LocalClient`] is its only client, and no
    /// socket file is made. On iOS the container paths also exceed
    /// `sun_path`, so a bind there would fail outright.
    listener: Option<UnixListener>,
    engine: Engine,
}

/// Claim the profile, bind the socket and boot the doc. `Ok(None)` when
/// another process holds the lock.
pub async fn start(profile: Profile) -> anyhow::Result<Option<Started>> {
    start_with(profile, true).await
}

/// `start`, with the socket optional: `serve_socket` false hosts the
/// doc for in-process clients only (the mobile host, which has no CLI
/// and no path short enough for a socket).
pub async fn start_with(profile: Profile, serve_socket: bool) -> anyhow::Result<Option<Started>> {
    let Some(lock) = try_acquire(&profile.dir)? else {
        return Ok(None);
    };
    trim_log(&profile.dir.join(LOG_FILE), LOG_CAP, LOG_KEEP);
    let listener = if serve_socket {
        Some(bind(&profile.dir)?)
    } else {
        None
    };
    let engine = Engine::open_resident(profile).await?;
    Ok(Some(Started {
        lock,
        listener,
        engine,
    }))
}

/// Cut the log at `path` back to its last `keep` bytes (from the next
/// line start) once it has grown past `cap`. Called once per engine
/// start, under the profile lock, so one process trims at a time and a
/// long-lived engine can overshoot until its next start. The file is
/// rewritten in place, never replaced: a headless daemon's stderr is an
/// append-mode descriptor on this same inode and carries on writing at
/// the new end. Best effort: a log that cannot be trimmed is left as
/// it is.
fn trim_log(path: &Path, cap: u64, keep: u64) {
    use std::io::{Read, Seek, SeekFrom, Write};
    let trim = || -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .append(true)
            .open(path)?;
        let len = file.metadata()?.len();
        if len <= cap {
            return Ok(());
        }
        file.seek(SeekFrom::Start(len.saturating_sub(keep)))?;
        let mut tail = Vec::new();
        file.read_to_end(&mut tail)?;
        let start = tail.iter().position(|&b| b == b'\n').map_or(0, |i| i + 1);
        file.set_len(0)?;
        file.write_all(&tail[start..])
    };
    let _ = trim();
}

/// Serve until `shutdown` fires or a client asks (`Request::Shutdown`).
/// `embedded` is what the hello reports: a GUI-hosted loop, which
/// clients must not restart for staleness and which the app, taking a
/// profile over at launch, leaves alone. Installs no signal handlers;
/// the headless `run` does that itself.
pub async fn serve(
    started: Started,
    shutdown: oneshot::Receiver<()>,
    embedded: bool,
) -> anyhow::Result<()> {
    if started.engine.is_local_only() {
        eprintln!("monoplan daemon: listening; local-only workspace, no server");
    } else {
        eprintln!("monoplan daemon: listening, connecting to server…");
    }
    let (serving, _client) = Serving::new(started, embedded);
    let result = serving.run(shutdown).await;
    eprintln!("monoplan daemon: stopped");
    Ok(result?)
}

/// A host wired up and about to run: the loop plus the socket's accept
/// task. [`Serving::new`] is the one place the channels are made, so
/// the socket connections and the in-process [`LocalClient`] are
/// clients of the same loop by construction.
pub struct Serving {
    host: Host,
    /// Absent when the host serves no socket (`Started::listener`).
    accept: Option<tokio::task::JoinHandle<()>>,
}

impl Serving {
    /// Wire the loop and start accepting socket connections. Must be
    /// called inside a Tokio runtime (the accept task is spawned here).
    pub fn new(started: Started, embedded: bool) -> (Serving, LocalClient) {
        let Started {
            lock,
            listener,
            engine,
        } = started;
        // What is known before the loop's first `publish`: a client
        // that reads the status straight after `start` (the app's
        // `observe`) must not be told a local-only workspace is an
        // account that is offline.
        let (status_tx, status_rx) = watch::channel(DaemonStatus {
            local_only: engine.is_local_only(),
            last_synced_at: engine.last_synced_at(),
            ..DaemonStatus::default()
        });
        // Unbounded: every client awaits its reply before its next
        // command, so the queue is bounded by the client count, and a
        // `Subscription`'s `Drop` can release its slot without a runtime.
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (watch_tx, _) = broadcast::channel(WATCH_BUFFER);
        let accept = listener.map(|listener| {
            tokio::spawn(accept_loop(
                listener,
                req_tx.clone(),
                status_rx.clone(),
                watch_tx.subscribe(),
                embedded,
            ))
        });
        let client = LocalClient {
            requests: req_tx,
            status: status_rx,
            watchers: watch_tx.subscribe(),
            embedded,
        };
        let host = Host::new(engine, lock, status_tx, req_rx, watch_tx, embedded);
        (Serving { host, accept }, client)
    }

    /// Serve until `shutdown` fires or a client asks (`Request::Shutdown`).
    /// The accept task ends with the loop: the socket file is gone by
    /// then, and nothing it accepted could be answered.
    pub async fn run(self, shutdown: oneshot::Receiver<()>) -> Result<(), SyncError> {
        let result = self.host.run(shutdown).await;
        if let Some(accept) = self.accept {
            accept.abort();
        }
        result
    }
}

/// An in-process client of the host: what a socket connection task is,
/// without the socket. The Apple apps reach their own host this way
/// through `monoplan-ffi` (`spec/swift-bindings-plan.md`), never over
/// the socket. Cheap to clone; every clone is a client.
///
/// `Hello` and `Watch` are answered here, as the connection task
/// answers them, since they need no doc. Everything else is handed to
/// the loop and answered when it has run.
pub struct LocalClient {
    requests: mpsc::UnboundedSender<HostCmd>,
    status: watch::Receiver<DaemonStatus>,
    /// A receiver, never read, to resubscribe from: holding the sender
    /// would keep the stream open after the loop (its only sender) has
    /// gone, and a watcher would never hear `Closed`.
    watchers: broadcast::Receiver<WatchEvent>,
    embedded: bool,
}

impl Clone for LocalClient {
    fn clone(&self) -> Self {
        LocalClient {
            requests: self.requests.clone(),
            status: self.status.clone(),
            watchers: self.watchers.resubscribe(),
            embedded: self.embedded,
        }
    }
}

impl LocalClient {
    /// The loop has ended (shutdown, or a client's `Shutdown`): no
    /// request will be answered and no event will come.
    pub fn is_stopped(&self) -> bool {
        self.requests.is_closed()
    }

    pub fn sync_status(&self) -> SyncStatus {
        self.status.borrow().sync()
    }

    fn hello(&self) -> Reply {
        Reply::Hello {
            build_id: ipc::build_id(),
            pid: std::process::id(),
            embedded: self.embedded,
            sync: self.sync_status(),
        }
    }

    fn stopped() -> Reply {
        Reply::Err {
            message: "the host has stopped".into(),
        }
    }

    /// One request, answered in order with the loop's other clients.
    pub async fn request(&self, req: Request) -> Reply {
        match req {
            Request::Hello { .. } => self.hello(),
            Request::Watch => Reply::Err {
                message: "watch is a subscription in process: use `LocalClient::watch`".into(),
            },
            other => {
                let (tx, rx) = oneshot::channel();
                if self.requests.send(HostCmd::Ipc(other, tx)).is_err() {
                    return Self::stopped();
                }
                rx.await.unwrap_or_else(|_| Self::stopped())
            }
        }
    }

    /// [`request`](Self::request) for a thread that is not on the
    /// runtime (an FFI call from the app's own threads). Must not be
    /// called from an async context.
    pub fn request_blocking(&self, req: Request) -> Reply {
        match req {
            Request::Hello { .. } => self.hello(),
            Request::Watch => Reply::Err {
                message: "watch is a subscription in process: use `LocalClient::watch`".into(),
            },
            other => {
                let (tx, rx) = oneshot::channel();
                if self.requests.send(HostCmd::Ipc(other, tx)).is_err() {
                    return Self::stopped();
                }
                rx.blocking_recv().unwrap_or_else(|_| Self::stopped())
            }
        }
    }

    /// A typed call (`crate::api`) from a thread that is not on the
    /// runtime (an FFI call from the app's own threads): `f` runs on
    /// the loop against the doc `workspace` names (`None` is Home),
    /// in order with the loop's other clients, and this blocks for its
    /// answer. What it committed is durable when this returns, and a
    /// `Changed` event goes to the watchers if it changed anything.
    pub fn call_blocking<T, F>(&self, workspace: Option<String>, f: F) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce(&Ctx<'_>) -> anyhow::Result<T> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let run: CallFn = Box::new(move |cx| {
            let result = cx.and_then(|cx| f(cx).map_err(ApiError::from));
            Box::new(move || {
                let _ = tx.send(result);
            })
        });
        self.requests
            .send(HostCmd::Call { workspace, run })
            .map_err(|_| ApiError::Stopped)?;
        rx.blocking_recv().map_err(|_| ApiError::Stopped)?
    }

    /// The profile as an app shows it: identity, server, and the open
    /// docs, Home first. A local-only workspace answers with empty
    /// identity fields and `local_only` set.
    pub async fn session(&self) -> Result<SessionInfo, ApiError> {
        let (tx, rx) = oneshot::channel();
        self.requests
            .send(HostCmd::Session { reply: tx })
            .map_err(|_| ApiError::Stopped)?;
        rx.await.map_err(|_| ApiError::Stopped)?
    }

    /// [`session`](Self::session) for a thread that is not on the
    /// runtime (an FFI call from the app's own threads).
    pub fn session_blocking(&self) -> Result<SessionInfo, ApiError> {
        let (tx, rx) = oneshot::channel();
        self.requests
            .send(HostCmd::Session { reply: tx })
            .map_err(|_| ApiError::Stopped)?;
        rx.blocking_recv().map_err(|_| ApiError::Stopped)?
    }

    /// Subscribe to the watch stream: the sync status as of now (what
    /// `Watching` carries on the socket) and the receiver of every
    /// later event. Subscribed before the status is read, so nothing
    /// that happens after the returned status can be missed. A
    /// receiver that lags `WATCH_BUFFER` events gets `Lagged`, which
    /// means exactly `Changed`; `Closed` means the loop has ended.
    pub fn watch(&self) -> (SyncStatus, broadcast::Receiver<WatchEvent>) {
        let rx = self.watchers.resubscribe();
        (self.sync_status(), rx)
    }
}

/// `monoplan daemon …` entry point.
pub async fn run(args: DaemonArgs) -> anyhow::Result<()> {
    let detach = match args.sub {
        DaemonSub::Stop => return stop().await,
        DaemonSub::Status => return status().await,
        DaemonSub::Install => return crate::service::install().await,
        DaemonSub::Uninstall => return crate::service::uninstall().await,
        DaemonSub::Start { detach } => detach,
    };
    // Everything up to "listening" is startup; in detach mode its
    // outcome is the one line the spawning client reads from stdout.
    // A fresh install gets its local-only workspace here, as it would
    // from any doc command.
    let started = async { start(crate::bootstrap::open_or_init().await?).await }.await;
    let started = match started {
        Ok(Some(parts)) => parts,
        Ok(None) => {
            if detach {
                ready_line("already-running");
                return Ok(());
            }
            anyhow::bail!("monoplan daemon is already running for this profile");
        }
        Err(e) => {
            if detach {
                ready_line(&format!("error: {e:#}"));
            }
            return Err(e);
        }
    };
    if detach {
        ready_line("ready");
    }
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    tokio::spawn(async move {
        wait_for_signal().await;
        let _ = shutdown_tx.send(());
    });
    serve(started, shutdown_rx, false).await
}

/// The one line a spawning client reads. Written with the error
/// ignored: if the client already went away the pipe is closed, and a
/// `println!` would panic on EPIPE and take the daemon down with it.
fn ready_line(line: &str) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

async fn stop() -> anyhow::Result<()> {
    let profile = Profile::require_active()?;
    if crate::client::stop_if_running(&profile.dir).await? {
        println!("stopped");
    } else {
        println!("not running");
    }
    Ok(())
}

async fn status() -> anyhow::Result<()> {
    let profile = Profile::require_active()?;
    if !is_running(&profile.dir)? {
        if crate::service::is_installed_for(&profile.dir) {
            println!(
                "not running  (installed; starts at login, or now with `monoplan daemon install`)"
            );
        } else {
            println!("not running");
        }
        return Ok(());
    }
    let path = ipc::socket_path(&profile.dir);
    let mut stream = UnixStream::connect(&path).await?;
    write_frame(
        &mut stream,
        &Request::Hello {
            build_id: ipc::build_id(),
        },
    )
    .await?;
    match read_frame::<_, Reply>(&mut stream).await? {
        Some(Reply::Hello {
            build_id,
            pid,
            embedded,
            sync,
        }) => {
            let mine = ipc::build_id();
            let note = if embedded {
                if ipc::version_of(&build_id) == ipc::version_of(&mine) {
                    "  (hosted by Monoplan.app)"
                } else {
                    "  (hosted by Monoplan.app at another version; commands will refuse)"
                }
            } else if build_id == mine {
                ""
            } else {
                "  (different build; the next command restarts it)"
            };
            let link = if sync.local_only {
                "local only (no account)".to_string()
            } else if sync.online {
                "online".to_string()
            } else {
                sync.offline_notice(crate::sync::now_millis())
                    .unwrap_or_default()
            };
            println!("running  pid {pid}  build {build_id}  {link}{note}");
        }
        other => println!("running but not answering hello: {other:?}"),
    }
    Ok(())
}

/// Listen on the profile's socket. Holding `engine.lock` means any
/// existing socket file is stale (its owner is gone), so unlink first.
fn bind(profile_dir: &Path) -> anyhow::Result<UnixListener> {
    let path = ipc::socket_path(profile_dir);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let listener = UnixListener::bind(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(listener)
}

/// One task per connection: frames in, requests to the host, replies
/// out. `Hello` and `Watch` are answered here (they need no doc);
/// everything else waits for the host loop, which runs requests inline
/// against the resident doc one at a time.
async fn accept_loop(
    listener: UnixListener,
    req_tx: mpsc::UnboundedSender<HostCmd>,
    status: watch::Receiver<DaemonStatus>,
    watch_rx: broadcast::Receiver<WatchEvent>,
    embedded: bool,
) {
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("accept: {e}");
                continue;
            }
        };
        if !peer_is_us(&stream) {
            tracing::warn!("rejected connection from another uid");
            continue;
        }
        let req_tx = req_tx.clone();
        let status = status.clone();
        let watch_rx = watch_rx.resubscribe();
        tokio::spawn(async move {
            if let Err(e) = serve_conn(stream, req_tx, status, watch_rx, embedded).await {
                tracing::debug!("conn: {e}");
            }
        });
    }
}

#[cfg(unix)]
fn peer_is_us(stream: &UnixStream) -> bool {
    // SAFETY: getuid has no preconditions.
    let me = unsafe { libc::getuid() };
    stream.peer_cred().map(|c| c.uid() == me).unwrap_or(false)
}

/// What an app needs to know about the profile (`Engine::account` and
/// `Engine::workspaces` on the FFI): the identity the loop booted with,
/// the server it syncs with, and every doc it has open, Home first.
/// In-process only: the socket has no frame for it.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// Empty for a local-only workspace, as are `email`, `device_id`
    /// and `server_url`.
    pub account_id: String,
    pub email: String,
    pub device_id: String,
    pub primary_doc_id: DocId,
    pub server_url: String,
    /// No account behind the profile (`crate::bootstrap`).
    pub local_only: bool,
    pub docs: Vec<SessionDoc>,
}

/// One open doc as the `-w` resolver sees it.
#[derive(Debug, Clone)]
pub struct SessionDoc {
    pub doc_id: DocId,
    /// `doc_name`; `None` for Home and for an unnamed doc.
    pub name: Option<String>,
}

/// Everything the loop is asked to do, from either client path. The
/// socket's connection task deserialises `Request`s and can only build
/// `Ipc`; the other variants exist for in-process clients alone.
pub(crate) enum HostCmd {
    Ipc(Request, oneshot::Sender<Reply>),
    /// The profile as an app shows it (`SessionInfo`).
    Session {
        reply: oneshot::Sender<Result<SessionInfo, ApiError>>,
    },
    /// A typed call (`crate::api`, `spec/swift-bindings-plan.md`):
    /// `run` is handed the context for `workspace` (`None` is Home),
    /// or why there is none, and returns the thunk that answers its
    /// caller. The loop runs that once what the call committed is
    /// durable, as a doc command's reply waits for the same.
    Call {
        workspace: Option<String>,
        run: CallFn,
    },
}

/// The body of a [`HostCmd::Call`], run once on the loop.
pub(crate) type CallFn =
    Box<dyn FnOnce(Result<&Ctx<'_>, ApiError>) -> Box<dyn FnOnce() + Send> + Send>;

/// Requests are read by a separate task and handed over a channel, so
/// the `select!` below never cancels a half-read frame (a `read_frame`
/// future dropped between its length prefix and payload would
/// desynchronise the stream). The loop then interleaves replies with
/// watch events on the one write half. `watch_rx` is only resubscribed
/// from: the host loop is the stream's one sender, so its end closes
/// the stream for every watcher.
async fn serve_conn(
    stream: UnixStream,
    req_tx: mpsc::UnboundedSender<HostCmd>,
    status: watch::Receiver<DaemonStatus>,
    watch_rx: broadcast::Receiver<WatchEvent>,
    embedded: bool,
) -> anyhow::Result<()> {
    let (mut rd, mut wr) = stream.into_split();
    let (in_tx, mut in_rx) = mpsc::channel::<Request>(1);
    let reader = tokio::spawn(async move {
        loop {
            match read_frame::<_, Request>(&mut rd).await {
                Ok(Some(req)) => {
                    if in_tx.send(req).await.is_err() {
                        return Ok(());
                    }
                }
                Ok(None) => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    });
    let mut events: Option<broadcast::Receiver<WatchEvent>> = None;
    loop {
        tokio::select! {
            req = in_rx.recv() => {
                let Some(req) = req else { break };
                let reply = match req {
                    Request::Hello { .. } => Reply::Hello {
                        build_id: ipc::build_id(),
                        pid: std::process::id(),
                        embedded,
                        sync: status.borrow().sync(),
                    },
                    Request::Watch => {
                        // Subscribe before replying: nothing that
                        // happens after `Watching` can be missed.
                        events = Some(watch_rx.resubscribe());
                        Reply::Watching {
                            sync: status.borrow().sync(),
                        }
                    }
                    other => {
                        let (tx, rx) = oneshot::channel();
                        if req_tx.send(HostCmd::Ipc(other, tx)).is_err() {
                            break;
                        }
                        match rx.await {
                            Ok(r) => r,
                            Err(_) => break,
                        }
                    }
                };
                write_frame(&mut wr, &reply).await?;
            }
            ev = async { events.as_mut().expect("guarded").recv().await }, if events.is_some() => {
                let ev = match ev {
                    Ok(ev) => ev,
                    // Behind by more than the buffer: the backlog
                    // collapses to "something changed", which is all
                    // a payload-free stream ever said anyway.
                    Err(broadcast::error::RecvError::Lagged(_)) => WatchEvent::Changed,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                write_frame(&mut wr, &Reply::Event(ev)).await?;
            }
        }
    }
    reader.abort();
    Ok(())
}

/// Spawn the host on the current runtime for a given profile. Fails
/// fast if a daemon already holds the profile. Used by tests; `run` is
/// the binary's path.
pub async fn spawn(profile: Profile) -> Result<DaemonHandle, SyncError> {
    spawn_with(profile, false).await
}

/// `spawn` reporting itself as embedded (what the app's host does), to
/// test the client-facing rules without an app.
pub async fn spawn_with(profile: Profile, embedded: bool) -> Result<DaemonHandle, SyncError> {
    let lock = try_acquire(&profile.dir)
        .map_err(crate::peer::PeerError::Io)?
        .ok_or(SyncError::DaemonRunning)?;
    let listener = bind(&profile.dir).map_err(|e| SyncError::Engine(format!("bind: {e}")))?;
    let engine = Engine::open_resident(profile).await?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (serving, client) = Serving::new(
        Started {
            lock,
            listener: Some(listener),
            engine,
        },
        embedded,
    );
    let status = client.status.clone();
    let task = tokio::spawn(serving.run(shutdown_rx));
    Ok(DaemonHandle {
        status,
        client,
        shutdown: Some(shutdown_tx),
        task,
    })
}

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Socket + per-connection version handshake. The replica's own
/// `OpenDoc` exchange starts in `handle_connected` afterwards. A free
/// function (not `&self`) so the host's `select!` future stays `Send`.
async fn handshake(profile: &Profile, server_url: &str) -> Result<WsStream, SyncError> {
    let mut ws = connect_ws(profile, server_url).await?;
    ws.send(Message::Binary(
        hello_frame(&engine_options("monoplan-daemon")).into(),
    ))
    .await?;
    let first = loop {
        match ws.next().await {
            Some(Ok(Message::Binary(b))) => break b.to_vec(),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Ok(other)) => {
                return Err(SyncError::Ws(format!(
                    "unexpected frame in hello: {other:?}"
                )));
            }
            Some(Err(e)) => return Err(e.into()),
            None => return Err(SyncError::Ws("closed during hello".into())),
        }
    };
    parse_hello_response(&first).map_err(|e| SyncError::Engine(e.to_string()))?;
    Ok(ws)
}

struct Host {
    engine: Engine,
    _lock: DaemonLock,
    /// Watched on the persist tick: if the profile dir is purged from
    /// under us (logout, a test's tempdir) there is nothing left to own.
    lock_path: std::path::PathBuf,
    socket_path: std::path::PathBuf,
    status: watch::Sender<DaemonStatus>,
    requests: mpsc::UnboundedReceiver<HostCmd>,
    /// Fan-out to watching connections. `send` fails when nobody is
    /// watching, which is the common case and not an error.
    watchers: broadcast::Sender<WatchEvent>,
    ws: Option<WsStream>,
    backoff: Duration,
    next_connect: Instant,
    connected_at: Option<Instant>,
    awaiting_pong: bool,
    /// In-flight connect + version handshake, run as its own task so
    /// the loop keeps serving clients and ticks while it waits.
    connecting: Option<tokio::task::JoinHandle<Result<WsStream, SyncError>>>,
    /// `Sync` requests parked until the current connect attempt
    /// resolves (initial pull done, or the attempt failed).
    pending_syncs: Vec<oneshot::Sender<Reply>>,
    /// The last connect failure or drop, until the next connect.
    offline_reason: Option<String>,
    /// No account: never connect, never report offline.
    local_only: bool,
    embedded: bool,
}

impl Host {
    fn new(
        engine: Engine,
        lock: DaemonLock,
        status: watch::Sender<DaemonStatus>,
        requests: mpsc::UnboundedReceiver<HostCmd>,
        watchers: broadcast::Sender<WatchEvent>,
        embedded: bool,
    ) -> Self {
        let dir = engine.profile_dir().to_path_buf();
        let local_only = engine.is_local_only();
        Host {
            engine,
            _lock: lock,
            lock_path: lock_path(&dir),
            socket_path: ipc::socket_path(&dir),
            status,
            requests,
            watchers,
            ws: None,
            backoff: BACKOFF_MIN,
            next_connect: Instant::now(),
            connected_at: None,
            awaiting_pong: false,
            connecting: None,
            pending_syncs: Vec::new(),
            offline_reason: None,
            local_only,
            embedded,
        }
    }

    async fn run(mut self, mut shutdown: oneshot::Receiver<()>) -> Result<(), SyncError> {
        let mut persist = tokio::time::interval(PERSIST_INTERVAL);
        persist.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut ping = tokio::time::interval(PING_INTERVAL);
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The first tick of an interval fires immediately; swallow the
        // ping's so we don't ping before the socket exists.
        ping.tick().await;

        loop {
            tokio::select! {
                _ = &mut shutdown => break,

                _ = tokio::time::sleep_until(self.next_connect.into()),
                    if self.ws.is_none() && self.connecting.is_none() && !self.local_only =>
                {
                    self.start_connect();
                }

                res = async { self.connecting.as_mut().expect("guarded").await }, if self.connecting.is_some() => {
                    self.connecting = None;
                    self.on_connect_result(res).await;
                }

                msg = async { self.ws.as_mut().expect("guarded").next().await }, if self.ws.is_some() => {
                    self.on_ws_message(msg).await;
                }

                Some(cmd) = self.requests.recv() => {
                    if self.on_cmd(cmd).await? {
                        break;
                    }
                }

                _ = persist.tick() => {
                    if !self.lock_path.exists() {
                        eprintln!("monoplan daemon: profile directory is gone, exiting");
                        return Ok(());
                    }
                    self.on_persist_tick().await?;
                }

                _ = ping.tick(), if self.ws.is_some() => {
                    self.on_ping_tick().await;
                }
            }
        }

        // Clean exit: everything durable, WAL folded (cheap boot next
        // time), socket closed politely. Errors here are reported, not
        // swallowed — the lock releases regardless when `self` drops.
        self.engine.persist_replica_state().await?;
        self.engine.force_snapshot()?;
        if let Some(mut ws) = self.ws.take() {
            let _ = ws.close(None).await;
        }
        let _ = std::fs::remove_file(&self.socket_path);
        Ok(())
    }

    /// One command from either client path. Returns `true` on `Shutdown`.
    async fn on_cmd(&mut self, cmd: HostCmd) -> Result<bool, SyncError> {
        match cmd {
            HostCmd::Ipc(req, reply) => self.on_request(req, reply).await,
            HostCmd::Session { reply } => {
                let _ = reply.send(self.on_session());
                Ok(false)
            }
            HostCmd::Call { workspace, run } => {
                self.on_call(workspace, run).await?;
                Ok(false)
            }
        }
    }

    /// Run a typed call against the doc `workspace` names, with the
    /// context a doc command gets (`run_doc`), then make what it
    /// committed durable and put it on the wire.
    async fn on_call(&mut self, workspace: Option<String>, run: CallFn) -> Result<(), SyncError> {
        let doc_id = match self.engine.resolve(workspace.as_deref()) {
            Ok(id) => id,
            Err(e) => {
                run(Err(ApiError::NoSuchWorkspace {
                    message: e.to_string(),
                }))();
                return Ok(());
            }
        };
        let answer = self.with_ctx(doc_id, |cx| run(Ok(cx)));
        self.after_doc_work().await?;
        answer();
        Ok(())
    }

    /// The context a command or call on `doc_id` runs with: that doc,
    /// every open doc (and each one's workspace tag, for the reads
    /// that span them), and Starred as seen from it. Starred is Home's
    /// alone; a command on another workspace stars through a foreign
    /// ref there.
    fn with_ctx<T>(&self, doc_id: DocId, f: impl FnOnce(&Ctx<'_>) -> T) -> T {
        let doc = self
            .engine
            .doc_for(doc_id)
            .expect("resolved against open docs");
        // Home first (`Engine::doc_ids`).
        let workspaces: Vec<(crate::workspace::Candidate, &monoplan_core::Doc)> = self
            .engine
            .candidates()
            .into_iter()
            .filter_map(|c| self.engine.doc_for(c.doc_id).map(|d| (c, d)))
            .collect();
        let open_docs: Vec<&monoplan_core::Doc> = workspaces.iter().map(|(_, d)| *d).collect();
        let lens = if doc_id == self.engine.primary_doc_id() {
            crate::commands::focus::Lens::home(doc)
        } else {
            crate::commands::focus::Lens::foreign(self.engine.doc(), doc_id)
        };
        f(&Ctx {
            doc,
            open_docs: &open_docs,
            lens: &lens,
            workspaces: &workspaces,
            home: Some(self.engine.primary_doc_id()),
        })
    }

    /// After a command or call ran: whatever it committed is durable
    /// before the client hears back, and on the wire right after.
    async fn after_doc_work(&mut self) -> Result<(), SyncError> {
        self.engine.persist_replica_state().await?;
        self.engine.flush_replicas();
        self.pump().await;
        self.notify_if_changed();
        Ok(())
    }

    /// The profile for an app: identity and server when there is an
    /// account; for a local-only workspace (`crate::bootstrap`) the
    /// identity fields are empty and `local_only` says so.
    fn on_session(&self) -> Result<SessionInfo, ApiError> {
        let account = self.engine.account().map_err(|e| ApiError::Failed {
            message: format!("read account: {e}"),
        })?;
        let (identity, server_url) = match (account.identity, self.engine.connect_params()) {
            (Some(identity), Some((_, server_url))) => (Some(identity), server_url.to_string()),
            _ => (None, String::new()),
        };
        let local_only = identity.is_none();
        let identity = identity.unwrap_or_default();
        Ok(SessionInfo {
            account_id: identity.account_id,
            email: identity.email,
            device_id: identity.device_id,
            primary_doc_id: account.primary_doc_id,
            server_url,
            local_only,
            docs: self
                .engine
                .candidates()
                .into_iter()
                .map(|c| SessionDoc {
                    doc_id: c.doc_id,
                    name: c.name,
                })
                .collect(),
        })
    }

    /// Run one client request inline. Returns `true` on `Shutdown`.
    async fn on_request(
        &mut self,
        req: Request,
        reply: oneshot::Sender<Reply>,
    ) -> Result<bool, SyncError> {
        let out = match req {
            Request::Hello { .. } => Reply::Hello {
                build_id: ipc::build_id(),
                pid: std::process::id(),
                embedded: self.embedded,
                sync: self.sync_status(),
            },
            // The reads that span every open doc (`agenda --all`,
            // `starred --all`, `find`) run from Home like any other
            // command; they have no workspace to be told.
            Request::Doc { workspace, cmd } => {
                if workspace.is_some()
                    && let Some(message) = cmd.spans_workspaces()
                {
                    let _ = reply.send(Reply::Err {
                        message: message.into(),
                    });
                    return Ok(false);
                }
                let doc_id = match self.engine.resolve(workspace.as_deref()) {
                    Ok(id) => id,
                    Err(e) => {
                        let _ = reply.send(Reply::Err {
                            message: e.to_string(),
                        });
                        return Ok(false);
                    }
                };
                let mut buf = Vec::new();
                let result = self.with_ctx(doc_id, |cx| run_doc(cmd, cx, &mut buf));
                self.after_doc_work().await?;
                match result {
                    Ok(()) => Reply::Output {
                        stdout: String::from_utf8_lossy(&buf).into_owned(),
                        sync: self.sync_status(),
                    },
                    Err(e) => Reply::Err {
                        message: format!("{e:#}"),
                    },
                }
            }
            Request::Sync => {
                self.engine.persist_replica_state().await?;
                if self.local_only {
                    // Nothing to push to; say so now rather than park
                    // the request behind a connect that never starts.
                    Reply::Synced {
                        sync: self.sync_status(),
                    }
                } else if self.ws.is_some() && self.engine.all_idle() {
                    self.engine.flush_replicas();
                    self.pump().await;
                    Reply::Synced {
                        sync: self.sync_status(),
                    }
                } else {
                    // Connecting, pulling, or in backoff: answer when
                    // the attempt resolves. In backoff, don't make the
                    // user wait it out — try now.
                    self.pending_syncs.push(reply);
                    if self.ws.is_none() && self.connecting.is_none() {
                        self.next_connect = Instant::now();
                    }
                    return Ok(false);
                }
            }
            Request::Reconnect => {
                if !self.local_only {
                    self.backoff = BACKOFF_MIN;
                    if self.ws.is_some() {
                        self.disconnect("network changed");
                    } else if self.connecting.is_none() {
                        self.next_connect = Instant::now();
                    }
                }
                Reply::Synced {
                    sync: self.sync_status(),
                }
            }
            Request::Account => match self.engine.account() {
                Ok(a) => match a.identity {
                    Some(i) => Reply::Account {
                        account_id: i.account_id,
                        email: i.email,
                        device_id: i.device_id,
                    },
                    // The shape stays what the app decodes; a
                    // local-only workspace has no identity to put in it.
                    None => Reply::Err {
                        message: "no account: this workspace is local-only".into(),
                    },
                },
                Err(e) => Reply::Err {
                    message: format!("read account: {e}"),
                },
            },
            Request::CreateWorkspace { name } => {
                if self.local_only {
                    Reply::Err {
                        message: "no account: sign in to create a workspace".into(),
                    }
                } else {
                    match self.engine.create_doc(&name).await {
                        Ok(doc_id) => {
                            // On a live socket the doc opens now, so the
                            // seed goes out with the usual OpenDoc →
                            // push sequence; otherwise the next connect
                            // opens it with the rest. Durable before
                            // the client hears back, like a doc command.
                            if self.ws.is_some() {
                                self.engine.connect_doc(doc_id);
                            }
                            self.engine.persist_replica_state().await?;
                            self.engine.flush_replicas();
                            self.pump().await;
                            self.notify_if_changed();
                            // Watchers list workspaces from the host
                            // (Settings, `-w` completion): a new one is
                            // a change even though no item moved.
                            let _ = self.watchers.send(WatchEvent::Changed);
                            Reply::WorkspaceCreated {
                                doc_id: doc_id.0.to_string(),
                                name: name.trim().to_string(),
                                sync: self.sync_status(),
                            }
                        }
                        Err(e) => Reply::Err {
                            message: format!("create workspace: {e}"),
                        },
                    }
                }
            }
            Request::DeleteWorkspace { workspace } => {
                let resolved = self.engine.resolve(Some(&workspace));
                match resolved {
                    Err(e) => Reply::Err {
                        message: e.to_string(),
                    },
                    Ok(doc_id) => match self.engine.delete_doc(doc_id).await {
                        Ok(()) => {
                            // Watchers re-list before the client's reply.
                            let _ = self.watchers.send(WatchEvent::Changed);
                            Reply::WorkspaceDeleted {
                                doc_id: doc_id.0.to_string(),
                                sync: self.sync_status(),
                            }
                        }
                        Err(e) => Reply::Err {
                            message: format!("delete workspace: {e}"),
                        },
                    },
                }
            }
            // Honoured by every host, the app's included: the app
            // re-hosts once the profile is free again (`monoplan-ffi`).
            Request::Shutdown => {
                let _ = reply.send(Reply::ShuttingDown);
                return Ok(true);
            }
            // Answered in the connection task; never reaches the host.
            Request::Watch => Reply::Err {
                message: "watch is handled per connection".into(),
            },
        };
        let _ = reply.send(out);
        Ok(false)
    }

    fn start_connect(&mut self) {
        // Guarded by `local_only` in the loop; belt and braces here.
        let Some((profile, server_url)) = self.engine.connect_params() else {
            return;
        };
        let profile = Profile::new(profile.dir.clone());
        let server_url = server_url.to_string();
        self.connecting = Some(tokio::spawn(async move {
            match tokio::time::timeout(CONNECT_TIMEOUT, handshake(&profile, &server_url)).await {
                Ok(r) => r,
                Err(_) => Err(SyncError::Ws("connect timed out".into())),
            }
        }));
    }

    async fn on_connect_result(
        &mut self,
        res: Result<Result<WsStream, SyncError>, tokio::task::JoinError>,
    ) {
        match res {
            Ok(Ok(ws)) => {
                self.ws = Some(ws);
                self.backoff = BACKOFF_MIN;
                self.connected_at = Some(Instant::now());
                self.awaiting_pong = false;
                self.engine.handle_connected();
                self.offline_reason = None;
                self.status.send_modify(|s| s.connects += 1);
                eprintln!("monoplan daemon: connected");
                self.pump().await;
            }
            Ok(Err(e)) => {
                self.resolve_pending_syncs(false);
                self.schedule_reconnect(&format!("connect failed: {e}"));
            }
            Err(e) => {
                self.resolve_pending_syncs(false);
                self.schedule_reconnect(&format!("connect task failed: {e}"));
            }
        }
    }

    fn resolve_pending_syncs(&mut self, online: bool) {
        let sync = SyncStatus {
            online,
            last_synced_at: self.engine.last_synced_at(),
            offline_reason: self.offline_reason.clone(),
            local_only: self.local_only,
            pending: self.has_pending_work(),
        };
        for tx in self.pending_syncs.drain(..) {
            let _ = tx.send(Reply::Synced { sync: sync.clone() });
        }
    }

    fn sync_status(&self) -> SyncStatus {
        SyncStatus {
            online: self.is_online(),
            last_synced_at: self.engine.last_synced_at(),
            offline_reason: self.offline_reason.clone(),
            local_only: self.local_only,
            pending: self.has_pending_work(),
        }
    }

    fn is_online(&self) -> bool {
        self.ws.is_some() && self.engine.home_replica().is_online()
    }

    /// Anything the server has not confirmed: committed ops not yet
    /// pushed or a push in flight on any doc, or, while the socket is
    /// up, a replica still opening or pulling. `online` alone flips
    /// true at the handshake, before a byte of the doc has moved, so
    /// this is what separates "syncing" from "synced".
    fn has_pending_work(&self) -> bool {
        self.engine.any_unsynced_ops() || (self.ws.is_some() && !self.engine.all_idle())
    }

    async fn on_ws_message(
        &mut self,
        msg: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    ) {
        match msg {
            Some(Ok(Message::Binary(bytes))) => {
                self.engine.route_server_bytes(&bytes);
                // Remote updates are WAL-appended by the replica as they
                // apply; persisting now advances the durable cursor,
                // which is what queues the Ack `pump` then sends.
                if let Err(e) = self.engine.persist_replica_state().await {
                    self.disconnect(&format!("persist after frame: {e}"));
                    return;
                }
                self.pump().await;
                self.notify_if_changed();
            }
            Some(Ok(Message::Pong(_))) => self.awaiting_pong = false,
            // tungstenite queues the Pong reply itself; it leaves on the
            // next send. Nothing to do.
            Some(Ok(Message::Ping(_))) => {}
            Some(Ok(Message::Close(_))) | None => self.disconnect("server closed"),
            Some(Ok(other)) => self.disconnect(&format!("unexpected frame: {other:?}")),
            Some(Err(e)) => self.disconnect(&format!("socket error: {e}")),
        }
    }

    async fn on_persist_tick(&mut self) -> Result<(), SyncError> {
        self.engine.persist_replica_state().await?;
        // Hot-path fold uses the real thresholds; exit folds everything.
        self.engine.snapshot_if_wal_exceeds()?;
        // Anything the two hot paths did not already report (there
        // should be nothing); also keeps the queue from accumulating.
        self.notify_if_changed();
        if self.ws.is_some() {
            if let Some(at) = self.connected_at
                && at.elapsed() > HANDSHAKE_TIMEOUT
            {
                // Escalates only while still in `Opening`; a no-op
                // once the doc is open, so calling it every tick
                // past the budget is harmless.
                self.engine.handle_timeout();
            }
            self.engine.flush_replicas();
            self.pump().await;
        }
        self.publish();
        Ok(())
    }

    async fn on_ping_tick(&mut self) {
        if self.awaiting_pong {
            self.disconnect("pong timeout");
            return;
        }
        let Some(ws) = self.ws.as_mut() else { return };
        match ws.send(Message::Ping(Default::default())).await {
            Ok(()) => self.awaiting_pong = true,
            Err(e) => self.disconnect(&format!("ping failed: {e}")),
        }
    }

    /// Drain the replica's outbox to the wire and act on its events.
    /// Called after every replica interaction while a socket is open.
    async fn pump(&mut self) {
        if let Some(ws) = self.ws.as_mut()
            && let Err(e) = send_outboxes(ws, &mut self.engine).await
        {
            self.disconnect(&format!("send failed: {e}"));
            return;
        }
        let home = self.engine.primary_doc_id();
        let mut forget: Vec<DocId> = Vec::new();
        for (doc_id, event) in self.engine.drain_events() {
            match event {
                // Not acted on as events: whether Home is open on the
                // socket is read off the replica after the drain (below),
                // because a `false` here may be stale. A host-initiated
                // `disconnect` (a `Reconnect` request, a pong timeout,
                // a replica error in this very loop) queues one with no
                // pump before the next connect; drained *then*, beside
                // the new `true`, it describes the old socket, and
                // dropping the new one for it left the replica `Opening`
                // with no socket, cycling "replica disconnected" /
                // "handle_connected called while already connected".
                // A non-Home replica going offline on its own is the
                // `DocClosed` below; the socket stays up for the rest.
                Event::ConnStateChanged { .. } => {}
                Event::PulledInitial => {
                    if let Err(e) = self.engine.mark_synced(doc_id) {
                        tracing::warn!("mark_synced: {e}");
                    }
                    if doc_id == home && !self.pending_syncs.is_empty() {
                        self.engine.flush_replicas();
                        self.resolve_pending_syncs(true);
                    }
                }
                Event::Pushed => {
                    if let Err(e) = self.engine.mark_synced(doc_id) {
                        tracing::warn!("mark_synced: {e}");
                    }
                }
                // A live batch from another device, applied: an
                // exchange completed, so the stamp moves. Not during
                // the initial pull, which stamps once on `PulledInitial`
                // rather than once per batch.
                Event::FrontierAdvanced { .. } => {
                    if self.engine.replica_idle(doc_id)
                        && let Err(e) = self.engine.mark_synced(doc_id)
                    {
                        tracing::warn!("mark_synced: {e}");
                    }
                }
                // Who else has the doc open. Nothing in the CLI shows
                // it yet; the web client's navbar does.
                Event::Presence { .. } => {}
                Event::Error(msg) => {
                    tracing::warn!("replica {}: {msg}", doc_id.0);
                    self.disconnect(&format!("replica error: {msg}"));
                }
                // The server will not serve this doc to this account
                // any more (membership removed, or the invite was
                // revoked under us): forget it, keep the socket for
                // the rest (`spec/sharing-plan.md` §Member removal).
                Event::DocClosed { reason } if doc_id != home => {
                    tracing::warn!("replica: doc {} closed: {reason}; forgetting it", doc_id.0);
                    forget.push(doc_id);
                }
                // Home is non-leavable, so this means the server
                // refused the account's own primary doc — a
                // server-side fault, not something a reconnect loop
                // will fix quickly. Treat it like any fatal replica
                // error: drop the socket and let the ordinary backoff
                // pace the retries.
                Event::DocClosed { reason } => {
                    tracing::warn!("replica: home doc closed: {reason}");
                    self.disconnect(&format!("doc closed: {reason}"));
                }
            }
        }
        // Home went Disconnected on its own (rejected handshake, fatal
        // frame) while the socket is up: drop the socket to match.
        if self.ws.is_some() && !self.engine.home_replica().is_online() {
            self.drop_socket("replica disconnected");
        }
        for doc_id in forget {
            if let Err(e) = self.engine.forget_doc(doc_id) {
                tracing::warn!("forget doc {}: {e}", doc_id.0);
            }
            let _ = self.watchers.send(WatchEvent::Changed);
        }
        self.publish();
    }

    /// Host-initiated disconnect: tell the replica, drop the socket,
    /// schedule a reconnect.
    fn disconnect(&mut self, reason: &str) {
        self.engine.handle_disconnected();
        self.drop_socket(reason);
    }

    fn drop_socket(&mut self, reason: &str) {
        if self.ws.take().is_some() {
            eprintln!("monoplan daemon: offline ({reason})");
        }
        self.resolve_pending_syncs(false);
        self.connected_at = None;
        self.awaiting_pong = false;
        self.schedule_reconnect(reason);
    }

    fn schedule_reconnect(&mut self, reason: &str) {
        // Once per distinct failure, not once per backoff step: a
        // wrong URL or a missing TLS backend should be one line in the
        // log, and the same line is what status replies carry.
        if self.offline_reason.as_deref() != Some(reason) {
            tracing::warn!("offline: {reason}; reconnecting");
            self.offline_reason = Some(reason.to_string());
        }
        tracing::debug!("reconnect in {:?}: {reason}", self.backoff);
        let jitter = Duration::from_millis(rand::random::<u64>() % 250);
        self.next_connect = Instant::now() + self.backoff + jitter;
        self.backoff = (self.backoff * 2).min(BACKOFF_MAX);
        self.publish();
    }

    fn publish(&self) {
        let next = DaemonStatus {
            online: self.is_online(),
            last_synced_at: self.engine.last_synced_at(),
            offline_reason: self.offline_reason.clone(),
            local_only: self.local_only,
            pending: self.has_pending_work(),
            last_contiguous_seq: self.engine.home_replica().last_contiguous_seq(),
            connects: self.status.borrow().connects,
        };
        let prev_sync = self.status.borrow().sync();
        let now_sync = next.sync();
        self.status.send_if_modified(|s| {
            if *s == next {
                false
            } else {
                *s = next;
                true
            }
        });
        if now_sync != prev_sync {
            let _ = self.watchers.send(WatchEvent::Sync(now_sync));
        }
    }

    /// Drain the doc's `AppEvent` queue and, if it held anything, tell
    /// watchers once. A watcher re-fetches its projection, so a burst
    /// of N events is one frame.
    fn notify_if_changed(&mut self) {
        if self.engine.drain_app_events() {
            let _ = self.watchers.send(WatchEvent::Changed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn trim_log_keeps_the_tail_from_a_line_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG_FILE);
        let lines: String = (0..100).map(|i| format!("line {i:03}\n")).collect();
        std::fs::write(&path, &lines).unwrap();

        // Under the cap: untouched.
        trim_log(&path, 10_000, 95);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), lines);

        // Over it: the last 95 bytes land mid-line, so the cut starts
        // at the next whole line. A writer that had the file open in
        // append mode beforehand (a daemon's stderr) carries on at the
        // new end.
        let mut stderr = OpenOptions::new().append(true).open(&path).unwrap();
        trim_log(&path, 500, 95);
        stderr.write_all(b"after\n").unwrap();
        let expected: String = (90..100)
            .map(|i| format!("line {i:03}\n"))
            .chain(["after\n".to_string()])
            .collect();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);

        // No file: nothing made.
        std::fs::remove_file(&path).unwrap();
        trim_log(&path, 500, 95);
        assert!(!path.exists());
    }
}
