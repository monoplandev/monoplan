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
    PushSnapshot {
        up_to_seq: u64,
        compaction_floor_seq: u64,
        blob: EncryptedBlob,
    },
    /// Request the latest snapshot blob. Reserved for the snapshot work.
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
    /// produced blob today. (Loro shallow snapshotting will be driven
    /// by a separate VV-horizon mechanism — see
    /// `spec/sync-protocol.md` §"Shallow snapshots (future)".)
    SnapshotRequest {
        up_to_seq: u64,
        compaction_floor_seq: u64,
    },
    /// Response to `PullSnapshot`. `up_to_seq` is the snapshot's
    /// encoded state frontier; the bootstrapping client uses it as
    /// its next `since_seq` for `PullOps`.
    Snapshot { up_to_seq: u64, blob: EncryptedBlob },
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
