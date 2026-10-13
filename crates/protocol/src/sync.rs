//! Wire types for the WebSocket sync surface.
//!
//! Frames are MessagePack-encoded tagged enums (`#[serde(tag = "type")]`)
//! so the discriminator travels in-band — belt and braces against
//! someone wiring the wrong serializer to the wrong endpoint. The
//! protocol version is negotiated on connect via the `Hello` /
//! `HelloAck` exchange before any payload frame is exchanged.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Current wire protocol version. Bump on breaking change; for purely
/// additive evolution rely on MessagePack's tagged-map semantics.
///
/// Note: the v2 per-list order-container CRDT schema
/// (`spec/data-model.md`) shipped *without* a wire bump — frames are
/// unchanged and the doc layout inside the encrypted blobs is opaque
/// to the protocol. Pre-release, single-user: the schema cutover is
/// handled operationally (export JSON → wipe → import), not fenced at
/// the handshake.
pub const PROTOCOL_VERSION: u32 = 1;

/// Encrypted op or snapshot payload. Server-side these are opaque
/// blobs — only the client (with the DEK) can decrypt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncryptedBlob {
    #[serde(with = "serde_bytes")]
    pub nonce: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub ciphertext: Vec<u8>,
}

/// Server-assigned per-account sequence number paired with the encrypted
/// payload. `seq` is dense and gap-free within an account — see
/// `spec/storage.md` for the counter mechanism.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredBlob {
    pub seq: u64,
    pub blob: EncryptedBlob,
}

/// One pushed op: a client-minted durable push identifier plus the
/// encrypted payload. `push_id` is the retry/idempotency key — the
/// server stores it alongside the originating device and, on a repeat
/// of the same `(device, push_id)`, returns the originally assigned
/// seq without inserting a second row. Minted once when the push
/// record is durably persisted client-side, *before* the first send,
/// so a crash between server insert and client ack retries the exact
/// same id. (Serialized as 16 raw bytes under MessagePack.)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PushBlob {
    pub push_id: Uuid,
    pub blob: EncryptedBlob,
}

/// One acknowledged push: which `push_id` landed and the server seq it
/// was (or already had been) assigned. Identifying acks by `push_id`
/// rather than array position makes the ack self-describing across
/// retries and reconnects.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PushAck {
    pub push_id: Uuid,
    pub seq: u64,
}

/// The starting point of a delta snapshot: the previous snapshot's
/// `up_to_seq` and its sealed `up_to_vv`, exactly as the producer of
/// that snapshot uploaded them. The server round-trips `vv` as opaque
/// bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotBase {
    pub seq: u64,
    pub vv: EncryptedBlob,
}

// ---------- handshake ----------

/// First frame on every WS connection (client → server).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    /// Free-form client identifier ("monoplan-cli", "monoplan-web", ...).
    pub client: String,
    pub client_version: String,
    /// Versions the client can speak. Server picks the highest shared.
    pub supported_protocol_versions: Vec<u32>,
}

/// Server's positive response to `Hello`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloAck {
    pub server_version: String,
    pub protocol_version: u32,
    /// The doc's current history epoch (`spec/storage.md` §Epochs).
    /// Informational for now: nothing advances it until trimmed
    /// snapshots land, and a mid-session advance is not yet signalled.
    #[serde(default)]
    pub epoch: u64,
}

/// Server's negative response to `Hello`. Connection closes after.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloRejected {
    pub reason: String,
}

