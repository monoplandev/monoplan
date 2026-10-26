//! Server runtime config.
//!
//! Precedence: defaults → `config.toml` → env vars → CLI flags. The
//! file is optional; if it is missing the server runs on defaults.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::sync::snapshot::SNAPSHOT_THRESHOLD_BLOBS;

const DEFAULT_CONFIG_PATH: &str = "local/server.toml";

/// How `Config::load` resolved the config path. The caller is expected
/// to log this *after* the tracing subscriber is initialized — we can't
/// log it from `load` itself because `log_level` comes from the config.
#[derive(Debug, Clone)]
pub enum ConfigSource {
    File(PathBuf),
    Defaults { tried: PathBuf },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_db")]
    pub db: PathBuf,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    /// Set the `Secure` attribute on the device-auth cookie. Default
    /// `true` (production behind TLS); flip to `false` for plain-HTTP
    /// dev because Safari refuses to store/return `Secure` cookies on
    /// `http://localhost` (Chromium/Firefox treat localhost as secure
    /// and don't care).
    #[serde(default = "default_secure_cookies")]
    pub secure_cookies: bool,
    /// Encrypted update blobs accumulated since the last snapshot before
    /// the server starts asking clients to snapshot. One blob = one
    /// `PushUpdates` push (see `spec/sync-protocol.md` §"Terminology"), so
    /// this counts pushes, not user actions. Lower in tests to exercise
    /// the path without churning out hundreds of pushes.
    #[serde(default = "default_snapshot_threshold_blobs")]
    pub snapshot_threshold_blobs: u64,
    /// Optional Argon2id PHC hash protecting the operator-only JSON API.
    /// When absent, admin routes are not mounted.
    #[serde(default)]
    pub admin_password_hash: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            db: default_db(),
            log_level: default_log_level(),
            secure_cookies: default_secure_cookies(),
            snapshot_threshold_blobs: default_snapshot_threshold_blobs(),
            admin_password_hash: None,
        }
    }
}

fn default_bind() -> String {
    "127.0.0.1:8000".to_string()
}

fn default_db() -> PathBuf {
    // XDG Base Directory spec: $XDG_DATA_HOME, falling back to
    // $HOME/.local/share. We don't follow OS-native conventions
    // (`~/Library/Application Support` on macOS) on purpose — keeps the
    // dev experience uniform across platforms and matches the rest of
    // the Rust toolchain (cargo, rustup) behaviour.
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("monoplan/monoplan.sqlite");
        }
    }
    if let Some(home) = dirs::home_dir() {
        return home.join(".local/share/monoplan/monoplan.sqlite");
    }
    PathBuf::from("monoplan.sqlite")
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_secure_cookies() -> bool {
    true
}

fn default_snapshot_threshold_blobs() -> u64 {
    SNAPSHOT_THRESHOLD_BLOBS
}

impl Config {
    /// Read a config from disk. Missing file → defaults; parse error → panic
    /// (a malformed config is operator error, not something to silently paper over).
    /// Returns the source so the caller can log it once tracing is up.
    pub fn load(path: Option<&Path>) -> (Self, ConfigSource) {
        let path = path
            .map(PathBuf::from)
            .or_else(|| std::env::var("MONOPLAN_CONFIG").ok().map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH));

        let (mut config, source) = match std::fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(c) => (c, ConfigSource::File(path)),
                Err(err) => panic!("failed to parse {}: {err}", path.display()),
            },
            Err(_) => (Config::default(), ConfigSource::Defaults { tried: path }),
        };

        if let Ok(v) = std::env::var("MONOPLAN_BIND") {
            config.bind = v;
        }
        if let Ok(v) = std::env::var("MONOPLAN_DB_PATH") {
            config.db = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("MONOPLAN_LOG_LEVEL") {
            config.log_level = v;
        }
        if let Ok(v) = std::env::var("MONOPLAN_SECURE_COOKIES") {
            config.secure_cookies = matches!(v.as_str(), "1" | "true" | "TRUE");
        }
        if let Ok(v) = std::env::var("MONOPLAN_SNAPSHOT_THRESHOLD_BLOBS") {
            config.snapshot_threshold_blobs = v
                .parse()
                .unwrap_or_else(|e| panic!("invalid MONOPLAN_SNAPSHOT_THRESHOLD_BLOBS={v:?}: {e}"));
        }
        if let Ok(v) = std::env::var("MONOPLAN_ADMIN_PASSWORD_HASH") {
            config.admin_password_hash = (!v.is_empty()).then_some(v);
        }

        (config, source)
    }

    pub fn bind_addr(&self) -> anyhow::Result<SocketAddr> {
        self.bind
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid bind address {:?}: {e}", self.bind))
    }
}

impl ConfigSource {
    pub fn log(&self) {
        match self {
            ConfigSource::File(path) => tracing::info!(path = %path.display(), "loaded config"),
            ConfigSource::Defaults { tried } => {
                tracing::info!(tried = %tried.display(), "no config file, using defaults")
            }
        }
    }
}
