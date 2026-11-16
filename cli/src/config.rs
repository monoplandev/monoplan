//! Per-install on-disk config + secrets.
//!
//! `config.toml` holds operator-facing config (currently just
//! `server_url`); it exists once the install has an account. The
//! secrets are one DEK per doc this install can open plus the device
//! token once there is an account; a profile without a token is a
//! local-only workspace (`crate::bootstrap`). Identity (account/device/
//! doc ids, email) and the sync cursor live in the sqlite db
//! (`crate::storage`), not here.
//!
//! Where the secrets live depends on the host (`spec/cli.md` §Local
//! state):
//!
//! - **File** (the CLI, tests, an Apple build without the keychain
//!   entitlement): `secrets.toml`, cleartext, mode 0600. Its presence is
//!   the "profile exists" marker.
//! - **Keychain** (the Apple apps, after `use_keychain`): one keychain
//!   item holding the same TOML, this device only. The profile dir then
//!   holds `keychain.toml` instead, with nothing secret in it: the
//!   item's id and whether there is an account. That file is the
//!   "profile exists" marker, so a copy of the profile dir (a backup, a
//!   device migration) carries ciphertext and no key. A `secrets.toml`
//!   found under this store is moved into the keychain on first read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use monoplan_core::{Dek, DocId};
use serde::{Deserialize, Serialize};

const ROOT_DIR: &str = "monoplan";
const CONFIG_FILE: &str = "config.toml";
const SECRETS_FILE: &str = "secrets.toml";
const KEYCHAIN_FILE: &str = "keychain.toml";
const DB_FILE: &str = "monoplan.sqlite";

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
    #[error("keychain: {0}")]
    Keychain(String),
    /// `keychain.toml` without `secrets.toml`, in a process that has no
    /// keychain: the keys were stored by an entitled build.
    #[error(
        "this profile's keys are in the keychain, which this build cannot read (it is not signed with the keychain entitlement)"
    )]
    KeychainUnavailable,
    /// `keychain.toml` names an item this device's keychain does not
    /// hold: the profile dir was copied here without its keys.
    #[error("this profile's keys are not in this device's keychain")]
    KeysMissing,
}

/// A secret store outside the profile dir: the OS keychain, or a fake
/// in tests. Holds one opaque blob per profile, under an id the
/// profile's `keychain.toml` names.
pub trait Vault: Send + Sync {
    /// `None` only when the store answered and holds no such item; a
    /// store that cannot answer (locked before first unlock, no
    /// entitlement) is an error, never `None`: callers discard a
    /// profile on `None`.
    fn get(&self, id: &str) -> Result<Option<Vec<u8>>, ConfigError>;
    fn set(&self, id: &str, secret: &[u8]) -> Result<(), ConfigError>;
    /// Deleting an absent item is not an error.
    fn delete(&self, id: &str) -> Result<(), ConfigError>;
}

static KEYCHAIN: OnceLock<Arc<dyn Vault>> = OnceLock::new();

/// Keep every profile's secrets in the OS keychain from here on, for
/// the rest of the process. An app calls this once, before it touches
/// its profile; the CLI never does. False, and nothing changes, when
/// the keychain is not usable: off Apple platforms, or in a build
/// without an application identifier to scope the items by (a Mac
/// build signed ad-hoc, `MONOPLAN_SIGN=adhoc`; a test binary).
pub fn use_keychain() -> bool {
    if KEYCHAIN.get().is_some() {
        return true;
    }
    #[cfg(target_vendor = "apple")]
    if crate::keychain::available() {
        let _ = KEYCHAIN.set(Arc::new(crate::keychain::Keychain));
        return true;
    }
    false
}

/// What `keychain.toml` holds. Nothing secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct KeychainMarker {
    /// The keychain item's account name: a random id minted with the
    /// profile, so the item is tied to this copy of the dir rather
    /// than to its path.
    id: String,
    /// Mirrors `Secrets::has_account`, for readers that must not or
    /// cannot ask the keychain (the apps' `ProfileLocation`).
    has_account: bool,
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
///   <root>/secrets.toml   — DEKs + device token (file store)
///   <root>/keychain.toml  — keychain item id (keychain store)
///   <root>/monoplan.sqlite — all docs (wal + snapshots) + identity + sync cursors
/// ```
/// Exactly one of `secrets.toml` / `keychain.toml` marks the profile
/// as existing (module docs).
#[derive(Clone)]
pub struct Profile {
    pub dir: PathBuf,
    /// Where the secrets live: `None` is `secrets.toml`.
    vault: Option<Arc<dyn Vault>>,
}

