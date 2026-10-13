//! Sqlite reads/writes for the op stream and per-device frontier.
//!
//! Ops, snapshots, and the seq counter are keyed by **doc_id**. Today every
//! account has exactly one doc (their primary), so the per-doc keying is
//! 1:1 with per-account — but the storage layer already speaks docs so the
//! eventual multi-doc / sharing migration (`spec/sharing-plan.md`) doesn't
//! need to re-key the data plane. Device frontiers stay per-device (which
//! is also per-doc while 1:1 holds).

use monoplan_protocol::{EncryptedBlob, PushAck, PushBlob, StoredBlob};
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::db::{Db, now_millis};

/// Hard caps per `OpsBatch` frame from `spec/sync-protocol.md`. The
/// byte cap is the safety net; the count cap is the common one.
pub const MAX_OPS_PER_BATCH: usize = 500;
pub const MAX_BYTES_PER_BATCH: usize = 256 * 1024;

/// Result of [`insert_ops`]: the wire acks (one per input blob, in
/// input order) plus the subset of ops that were freshly inserted —
/// deduplicated retries must be acked but NOT re-broadcast (peers
/// already received them on the original insert).
pub struct InsertedOps {
    pub acks: Vec<PushAck>,
    pub fresh: Vec<StoredBlob>,
}

/// Append pushed ops in a single transaction, deduplicating on
/// `(doc, device, push_id)`. A repeat of an already-inserted push
/// returns its original seq without inserting another row — the
/// client-side crash-retry contract (spec/sync-protocol.md). Fresh
/// inserts get dense, gap-free per-doc seqs: a holes-in-the-stream
/// check on the client is meaningful, unlike a global counter where
/// another doc's writes would look like holes from this doc's
/// perspective.
pub async fn insert_ops(
    db: &Db,
    doc_id: Uuid,
    device_id: Uuid,
    blobs: Vec<PushBlob>,
) -> anyhow::Result<InsertedOps> {
    if blobs.is_empty() {
        return Ok(InsertedOps {
            acks: Vec::new(),
            fresh: Vec::new(),
        });
    }
    let doc_bytes = doc_id.as_bytes().to_vec();
    let dev_bytes = device_id.as_bytes().to_vec();
    let now = now_millis();
    db.call(move |c| {
        let tx = c.transaction()?;
        let mut acks = Vec::with_capacity(blobs.len());
        let mut fresh = Vec::new();
        for op in blobs {
            let push_bytes = op.push_id.as_bytes().to_vec();
            // Dedup lookup before reserving a seq — a retry must not
            // burn a counter value (seqs are dense and gap-free).
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT seq FROM ops
                     WHERE doc_id = ? AND device_id = ? AND push_id = ?",
                    params![doc_bytes, dev_bytes, push_bytes],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(seq) = existing {
                acks.push(PushAck {
                    push_id: op.push_id,
                    seq: seq as u64,
                });
                continue;
            }
            // Reserve one seq atomically. UPSERT keeps the counter row
            // initialisation in the same write so we don't need a
            // separate bootstrap step at doc creation. The doc's
            // current epoch rides along under the same row lock so
            // the op row is stamped consistently with it.
            let (seq, epoch): (i64, i64) = tx.query_row(
                "INSERT INTO doc_sequences (doc_id, next_seq) VALUES (?, 2)
                 ON CONFLICT(doc_id) DO UPDATE SET next_seq = next_seq + 1
                 RETURNING next_seq - 1, epoch",
                params![doc_bytes],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            tx.execute(
                "INSERT INTO ops (doc_id, seq, epoch, device_id, push_id, payload, payload_nonce, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    doc_bytes,
                    seq,
                    epoch,
                    dev_bytes,
                    push_bytes,
                    op.blob.ciphertext,
                    op.blob.nonce,
                    now
                ],
            )?;
            acks.push(PushAck {
                push_id: op.push_id,
                seq: seq as u64,
            });
            fresh.push(StoredBlob {
                seq: seq as u64,
                blob: op.blob,
            });
        }
        tx.commit()?;
        Ok(InsertedOps { acks, fresh })
    })
    .await
}

