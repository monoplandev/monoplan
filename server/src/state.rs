use std::path::Path;
use std::sync::Arc;

use argon2::{Argon2, PasswordHash, PasswordVerifier};

use crate::config::DEFAULT_MAX_WORKSPACES;
use crate::db::Db;
use crate::sync::{SnapshotCoordinator, SyncSessions};

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub clock: Arc<dyn Clock>,
    pub sync_sessions: SyncSessions,
    pub snapshot_coordinator: SnapshotCoordinator,
    /// Mirror of `Config::secure_cookies`. Lives on state because the
    /// cookie helpers run from request handlers, not at startup.
    pub secure_cookies: bool,
    /// Mirror of `Config::max_workspaces`: the most docs one account
    /// may own, Home included.
    pub max_workspaces: u32,
    admin_auth: Option<AdminAuth>,
}

#[derive(Clone)]
struct AdminAuth {
    password_hash: Arc<str>,
}

impl AdminAuth {
    fn new(password_hash: impl Into<Arc<str>>) -> anyhow::Result<Self> {
        let password_hash = password_hash.into();
        let parsed = PasswordHash::new(&password_hash)
            .map_err(|e| anyhow::anyhow!("invalid admin_password_hash PHC string: {e}"))?;
        if parsed.algorithm.as_str() != "argon2id" {
            anyhow::bail!("admin_password_hash must use Argon2id");
        }
        Ok(Self { password_hash })
    }

    fn verify(&self, password: &[u8]) -> bool {
        let Ok(hash) = PasswordHash::new(&self.password_hash) else {
            return false;
        };
        Argon2::default().verify_password(password, &hash).is_ok()
    }
}

impl AppState {
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            db: Db::open(path).await?,
            clock: Arc::new(SystemClock),
            sync_sessions: SyncSessions::new(),
            snapshot_coordinator: SnapshotCoordinator::new(),
            secure_cookies: true,
            max_workspaces: DEFAULT_MAX_WORKSPACES,
            admin_auth: None,
        })
    }

    pub async fn open_in_memory() -> anyhow::Result<Self> {
        Ok(Self {
            db: Db::open_in_memory().await?,
            clock: Arc::new(SystemClock),
            sync_sessions: SyncSessions::new(),
            snapshot_coordinator: SnapshotCoordinator::new(),
            secure_cookies: true,
            max_workspaces: DEFAULT_MAX_WORKSPACES,
            admin_auth: None,
        })
    }

    pub fn with_secure_cookies(mut self, secure: bool) -> Self {
        self.secure_cookies = secure;
        self
    }

    pub fn with_max_workspaces(mut self, max: u32) -> Self {
        self.max_workspaces = max;
        self
    }

    pub fn with_snapshot_threshold_blobs(mut self, threshold_blobs: u64) -> Self {
        self.snapshot_coordinator = SnapshotCoordinator::with_threshold_blobs(threshold_blobs);
        self
    }

    pub fn with_admin_password_hash(
        mut self,
        password_hash: impl Into<Arc<str>>,
    ) -> anyhow::Result<Self> {
        self.admin_auth = Some(AdminAuth::new(password_hash)?);
        Ok(self)
    }

    pub(crate) fn admin_enabled(&self) -> bool {
        self.admin_auth.is_some()
    }

    pub(crate) async fn verify_admin_password(&self, password: &[u8]) -> bool {
        let Some(auth) = self.admin_auth.clone() else {
            return false;
        };
        let password = password.to_vec();
        tokio::task::spawn_blocking(move || auth.verify(&password))
            .await
            .unwrap_or(false)
    }
}

/// Indirection so tests can fast-forward time without sleeping (used
/// for recovery-session expiry today, will see more use as features
/// land).
pub trait Clock: Send + Sync + std::fmt::Debug + 'static {
    fn now_millis(&self) -> i64;
}

#[derive(Debug)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> i64 {
        crate::db::now_millis()
    }
}
