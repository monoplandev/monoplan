//! Local-storage trait shared by every Monoplan client.
//!
//! Local persistence and outbound sync are **separate concerns**
//! (`spec/vv-wal-separation.md`):
//!
//!   - Crash recovery is snapshot + WAL: one encrypted full-history
//!     Loro snapshot per doc plus a bounded encrypted WAL of updates
//!     applied after it. Every commit — local mutation or applied
//!     remote op — is appended to the WAL; when the WAL crosses the
//!     host's threshold the engine folds it into a fresh snapshot and
//!     prunes the folded prefix. This happens regardless of
//!     online/sync status; a full Loro snapshot retains operation
//!     history, so unsent local operations survive folding and can be
//!     re-derived later.
//!   - What needs uploading is derived from Loro history against
//!     `server_known_vv` — the operations proven to exist in the
//!     server update stream — never from which WAL rows still exist.
//!   - `last_acked_server_seq` is the durable server-log delivery
//!     frontier (the resume `PullUpdates` cursor). Server seq stays the
//!     opaque delivery/resume/compaction coordinate; VVs never cross
//!     the wire in plaintext.
//!   - At most one durable in-flight push per doc records the exact
//!     encrypted blob (and its `push_id`) currently on the wire, so a
//!     crash between server insert and client ack retries the same
//!     `push_id` and the server deduplicates.
//!
//! Two implementations satisfy the trait:
//!
//!   - `SqliteStorage` (CLI, future native apps) — `rusqlite` against a
//!     file on disk. Writes are synchronously durable: the trait method
//!     returns only after the SQL transaction commits.
//!   - `IdbStorage` (web) — IndexedDB behind a wasm-bindgen `extern`
//!     interface. Writes update an in-memory mirror synchronously (so
//!     the trait method can return immediately) and the underlying IDB
//!     transaction flushes in the background. The engine learns about
//!     real durability via a separate callback so the server-side `Ack`
//!     frame isn't shipped until the bytes are actually on disk.
//!
//! Both impls live outside `core/`: native in
//! `crates/storage-sqlite`, web in `js/core/src/storage/idb-storage.ts`
//! plus `core/web/src/lib.rs`. `MemStorage` (this file) is the
//! in-memory test double used by `core` unit tests.
//!
//! See `spec/local-storage.md` for the schema and boot semantics.

use monoplan_protocol::EncryptedBlob;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::crypto::Dek;
use crate::doc::{Doc, DocError};

// ---------- newtypes ----------

/// Server-assigned doc identifier. UUID v7 bytes; matches
/// `server/spec/storage.md`'s `docs.id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DocId(pub Uuid);

/// Client-minted durable push identifier. Persisted with the in-flight
/// push record *before* the first send and transmitted in `PushUpdates`;
/// the server stores it per originating device and deduplicates
/// repeats (see `spec/sync-protocol.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PushId(pub Uuid);

impl PushId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Storage-assigned monotonic id within a doc's WAL. Dense (no gaps),
/// strictly increasing per insert. Native impls source this from the
/// sqlite primary key; web mints it from an in-memory counter.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct LocalSeq(pub u64);

/// Server-assigned per-account sequence number. Mirrors
/// `monoplan_protocol::StoredBlob::seq`. Dense within an account.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct ServerSeq(pub u64);

// ---------- row types ----------

/// An op that arrived from another device via the server, headed for
/// the WAL. Carries its `server_seq` for idempotent re-delivery
/// detection (resume re-pull, broadcast overlap) — not as a sync
/// coordinate; upload derivation is VV-based.
#[derive(Debug, Clone)]
pub struct RemoteWalRow {
    pub server_seq: ServerSeq,
    pub payload: EncryptedBlob,
}

/// One surviving WAL row, used only on boot to replay the doc up to
/// current state. Provenance (local vs remote) is irrelevant for
/// replay — both decrypt the same way via the DEK.
#[derive(Debug, Clone)]
pub struct WalRow {
    pub local_seq: LocalSeq,
    pub payload: EncryptedBlob,
}