/// One batch's worth of ops with `seq > since_seq`, ordered ascending.
/// Caps at `MAX_OPS_PER_BATCH` or `MAX_BYTES_PER_BATCH`, whichever
/// trips first. `has_more` is true iff the caller should issue another
/// fetch from the last returned seq.
pub struct FetchedBatch {
    pub ops: Vec<StoredBlob>,
    pub has_more: bool,
}

pub async fn fetch_ops_batch(
    db: &Db,
    doc_id: Uuid,
    since_seq: u64,
) -> anyhow::Result<FetchedBatch> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        // Pull at most one extra row past the count cap; if it exists,
        // we know there's more without having to issue a count query.
        let limit = (MAX_OPS_PER_BATCH + 1) as i64;
        let mut stmt = c.prepare(
            "SELECT seq, payload, payload_nonce
             FROM ops
             WHERE doc_id = ? AND seq > ?
             ORDER BY seq ASC
             LIMIT ?",
        )?;
        let rows = stmt.query_map(params![doc_bytes, since_seq as i64, limit], |r| {
            Ok((
                r.get::<_, i64>(0)? as u64,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        let mut ops = Vec::new();
        let mut bytes = 0usize;
        let mut over_count = false;
        let mut over_bytes = false;
        for row in rows {
            let (seq, ciphertext, nonce) = row?;
            if ops.len() >= MAX_OPS_PER_BATCH {
                over_count = true;
                break;
            }
            let row_bytes = ciphertext.len() + nonce.len();
            if !ops.is_empty() && bytes + row_bytes > MAX_BYTES_PER_BATCH {
                over_bytes = true;
                break;
            }
            bytes += row_bytes;
            ops.push(StoredBlob {
                seq,
                blob: EncryptedBlob { nonce, ciphertext },
            });
        }
        Ok(FetchedBatch {
            ops,
            has_more: over_count || over_bytes,
        })
    })
    .await
}

/// Advance a device's frontier. Monotonic — silently ignores attempts
/// to move backwards so a slow client retransmitting old acks can't
/// corrupt the horizon calculation.
pub async fn advance_last_acked_seq(
    db: &Db,
    device_id: Uuid,
    last_acked: u64,
) -> anyhow::Result<()> {
    let dev_bytes = device_id.as_bytes().to_vec();
    db.call(move |c| {
        c.execute(
            "UPDATE devices
             SET last_acked_seq = ?
             WHERE id = ? AND last_acked_seq < ?",
            params![last_acked as i64, dev_bytes, last_acked as i64],
        )
    })
    .await?;
    Ok(())
}

/// Latest snapshot for a doc (highest `id`, which monotonically
/// tracks `up_to_seq` since snapshots are append-only). Returns
/// `None` when no snapshot exists yet — the bootstrap path is dormant
/// in that case and `pull_ops` falls through to op streaming.
pub struct LatestSnapshot {
    pub up_to_seq: u64,
    pub compaction_floor_seq: u64,
    pub epoch: u64,
    pub blob: EncryptedBlob,
}

/// The seq columns and epoch of the latest snapshot, without the
/// payload. Hot path for `pull_ops` (compaction-floor check) and the
/// snapshot coordinator (trigger eval). `None` if no snapshot exists.
pub struct LatestSnapshotMeta {
    pub up_to_seq: u64,
    pub compaction_floor_seq: u64,
    pub epoch: u64,
}

/// A doc's current history epoch (`spec/storage.md` §Epochs). Docs
/// with no `doc_sequences` row yet (nothing pushed) are in epoch 0.
pub async fn doc_epoch(db: &Db, doc_id: Uuid) -> anyhow::Result<u64> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| read_doc_epoch(c, &doc_bytes))
        .await
        .map(|v| v as u64)
}

