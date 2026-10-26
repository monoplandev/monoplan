//! Sqlite reads/writes for the update stream and per-device frontier.
//!
//! Updates, snapshots, and the seq counter are keyed by **doc_id**. Today every
//! account has exactly one doc (their primary), so the per-doc keying is
//! 1:1 with per-account — but the storage layer already speaks docs so the
//! eventual multi-doc / sharing migration (`spec/sharing-plan.md`) doesn't
//! need to re-key the data plane. Device frontiers are per-(device, doc)
//! (`device_doc_frontiers`); the horizon for a doc spans the devices of
//! every account that can open it (today: just the owner).

use monoplan_protocol::{EncryptedBlob, PushAck, PushBlob, StoredBlob};
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::db::{Db, now_millis};

/// Hard caps per `UpdatesBatch` frame from `spec/sync-protocol.md`. The
/// byte cap is the safety net; the count cap is the common one.
pub const MAX_UPDATES_PER_BATCH: usize = 500;
pub const MAX_BYTES_PER_BATCH: usize = 256 * 1024;

/// Result of [`insert_updates`]: the wire acks (one per input blob, in
/// input order) plus the subset of updates that were freshly inserted —
/// deduplicated retries must be acked but NOT re-broadcast (peers
/// already received them on the original insert).
pub struct InsertedUpdates {
    pub acks: Vec<PushAck>,
    pub fresh: Vec<StoredBlob>,
}

/// Append pushed updates in a single transaction, deduplicating on
/// `(doc, device, push_id)`. A repeat of an already-inserted push
/// returns its original seq without inserting another row — the
/// client-side crash-retry contract (spec/sync-protocol.md). Fresh
/// inserts get dense, gap-free per-doc seqs: a holes-in-the-stream
/// check on the client is meaningful, unlike a global counter where
/// another doc's writes would look like holes from this doc's
/// perspective.
pub async fn insert_updates(
    db: &Db,
    doc_id: Uuid,
    device_id: Uuid,
    blobs: Vec<PushBlob>,
) -> anyhow::Result<InsertedUpdates> {
    if blobs.is_empty() {
        return Ok(InsertedUpdates {
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
        for update in blobs {
            let push_bytes = update.push_id.as_bytes().to_vec();
            // Dedup lookup before reserving a seq — a retry must not
            // burn a counter value (seqs are dense and gap-free).
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT seq FROM updates
                     WHERE doc_id = ? AND device_id = ? AND push_id = ?",
                    params![doc_bytes, dev_bytes, push_bytes],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(seq) = existing {
                acks.push(PushAck {
                    push_id: update.push_id,
                    seq: seq as u64,
                });
                continue;
            }
            // Reserve one seq atomically. UPSERT keeps the counter row
            // initialisation in the same write so we don't need a
            // separate bootstrap step at doc creation. The doc's
            // current epoch rides along under the same row lock so
            // the update row is stamped consistently with it.
            let (seq, epoch): (i64, i64) = tx.query_row(
                "INSERT INTO doc_sequences (doc_id, next_seq) VALUES (?, 2)
                 ON CONFLICT(doc_id) DO UPDATE SET next_seq = next_seq + 1
                 RETURNING next_seq - 1, epoch",
                params![doc_bytes],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            tx.execute(
                "INSERT INTO updates (doc_id, seq, epoch, device_id, push_id, payload, payload_nonce, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    doc_bytes,
                    seq,
                    epoch,
                    dev_bytes,
                    push_bytes,
                    update.blob.ciphertext,
                    update.blob.nonce,
                    now
                ],
            )?;
            acks.push(PushAck {
                push_id: update.push_id,
                seq: seq as u64,
            });
            fresh.push(StoredBlob {
                seq: seq as u64,
                blob: update.blob,
            });
        }
        tx.commit()?;
        Ok(InsertedUpdates { acks, fresh })
    })
    .await
}

/// One batch's worth of updates with `seq > since_seq`, ordered ascending.
/// Caps at `MAX_UPDATES_PER_BATCH` or `MAX_BYTES_PER_BATCH`, whichever
/// trips first. `has_more` is true iff the caller should issue another
/// fetch from the last returned seq.
pub struct FetchedBatch {
    pub updates: Vec<StoredBlob>,
    pub has_more: bool,
}

pub async fn fetch_updates_batch(
    db: &Db,
    doc_id: Uuid,
    since_seq: u64,
) -> anyhow::Result<FetchedBatch> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        // Pull at most one extra row past the count cap; if it exists,
        // we know there's more without having to issue a count query.
        let limit = (MAX_UPDATES_PER_BATCH + 1) as i64;
        let mut stmt = c.prepare(
            "SELECT seq, payload, payload_nonce
             FROM updates
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
        let mut updates = Vec::new();
        let mut bytes = 0usize;
        let mut over_count = false;
        let mut over_bytes = false;
        for row in rows {
            let (seq, ciphertext, nonce) = row?;
            if updates.len() >= MAX_UPDATES_PER_BATCH {
                over_count = true;
                break;
            }
            let row_bytes = ciphertext.len() + nonce.len();
            if !updates.is_empty() && bytes + row_bytes > MAX_BYTES_PER_BATCH {
                over_bytes = true;
                break;
            }
            bytes += row_bytes;
            updates.push(StoredBlob {
                seq,
                blob: EncryptedBlob { nonce, ciphertext },
            });
        }
        Ok(FetchedBatch {
            updates,
            has_more: over_count || over_bytes,
        })
    })
    .await
}

