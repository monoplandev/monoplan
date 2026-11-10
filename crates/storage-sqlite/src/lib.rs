//! Native sqlite `LocalStorage` for Monoplan.
//!
//! `SqliteStorage` is the native implementation of `monoplan_core::LocalStorage`:
//! plain `rusqlite` behind a `Mutex`, synchronously durable (the trait
//! method returns only after the `INSERT`/`UPDATE` commits). The generic
//! schema is `migrations/001_init.sql` — an encrypted `wal` log plus one
//! `snapshots` row per doc, the per-doc sync cursors
//! (`last_acked_server_seq`, `server_known_vv`), and the single durable
//! `in_flight_push` record, keyed by `doc_id`. See
//! `spec/vv-wal-separation.md` for the model.
//!
//! This crate was hoisted out of the CLI so an FFI build (Apple, future
//! native clients) can share the exact same storage without pulling in
//! the CLI. It stays deliberately client-agnostic: it owns only the
//! shared doc-storage tables and the `_migrations` ledger. A caller that
//! needs its own tables (the CLI's singleton `account` identity row)
//! supplies them as *extra* migrations via [`SqliteStorage::open_with_extra`],
//! so identity and the doc cache share one db file and one transactional
//! store. Boot / seed / load glue lives in `monoplan_core::storage` — it is
//! generic over the trait and DEK-holding, so it belongs beside the trait,
//! not here.
//!
//! `core/` must stay wasm-clean, so this crate — not `monoplan-core` — is
//! where `rusqlite` lands.

use std::path::Path;
use std::sync::{Arc, Mutex};

use monoplan_core::{
    BootState, DocId, InFlightPush, LocalSeq, LocalStorage, PushId, RemoteWalRow, ServerSeq,
    SnapshotRow, StorageError, WalRow,
};
use monoplan_protocol::EncryptedBlob;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

const MIGRATION_001: &str = include_str!("../migrations/001_init.sql");

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Native `LocalStorage` over a sqlite file.
///
/// `Clone` is shallow: clones share one `Connection` behind the
/// `Arc<Mutex>`, so a replica and its owning session can both hold a
/// handle to the *same* db without a second file open. WAL + the `Mutex`
/// serialise their accesses.
#[derive(Clone)]
pub struct SqliteStorage {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStorage {
    /// Open (creating + migrating if needed) the sqlite file at `path`,
    /// applying only the generic doc-storage schema.
    pub fn open(path: &Path) -> Result<Self, DbError> {
        Self::open_with_extra(path, &[])
    }

    /// As [`open`](Self::open), but also apply caller-supplied extra
    /// migrations against the same db file after the core schema. Each
    /// entry is `(ledger_name, sql)`; `ledger_name` must be distinct from
    /// the core `"001_init"` and from every other extra migration, since
    /// the shared `_migrations` table dedupes by name. Used by the CLI to
    /// add its `account` table to the same file (see spec/cli.md).
    pub fn open_with_extra(path: &Path, extra: &[(&str, &str)]) -> Result<Self, DbError> {
        let mut conn = Connection::open(path)?;
        apply_pragmas(&conn)?;
        run_migrations(&mut conn, extra)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Wrap an already-open, already-migrated connection. Lets a caller
    /// that opened the db another way (or wants a second `SqliteStorage`
    /// view over an existing handle) reuse it — the shared connection is
    /// the single serialisation point.
    pub fn from_connection(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// The shared connection handle. Exposed so callers can run their own
    /// (non-doc) queries — e.g. the CLI's `account` / sync-cursor rows —
    /// against the same transactional store the trait writes to.
    pub fn connection(&self) -> &Arc<Mutex<Connection>> {
        &self.conn
    }
}

fn apply_pragmas(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA busy_timeout=5000;
         PRAGMA foreign_keys=ON;",
    )?;
    Ok(())
}

fn run_migrations(conn: &mut Connection, extra: &[(&str, &str)]) -> Result<(), DbError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
           name        TEXT PRIMARY KEY,
           applied_at  INTEGER NOT NULL
         );",
    )?;
    apply_migration(conn, "001_init", MIGRATION_001)?;
    for (name, sql) in extra {
        apply_migration(conn, name, sql)?;
    }
    Ok(())
}