impl Profile {
    /// On the process's secret store: the keychain after
    /// `use_keychain`, else the file.
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            vault: KEYCHAIN.get().cloned(),
        }
    }

    /// On an explicit vault (tests).
    pub fn with_vault(dir: PathBuf, vault: Arc<dyn Vault>) -> Self {
        Self {
            dir,
            vault: Some(vault),
        }
    }

    /// The single per-install profile, if one exists: a local-only
    /// workspace or an account's device. Monoplan is single-human-user
    /// (see product thesis); one workspace at a time, no profile
    /// switching, so "active" collapses to "does its marker file exist".
    /// Run two side-by-side in dev by pointing `MONOPLAN_DATA_DIR` at
    /// distinct roots. Doc commands do not call this: they go through
    /// `crate::bootstrap::open_or_init`, which creates the local-only
    /// workspace on a fresh install.
    pub fn active() -> Result<Option<Self>, ConfigError> {
        let dir = root_dir()?;
        Ok(Self::active_at(dir))
    }

    /// `active()` for an explicit directory: an app's host
    /// (`monoplan-ffi`) is handed the profile path by the app rather
    /// than reading `MONOPLAN_DATA_DIR`.
    pub fn active_at(dir: PathBuf) -> Option<Self> {
        let profile = Self::new(dir);
        profile.exists().then_some(profile)
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
    /// token in its secrets? False for a local-only workspace.
    pub fn has_account(&self) -> Result<bool, ConfigError> {
        Ok(self.read_secrets()?.has_account())
    }

    pub fn exists(&self) -> bool {
        self.dir.join(SECRETS_FILE).exists() || self.dir.join(KEYCHAIN_FILE).exists()
    }

    /// The profile's keys are gone for good: it is marked as
    /// keychain-backed and the keychain, asked, holds no such item.
    /// That is a profile dir restored or migrated onto another device
    /// (the item never leaves the one it was written on). Nothing in
    /// the dir can be decrypted, so `bootstrap::open_or_init_at`
    /// discards it. False whenever the keychain cannot answer.
    pub fn orphaned(&self) -> bool {
        let Some(vault) = &self.vault else {
            return false;
        };
        if self.dir.join(SECRETS_FILE).exists() {
            return false;
        }
        match self.read_marker() {
            Ok(marker) => matches!(vault.get(&marker.id), Ok(None)),
            Err(_) => false,
        }
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
        let file = self.dir.join(SECRETS_FILE);
        match &self.vault {
            None => {
                write_toml(&file, secrets, 0o600)?;
                // A marker left by a keychain build no longer describes
                // this profile.
                remove_if_present(&self.dir.join(KEYCHAIN_FILE))
            }
            Some(vault) => {
                self.write_keychain(vault.as_ref(), secrets)?;
                remove_if_present(&file)
            }
        }
    }

    pub fn read_secrets(&self) -> Result<Secrets, ConfigError> {
        let file = self.dir.join(SECRETS_FILE);
        let Some(vault) = &self.vault else {
            if !file.exists() && self.dir.join(KEYCHAIN_FILE).exists() {
                return Err(ConfigError::KeychainUnavailable);
            }
            return read_toml(&file);
        };
        if file.exists() {
            // A profile written by the file store (an earlier build):
            // move it. The file goes last, so a crash in between
            // leaves it to be moved again.
            let secrets: Secrets = read_toml(&file)?;
            self.write_keychain(vault.as_ref(), &secrets)?;
            std::fs::remove_file(&file)?;
            return Ok(secrets);
        }
        let marker = self.read_marker()?;
        let bytes = vault.get(&marker.id)?.ok_or(ConfigError::KeysMissing)?;
        let text = String::from_utf8(bytes)
            .map_err(|_| ConfigError::Keychain("stored secrets are not UTF-8".into()))?;
        Ok(toml::from_str(&text)?)
    }

    /// The keychain item, then the marker that names it: the marker is
    /// the "profile exists" signal, so it never points at an item not
    /// yet written.
    fn write_keychain(&self, vault: &dyn Vault, secrets: &Secrets) -> Result<(), ConfigError> {
        let id = match self.read_marker() {
            Ok(marker) => marker.id,
            Err(_) => uuid::Uuid::new_v4().to_string(),
        };
        vault.set(&id, toml::to_string_pretty(secrets)?.as_bytes())?;
        write_toml(
            &self.dir.join(KEYCHAIN_FILE),
            &KeychainMarker {
                id,
                has_account: secrets.has_account(),
            },
            0o600,
        )
    }

    fn read_marker(&self) -> Result<KeychainMarker, ConfigError> {
        read_toml(&self.dir.join(KEYCHAIN_FILE))
    }

    /// Path to the per-profile sqlite file. The doc itself is persisted
    /// through `crate::storage::SqliteStorage` (opened against this
    /// path); `Profile` only owns the TOML config/secrets files and the
    /// directory layout.
    pub fn db_path(&self) -> PathBuf {
        self.dir.join(DB_FILE)
    }

    /// Wipe local state, the keychain item included. Used by
    /// `monoplan auth logout`. A build without the keychain cannot
    /// reach an item an entitled build left; without its marker that
    /// item is never read again.
    pub fn purge(&self) -> Result<(), ConfigError> {
        if let (Some(vault), Ok(marker)) = (&self.vault, self.read_marker()) {
            vault.delete(&marker.id)?;
        }
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

fn remove_if_present(path: &Path) -> Result<(), ConfigError> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

fn read_toml<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, ConfigError> {
    let text = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// The keychain, in memory.
    #[derive(Default)]
    struct FakeVault(Mutex<HashMap<String, Vec<u8>>>);

    impl Vault for FakeVault {
        fn get(&self, id: &str) -> Result<Option<Vec<u8>>, ConfigError> {
            Ok(self.0.lock().unwrap().get(id).cloned())
        }
        fn set(&self, id: &str, secret: &[u8]) -> Result<(), ConfigError> {
            self.0.lock().unwrap().insert(id.into(), secret.to_vec());
            Ok(())
        }
        fn delete(&self, id: &str) -> Result<(), ConfigError> {
            self.0.lock().unwrap().remove(id);
            Ok(())
        }
    }

    /// A keychain that cannot answer: locked before the first unlock.
    struct LockedVault;

    impl Vault for LockedVault {
        fn get(&self, _: &str) -> Result<Option<Vec<u8>>, ConfigError> {
            Err(ConfigError::Keychain("locked".into()))
        }
        fn set(&self, _: &str, _: &[u8]) -> Result<(), ConfigError> {
            Err(ConfigError::Keychain("locked".into()))
        }
        fn delete(&self, _: &str) -> Result<(), ConfigError> {
            Err(ConfigError::Keychain("locked".into()))
        }
    }

    fn secrets(token: Option<&str>) -> Secrets {
        let dek = Dek::generate();
        Secrets::new(
            token.map(String::from),
            [(DocId(uuid::Uuid::new_v4()), &dek)],
        )
    }

    fn on_disk(dir: &Path) -> String {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
            .collect()
    }

    #[test]
    fn keychain_store_keeps_secrets_out_of_the_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = Arc::new(FakeVault::default());
        let profile = Profile::with_vault(tmp.path().into(), vault.clone());
        assert!(!profile.exists());

        let written = secrets(Some("tok-secret"));
        profile.write_secrets(&written).unwrap();
        assert!(profile.exists());
        assert!(!tmp.path().join(SECRETS_FILE).exists());
        let read = profile.read_secrets().unwrap();
        assert_eq!(read.device_token, written.device_token);
        assert_eq!(read.docs.len(), 1);

        let disk = on_disk(tmp.path());
        assert!(!disk.contains("tok-secret"));
        assert!(!disk.contains(&written.docs.values().next().unwrap().dek_hex));
        assert!(disk.contains("has_account = true"));

        // A rewrite keeps the one item.
        profile.write_secrets(&secrets(None)).unwrap();
        assert_eq!(vault.0.lock().unwrap().len(), 1);
        assert!(!profile.has_account().unwrap());
        assert!(on_disk(tmp.path()).contains("has_account = false"));

        profile.purge().unwrap();
        assert!(vault.0.lock().unwrap().is_empty());
        assert!(!tmp.path().exists());
    }

    #[test]
    fn file_profile_moves_into_the_keychain_on_first_read() {
        let tmp = tempfile::tempdir().unwrap();
        let written = secrets(Some("tok"));
        Profile {
            dir: tmp.path().into(),
            vault: None,
        }
        .write_secrets(&written)
        .unwrap();

        let vault = Arc::new(FakeVault::default());
        let profile = Profile::with_vault(tmp.path().into(), vault.clone());
        assert!(profile.exists());
        assert!(!profile.orphaned());
        assert_eq!(
            profile.read_secrets().unwrap().device_token,
            written.device_token
        );
        assert!(!tmp.path().join(SECRETS_FILE).exists());
        assert_eq!(vault.0.lock().unwrap().len(), 1);
        // And from the keychain alone the second time.
        assert_eq!(profile.read_secrets().unwrap().docs.len(), 1);
    }

    #[test]
    fn dir_without_its_keys_is_orphaned_only_when_the_keychain_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let home = Profile::with_vault(tmp.path().into(), Arc::new(FakeVault::default()));
        home.write_secrets(&secrets(Some("tok"))).unwrap();
        assert!(!home.orphaned());

        // The same dir on a device whose keychain never held the item.
        let moved = Profile::with_vault(tmp.path().into(), Arc::new(FakeVault::default()));
        assert!(moved.exists());
        assert!(moved.orphaned());
        assert!(matches!(
            moved.read_secrets(),
            Err(ConfigError::KeysMissing)
        ));

        // A keychain that cannot answer proves nothing.
        let locked = Profile::with_vault(tmp.path().into(), Arc::new(LockedVault));
        assert!(!locked.orphaned());
        assert!(matches!(
            locked.read_secrets(),
            Err(ConfigError::Keychain(_))
        ));

        // A build with no keychain says why, and keeps the profile.
        let file = Profile {
            dir: tmp.path().into(),
            vault: None,
        };
        assert!(file.exists());
        assert!(!file.orphaned());
        assert!(matches!(
            file.read_secrets(),
            Err(ConfigError::KeychainUnavailable)
        ));
    }
}
