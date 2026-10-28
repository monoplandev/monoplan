//! CLI-side sync runtime — a thin tokio-tungstenite adapter that
//! drives the sans-IO `monoplan_core::SyncEngine`.
//!
//! `Session` is the docs-plus-engines handle (one engine per doc in
//! `secrets.toml`, one socket). Two hosts use it:
//! - the daemon (`crate::daemon`) holds one open for its lifetime and
//!   drives the socket itself;
//! - the one-shot shape (`open_with_profile` → mutate via `doc()` →
//!   `flush()`) connects, pulls, pushes and acks inline, then exits.
//!
//! The CLI no longer runs the one-shot shape (commands go through the
//! daemon), but tests do, and it is the exchange a mobile host runs on
//! each wake (`spec/daemon-plan.md`).

use std::collections::BTreeMap;
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

/// One doc's engine plus the CLI-only `last_sync_at` stamp.
pub struct DocState {
    pub(crate) engine: SyncEngine,
    /// In-memory copy of the persisted `last_sync_at` stamp, seeded at
    /// boot and advanced by `mark_synced`, so hosts can report it
    /// without a db read.
    last_synced_at: Option<i64>,
}

/// The profile's docs plus the one socket they share: every doc in
/// `secrets.toml` booted under one peer lease, one engine each
/// (`spec/cli.md` §Workspaces). Hosts route server frames to engines by
/// doc id and drive every engine the same way.
pub struct Session {
    profile: Profile,
    /// `None` for a local-only workspace (`crate::bootstrap`): no
    /// account, so no server to connect to. Hosts skip the socket
    /// entirely and report `local_only` instead of offline.
    server_url: Option<String>,
    /// The Home doc: what an absent `-w` means, and whose sync state
    /// the status replies describe.
    primary: DocId,
    docs: BTreeMap<DocId, DocState>,
    /// Handle for persisting the `last_sync_at` observability stamp — a
    /// clone of the storage the engines own (same connection, see
    /// `SqliteStorage`'s `Clone`). The engine persists the *resume
    /// cursor* itself via the `LocalStorage` trait; this clone exists
    /// only because `last_sync_at` is a CLI-only concern the time-free
    /// engine can't stamp.
    store: SqliteStorage,
    ws: Option<WsStream>,
    /// Exclusive claim on the device's peer id — held for the session's
    /// lifetime so no concurrent invocation can commit under the same
    /// peer (see `crate::peer`). Every doc commits under it: the rule
    /// is one live instance per (doc, peer), and each doc has one.
    _peer: crate::peer::PeerLease,
}