/// Advance a device's frontier on a doc. Monotonic — silently ignores
/// attempts to move backwards so a slow client retransmitting old acks
/// can't corrupt the horizon calculation. The `(device, doc)` row is
/// created on the first ack.
pub async fn advance_last_acked_seq(
    db: &Db,
    device_id: Uuid,
    doc_id: Uuid,
    last_acked: u64,
) -> anyhow::Result<()> {
    let dev_bytes = device_id.as_bytes().to_vec();
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        c.execute(
            "INSERT INTO device_doc_frontiers (device_id, doc_id, last_acked_seq)
             VALUES (?, ?, ?)
             ON CONFLICT(device_id, doc_id) DO UPDATE
               SET last_acked_seq = excluded.last_acked_seq
               WHERE excluded.last_acked_seq > device_doc_frontiers.last_acked_seq",
            params![dev_bytes, doc_bytes, last_acked as i64],
        )
    })
    .await?;
    Ok(())
}

/// The latest snapshot row for a doc (highest `id`, which monotonically
/// tracks `up_to_seq` since snapshots are append-only), payload
/// included. Tests and diagnostics; the bootstrap path serves
/// [`snapshot_chain`] instead. `None` when no snapshot exists yet.
pub struct LatestSnapshot {
    pub up_to_seq: u64,
    pub compaction_floor_seq: u64,
    pub epoch: u64,
    /// `None` = full snapshot; `Some(seq)` = delta chained after the
    /// row whose `up_to_seq` is `seq`.
    pub from_seq: Option<u64>,
    /// Producer's Loro VV at `up_to_seq`, sealed under the DEK.
    pub up_to_vv: EncryptedBlob,
    pub blob: EncryptedBlob,
}

/// The latest snapshot's seq columns, epoch, chain shape and sealed
/// VV, without the payload. Hot path for `pull_updates` (compaction-floor
/// check) and the snapshot coordinator (trigger eval + delta base).
/// `None` if no snapshot exists.
pub struct LatestSnapshotMeta {
    pub up_to_seq: u64,
    pub compaction_floor_seq: u64,
    pub epoch: u64,
    pub from_seq: Option<u64>,
    pub up_to_vv: EncryptedBlob,
}

/// One row of a [`SnapshotChain`]: the blob and the state frontier it
/// brings the importer to.
pub struct SnapshotChainRow {
    pub up_to_seq: u64,
    pub blob: EncryptedBlob,
}