/// The persisted snapshot, if any. `up_to_local_seq` is **not** a
/// replay cutoff — it's the local-counter high-water at the moment the
/// snapshot was written, kept only so `append_*` keeps minting
/// monotonic `local_seq`s after a prune deletes the rows that carried
/// the previous maximum. Every surviving WAL row is replayed on boot
/// regardless of its `local_seq`; `write_snapshot` already pruned
/// exactly the prefix the `payload` contains.
#[derive(Debug, Clone)]
pub struct SnapshotRow {
    pub up_to_local_seq: LocalSeq,
    pub payload: EncryptedBlob,
}

/// The durable in-flight push record: the exact encrypted update blob
/// currently (or last) on the wire, its identity, and the VV span it
/// covers. At most one per doc. Persisted *before* the first send;
/// cleared by `complete_push` on ack. A crash at any point in between
/// retries the same `push_id` on the next connection (after the pull),
/// and the server deduplicates.
///
/// `from_vv` / `to_vv` are encoded Loro VersionVectors. `to_vv` is the
/// value merged into `server_known_vv` on ack; `from_vv` is kept for
/// forensics/debugging (the blob was exported as `Updates(from_vv)`).
#[derive(Debug, Clone)]
pub struct InFlightPush {
    pub push_id: PushId,
    pub payload: EncryptedBlob,
    pub from_vv: Vec<u8>,
    pub to_vv: Vec<u8>,
}

/// Everything the engine needs at startup to reconstruct in-memory
/// state for one doc. Empty for a brand-new doc (`Default`).
#[derive(Debug, Clone, Default)]
pub struct BootState {
    /// Persisted snapshot, if one has been written.
    pub snapshot: Option<SnapshotRow>,
    /// Every surviving WAL row, in ascending `local_seq` order. The
    /// snapshot (if any) already pruned the prefix it contains, so the
    /// engine replays all of these on top of the snapshot. Empty for a
    /// fresh doc. Engine decrypts each and feeds the plaintext through
    /// Loro.
    pub replay: Vec<WalRow>,
    /// Highest `local_seq` ever assigned for this doc. The next
    /// append returns `LocalSeq(this + 1)`.
    pub last_local_seq: LocalSeq,
    /// Highest contiguous `server_seq` we've durably applied. Seeds
    /// `SyncEngine::last_contiguous_seq` / `last_durable_seq` — the
    /// `since_seq` of the resume `PullUpdates`.
    pub last_acked_server_seq: ServerSeq,
    /// Encoded `server_known_vv` — the Loro operations proven to exist
    /// in the server update stream. Empty when the doc has never synced.
    pub server_known_vv: Vec<u8>,
    /// The durable in-flight push, if a previous session crashed (or
    /// disconnected) with one outstanding.
    pub in_flight_push: Option<InFlightPush>,
    /// Total payload bytes across the surviving WAL rows — with
    /// `replay.len()`, seeds the engine's snapshot-threshold stats.
    pub wal_bytes: u64,
}

/// The engine-facing subset of [`BootState`] — everything except the
/// blobs the host already replayed into the `Doc`. Hosts hand this to
/// [`crate::SyncEngine::new`] so the engine starts with the persisted
/// cursors, VV, in-flight push, and WAL statistics.
#[derive(Debug, Clone, Default)]
pub struct BootMeta {
    pub last_local_seq: LocalSeq,
    pub last_acked_server_seq: ServerSeq,
    pub server_known_vv: Vec<u8>,
    pub in_flight_push: Option<InFlightPush>,
    pub wal_rows: u64,
    pub wal_bytes: u64,
}

impl BootState {
    /// Split off the engine-facing metadata (see [`BootMeta`]).
    pub fn meta(&self) -> BootMeta {
        BootMeta {
            last_local_seq: self.last_local_seq,
            last_acked_server_seq: self.last_acked_server_seq,
            server_known_vv: self.server_known_vv.clone(),
            in_flight_push: self.in_flight_push.clone(),
            wal_rows: self.replay.len() as u64,
            wal_bytes: self.wal_bytes,
        }
    }
}

// ---------- errors ----------

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// Backend-specific failure (sqlite error, IDB transaction abort,
    /// JS exception across the wasm boundary). Stringified at the
    /// boundary so the trait stays portable.
    #[error("storage backend: {0}")]
    Backend(String),
    /// Caller asked for a doc the storage has never seen. Engine
    /// treats this as "fresh doc, empty BootState."
    #[error("doc not found: {0:?}")]
    DocNotFound(DocId),
}

// ---------- trait ----------