impl Session {
    /// One-shot open on an explicit profile: boot the docs and, with
    /// `sync`, connect and run the initial pull inline. Refused while a
    /// daemon holds the profile. This is the library primitive the
    /// mobile hosts' short exchange is shaped like (`spec/daemon-plan.md`);
    /// the CLI itself no longer uses it — tests do.
    pub async fn open_with_profile(profile: Profile, sync: bool) -> Result<Self, SyncError> {
        // A resident daemon owns the docs for as long as it runs; a
        // one-shot process writing WAL rows beside it would not corrupt
        // anything (the peer lease prevents that) but the daemon's
        // in-memory docs would never see them (spec/daemon-plan.md).
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
        let storage = crate::storage::open_storage(&profile)?;
        let account = storage.read_account()?;
        let primary = account.primary_doc_id;
        // Claim the device's peer id before any doc exists, so every
        // local commit this process makes carries the leased peer
        // (spec/peer-id-plan.md).
        let peer = crate::peer::claim(&profile.dir, &storage)?;
        // `store` shares the engines' connection (see `SqliteStorage`'s
        // `Clone`) — used only for the `last_sync_at` stamp; the engine
        // persists the resume cursor itself.
        let store = storage.clone();

        // Home first, so a missing primary key is the error rather
        // than a half-booted session.
        let mut doc_ids: Vec<DocId> = vec![primary];
        for key in secrets.docs.keys() {
            if let Ok(id) = crate::keystore::parse_doc_id(key)
                && id != primary
            {
                doc_ids.push(id);
            }
        }
        // Local item creation is attributed to the account; a local-only
        // profile has no identity and writes no `created_by`.
        let author = account.identity.as_ref().map(|i| i.account_id.as_str());
        let mut docs = BTreeMap::new();
        for doc_id in doc_ids {
            let state = boot_doc_state(&storage, &secrets, doc_id, peer.peer_id, author)?;
            docs.insert(doc_id, state);
        }

        let mut session = Session {
            profile,
            server_url,
            primary,
            docs,
            store,
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

    /// The enrolled account row (id, email, device), read from the db
    /// on each call; it only changes across a login, which restarts
    /// the daemon.
    pub fn account(&self) -> Result<crate::storage::Account, monoplan_core::StorageError> {
        self.store.read_account()
    }

    pub fn primary_doc_id(&self) -> DocId {
        self.primary
    }

    /// Borrow the Home doc — commands read and mutate via this. Doc's
    /// own methods take `&self` (loro uses interior mutability) so a
    /// shared reference is enough for both reads and writes.
    pub fn doc(&self) -> &Doc {
        self.home().engine.doc()
    }

    /// The doc behind `doc_id`, if this session has it open.
    pub fn doc_for(&self, doc_id: DocId) -> Option<&Doc> {
        self.docs.get(&doc_id).map(|d| d.engine.doc())
    }

    /// Every open doc, Home first.
    pub fn doc_ids(&self) -> Vec<DocId> {
        let mut ids = vec![self.primary];
        ids.extend(self.docs.keys().copied().filter(|id| *id != self.primary));
        ids
    }

    /// The docs as the `-w` resolver sees them (`crate::workspace`).
    pub fn candidates(&self) -> Vec<crate::workspace::Candidate> {
        self.doc_ids()
            .into_iter()
            .map(|doc_id| crate::workspace::Candidate {
                doc_id,
                name: if doc_id == self.primary {
                    None
                } else {
                    self.docs[&doc_id].engine.doc().get_settings().doc_name
                },
            })
            .collect()
    }

    /// The doc a `-w` selector names (`spec/cli.md` §Workspaces).
    pub fn resolve(
        &self,
        selector: Option<&str>,
    ) -> Result<DocId, crate::workspace::WorkspaceError> {
        crate::workspace::resolve(self.primary, &self.candidates(), selector)
    }

    fn home(&self) -> &DocState {
        self.docs
            .get(&self.primary)
            .expect("home doc is always open")
    }

    /// The Home engine: what the host's status describes.
    pub(crate) fn home_engine(&self) -> &SyncEngine {
        &self.home().engine
    }

    pub(crate) fn engines_mut(&mut self) -> impl Iterator<Item = (DocId, &mut SyncEngine)> {
        self.docs.iter_mut().map(|(id, d)| (*id, &mut d.engine))
    }

    /// Persist the docs, push any pending ops, ack the frontier, close
    /// the socket. Local persistence runs first so a network failure
    /// after a mutation can't silently drop the change.
    pub async fn flush(mut self) -> Result<(), SyncError> {
        self.persist_engine_state().await?;

        if let Some(mut ws) = self.ws.take() {
            self.flush_engines();
            drive_until_idle(&mut ws, &mut self).await?;
            // `drive_until_idle` returned with every engine Idle. Any
            // server frame applied during the drive advanced the
            // engine's in-memory frontier but **did not** queue an
            // Ack (gated on `notify_oplog_durable`). Persist now —
            // that writes the doc and calls `notify_oplog_durable`,
            // which queues the Ack — then flush the outbox before
            // closing so the ack actually leaves the wire.
            self.persist_engine_state().await?;
            send_outboxes(&mut ws, &mut self).await?;

            // Best-effort close — server will disconnect on its own
            // if the close frame is lost.
            let _ = ws.close(None).await;

            // We completed an online push/pull/ack exchange — record it.
            for id in self.doc_ids() {
                self.mark_synced(id)?;
            }
        }
        Ok(())
    }

    async fn try_connect_and_pull(&mut self) -> Result<(), SyncError> {
        let mut ws = tokio::time::timeout(CONNECT_TIMEOUT, self.connect())
            .await
            .map_err(|_| SyncError::Ws(format!("connect timed out after {CONNECT_TIMEOUT:?}")))??;
        // The version handshake is per socket, not per doc: run it
        // once here, then bring every doc's engine online.
        ws.send(Message::Binary(
            hello_frame(&engine_options("monoplan-cli")).into(),
        ))
        .await?;
        let first = tokio::time::timeout(CONNECT_TIMEOUT, recv_bytes(&mut ws))
            .await
            .map_err(|_| {
                SyncError::Ws(format!("handshake timed out after {CONNECT_TIMEOUT:?}"))
            })??;
        parse_hello_response(&first).map_err(|e| SyncError::Engine(e.to_string()))?;
        self.handle_connected();
        drive_until_idle(&mut ws, self).await?;
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

    // ---- every-engine operations ----

    /// Socket up and the version handshake done: open every doc.
    pub(crate) fn handle_connected(&mut self) {
        for (_, e) in self.engines_mut() {
            e.handle_connected();
        }
    }

    pub(crate) fn handle_disconnected(&mut self) {
        for (_, e) in self.engines_mut() {
            e.handle_disconnected();
        }
    }

    pub(crate) fn handle_timeout(&mut self) {
        for (_, e) in self.engines_mut() {
            e.handle_timeout();
        }
    }

    /// Tell every engine local mutations may be waiting.
    pub(crate) fn flush_engines(&mut self) {
        for (_, e) in self.engines_mut() {
            e.flush();
        }
    }

    /// Every engine idle: nothing in flight on any doc.
    pub(crate) fn all_idle(&self) -> bool {
        self.docs.values().all(|d| d.engine.is_idle())
    }

    /// Hand a post-handshake server frame to the engine it is about.
    /// A frame for a doc this session does not hold is dropped (the
    /// server only sends frames for docs we opened, so this is a race
    /// with a just-forgotten doc at worst); undecodable bytes go to
    /// Home, which reports the decode error as an event.
    pub(crate) fn route_server_bytes(&mut self, bytes: &[u8]) {
        let target = match monoplan_core::server_frame_doc_id(bytes) {
            Some(id) => DocId(id),
            None => self.primary,
        };
        match self.docs.get_mut(&target) {
            Some(d) => d.engine.handle_server_bytes(bytes),
            None => tracing::debug!("frame for unknown doc {}; dropped", target.0),
        }
    }

    /// Fold every engine's WAL past the hot-path thresholds.
    pub(crate) fn snapshot_if_wal_exceeds(&mut self) -> Result<(), SyncError> {
        for (_, e) in self.engines_mut() {
            e.snapshot_if_wal_exceeds(SNAPSHOT_MAX_WAL_ROWS, SNAPSHOT_MAX_WAL_BYTES)?;
        }
        Ok(())
    }

    /// Fold every engine's WAL unconditionally (clean exit).
    pub(crate) fn force_snapshot(&mut self) -> Result<(), SyncError> {
        for (_, e) in self.engines_mut() {
            e.force_snapshot()?;
        }
        Ok(())
    }

    /// Drain every engine's events, tagged by doc.
    pub(crate) fn drain_events(&mut self) -> Vec<(DocId, Event)> {
        let mut out = Vec::new();
        for (id, e) in self.engines_mut() {
            while let Some(ev) = e.pop_event() {
                out.push((id, ev));
            }
        }
        out
    }

    /// Drain every doc's `AppEvent` queue; true if any held something.
    pub(crate) fn drain_app_events(&mut self) -> bool {
        let mut changed = false;
        for (_, e) in self.engines_mut() {
            while e.pop_app_event().is_some() {
                changed = true;
            }
        }
        changed
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
        for (_, engine) in self.engines_mut() {
            engine.capture_local_ops()?;
            engine.snapshot_if_wal_exceeds(SNAPSHOT_MAX_WAL_ROWS, SNAPSHOT_MAX_WAL_BYTES)?;
            let contiguous = engine.last_contiguous_seq();
            // `notify_oplog_durable` advances the engine's durable
            // frontier and persists the resume cursor itself (via the
            // `LocalStorage` trait).
            engine.notify_oplog_durable(contiguous);
        }
        Ok(())
    }

    /// Stamp "last successful online sync" = now on one doc. Called
    /// only after a confirmed online exchange — never on an offline or
    /// purely local flush — so `monoplan workspace status` reports when
    /// this device actually last reached the server, not when it last
    /// ran a command. Pure observability; nothing in the sync path
    /// reads it.
    pub(crate) fn mark_synced(&mut self, doc_id: DocId) -> Result<(), SyncError> {
        let now = now_millis();
        self.store.write_last_sync_at(doc_id, now)?;
        if let Some(d) = self.docs.get_mut(&doc_id) {
            d.last_synced_at = Some(now);
        }
        Ok(())
    }

    /// Home's last sync stamp: what the status replies carry.
    pub(crate) fn last_synced_at(&self) -> Option<i64> {
        self.home().last_synced_at
    }

    /// The server will not serve `doc_id` to this account any more
    /// (`Event::DocClosed`): drop the engine, the key and the local
    /// rows, so nothing reopens it. Never Home — the caller keeps that
    /// as a fatal engine error.
    pub(crate) fn forget_doc(&mut self, doc_id: DocId) -> Result<(), SyncError> {
        if doc_id == self.primary {
            return Err(SyncError::Engine("refusing to forget the Home doc".into()));
        }
        self.docs.remove(&doc_id);
        let mut secrets = self.profile.read_secrets()?;
        secrets.docs.remove(&doc_id.0.to_string());
        self.profile.write_secrets(&secrets)?;
        self.store.forget_doc(doc_id)?;
        Ok(())
    }
}

/// Boot one doc from its op log + snapshot. A doc with no rows yet
/// (a membership first seen on this device) reconstructs empty; the
/// server's snapshot fills it on open.
fn boot_doc_state(
    storage: &SqliteStorage,
    secrets: &crate::config::Secrets,
    doc_id: DocId,
    peer_id: u64,
    author: Option<&str>,
) -> Result<DocState, SyncError> {
    let dek = dek_for_doc(secrets, doc_id)?;
    let (mut doc, boot_meta) = crate::storage::boot_doc(storage, &dek, doc_id, Some(peer_id))?;
    doc.set_author(author);
    let last_synced_at = storage.read_sync_cursor(doc_id)?.last_sync_at;
    let mut engine = SyncEngine::new(
        doc,
        doc_id,
        dek,
        boot_meta.last_acked_server_seq.0,
        Box::new(storage.clone()),
    );
    engine.seed_boot(&boot_meta);
    // Boot-time threshold check: if a previous run crashed after
    // filling the WAL but before folding it, self-heal now instead
    // of replaying an ever-growing log on every boot.
    engine.snapshot_if_wal_exceeds(SNAPSHOT_MAX_WAL_ROWS, SNAPSHOT_MAX_WAL_BYTES)?;
    Ok(DocState {
        engine,
        last_synced_at,
    })
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

/// Shuffle bytes between the WS and the session's engines until every
/// engine reports `Idle`. Drains the outboxes after every step so a
/// stalled engine can't sit on a frame the server is waiting for.
///
/// Engine `Error` events become `SyncError::Engine` — they're how the
/// engine surfaces handshake rejection and frame-decode failures.
async fn drive_until_idle(ws: &mut WsStream, session: &mut Session) -> Result<(), SyncError> {
    loop {
        send_outboxes(ws, session).await?;
        for (id, event) in session.drain_events() {
            match event {
                Event::Error(msg) => return Err(SyncError::Engine(msg)),
                Event::DocClosed { reason } if id != session.primary => {
                    // One-shot hosts get the same contract as the
                    // daemon: a closed doc is forgotten, not retried.
                    tracing::warn!("doc {} closed: {reason}; forgetting it", id.0);
                    session.forget_doc(id)?;
                }
                Event::DocClosed { reason } => {
                    return Err(SyncError::Engine(format!("home doc closed: {reason}")));
                }
                _ => {}
            }
        }
        if !session.home_engine().is_online() {
            return Err(SyncError::Engine("engine disconnected mid-drive".into()));
        }
        if session.all_idle() {
            // One last sweep: handling earlier bytes might have queued
            // an Ack that hasn't been flushed yet.
            send_outboxes(ws, session).await?;
            return Ok(());
        }
        let bytes = recv_bytes(ws).await?;
        session.route_server_bytes(&bytes);
    }
}

/// Every engine's outbox to the wire.
pub(crate) async fn send_outboxes(
    ws: &mut WsStream,
    session: &mut Session,
) -> Result<(), SyncError> {
    for (_, engine) in session.engines_mut() {
        send_outbox(ws, engine).await?;
    }
    Ok(())
}

pub(crate) async fn send_outbox(
    ws: &mut WsStream,
    engine: &mut SyncEngine,
) -> Result<(), SyncError> {
    while let Some(bytes) = engine.pop_outbox() {
        ws.send(Message::Binary(bytes.into())).await?;
    }
    Ok(())
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
