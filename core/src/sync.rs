//! Sans-IO sync engine: the protocol state machine in shared Rust.
//!
//! The engine owns no socket, no clock, no thread. The caller (CLI's
//! tokio adapter, the browser's `WebSocket`, mobile's `URLSession` /
//! OkHttp) feeds it transport events via `handle_*` and drains
//! ready-to-send frame bytes via `pop_outbox()` and engine events via
//! `pop_event()`.
//!
//! Two independent responsibilities meet here
//! (`spec/vv-wal-separation.md`):
//!
//!   - **Local crash recovery** — every commit (local mutation or
//!     applied remote op) is captured into the encrypted WAL through
//!     the [`LocalStorage`] trait; when the WAL crosses the host's
//!     threshold the engine folds it into a fresh full-history
//!     snapshot and prunes the folded prefix. This runs regardless of
//!     online/sync status.
//!   - **Outbound sync** — what needs uploading is derived from Loro
//!     history against `server_known_vv` (the operations proven to
//!     exist in the server op stream), never from which WAL rows still
//!     exist. At most one push is in flight at a time, durably
//!     recorded with a `push_id` before its first send so a crash
//!     between server insert and client ack retries the exact same
//!     push and the server deduplicates.
//!
//! See `spec/architecture.md` for the client-boundary rationale. The
//! state diagram is reproduced below:
//!
//! ```text
//!                          flush w/ pending
//!   Disconnected           ┌───────────────┐
//!        │                 │               ▼
//!   handle_connected       │           Pushing ─── on OpsAck ──┐
//!        ▼                 │             │  ▲                   │
//!       Hello              │       mutated │  │ ack arrived,    │
//!        │                 │       mid-push│  │ ack arrived &   │
//!     HelloAck             │               ▼  │ nothing more    │
//!        ▼                 │       PushingDirty                 │
//!     Pulling ── complete ─▶ Idle ◀──────────────────────────── ┘
//! ```

use std::collections::VecDeque;

use loro::VersionVector;
use monoplan_protocol::{
    ClientFrame, EncryptedBlob, Hello, HelloAck, HelloRejected, PROTOCOL_VERSION, PushBlob,
    ServerFrame, StoredBlob,
};
use serde::Serialize;

use crate::crypto::Dek;
use crate::doc::Doc;
use crate::storage::{
    BootMeta, DocId, InFlightPush, LocalSeq, LocalStorage, PushId, RemoteWalRow, ServerSeq,
    StorageError,
};

/// `Box<dyn LocalStorage>` with a `Send` bound on native targets only.
/// Wasm runs single-threaded and `JsValue`-holding impls are `!Send` by
/// construction, so the bound flips off there. Engine call sites use
/// `self.storage.method()` regardless.
#[cfg(not(target_arch = "wasm32"))]
pub type DynStorage = Box<dyn LocalStorage + Send>;
#[cfg(target_arch = "wasm32")]
pub type DynStorage = Box<dyn LocalStorage>;

/// Engine-emitted notification, drained via `pop_event()`. None of
/// these are fatal on their own — the caller decides whether to
/// disconnect on `Error`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// `online=true` after `handle_connected`; `false` after the engine
    /// transitions to `Disconnected` (caller-driven, rejected handshake,
    /// or fatal frame error).
    ConnStateChanged { online: bool },
    /// Initial pull (`PullOps` → terminal `OpsBatch{complete: true}`)
    /// finished. Catch-up done; from here on, broadcasts deliver peer
    /// ops live.
    PulledInitial,
    /// Our own `PushOps` was acked and `server_known_vv` advanced.
    Pushed,
    /// The contiguous-prefix seq the engine has applied advanced.
    /// Useful for callers persisting `last_acked_seq` between sessions.
    FrontierAdvanced { seq: u64 },
    /// Recoverable error — the caller may choose to disconnect or just
    /// log and continue. Fatal handshake errors come paired with a
    /// `ConnStateChanged { online: false }` so the caller knows the
    /// engine is already back to `Disconnected`.
    Error(String),
}