/// What a bootstrapping device imports: the latest full snapshot
/// followed by every delta landed after it, in import order.
pub struct SnapshotChain {
    /// The chain's final state frontier (= the last row's `up_to_seq`).
    pub up_to_seq: u64,
    pub compaction_floor_seq: u64,
    pub epoch: u64,
    pub rows: Vec<SnapshotChainRow>,
}

/// Outcome of [`insert_snapshot`].
#[derive(Debug)]
pub enum InsertSnapshot {
    Inserted {
        id: u64,
    },
    /// A delta's `from_seq` did not match the latest stored row's
    /// `up_to_seq` (or there was no row to chain onto), or its
    /// `up_to_seq` sat below `from_seq`. Nothing was written: the
    /// chain must stay linear, and a mislinked delta would corrupt
    /// every later bootstrap.
    ChainMismatch {
        latest_up_to_seq: Option<u64>,
    },
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

fn read_latest_up_to_seq(
    c: &rusqlite::Connection,
    doc_bytes: &[u8],
) -> rusqlite::Result<Option<i64>> {
    c.query_row(
        "SELECT up_to_seq FROM snapshots WHERE doc_id = ? ORDER BY id DESC LIMIT 1",
        [doc_bytes],
        |r| r.get::<_, i64>(0),
    )
    .optional()
}

/// Insert a snapshot row, stamped with the doc's current epoch. Every
/// snapshot today is full-history or a lossless delta on one, so it
/// belongs to the epoch it was taken in; a future *trimmed* snapshot
/// is the one that advances `doc_sequences.epoch` (in this same
/// transaction) and is stamped with the new value.
///
/// `from_seq = Some(s)` declares a delta chained onto the latest row,
/// which must have `up_to_seq == s`; the check runs under the same
/// transaction as the insert so two producers can't both chain onto
/// the same base. Used by snapshot orchestration; integration tests
/// also call this directly to seed snapshot state.
pub async fn insert_snapshot(
    db: &Db,
    doc_id: Uuid,
    up_to_seq: u64,
    compaction_floor_seq: u64,
    from_seq: Option<u64>,
    up_to_vv: EncryptedBlob,
    blob: EncryptedBlob,
) -> anyhow::Result<InsertSnapshot> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    let now = now_millis();
    db.call(move |c| {
        let tx = c.transaction()?;
        if let Some(from) = from_seq {
            let latest = read_latest_up_to_seq(&tx, &doc_bytes)?;
            if latest != Some(from as i64) || up_to_seq < from {
                tx.commit()?;
                return Ok(InsertSnapshot::ChainMismatch {
                    latest_up_to_seq: latest.map(|v| v as u64),
                });
            }
        }
        let epoch = read_doc_epoch(&tx, &doc_bytes)?;
        tx.execute(
            "INSERT INTO snapshots (doc_id, up_to_seq, compaction_floor_seq, epoch, from_seq, up_to_vv, up_to_vv_nonce, payload, payload_nonce, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                doc_bytes,
                up_to_seq as i64,
                compaction_floor_seq as i64,
                epoch,
                from_seq.map(|v| v as i64),
                up_to_vv.ciphertext,
                up_to_vv.nonce,
                blob.ciphertext,
                blob.nonce,
                now,
            ],
        )?;
        let id = tx.last_insert_rowid() as u64;
        tx.commit()?;
        Ok(InsertSnapshot::Inserted { id })
    })
    .await
}

