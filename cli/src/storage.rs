//! CLI local persistence.
//!
//! The generic doc storage — the `wal` / `snapshots` log and the
//! `LocalStorage` trait impl — now lives in `monoplan-storage-sqlite` so
//! the FFI / native app builds can share it. This module is the thin
//! CLI-specific layer on top: it adds the singleton `account` identity
//! row and the per-doc sync-cursor columns to the *same* db file, and
//! re-exports the DEK-holding boot/seed/load glue (which lives beside the
//! `LocalStorage` trait in `monoplan-core`).
//!
//! `SqliteStorage` here is a newtype over the shared backend. It carries
//! the CLI's inherent account/cursor queries (which reach the shared
//! connection directly) and forwards the `LocalStorage` trait to the
//! backend, so the engine and every command keep the same API they had
//! when this type was defined in-crate.

use std::sync::Arc;
use std::sync::Mutex;

use monoplan_core::{
    BootState, DocId, InFlightPush, LocalSeq, LocalStorage, PushId, RemoteWalRow, ServerSeq,
    StorageError,
};
use monoplan_protocol::EncryptedBlob;
use monoplan_storage_sqlite::SqliteStorage as SqliteBackend;
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::config::Profile;

pub use monoplan_core::{BootError, boot_doc, has_unsynced_ops, load_doc, seed_snapshot};
pub use monoplan_storage_sqlite::DbError;

/// Ledger name for the CLI's extra migration (`cli/migrations/001_init.sql`).
/// Distinct from the storage crate's own `"001_init"` so both can live in
/// the one shared `_migrations` table without colliding.
const CLI_MIGRATION_NAME: &str = "001_cli";
const CLI_MIGRATION_SQL: &str = include_str!("../migrations/001_init.sql");

/// The doc this install runs and, when it has one, the account + device
/// identity behind it. Persisted as the singleton `account` row (see
/// `001_init.sql`); written at first use / signup / login / recover,
/// read by every command that needs the doc id or the identity.
#[derive(Debug, Clone)]
pub struct Account {
    /// The primary (Home) doc. Server-assigned at login / recover;
    /// minted locally for a local-only workspace and kept when signup
    /// adopts it.
    pub primary_doc_id: DocId,
    /// `None` for a local-only workspace: no server account, nothing
    /// to sync to. `Some` once this install is a device of an account.
    pub identity: Option<Identity>,
}

/// The server-side identity of an install that has an account.
#[derive(Debug, Clone)]
pub struct Identity {
    pub account_id: String,
    pub email: String,
    pub device_id: String,
}

impl Account {
    /// The identity, or the typed "no account" error for commands that
    /// need a server behind them.
    pub fn identity(&self) -> Result<&Identity, crate::config::ConfigError> {
        self.identity
            .as_ref()
            .ok_or(crate::config::ConfigError::NoAccount)
    }

    pub fn is_local_only(&self) -> bool {
        self.identity.is_none()
    }
}

/// Per-doc sync cursor (the `last_acked_server_seq` / `last_sync_at`
/// columns on `docs`). `last_acked_server_seq` is the highest server_seq
/// pulled+applied — the pull cursor for the next sync. `last_sync_at` is
/// unix millis of the last successful online flush (`None` = never).
#[derive(Debug, Clone, Copy)]
pub struct SyncCursor {
    pub last_acked_server_seq: ServerSeq,
    pub last_sync_at: Option<i64>,
}

/// Native `LocalStorage` over a per-profile sqlite file — a CLI newtype
/// around the shared [`SqliteBackend`].
///
/// `Clone` is shallow: clones share one connection (see the backend's
/// `Clone`), so the engine and the owning `Session` can both hold a
/// handle to the *same* db without a second file open.
#[derive(Clone)]
pub struct SqliteStorage(SqliteBackend);

impl SqliteStorage {
    /// Open (creating + migrating if needed) the sqlite file at `path`,
    /// applying the shared doc-storage schema plus the CLI's `account`
    /// table.
    pub fn open(path: &std::path::Path) -> Result<Self, DbError> {
        let backend =
            SqliteBackend::open_with_extra(path, &[(CLI_MIGRATION_NAME, CLI_MIGRATION_SQL)])?;
        Ok(Self(backend))
    }