fn read_doc_epoch(c: &rusqlite::Connection, doc_bytes: &[u8]) -> rusqlite::Result<i64> {
    c.query_row(
        "SELECT epoch FROM doc_sequences WHERE doc_id = ?",
        [doc_bytes],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .map(|v| v.unwrap_or(0))
}

/// Insert a snapshot row, stamped with the doc's current epoch. Every
/// snapshot today is full-history, so it belongs to the epoch it was
/// taken in; a future *trimmed* snapshot is the one that advances
/// `doc_sequences.epoch` (in this same transaction) and is stamped
/// with the new value. Used by snapshot orchestration; integration
/// tests also call this directly to seed snapshot state.
pub async fn insert_snapshot(
    db: &Db,
    doc_id: Uuid,
    up_to_seq: u64,
    compaction_floor_seq: u64,
    blob: EncryptedBlob,
) -> anyhow::Result<u64> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    let now = now_millis();
    db.call(move |c| {
        let tx = c.transaction()?;
        let epoch = read_doc_epoch(&tx, &doc_bytes)?;
        tx.execute(
            "INSERT INTO snapshots (doc_id, up_to_seq, compaction_floor_seq, epoch, payload, payload_nonce, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                doc_bytes,
                up_to_seq as i64,
                compaction_floor_seq as i64,
                epoch,
                blob.ciphertext,
                blob.nonce,
                now,
            ],
        )?;
        let id = tx.last_insert_rowid() as u64;
        tx.commit()?;
        Ok(id)
    })
    .await
}

pub async fn latest_snapshot(db: &Db, doc_id: Uuid) -> anyhow::Result<Option<LatestSnapshot>> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let row = c
            .query_row(
                "SELECT up_to_seq, compaction_floor_seq, epoch, payload, payload_nonce
                 FROM snapshots
                 WHERE doc_id = ?
                 ORDER BY id DESC
                 LIMIT 1",
                [doc_bytes],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)? as u64,
                        r.get::<_, i64>(1)? as u64,
                        r.get::<_, i64>(2)? as u64,
                        r.get::<_, Vec<u8>>(3)?,
                        r.get::<_, Vec<u8>>(4)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(
            |(up_to_seq, compaction_floor_seq, epoch, ciphertext, nonce)| LatestSnapshot {
                up_to_seq,
                compaction_floor_seq,
                epoch,
                blob: EncryptedBlob { nonce, ciphertext },
            },
        ))
    })
    .await
}

pub async fn latest_snapshot_meta(
    db: &Db,
    doc_id: Uuid,
) -> anyhow::Result<Option<LatestSnapshotMeta>> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let row = c
            .query_row(
                "SELECT up_to_seq, compaction_floor_seq, epoch
                 FROM snapshots
                 WHERE doc_id = ?
                 ORDER BY id DESC
                 LIMIT 1",
                [doc_bytes],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)? as u64,
                        r.get::<_, i64>(1)? as u64,
                        r.get::<_, i64>(2)? as u64,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(
            |(up_to_seq, compaction_floor_seq, epoch)| LatestSnapshotMeta {
                up_to_seq,
                compaction_floor_seq,
                epoch,
            },
        ))
    })
    .await
}

/// Read a device's recorded frontier. Used by tests; production paths
/// just call `advance_last_acked_seq`.
pub async fn get_last_acked_seq(db: &Db, device_id: Uuid) -> anyhow::Result<u64> {
    let dev_bytes = device_id.as_bytes().to_vec();
    db.call(move |c| {
        c.query_row(
            "SELECT last_acked_seq FROM devices WHERE id = ?",
            [dev_bytes],
            |r| r.get::<_, i64>(0),
        )
    })
    .await
    .map(|v| v as u64)
}