pub async fn latest_snapshot(db: &Db, doc_id: Uuid) -> anyhow::Result<Option<LatestSnapshot>> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        c.query_row(
            "SELECT up_to_seq, compaction_floor_seq, epoch, from_seq, up_to_vv, up_to_vv_nonce, payload, payload_nonce
             FROM snapshots
             WHERE doc_id = ?
             ORDER BY id DESC
             LIMIT 1",
            [doc_bytes],
            |r| {
                Ok(LatestSnapshot {
                    up_to_seq: r.get::<_, i64>(0)? as u64,
                    compaction_floor_seq: r.get::<_, i64>(1)? as u64,
                    epoch: r.get::<_, i64>(2)? as u64,
                    from_seq: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                    up_to_vv: EncryptedBlob {
                        ciphertext: r.get::<_, Vec<u8>>(4)?,
                        nonce: r.get::<_, Vec<u8>>(5)?,
                    },
                    blob: EncryptedBlob {
                        ciphertext: r.get::<_, Vec<u8>>(6)?,
                        nonce: r.get::<_, Vec<u8>>(7)?,
                    },
                })
            },
        )
        .optional()
    })
    .await
}

pub async fn latest_snapshot_meta(
    db: &Db,
    doc_id: Uuid,
) -> anyhow::Result<Option<LatestSnapshotMeta>> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        c.query_row(
            "SELECT up_to_seq, compaction_floor_seq, epoch, from_seq, up_to_vv, up_to_vv_nonce
             FROM snapshots
             WHERE doc_id = ?
             ORDER BY id DESC
             LIMIT 1",
            [doc_bytes],
            |r| {
                Ok(LatestSnapshotMeta {
                    up_to_seq: r.get::<_, i64>(0)? as u64,
                    compaction_floor_seq: r.get::<_, i64>(1)? as u64,
                    epoch: r.get::<_, i64>(2)? as u64,
                    from_seq: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                    up_to_vv: EncryptedBlob {
                        ciphertext: r.get::<_, Vec<u8>>(4)?,
                        nonce: r.get::<_, Vec<u8>>(5)?,
                    },
                })
            },
        )
        .optional()
    })
    .await
}

/// The bootstrap payload: the latest full snapshot and every delta
/// chained after it, oldest first. `None` when no full snapshot
/// exists. A delta whose `from_seq` doesn't link to the row before it
/// (impossible via `insert_snapshot`, so only ever manual damage) ends
/// the chain early with a warning rather than serving a blob the
/// client could not import.
pub async fn snapshot_chain(db: &Db, doc_id: Uuid) -> anyhow::Result<Option<SnapshotChain>> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let mut stmt = c.prepare(
            "SELECT up_to_seq, compaction_floor_seq, epoch, from_seq, payload, payload_nonce
             FROM snapshots
             WHERE doc_id = ?1
               AND id >= (SELECT COALESCE(MAX(id), 0) FROM snapshots
                           WHERE doc_id = ?1 AND from_seq IS NULL)
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([&doc_bytes], |r| {
            Ok((
                r.get::<_, i64>(0)? as u64,
                r.get::<_, i64>(1)? as u64,
                r.get::<_, i64>(2)? as u64,
                r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                EncryptedBlob {
                    ciphertext: r.get::<_, Vec<u8>>(4)?,
                    nonce: r.get::<_, Vec<u8>>(5)?,
                },
            ))
        })?;
        let mut chain: Option<SnapshotChain> = None;
        for row in rows {
            let (up_to_seq, compaction_floor_seq, epoch, from_seq, blob) = row?;
            match (&mut chain, from_seq) {
                (None, None) => {
                    chain = Some(SnapshotChain {
                        up_to_seq,
                        compaction_floor_seq,
                        epoch,
                        rows: vec![SnapshotChainRow { up_to_seq, blob }],
                    });
                }
                (None, Some(_)) => {
                    tracing::warn!(up_to_seq, "snapshot chain has no full head; ignoring");
                    break;
                }
                (Some(c), Some(from)) if from == c.up_to_seq => {
                    c.up_to_seq = up_to_seq;
                    c.compaction_floor_seq = compaction_floor_seq;
                    c.epoch = epoch;
                    c.rows.push(SnapshotChainRow { up_to_seq, blob });
                }
                (Some(c), from) => {
                    tracing::warn!(
                        up_to_seq,
                        ?from,
                        chain_up_to_seq = c.up_to_seq,
                        "snapshot chain broken; serving the linked prefix"
                    );
                    break;
                }
            }
        }
        Ok(chain)
    })
    .await
}