    /// The shared connection, for the CLI's own (non-doc) queries.
    fn conn(&self) -> &Arc<Mutex<Connection>> {
        self.0.connection()
    }

    /// Read the singleton account row. Errors if there is none (callers
    /// gate on `Profile::require_active` first, so a missing row means
    /// a corrupt/half-written profile).
    pub fn read_account(&self) -> Result<Account, StorageError> {
        let conn = self.conn().lock().expect("SqliteStorage mutex poisoned");
        let row = conn
            .query_row(
                "SELECT account_id, email, device_id, primary_doc_id FROM account WHERE id = 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Vec<u8>>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(backend)?;
        let (account_id, email, device_id, doc_bytes) =
            row.ok_or_else(|| StorageError::Backend("account row missing".into()))?;
        // The schema's CHECK keeps the three together; a partial row
        // can only be a hand edit, and is reported rather than read as
        // local-only.
        let identity = match (account_id, email, device_id) {
            (None, None, None) => None,
            (Some(account_id), Some(email), Some(device_id)) => Some(Identity {
                account_id,
                email,
                device_id,
            }),
            _ => {
                return Err(StorageError::Backend(
                    "account row has a partial identity".into(),
                ));
            }
        };
        Ok(Account {
            primary_doc_id: DocId(uuid_from_slice(&doc_bytes)?),
            identity,
        })
    }

    /// Upsert the singleton account row.
    pub fn write_account(&self, account: &Account) -> Result<(), StorageError> {
        let conn = self.conn().lock().expect("SqliteStorage mutex poisoned");
        let identity = account.identity.as_ref();
        conn.execute(
            "INSERT INTO account (id, account_id, email, device_id, primary_doc_id)
             VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
               account_id     = excluded.account_id,
               email          = excluded.email,
               device_id      = excluded.device_id,
               primary_doc_id = excluded.primary_doc_id",
            params![
                identity.map(|i| i.account_id.as_str()),
                identity.map(|i| i.email.as_str()),
                identity.map(|i| i.device_id.as_str()),
                account.primary_doc_id.0.as_bytes().to_vec(),
            ],
        )
        .map_err(backend)?;
        Ok(())
    }

    /// Read slot's peer id, minting `mint` as its permanent value on
    /// the slot's first claim (see `peer_slots` in `001_init.sql` and
    /// `spec/peer-id-plan.md`). Stored as an i64 bit-cast of the u64.
    /// The caller holds the slot's flock, so there is no same-slot
    /// race; `INSERT OR IGNORE` + re-read is belt and braces.
    pub fn read_or_mint_peer_slot(&self, slot: u32, mint: u64) -> Result<u64, StorageError> {
        let conn = self.conn().lock().expect("SqliteStorage mutex poisoned");
        conn.execute(
            "INSERT OR IGNORE INTO peer_slots (slot, peer_id) VALUES (?1, ?2)",
            params![slot, mint as i64],
        )
        .map_err(backend)?;
        let peer: i64 = conn
            .query_row(
                "SELECT peer_id FROM peer_slots WHERE slot = ?1",
                [slot],
                |r| r.get(0),
            )
            .map_err(backend)?;
        Ok(peer as u64)
    }

    /// Read the per-doc sync cursor. Returns the zero cursor for a doc
    /// with no row yet.
    pub fn read_sync_cursor(&self, doc_id: DocId) -> Result<SyncCursor, StorageError> {
        let conn = self.conn().lock().expect("SqliteStorage mutex poisoned");
        let id = doc_id.0.as_bytes().to_vec();
        let row = conn
            .query_row(
                "SELECT last_acked_server_seq, last_sync_at FROM docs WHERE id = ?1",
                [&id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)),
            )
            .optional()
            .map_err(backend)?;
        Ok(match row {
            Some((acked, at)) => SyncCursor {
                last_acked_server_seq: ServerSeq(acked as u64),
                last_sync_at: at,
            },
            None => SyncCursor {
                last_acked_server_seq: ServerSeq(0),
                last_sync_at: None,
            },
        })
    }