/// Per-doc local persistence. Methods take `&self` so impls can use
/// interior mutability (sqlite `Mutex<Connection>`, JS handle held by
/// reference) — caller doesn't need `&mut`.
///
/// All methods are synchronous. Native impls are also synchronously
/// durable; the web impl returns from an in-memory mirror and flushes
/// IDB in the background (durability signalled out-of-band — see
/// `spec/local-storage.md`).
pub trait LocalStorage {
    /// Load everything the engine needs to bring a doc back to its
    /// last-persisted state. For a doc the storage has never seen,
    /// returns `Ok(BootState::default())` — not `Err(DocNotFound)` —
    /// so the first-boot path is a single happy path.
    fn boot(&self, doc_id: DocId) -> Result<BootState, StorageError>;

    /// Append a locally-originated update to the WAL. Returns the
    /// freshly-assigned `LocalSeq`. The engine calls this from
    /// `capture_local_ops` and advances the doc's `last_persisted_vv`
    /// only after this returns.
    fn append_local_wal(
        &self,
        doc_id: DocId,
        payload: EncryptedBlob,
    ) -> Result<LocalSeq, StorageError>;

    /// Append a server-delivered update to the WAL **and** persist the
    /// advanced `server_known_vv`, atomically (one transaction on
    /// sqlite; one queued IDB transaction on web). `server_known_vv`
    /// is the full merged encoded VV, not a delta. Idempotent on
    /// `server_seq`: a re-delivered row returns the existing
    /// `local_seq` with `inserted == false` — but the VV write still
    /// happens, because a duplicate delivery still proves server
    /// possession.
    fn append_remote_wal(
        &self,
        doc_id: DocId,
        row: RemoteWalRow,
        server_known_vv: &[u8],
    ) -> Result<(LocalSeq, bool), StorageError>;

    /// Replace the snapshot row for this doc and prune every WAL row
    /// with `local_seq <= cutoff`, atomically. The snapshot `payload`
    /// is a full-history export taken after every current commit was
    /// WAL-appended, so the pruned prefix is provably contained —
    /// including unsent local operations (upload is derived from Loro
    /// history via `server_known_vv`, not from WAL rows). Rows
    /// appended after `cutoff` must survive. `server_known_vv`,
    /// `last_acked_server_seq`, and any in-flight push are preserved.
    /// Impls must record the current local-counter high-water as the
    /// row's `up_to_local_seq` so post-prune appends keep `local_seq`
    /// monotonic.
    fn write_snapshot(
        &self,
        doc_id: DocId,
        cutoff: LocalSeq,
        payload: EncryptedBlob,
    ) -> Result<(), StorageError>;

    /// Persist the resume cursor: the highest *contiguous* `server_seq`
    /// the engine has durably applied. The engine calls this from
    /// `notify_oplog_durable` whenever the durable frontier advances, and
    /// impls must return this exact value as `BootState::last_acked_server_seq`
    /// next boot.
    ///
    /// This is set **explicitly**, never derived from
    /// `MAX(wal.server_seq)`: that derivation underestimates once a
    /// snapshot prunes the rows it was derived from, and over-estimates
    /// past a gap. The engine is the sole authority for the value —
    /// impls just store the last one handed to them. The engine only
    /// ever advances it after the corresponding `server_known_vv`
    /// write, so the persisted cursor never runs ahead of the VV.
    fn write_acked_seq(&self, doc_id: DocId, seq: ServerSeq) -> Result<(), StorageError>;

    /// Persist the full merged encoded `server_known_vv` outside the
    /// remote-append path — used when server knowledge advances without
    /// a WAL row (bootstrap-from-server-snapshot).
    fn write_server_known_vv(&self, doc_id: DocId, vv: &[u8]) -> Result<(), StorageError>;

    /// Persist the (single) durable in-flight push record, replacing
    /// any previous one. Called *before* the blob's first send.
    fn put_in_flight_push(&self, doc_id: DocId, push: InFlightPush) -> Result<(), StorageError>;

    /// The push identified by `push_id` was acknowledged: clear the
    /// in-flight record and persist the merged `server_known_vv`,
    /// atomically. A `push_id` that doesn't match the stored record is
    /// a no-op for the clear (the VV write still happens).
    fn complete_push(
        &self,
        doc_id: DocId,
        push_id: PushId,
        server_known_vv: &[u8],
    ) -> Result<(), StorageError>;
}

