//! `monoplan daemon` — the resident host (`spec/daemon-plan.md`).
//!
//! Hold a [`Session`] open for the life of the process and drive the
//! sans-IO engine from one `select!` loop over the WebSocket, client
//! requests arriving on a Unix socket, a persist tick, a liveness ping,
//! and shutdown signals.
//!
//! - **Ownership.** The daemon holds `daemon.lock` (a kernel flock,
//!   released on any exit path) for its lifetime. `Session::open`
//!   refuses while it is held, so a one-shot command can never write
//!   WAL rows the resident doc would not see.
//! - **Transport policy** (host-owned per `spec/architecture.md`):
//!   reconnect with capped exponential backoff, a ping every
//!   `PING_INTERVAL` whose missed pong counts as a disconnect (after
//!   laptop sleep the socket is usually dead before the OS says so),
//!   and the engine's `OpenDoc` handshake timeout.
//! - **Persist cadence** mirroring the web host: capture on a ~1 s
//!   tick and right after remote frames apply (so the Ack goes out
//!   against durable bytes), threshold fold on the hot path, full fold
//!   on clean exit.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use monoplan_core::{Event, hello_frame, parse_hello_response};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;

use crate::commands::run_doc;
use crate::config::Profile;
use crate::ipc::{self, Reply, Request, SyncStatus, WatchEvent, read_frame, write_frame};
use crate::sync::{
    SNAPSHOT_MAX_WAL_BYTES, SNAPSHOT_MAX_WAL_ROWS, Session, SyncError, WsStream, connect_ws,
    drain_events, engine_options, send_outbox,
};

/// Socket connect + version handshake budget. Longer than the one-shot
/// CLI's 2 s: the daemon is not holding up a command, and real networks
/// (captive portals, sleepy wifi) take longer than localhost.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Engine-level `OpenDoc` → `DocOpened` budget after the socket is up.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Reconnect backoff: doubles from `BACKOFF_MIN` to `BACKOFF_MAX` with
/// jitter; reset on a successful handshake. While the server is
/// unreachable this *is* the slow poll (`spec/daemon-plan.md`).
const BACKOFF_MIN: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// Liveness ping cadence; a ping sent while the previous pong is still
/// outstanding means the socket is dead.
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// Persist tick: WAL capture + threshold fold + engine timeout check.
const PERSIST_INTERVAL: Duration = Duration::from_secs(1);
/// Events buffered per watcher before it is deemed lagging. A lagging
/// watcher gets one `Changed` instead of the backlog, so this only has
/// to cover a burst between two of its reads.
const WATCH_BUFFER: usize = 64;

const LOCK_FILE: &str = "daemon.lock";

#[derive(Parser, Debug, Default)]
pub struct DaemonArgs {
    #[command(subcommand)]
    pub sub: Option<DaemonSub>,
    /// Started by a client: report readiness as one line on stdout
    /// (`ready`, `already-running`, or `error: …`) and keep running.
    /// Without it the daemon stays attached to the terminal.
    #[arg(long)]
    pub detach: bool,
}

#[derive(Subcommand, Debug)]
pub enum DaemonSub {
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
    /// Contiguous-prefix server seq the engine has applied in memory.
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
        }
    }
}

