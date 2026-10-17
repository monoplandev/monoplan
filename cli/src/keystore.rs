//! Keystore = the union of `Profile.read_secrets()` + Argon2 derivation
//! at session start. This module is currently a thin facade over the
//! plain-file `Secrets` blob. It exists so the eventual keychain
//! integration drops into one place rather than threading through every
//! command.

use monoplan_core::{Dek, DocId, Kek, PasswordMaster};
use monoplan_protocol::KdfParams;

use crate::config::Secrets;

#[derive(Debug, thiserror::Error)]
pub enum KeystoreError {
    #[error(transparent)]
    Crypto(#[from] monoplan_core::CryptoError),
    #[error("invalid hex: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("no key stored for doc {0}; log in again to refresh doc keys")]
    MissingDocKey(uuid::Uuid),
}

/// The DEK for `doc_id` from the profile's secrets. Every doc-reading
/// path goes through here so a missing entry is one typed error rather
/// than a hex-decode failure three layers down.
pub fn dek_for_doc(secrets: &Secrets, doc_id: DocId) -> Result<Dek, KeystoreError> {
    let entry = secrets
        .doc(doc_id)
        .ok_or(KeystoreError::MissingDocKey(doc_id.0))?;
    dek_from_hex(&entry.dek_hex)
}

pub fn derive_master(
    password: &str,
    salt: &[u8],
    params: KdfParams,
) -> Result<PasswordMaster, KeystoreError> {
    Ok(monoplan_core::derive_password_master(
        password.as_bytes(),
        salt,
        params,
    )?)
}

pub fn dek_from_hex(hex_dek: &str) -> Result<Dek, KeystoreError> {
    let bytes = hex::decode(hex_dek)?;
    Ok(Dek::from_bytes(&bytes)?)
}

pub fn dek_to_hex(dek: &Dek) -> String {
    hex::encode(dek.as_bytes())
}

#[allow(dead_code)]
pub fn kek_from_master(master: &PasswordMaster) -> Result<Kek, KeystoreError> {
    Ok(master.kek()?)
}