/// Lets tests share one MemStorage between the engine (via
/// `Box<Arc<MemStorage>>`) and the test body (via the original `Arc`)
/// without inventing a per-storage adapter type. Production impls
/// don't need this — they're constructed once and owned by the engine.
impl<T: LocalStorage + ?Sized> LocalStorage for Arc<T> {
    fn boot(&self, doc_id: DocId) -> Result<BootState, StorageError> {
        (**self).boot(doc_id)
    }
    fn append_local_wal(
        &self,
        doc_id: DocId,
        payload: EncryptedBlob,
    ) -> Result<LocalSeq, StorageError> {
        (**self).append_local_wal(doc_id, payload)
    }
    fn append_remote_wal(
        &self,
        doc_id: DocId,
        row: RemoteWalRow,
        server_known_vv: &[u8],
    ) -> Result<(LocalSeq, bool), StorageError> {
        (**self).append_remote_wal(doc_id, row, server_known_vv)
    }
    fn write_snapshot(
        &self,
        doc_id: DocId,
        cutoff: LocalSeq,
        payload: EncryptedBlob,
    ) -> Result<(), StorageError> {
        (**self).write_snapshot(doc_id, cutoff, payload)
    }
    fn write_acked_seq(&self, doc_id: DocId, seq: ServerSeq) -> Result<(), StorageError> {
        (**self).write_acked_seq(doc_id, seq)
    }
    fn write_server_known_vv(&self, doc_id: DocId, vv: &[u8]) -> Result<(), StorageError> {
        (**self).write_server_known_vv(doc_id, vv)
    }
    fn put_in_flight_push(&self, doc_id: DocId, push: InFlightPush) -> Result<(), StorageError> {
        (**self).put_in_flight_push(doc_id, push)
    }
    fn complete_push(
        &self,
        doc_id: DocId,
        push_id: PushId,
        server_known_vv: &[u8],
    ) -> Result<(), StorageError> {
        (**self).complete_push(doc_id, push_id, server_known_vv)
    }
}

// ---------- boot / seed / load glue ----------

/// Failure reconstructing (or seeding) a live `Doc` from a `LocalStorage`.
/// Wraps the two error sources these helpers touch — the storage backend
/// and Loro import/export — so callers get one `?`-able error.
#[derive(Debug, thiserror::Error)]
pub enum BootError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Doc(#[from] DocError),
}

/// Reconstruct the live `Doc` from persisted state: load the snapshot
/// (if any) and replay every WAL row past it. `apply_remote_batch`
/// decrypts and imports each blob and advances `last_persisted_vv` to
/// cover them, so the returned doc reports `has_uncaptured_ops() ==
/// false` — every stored op is already captured. Returns the doc and
/// the [`BootMeta`] to seed `SyncEngine::new` with.
///
/// A doc the storage has never seen yields `BootState::default()`, so the
/// result is a fresh empty `Doc` — the first-boot happy path.
///
/// `peer_id` is the leased local peer for this process (`None` = Loro's
/// default random peer, one fresh peer per construction). Set before the
/// replay import, so Loro resumes that peer's counter from the imported
/// oplog high-water and new commits extend it. The caller must guarantee
/// single ownership across live docs — see `spec/peer-id-plan.md`.
pub fn boot_doc<S: LocalStorage + ?Sized>(
    storage: &S,
    dek: &Dek,
    doc_id: DocId,
    peer_id: Option<u64>,
) -> Result<(Doc, BootMeta), BootError> {
    let boot = storage.boot(doc_id)?;
    let meta = boot.meta();
    let mut doc = match peer_id {
        Some(peer) => Doc::empty_with_peer(peer)?,
        None => Doc::empty(),
    };
    let mut blobs: Vec<EncryptedBlob> = Vec::with_capacity(1 + boot.replay.len());
    if let Some(snap) = boot.snapshot {
        blobs.push(snap.payload);
    }
    blobs.extend(boot.replay.into_iter().map(|r| r.payload));
    if !blobs.is_empty() {
        doc.apply_remote_batch(dek, blobs.iter())?;
    }
    // Replaying historical state shouldn't surface as live UI changes —
    // drop the AppEvents the import emitted.
    while doc.pop_event().is_some() {}
    Ok((doc, meta))
}