/// Read a device's recorded frontier on a doc (0 when it has never
/// acked). Used by tests; production paths just call
/// `advance_last_acked_seq`.
pub async fn get_last_acked_seq(db: &Db, device_id: Uuid, doc_id: Uuid) -> anyhow::Result<u64> {
    let dev_bytes = device_id.as_bytes().to_vec();
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let v: Option<i64> = c
            .query_row(
                "SELECT last_acked_seq FROM device_doc_frontiers
                 WHERE device_id = ? AND doc_id = ?",
                params![dev_bytes, doc_bytes],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        Ok(v.unwrap_or(0) as u64)
    })
    .await
}

/// Horizon for a doc: `min(last_acked_seq)` across every device that
/// syncs it, with `override_device_id`'s row replaced by
/// `override_value`. Used as the compaction-floor input to the
/// snapshot coordinator (see `spec/sync-protocol.md` §"Snapshot
/// orchestration") and computed from the calling device's optimistic
/// frontier: during a push, the device has the just-assigned seqs
/// locally but hasn't sent the `Ack` yet, so its DB row would drag
/// horizon down artificially. Substituting the optimistic value
/// reflects the device's true frontier.
///
/// A device with no `device_doc_frontiers` row for the doc counts as
/// 0 — it has acked nothing, so nothing below it is safe to compact.
///
/// The device set is every device of every account with an active
/// `doc_members` row on the doc — the same set `can_open` admits, so
/// nobody who may pull the updates is left out of the floor.
///
/// Returns 0 when no member has a device — caller treats that as "no
/// compaction possible" via the trigger's horizon guard.
pub async fn doc_horizon(
    db: &Db,
    doc_id: Uuid,
    override_device_id: Uuid,
    override_value: u64,
) -> anyhow::Result<u64> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    let dev_bytes = override_device_id.as_bytes().to_vec();
    db.call(move |c| {
        // Every device of every account with an active membership on
        // the doc pins the horizon; a device with no frontier row is 0.
        let v: Option<i64> = c.query_row(
            "SELECT MIN(CASE WHEN d.id = ? THEN ? ELSE COALESCE(f.last_acked_seq, 0) END)
             FROM doc_members m
             JOIN devices d ON d.account_id = m.account_id
             LEFT JOIN device_doc_frontiers f
               ON f.device_id = d.id AND f.doc_id = m.doc_id
             WHERE m.doc_id = ? AND m.removed_at IS NULL",
            params![dev_bytes, override_value as i64, doc_bytes],
            |r| r.get::<_, Option<i64>>(0),
        )?;
        Ok(v.unwrap_or(0) as u64)
    })
    .await
}

pub struct CompactionStats {
    pub updates_deleted: u64,
}

/// Delete updates at or below the latest snapshot's `compaction_floor_seq`.
/// Snapshot rows are never deleted: the chain is one full head plus
/// every delta since, and only a trimmed snapshot (`storage.md`
/// §Epochs, not built) will supersede it. The floor read + delete run
/// in one transaction so a concurrent `insert_snapshot` can't shift
/// the floor mid-deletion. Returns 0 when no snapshot exists yet.
pub async fn compact_doc(db: &Db, doc_id: Uuid) -> anyhow::Result<CompactionStats> {
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
            return Ok(CompactionStats { updates_deleted: 0 });
        };
        let updates_deleted = tx.execute(
            "DELETE FROM updates WHERE doc_id = ? AND seq <= ?",
            params![doc_bytes, floor],
        )? as u64;
        tx.commit()?;
        Ok(CompactionStats { updates_deleted })
    })
    .await
}

pub async fn latest_doc_seq(db: &Db, doc_id: Uuid) -> anyhow::Result<u64> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let row = c
            .query_row(
                "SELECT seq
                 FROM updates
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