/// Control handle for an in-process host (tests, and later the tray /
/// GUI hosts that embed the loop instead of spawning it).
pub struct DaemonHandle {
    pub status: watch::Receiver<DaemonStatus>,
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
/// (`monoplan-ffi::Host`, `spec/daemon-plan.md` amendment 2026-10-27).
pub struct Started {
    lock: DaemonLock,
    listener: UnixListener,
    session: Session,
}

/// Claim the profile, bind the socket and boot the doc. `Ok(None)` when
/// another process holds the lock.
pub async fn start(profile: Profile) -> anyhow::Result<Option<Started>> {
    let Some(lock) = try_acquire(&profile.dir)? else {
        return Ok(None);
    };
    let listener = bind(&profile.dir)?;
    let session = Session::open_resident(profile).await?;
    Ok(Some(Started {
        lock,
        listener,
        session,
    }))
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
    let Started {
        lock,
        listener,
        session,
    } = started;
    if session.is_local_only() {
        eprintln!("monoplan daemon: listening; local-only workspace, no server");
    } else {
        eprintln!("monoplan daemon: listening, connecting to server…");
    }
    let (status_tx, status_rx) = watch::channel(DaemonStatus::default());
    let (req_tx, req_rx) = mpsc::channel(16);
    let (watch_tx, _) = broadcast::channel(WATCH_BUFFER);
    tokio::spawn(accept_loop(
        listener,
        req_tx,
        status_rx,
        watch_tx.clone(),
        embedded,
    ));
    let result = Host::new(session, lock, status_tx, req_rx, watch_tx, embedded)
        .run(shutdown)
        .await;
    eprintln!("monoplan daemon: stopped");
    Ok(result?)
}

/// `monoplan daemon` entry point.
pub async fn run(args: DaemonArgs) -> anyhow::Result<()> {
    match args.sub {
        Some(DaemonSub::Stop) => return stop().await,
        Some(DaemonSub::Status) => return status().await,
        Some(DaemonSub::Install) => return crate::service::install().await,
        Some(DaemonSub::Uninstall) => return crate::service::uninstall().await,
        None => {}
    }
    let detach = args.detach;
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

/// Listen on the profile's socket. Holding `daemon.lock` means any
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
    req_tx: mpsc::Sender<Pending>,
    status: watch::Receiver<DaemonStatus>,
    watch_tx: broadcast::Sender<WatchEvent>,
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
        let watch_tx = watch_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_conn(stream, req_tx, status, watch_tx, embedded).await {
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

type Pending = (Request, oneshot::Sender<Reply>);

/// Requests are read by a separate task and handed over a channel, so
/// the `select!` below never cancels a half-read frame (a `read_frame`
/// future dropped between its length prefix and payload would
/// desynchronise the stream). The loop then interleaves replies with
/// watch events on the one write half.
async fn serve_conn(
    stream: UnixStream,
    req_tx: mpsc::Sender<Pending>,
    status: watch::Receiver<DaemonStatus>,
    watch_tx: broadcast::Sender<WatchEvent>,
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
                        events = Some(watch_tx.subscribe());
                        Reply::Watching {
                            sync: status.borrow().sync(),
                        }
                    }
                    other => {
                        let (tx, rx) = oneshot::channel();
                        if req_tx.send((other, tx)).await.is_err() {
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
    let session = Session::open_resident(profile).await?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (status_tx, status_rx) = watch::channel(DaemonStatus::default());
    let (req_tx, req_rx) = mpsc::channel(16);
    let (watch_tx, _) = broadcast::channel(WATCH_BUFFER);
    tokio::spawn(accept_loop(
        listener,
        req_tx,
        status_rx.clone(),
        watch_tx.clone(),
        embedded,
    ));
    let task = tokio::spawn(
        Host::new(session, lock, status_tx, req_rx, watch_tx, embedded).run(shutdown_rx),
    );
    Ok(DaemonHandle {
        status: status_rx,
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

/// Socket + per-connection version handshake. The engine's own
/// `OpenDoc` exchange starts in `handle_connected` afterwards. A free
/// function (not `&self`) so the host's `select!` future stays `Send`.
async fn handshake(profile: &Profile, server_url: &str) -> Result<WsStream, SyncError> {
    let mut ws = connect_ws(profile, server_url).await?;
    ws.send(Message::Binary(hello_frame(&engine_options(
        "monoplan-daemon",
    ))))
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
    session: Session,
    _lock: DaemonLock,
    /// Watched on the persist tick: if the profile dir is purged from
    /// under us (logout, a test's tempdir) there is nothing left to own.
    lock_path: std::path::PathBuf,
    socket_path: std::path::PathBuf,
    status: watch::Sender<DaemonStatus>,
    requests: mpsc::Receiver<Pending>,
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
        session: Session,
        lock: DaemonLock,
        status: watch::Sender<DaemonStatus>,
        requests: mpsc::Receiver<Pending>,
        watchers: broadcast::Sender<WatchEvent>,
        embedded: bool,
    ) -> Self {
        let dir = session.profile_dir().to_path_buf();
        let local_only = session.is_local_only();
        Host {
            session,
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

                Some((req, reply)) = self.requests.recv() => {
                    if self.on_request(req, reply).await? {
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
        self.session.persist_engine_state().await?;
        self.session.engine.force_snapshot()?;
        if let Some(mut ws) = self.ws.take() {
            let _ = ws.close(None).await;
        }
        let _ = std::fs::remove_file(&self.socket_path);
        Ok(())
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
            Request::Doc(cmd) => {
                let mut buf = Vec::new();
                let result = run_doc(cmd, self.session.doc(), &mut buf);
                // Whatever the command committed is durable before the
                // client hears back, and on the wire right after.
                self.session.persist_engine_state().await?;
                self.session.engine.flush();
                self.pump().await;
                self.notify_if_changed();
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
                self.session.persist_engine_state().await?;
                if self.local_only {
                    // Nothing to push to; say so now rather than park
                    // the request behind a connect that never starts.
                    Reply::Synced {
                        sync: self.sync_status(),
                    }
                } else if self.ws.is_some() && self.session.engine.is_idle() {
                    self.session.engine.flush();
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
            Request::Account => match self.session.account() {
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
        let Some((profile, server_url)) = self.session.connect_params() else {
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
                self.session.engine.handle_connected();
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
            last_synced_at: self.session.last_synced_at(),
            offline_reason: self.offline_reason.clone(),
            local_only: self.local_only,
        };
        for tx in self.pending_syncs.drain(..) {
            let _ = tx.send(Reply::Synced { sync: sync.clone() });
        }
    }

    fn sync_status(&self) -> SyncStatus {
        SyncStatus {
            online: self.is_online(),
            last_synced_at: self.session.last_synced_at(),
            offline_reason: self.offline_reason.clone(),
            local_only: self.local_only,
        }
    }

    fn is_online(&self) -> bool {
        self.ws.is_some() && self.session.engine.is_online()
    }

    async fn on_ws_message(
        &mut self,
        msg: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    ) {
        match msg {
            Some(Ok(Message::Binary(bytes))) => {
                self.session.engine.handle_server_bytes(&bytes);
                // Remote updates are WAL-appended by the engine as they
                // apply; persisting now advances the durable cursor,
                // which is what queues the Ack `pump` then sends.
                if let Err(e) = self.session.persist_engine_state().await {
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
        self.session.persist_engine_state().await?;
        // Hot-path fold uses the real thresholds; exit folds everything.
        self.session
            .engine
            .snapshot_if_wal_exceeds(SNAPSHOT_MAX_WAL_ROWS, SNAPSHOT_MAX_WAL_BYTES)?;
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
                self.session.engine.handle_timeout();
            }
            self.session.engine.flush();
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
        match ws.send(Message::Ping(Vec::new())).await {
            Ok(()) => self.awaiting_pong = true,
            Err(e) => self.disconnect(&format!("ping failed: {e}")),
        }
    }

    /// Drain the engine's outbox to the wire and act on its events.
    /// Called after every engine interaction while a socket is open.
    async fn pump(&mut self) {
        if let Some(ws) = self.ws.as_mut()
            && let Err(e) = send_outbox(ws, &mut self.session.engine).await
        {
            self.disconnect(&format!("send failed: {e}"));
            return;
        }
        for event in drain_events(&mut self.session.engine) {
            match event {
                Event::ConnStateChanged { online: false } => {
                    // Engine already went Disconnected (rejected
                    // handshake, fatal frame); drop the socket to match.
                    self.drop_socket("engine disconnected");
                }
                Event::ConnStateChanged { online: true } => {}
                Event::PulledInitial => {
                    if let Err(e) = self.session.mark_synced() {
                        tracing::warn!("mark_synced: {e}");
                    }
                    if !self.pending_syncs.is_empty() {
                        self.session.engine.flush();
                        self.resolve_pending_syncs(true);
                    }
                }
                Event::Pushed => {
                    if let Err(e) = self.session.mark_synced() {
                        tracing::warn!("mark_synced: {e}");
                    }
                }
                Event::FrontierAdvanced { .. } => {}
                Event::Error(msg) => {
                    tracing::warn!("engine: {msg}");
                    self.disconnect(&format!("engine error: {msg}"));
                }
                // The daemon syncs the Home doc only, and Home is
                // non-leavable, so this means the server refused the
                // account's own primary doc — a server-side fault, not
                // something a reconnect loop will fix quickly. Treat
                // it like any fatal engine error: drop the socket and
                // let the ordinary backoff pace the retries.
                Event::DocClosed { reason } => {
                    tracing::warn!("engine: home doc closed: {reason}");
                    self.disconnect(&format!("doc closed: {reason}"));
                }
            }
        }
        self.publish();
    }

    /// Host-initiated disconnect: tell the engine, drop the socket,
    /// schedule a reconnect.
    fn disconnect(&mut self, reason: &str) {
        self.session.engine.handle_disconnected();
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
            last_synced_at: self.session.last_synced_at(),
            offline_reason: self.offline_reason.clone(),
            local_only: self.local_only,
            last_contiguous_seq: self.session.engine.last_contiguous_seq(),
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
    /// watchers once. The per-event payloads are for in-process
    /// mirrors; over IPC a watcher re-fetches its projection, so a
    /// burst of N events is one frame.
    fn notify_if_changed(&mut self) {
        let mut changed = false;
        while self.session.engine.pop_app_event().is_some() {
            changed = true;
        }
        if changed {
            let _ = self.watchers.send(WatchEvent::Changed);
        }
    }
}