/// As [`boot_doc`], but discards the metadata — for read-only callers.
pub fn load_doc<S: LocalStorage + ?Sized>(
    storage: &S,
    dek: &Dek,
    doc_id: DocId,
) -> Result<Doc, BootError> {
    Ok(boot_doc(storage, dek, doc_id, None)?.0)
}

/// True when the local doc holds operations the server has no proof of:
/// commits beyond `server_known_vv`, or a still-outstanding in-flight
/// push. Needs the DEK because the answer is derived from the decrypted
/// Loro history, not from which WAL rows exist (folded rows may carry
/// unsent ops).
pub fn has_unsynced_ops<S: LocalStorage + ?Sized>(
    storage: &S,
    dek: &Dek,
    doc_id: DocId,
) -> Result<bool, BootError> {
    let (doc, meta) = boot_doc(storage, dek, doc_id, None)?;
    if meta.in_flight_push.is_some() {
        return Ok(true);
    }
    let server_known_vv = if meta.server_known_vv.is_empty() {
        loro::VersionVector::default()
    } else {
        loro::VersionVector::decode(&meta.server_known_vv)
            .map_err(|e| StorageError::Backend(format!("decode server_known_vv: {e}")))?
    };
    Ok(!server_known_vv.includes_vv(&doc.oplog_vv()))
}

/// Write `doc`'s full state as the doc's baseline snapshot, pruning
/// nothing (`cutoff` 0). Used at signup / login / recover to lay down
/// an initial snapshot.
pub fn seed_snapshot<S: LocalStorage + ?Sized>(
    storage: &S,
    dek: &Dek,
    doc_id: DocId,
    doc: &Doc,
) -> Result<(), BootError> {
    let blob = doc.snapshot_blob(dek)?;
    storage.write_snapshot(doc_id, LocalSeq(0), blob)?;
    Ok(())
}

// ---------- in-memory impl ----------

/// In-memory `LocalStorage` for `core` unit tests. Single doc per
/// instance is fine — tests construct a fresh `MemStorage` per case.
/// Not durable, not crash-safe, not for production use.
#[derive(Debug, Default)]
pub struct MemStorage {
    inner: Mutex<MemInner>,
}

#[derive(Debug, Default)]
struct MemInner {
    next_local_seq: u64,
    snapshot: Option<SnapshotRow>,
    /// Surviving WAL rows past the most recent snapshot, in insertion
    /// (== `local_seq`) order.
    wal: Vec<MemWalRow>,
    last_acked_server_seq: u64,
    server_known_vv: Vec<u8>,
    in_flight: Option<InFlightPush>,
}

#[derive(Debug, Clone)]
struct MemWalRow {
    local_seq: LocalSeq,
    /// `Some` for server-delivered rows; `None` for local-origin.
    server_seq: Option<ServerSeq>,
    payload: EncryptedBlob,
}

impl MemStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test hook: the stored in-flight push, if any.
    pub fn in_flight(&self) -> Option<InFlightPush> {
        self.inner
            .lock()
            .expect("MemStorage mutex poisoned")
            .in_flight
            .clone()
    }

    /// Test hook: the persisted encoded `server_known_vv`.
    pub fn server_known_vv(&self) -> Vec<u8> {
        self.inner
            .lock()
            .expect("MemStorage mutex poisoned")
            .server_known_vv
            .clone()
    }

    /// Test hook: surviving WAL row count.
    pub fn wal_len(&self) -> usize {
        self.inner
            .lock()
            .expect("MemStorage mutex poisoned")
            .wal
            .len()
    }
}

impl LocalStorage for MemStorage {
    fn boot(&self, _doc_id: DocId) -> Result<BootState, StorageError> {
        let inner = self.inner.lock().expect("MemStorage mutex poisoned");
        Ok(BootState {
            snapshot: inner.snapshot.clone(),
            replay: inner
                .wal
                .iter()
                .map(|r| WalRow {
                    local_seq: r.local_seq,
                    payload: r.payload.clone(),
                })
                .collect(),
            last_local_seq: LocalSeq(inner.next_local_seq),
            last_acked_server_seq: ServerSeq(inner.last_acked_server_seq),
            server_known_vv: inner.server_known_vv.clone(),
            in_flight_push: inner.in_flight.clone(),
            wal_bytes: inner
                .wal
                .iter()
                .map(|r| (r.payload.ciphertext.len() + r.payload.nonce.len()) as u64)
                .sum(),
        })
    }

