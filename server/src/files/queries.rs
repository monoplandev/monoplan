//! Sqlite reads/writes on `doc_files`.

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::FileId;
use crate::db::Db;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub id: FileId,
    /// Stored (ciphertext) size.
    pub bytes: u64,
    pub created_at: i64,
    pub deleted_at: Option<i64>,
}

/// What the quota is checked against: the live (not tombstoned) bytes
/// across **every doc owned by this doc's owner**. The allowance is the
/// owner account's, pooled over its workspaces, so an upload to a
/// shared workspace counts against whoever owns it, not whoever sent
/// it, and creating another workspace adds no space. A doc has exactly
/// one active owner; one with none (not a state the server produces)
/// counts as empty.
fn owner_used_bytes_in(c: &Connection, doc_bytes: &[u8]) -> rusqlite::Result<u64> {
    let used: i64 = c.query_row(
        "SELECT COALESCE(SUM(f.bytes), 0)
         FROM doc_files f
         JOIN doc_members m
           ON m.doc_id = f.doc_id AND m.role = 'owner' AND m.removed_at IS NULL
         WHERE f.deleted_at IS NULL
           AND m.account_id = (
             SELECT account_id FROM doc_members
             WHERE doc_id = ? AND role = 'owner' AND removed_at IS NULL
           )",
        params![doc_bytes],
        |r| r.get(0),
    )?;
    Ok(used.max(0) as u64)
}

/// [`owner_used_bytes_in`] for the owner of `doc_id`.
pub async fn owner_used_bytes(db: &Db, doc_id: Uuid) -> anyhow::Result<u64> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| owner_used_bytes_in(c, &doc_bytes)).await
}

/// What `GET .../files` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileListing {
    /// Live bytes in this doc alone.
    pub doc_bytes: u64,
    /// The owner's pooled usage, only when the caller is that owner: to
    /// anyone else it would say something about workspaces they are
    /// not in.
    pub owner_used_bytes: Option<u64>,
    pub files: Vec<FileRow>,
}

pub async fn find_file(db: &Db, doc_id: Uuid, file_id: FileId) -> anyhow::Result<Option<FileRow>> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        c.query_row(
            "SELECT bytes, created_at, deleted_at FROM doc_files
             WHERE doc_id = ? AND id = ?",
            params![doc_bytes, file_id.as_bytes().as_slice()],
            |r| {
                Ok(FileRow {
                    id: file_id,
                    bytes: r.get::<_, i64>(0)?.max(0) as u64,
                    created_at: r.get(1)?,
                    deleted_at: r.get(2)?,
                })
            },
        )
        .optional()
    })
    .await
}

/// Every row of the doc, tombstoned ones included, oldest first, with
/// the usage figures read in the same call.
pub async fn list_files(db: &Db, doc_id: Uuid, account_id: Uuid) -> anyhow::Result<FileListing> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    let acc_bytes = account_id.as_bytes().to_vec();
    db.call(move |c| {
        let is_owner: i64 = c.query_row(
            "SELECT COUNT(*) FROM doc_members
             WHERE doc_id = ? AND account_id = ? AND role = 'owner' AND removed_at IS NULL",
            params![doc_bytes, acc_bytes],
            |r| r.get(0),
        )?;
        let owner_used_bytes = if is_owner > 0 {
            Some(owner_used_bytes_in(c, &doc_bytes)?)
        } else {
            None
        };
        let mut stmt = c.prepare(
            "SELECT id, bytes, created_at, deleted_at FROM doc_files
             WHERE doc_id = ? ORDER BY created_at, id",
        )?;
        let rows = stmt
            .query_map(params![doc_bytes], |r| {
                let id_bytes: Vec<u8> = r.get(0)?;
                let id = FileId::from_bytes(&id_bytes).ok_or_else(|| {
                    rusqlite::Error::InvalidColumnType(0, "id".into(), rusqlite::types::Type::Blob)
                })?;
                Ok(FileRow {
                    id,
                    bytes: r.get::<_, i64>(1)?.max(0) as u64,
                    created_at: r.get(2)?,
                    deleted_at: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let doc_bytes = rows
            .iter()
            .filter(|r| r.deleted_at.is_none())
            .map(|r| r.bytes)
            .sum();
        Ok(FileListing {
            doc_bytes,
            owner_used_bytes,
            files: rows,
        })
    })
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertFile {
    Inserted,
    /// A row with this id is already there, live or tombstoned.
    Exists,
    /// The row would take the doc's owner over `limit_bytes`.
    OverQuota,
}

/// Record a body that is already in the blob store. The existence and
/// quota checks share the transaction with the insert, so concurrent
/// uploads cannot overshoot the limit.
pub async fn insert_file(
    db: &Db,
    doc_id: Uuid,
    file_id: FileId,
    bytes: u64,
    device_id: Uuid,
    now: i64,
    limit_bytes: u64,
) -> anyhow::Result<InsertFile> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    let device_bytes = device_id.as_bytes().to_vec();
    db.call(move |c| {
        let tx = c.transaction()?;
        let exists: i64 = tx.query_row(
            "SELECT COUNT(*) FROM doc_files WHERE doc_id = ? AND id = ?",
            params![doc_bytes, file_id.as_bytes().as_slice()],
            |r| r.get(0),
        )?;
        if exists > 0 {
            return Ok(InsertFile::Exists);
        }
        if owner_used_bytes_in(&tx, &doc_bytes)?.saturating_add(bytes) > limit_bytes {
            return Ok(InsertFile::OverQuota);
        }
        tx.execute(
            "INSERT INTO doc_files (doc_id, id, bytes, device_id, created_at)
             VALUES (?, ?, ?, ?, ?)",
            params![
                doc_bytes,
                file_id.as_bytes().as_slice(),
                bytes as i64,
                device_bytes,
                now
            ],
        )?;
        tx.commit()?;
        Ok(InsertFile::Inserted)
    })
    .await
}

/// Stamp `deleted_at`. `false` when there is no such row. A row that
/// is already tombstoned keeps its original stamp, so repeating the
/// call never extends the grace period.
pub async fn tombstone_file(
    db: &Db,
    doc_id: Uuid,
    file_id: FileId,
    now: i64,
) -> anyhow::Result<bool> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let n = c.execute(
            "UPDATE doc_files SET deleted_at = COALESCE(deleted_at, ?)
             WHERE doc_id = ? AND id = ?",
            params![now, doc_bytes, file_id.as_bytes().as_slice()],
        )?;
        Ok(n > 0)
    })
    .await
}