/// Horizon: `min(last_acked_seq)` across all of an account's
/// devices, with `override_device_id`'s row replaced by
/// `override_value`. Used as the compaction-floor input to the
/// snapshot coordinator (see `spec/sync-protocol.md` §"Snapshot
/// orchestration") and computed from the calling device's optimistic
/// frontier: during a push, the device has the just-assigned seqs
/// locally but hasn't sent the `Ack` yet, so its DB row would drag
/// horizon down artificially. Substituting the optimistic value
/// reflects the device's true frontier.
///
/// Takes `account_id` because devices are still account-scoped — the
/// frontier-bearing entity is the device, and devices belong to
/// accounts, not docs. While each account has exactly one doc (1:1),
/// the per-account horizon equals the per-doc horizon. When per-doc
/// memberships land (`spec/sharing-plan.md`) this becomes
/// `min(last_acked_seq)` over `device_doc_frontiers` rows for that doc,
/// joined to active memberships.
///
/// Returns 0 when the account has no devices — caller treats that
/// as "no compaction possible" via the trigger's horizon guard.
pub async fn account_horizon(
    db: &Db,
    account_id: Uuid,
    override_device_id: Uuid,
    override_value: u64,
) -> anyhow::Result<u64> {
    let acc_bytes = account_id.as_bytes().to_vec();
    let dev_bytes = override_device_id.as_bytes().to_vec();
    db.call(move |c| {
        let v: Option<i64> = c.query_row(
            "SELECT MIN(CASE WHEN id = ? THEN ? ELSE last_acked_seq END)
             FROM devices WHERE account_id = ?",
            params![dev_bytes, override_value as i64, acc_bytes],
            |r| r.get::<_, Option<i64>>(0),
        )?;
        Ok(v.unwrap_or(0) as u64)
    })
    .await
}

/// Max snapshots retained per doc after compaction (see
/// `spec/storage.md` §"Compaction"). Older snapshots are deleted.
pub const KEEP_SNAPSHOTS: u64 = 2;

pub struct CompactionStats {
    pub ops_deleted: u64,
    pub snapshots_deleted: u64,
}

/// Delete ops at or below the latest snapshot's `compaction_floor_seq`
/// and prune snapshots older than the `keep_snapshots` newest. The
/// snapshot read + both deletes run in one transaction so a concurrent
/// `insert_snapshot` can't shift the floor mid-deletion. Returns 0/0
/// when no snapshot exists yet.
pub async fn compact_doc(
    db: &Db,
    doc_id: Uuid,
    keep_snapshots: u64,
) -> anyhow::Result<CompactionStats> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let tx = c.transaction()?;
        let floor: Option<i64> = tx
            .query_row(
                "SELECT compaction_floor_seq
                 FROM snapshots
                 WHERE doc_id = ?
                 ORDER BY id DESC
                 LIMIT 1",
                [&doc_bytes],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        let Some(floor) = floor else {
            tx.commit()?;
            return Ok(CompactionStats {
                ops_deleted: 0,
                snapshots_deleted: 0,
            });
        };
        let ops_deleted = tx.execute(
            "DELETE FROM ops WHERE doc_id = ? AND seq <= ?",
            params![doc_bytes, floor],
        )? as u64;
        // Subquery returns NULL when fewer than keep_snapshots+1 rows
        // exist, so `id <= NULL` evaluates false and nothing is pruned.
        let snapshots_deleted = tx.execute(
            "DELETE FROM snapshots
             WHERE doc_id = ?
               AND id <= (
                 SELECT id FROM snapshots
                 WHERE doc_id = ?
                 ORDER BY id DESC
                 LIMIT 1 OFFSET ?
               )",
            params![doc_bytes, doc_bytes, keep_snapshots as i64],
        )? as u64;
        tx.commit()?;
        Ok(CompactionStats {
            ops_deleted,
            snapshots_deleted,
        })
    })
    .await
}

pub async fn latest_doc_seq(db: &Db, doc_id: Uuid) -> anyhow::Result<u64> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let row = c
            .query_row(
                "SELECT seq
                 FROM ops
                 WHERE doc_id = ?
                 ORDER BY seq DESC
                 LIMIT 1",
                [doc_bytes],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        Ok(row.unwrap_or(0) as u64)
    })
    .await
}
