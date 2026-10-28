//! CLI-side sync runtime — a thin tokio-tungstenite adapter that
//! drives the sans-IO `monoplan_core::SyncEngine`.
//!
//! `Session` is the doc-plus-engine handle. Two hosts use it:
//! - the daemon (`crate::daemon`) holds one open for its lifetime and
//!   drives the socket itself;
//! - the one-shot shape (`open_with_profile` → mutate via `doc()` →
//!   `flush()`) connects, pulls, pushes and acks inline, then exits.
//!
//! The CLI no longer runs the one-shot shape (commands go through the
//! daemon), but tests do, and it is the exchange a mobile host runs on
//! each wake (`spec/daemon-plan.md`).

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use http::header::AUTHORIZATION;
use monoplan_core::{
    Doc, DocId, EngineOptions, Event, SyncEngine, hello_frame, parse_hello_response,
};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::config::{ConfigError, Profile};
use crate::keystore::{KeystoreError, dek_for_doc};
use crate::storage::SqliteStorage;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// WAL fold thresholds (spec/vv-wal-separation.md). Evaluated after
/// every command's WAL append and once at boot (so an interrupted
/// snapshot attempt self-heals). 100 rows bounds boot replay; the byte
/// cap is a safety net against a few huge rows (e.g. bulk imports) —
/// initial value pending real measurement.
pub(crate) const SNAPSHOT_MAX_WAL_ROWS: u64 = 100;
pub(crate) const SNAPSHOT_MAX_WAL_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Keystore(#[from] KeystoreError),
    #[error(transparent)]
    Doc(#[from] monoplan_core::DocError),
    #[error(transparent)]
    Db(#[from] crate::storage::DbError),
    #[error(transparent)]
    Boot(#[from] monoplan_core::BootError),
    #[error("storage: {0}")]
    Storage(#[from] monoplan_core::StorageError),
    #[error(transparent)]
    Peer(#[from] crate::peer::PeerError),
    #[error(
        "monoplan daemon is running for this profile; one-shot commands are disabled while it holds the doc (spec/daemon-plan.md)"
    )]
    DaemonRunning,
    #[error("ws: {0}")]
    Ws(String),
    #[error("engine: {0}")]
    Engine(String),
    #[error("local-only workspace: no account to sync to")]
    LocalOnly,
}

impl From<tokio_tungstenite::tungstenite::Error> for SyncError {
    fn from(e: tokio_tungstenite::tungstenite::Error) -> Self {
        SyncError::Ws(e.to_string())
    }
}

pub(crate) type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// One-shot per-command sync handle.
pub struct Session {
    profile: Profile,
    /// `None` for a local-only workspace (`crate::bootstrap`): no
    /// account, so no server to connect to. Hosts skip the socket
    /// entirely and report `local_only` instead of offline.
    server_url: Option<String>,
    doc_id: DocId,
    /// Handle for persisting the `last_sync_at` observability stamp — a
    /// clone of the storage the engine owns (same connection, see
    /// `SqliteStorage`'s `Clone`). The engine persists the *resume
    /// cursor* itself via the `LocalStorage` trait; this clone exists
    /// only because `last_sync_at` is a CLI-only concern the time-free
    /// engine can't stamp.
    store: SqliteStorage,
    /// In-memory copy of the persisted `last_sync_at` stamp, seeded at
    /// boot and advanced by `mark_synced`, so hosts can report it
    /// without a db read.
    last_synced_at: Option<i64>,
    pub(crate) engine: SyncEngine,
    ws: Option<WsStream>,
    /// Exclusive claim on the doc's peer id — held for the session's
    /// lifetime so no concurrent invocation can commit under the same
    /// peer (see `crate::peer`). Dropping the session releases it.
    _peer: crate::peer::PeerLease,
}

impl Session {
    /// One-shot open on an explicit profile: boot the doc and, with
    /// `sync`, connect and run the initial pull inline. Refused while a
    /// daemon holds the profile. This is the library primitive the
    /// mobile hosts' short exchange is shaped like (`spec/daemon-plan.md`);
    /// the CLI itself no longer uses it — tests do.
    pub async fn open_with_profile(profile: Profile, sync: bool) -> Result<Self, SyncError> {
        // A resident daemon owns the doc for as long as it runs; a
        // one-shot process writing WAL rows beside it would not corrupt
        // anything (the peer lease prevents that) but the daemon's
        // in-memory doc would never see them (spec/daemon-plan.md).
        if crate::daemon::is_running(&profile.dir).map_err(crate::peer::PeerError::Io)? {
            return Err(SyncError::DaemonRunning);
        }
        Self::open_inner(profile, sync).await
    }

    /// The daemon host's open: no daemon guard (it *is* the daemon) and
    /// never an inline connect — the host loop owns the socket.
    pub(crate) async fn open_resident(profile: Profile) -> Result<Self, SyncError> {
        Self::open_inner(profile, false).await
    }

    async fn open_inner(profile: Profile, sync: bool) -> Result<Self, SyncError> {
        let secrets = profile.read_secrets()?;
        // `config.toml` exists once there is an account; a local-only
        // workspace has no server to name.
        let server_url = if secrets.has_account() {
            Some(profile.read_config()?.server_url)
        } else {
            None
        };
        // Boot the doc from the op log + snapshot. A fresh profile (no
        // snapshot yet) reconstructs an empty doc — the same healing
        // behaviour the old file-based `read_doc` NotFound branch had.
        let storage = crate::storage::open_storage(&profile)?;
        let account = storage.read_account()?;
        let doc_id = account.primary_doc_id;
        let dek = dek_for_doc(&secrets, doc_id)?;
        // Claim the device's peer id before the doc exists, so every
        // local commit this process makes carries the leased peer
        // (spec/peer-id-plan.md).
        let peer = crate::peer::claim(&profile.dir, &storage)?;
        let (doc, boot_meta) =
            crate::storage::boot_doc(&storage, &dek, doc_id, Some(peer.peer_id))?;
        // `store` shares the engine's connection (see `SqliteStorage`'s
        // `Clone`) — used only for the `last_sync_at` stamp; the engine
        // persists the resume cursor itself.
        let store = storage.clone();
        let last_synced_at = store.read_sync_cursor(doc_id)?.last_sync_at;

        let mut engine = SyncEngine::new(
            doc,
            doc_id,
            dek,
            boot_meta.last_acked_server_seq.0,
            Box::new(storage),
        );
        engine.seed_boot(&boot_meta);
        // Boot-time threshold check: if a previous run crashed after
        // filling the WAL but before folding it, self-heal now instead
        // of replaying an ever-growing log on every boot.
        engine.snapshot_if_wal_exceeds(SNAPSHOT_MAX_WAL_ROWS, SNAPSHOT_MAX_WAL_BYTES)?;

        let mut session = Session {
            profile,
            server_url,
            doc_id,
            store,
            last_synced_at,
            engine,
            ws: None,
            _peer: peer,
        };

        if !sync || session.is_local_only() {
            return Ok(session);
        }

        match session.try_connect_and_pull().await {
            Ok(()) => Ok(session),
            Err(e) => {
                eprintln!("offline — sync deferred ({e})");
                session.ws = None;
                Ok(session)
            }
        }
    }

    pub fn is_online(&self) -> bool {
        self.ws.is_some()
    }

    /// No account behind this workspace: nothing ever connects.
    pub fn is_local_only(&self) -> bool {
        self.server_url.is_none()
    }

    /// Borrow the local doc — commands read and mutate via this. Doc's
    /// own methods take `&self` (loro uses interior mutability) so a
    /// shared reference is enough for both reads and writes.
    /// The enrolled account row (id, email, device), read from the db
    /// on each call; it only changes across a login, which restarts
    /// the daemon.
    pub fn account(&self) -> Result<crate::storage::Account, monoplan_core::StorageError> {
        self.store.read_account()
    }

    pub fn doc(&self) -> &Doc {
        self.engine.doc()
    }

    /// Persist the doc, push any pending ops, ack the frontier, close
    /// the socket. Local persistence runs first so a network failure
    /// after a mutation can't silently drop the change.
    pub async fn flush(mut self) -> Result<(), SyncError> {
        self.persist_engine_state().await?;

        if let Some(mut ws) = self.ws.take() {
            self.engine.flush();
            drive_until_idle(&mut ws, &mut self.engine).await?;
            // `drive_until_idle` returned with the engine Idle. Any
            // server frame applied during the drive advanced the
            // engine's in-memory frontier but **did not** queue an
            // Ack (gated on `notify_oplog_durable`). Persist now —
            // that writes the doc and calls `notify_oplog_durable`,
            // which queues the Ack — then flush the outbox before
            // closing so the ack actually leaves the wire.
            self.persist_engine_state().await?;
            send_outbox(&mut ws, &mut self.engine).await?;

            // Best-effort close — server will disconnect on its own
            // if the close frame is lost.
            let _ = ws.close(None).await;

            // We completed an online push/pull/ack exchange — record it.
            self.mark_synced()?;
        }
        Ok(())
    }

    async fn try_connect_and_pull(&mut self) -> Result<(), SyncError> {
        let mut ws = tokio::time::timeout(CONNECT_TIMEOUT, self.connect())
            .await
            .map_err(|_| SyncError::Ws(format!("connect timed out after {CONNECT_TIMEOUT:?}")))??;
        // The version handshake is per socket, not per doc: run it
        // once here, then bring the doc's engine online.
        ws.send(Message::Binary(hello_frame(&engine_options(
            "monoplan-cli",
        ))))
        .await?;
        let first = tokio::time::timeout(CONNECT_TIMEOUT, recv_bytes(&mut ws))
            .await
            .map_err(|_| {
                SyncError::Ws(format!("handshake timed out after {CONNECT_TIMEOUT:?}"))
            })??;
        parse_hello_response(&first).map_err(|e| SyncError::Engine(e.to_string()))?;
        self.engine.handle_connected();
        drive_until_idle(&mut ws, &mut self.engine).await?;
        self.ws = Some(ws);
        Ok(())
    }

    async fn connect(&self) -> Result<WsStream, SyncError> {
        let server_url = self.server_url.as_deref().ok_or(SyncError::LocalOnly)?;
        connect_ws(&self.profile, server_url).await
    }

    pub(crate) fn profile_dir(&self) -> &std::path::Path {
        &self.profile.dir
    }

    /// Borrow what a host needs to open its own socket (the daemon
    /// does this from a `select!` loop without holding `&Session`).
    /// `None` for a local-only workspace.
    pub(crate) fn connect_params(&self) -> Option<(&Profile, &str)> {
        self.server_url.as_deref().map(|url| (&self.profile, url))
    }

    pub(crate) async fn persist_engine_state(&mut self) -> Result<(), SyncError> {
        // Capture any locally-committed mutations into a durable WAL
        // row first — on disk before any Ack leaves the wire. Then fold
        // the WAL into a snapshot if it crossed the threshold; this is
        // independent of sync state (unsent ops survive folding — the
        // push derives them from `server_known_vv`). `SqliteStorage` is
        // synchronously durable, so by the time these return every
        // applied seq is on disk and we can tell the engine "everything
        // up to `last_contiguous_seq` is durable" — which advances the
        // durable cursor and queues an Ack. Callers needing the ack on
        // the wire (`flush`) must `send_outbox` next.
        self.engine.capture_local_ops()?;
        self.engine
            .snapshot_if_wal_exceeds(SNAPSHOT_MAX_WAL_ROWS, SNAPSHOT_MAX_WAL_BYTES)?;
        let contiguous = self.engine.last_contiguous_seq();
        // `notify_oplog_durable` advances the engine's durable frontier and
        // persists the resume cursor itself (via the `LocalStorage`
        // trait). The CLI no longer reads it back or re-writes it.
        self.engine.notify_oplog_durable(contiguous);
        Ok(())
    }

    /// Stamp "last successful online sync" = now. Called only after a
    /// confirmed online exchange — never on an offline or purely local
    /// flush — so `monoplan workspace status` reports when this device actually last
    /// reached the server, not when it last ran a command. Pure
    /// observability; nothing in the sync path reads it.
    pub(crate) fn mark_synced(&mut self) -> Result<(), SyncError> {
        let now = now_millis();
        self.store.write_last_sync_at(self.doc_id, now)?;
        self.last_synced_at = Some(now);
        Ok(())
    }

    pub(crate) fn last_synced_at(&self) -> Option<i64> {
        self.last_synced_at
    }
}

/// Open an authenticated WebSocket to the profile's server. The
/// per-connection version handshake is the caller's next step.
pub(crate) async fn connect_ws(profile: &Profile, server_url: &str) -> Result<WsStream, SyncError> {
    let url = ws_url(server_url);
    let mut req = url
        .into_client_request()
        .map_err(|e| SyncError::Ws(e.to_string()))?;
    let secrets = profile.read_secrets()?;
    req.headers_mut().insert(
        AUTHORIZATION,
        format!("Bearer {}", secrets.device_token()?)
            .parse()
            .map_err(|_| SyncError::Ws("invalid bearer header".into()))?,
    );
    let (ws, _) = tokio_tungstenite::connect_async(req).await?;
    Ok(ws)
}

pub(crate) fn engine_options(client_name: &str) -> EngineOptions {
    EngineOptions {
        client_name: client_name.into(),
        client_version: env!("CARGO_PKG_VERSION").into(),
    }
}

/// Shuffle bytes between the WS and the engine until the engine
/// reports `Idle`. Drains the outbox after every step so a stalled
/// engine can't sit on a frame the server is waiting for.
///
/// Engine `Error` events become `SyncError::Engine` — they're how the
/// engine surfaces handshake rejection and frame-decode failures.
async fn drive_until_idle(ws: &mut WsStream, engine: &mut SyncEngine) -> Result<(), SyncError> {
    loop {
        send_outbox(ws, engine).await?;
        drain_engine_errors(engine)?;
        if !engine.is_online() {
            return Err(SyncError::Engine("engine disconnected mid-drive".into()));
        }
        if engine.is_idle() {
            // One last sweep: handling earlier bytes might have queued
            // an Ack that hasn't been flushed yet.
            send_outbox(ws, engine).await?;
            return Ok(());
        }
        let bytes = recv_bytes(ws).await?;
        engine.handle_server_bytes(&bytes);
    }
}

pub(crate) async fn send_outbox(
    ws: &mut WsStream,
    engine: &mut SyncEngine,
) -> Result<(), SyncError> {
    while let Some(bytes) = engine.pop_outbox() {
        ws.send(Message::Binary(bytes)).await?;
    }
    Ok(())
}

fn drain_engine_errors(engine: &mut SyncEngine) -> Result<(), SyncError> {
    for event in drain_events(engine) {
        if let Event::Error(msg) = event {
            return Err(SyncError::Engine(msg));
        }
    }
    Ok(())
}

pub(crate) fn drain_events(engine: &mut SyncEngine) -> Vec<Event> {
    let mut out = Vec::new();
    while let Some(ev) = engine.pop_event() {
        out.push(ev);
    }
    out
}

pub(crate) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn ws_url(server_url: &str) -> String {
    let trimmed = server_url.trim_end_matches('/');
    let base = if let Some(rest) = trimmed.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        trimmed.to_string()
    };
    format!("{base}/api/sync")
}

pub(crate) async fn recv_bytes_timeout(
    ws: &mut WsStream,
    timeout: Duration,
) -> Result<Option<Vec<u8>>, SyncError> {
    loop {
        let next = tokio::time::timeout(timeout, ws.next()).await;
        let Some(msg) = (match next {
            Ok(msg) => msg,
            Err(_) => return Ok(None),
        }) else {
            return Err(SyncError::Ws("stream closed".into()));
        };
        match msg? {
            Message::Binary(bytes) => return Ok(Some(bytes.to_vec())),
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => return Err(SyncError::Ws("server closed".into())),
            other => return Err(SyncError::Ws(format!("unexpected ws frame: {other:?}"))),
        }
    }
}

async fn recv_bytes(ws: &mut WsStream) -> Result<Vec<u8>, SyncError> {
    loop {
        if let Some(bytes) = recv_bytes_timeout(ws, Duration::from_secs(365 * 24 * 60 * 60)).await?
        {
            return Ok(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `wss://` needs a TLS backend compiled into tokio-tungstenite;
    /// without one `connect_async` opens the TCP connection and then
    /// fails with `TlsFeatureNotEnabled`, which is how the first hosted
    /// server (https) silently never synced. A local listener that
    /// accepts and hangs up stands in for a server: with TLS present
    /// the client gets as far as a handshake and is cut off.
    #[tokio::test]
    async fn wss_has_a_tls_backend() {
        use tokio_tungstenite::tungstenite::{Error, error::UrlError};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                drop(stream);
            }
        });
        let err = tokio_tungstenite::connect_async(format!("wss://127.0.0.1:{port}/api/sync"))
            .await
            .expect_err("the listener hangs up");
        assert!(
            !matches!(err, Error::Url(UrlError::TlsFeatureNotEnabled)),
            "TLS backend missing: {err}"
        );
    }

    #[test]
    fn ws_url_strips_http_scheme() {
        assert_eq!(
            ws_url("http://localhost:8080"),
            "ws://localhost:8080/api/sync"
        );
        assert_eq!(
            ws_url("https://monoplan.example/"),
            "wss://monoplan.example/api/sync"
        );
        assert_eq!(
            ws_url("ws://localhost:9000"),
            "ws://localhost:9000/api/sync"
        );
    }
}