fn apply_migration(c: &mut Connection, name: &str, sql: &str) -> Result<(), DbError> {
    if migration_applied(c, name)? {
        return Ok(());
    }
    let tx = c.transaction()?;
    tx.execute_batch(sql)?;
    tx.execute(
        "INSERT INTO _migrations (name, applied_at) VALUES (?, ?)",
        params![name, now_millis()],
    )?;
    tx.commit()?;
    Ok(())
}

fn migration_applied(c: &Connection, name: &str) -> Result<bool, DbError> {
    let found = c
        .query_row("SELECT 1 FROM _migrations WHERE name = ?", [name], |r| {
            r.get::<_, i64>(0)
        })
        .map(|_| true)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(false),
            other => Err(other),
        })?;
    Ok(found)
}

fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl LocalStorage for SqliteStorage {
    fn boot(&self, doc_id: DocId) -> Result<BootState, StorageError> {
        let conn = self.conn.lock().expect("SqliteStorage mutex poisoned");
        let id = doc_id.0.as_bytes().to_vec();

        let snapshot = conn
            .query_row(
                "SELECT up_to_local_seq, payload, payload_nonce FROM snapshots WHERE doc_id = ?1",
                [&id],
                |r| {
                    Ok(SnapshotRow {
                        up_to_local_seq: LocalSeq(r.get::<_, i64>(0)? as u64),
                        payload: blob_from(r.get(1)?, r.get(2)?),
                    })
                },
            )
            .optional()
            .map_err(backend)?;

        // Replay every surviving row: `write_snapshot` already pruned
        // the prefix the snapshot contains.
        let mut wal_bytes = 0u64;
        let replay = {
            let mut stmt = conn
                .prepare(
                    "SELECT local_seq, payload, payload_nonce FROM wal
                     WHERE doc_id = ?1 ORDER BY local_seq",
                )
                .map_err(backend)?;
            let rows = stmt
                .query_map(params![id], |r| {
                    Ok(WalRow {
                        local_seq: LocalSeq(r.get::<_, i64>(0)? as u64),
                        payload: blob_from(r.get(1)?, r.get(2)?),
                    })
                })
                .map_err(backend)?;
            let rows = rows
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(backend)?;
            for row in &rows {
                wal_bytes += (row.payload.ciphertext.len() + row.payload.nonce.len()) as u64;
            }
            rows
        };

        let last_local_seq = LocalSeq(max_local_seq(&conn, &id)?);
        // Read the persisted cursors, never derived from the WAL rows
        // (which folding prunes). See `docs.last_acked_server_seq` /
        // `docs.server_known_vv`.
        let (last_acked, server_known_vv): (i64, Option<Vec<u8>>) = conn
            .query_row(
                "SELECT COALESCE(last_acked_server_seq, 0), server_known_vv
                 FROM docs WHERE id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(backend)?
            .unwrap_or((0, None));

        let in_flight_push = conn
            .query_row(
                "SELECT push_id, payload, payload_nonce, from_vv, to_vv
                 FROM in_flight_push WHERE doc_id = ?1",
                [&id],
                |r| {
                    Ok((
                        r.get::<_, Vec<u8>>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, Vec<u8>>(2)?,
                        r.get::<_, Vec<u8>>(3)?,
                        r.get::<_, Vec<u8>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(backend)?
            .map(|(push_id, ciphertext, nonce, from_vv, to_vv)| {
                Ok::<_, StorageError>(InFlightPush {
                    push_id: PushId(uuid_from_slice(&push_id)?),
                    payload: EncryptedBlob { nonce, ciphertext },
                    from_vv,
                    to_vv,
                })
            })
            .transpose()?;

        Ok(BootState {
            snapshot,
            replay,
            last_local_seq,
            last_acked_server_seq: ServerSeq(last_acked as u64),
            server_known_vv: server_known_vv.unwrap_or_default(),
            in_flight_push,
            wal_bytes,
        })
    }

    fn append_local_wal(
        &self,
        doc_id: DocId,
        payload: EncryptedBlob,
    ) -> Result<LocalSeq, StorageError> {
        let conn = self.conn.lock().expect("SqliteStorage mutex poisoned");
        let id = doc_id.0.as_bytes().to_vec();
        ensure_doc(&conn, &id)?;
        let next = max_local_seq(&conn, &id)? + 1;
        conn.execute(
            "INSERT INTO wal
               (doc_id, local_seq, server_seq, payload, payload_nonce, created_at)
             VALUES (?1, ?2, NULL, ?3, ?4, unixepoch())",
            params![id, next as i64, payload.ciphertext, payload.nonce],
        )
        .map_err(backend)?;
        Ok(LocalSeq(next))
    }

    fn append_remote_wal(
        &self,
        doc_id: DocId,
        row: RemoteWalRow,
        server_known_vv: &[u8],
    ) -> Result<(LocalSeq, bool), StorageError> {
        let mut conn = self.conn.lock().expect("SqliteStorage mutex poisoned");
        let id = doc_id.0.as_bytes().to_vec();
        let tx = conn.transaction().map_err(backend)?;
        ensure_doc(&tx, &id)?;
        // The VV write and the row append commit together — and the VV
        // advances on the duplicate path too, because a re-delivered
        // blob still proves the server has those ops.
        tx.execute(
            "UPDATE docs SET server_known_vv = ?2 WHERE id = ?1",
            params![id, server_known_vv],
        )
        .map_err(backend)?;
        // Idempotent on server_seq: a re-delivered row (resume re-pull,
        // broadcast overlap) returns its existing local_seq rather than
        // violating the unique index or minting a phantom row.
        let existing = tx
            .query_row(
                "SELECT local_seq FROM wal WHERE doc_id = ?1 AND server_seq = ?2",
                params![id, row.server_seq.0 as i64],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .map_err(backend)?;
        let result = if let Some(existing) = existing {
            (LocalSeq(existing as u64), false)
        } else {
            let next = max_local_seq(&tx, &id)? + 1;
            tx.execute(
                "INSERT INTO wal
                   (doc_id, local_seq, server_seq, payload, payload_nonce, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, unixepoch())",
                params![
                    id,
                    next as i64,
                    row.server_seq.0 as i64,
                    row.payload.ciphertext,
                    row.payload.nonce,
                ],
            )
            .map_err(backend)?;
            (LocalSeq(next), true)
        };
        tx.commit().map_err(backend)?;
        Ok(result)
    }

    fn write_snapshot(
        &self,
        doc_id: DocId,
        cutoff: LocalSeq,
        payload: EncryptedBlob,
    ) -> Result<(), StorageError> {
        let mut conn = self.conn.lock().expect("SqliteStorage mutex poisoned");
        let id = doc_id.0.as_bytes().to_vec();
        let tx = conn.transaction().map_err(backend)?;
        ensure_doc(&tx, &id)?;
        // Record the current high-water (before pruning drops the rows
        // carrying it) so post-prune appends stay monotonic.
        let high_water = max_local_seq(&tx, &id)? as i64;
        tx.execute(
            "INSERT INTO snapshots
               (doc_id, up_to_local_seq, payload, payload_nonce, created_at)
             VALUES (?1, ?2, ?3, ?4, unixepoch())
             ON CONFLICT(doc_id) DO UPDATE SET
               up_to_local_seq = excluded.up_to_local_seq,
               payload         = excluded.payload,
               payload_nonce   = excluded.payload_nonce,
               created_at      = excluded.created_at",
            params![id, high_water, payload.ciphertext, payload.nonce],
        )
        .map_err(backend)?;
        // Prune the folded prefix unconditionally — the full-history
        // snapshot provably contains every row at or below the cutoff,
        // including unsent local operations (upload is VV-derived, not
        // row-derived). Rows appended after the cutoff survive.
        tx.execute(
            "DELETE FROM wal WHERE doc_id = ?1 AND local_seq <= ?2",
            params![id, cutoff.0 as i64],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)?;
        Ok(())
    }

    fn write_acked_seq(&self, doc_id: DocId, seq: ServerSeq) -> Result<(), StorageError> {
        let conn = self.conn.lock().expect("SqliteStorage mutex poisoned");
        let id = doc_id.0.as_bytes().to_vec();
        ensure_doc(&conn, &id)?;
        conn.execute(
            "UPDATE docs SET last_acked_server_seq = ?2 WHERE id = ?1",
            params![id, seq.0 as i64],
        )
        .map_err(backend)?;
        Ok(())
    }

    fn write_server_known_vv(&self, doc_id: DocId, vv: &[u8]) -> Result<(), StorageError> {
        let conn = self.conn.lock().expect("SqliteStorage mutex poisoned");
        let id = doc_id.0.as_bytes().to_vec();
        ensure_doc(&conn, &id)?;
        conn.execute(
            "UPDATE docs SET server_known_vv = ?2 WHERE id = ?1",
            params![id, vv],
        )
        .map_err(backend)?;
        Ok(())
    }

    fn put_in_flight_push(&self, doc_id: DocId, push: InFlightPush) -> Result<(), StorageError> {
        let conn = self.conn.lock().expect("SqliteStorage mutex poisoned");
        let id = doc_id.0.as_bytes().to_vec();
        ensure_doc(&conn, &id)?;
        conn.execute(
            "INSERT INTO in_flight_push
               (doc_id, push_id, payload, payload_nonce, from_vv, to_vv, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch())
             ON CONFLICT(doc_id) DO UPDATE SET
               push_id       = excluded.push_id,
               payload       = excluded.payload,
               payload_nonce = excluded.payload_nonce,
               from_vv       = excluded.from_vv,
               to_vv         = excluded.to_vv,
               created_at    = excluded.created_at",
            params![
                id,
                push.push_id.0.as_bytes().to_vec(),
                push.payload.ciphertext,
                push.payload.nonce,
                push.from_vv,
                push.to_vv,
            ],
        )
        .map_err(backend)?;
        Ok(())
    }

    fn complete_push(
        &self,
        doc_id: DocId,
        push_id: PushId,
        server_known_vv: &[u8],
    ) -> Result<(), StorageError> {
        let mut conn = self.conn.lock().expect("SqliteStorage mutex poisoned");
        let id = doc_id.0.as_bytes().to_vec();
        let tx = conn.transaction().map_err(backend)?;
        ensure_doc(&tx, &id)?;
        tx.execute(
            "UPDATE docs SET server_known_vv = ?2 WHERE id = ?1",
            params![id, server_known_vv],
        )
        .map_err(backend)?;
        // Clear only the matching record — a stale complete (crossed
        // wires with a fresher push) must not drop the newer record.
        tx.execute(
            "DELETE FROM in_flight_push WHERE doc_id = ?1 AND push_id = ?2",
            params![id, push_id.0.as_bytes().to_vec()],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)?;
        Ok(())
    }
}

fn backend(e: rusqlite::Error) -> StorageError {
    StorageError::Backend(e.to_string())
}

fn blob_from(ciphertext: Vec<u8>, nonce: Vec<u8>) -> EncryptedBlob {
    EncryptedBlob { nonce, ciphertext }
}

fn uuid_from_slice(bytes: &[u8]) -> Result<Uuid, StorageError> {
    Uuid::from_slice(bytes).map_err(|e| StorageError::Backend(format!("invalid uuid bytes: {e}")))
}

fn ensure_doc(conn: &Connection, id: &[u8]) -> Result<(), StorageError> {
    conn.execute(
        "INSERT OR IGNORE INTO docs (id, created_at) VALUES (?1, unixepoch())",
        [id],
    )
    .map_err(backend)?;
    Ok(())
}

/// `max(snapshot.up_to_local_seq, max wal.local_seq)` — the highest
/// `local_seq` ever assigned for this doc. The snapshot term (its stored
/// high-water) matters after a prune deletes the rows carrying the max:
/// otherwise the next append would restart `local_seq` and collide
/// with a surviving row.
fn max_local_seq(conn: &Connection, id: &[u8]) -> Result<u64, StorageError> {
    let n: i64 = conn
        .query_row(
            "SELECT MAX(
                 COALESCE((SELECT MAX(local_seq)      FROM wal       WHERE doc_id = ?1), 0),
                 COALESCE((SELECT up_to_local_seq     FROM snapshots WHERE doc_id = ?1), 0)
             )",
            [id],
            |r| r.get(0),
        )
        .map_err(backend)?;
    Ok(n as u64)
}
