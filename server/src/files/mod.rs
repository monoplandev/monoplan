//! Attachments: the server half of `spec/attachments.md`.
//!
//! Two things live here: the `doc_files` rows ([`queries`]) and the
//! bytes behind them ([`BlobStore`]). The row is the record. A write
//! stores the body first and inserts the row second; a purge deletes
//! the row first and the body second. A crash on either path leaves a
//! stored object with no row, which is garbage and harms nobody, and
//! never a live row whose bytes are gone.

pub mod queries;
mod store;

use std::fmt;
use std::time::Duration;

pub use store::{BlobStore, DirStore, MemStore};

use crate::state::AppState;

/// Largest plaintext a single attachment may hold.
pub const MAX_FILE_PLAINTEXT_BYTES: u64 = 25 * 1024 * 1024;

/// What sealing adds: a 24-byte XChaCha20 nonce and a 16-byte Poly1305
/// tag. Also the smallest body that can be a sealed file at all.
pub const SEAL_OVERHEAD_BYTES: u64 = 24 + 16;

/// Largest body `PUT` accepts.
pub const MAX_FILE_BODY_BYTES: u64 = MAX_FILE_PLAINTEXT_BYTES + SEAL_OVERHEAD_BYTES;

/// Default for `Config::max_account_file_bytes`.
pub const DEFAULT_MAX_ACCOUNT_FILE_BYTES: u64 = 1024 * 1024 * 1024;

/// How long a tombstoned file can still be restored.
pub const TOMBSTONE_GRACE_MILLIS: i64 = 30 * 24 * 60 * 60 * 1000;

const PURGE_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// A file id: 16 random bytes chosen by the attaching client, 32
/// lowercase hex characters on the wire and in store keys (the item id
/// grammar).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId([u8; 16]);

impl FileId {
    /// Strict: exactly 32 characters of `[0-9a-f]`. Uppercase is
    /// refused so one file never has two spellings.
    pub fn parse(s: &str) -> Option<Self> {
        if s.len() != 32 || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return None;
        }
        let mut out = [0u8; 16];
        hex::decode_to_slice(s, &mut out).ok()?;
        Some(Self(out))
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        Some(Self(bytes.try_into().ok()?))
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

/// Remove every file whose tombstone is older than the grace period:
/// the row, then the bytes. Besides a member's `DELETE` and
/// deleting the whole doc, this is the only place attachment bytes
/// are destroyed. The row delete
/// re-checks the tombstone, so a file restored while the pass runs
/// keeps both its row and its bytes.
pub async fn purge_expired(state: &AppState) -> anyhow::Result<usize> {
    let cutoff = state.clock.now_millis() - TOMBSTONE_GRACE_MILLIS;
    let mut purged = 0;
    for (doc_id, file_id) in queries::expired_tombstones(&state.db, cutoff).await? {
        if !queries::delete_expired_row(&state.db, doc_id, file_id, cutoff).await? {
            continue;
        }
        purged += 1;
        if let Err(err) = state.blobs.delete(doc_id, file_id).await {
            tracing::error!(doc_id = %doc_id, file_id = %file_id, error = %err, "purged attachment row but not its bytes");
        }
    }
    Ok(purged)
}

/// Run [`purge_expired`] now and then hourly, for the life of the
/// process.
pub fn spawn_purge_task(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(PURGE_INTERVAL);
        loop {
            tick.tick().await;
            match purge_expired(&state).await {
                Ok(0) => {}
                Ok(purged) => tracing::info!(purged, "purged tombstoned attachments"),
                Err(err) => tracing::error!(error = %err, "attachment purge failed"),
            }
        }
    })
}
