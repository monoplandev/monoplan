//! Strongly-typed key material.
//!
//! Each type is a thin newtype around a 32-byte secret. They are *not*
//! `Serialize` — wire types live in `monoplan-protocol` and convert via
//! `as_bytes()`. Drop zeroes the bytes.

use zeroize::{Zeroize, ZeroizeOnDrop};

use super::{CryptoError, Result};

pub(crate) const KEY_LEN: usize = 32;

macro_rules! key32 {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Zeroize, ZeroizeOnDrop)]
        pub struct $name(pub(crate) [u8; KEY_LEN]);

        impl $name {
            /// Build from raw bytes. Used when reconstituting from the
            /// wire or local persistence.
            pub fn from_bytes(b: &[u8]) -> Result<Self> {
                if b.len() != KEY_LEN {
                    return Err(CryptoError::InvalidKeyLength {
                        expected: KEY_LEN,
                        actual: b.len(),
                    });
                }
                let mut k = [0u8; KEY_LEN];
                k.copy_from_slice(b);
                Ok(Self(k))
            }

            pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
                &self.0
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_tuple(stringify!($name))
                    .field(&"<redacted>")
                    .finish()
            }
        }
    };
}

key32!(
    PasswordMaster,
    "32-byte output of Argon2id(password, master_salt). Never leaves the client."
);
key32!(
    RecoveryMaster,
    "32-byte output of Argon2id(recovery_code, recovery_salt). Never leaves the client."
);
key32!(
    Kek,
    "Key-encryption key. Wraps/unwraps the DEK. Never leaves the client."
);
key32!(
    AuthSecret,
    "Login credential sent to the server. The server stores SHA-256(self)."
);
key32!(
    Dek,
    "Data-encryption key. Encrypts every op + snapshot blob."
);

impl Dek {
    /// Generate a fresh random DEK. Called once at signup.
    pub fn generate() -> Self {
        Self(super::random_bytes::<KEY_LEN>())
    }
}
