//! Sqlite connection + migration runner.
//!
//! One connection per server for now. `tokio-rusqlite` runs queries on
//! a dedicated background thread, so the runtime is never blocked. WAL
//! mode means we can promote to a pool later without schema changes.

use std::path::Path;

use tokio_rusqlite::Connection;

const MIGRATION_001: &str = include_str!("../migrations/001_init.sql");

#[derive(Clone)]
pub struct Db {
    conn: Connection,
}

impl Db {
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path).await?;
        let db = Self { conn };
        db.apply_pragmas().await?;
        db.run_migrations().await?;
        Ok(db)
    }

    /// Used by integration tests so we don't litter temp files.
    pub async fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory().await?;
        let db = Self { conn };
        db.apply_pragmas().await?;
        db.run_migrations().await?;
        Ok(db)
    }

    /// Borrow the underlying connection for an arbitrary closure.
    pub async fn call<F, T>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&mut rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        Ok(self.conn.call(f).await?)
    }

    async fn apply_pragmas(&self) -> anyhow::Result<()> {
        self.call(|c| {
            // WAL gives us concurrent readers + a single writer without
            // blocking. NORMAL synchrony trades a single fsync per WAL
            // checkpoint for ~10x write throughput; we accept losing
            // the last few seconds of writes on a power-cut crash.
            c.execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=NORMAL;
                 PRAGMA busy_timeout=5000;
                 PRAGMA foreign_keys=ON;",
            )
        })
        .await?;
        Ok(())
    }

    async fn run_migrations(&self) -> anyhow::Result<()> {
        self.call(|c| {
            c.execute_batch(
                "CREATE TABLE IF NOT EXISTS _migrations (
                   name        TEXT PRIMARY KEY,
                   applied_at  INTEGER NOT NULL
                 );",
            )
        })
        .await?;

        self.apply_migration("001_init", MIGRATION_001).await?;
        Ok(())
    }

    async fn apply_migration(&self, name: &'static str, sql: &'static str) -> anyhow::Result<()> {
        self.call(move |c| {
            let already = migration_applied(c, name)?;
            if already {
                return Ok(());
            }
            let tx = c.transaction()?;
            tx.execute_batch(sql)?;
            record_migration(&tx, name)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
}

fn migration_applied(c: &rusqlite::Connection, name: &str) -> rusqlite::Result<bool> {
    c.query_row("SELECT 1 FROM _migrations WHERE name = ?", [name], |r| {
        r.get::<_, i64>(0)
    })
    .map(|_| true)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(false),
        other => Err(other),
    })
}

fn record_migration(c: &rusqlite::Connection, name: &str) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO _migrations (name, applied_at) VALUES (?, ?)",
        rusqlite::params![name, now_millis()],
    )?;
    Ok(())
}

pub fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
