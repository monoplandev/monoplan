//! Crypto primitives for the auth + E2EE layer.
//!
//! Layout maps onto `spec/encryption.md`:
//!
//! ```text
//!   password ──Argon2id──▶ PasswordMaster ──HKDF──▶ Kek          (wraps DEK; client-only)
//!                                          ──HKDF──▶ AuthSecret   (sent to server)
//!
//!   recovery code ──Argon2id──▶ RecoveryMaster ──HKDF──▶ Kek
//!                                                ──HKDF──▶ AuthSecret
//!
//!   Dek (random) ──XChaCha20Poly1305(Kek)──▶ WrappedDek          (the primary doc's DEK, i.e. the Home DEK)
//!   Dek (random) ──XChaCha20Poly1305(Home Dek)──▶ WrappedDek     (every other doc's DEK)
//! ```

mod aead;
mod kdf;
mod recovery;
mod secrets;

pub use aead::{AEAD_NONCE_LEN, WrappedDek};
pub use kdf::{
    AUTH_INFO, KEK_INFO, RECOVERY_AUTH_INFO, RECOVERY_KEK_INFO, derive_password_master,
    derive_recovery_master, kek_from_master, recovery_kek_from_master,
};
pub use recovery::{RecoveryCode, generate_recovery_code, parse_recovery_code};
pub use secrets::{AuthSecret, Dek, Kek, PasswordMaster, RecoveryMaster};

/// Convenience re-export so callers don't need an explicit dep on
/// `monoplan-protocol` for the one struct that crosses the boundary.
pub use monoplan_protocol::KdfParams;

use rand::RngCore;

/// Generate `N` cryptographically-random bytes.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    rand::thread_rng().fill_bytes(&mut out);
    out
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("argon2: {0}")]
    Argon2(String),
    #[error("hkdf: invalid output length")]
    Hkdf,
    #[error("aead: encrypt/decrypt failed (wrong key, corrupted ciphertext, or bad nonce)")]
    Aead,
    #[error("invalid recovery code: {0}")]
    InvalidRecoveryCode(String),
    #[error("invalid key length: expected {expected}, got {actual}")]
    InvalidKeyLength { expected: usize, actual: usize },
    #[error("invalid nonce length: expected {expected}, got {actual}")]
    InvalidNonceLength { expected: usize, actual: usize },
}

pub type Result<T> = std::result::Result<T, CryptoError>;