/// Identity advertised in the `Hello` frame. Set once at construction.
#[derive(Debug, Clone)]
pub struct EngineOptions {
    pub client_name: String,
    pub client_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnState {
    Disconnected,
    Hello,
    Pulling,
    /// Snapshot bootstrap path: we've received `SnapshotRequired`,
    /// emitted `PullSnapshot`, and are waiting for the `Snapshot`
    /// frame. Cold path — only entered when the server has compacted
    /// past our cursor (or we're a fresh device after compaction).
    Bootstrapping,
    Idle,
    Pushing,
    PushingDirty,
}

/// In-memory view of the durable in-flight push record.
struct InFlightState {
    push_id: PushId,
    payload: monoplan_protocol::EncryptedBlob,
    /// Decoded `to_vv`: the oplog VV captured at export time. Merged
    /// into `server_known_vv` on ack — merged, never assigned, because
    /// remote updates may advance other peers' ranges mid-flight.
    to_vv: VersionVector,
}

pub struct SyncEngine {
    doc: Doc,
    dek: Dek,
    /// Identifies which doc this engine syncs. Passed verbatim to every
    /// `storage.*` call so the trait can scope rows by doc (matters
    /// once multi-doc sharing lands; today every engine only ever
    /// drives one doc).
    doc_id: DocId,
    /// Per-doc local persistence (`SqliteStorage` on the CLI,
    /// `IdbStorage` on web). Load-bearing: every commit is WAL-captured
    /// through it, snapshots fold through it, and the sync cursors
    /// (`server_known_vv`, `last_acked_server_seq`, the in-flight push)
    /// persist through it.
    storage: DynStorage,
    /// The Loro operations proven to exist in the server op stream:
    /// merged from every applied server blob's declared range and from
    /// each acked push's `to_vv`. The outbound delta is
    /// `Updates(server_known_vv)`. Persisted alongside every advance;
    /// seeded from `BootMeta`.
    server_known_vv: VersionVector,
    /// The one in-flight push, mirroring the durable record. `Some`
    /// while a push awaits ack — including across a disconnect (the
    /// durable record survives, and reconnect re-sends the same
    /// `push_id` after the pull completes).
    in_flight: Option<InFlightState>,
    /// Highest `local_seq` the storage has assigned for this doc.
    /// Seeded from `BootMeta` and advanced by every WAL append. The
    /// snapshot cutoff.
    last_local_seq: LocalSeq,
    /// Surviving WAL rows / payload bytes past the last snapshot —
    /// the snapshot-threshold statistics. Seeded from `BootMeta`,
    /// bumped on append, reset on fold. `wal_bytes` counts ciphertext +
    /// nonce, matching `BootState::wal_bytes`.
    wal_rows: u64,
    wal_bytes: u64,
    opts: EngineOptions,
    state: ConnState,
    /// Contiguous-prefix seq we've applied in memory. Advances
    /// synchronously inside `apply_remote_ops` / `OpsAck` / `Snapshot`.
    /// Server seqs are dense and delivered in order, so this equals the
    /// maximum seq seen. **Not** the value we ack — the engine only
    /// ships an Ack for a seq the host has confirmed durable via
    /// `notify_oplog_durable`.
    last_contiguous_seq: u64,
    /// The doc's history epoch as reported by the server on the last
    /// `HelloAck` (`spec/storage.md` §Epochs). `None` until the first
    /// successful handshake. Display-only today.
    server_epoch: Option<u64>,
    /// Contiguous-prefix seq the host has confirmed locally durable
    /// (encrypted WAL row committed for the bytes covering this seq).
    /// `<= last_contiguous_seq`. This is the value the engine sends
    /// in `Ack { last_acked_seq }`, and the value callers persist
    /// between sessions as the resume cursor (`PullOps`'s
    /// `since_seq`). Advances only via `notify_oplog_durable`.
    last_durable_seq: u64,
    /// Highest seq we've already shipped in an `Ack`. Lets us coalesce:
    /// queue an `Ack` only when `last_durable_seq` overtakes this.
    last_sent_ack: u64,
    outbox: VecDeque<Vec<u8>>,
    events: VecDeque<Event>,
}

fn blob_len(blob: &monoplan_protocol::EncryptedBlob) -> u64 {
    (blob.ciphertext.len() + blob.nonce.len()) as u64
}

/// Seal an encoded Loro version vector under the DEK — the
/// `PushSnapshot.up_to_vv` envelope. The server stores and returns it
/// without reading it (`spec/sync-protocol.md` §"Snapshot orchestration").
fn seal_vv(dek: &Dek, vv: &VersionVector) -> Result<EncryptedBlob, String> {
    let (ciphertext, nonce) = dek.seal(&vv.encode()).map_err(|e| e.to_string())?;
    Ok(EncryptedBlob {
        nonce: nonce.to_vec(),
        ciphertext,
    })
}

/// Inverse of [`seal_vv`]: the `SnapshotRequest.base.vv` a producer
/// starts its delta from.
fn open_vv(dek: &Dek, blob: &EncryptedBlob) -> Result<VersionVector, String> {
    let plaintext = dek
        .open(&blob.ciphertext, &blob.nonce)
        .map_err(|e| e.to_string())?;
    VersionVector::decode(&plaintext).map_err(|e| e.to_string())
}

impl SyncEngine {
    /// Build a fresh engine. `last_acked_seq` is the persisted
    /// durable-prefix from the previous session — used as `since_seq`
    /// in the initial pull and as the floor for `last_durable_seq`.
    /// `storage` is mandatory: WAL capture, snapshots, and every sync
    /// cursor flow through it. Hosts with persisted state should
    /// follow up with [`seed_boot`](Self::seed_boot) (or the individual
    /// seed setters) before going online.
    pub fn new(
        doc: Doc,
        doc_id: DocId,
        dek: Dek,
        last_acked_seq: u64,
        opts: EngineOptions,
        storage: DynStorage,
    ) -> Self {
        Self {
            doc,
            dek,
            doc_id,
            storage,
            server_known_vv: VersionVector::default(),
            in_flight: None,
            last_local_seq: LocalSeq(0),
            wal_rows: 0,
            wal_bytes: 0,
            opts,
            state: ConnState::Disconnected,
            last_contiguous_seq: last_acked_seq,
            server_epoch: None,
            last_durable_seq: last_acked_seq,
            last_sent_ack: last_acked_seq,
            outbox: VecDeque::new(),
            events: VecDeque::new(),
        }
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    pub fn doc_mut(&mut self) -> &mut Doc {
        &mut self.doc
    }

    /// Seed every persisted cursor from a [`BootMeta`] in one call —
    /// the host-side pairing of `boot_doc`. Call once right after
    /// construction, before going online.
    pub fn seed_boot(&mut self, meta: &BootMeta) {
        self.set_last_local_seq(meta.last_local_seq);
        self.seed_wal_stats(meta.wal_rows, meta.wal_bytes);
        self.seed_server_known_vv(&meta.server_known_vv);
        if let Some(push) = &meta.in_flight_push {
            self.seed_in_flight_push(push.clone());
        }
    }

    /// Seed the highest `local_seq` the storage has assigned for this
    /// doc. Thereafter the engine maintains it from every WAL append.
    pub fn set_last_local_seq(&mut self, seq: LocalSeq) {
        self.last_local_seq = seq;
    }

    /// Seed the WAL statistics (surviving rows / payload bytes past the
    /// last snapshot) that drive the snapshot threshold.
    pub fn seed_wal_stats(&mut self, rows: u64, bytes: u64) {
        self.wal_rows = rows;
        self.wal_bytes = bytes;
    }

    /// Seed `server_known_vv` from its persisted encoding. An empty
    /// slice is the never-synced default. A corrupt encoding surfaces
    /// as an `Event::Error` and leaves the default VV in place — the
    /// next push then re-ships history the server already has, which
    /// peers deduplicate at the Loro layer.
    pub fn seed_server_known_vv(&mut self, encoded: &[u8]) {
        if encoded.is_empty() {
            return;
        }
        match VersionVector::decode(encoded) {
            Ok(vv) => self.server_known_vv = vv,
            Err(e) => self
                .events
                .push_back(Event::Error(format!("decode server_known_vv: {e}"))),
        }
    }

    /// Seed the durable in-flight push from a previous session. The
    /// reconnect flow re-sends it (same `push_id`) once the initial
    /// pull completes; the server deduplicates. A record whose `to_vv`
    /// fails to decode is dropped with an `Event::Error` — its ops
    /// remain covered by the VV-derived export.
    pub fn seed_in_flight_push(&mut self, push: InFlightPush) {
        match VersionVector::decode(&push.to_vv) {
            Ok(to_vv) => {
                self.in_flight = Some(InFlightState {
                    push_id: push.push_id,
                    payload: push.payload,
                    to_vv,
                });
            }
            Err(e) => self
                .events
                .push_back(Event::Error(format!("decode in-flight to_vv: {e}"))),
        }
    }

    /// The operations proven to exist in the server op stream.
    pub fn server_known_vv(&self) -> &VersionVector {
        &self.server_known_vv
    }

    /// True when the doc holds operations the server has no proof of:
    /// commits beyond `server_known_vv`, or an outstanding in-flight
    /// push. The UI's "pending changes" indicator.
    pub fn has_unsynced_ops(&self) -> bool {
        self.in_flight.is_some() || !self.server_known_vv.includes_vv(&self.doc.oplog_vv())
    }

    /// Surviving WAL rows past the last snapshot.
    pub fn wal_rows(&self) -> u64 {
        self.wal_rows
    }

    /// Payload bytes across the surviving WAL rows.
    pub fn wal_bytes(&self) -> u64 {
        self.wal_bytes
    }

    /// Capture any locally-committed mutations into the WAL as a
    /// single encrypted row (the merged delta since the doc's
    /// `last_persisted_vv`). Synchronously durable on `SqliteStorage`,
    /// so the CLI calls this after every command — binding every
    /// commit to bytes on disk before anything can go on the wire.
    /// Returns the freshly-assigned `LocalSeq`, or `None` when the doc
    /// had nothing uncaptured. Advances the capture cursor
    /// (`Doc::last_persisted_vv`) so the same commits aren't
    /// re-captured. Does **not** touch `server_known_vv`.
    pub fn capture_local_ops(&mut self) -> Result<Option<LocalSeq>, StorageError> {
        if !self.doc.has_uncaptured_ops() {
            return Ok(None);
        }
        // Snapshot the oplog VV *before* export so a mutation committed
        // between here and the cursor advance stays uncaptured for the
        // next capture.
        let vv = self.doc.oplog_vv();
        let blob = match self.doc.pending_export(&self.dek) {
            Ok(Some(b)) => b,
            // `has_uncaptured_ops` was true, so `None` shouldn't happen;
            // treat it as "nothing to capture" rather than erroring.
            Ok(None) => return Ok(None),
            Err(e) => return Err(StorageError::Backend(format!("pending_export: {e}"))),
        };
        let bytes = blob_len(&blob);
        let local_seq = self.storage.append_local_wal(self.doc_id, blob)?;
        self.last_local_seq = local_seq;
        self.wal_rows += 1;
        self.wal_bytes += bytes;
        self.doc.mark_persisted_at(vv);
        Ok(Some(local_seq))
    }

    /// Fold the WAL into a fresh full-history snapshot when it has
    /// crossed either threshold: at least `max_rows` rows, or more
    /// than `max_bytes` payload bytes. Captures any uncommitted
    /// mutations first so the snapshot provably contains every pruned
    /// row. Runs regardless of online/sync status — a full Loro
    /// snapshot retains operation history, so unsent local operations
    /// survive the fold and the next push still derives them from
    /// `server_known_vv`. Returns whether a snapshot was written.
    ///
    /// Snapshot export is O(doc) (tens of ms at ~10k lifetime items,
    /// more under wasm), so interactive hosts must NOT pass a tiny
    /// `max_rows` on their hot pulse — pass a real threshold there and
    /// `1` from an idle/exit hook so short sessions still fold down.
    /// Boot replay stays bounded by the threshold either way.
    pub fn snapshot_if_wal_exceeds(
        &mut self,
        max_rows: u64,
        max_bytes: u64,
    ) -> Result<bool, StorageError> {
        self.capture_local_ops()?;
        if self.wal_rows < max_rows.max(1) && self.wal_bytes <= max_bytes {
            return Ok(false);
        }
        self.write_local_snapshot()
    }

    /// Unconditionally fold any non-empty WAL into a fresh snapshot.
    /// `Ok(false)` when the WAL is already empty (nothing has advanced
    /// since the last fold). Exit/hide hook material.
    pub fn force_snapshot(&mut self) -> Result<bool, StorageError> {
        self.capture_local_ops()?;
        if self.wal_rows == 0 {
            return Ok(false);
        }
        self.write_local_snapshot()
    }

    fn write_local_snapshot(&mut self) -> Result<bool, StorageError> {
        // Every current commit is WAL-captured (capture ran above), so
        // the full-state export contains every row at or below the
        // cutoff — including unsent local ops, which Loro's
        // full-history snapshot preserves for later VV-derived export.
        let cutoff = self.last_local_seq;
        let blob = self
            .doc
            .snapshot_blob(&self.dek)
            .map_err(|e| StorageError::Backend(format!("snapshot_blob: {e}")))?;
        self.storage.write_snapshot(self.doc_id, cutoff, blob)?;
        self.wal_rows = 0;
        self.wal_bytes = 0;
        Ok(true)
    }

    /// Contiguous-prefix seq the engine has applied **in memory**.
    /// Use this for transport-layer decisions (the `since_seq` of a
    /// mid-session resume `PullOps`, snapshot eligibility) — NOT as
    /// the persisted resume cursor. The persisted cursor must be
    /// `last_durable_seq()` so a crash never resumes from a seq the
    /// local doc/WAL doesn't actually contain.
    pub fn last_contiguous_seq(&self) -> u64 {
        self.last_contiguous_seq
    }

    /// The doc's history epoch from the last `HelloAck`, or `None`
    /// before the first successful handshake.
    pub fn server_epoch(&self) -> Option<u64> {
        self.server_epoch
    }

    /// Contiguous-prefix seq the host has confirmed locally durable.
    /// This is the value the engine has shipped (or will ship) in
    /// `Ack` frames, and the value callers persist between sessions
    /// as the resume cursor.
    pub fn last_durable_seq(&self) -> u64 {
        self.last_durable_seq
    }

    /// Host signal: every byte the engine had advanced through up to
    /// `seq` is now durable in local storage (encrypted WAL row
    /// committed). Advances `last_durable_seq` — clamped to
    /// `last_contiguous_seq` and monotonic — and queues an `Ack` if
    /// that advance overtakes `last_sent_ack`. Caller must
    /// `pop_outbox()` afterwards to ship the queued frame.
    ///
    /// Callers should sample `last_contiguous_seq()` *synchronously*
    /// at the moment of the durability work (e.g. just before queueing
    /// the IDB append promise) and pass that sample back here after
    /// the write commits — this binds the notify to bytes that were
    /// actually persisted, not to wherever the in-memory engine has
    /// run on to in the meantime.
    ///
    /// The corresponding `server_known_vv` advance was persisted
    /// atomically with the WAL append itself, so the cursor written
    /// here never runs ahead of the VV.
    pub fn notify_oplog_durable(&mut self, seq: u64) {
        let clamped = seq.min(self.last_contiguous_seq);
        if clamped > self.last_durable_seq {
            self.last_durable_seq = clamped;
            // Persist the resume cursor at the one moment it advances —
            // through the engine's own storage handle, so the host never
            // has to read it back out and re-persist it. Storage failure
            // surfaces as an Event::Error (consistent with the other
            // storage calls); the ack still queues so the wire isn't
            // blocked on a local write.
            if let Err(e) = self
                .storage
                .write_acked_seq(self.doc_id, ServerSeq(clamped))
            {
                self.events
                    .push_back(Event::Error(format!("storage.write_acked_seq: {e}")));
            }
            self.queue_ack_if_advanced();
        }
    }

    /// True if the engine is past `Disconnected` — caller can treat
    /// this as "socket should be open."
    pub fn is_online(&self) -> bool {
        !matches!(self.state, ConnState::Disconnected)
    }

    /// True iff the engine has finished initial pull and isn't pushing.
    /// Useful for tests; callers usually don't need it.
    pub fn is_idle(&self) -> bool {
        matches!(self.state, ConnState::Idle)
    }

    /// Drain the next frame to write to the wire. Returns `None` when
    /// the outbox is empty. Caller is responsible for pacing — usually
    /// "drain to empty after every `handle_*` call."
    pub fn pop_outbox(&mut self) -> Option<Vec<u8>> {
        self.outbox.pop_front()
    }

    /// Drain the next engine event. Returns `None` when the event queue
    /// is empty.
    pub fn pop_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// Drain the next domain-level change event from the underlying
    /// doc. Pair with `pop_event` — the engine emits protocol events
    /// (`ConnStateChanged`, `Pushed`, `FrontierAdvanced`, `Error`),
    /// the doc emits `AppEvent`s (item / list lifecycle).
    pub fn pop_app_event(&self) -> Option<crate::events::AppEvent> {
        self.doc.pop_event()
    }

    /// Caller has a usable socket. Engine sends `Hello`, transitions to
    /// `Hello` state, awaits the server's response.
    pub fn handle_connected(&mut self) {
        if !matches!(self.state, ConnState::Disconnected) {
            self.events.push_back(Event::Error(
                "handle_connected called while already connected".into(),
            ));
            return;
        }
        self.state = ConnState::Hello;
        self.events
            .push_back(Event::ConnStateChanged { online: true });
        let hello = Hello {
            client: self.opts.client_name.clone(),
            client_version: self.opts.client_version.clone(),
            supported_protocol_versions: vec![PROTOCOL_VERSION],
        };
        if let Err(e) = self.encode_into_outbox(&hello) {
            self.events
                .push_back(Event::Error(format!("encode Hello: {e}")));
            self.go_disconnected();
        }
    }

    /// Caller's socket dropped (clean close, network error, tab closed).
    /// Engine returns to `Disconnected`; outbox is cleared (those bytes
    /// will be re-derived on reconnect).
    pub fn handle_disconnected(&mut self) {
        if matches!(self.state, ConnState::Disconnected) {
            return;
        }
        self.go_disconnected();
    }

    /// Caller's tick. Escalates the `Hello` handshake timeout
    /// (idempotent in non-Hello states); a no-op otherwise. Hosts call
    /// this periodically (e.g., every ~1s via setInterval /
    /// tokio::time::interval).
    pub fn handle_timeout(&mut self) {
        if matches!(self.state, ConnState::Hello) {
            self.events
                .push_back(Event::Error("handshake timed out".into()));
        }
    }

    /// One frame's worth of bytes from the server.
    pub fn handle_server_bytes(&mut self, bytes: &[u8]) {
        match self.state {
            ConnState::Disconnected => {
                self.events.push_back(Event::Error(
                    "received server bytes while disconnected".into(),
                ));
            }
            ConnState::Hello => self.handle_hello_response(bytes),
            ConnState::Pulling
            | ConnState::Bootstrapping
            | ConnState::Idle
            | ConnState::Pushing
            | ConnState::PushingDirty => self.handle_server_frame(bytes),
        }
    }

    /// Caller signal: "user committed local mutations." If we're idle
    /// and there's something to ship, push. Otherwise the engine
    /// re-checks on the next transition into `Idle`, so flushing during
    /// `Pulling` / `Hello` / `Disconnected` is safe and just queues the
    /// intent.
    pub fn flush(&mut self) {
        match self.state {
            ConnState::Idle => self.try_start_push(),
            ConnState::Pushing => self.state = ConnState::PushingDirty,
            ConnState::PushingDirty
            | ConnState::Pulling
            | ConnState::Bootstrapping
            | ConnState::Hello
            | ConnState::Disconnected => {
                // Nothing to send right now — the next Idle transition
                // re-derives the delta from `server_known_vv`.
            }
        }
    }

    // ---------- internals ----------

    fn handle_hello_response(&mut self, bytes: &[u8]) {
        // Hello{Ack,Rejected} aren't part of the `ServerFrame` tagged
        // enum — the spec sends them as bare types pre-handshake. Try
        // both decoders against the same buffer.
        if let Ok(ack) = rmp_serde::from_slice::<HelloAck>(bytes) {
            if ack.protocol_version != PROTOCOL_VERSION {
                self.events.push_back(Event::Error(format!(
                    "server picked protocol {} but client speaks {PROTOCOL_VERSION}",
                    ack.protocol_version,
                )));
                self.go_disconnected();
                return;
            }
            self.server_epoch = Some(ack.epoch);
            self.state = ConnState::Pulling;
            let frame = ClientFrame::PullOps {
                since_seq: self.last_contiguous_seq,
            };
            if let Err(e) = self.encode_into_outbox(&frame) {
                self.events
                    .push_back(Event::Error(format!("encode PullOps: {e}")));
                self.go_disconnected();
            }
            return;
        }
        if let Ok(rej) = rmp_serde::from_slice::<HelloRejected>(bytes) {
            self.events
                .push_back(Event::Error(format!("handshake rejected: {}", rej.reason)));
            self.go_disconnected();
            return;
        }
        self.events.push_back(Event::Error(
            "first server frame was neither HelloAck nor HelloRejected".into(),
        ));
        self.go_disconnected();
    }

    fn handle_server_frame(&mut self, bytes: &[u8]) {
        let frame = match rmp_serde::from_slice::<ServerFrame>(bytes) {
            Ok(f) => f,
            Err(e) => {
                self.events
                    .push_back(Event::Error(format!("decode ServerFrame: {e}")));
                return;
            }
        };
        match frame {
            ServerFrame::OpsBatch { ops, complete } => {
                if matches!(self.state, ConnState::Bootstrapping) {
                    self.events.push_back(Event::Error(
                        "OpsBatch received during Bootstrapping".into(),
                    ));
                    return;
                }
                self.apply_remote_ops(ops);
                // Ack is gated on `notify_oplog_durable` — the host
                // calls back once the encrypted WAL row covering these
                // ops is committed.
                if complete && matches!(self.state, ConnState::Pulling) {
                    self.state = ConnState::Idle;
                    self.events.push_back(Event::PulledInitial);
                    // Pull-before-push just completed: retry any durable
                    // in-flight push (same push_id — the server
                    // deduplicates), else ship whatever the fresh
                    // `server_known_vv` delta derives.
                    self.try_start_push();
                }
            }
            ServerFrame::OpsBroadcast { ops } => {
                if matches!(self.state, ConnState::Bootstrapping) {
                    // Broadcasts during bootstrap would either re-fire
                    // AppEvents on next pull or land before the snapshot
                    // baseline. Drop them — the post-bootstrap PullOps
                    // re-delivers anything past `up_to_seq`.
                    return;
                }
                self.apply_remote_ops(ops);
                // Ack gated on host's `notify_oplog_durable`.
            }
            ServerFrame::OpsAck { acks } => {
                if !matches!(self.state, ConnState::Pushing | ConnState::PushingDirty) {
                    self.events.push_back(Event::Error(format!(
                        "OpsAck received in unexpected state {:?}",
                        self.state
                    )));
                    return;
                }
                let Some(in_flight) = self.in_flight.take() else {
                    self.events
                        .push_back(Event::Error("OpsAck with no in-flight push".into()));
                    self.state = ConnState::Idle;
                    return;
                };
                // We ship exactly one blob per PushOps, so exactly one
                // ack identifying our push_id is well-formed.
                let seq = match acks.as_slice() {
                    [ack] if ack.push_id == in_flight.push_id.0 => ack.seq,
                    other => {
                        self.events.push_back(Event::Error(format!(
                            "OpsAck mismatch: expected push_id {}, got {:?}",
                            in_flight.push_id.0,
                            other.iter().map(|a| a.push_id).collect::<Vec<_>>(),
                        )));
                        // Keep the durable record; the reconnect path
                        // retries it.
                        self.in_flight = Some(in_flight);
                        return;
                    }
                };
                // Merge — never assign — the acked span into
                // `server_known_vv`: remote updates may have advanced
                // other peers' ranges while the push was in flight.
                self.server_known_vv.merge(&in_flight.to_vv);
                if let Err(e) = self.storage.complete_push(
                    self.doc_id,
                    in_flight.push_id,
                    &self.server_known_vv.encode(),
                ) {
                    self.events
                        .push_back(Event::Error(format!("storage.complete_push: {e}")));
                }
                // A deduplicated retry returns the *original* seq, which
                // we may already have pulled past — `ingest_seq` treats
                // seqs at or below the frontier as duplicates.
                let prev_contig = self.last_contiguous_seq;
                self.ingest_seq(seq);
                if self.last_contiguous_seq > prev_contig {
                    self.events.push_back(Event::FrontierAdvanced {
                        seq: self.last_contiguous_seq,
                    });
                }
                self.events.push_back(Event::Pushed);
                // Ack (of received seqs) stays gated on
                // `notify_oplog_durable` — the local WAL append covering
                // this seq may still be flushing on async hosts.
                self.state = ConnState::Idle;
                // If the doc advanced beyond the acked `to_vv` while the
                // push was in flight (PushingDirty, or a mutation that
                // never flushed), export the next delta now — a no-op
                // when nothing is pending.
                self.try_start_push();
            }
            ServerFrame::SnapshotRequired { up_to_seq: _ } => {
                if !matches!(self.state, ConnState::Pulling) {
                    self.events.push_back(Event::Error(format!(
                        "SnapshotRequired received in unexpected state {:?}",
                        self.state
                    )));
                    return;
                }
                // `up_to_seq` is informational — the authoritative
                // value is the one returned in the `Snapshot` frame.
                self.state = ConnState::Bootstrapping;
                let frame = ClientFrame::PullSnapshot;
                if let Err(e) = self.encode_into_outbox(&frame) {
                    self.events
                        .push_back(Event::Error(format!("encode PullSnapshot: {e}")));
                    self.go_disconnected();
                }
            }
            ServerFrame::Snapshot {
                up_to_seq,
                blob,
                complete,
            } => {
                if !matches!(self.state, ConnState::Bootstrapping) {
                    self.events.push_back(Event::Error(format!(
                        "Snapshot received in unexpected state {:?}",
                        self.state
                    )));
                    return;
                }
                // The chain streams one row per frame (full head, then
                // each delta in order). Import each as it lands and
                // learn its exact declared frontier: everything it
                // contains is proven server history. Loro import is
                // idempotent, so a chain cut short by a disconnect is
                // simply re-pulled whole on reconnect.
                let imported_vv = match self
                    .doc
                    .apply_remote_batch(&self.dek, std::iter::once(&blob))
                {
                    Ok(vv) => vv,
                    Err(e) => {
                        self.events
                            .push_back(Event::Error(format!("apply snapshot: {e}")));
                        self.go_disconnected();
                        return;
                    }
                };
                self.server_known_vv.merge(&imported_vv);
                if !complete {
                    // More rows to come. Persisting waits for the last
                    // one: the checkpoint export below is O(doc), and
                    // a `server_known_vv` written ahead of its
                    // checkpoint would claim history the stored doc
                    // does not hold.
                    return;
                }
                if let Err(e) = self
                    .storage
                    .write_server_known_vv(self.doc_id, &self.server_known_vv.encode())
                {
                    self.events
                        .push_back(Event::Error(format!("storage.write_server_known_vv: {e}")));
                    self.go_disconnected();
                    return;
                }
                // Re-export the *merged* doc (server state ∪ any unsent
                // local work) as the local checkpoint and prune the whole
                // WAL prefix. The server blob alone would NOT be a valid
                // checkpoint here: it lacks our unsent local operations,
                // which pruned WAL rows may have carried. A full-history
                // export of the merged doc contains both, and the next
                // push still derives the unsent ops from
                // `server_known_vv`.
                let cutoff = self.last_local_seq;
                let local_blob = match self.doc.snapshot_blob(&self.dek) {
                    Ok(b) => b,
                    Err(e) => {
                        self.events
                            .push_back(Event::Error(format!("snapshot_blob: {e}")));
                        self.go_disconnected();
                        return;
                    }
                };
                if let Err(e) = self.storage.write_snapshot(self.doc_id, cutoff, local_blob) {
                    self.events.push_back(Event::Error(format!(
                        "storage.write bootstrap snapshot: {e}"
                    )));
                    self.go_disconnected();
                    return;
                }
                // The checkpoint covers the whole oplog — nothing is
                // uncaptured, and the WAL is freshly empty.
                self.doc.mark_persisted();
                self.wal_rows = 0;
                self.wal_bytes = 0;
                if up_to_seq > self.last_contiguous_seq {
                    self.last_contiguous_seq = up_to_seq;
                    self.events
                        .push_back(Event::FrontierAdvanced { seq: up_to_seq });
                }
                // The storage mirror now contains the snapshot. Ack remains
                // gated on `notify_oplog_durable`: async hosts call it only
                // after the queued disk transaction commits.
                // Resume the catch-up: pull any ops written after the
                // snapshot was taken.
                self.state = ConnState::Pulling;
                let frame = ClientFrame::PullOps {
                    since_seq: self.last_contiguous_seq,
                };
                if let Err(e) = self.encode_into_outbox(&frame) {
                    self.events
                        .push_back(Event::Error(format!("encode PullOps: {e}")));
                    self.go_disconnected();
                }
            }
            ServerFrame::SnapshotRequest {
                up_to_seq: _,
                compaction_floor_seq,
                base,
            } => {
                // Server picked us as the snapshot producer. Produce at
                // the frontiers of `server_known_vv` — exactly the
                // state/history the server op stream represents — and
                // tag with `last_contiguous_seq`, its seq-coordinate
                // twin. Never the current doc: that may contain unsent
                // local operations, and a bootstrapping device must not
                // receive ops the server op stream doesn't have.
                //
                // With a `base` the server wants a *delta*: the updates
                // strictly after the previous snapshot's VV (which the
                // server stored sealed and hands back opaquely). The
                // base must sit inside our server-known frontier; if it
                // doesn't we decline rather than upload a blob that
                // would break the chain, and the server's lease times
                // out onto another producer.
                //
                // `compaction_floor_seq` is server-side bookkeeping for
                // op-blob GC and is echoed back verbatim — it doesn't
                // influence the produced blob.
                let produced = match &base {
                    None => self.doc.snapshot_blob_at(&self.dek, &self.server_known_vv),
                    Some(b) => match open_vv(&self.dek, &b.vv) {
                        Ok(from_vv) if self.server_known_vv.includes_vv(&from_vv) => self
                            .doc
                            .delta_blob_between(&self.dek, &from_vv, &self.server_known_vv),
                        Ok(_) => {
                            self.events.push_back(Event::Error(format!(
                                "SnapshotRequest: delta base at seq {} is outside our \
                                 server-known frontier; declining",
                                b.seq
                            )));
                            return;
                        }
                        Err(e) => {
                            self.events.push_back(Event::Error(format!(
                                "SnapshotRequest: cannot open delta base: {e}"
                            )));
                            return;
                        }
                    },
                };
                let blob = match produced {
                    Ok(b) => b,
                    Err(e) => {
                        self.events
                            .push_back(Event::Error(format!("snapshot export: {e}")));
                        return;
                    }
                };
                let up_to_vv = match seal_vv(&self.dek, &self.server_known_vv) {
                    Ok(b) => b,
                    Err(e) => {
                        self.events
                            .push_back(Event::Error(format!("seal up_to_vv: {e}")));
                        return;
                    }
                };
                let frame = ClientFrame::PushSnapshot {
                    up_to_seq: self.last_contiguous_seq,
                    compaction_floor_seq,
                    from_seq: base.as_ref().map(|b| b.seq),
                    up_to_vv,
                    blob,
                };
                if let Err(e) = self.encode_into_outbox(&frame) {
                    self.events
                        .push_back(Event::Error(format!("encode PushSnapshot: {e}")));
                }
            }
        }
    }

    fn apply_remote_ops(&mut self, ops: Vec<StoredBlob>) {
        if ops.is_empty() {
            return;
        }

        // Apply all blobs to Loro first — the CRDT tolerates
        // out-of-order arrivals (VV handles causality), so a hole at
        // the seq layer doesn't block doc convergence. This also
        // keeps the UI responsive under replica lag. The returned VV
        // is the union of every blob's *declared* range — proof of
        // server possession even when an import was a no-op duplicate.
        let imported_vv = match self
            .doc
            .apply_remote_batch(&self.dek, ops.iter().map(|op| &op.blob))
        {
            Ok(vv) => vv,
            Err(e) => {
                let failed_seq = ops
                    .iter()
                    .find(|op| op.seq > self.last_contiguous_seq)
                    .map(|op| op.seq)
                    .unwrap_or_else(|| ops.iter().map(|op| op.seq).max().unwrap_or(0));
                self.events
                    .push_back(Event::Error(format!("apply remote blob {failed_seq}: {e}")));
                return;
            }
        };
        self.server_known_vv.merge(&imported_vv);
        let vv_bytes = self.server_known_vv.encode();

        // Mirror each applied blob into the WAL. Order matches the wire
        // (server-side seq order), which is also the order boot replay
        // needs. Each append persists the advanced `server_known_vv`
        // atomically with its row. Errors don't roll back the Loro
        // apply — the host surfaces the event and the next boot
        // re-pulls from the durable cursor.
        for op in &ops {
            match self.storage.append_remote_wal(
                self.doc_id,
                RemoteWalRow {
                    server_seq: ServerSeq(op.seq),
                    payload: op.blob.clone(),
                },
                &vv_bytes,
            ) {
                Ok((local_seq, inserted)) => {
                    if inserted {
                        self.last_local_seq = local_seq;
                        self.wal_rows += 1;
                        self.wal_bytes += blob_len(&op.blob);
                    }
                }
                Err(e) => {
                    self.events
                        .push_back(Event::Error(format!("storage.append_remote_wal: {e}")));
                    // Don't bail — same reasoning as `capture_local_ops`.
                }
            }
        }

        // Bookkeep each seq: advance the contiguous frontier. Server
        // seqs are dense and delivered in order, so each op is the
        // contiguous next; the Loro state is already current — we just
        // track what the *server* has gotten back to us about.
        let prev_contig = self.last_contiguous_seq;
        for op in &ops {
            self.ingest_seq(op.seq);
        }
        if self.last_contiguous_seq > prev_contig {
            self.events.push_back(Event::FrontierAdvanced {
                seq: self.last_contiguous_seq,
            });
        }

        // Domain-level deltas (`AppEvent`) flow through `Doc`'s own
        // queue — drained by the host alongside this protocol event
        // queue.
    }

    /// Bookkeep a single inbound seq by advancing the contiguous
    /// frontier. Does not touch the Loro doc — callers apply the blob.
    ///
    /// Server seqs are dense and delivered in order over a single
    /// ordered connection, so the only seq we ever expect is the
    /// contiguous next. A lower seq is a duplicate (re-pull overlap,
    /// or a deduplicated push retry acked with its original seq) —
    /// drop it. A higher seq would be a forward gap, which is
    /// structurally impossible here; never advance past one, and trip
    /// a debug assertion so a broken invariant is loud, not silent.
    fn ingest_seq(&mut self, n: u64) {
        if n == self.last_contiguous_seq + 1 {
            self.last_contiguous_seq = n;
        } else {
            debug_assert!(
                n <= self.last_contiguous_seq,
                "non-contiguous seq {n} above frontier {} — dense-seq invariant violated",
                self.last_contiguous_seq,
            );
        }
    }

    fn queue_ack_if_advanced(&mut self) {
        if self.last_durable_seq > self.last_sent_ack {
            let ack = ClientFrame::Ack {
                last_acked_seq: self.last_durable_seq,
            };
            if self.encode_into_outbox(&ack).is_ok() {
                self.last_sent_ack = self.last_durable_seq;
            }
        }
    }

    fn try_start_push(&mut self) {
        if !matches!(self.state, ConnState::Idle) {
            return;
        }
        if self.in_flight.is_none() {
            // Bind pending commits to disk before they can reach the
            // wire — the WAL append is the local durability point.
            if let Err(e) = self.capture_local_ops() {
                self.events
                    .push_back(Event::Error(format!("storage.append_local_wal: {e}")));
                return;
            }
            // The outbound delta is derived from Loro history against
            // `server_known_vv` — independent of which WAL rows still
            // exist (folded rows are re-derivable from the snapshot's
            // retained history).
            let to_vv = self.doc.oplog_vv();
            let blob = match self
                .doc
                .export_updates_since(&self.dek, &self.server_known_vv)
            {
                Ok(Some(b)) => b,
                Ok(None) => return,
                Err(e) => {
                    self.events
                        .push_back(Event::Error(format!("export_updates_since: {e}")));
                    return;
                }
            };
            let push = InFlightPush {
                push_id: PushId::generate(),
                payload: blob.clone(),
                from_vv: self.server_known_vv.encode(),
                to_vv: to_vv.encode(),
            };
            // Durable BEFORE the first send: a crash after the server
            // inserts but before our ack lands must retry this exact
            // push_id, not mint a fresh one.
            if let Err(e) = self.storage.put_in_flight_push(self.doc_id, push.clone()) {
                self.events
                    .push_back(Event::Error(format!("storage.put_in_flight_push: {e}")));
                return;
            }
            self.in_flight = Some(InFlightState {
                push_id: push.push_id,
                payload: push.payload,
                to_vv,
            });
        }
        let in_flight = self.in_flight.as_ref().expect("just ensured");
        let frame = ClientFrame::PushOps {
            ops: vec![PushBlob {
                push_id: in_flight.push_id.0,
                blob: in_flight.payload.clone(),
            }],
        };
        if let Err(e) = self.encode_into_outbox(&frame) {
            self.events
                .push_back(Event::Error(format!("encode PushOps: {e}")));
            return;
        }
        self.state = ConnState::Pushing;
    }

    fn go_disconnected(&mut self) {
        self.state = ConnState::Disconnected;
        // The in-flight push survives — both here and durably. On
        // reconnect the engine pulls first (the pull may prove the ops
        // landed), then re-sends the same push_id; the server
        // deduplicates either way.
        self.outbox.clear();
        self.events
            .push_back(Event::ConnStateChanged { online: false });
    }

    fn encode_into_outbox<T: Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), rmp_serde::encode::Error> {
        let bytes = rmp_serde::to_vec_named(value)?;
        self.outbox.push_back(bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{Doc, LIST_INBOX};
    use crate::storage::MemStorage;
    use monoplan_protocol::{EncryptedBlob, PushAck, ServerFrame, SnapshotBase, StoredBlob};

    fn opts() -> EngineOptions {
        EngineOptions {
            client_name: "test".into(),
            client_version: "0.0.0".into(),
        }
    }

    fn mem() -> DynStorage {
        Box::new(MemStorage::new())
    }

    /// Tests don't care which doc — the trait isn't load-bearing on
    /// the wire yet, and `MemStorage` instances are per-engine.
    fn fake_doc_id() -> DocId {
        DocId(uuid::Uuid::nil())
    }

    /// Engine over a fresh doc. Its seeded built-ins are pending
    /// against an empty `server_known_vv`, so pull-complete triggers a
    /// seed push.
    fn fresh_engine() -> SyncEngine {
        SyncEngine::new(
            Doc::new().unwrap(),
            fake_doc_id(),
            Dek::generate(),
            0,
            opts(),
            mem(),
        )
    }

    /// Engine whose doc's seeds are already covered by
    /// `server_known_vv` — pull-complete leaves it cleanly Idle
    /// without queueing a seed push. Default for state-machine tests
    /// so each one isolates a single transition.
    fn fresh_engine_clean() -> SyncEngine {
        let mut doc = Doc::new().unwrap();
        doc.mark_persisted();
        let vv = doc.oplog_vv();
        let mut eng = SyncEngine::new(doc, fake_doc_id(), Dek::generate(), 0, opts(), mem());
        eng.server_known_vv = vv;
        eng
    }

    fn enc<T: Serialize>(value: &T) -> Vec<u8> {
        rmp_serde::to_vec_named(value).unwrap()
    }

    fn dec<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> T {
        rmp_serde::from_slice(bytes).unwrap()
    }

    fn drain_outbox(eng: &mut SyncEngine) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while let Some(b) = eng.pop_outbox() {
            out.push(b);
        }
        out
    }

    fn drain_events(eng: &mut SyncEngine) -> Vec<Event> {
        let mut out = Vec::new();
        while let Some(e) = eng.pop_event() {
            out.push(e);
        }
        out
    }

    /// Decode a wire frame as PushOps and build the matching one-blob
    /// OpsAck at `seq` — the fake server's happy path.
    fn ack_frame(push_bytes: &[u8], seq: u64) -> ServerFrame {
        let frame: ClientFrame = dec(push_bytes);
        match frame {
            ClientFrame::PushOps { ops } => {
                assert_eq!(ops.len(), 1, "engine ships exactly one blob per push");
                ServerFrame::OpsAck {
                    acks: vec![PushAck {
                        push_id: ops[0].push_id,
                        seq,
                    }],
                }
            }
            other => panic!("expected PushOps, got {other:?}"),
        }
    }

    /// Push a `fresh_engine_clean` through `Disconnected → ... → Idle`
    /// with an empty initial pull.
    fn drive_to_idle(eng: &mut SyncEngine) {
        eng.handle_connected();
        let _hello = eng
            .pop_outbox()
            .expect("Hello should be queued after handle_connected()");
        eng.handle_server_bytes(&enc(&HelloAck {
            server_version: "test".into(),
            protocol_version: PROTOCOL_VERSION,
            epoch: 0,
        }));
        let _pull = eng
            .pop_outbox()
            .expect("PullOps should be queued after successful HelloAck()");
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: vec![],
            complete: true,
        }));
        assert!(eng.is_idle(), "expected Idle after empty pull");
    }

    /// Encrypt a blob using a separate "remote" doc + matching DEK.
    /// Lets a test simulate "another device pushed these ops to the
    /// server" without standing up a real network.
    ///
    /// We export the *full* remote state (seed + mutation) rather than
    /// a delta — Loro tracks per-peer monotonic op counters and rejects
    /// imports whose first op skips counters the importer doesn't have,
    /// so a "delta from a marked-persisted point" can land in `pending`
    /// instead of `success` when applied cold.
    fn make_remote_blob(dek: &Dek, mutate: impl FnOnce(&mut Doc)) -> EncryptedBlob {
        let mut remote = Doc::new().unwrap();
        mutate(&mut remote);
        remote.pending_export(dek).unwrap().expect("blob")
    }

    #[test]
    fn happy_path_handshake_and_empty_pull() {
        let mut eng = fresh_engine_clean();
        eng.handle_connected();
        let hello: Hello = dec(&eng.pop_outbox().unwrap());
        assert_eq!(hello.supported_protocol_versions, vec![PROTOCOL_VERSION]);
        assert_eq!(hello.client, "test");
        assert!(matches!(
            drain_events(&mut eng).as_slice(),
            [Event::ConnStateChanged { online: true }]
        ));

        eng.handle_server_bytes(&enc(&HelloAck {
            server_version: "s".into(),
            protocol_version: PROTOCOL_VERSION,
            epoch: 0,
        }));
        let pull: ClientFrame = dec(&eng.pop_outbox().unwrap());
        assert!(matches!(pull, ClientFrame::PullOps { since_seq: 0 }));

        eng.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: vec![],
            complete: true,
        }));
        assert!(eng.is_idle());
        let events = drain_events(&mut eng);
        assert!(events.contains(&Event::PulledInitial));
        assert!(
            eng.pop_outbox().is_none(),
            "no follow-up frames on empty pull"
        );
    }

    #[test]
    fn pull_complete_on_fresh_doc_stays_idle() {
        // A fresh doc has an empty oplog (built-ins are virtual), so an
        // empty initial pull completes without queuing a follow-up push.
        let mut eng = fresh_engine();
        eng.handle_connected();
        let _hello = eng.pop_outbox().unwrap();
        eng.handle_server_bytes(&enc(&HelloAck {
            server_version: "s".into(),
            protocol_version: PROTOCOL_VERSION,
            epoch: 0,
        }));
        let _pull = eng.pop_outbox().unwrap();
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: vec![],
            complete: true,
        }));
        assert!(
            eng.pop_outbox().is_none(),
            "no follow-up push for untouched fresh doc"
        );
        assert!(eng.is_idle());
    }

    #[test]
    fn hello_rejected_disconnects_with_error() {
        let mut eng = fresh_engine_clean();
        eng.handle_connected();
        let _hello = eng.pop_outbox().unwrap();
        let _ = drain_events(&mut eng);
        eng.handle_server_bytes(&enc(&HelloRejected {
            reason: "no overlap".into(),
        }));
        assert!(!eng.is_online());
        let events = drain_events(&mut eng);
        assert!(matches!(events[0], Event::Error(ref s) if s.contains("rejected")));
        assert!(matches!(
            events.last(),
            Some(Event::ConnStateChanged { online: false })
        ));
    }

    #[test]
    fn unknown_protocol_version_disconnects_with_error() {
        let mut eng = fresh_engine_clean();
        eng.handle_connected();
        let _ = eng.pop_outbox().unwrap();
        let _ = drain_events(&mut eng);
        eng.handle_server_bytes(&enc(&HelloAck {
            server_version: "s".into(),
            protocol_version: 9999,
            epoch: 0,
        }));
        assert!(!eng.is_online());
        let events = drain_events(&mut eng);
        assert!(matches!(events[0], Event::Error(ref s) if s.contains("9999")));
    }

    #[test]
    fn handle_timeout_in_hello_emits_error_only() {
        let mut eng = fresh_engine_clean();
        eng.handle_timeout();
        assert!(
            drain_events(&mut eng).is_empty(),
            "no error in Disconnected"
        );

        eng.handle_connected();
        let _ = drain_events(&mut eng);
        let _ = drain_outbox(&mut eng);
        eng.handle_timeout();
        let evs = drain_events(&mut eng);
        assert!(matches!(evs.as_slice(), [Event::Error(s)] if s.contains("timed out")));

        // In Idle, timeout is a no-op.
        let mut eng2 = fresh_engine_clean();
        drive_to_idle(&mut eng2);
        let _ = drain_events(&mut eng2);
        eng2.handle_timeout();
        assert!(drain_events(&mut eng2).is_empty());
    }

    #[test]
    fn flush_in_idle_pushes_when_pending() {
        let mut eng = fresh_engine_clean();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);
        let _ = drain_outbox(&mut eng);

        eng.doc_mut().add_item(LIST_INBOX, "thing").unwrap();
        eng.flush();
        let push_bytes = eng.pop_outbox().expect("PushOps");

        eng.handle_server_bytes(&enc(&ack_frame(&push_bytes, 1)));
        assert!(eng.is_idle());
        let events = drain_events(&mut eng);
        assert!(events.contains(&Event::Pushed));
        assert!(events.contains(&Event::FrontierAdvanced { seq: 1 }));

        // Engine must NOT queue an Ack before the host confirms the
        // op's bytes are locally durable — even on our own OpsAck.
        assert!(
            eng.pop_outbox().is_none(),
            "Ack frame queued before notify_oplog_durable",
        );

        // Host signals "WAL row covering seq 1 is on disk" → ack ships.
        eng.notify_oplog_durable(1);
        let ack: ClientFrame = dec(&eng.pop_outbox().expect("Ack"));
        assert!(matches!(ack, ClientFrame::Ack { last_acked_seq: 1 }));

        // No double-push: `server_known_vv` covers the acked ops.
        eng.flush();
        assert!(eng.pop_outbox().is_none(), "second flush is a no-op");
    }

    #[test]
    fn flush_captures_into_wal_before_wire() {
        // The push path itself captures the pending commit into a
        // durable WAL row before the frame is queued.
        let storage = std::sync::Arc::new(MemStorage::new());
        let mut doc = Doc::new().unwrap();
        doc.mark_persisted();
        let vv = doc.oplog_vv();
        let mut eng = SyncEngine::new(
            doc,
            fake_doc_id(),
            Dek::generate(),
            0,
            opts(),
            Box::new(storage.clone()),
        );
        eng.server_known_vv = vv;
        drive_to_idle(&mut eng);
        let _ = drain_outbox(&mut eng);

        eng.doc_mut().add_item(LIST_INBOX, "buy milk").unwrap();
        eng.flush();
        let _push = eng.pop_outbox().expect("PushOps");
        assert_eq!(storage.wal_len(), 1, "commit WAL-captured at push time");
        assert!(
            !eng.doc().has_uncaptured_ops(),
            "capture cursor advanced with the append"
        );
        // The in-flight record is durable before the send.
        assert!(storage.in_flight().is_some());
    }

    #[test]
    fn wal_threshold_gates_hot_pulse_but_not_forced_fold() {
        let mut eng = fresh_engine_clean();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);
        let _ = drain_outbox(&mut eng);

        eng.doc_mut().add_item(LIST_INBOX, "thing").unwrap();
        eng.capture_local_ops().unwrap();
        assert_eq!(eng.wal_rows(), 1);

        // One row is under the hot-pulse threshold; the forced fold
        // compacts it; a second forced fold is a no-op.
        assert!(!eng.snapshot_if_wal_exceeds(250, u64::MAX).unwrap());
        assert!(eng.force_snapshot().unwrap());
        assert_eq!(eng.wal_rows(), 0);
        assert!(!eng.force_snapshot().unwrap());
        assert!(!eng.snapshot_if_wal_exceeds(250, u64::MAX).unwrap());
    }

    #[test]
    fn wal_byte_threshold_triggers_fold() {
        let mut eng = fresh_engine_clean();
        eng.doc_mut().add_item(LIST_INBOX, "thing").unwrap();
        eng.capture_local_ops().unwrap();
        assert!(eng.wal_bytes() > 0);
        // Row count far below threshold, but byte cap of 1 is exceeded.
        assert!(eng.snapshot_if_wal_exceeds(1_000_000, 1).unwrap());
        assert_eq!(eng.wal_bytes(), 0);
    }

    #[test]
    fn snapshot_folds_unsent_ops_and_push_still_derives_them() {
        // Spec test 3: after pending WAL rows are folded and deleted,
        // sync still exports every unsent operation from
        // `server_known_vv` — upload is derived from Loro history.
        let storage = std::sync::Arc::new(MemStorage::new());
        let mut doc = Doc::new().unwrap();
        doc.mark_persisted();
        let vv = doc.oplog_vv();
        let dek = Dek::generate();
        let mut eng = SyncEngine::new(
            doc,
            fake_doc_id(),
            dek.clone(),
            0,
            opts(),
            Box::new(storage.clone()),
        );
        eng.server_known_vv = vv;

        // Offline: mutate, capture, fold — the pending row is pruned.
        let item_id = eng.doc_mut().add_item(LIST_INBOX, "offline work").unwrap();
        eng.capture_local_ops().unwrap();
        assert!(eng.force_snapshot().unwrap());
        assert_eq!(storage.wal_len(), 0, "pending row folded away");

        // Online: the push still derives the unsent op. (Pull-complete
        // auto-starts the push, so the engine lands in Pushing, not
        // Idle.)
        eng.handle_connected();
        let _hello = eng.pop_outbox().unwrap();
        eng.handle_server_bytes(&enc(&HelloAck {
            server_version: "s".into(),
            protocol_version: PROTOCOL_VERSION,
            epoch: 0,
        }));
        let _pull = eng.pop_outbox().unwrap();
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: vec![],
            complete: true,
        }));
        let push_bytes = eng.pop_outbox().expect("push of folded-but-unsent op");
        let frame: ClientFrame = dec(&push_bytes);
        let blob = match frame {
            ClientFrame::PushOps { ops } => ops.into_iter().next().unwrap().blob,
            other => panic!("expected PushOps, got {other:?}"),
        };
        // A peer applying the blob sees the folded op.
        let mut peer = Doc::new().unwrap();
        peer.apply_remote(&dek, &blob).unwrap();
        assert!(peer.get_item(&item_id).is_some());
    }

    #[test]
    fn flush_in_idle_with_no_pending_is_noop() {
        let mut eng = fresh_engine_clean();
        drive_to_idle(&mut eng);
        let _ = drain_outbox(&mut eng);
        let _ = drain_events(&mut eng);
        eng.flush();
        assert!(eng.pop_outbox().is_none());
    }

    #[test]
    fn pushing_dirty_re_pushes_after_ack() {
        let mut eng = fresh_engine_clean();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);

        // First mutation + flush starts a push.
        eng.doc_mut().add_item(LIST_INBOX, "first").unwrap();
        eng.flush();
        let first_push = eng.pop_outbox().expect("first PushOps");

        // Mutate again while the push is in flight.
        let item_id = eng.doc_mut().add_item(LIST_INBOX, "during-push").unwrap();
        eng.flush();
        // No new wire bytes yet — engine is in PushingDirty, waiting.
        assert!(eng.pop_outbox().is_none());

        // Server acks the first push.
        eng.handle_server_bytes(&enc(&ack_frame(&first_push, 1)));

        // Engine should immediately re-push with the mid-push mutation.
        let second_push = eng.pop_outbox().expect("re-push after ack");
        let frame: ClientFrame = dec(&second_push);
        assert!(matches!(frame, ClientFrame::PushOps { .. }));

        eng.handle_server_bytes(&enc(&ack_frame(&second_push, 2)));
        assert!(eng.is_idle());
        // The mid-push mutation made it into the doc and the second
        // push acked it — nothing further pending.
        assert!(eng.doc().get_item(&item_id).is_some());
        eng.flush();
        assert!(eng.pop_outbox().is_none());
    }

    #[test]
    fn mutation_during_in_flight_push_stays_pending_after_ack() {
        // Spec test 7: a mutation made during an in-flight push is
        // beyond the acked `to_vv`, so it remains pending (and the
        // engine re-derives it) even without an explicit flush.
        let mut eng = fresh_engine_clean();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);

        eng.doc_mut().add_item(LIST_INBOX, "first").unwrap();
        eng.flush();
        let first_push = eng.pop_outbox().expect("PushOps");

        // Mutation mid-flight, no flush call.
        let mid_id = eng.doc_mut().add_item(LIST_INBOX, "mid-flight").unwrap();

        eng.handle_server_bytes(&enc(&ack_frame(&first_push, 1)));
        // The ack merged only `to_vv`; the mid-flight op is beyond it
        // and ships in the auto-derived follow-up push.
        let follow_up = eng.pop_outbox().expect("follow-up push of mid-flight op");
        let dek = eng.dek.clone();
        let frame: ClientFrame = dec(&follow_up);
        let blob = match frame {
            ClientFrame::PushOps { ops } => ops.into_iter().next().unwrap().blob,
            other => panic!("expected PushOps, got {other:?}"),
        };
        let mut peer = Doc::new().unwrap();
        // Peer needs the first push too for causal completeness.
        let first_frame: ClientFrame = dec(&first_push);
        if let ClientFrame::PushOps { ops } = first_frame {
            peer.apply_remote(&dek, &ops[0].blob).unwrap();
        }
        peer.apply_remote(&dek, &blob).unwrap();
        assert!(peer.get_item(&mid_id).is_some());
    }

    #[test]
    fn flush_during_pull_pushes_when_pull_completes() {
        let mut eng = fresh_engine_clean();
        eng.handle_connected();
        let _ = eng.pop_outbox().unwrap();
        eng.handle_server_bytes(&enc(&HelloAck {
            server_version: "s".into(),
            protocol_version: PROTOCOL_VERSION,
            epoch: 0,
        }));
        let _ = eng.pop_outbox().unwrap(); // PullOps

        // User mutates and flushes while we're still Pulling.
        eng.doc_mut().add_item(LIST_INBOX, "during pull").unwrap();
        eng.flush();
        assert!(eng.pop_outbox().is_none(), "Pulling defers push until Idle");

        // Pull completes — engine should self-trigger the push.
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: vec![],
            complete: true,
        }));
        let frame: ClientFrame = dec(&eng.pop_outbox().expect("PushOps after pull"));
        assert!(matches!(frame, ClientFrame::PushOps { .. }));
    }

    #[test]
    fn broadcast_during_idle_applies_and_acks() {
        let mut eng = fresh_engine_clean();
        let dek = eng.dek.clone();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);

        let remote_blob = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "from peer").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBroadcast {
            ops: vec![StoredBlob {
                seq: 1,
                blob: remote_blob,
            }],
        }));
        let events = drain_events(&mut eng);
        assert!(events.contains(&Event::FrontierAdvanced { seq: 1 }));
        let app_evs: Vec<_> = std::iter::from_fn(|| eng.pop_app_event()).collect();
        assert!(
            app_evs
                .iter()
                .any(|e| matches!(e, crate::events::AppEvent::ItemAdded { text, .. } if text == "from peer")),
            "expected ItemAdded for `from peer` in {app_evs:?}"
        );
        assert_eq!(eng.last_contiguous_seq(), 1);
        assert_eq!(eng.last_durable_seq(), 0, "durable lags until notify");
        assert!(
            eng.pop_outbox().is_none(),
            "Ack frame queued before notify_oplog_durable",
        );
        eng.notify_oplog_durable(1);
        let ack: ClientFrame = dec(&eng.pop_outbox().expect("Ack"));
        assert!(matches!(ack, ClientFrame::Ack { last_acked_seq: 1 }));

        // Local doc reflects the applied peer op.
        let names: Vec<_> = eng
            .doc()
            .items_in_list(LIST_INBOX, false)
            .into_iter()
            .map(|i| i.text)
            .collect();
        assert!(names.iter().any(|t| t == "from peer"));
    }

    #[test]
    fn ops_batch_applies_multiple_remote_ops_before_ack() {
        let mut eng = fresh_engine_clean();
        let dek = eng.dek.clone();
        let mut remote = Doc::new().unwrap();
        let id = remote.add_item(LIST_INBOX, "old").unwrap();
        let setup_blob = remote.pending_export(&dek).unwrap().unwrap();
        remote.mark_persisted();
        remote.edit_item_text(&id, "new").unwrap();
        let edit_blob = remote.pending_export(&dek).unwrap().unwrap();

        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);
        let _ = drain_outbox(&mut eng);

        eng.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: vec![
                StoredBlob {
                    seq: 1,
                    blob: setup_blob,
                },
                StoredBlob {
                    seq: 2,
                    blob: edit_blob,
                },
            ],
            complete: false,
        }));

        let events = drain_events(&mut eng);
        assert_eq!(events, vec![Event::FrontierAdvanced { seq: 2 }]);

        let app_evs: Vec<_> = std::iter::from_fn(|| eng.pop_app_event()).collect();
        assert!(
            app_evs.iter().any(|e| matches!(
                e,
                crate::events::AppEvent::ItemAdded { id: eid, text, .. } if eid == &id && text == "new"
            )),
            "expected final ItemAdded for {id} in {app_evs:?}"
        );

        // Ack gated on host-confirmed durability.
        assert!(eng.pop_outbox().is_none());
        eng.notify_oplog_durable(2);
        let ack: ClientFrame = dec(&eng.pop_outbox().expect("Ack"));
        assert!(matches!(ack, ClientFrame::Ack { last_acked_seq: 2 }));
        assert_eq!(eng.last_contiguous_seq(), 2);
        assert_eq!(eng.last_durable_seq(), 2);
    }

    #[test]
    fn broadcast_during_pushing_does_not_clobber_in_flight() {
        let mut eng = fresh_engine_clean();
        let dek = eng.dek.clone();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);

        // Mutate locally, start a push.
        eng.doc_mut().add_item(LIST_INBOX, "local-pushing").unwrap();
        eng.flush();
        let push_bytes = eng.pop_outbox().expect("PushOps");

        // Broadcast arrives during Pushing.
        let remote_blob = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "peer-during-push").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBroadcast {
            ops: vec![StoredBlob {
                seq: 1,
                blob: remote_blob,
            }],
        }));
        // State still Pushing — broadcast doesn't transition.
        assert!(!eng.is_idle());
        assert!(
            eng.doc()
                .items_in_list(LIST_INBOX, false)
                .iter()
                .any(|i| i.text == "peer-during-push")
        );

        // Server acks our push with seq 2 (continuing the contiguous
        // sequence after the broadcast at seq 1).
        eng.handle_server_bytes(&enc(&ack_frame(&push_bytes, 2)));
        assert!(eng.is_idle());
        let _ = drain_outbox(&mut eng);
        // `server_known_vv` now covers BOTH our local mutation (via the
        // ack's to_vv) and the peer op (via the broadcast's declared
        // range) — so a fresh flush has nothing new to ship.
        eng.flush();
        assert!(eng.pop_outbox().is_none(), "no re-push after broadcast+ack");
    }

    #[test]
    fn multi_batch_pull_reaches_idle_only_on_complete() {
        let mut eng = fresh_engine_clean();
        let dek = eng.dek.clone();
        eng.handle_connected();
        let _ = eng.pop_outbox().unwrap();
        eng.handle_server_bytes(&enc(&HelloAck {
            server_version: "s".into(),
            protocol_version: PROTOCOL_VERSION,
            epoch: 0,
        }));
        let _ = eng.pop_outbox().unwrap();

        let blob1 = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "p1").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: vec![StoredBlob {
                seq: 1,
                blob: blob1,
            }],
            complete: false,
        }));
        assert!(!eng.is_idle());
        let events = drain_events(&mut eng);
        assert!(!events.contains(&Event::PulledInitial));

        let blob2 = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "p2").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: vec![StoredBlob {
                seq: 2,
                blob: blob2,
            }],
            complete: true,
        }));
        assert!(eng.is_idle());
        let events = drain_events(&mut eng);
        assert!(events.contains(&Event::PulledInitial));
        assert_eq!(eng.last_contiguous_seq(), 2);
    }

    #[test]
    fn disconnect_mid_push_retries_same_push_id_on_reconnect() {
        // Spec test 10's client half: the durable in-flight record
        // survives a disconnect, and the reconnect re-sends the SAME
        // push_id after the pull completes.
        let storage = std::sync::Arc::new(MemStorage::new());
        let mut doc = Doc::new().unwrap();
        doc.mark_persisted();
        let vv = doc.oplog_vv();
        let mut eng = SyncEngine::new(
            doc,
            fake_doc_id(),
            Dek::generate(),
            0,
            opts(),
            Box::new(storage.clone()),
        );
        eng.server_known_vv = vv;
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);
        eng.doc_mut().add_item(LIST_INBOX, "stranded").unwrap();
        eng.flush();
        let first_push = eng.pop_outbox().unwrap();
        let first_id = match dec::<ClientFrame>(&first_push) {
            ClientFrame::PushOps { ops } => ops[0].push_id,
            other => panic!("expected PushOps, got {other:?}"),
        };
        let _ = drain_events(&mut eng);

        eng.handle_disconnected();
        assert!(!eng.is_online());
        // The durable record survives the disconnect.
        assert_eq!(storage.in_flight().map(|p| p.push_id.0), Some(first_id));

        // Re-connect, run an empty pull; the retry ships the same id.
        eng.handle_connected();
        let _ = eng.pop_outbox().unwrap();
        eng.handle_server_bytes(&enc(&HelloAck {
            server_version: "s".into(),
            protocol_version: PROTOCOL_VERSION,
            epoch: 0,
        }));
        let _ = eng.pop_outbox().unwrap();
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: vec![],
            complete: true,
        }));
        let retry = eng.pop_outbox().expect("re-push after reconnect");
        match dec::<ClientFrame>(&retry) {
            ClientFrame::PushOps { ops } => assert_eq!(ops[0].push_id, first_id),
            other => panic!("expected PushOps, got {other:?}"),
        }

        // Ack completes it: record cleared, VV persisted.
        eng.handle_server_bytes(&enc(&ack_frame(&retry, 1)));
        assert!(storage.in_flight().is_none());
        assert!(!storage.server_known_vv().is_empty());
    }

    #[test]
    fn seeded_in_flight_push_from_boot_is_resent() {
        // A crashed session's durable push record, seeded via
        // `seed_boot`, is retried verbatim on the next connection.
        let dek = Dek::generate();
        let mut doc = Doc::new().unwrap();
        doc.add_item(LIST_INBOX, "crashed-mid-push").unwrap();
        let blob = doc.pending_export(&dek).unwrap().unwrap();
        let to_vv = doc.oplog_vv();
        doc.mark_persisted();

        let push = InFlightPush {
            push_id: PushId::generate(),
            payload: blob,
            from_vv: VersionVector::default().encode(),
            to_vv: to_vv.encode(),
        };
        let meta = BootMeta {
            in_flight_push: Some(push.clone()),
            ..Default::default()
        };
        let mut eng = SyncEngine::new(doc, fake_doc_id(), dek, 0, opts(), mem());
        eng.seed_boot(&meta);

        // Pull-complete auto-starts the retry, so the engine ends in
        // Pushing rather than Idle.
        eng.handle_connected();
        let _hello = eng.pop_outbox().unwrap();
        eng.handle_server_bytes(&enc(&HelloAck {
            server_version: "s".into(),
            protocol_version: PROTOCOL_VERSION,
            epoch: 0,
        }));
        let _pull = eng.pop_outbox().unwrap();
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: vec![],
            complete: true,
        }));
        let retry = eng.pop_outbox().expect("seeded push resent");
        match dec::<ClientFrame>(&retry) {
            ClientFrame::PushOps { ops } => assert_eq!(ops[0].push_id, push.push_id.0),
            other => panic!("expected PushOps, got {other:?}"),
        }

        // Acking it merges to_vv → nothing further to push.
        eng.handle_server_bytes(&enc(&ack_frame(&retry, 1)));
        assert!(eng.is_idle());
        eng.flush();
        assert!(eng.pop_outbox().is_none(), "to_vv fully acked");
    }

    #[test]
    fn deduped_ack_with_original_seq_below_frontier_is_tolerated() {
        // Crash-retry flow: the pull already delivered our own op (the
        // server had inserted it), so the dedup ack's original seq is
        // below the frontier. It must not trip the contiguity
        // assertion or regress anything.
        let mut eng = fresh_engine_clean();
        let dek = eng.dek.clone();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);

        eng.doc_mut().add_item(LIST_INBOX, "mine").unwrap();
        eng.flush();
        let push_bytes = eng.pop_outbox().expect("PushOps");

        // A peer broadcast lands first (seq 1), then our ack arrives
        // deduplicated at seq 2... then simulate the *retry* case by
        // acking at a seq we've already ingested: first ingest 1 via
        // broadcast, then ack with seq 2 (contiguous next) — and a
        // second scenario would ack ≤ frontier. Here: ack at 1 after
        // the broadcast already ingested 1.
        let remote_blob = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "peer").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBroadcast {
            ops: vec![StoredBlob {
                seq: 1,
                blob: remote_blob,
            }],
        }));
        assert_eq!(eng.last_contiguous_seq(), 1);
        // Dedup ack returns original seq 1 (≤ frontier) — tolerated.
        eng.handle_server_bytes(&enc(&ack_frame(&push_bytes, 1)));
        assert!(eng.is_idle());
        assert_eq!(eng.last_contiguous_seq(), 1);
        let events = drain_events(&mut eng);
        assert!(events.contains(&Event::Pushed));
        assert!(
            !events.iter().any(|e| matches!(e, Event::Error(_))),
            "no error on deduped ack: {events:?}"
        );
    }

    #[test]
    fn duplicate_remote_delivery_still_advances_server_known_vv() {
        // Spec test 6: a duplicate server update advances server
        // knowledge even when the Loro import is a no-op.
        let storage = std::sync::Arc::new(MemStorage::new());
        let mut doc = Doc::new().unwrap();
        doc.mark_persisted();
        let vv = doc.oplog_vv();
        let dek = Dek::generate();
        let mut eng = SyncEngine::new(
            doc,
            fake_doc_id(),
            dek.clone(),
            0,
            opts(),
            Box::new(storage.clone()),
        );
        eng.server_known_vv = vv.clone();
        drive_to_idle(&mut eng);

        let remote_blob = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "dup").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBroadcast {
            ops: vec![StoredBlob {
                seq: 1,
                blob: remote_blob.clone(),
            }],
        }));
        let after_first = eng.server_known_vv().clone();
        assert!(after_first != vv, "first delivery advanced the VV");

        // Wipe the in-memory VV to prove the duplicate *re-proves* it.
        eng.server_known_vv = vv.clone();
        // Re-deliver the same blob at the same seq (re-pull overlap).
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: vec![StoredBlob {
                seq: 1,
                blob: remote_blob,
            }],
            complete: false,
        }));
        assert_eq!(
            eng.server_known_vv().encode(),
            after_first.encode(),
            "duplicate delivery re-proved the range (import was a no-op)"
        );
        // And the WAL did not grow a second row for seq 1.
        assert_eq!(storage.wal_len(), 1);
    }

    #[test]
    fn server_bytes_while_disconnected_is_an_error() {
        let mut eng = fresh_engine_clean();
        eng.handle_server_bytes(b"\x00\x01");
        let evs = drain_events(&mut eng);
        assert!(matches!(evs.as_slice(), [Event::Error(_)]));
    }

    #[test]
    fn handle_connected_twice_emits_error() {
        let mut eng = fresh_engine_clean();
        eng.handle_connected();
        let _ = drain_events(&mut eng);
        eng.handle_connected();
        let evs = drain_events(&mut eng);
        assert!(matches!(evs.as_slice(), [Event::Error(_)]));
    }

    #[test]
    fn opsack_outside_pushing_emits_error() {
        let mut eng = fresh_engine_clean();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);
        eng.handle_server_bytes(&enc(&ServerFrame::OpsAck {
            acks: vec![PushAck {
                push_id: uuid::Uuid::new_v4(),
                seq: 42,
            }],
        }));
        let evs = drain_events(&mut eng);
        assert!(matches!(evs.as_slice(), [Event::Error(s)] if s.contains("OpsAck")));
    }

    #[test]
    fn since_seq_carries_persisted_frontier() {
        let mut doc = Doc::new().unwrap();
        doc.mark_persisted();
        let mut eng = SyncEngine::new(doc, fake_doc_id(), Dek::generate(), 42, opts(), mem());
        eng.handle_connected();
        let _hello = eng.pop_outbox().unwrap();
        eng.handle_server_bytes(&enc(&HelloAck {
            server_version: "s".into(),
            protocol_version: PROTOCOL_VERSION,
            epoch: 0,
        }));
        let pull: ClientFrame = dec(&eng.pop_outbox().unwrap());
        assert!(matches!(pull, ClientFrame::PullOps { since_seq: 42 }));
    }

    #[test]
    fn snapshot_request_produces_at_server_known_frontier_not_current_doc() {
        // Spec test 12: a server snapshot excludes the producer's
        // unsent local operations.
        let mut eng = fresh_engine_clean();
        let dek = eng.dek.clone();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);
        let _ = drain_outbox(&mut eng);

        // A synced peer op raises our frontier to seq 1.
        let peer_blob = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "synced-peer-item").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBroadcast {
            ops: vec![StoredBlob {
                seq: 1,
                blob: peer_blob,
            }],
        }));
        let _ = drain_outbox(&mut eng);

        // An UNSENT local mutation — beyond `server_known_vv`.
        let unsent_id = eng.doc_mut().add_item(LIST_INBOX, "unsent-local").unwrap();

        eng.handle_server_bytes(&enc(&ServerFrame::SnapshotRequest {
            up_to_seq: 1,
            compaction_floor_seq: 0,
            base: None,
        }));
        let push: ClientFrame = dec(&eng.pop_outbox().expect("PushSnapshot"));
        let (tagged_up_to, tagged_floor, from_seq, up_to_vv, blob) = match push {
            ClientFrame::PushSnapshot {
                up_to_seq,
                compaction_floor_seq,
                from_seq,
                up_to_vv,
                blob,
            } => (up_to_seq, compaction_floor_seq, from_seq, up_to_vv, blob),
            other => panic!("expected PushSnapshot, got {other:?}"),
        };
        assert_eq!(tagged_up_to, 1);
        assert_eq!(tagged_floor, 0);
        assert_eq!(from_seq, None);
        // The sealed VV is the server-known frontier, not the doc's.
        let vv = open_vv(&dek, &up_to_vv).unwrap();
        assert_eq!(vv, eng.server_known_vv);
        assert!(!vv.includes_vv(&VersionVector::decode(&eng.doc().oplog_vv_bytes()).unwrap()));

        // The snapshot contains the synced peer item but NOT the
        // unsent local op.
        let mut peer = Doc::empty();
        peer.apply_remote(&dek, &blob).unwrap();
        assert!(
            peer.items_in_list(LIST_INBOX, false)
                .iter()
                .any(|i| i.text == "synced-peer-item"),
            "synced history must be in the server snapshot"
        );
        assert!(
            peer.get_item(&unsent_id).is_none(),
            "unsent local op leaked into the server snapshot"
        );
        // The producer's own doc still has the unsent op.
        assert!(eng.doc().get_item(&unsent_id).is_some());
    }

    #[test]
    fn delta_snapshot_request_exports_only_ops_after_base() {
        // Two synced peer ops (seq 1, 2). The server holds the VV at
        // seq 1 from an earlier full snapshot and asks for a delta
        // from it. The delta must carry seq 2's ops, nothing from seq
        // 1, and nothing unsent.
        let mut eng = fresh_engine_clean();
        let dek = eng.dek.clone();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);
        let _ = drain_outbox(&mut eng);

        let first = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "first").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBroadcast {
            ops: vec![StoredBlob {
                seq: 1,
                blob: first,
            }],
        }));
        let base_vv = eng.server_known_vv.clone();
        let full_at_base = eng.doc().snapshot_blob_at(&dek, &base_vv).unwrap();

        let second = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "second").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBroadcast {
            ops: vec![StoredBlob {
                seq: 2,
                blob: second,
            }],
        }));
        let _ = drain_outbox(&mut eng);
        let unsent_id = eng.doc_mut().add_item(LIST_INBOX, "unsent-local").unwrap();

        eng.handle_server_bytes(&enc(&ServerFrame::SnapshotRequest {
            up_to_seq: 2,
            compaction_floor_seq: 0,
            base: Some(SnapshotBase {
                seq: 1,
                vv: seal_vv(&dek, &base_vv).unwrap(),
            }),
        }));
        let push: ClientFrame = dec(&eng.pop_outbox().expect("PushSnapshot"));
        let (up_to, from_seq, up_to_vv, delta) = match push {
            ClientFrame::PushSnapshot {
                up_to_seq,
                from_seq,
                up_to_vv,
                blob,
                ..
            } => (up_to_seq, from_seq, up_to_vv, blob),
            other => panic!("expected PushSnapshot, got {other:?}"),
        };
        assert_eq!(up_to, 2);
        assert_eq!(from_seq, Some(1));
        assert_eq!(open_vv(&dek, &up_to_vv).unwrap(), eng.server_known_vv);

        // The delta alone is strictly the (base, frontier] range: it
        // declares nothing the base already covered.
        let plaintext = dek.open(&delta.ciphertext, &delta.nonce).unwrap();
        let meta = loro::LoroDoc::decode_import_blob_meta(&plaintext, false).unwrap();
        assert!(base_vv.includes_vv(&meta.partial_start_vv));
        assert!(eng.server_known_vv.includes_vv(&meta.partial_end_vv));
        assert!(!base_vv.includes_vv(&meta.partial_end_vv));

        // Chain = full-at-base + delta reproduces the server frontier.
        let mut peer = Doc::empty();
        peer.apply_remote_batch(&dek, [&full_at_base, &delta])
            .unwrap();
        let texts: Vec<String> = peer
            .items_in_list(LIST_INBOX, false)
            .into_iter()
            .map(|i| i.text)
            .collect();
        assert!(texts.contains(&"first".to_string()));
        assert!(texts.contains(&"second".to_string()));
        assert!(
            peer.get_item(&unsent_id).is_none(),
            "unsent op leaked into the delta"
        );
    }

    #[test]
    fn delta_request_with_base_outside_frontier_is_declined() {
        // A base VV we don't hold (another doc's history) must not
        // produce a blob — that would fork the server's chain.
        let mut eng = fresh_engine_clean();
        let dek = eng.dek.clone();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);
        let _ = drain_outbox(&mut eng);

        let foreign = Doc::new().unwrap();
        foreign.add_item(LIST_INBOX, "foreign").unwrap();
        let foreign_vv = VersionVector::decode(&foreign.oplog_vv_bytes()).unwrap();
        assert!(!foreign_vv.is_empty());
        eng.handle_server_bytes(&enc(&ServerFrame::SnapshotRequest {
            up_to_seq: 0,
            compaction_floor_seq: 0,
            base: Some(SnapshotBase {
                seq: 7,
                vv: seal_vv(&dek, &foreign_vv).unwrap(),
            }),
        }));
        assert!(
            eng.pop_outbox().is_none(),
            "no PushSnapshot for a foreign base"
        );
        let evs = drain_events(&mut eng);
        assert!(matches!(evs.as_slice(), [Event::Error(s)] if s.contains("declining")));
    }

    #[test]
    fn unsolicited_snapshot_in_idle_is_an_error() {
        let mut eng = fresh_engine_clean();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);
        eng.handle_server_bytes(&enc(&ServerFrame::Snapshot {
            up_to_seq: 99,
            blob: EncryptedBlob {
                nonce: vec![0; 24],
                ciphertext: vec![],
            },
            complete: true,
        }));
        let evs = drain_events(&mut eng);
        assert!(matches!(evs.as_slice(), [Event::Error(s)] if s.contains("Snapshot")));
    }

    #[test]
    fn snapshot_required_in_pulling_drives_bootstrap() {
        // Wire:  HelloAck -> PullOps(since=0) -> SnapshotRequired
        //   ->   PullSnapshot -> Snapshot -> PullOps(since=up_to)
        //   ->   OpsBatch{complete} -> Idle
        let dek = Dek::generate();
        let storage = std::sync::Arc::new(MemStorage::new());
        let mut eng = SyncEngine::new(
            Doc::empty(),
            fake_doc_id(),
            dek.clone(),
            0,
            opts(),
            Box::new(storage.clone()),
        );
        eng.handle_connected();
        let _ = eng.pop_outbox().unwrap();
        eng.handle_server_bytes(&enc(&HelloAck {
            server_version: "s".into(),
            protocol_version: PROTOCOL_VERSION,
            epoch: 0,
        }));
        let _ = eng.pop_outbox().unwrap(); // PullOps

        // Server says cursor is below the floor.
        eng.handle_server_bytes(&enc(&ServerFrame::SnapshotRequired { up_to_seq: 42 }));
        let pull_snap: ClientFrame = dec(&eng.pop_outbox().expect("PullSnapshot"));
        assert!(matches!(pull_snap, ClientFrame::PullSnapshot));
        assert!(!eng.is_idle());

        let snapshot_blob = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "from-snapshot").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::Snapshot {
            up_to_seq: 42,
            blob: snapshot_blob.clone(),
            complete: true,
        }));

        // The local checkpoint (a re-export of the merged doc) is
        // persisted, `server_known_vv` covers the snapshot's declared
        // frontier, and the WAL is empty.
        let boot = storage.boot(fake_doc_id()).unwrap();
        let persisted = boot.snapshot.expect("bootstrap checkpoint persisted");
        let mut reloaded = Doc::empty();
        reloaded.apply_remote(&dek, &persisted.payload).unwrap();
        assert!(
            reloaded
                .items_in_list(LIST_INBOX, false)
                .iter()
                .any(|item| item.text == "from-snapshot")
        );
        assert!(boot.replay.is_empty());
        assert!(
            !boot.server_known_vv.is_empty(),
            "VV persisted at bootstrap"
        );

        // Engine advanced its frontier and re-issued PullOps. The Ack
        // waits for the host durability confirm.
        assert_eq!(eng.last_contiguous_seq(), 42);
        assert_eq!(eng.last_durable_seq(), 0);
        let mut frames: Vec<ClientFrame> = Vec::new();
        while let Some(b) = eng.pop_outbox() {
            frames.push(dec(&b));
        }
        assert!(
            !frames.iter().any(|f| matches!(f, ClientFrame::Ack { .. })),
            "Ack queued before notify_oplog_durable: {frames:?}",
        );
        assert!(
            frames
                .iter()
                .any(|f| matches!(f, ClientFrame::PullOps { since_seq: 42 }))
        );

        eng.notify_oplog_durable(42);
        let ack: ClientFrame = dec(&eng.pop_outbox().expect("Ack"));
        assert!(matches!(ack, ClientFrame::Ack { last_acked_seq: 42 }));

        assert!(
            eng.doc()
                .items_in_list(LIST_INBOX, false)
                .iter()
                .any(|i| i.text == "from-snapshot")
        );

        eng.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: vec![],
            complete: true,
        }));
        assert!(eng.is_idle());
    }

    #[test]
    fn bootstrap_checkpoint_preserves_unsent_local_work() {
        // A device with unsent local ops receives a server bootstrap
        // snapshot. The new checkpoint prunes the whole WAL, so it must
        // be a re-export of the MERGED doc — and the next push must
        // still derive the unsent ops.
        let dek = Dek::generate();
        let storage = std::sync::Arc::new(MemStorage::new());
        let doc = Doc::new().unwrap();
        let local_id = doc.add_item(LIST_INBOX, "pending-local").unwrap();
        let mut eng = SyncEngine::new(
            doc,
            fake_doc_id(),
            dek.clone(),
            0,
            opts(),
            Box::new(storage.clone()),
        );
        // Capture the pending work into the WAL (as a real session
        // would have).
        eng.capture_local_ops().unwrap();
        assert_eq!(storage.wal_len(), 1);

        eng.handle_connected();
        let _ = eng.pop_outbox().unwrap();
        eng.handle_server_bytes(&enc(&HelloAck {
            server_version: "s".into(),
            protocol_version: PROTOCOL_VERSION,
            epoch: 0,
        }));
        let _ = eng.pop_outbox().unwrap(); // PullOps
        eng.handle_server_bytes(&enc(&ServerFrame::SnapshotRequired { up_to_seq: 42 }));
        let _ = eng.pop_outbox().unwrap(); // PullSnapshot
        let snapshot_blob = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "from-snapshot").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::Snapshot {
            up_to_seq: 42,
            blob: snapshot_blob,
            complete: true,
        }));

        // The WAL is fully pruned, but the checkpoint contains the
        // unsent local op.
        let boot = storage.boot(fake_doc_id()).unwrap();
        assert!(boot.replay.is_empty());
        let mut reloaded = Doc::empty();
        reloaded
            .apply_remote(&dek, &boot.snapshot.unwrap().payload)
            .unwrap();
        assert!(
            reloaded.get_item(&local_id).is_some(),
            "unsent local op must survive the bootstrap checkpoint"
        );

        // The Snapshot frame queued the resume PullOps — drain it
        // before finishing the catch-up.
        let resume: ClientFrame = dec(&eng.pop_outbox().expect("resume PullOps"));
        assert!(matches!(resume, ClientFrame::PullOps { since_seq: 42 }));
        // Finish the catch-up; the push derives the unsent op.
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: vec![],
            complete: true,
        }));
        let push = eng.pop_outbox().expect("push of unsent local op");
        let blob = match dec::<ClientFrame>(&push) {
            ClientFrame::PushOps { ops } => ops.into_iter().next().unwrap().blob,
            other => panic!("expected PushOps, got {other:?}"),
        };
        // Applied on top of the snapshot state, the delta must carry
        // the local op.
        reloaded.apply_remote(&dek, &blob).unwrap();
        assert!(reloaded.get_item(&local_id).is_some());
    }

    #[test]
    fn opsbroadcast_during_bootstrap_is_dropped() {
        let dek = Dek::generate();
        let mut eng = SyncEngine::new(Doc::empty(), fake_doc_id(), dek.clone(), 0, opts(), mem());
        eng.handle_connected();
        let _ = eng.pop_outbox().unwrap();
        eng.handle_server_bytes(&enc(&HelloAck {
            server_version: "s".into(),
            protocol_version: PROTOCOL_VERSION,
            epoch: 0,
        }));
        let _ = eng.pop_outbox().unwrap(); // PullOps
        eng.handle_server_bytes(&enc(&ServerFrame::SnapshotRequired { up_to_seq: 10 }));
        let _ = eng.pop_outbox().unwrap(); // PullSnapshot

        let stray = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "should-not-appear").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBroadcast {
            ops: vec![StoredBlob {
                seq: 11,
                blob: stray,
            }],
        }));
        assert!(
            !eng.doc()
                .items_in_list(LIST_INBOX, false)
                .iter()
                .any(|i| i.text == "should-not-appear"),
            "broadcast applied during Bootstrapping",
        );
        assert_eq!(eng.last_contiguous_seq(), 0, "frontier must not advance");
    }

    #[test]
    fn produce_then_bootstrap_round_trip() {
        // Full producer/consumer loop with a fake server in between:
        //   A pushes ops; server stores them.
        //   Server asks A for a snapshot; A produces PushSnapshot at
        //   its server-known frontier.
        //   B (fresh) connects with since=0; below the snapshot
        //   floor, so server replies SnapshotRequired -> Snapshot.
        //   B applies, pulls past-the-snapshot ops, reaches Idle.
        //   Fingerprints converge.
        let dek = Dek::generate();
        let mut a = SyncEngine::new(
            Doc::new().unwrap(),
            fake_doc_id(),
            dek.clone(),
            0,
            opts(),
            mem(),
        );
        let mut b = {
            let mut doc = Doc::empty();
            doc.mark_persisted();
            SyncEngine::new(doc, fake_doc_id(), dek.clone(), 0, opts(), mem())
        };

        // Fake server state.
        let mut next_seq: u64 = 0;
        let mut ops_log: Vec<StoredBlob> = Vec::new();

        // -- A connects; the pull-complete triggers its seed push --
        a.handle_connected();
        let _ = a.pop_outbox().unwrap();
        a.handle_server_bytes(&enc(&HelloAck {
            server_version: "s".into(),
            protocol_version: PROTOCOL_VERSION,
            epoch: 0,
        }));
        let _ = a.pop_outbox().unwrap();
        a.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: vec![],
            complete: true,
        }));
        // Drain A's pushes; store + ack each.
        while let Some(bytes) = a.pop_outbox() {
            if let Ok(ClientFrame::PushOps { ops }) = rmp_serde::from_slice::<ClientFrame>(&bytes) {
                let mut acks = Vec::new();
                for op in ops {
                    next_seq += 1;
                    ops_log.push(StoredBlob {
                        seq: next_seq,
                        blob: op.blob,
                    });
                    acks.push(PushAck {
                        push_id: op.push_id,
                        seq: next_seq,
                    });
                }
                a.handle_server_bytes(&enc(&ServerFrame::OpsAck { acks }));
            }
        }

        // -- A makes a real-content change --
        let item_id = a.doc_mut().add_item(LIST_INBOX, "snapshotted").unwrap();
        a.flush();
        while let Some(bytes) = a.pop_outbox() {
            if let Ok(ClientFrame::PushOps { ops }) = rmp_serde::from_slice::<ClientFrame>(&bytes) {
                let mut acks = Vec::new();
                for op in ops {
                    next_seq += 1;
                    ops_log.push(StoredBlob {
                        seq: next_seq,
                        blob: op.blob,
                    });
                    acks.push(PushAck {
                        push_id: op.push_id,
                        seq: next_seq,
                    });
                }
                a.handle_server_bytes(&enc(&ServerFrame::OpsAck { acks }));
            }
        }
        let _ = drain_outbox(&mut a);
        assert_eq!(a.last_contiguous_seq(), next_seq);

        // -- Server requests a full snapshot from A --
        a.handle_server_bytes(&enc(&ServerFrame::SnapshotRequest {
            up_to_seq: next_seq,
            compaction_floor_seq: next_seq,
            base: None,
        }));
        let push: ClientFrame = dec(&a.pop_outbox().expect("PushSnapshot"));
        let (full_up_to, snapshot_floor, full_from, full_vv, full_blob) = match push {
            ClientFrame::PushSnapshot {
                up_to_seq,
                compaction_floor_seq,
                from_seq,
                up_to_vv,
                blob,
            } => (up_to_seq, compaction_floor_seq, from_seq, up_to_vv, blob),
            other => panic!("expected PushSnapshot, got {other:?}"),
        };
        assert_eq!(full_up_to, next_seq);
        assert_eq!(snapshot_floor, next_seq);
        assert_eq!(full_from, None);

        // -- A mutates again; the server then asks for a *delta* from
        //    the full snapshot's stored VV. --
        let mid_id = a.doc_mut().add_item(LIST_INBOX, "in-delta").unwrap();
        a.flush();
        while let Some(bytes) = a.pop_outbox() {
            if let Ok(ClientFrame::PushOps { ops }) = rmp_serde::from_slice::<ClientFrame>(&bytes) {
                let mut acks = Vec::new();
                for op in ops {
                    next_seq += 1;
                    ops_log.push(StoredBlob {
                        seq: next_seq,
                        blob: op.blob,
                    });
                    acks.push(PushAck {
                        push_id: op.push_id,
                        seq: next_seq,
                    });
                }
                a.handle_server_bytes(&enc(&ServerFrame::OpsAck { acks }));
            }
        }
        let _ = drain_outbox(&mut a);
        assert!(
            next_seq > full_up_to,
            "expected an op after the full snapshot"
        );
        a.handle_server_bytes(&enc(&ServerFrame::SnapshotRequest {
            up_to_seq: next_seq,
            compaction_floor_seq: snapshot_floor,
            base: Some(SnapshotBase {
                seq: full_up_to,
                vv: full_vv,
            }),
        }));
        let push: ClientFrame = dec(&a.pop_outbox().expect("PushSnapshot"));
        let (snapshot_up_to, delta_from, delta_blob) = match push {
            ClientFrame::PushSnapshot {
                up_to_seq,
                from_seq,
                blob,
                ..
            } => (up_to_seq, from_seq, blob),
            other => panic!("expected PushSnapshot, got {other:?}"),
        };
        assert_eq!(snapshot_up_to, next_seq);
        assert_eq!(delta_from, Some(full_up_to));

        // -- A keeps mutating after the delta was taken, so B's
        //    bootstrap exercises the chain apply *and* the
        //    post-snapshot catch-up via OpsBatch. --
        let post_snap_id = a.doc_mut().add_item(LIST_INBOX, "post-snap").unwrap();
        a.flush();
        let mut post_snap_ops: Vec<StoredBlob> = Vec::new();
        while let Some(bytes) = a.pop_outbox() {
            if let Ok(ClientFrame::PushOps { ops }) = rmp_serde::from_slice::<ClientFrame>(&bytes) {
                let mut acks = Vec::new();
                for op in ops {
                    next_seq += 1;
                    post_snap_ops.push(StoredBlob {
                        seq: next_seq,
                        blob: op.blob.clone(),
                    });
                    ops_log.push(StoredBlob {
                        seq: next_seq,
                        blob: op.blob,
                    });
                    acks.push(PushAck {
                        push_id: op.push_id,
                        seq: next_seq,
                    });
                }
                a.handle_server_bytes(&enc(&ServerFrame::OpsAck { acks }));
            }
        }
        let _ = drain_outbox(&mut a);
        assert!(
            !post_snap_ops.is_empty(),
            "expected at least one post-snapshot op"
        );

        // -- B connects fresh, since=0 < snapshot floor --
        b.handle_connected();
        let _ = b.pop_outbox().unwrap();
        b.handle_server_bytes(&enc(&HelloAck {
            server_version: "s".into(),
            protocol_version: PROTOCOL_VERSION,
            epoch: 0,
        }));
        let pull: ClientFrame = dec(&b.pop_outbox().unwrap());
        assert!(matches!(pull, ClientFrame::PullOps { since_seq: 0 }));

        b.handle_server_bytes(&enc(&ServerFrame::SnapshotRequired {
            up_to_seq: snapshot_up_to,
        }));
        let pull_snap: ClientFrame = dec(&b.pop_outbox().expect("PullSnapshot"));
        assert!(matches!(pull_snap, ClientFrame::PullSnapshot));

        // The chain arrives one row per frame; nothing resumes until
        // the `complete` row lands.
        b.handle_server_bytes(&enc(&ServerFrame::Snapshot {
            up_to_seq: full_up_to,
            blob: full_blob,
            complete: false,
        }));
        assert!(
            b.pop_outbox().is_none(),
            "no PullOps before the chain completes"
        );
        b.handle_server_bytes(&enc(&ServerFrame::Snapshot {
            up_to_seq: snapshot_up_to,
            blob: delta_blob,
            complete: true,
        }));

        // B should re-issue PullOps from the snapshot's up_to.
        let mut saw_resume_pull = false;
        while let Some(bytes) = b.pop_outbox() {
            if let Ok(frame) = rmp_serde::from_slice::<ClientFrame>(&bytes)
                && matches!(frame, ClientFrame::PullOps { since_seq } if since_seq == snapshot_up_to)
            {
                saw_resume_pull = true;
            }
        }
        assert!(saw_resume_pull, "B must PullOps from snapshot.up_to");

        b.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: post_snap_ops,
            complete: true,
        }));
        assert!(b.is_idle());

        assert!(
            b.doc().get_item(&item_id).is_some(),
            "snapshot item missing"
        );
        assert!(b.doc().get_item(&mid_id).is_some(), "delta item missing");
        assert!(
            b.doc().get_item(&post_snap_id).is_some(),
            "post-snapshot item missing",
        );
        assert_eq!(a.doc().fingerprint(), b.doc().fingerprint());
    }

    #[test]
    fn snapshot_required_outside_pulling_is_an_error() {
        let mut eng = fresh_engine_clean();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);
        eng.handle_server_bytes(&enc(&ServerFrame::SnapshotRequired { up_to_seq: 1 }));
        let evs = drain_events(&mut eng);
        assert!(matches!(evs.as_slice(), [Event::Error(s)] if s.contains("SnapshotRequired")));
    }

    #[test]
    fn notify_oplog_durable_clamps_to_last_contiguous() {
        let mut eng = fresh_engine_clean();
        let dek = eng.dek.clone();
        drive_to_idle(&mut eng);
        let _ = drain_outbox(&mut eng);
        let _ = drain_events(&mut eng);
        let blob = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "x").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBroadcast {
            ops: vec![StoredBlob { seq: 1, blob }],
        }));
        assert_eq!(eng.last_contiguous_seq(), 1);
        eng.notify_oplog_durable(999);
        assert_eq!(eng.last_durable_seq(), 1);
        let ack: ClientFrame = dec(&eng.pop_outbox().expect("Ack"));
        assert!(matches!(ack, ClientFrame::Ack { last_acked_seq: 1 }));
    }

    #[test]
    fn notify_oplog_durable_is_monotonic_and_coalesces() {
        let mut eng = fresh_engine_clean();
        let dek = eng.dek.clone();
        drive_to_idle(&mut eng);
        let _ = drain_outbox(&mut eng);
        let _ = drain_events(&mut eng);
        let b1 = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "a").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBroadcast {
            ops: vec![StoredBlob { seq: 1, blob: b1 }],
        }));
        assert!(eng.pop_outbox().is_none());

        eng.notify_oplog_durable(1);
        let _ack: ClientFrame = dec(&eng.pop_outbox().expect("first Ack"));

        eng.notify_oplog_durable(1);
        assert!(eng.pop_outbox().is_none());

        eng.notify_oplog_durable(0);
        assert_eq!(eng.last_durable_seq(), 1);
        assert!(eng.pop_outbox().is_none());
    }

    #[test]
    fn inbound_apply_does_not_queue_ack_until_durable() {
        let mut eng = fresh_engine_clean();
        let dek = eng.dek.clone();
        drive_to_idle(&mut eng);
        let _ = drain_outbox(&mut eng);
        let _ = drain_events(&mut eng);

        let blob = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "remote").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBroadcast {
            ops: vec![StoredBlob { seq: 1, blob }],
        }));

        assert_eq!(eng.last_contiguous_seq(), 1);
        assert_eq!(eng.last_durable_seq(), 0);
        while let Some(bytes) = eng.pop_outbox() {
            let frame: ClientFrame = dec(&bytes);
            assert!(
                !matches!(frame, ClientFrame::Ack { .. }),
                "outbox carried an Ack before notify_oplog_durable: {frame:?}",
            );
        }
    }

    #[test]
    fn malformed_server_frame_emits_error_without_disconnect() {
        let mut eng = fresh_engine_clean();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);
        eng.handle_server_bytes(b"not msgpack at all");
        let evs = drain_events(&mut eng);
        assert!(matches!(evs.as_slice(), [Event::Error(_)]));
        assert!(eng.is_idle());
    }

    #[test]
    fn end_to_end_two_engines_converge_via_engine_loop() {
        let dek = Dek::generate();
        let mut a = SyncEngine::new(
            Doc::new().unwrap(),
            fake_doc_id(),
            dek.clone(),
            0,
            opts(),
            mem(),
        );
        let mut b = {
            let mut doc = Doc::empty();
            doc.mark_persisted();
            SyncEngine::new(doc, fake_doc_id(), dek.clone(), 0, opts(), mem())
        };

        let mut next_seq: u64 = 0;
        let mut ops_log: Vec<StoredBlob> = Vec::new();

        // -- A connects; pull-complete pushes its seed --
        a.handle_connected();
        let _ = a.pop_outbox().unwrap();
        a.handle_server_bytes(&enc(&HelloAck {
            server_version: "s".into(),
            protocol_version: PROTOCOL_VERSION,
            epoch: 0,
        }));
        let _ = a.pop_outbox().unwrap();
        a.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: vec![],
            complete: true,
        }));
        while let Some(bytes) = a.pop_outbox() {
            if let Ok(ClientFrame::PushOps { ops }) = rmp_serde::from_slice::<ClientFrame>(&bytes) {
                let mut acks = Vec::new();
                for op in ops {
                    next_seq += 1;
                    ops_log.push(StoredBlob {
                        seq: next_seq,
                        blob: op.blob,
                    });
                    acks.push(PushAck {
                        push_id: op.push_id,
                        seq: next_seq,
                    });
                }
                a.handle_server_bytes(&enc(&ServerFrame::OpsAck { acks }));
            }
        }
        let _ = drain_outbox(&mut a);

        // -- A makes a local change and pushes --
        let item_a = a.doc_mut().add_item(LIST_INBOX, "from-a").unwrap();
        a.flush();
        while let Some(bytes) = a.pop_outbox() {
            if let Ok(ClientFrame::PushOps { ops }) = rmp_serde::from_slice::<ClientFrame>(&bytes) {
                let mut acks = Vec::new();
                for op in ops {
                    next_seq += 1;
                    ops_log.push(StoredBlob {
                        seq: next_seq,
                        blob: op.blob,
                    });
                    acks.push(PushAck {
                        push_id: op.push_id,
                        seq: next_seq,
                    });
                }
                a.handle_server_bytes(&enc(&ServerFrame::OpsAck { acks }));
            }
        }
        let _ = drain_outbox(&mut a);

        // -- B connects, pulls everything from the log --
        b.handle_connected();
        let _ = b.pop_outbox().unwrap();
        b.handle_server_bytes(&enc(&HelloAck {
            server_version: "s".into(),
            protocol_version: PROTOCOL_VERSION,
            epoch: 0,
        }));
        let _ = b.pop_outbox().unwrap();
        b.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: ops_log.clone(),
            complete: true,
        }));
        assert!(b.doc().get_item(&item_a).is_some());
        assert_eq!(a.doc().fingerprint(), b.doc().fingerprint());
    }

    #[test]
    fn remote_ops_append_wal_rows_and_count_toward_threshold() {
        // Spec test 4: remote updates enter the WAL and trigger local
        // snapshotting through the same threshold as local rows.
        let (mut eng, mem) = engine_with_mem();
        let dek = eng.dek.clone();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);

        let blob1 = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "from peer A").unwrap();
        });
        let blob2 = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "from peer B").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBatch {
            ops: vec![
                StoredBlob {
                    seq: 1,
                    blob: blob1,
                },
                StoredBlob {
                    seq: 2,
                    blob: blob2,
                },
            ],
            complete: true,
        }));

        let boot = mem.boot(fake_doc_id()).unwrap();
        assert_eq!(boot.last_local_seq, LocalSeq(2), "two WAL rows expected");
        assert_eq!(eng.wal_rows(), 2);
        assert_eq!(eng.last_contiguous_seq(), 2);
        // The *persisted* resume cursor must NOT move on append alone.
        assert_eq!(
            boot.last_acked_server_seq,
            ServerSeq(0),
            "append must not persist the cursor",
        );
        // But `server_known_vv` was persisted atomically with the rows —
        // the cursor can never overtake it (spec test 11's ordering).
        assert!(!boot.server_known_vv.is_empty());
        eng.notify_oplog_durable(2);
        assert_eq!(
            mem.boot(fake_doc_id()).unwrap().last_acked_server_seq,
            ServerSeq(2),
            "durability signal persists the contiguous cursor",
        );

        // The remote rows trip the row threshold.
        assert!(eng.snapshot_if_wal_exceeds(2, u64::MAX).unwrap());
        assert_eq!(mem.wal_len(), 0);
    }

    fn engine_with_mem() -> (SyncEngine, std::sync::Arc<MemStorage>) {
        let mut doc = Doc::new().unwrap();
        doc.mark_persisted();
        let vv = doc.oplog_vv();
        let mem = std::sync::Arc::new(MemStorage::new());
        let mut eng = SyncEngine::new(
            doc,
            fake_doc_id(),
            Dek::generate(),
            0,
            opts(),
            Box::new(std::sync::Arc::clone(&mem)),
        );
        eng.server_known_vv = vv;
        (eng, mem)
    }

    #[test]
    fn mixed_history_exports_only_beyond_server_known_vv() {
        // Spec test 5: with mixed local and remote history, the push
        // delta carries only operations beyond `server_known_vv`.
        let (mut eng, _mem) = engine_with_mem();
        let dek = eng.dek.clone();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);

        // Remote op arrives (seq 1) — proven server history.
        let remote_blob = make_remote_blob(&dek, |d| {
            d.add_item(LIST_INBOX, "remote-item").unwrap();
        });
        eng.handle_server_bytes(&enc(&ServerFrame::OpsBroadcast {
            ops: vec![StoredBlob {
                seq: 1,
                blob: remote_blob,
            }],
        }));
        let _ = drain_outbox(&mut eng);

        // Local mutation — beyond the VV.
        eng.doc_mut().add_item(LIST_INBOX, "local-item").unwrap();
        eng.flush();
        let push = eng.pop_outbox().expect("PushOps");
        let blob = match dec::<ClientFrame>(&push) {
            ClientFrame::PushOps { ops } => ops.into_iter().next().unwrap().blob,
            other => panic!("expected PushOps, got {other:?}"),
        };

        // A peer that already has the remote history imports ONLY the
        // local op from our delta — if the delta re-carried the remote
        // ops, the declared range would include the remote peer.
        let plaintext_meta = {
            let plain = dek.open(&blob.ciphertext, &blob.nonce).unwrap();
            loro::LoroDoc::decode_import_blob_meta(&plain, false).unwrap()
        };
        // The delta's declared start must not be genesis for peers the
        // server already knows — i.e. the blob must not span the
        // remote peer's ops at all. The simplest check: every peer in
        // the delta is absent from... the remote blob's peers, except
        // our own. Equivalent practical assertion: applying the delta
        // to a doc that lacks the remote history leaves it pending
        // (missing deps) or the delta's peer set == our local peer only.
        assert_eq!(
            plaintext_meta.partial_start_vv.len(),
            plaintext_meta.partial_end_vv.len(),
        );
        // Strongest form: the local doc's own peer is the only one
        // whose counter range the delta covers beyond server_known_vv.
        for (peer, end) in plaintext_meta.partial_end_vv.iter() {
            let known = eng.server_known_vv().get(peer).copied().unwrap_or(0);
            let start = plaintext_meta
                .partial_start_vv
                .get(peer)
                .copied()
                .unwrap_or(0);
            assert!(
                *end > known || start >= known,
                "delta re-ships fully-known range for peer {peer}: start {start}, end {end}, known {known}"
            );
        }
    }

    #[test]
    fn disconnect_mid_push_keeps_durable_record() {
        let (mut eng, mem) = engine_with_mem();
        drive_to_idle(&mut eng);
        let _ = drain_events(&mut eng);

        eng.doc_mut().add_item(LIST_INBOX, "stranded op").unwrap();
        eng.flush();
        let _push = eng.pop_outbox().expect("PushOps");
        assert!(mem.in_flight().is_some());

        eng.handle_disconnected();

        // The durable record survives — reconnect retries the same
        // push_id (server dedups).
        assert!(
            mem.in_flight().is_some(),
            "in-flight record must survive disconnect"
        );
    }
}