    fn append_local_wal(
        &self,
        _doc_id: DocId,
        payload: EncryptedBlob,
    ) -> Result<LocalSeq, StorageError> {
        let mut inner = self.inner.lock().expect("MemStorage mutex poisoned");
        inner.next_local_seq += 1;
        let local_seq = LocalSeq(inner.next_local_seq);
        inner.wal.push(MemWalRow {
            local_seq,
            server_seq: None,
            payload,
        });
        Ok(local_seq)
    }

    fn append_remote_wal(
        &self,
        _doc_id: DocId,
        row: RemoteWalRow,
        server_known_vv: &[u8],
    ) -> Result<(LocalSeq, bool), StorageError> {
        let mut inner = self.inner.lock().expect("MemStorage mutex poisoned");
        // The VV write happens on the duplicate path too — a re-delivered
        // blob still proves the server has those ops.
        inner.server_known_vv = server_known_vv.to_vec();
        if let Some(existing) = inner
            .wal
            .iter()
            .find(|r| r.server_seq == Some(row.server_seq))
        {
            return Ok((existing.local_seq, false));
        }
        inner.next_local_seq += 1;
        let local_seq = LocalSeq(inner.next_local_seq);
        // Appending does NOT advance the resume cursor — that's
        // `write_acked_seq`'s job. Mirrors the real sqlite/IDB impls.
        inner.wal.push(MemWalRow {
            local_seq,
            server_seq: Some(row.server_seq),
            payload: row.payload,
        });
        Ok((local_seq, true))
    }

    fn write_snapshot(
        &self,
        _doc_id: DocId,
        cutoff: LocalSeq,
        payload: EncryptedBlob,
    ) -> Result<(), StorageError> {
        let mut inner = self.inner.lock().expect("MemStorage mutex poisoned");
        // High-water is the counter, not the cutoff: pruning may delete
        // the rows carrying the current max local_seq, so record it here
        // to keep appends monotonic.
        inner.snapshot = Some(SnapshotRow {
            up_to_local_seq: LocalSeq(inner.next_local_seq),
            payload,
        });
        inner.wal.retain(|r| r.local_seq > cutoff);
        Ok(())
    }

    fn write_acked_seq(&self, _doc_id: DocId, seq: ServerSeq) -> Result<(), StorageError> {
        // Standalone field, independent of the WAL — so it survives
        // `write_snapshot` pruning the rows it was derived from, the same
        // way the real impls persist a dedicated cursor column.
        let mut inner = self.inner.lock().expect("MemStorage mutex poisoned");
        inner.last_acked_server_seq = seq.0;
        Ok(())
    }

    fn write_server_known_vv(&self, _doc_id: DocId, vv: &[u8]) -> Result<(), StorageError> {
        let mut inner = self.inner.lock().expect("MemStorage mutex poisoned");
        inner.server_known_vv = vv.to_vec();
        Ok(())
    }

    fn put_in_flight_push(&self, _doc_id: DocId, push: InFlightPush) -> Result<(), StorageError> {
        let mut inner = self.inner.lock().expect("MemStorage mutex poisoned");
        inner.in_flight = Some(push);
        Ok(())
    }