    /// Persist the observability timestamp only — the "Last sync" shown
    /// by `monoplan status`. Deliberately separate from the resume cursor:
    /// the engine owns `last_acked_server_seq` (via the `LocalStorage`
    /// trait's `write_acked_seq`), while `last_sync_at` is a CLI-only
    /// stamp nothing in the sync path reads. (Creates the `docs` row if
    /// the first write lands before any op.)
    pub fn write_last_sync_at(&self, doc_id: DocId, at_millis: i64) -> Result<(), StorageError> {
        let conn = self.conn().lock().expect("SqliteStorage mutex poisoned");
        let id = doc_id.0.as_bytes().to_vec();
        ensure_doc(&conn, &id)?;
        conn.execute(
            "UPDATE docs SET last_sync_at = ?2 WHERE id = ?1",
            params![id, at_millis],
        )
        .map_err(backend)?;
        Ok(())
    }

    /// Drop the doc cache (WAL + snapshot + in-flight push) and reset
    /// every sync cursor, keeping account identity. Backs `monoplan cache
    /// clear`: the next sync rehydrates from server_seq 0.
    pub fn clear_cache(&self, doc_id: DocId) -> Result<(), StorageError> {
        let mut conn = self.conn().lock().expect("SqliteStorage mutex poisoned");
        let id = doc_id.0.as_bytes().to_vec();
        let tx = conn.transaction().map_err(backend)?;
        tx.execute("DELETE FROM wal WHERE doc_id = ?1", [&id])
            .map_err(backend)?;
        tx.execute("DELETE FROM snapshots WHERE doc_id = ?1", [&id])
            .map_err(backend)?;
        tx.execute("DELETE FROM in_flight_push WHERE doc_id = ?1", [&id])
            .map_err(backend)?;
        tx.execute(
            "UPDATE docs SET last_acked_server_seq = 0, server_known_vv = NULL,
                             last_sync_at = NULL WHERE id = ?1",
            [&id],
        )
        .map_err(backend)?;
        tx.commit().map_err(backend)?;
        Ok(())
    }
}

/// Forward the whole `LocalStorage` trait to the shared backend — the
/// engine only ever sees the trait, so the newtype is transparent to it.
impl LocalStorage for SqliteStorage {
    fn boot(&self, doc_id: DocId) -> Result<BootState, StorageError> {
        self.0.boot(doc_id)
    }
    fn append_local_wal(
        &self,
        doc_id: DocId,
        payload: EncryptedBlob,
    ) -> Result<LocalSeq, StorageError> {
        self.0.append_local_wal(doc_id, payload)
    }
    fn append_remote_wal(
        &self,
        doc_id: DocId,
        row: RemoteWalRow,
        server_known_vv: &[u8],
    ) -> Result<(LocalSeq, bool), StorageError> {
        self.0.append_remote_wal(doc_id, row, server_known_vv)
    }
    fn write_snapshot(
        &self,
        doc_id: DocId,
        cutoff: LocalSeq,
        payload: EncryptedBlob,
    ) -> Result<(), StorageError> {
        self.0.write_snapshot(doc_id, cutoff, payload)
    }
    fn write_acked_seq(&self, doc_id: DocId, seq: ServerSeq) -> Result<(), StorageError> {
        self.0.write_acked_seq(doc_id, seq)
    }
    fn write_server_known_vv(&self, doc_id: DocId, vv: &[u8]) -> Result<(), StorageError> {
        self.0.write_server_known_vv(doc_id, vv)
    }
    fn put_in_flight_push(&self, doc_id: DocId, push: InFlightPush) -> Result<(), StorageError> {
        self.0.put_in_flight_push(doc_id, push)
    }
    fn complete_push(
        &self,
        doc_id: DocId,
        push_id: PushId,
        server_known_vv: &[u8],
    ) -> Result<(), StorageError> {
        self.0.complete_push(doc_id, push_id, server_known_vv)
    }
}

fn backend(e: rusqlite::Error) -> StorageError {
    StorageError::Backend(e.to_string())
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

/// Open the profile's `SqliteStorage`.
pub fn open_storage(profile: &Profile) -> Result<SqliteStorage, DbError> {
    SqliteStorage::open(&profile.doc_path())
}