// ---------- client → server frames ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientFrame {
    /// Append ops. Server assigns per-account seqs and replies `OpsAck`.
    /// Each blob carries a durable `push_id`; re-sending a previously
    /// inserted `(device, push_id)` is idempotent (see [`PushBlob`]).
    PushOps { ops: Vec<PushBlob> },
    /// Request all ops with seq > since_seq. Streamed back as one or
    /// more `OpsBatch` frames; the last carries `complete=true`.
    PullOps { since_seq: u64 },
    /// Advance this device's frontier. Sent after Loro accepts the ops
    /// locally — never on raw byte receipt. Carries the contiguous
    /// prefix of seqs the device has applied.
    Ack { last_acked_seq: u64 },
    /// Response to a `SnapshotRequest`. `up_to_seq` is the encoded
    /// state frontier; `compaction_floor_seq` is the seq at/below which
    /// op blobs become eligible for server-side GC once this snapshot
    /// lands. Echoed verbatim from `SnapshotRequest` — the producing
    /// client doesn't compute it.
    ///
    /// `up_to_vv` is the producer's Loro version vector at `up_to_seq`
    /// (`VersionVector::encode`, sealed under the DEK). The server
    /// stores it beside the blob and hands it back as the `from` base
    /// of the next `SnapshotRequest`, so the next producer knows where
    /// its delta starts without any seq-to-VV mapping of its own. The
    /// server never reads it.
    ///
    /// `from_seq` echoes `SnapshotRequest.base.seq`: `Some` means the
    /// blob is a *delta* (Loro updates strictly after the base VV, up
    /// to `up_to_vv`), `None` means a full snapshot. The server rejects
    /// a delta whose `from_seq` is not the latest stored snapshot's
    /// `up_to_seq`, so the chain can't fork.
    PushSnapshot {
        up_to_seq: u64,
        compaction_floor_seq: u64,
        from_seq: Option<u64>,
        up_to_vv: EncryptedBlob,
        blob: EncryptedBlob,
    },
    /// Request the latest snapshot chain (see `ServerFrame::Snapshot`).
    PullSnapshot,
}

// ---------- server → client frames ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerFrame {
    /// Response to `PushOps`. One `PushAck` per pushed blob, in input
    /// order, each identifying the acknowledged `push_id` and its
    /// (possibly pre-existing, when deduplicated) server seq.
    OpsAck { acks: Vec<PushAck> },
    /// Chunk of pulled ops. May be one of several frames per `PullOps`.
    OpsBatch {
        ops: Vec<StoredBlob>,
        complete: bool,
    },
    /// Pushed when another device on the same account commits ops.
    /// Reserved for the broadcast work.
    OpsBroadcast { ops: Vec<StoredBlob> },
    /// Server asks a connected, caught-up client to produce a snapshot.
    /// `up_to_seq` is the requested state frontier (= server's idea of
    /// the producer's `last_acked_seq`); `compaction_floor_seq` is the
    /// seq at/below which op blobs become eligible for server-side GC
    /// once this snapshot lands (= `max(horizon, prev snapshot's
    /// compaction_floor_seq)`). The client echoes `compaction_floor_seq`
    /// back verbatim in `PushSnapshot`; it does not influence the
    /// produced blob.
    ///
    /// `base` selects the shape of the blob. `None`: a full Loro
    /// snapshot at the producer's server-known frontier. `Some`: a
    /// *delta* — Loro updates strictly after `base.vv` (the latest
    /// stored snapshot's `up_to_vv`, opaque to the server) up to the
    /// producer's server-known frontier. The producer must already
    /// hold everything in `base.vv`, which the server guarantees by
    /// only asking a device acked at or past `base.seq`.
    SnapshotRequest {
        up_to_seq: u64,
        compaction_floor_seq: u64,
        base: Option<SnapshotBase>,
    },
    /// Response to `PullSnapshot`. `blobs` is the stored chain in
    /// import order: the latest full snapshot first, then every delta
    /// landed after it. `up_to_seq` is the chain's final state
    /// frontier; the bootstrapping client uses it as its next
    /// `since_seq` for `PullOps`.
    Snapshot {
        up_to_seq: u64,
        blobs: Vec<EncryptedBlob>,
    },
    /// Sent in lieu of `OpsBatch` when the client's `since_seq` is
    /// below the latest snapshot's `compaction_floor_seq` — the ops it
    /// needs have been compacted, so it can't resume from ops alone.
    /// (Devices between `compaction_floor_seq` and `up_to_seq` can
    /// still delta-pull — horizon-bounded compaction preserves those.)
    /// The client must `PullSnapshot`, apply the returned `Snapshot`,
    /// then re-issue `PullOps` from the snapshot's state frontier.
    /// `up_to_seq` here is informational; the authoritative value is
    /// the one in the `Snapshot` frame.
    SnapshotRequired { up_to_seq: u64 },
}