    fn complete_push(
        &self,
        _doc_id: DocId,
        push_id: PushId,
        server_known_vv: &[u8],
    ) -> Result<(), StorageError> {
        let mut inner = self.inner.lock().expect("MemStorage mutex poisoned");
        inner.server_known_vv = server_known_vv.to_vec();
        if inner.in_flight.as_ref().map(|p| p.push_id) == Some(push_id) {
            inner.in_flight = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(byte: u8) -> EncryptedBlob {
        EncryptedBlob {
            nonce: vec![byte; 12],
            ciphertext: vec![byte; 8],
        }
    }

    fn doc_id() -> DocId {
        DocId(Uuid::nil())
    }

    #[test]
    fn empty_boot_state_for_fresh_storage() {
        let s = MemStorage::new();
        let boot = s.boot(doc_id()).unwrap();
        assert!(boot.snapshot.is_none());
        assert!(boot.replay.is_empty());
        assert_eq!(boot.last_local_seq, LocalSeq(0));
        assert_eq!(boot.last_acked_server_seq, ServerSeq(0));
        assert!(boot.server_known_vv.is_empty());
        assert!(boot.in_flight_push.is_none());
        assert_eq!(boot.wal_bytes, 0);
    }

    #[test]
    fn append_local_wal_assigns_monotonic_local_seqs() {
        let s = MemStorage::new();
        assert_eq!(s.append_local_wal(doc_id(), blob(1)).unwrap(), LocalSeq(1));
        assert_eq!(s.append_local_wal(doc_id(), blob(2)).unwrap(), LocalSeq(2));
    }

    #[test]
    fn append_remote_wal_is_idempotent_but_still_advances_vv() {
        let s = MemStorage::new();
        let (seq1, ins1) = s
            .append_remote_wal(
                doc_id(),
                RemoteWalRow {
                    server_seq: ServerSeq(7),
                    payload: blob(1),
                },
                b"vv1",
            )
            .unwrap();
        assert!(ins1);
        let (seq2, ins2) = s
            .append_remote_wal(
                doc_id(),
                RemoteWalRow {
                    server_seq: ServerSeq(7),
                    payload: blob(1),
                },
                b"vv2",
            )
            .unwrap();
        assert!(!ins2);
        assert_eq!(seq1, seq2);
        // Duplicate delivery still proved possession: VV advanced.
        assert_eq!(s.boot(doc_id()).unwrap().server_known_vv, b"vv2");
        assert_eq!(s.boot(doc_id()).unwrap().replay.len(), 1);
    }

    #[test]
    fn write_snapshot_prunes_prefix_and_keeps_tail() {
        let s = MemStorage::new();
        for i in 1..=3u8 {
            s.append_local_wal(doc_id(), blob(i)).unwrap();
        }
        s.write_snapshot(doc_id(), LocalSeq(2), blob(0xff)).unwrap();

        let boot = s.boot(doc_id()).unwrap();
        let snap = boot.snapshot.unwrap();
        // up_to_local_seq is the high-water (3 rows appended), not the cutoff.
        assert_eq!(snap.up_to_local_seq, LocalSeq(3));
        assert_eq!(boot.replay.len(), 1);
        assert_eq!(boot.replay[0].local_seq, LocalSeq(3));
        // next_local_seq is preserved across snapshot — new rows continue
        // from where they left off.
        assert_eq!(boot.last_local_seq, LocalSeq(3));
    }

    #[test]
    fn snapshot_preserves_vv_cursor_and_in_flight() {
        let s = MemStorage::new();
        s.append_local_wal(doc_id(), blob(1)).unwrap();
        s.write_server_known_vv(doc_id(), b"vv").unwrap();
        s.write_acked_seq(doc_id(), ServerSeq(9)).unwrap();
        let push = InFlightPush {
            push_id: PushId::generate(),
            payload: blob(5),
            from_vv: b"f".to_vec(),
            to_vv: b"t".to_vec(),
        };
        s.put_in_flight_push(doc_id(), push.clone()).unwrap();

        s.write_snapshot(doc_id(), LocalSeq(1), blob(0xff)).unwrap();

        let boot = s.boot(doc_id()).unwrap();
        assert!(boot.replay.is_empty());
        assert_eq!(boot.server_known_vv, b"vv");
        assert_eq!(boot.last_acked_server_seq, ServerSeq(9));
        assert_eq!(
            boot.in_flight_push.map(|p| p.push_id),
            Some(push.push_id),
            "snapshot must not clear the in-flight push",
        );
    }

    #[test]
    fn complete_push_clears_matching_record_only() {
        let s = MemStorage::new();
        let push = InFlightPush {
            push_id: PushId::generate(),
            payload: blob(5),
            from_vv: vec![],
            to_vv: vec![],
        };
        s.put_in_flight_push(doc_id(), push.clone()).unwrap();

        // Mismatched id: VV persists, record survives.
        s.complete_push(doc_id(), PushId::generate(), b"vv1")
            .unwrap();
        let boot = s.boot(doc_id()).unwrap();
        assert_eq!(boot.server_known_vv, b"vv1");
        assert!(boot.in_flight_push.is_some());

        // Matching id: record cleared.
        s.complete_push(doc_id(), push.push_id, b"vv2").unwrap();
        let boot = s.boot(doc_id()).unwrap();
        assert_eq!(boot.server_known_vv, b"vv2");
        assert!(boot.in_flight_push.is_none());
    }
}
