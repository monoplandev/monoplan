//! Per-install on-disk config + secrets.
//!
//! `config.toml` holds operator-facing config (currently just
//! `server_url`); it exists once the install has an account. `secrets.toml`
//! holds one DEK per doc this install can open, in cleartext, plus the
//! device token once there is an account; its presence is the "profile
//! exists" marker and the token the "has an account" one. A profile
//! without a token is a local-only workspace (`crate::bootstrap`).
//! Identity (account/device/doc ids, email) and the sync cursor live in
//! the sqlite db (`crate::storage`), not here. When keychain-backed
//! storage lands, `secrets.toml` becomes a fallback for non-keychain
//! hosts.

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
    /// No profile at all: nothing has been run on this install yet.
    #[error("no local workspace yet; any item command creates one")]
    NoProfile,
    /// A local-only workspace: the command needs a server account.
    #[error(
        "no account on this install; `monoplan auth signup` creates one (the local workspace comes with it) or `monoplan auth login` joins an existing one"
    )]
    NoAccount,
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
    /// This device's server token. `None` for a local-only workspace,
    /// which is what makes it one: every other file is the same shape
    /// with or without an account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_token: Option<String>,
    /// The DEK of every doc this install can open, keyed by doc id
    /// (uuid hex). The DEK is a doc key, not an account key: login
    /// hands back one wrap per membership and each lands here; a
    /// local-only workspace holds exactly its one minted doc. Nothing
    /// in the CLI may assume a count.
    pub docs: BTreeMap<String, DocSecrets>,
}

impl Secrets {
    /// Build from the device token (none for a local-only workspace)
    /// plus `(doc, dek)` pairs.
    pub fn new<'a>(
        device_token: Option<String>,
        docs: impl IntoIterator<Item = (DocId, &'a Dek)>,
    ) -> Self {
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

    /// The stored DEK hex for `doc_id`, if this install holds one.
    pub fn doc(&self, doc_id: DocId) -> Option<&DocSecrets> {
        self.docs.get(&doc_id.0.to_string())
    }

    /// The device token, or the typed "no account" error for commands
    /// that need a server behind them.
    pub fn device_token(&self) -> Result<&str, ConfigError> {
        self.device_token.as_deref().ok_or(ConfigError::NoAccount)
    }

    pub fn has_account(&self) -> bool {
        self.device_token.is_some()
    }
}

/// Top-level on-disk handle. Path layout under the root dir (system
/// default `<data>/monoplan/`, or `MONOPLAN_DATA_DIR` if set):
/// ```text
///   <root>/config.toml    — server_url (once there is an account)
///   <root>/secrets.toml   — DEKs + device token ("profile exists" marker)
///   <root>/monoplan.sqlite  — doc cache + identity + sync cursor
/// ```
pub struct Profile {
    pub dir: PathBuf,
}

impl Profile {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The single per-install profile, if one exists: a local-only
    /// workspace or an account's device. Monoplan is single-human-user
    /// (see product thesis); one workspace at a time, no profile
    /// switching, so "active" collapses to "does `secrets.toml` exist".
    /// Run two side-by-side in dev by pointing `MONOPLAN_DATA_DIR` at
    /// distinct roots. Doc commands do not call this: they go through
    /// `crate::bootstrap::open_or_init`, which creates the local-only
    /// workspace on a fresh install.
    pub fn active() -> Result<Option<Self>, ConfigError> {
        let dir = root_dir()?;
        Ok(Self::active_at(dir))
    }

    /// `active()` for an explicit directory: the embedded host
    /// (`monoplan-ffi`) is handed the profile path by the app rather
    /// than reading `MONOPLAN_DATA_DIR`.
    pub fn active_at(dir: PathBuf) -> Option<Self> {
        dir.join(SECRETS_FILE).exists().then(|| Self::new(dir))
    }

    pub fn require_active() -> Result<Self, ConfigError> {
        Self::active()?.ok_or(ConfigError::NoProfile)
    }

    /// The profile's `MONOPLAN_DATA_DIR`-or-default root, whether or
    /// not anything is there yet.
    pub fn root() -> Result<PathBuf, ConfigError> {
        root_dir()
    }

    /// Does this profile have an account behind it, i.e. a device
    /// token in `secrets.toml`? False for a local-only workspace.
    pub fn has_account(&self) -> Result<bool, ConfigError> {
        Ok(self.read_secrets()?.has_account())
    }

    pub fn exists(&self) -> bool {
        self.dir.join(SECRETS_FILE).exists()
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
