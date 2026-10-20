//! KDF: Argon2id master derivation + HKDF subkey expansion.

use argon2::{Algorithm, Argon2, Params, Version};
use hkdf::Hkdf;
use sha2::Sha256;

use super::{
    CryptoError, KdfParams, Result,
    secrets::{AuthSecret, Kek, PasswordMaster, RecoveryMaster},
};

/// HKDF info strings. Versioned so we can rotate the derivation without
/// breaking older accounts (bump the suffix, keep the old path for
/// migration).
pub const KEK_INFO: &[u8] = b"monoplan/kek/v1";
pub const AUTH_INFO: &[u8] = b"monoplan/auth/v1";
pub const RECOVERY_KEK_INFO: &[u8] = b"monoplan/recovery_kek/v1";
pub const RECOVERY_AUTH_INFO: &[u8] = b"monoplan/recovery_auth/v1";

const MASTER_LEN: usize = 32;

/// Argon2id over the password. Produces 32 bytes of master key material.
pub fn derive_password_master(
    password: &[u8],
    salt: &[u8],
    params: KdfParams,
) -> Result<PasswordMaster> {
    let mut out = [0u8; MASTER_LEN];
    argon2id(password, salt, params, &mut out)?;
    Ok(PasswordMaster(out))
}

/// Argon2id over the recovery code (the BIP39 phrase). Same parameter
/// set as the password path; the recovery code's high entropy makes the
/// cost less critical, but symmetric params keep the implementation
/// boring.
pub fn derive_recovery_master(
    recovery_code: &str,
    salt: &[u8],
    params: KdfParams,
) -> Result<RecoveryMaster> {
    let mut out = [0u8; MASTER_LEN];
    argon2id(recovery_code.as_bytes(), salt, params, &mut out)?;
    Ok(RecoveryMaster(out))
}

/// HKDF-SHA256 expansion of a 32-byte master into a 32-byte subkey.
pub(super) fn hkdf_expand(master: &[u8; 32], info: &[u8]) -> Result<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(None, master);
    let mut out = [0u8; 32];
    hk.expand(info, &mut out).map_err(|_| CryptoError::Hkdf)?;
    Ok(out)
}

pub fn kek_from_master(master: &PasswordMaster) -> Result<Kek> {
    Ok(Kek(hkdf_expand(&master.0, KEK_INFO)?))
}

pub fn auth_secret_from_master(master: &PasswordMaster) -> Result<AuthSecret> {
    Ok(AuthSecret(hkdf_expand(&master.0, AUTH_INFO)?))
}

pub fn recovery_kek_from_master(master: &RecoveryMaster) -> Result<Kek> {
    Ok(Kek(hkdf_expand(&master.0, RECOVERY_KEK_INFO)?))
}

pub fn recovery_auth_secret_from_master(master: &RecoveryMaster) -> Result<AuthSecret> {
    Ok(AuthSecret(hkdf_expand(&master.0, RECOVERY_AUTH_INFO)?))
}

impl PasswordMaster {
    pub fn kek(&self) -> Result<Kek> {
        kek_from_master(self)
    }
    pub fn auth_secret(&self) -> Result<AuthSecret> {
        auth_secret_from_master(self)
    }
}

impl RecoveryMaster {
    pub fn kek(&self) -> Result<Kek> {
        recovery_kek_from_master(self)
    }
    pub fn auth_secret(&self) -> Result<AuthSecret> {
        recovery_auth_secret_from_master(self)
    }
}

fn argon2id(input: &[u8], salt: &[u8], params: KdfParams, out: &mut [u8]) -> Result<()> {
    let p = Params::new(params.m_kib, params.t, params.p, Some(out.len()))
        .map_err(|e| CryptoError::Argon2(e.to_string()))?;
    Argon2::new(Algorithm::Argon2id, Version::V0x13, p)
        .hash_password_into(input, salt, out)
        .map_err(|e| CryptoError::Argon2(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only weak params — real defaults take ~150ms which is too
    /// slow for unit tests.
    fn weak() -> KdfParams {
        KdfParams {
            m_kib: 8,
            t: 1,
            p: 1,
        }
    }

    #[test]
    fn argon2_is_deterministic_per_salt() {
        let salt = [42u8; 16];
        let a = derive_password_master(b"password", &salt, weak()).unwrap();
        let b = derive_password_master(b"password", &salt, weak()).unwrap();
        assert_eq!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn hkdf_subkeys_diverge() {
        let salt = [1u8; 16];
        let m = derive_password_master(b"password", &salt, weak()).unwrap();
        let kek = m.kek().unwrap();
        let auth = m.auth_secret().unwrap();
        assert_ne!(kek.as_bytes(), auth.as_bytes());
    }
}
