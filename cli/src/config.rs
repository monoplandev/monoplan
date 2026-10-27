//! Per-account on-disk config + secrets.
//!
//! `config.toml` holds operator-facing config (currently just
//! `server_url`). `secrets.toml` holds the device token and one DEK per
//! doc this account can open, in cleartext, and doubles as the "logged
//! in" marker. Account identity
//! (account/device/doc ids, email) and the sync cursor live in the
//! sqlite db (`crate::storage`), not here. When keychain-backed storage
//! lands, `secrets.toml` becomes a fallback for non-keychain hosts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use monoplan_core::{Dek, DocId};
use serde::{Deserialize, Serialize};

const ROOT_DIR: &str = "monoplan";
const CONFIG_FILE: &str = "config.toml";
const SECRETS_FILE: &str = "secrets.toml";
const DOC_DB_FILE: &str = "monoplan.sqlite";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("encode: {0}")]
    Encode(#[from] toml::ser::Error),
    #[error("decode: {0}")]
    Decode(#[from] toml::de::Error),
    #[error("no XDG data directory available")]
    NoDataDir,
    #[error("not logged in")]
    NotLoggedIn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Base URL of the server this install talks to. Bootstrap input
    /// (needed before any local db exists), so it lives in a file rather
    /// than the sqlite db.
    pub server_url: String,
}

/// Per-doc key material. One table per doc under `[docs."<uuid>"]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocSecrets {
    /// Hex-encoded DEK for this doc. Plain-file storage is the current
    /// stopgap; see module docs.
    pub dek_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Secrets {
    pub device_token: String,
    /// The DEK of every doc this account is a member of, keyed by doc
    /// id (uuid hex). The DEK is a doc key, not an account key: login
    /// hands back one wrap per membership and each lands here. Exactly
    /// one entry (the primary doc) until sharing exists, but nothing
    /// in the CLI may assume that.
    pub docs: BTreeMap<String, DocSecrets>,
}

impl Secrets {
    /// Build from the device token plus `(doc, dek)` pairs.
    pub fn new<'a>(device_token: String, docs: impl IntoIterator<Item = (DocId, &'a Dek)>) -> Self {
        Self {
            device_token,
            docs: docs
                .into_iter()
                .map(|(doc_id, dek)| {
                    (
                        doc_id.0.to_string(),
                        DocSecrets {
                            dek_hex: hex::encode(dek.as_bytes()),
                        },
                    )
                })
                .collect(),
        }
    }

    /// The stored DEK hex for `doc_id`, if this account holds one.
    pub fn doc(&self, doc_id: DocId) -> Option<&DocSecrets> {
        self.docs.get(&doc_id.0.to_string())
    }
}

/// Top-level on-disk handle. Path layout under the root dir (system
/// default `<data>/monoplan/`, or `MONOPLAN_DATA_DIR` if set):
/// ```text
///   <root>/config.toml    — server_url
///   <root>/secrets.toml   — device token + DEK ("logged in" marker)
///   <root>/monoplan.sqlite  — doc cache + account identity + sync cursor
/// ```
pub struct Profile {
    pub dir: PathBuf,
}

impl Profile {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The single per-install profile, if one is logged in. Monoplan is
    /// single-human-user (see product thesis); one account at a time,
    /// no profile switching, so "active" collapses to "do we hold device
    /// credentials?" — i.e. does `secrets.toml` exist. Run two accounts
    /// side-by-side in dev by pointing `MONOPLAN_DATA_DIR` at distinct
    /// roots.
    pub fn active() -> Result<Option<Self>, ConfigError> {
        let dir = root_dir()?;
        if !dir.join(SECRETS_FILE).exists() {
            return Ok(None);
        }
        Ok(Some(Self::new(dir)))
    }

    /// `active()` for an explicit directory: the embedded host
    /// (`monoplan-ffi`) is handed the profile path by the app rather
    /// than reading `MONOPLAN_DATA_DIR`.
    pub fn active_at(dir: PathBuf) -> Option<Self> {
        dir.join(SECRETS_FILE).exists().then(|| Self::new(dir))
    }

    pub fn require_active() -> Result<Self, ConfigError> {
        Self::active()?.ok_or(ConfigError::NotLoggedIn)
    }

    /// Create (or reuse) the single per-install profile directory.
    pub fn create() -> Result<Self, ConfigError> {
        Self::create_at(root_dir()?)
    }

    /// `create()` for an explicit directory (the app's sign-in).
    pub fn create_at(dir: PathBuf) -> Result<Self, ConfigError> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self::new(dir))
    }

    pub fn write_config(&self, cfg: &Config) -> Result<(), ConfigError> {
        write_toml(&self.dir.join(CONFIG_FILE), cfg, 0o600)
    }

    pub fn read_config(&self) -> Result<Config, ConfigError> {
        read_toml(&self.dir.join(CONFIG_FILE))
    }

    pub fn write_secrets(&self, secrets: &Secrets) -> Result<(), ConfigError> {
        write_toml(&self.dir.join(SECRETS_FILE), secrets, 0o600)
    }

    pub fn read_secrets(&self) -> Result<Secrets, ConfigError> {
        read_toml(&self.dir.join(SECRETS_FILE))
    }

    /// Path to the per-profile sqlite file. The doc itself is persisted
    /// through `crate::storage::SqliteStorage` (opened against this
    /// path); `Profile` only owns the TOML config/secrets files and the
    /// directory layout.
    pub fn doc_path(&self) -> PathBuf {
        self.dir.join(DOC_DB_FILE)
    }

    /// Wipe local state. Used by `monoplan auth logout`.
    pub fn purge(&self) -> Result<(), ConfigError> {
        if self.dir.exists() {
            std::fs::remove_dir_all(&self.dir)?;
        }
        Ok(())
    }
}

fn root_dir() -> Result<PathBuf, ConfigError> {
    // `MONOPLAN_DATA_DIR` lets tests (and adventurous users) override the
    // platform-default data dir without rebuilding. Taken verbatim:
    // the user picked the path, so we don't append `monoplan/` to it.
    // Production paths leave the env var unset, falling through to the
    // namespaced default below — `data_local_dir` is always absolute.
    let raw = match std::env::var("MONOPLAN_DATA_DIR") {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => dirs::data_local_dir()
            .ok_or(ConfigError::NoDataDir)?
            .join(ROOT_DIR),
    };
    // Resolve relative paths against CWD at first read so downstream
    // calls don't silently look in the wrong place if the user `cd`s
    // mid-session. `std::path::absolute` doesn't require the path to
    // exist (unlike `canonicalize`), which matches first-run UX.
    if raw.is_absolute() {
        Ok(raw)
    } else {
        Ok(std::path::absolute(&raw)?)
    }
}

fn write_toml<T: Serialize>(path: &Path, value: &T, _mode: u32) -> Result<(), ConfigError> {
    let text = toml::to_string_pretty(value)?;
    std::fs::write(path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(path)?.permissions();
        perm.set_mode(_mode);
        std::fs::set_permissions(path, perm)?;
    }
    Ok(())
}

fn read_toml<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, ConfigError> {
    let text = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}