/// Delete a row outright, tombstoned or not: the record half of a
/// `DELETE`. `false` when there is no such row.
pub async fn delete_file_row(db: &Db, doc_id: Uuid, file_id: FileId) -> anyhow::Result<bool> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let n = c.execute(
            "DELETE FROM doc_files WHERE doc_id = ? AND id = ?",
            params![doc_bytes, file_id.as_bytes().as_slice()],
        )?;
        Ok(n > 0)
    })
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreFile {
    /// The tombstone is cleared (or was never set).
    Restored,
    NotFound,
    /// Bringing the bytes back would take the doc's owner over
    /// `limit_bytes`.
    OverQuota,
}

/// Clear a tombstone. Checked against the quota, because a tombstoned
/// row does not count towards it: without the check, delete / upload /
/// restore in a loop would store without bound.
pub async fn restore_file(
    db: &Db,
    doc_id: Uuid,
    file_id: FileId,
    limit_bytes: u64,
) -> anyhow::Result<RestoreFile> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let tx = c.transaction()?;
        let row: Option<(i64, Option<i64>)> = tx
            .query_row(
                "SELECT bytes, deleted_at FROM doc_files WHERE doc_id = ? AND id = ?",
                params![doc_bytes, file_id.as_bytes().as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((bytes, deleted_at)) = row else {
            return Ok(RestoreFile::NotFound);
        };
        if deleted_at.is_none() {
            return Ok(RestoreFile::Restored);
        }
        if owner_used_bytes_in(&tx, &doc_bytes)?.saturating_add(bytes.max(0) as u64) > limit_bytes {
            return Ok(RestoreFile::OverQuota);
        }
        tx.execute(
            "UPDATE doc_files SET deleted_at = NULL WHERE doc_id = ? AND id = ?",
            params![doc_bytes, file_id.as_bytes().as_slice()],
        )?;
        tx.commit()?;
        Ok(RestoreFile::Restored)
    })
    .await
}

/// Rows tombstoned at or before `cutoff`, across every doc.
pub async fn expired_tombstones(db: &Db, cutoff: i64) -> anyhow::Result<Vec<(Uuid, FileId)>> {
    db.call(move |c| {
        let mut stmt = c.prepare(
            "SELECT doc_id, id FROM doc_files
             WHERE deleted_at IS NOT NULL AND deleted_at <= ?",
        )?;
        let rows = stmt
            .query_map(params![cutoff], |r| {
                Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(doc, id)| Some((Uuid::from_slice(&doc).ok()?, FileId::from_bytes(&id)?)))
            .collect())
    })
    .await
}

/// Delete one row, only if it is still tombstoned at or before
/// `cutoff`: a restore that landed since the scan keeps its row.
pub async fn delete_expired_row(
    db: &Db,
    doc_id: Uuid,
    file_id: FileId,
    cutoff: i64,
) -> anyhow::Result<bool> {
    let doc_bytes = doc_id.as_bytes().to_vec();
    db.call(move |c| {
        let n = c.execute(
            "DELETE FROM doc_files
             WHERE doc_id = ? AND id = ? AND deleted_at IS NOT NULL AND deleted_at <= ?",
            params![doc_bytes, file_id.as_bytes().as_slice(), cutoff],
        )?;
        Ok(n > 0)
    })
    .await
}
