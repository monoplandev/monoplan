//! Keystore = the union of `Profile.read_secrets()` + Argon2 derivation
//! at session start. This module is currently a thin facade over the
//! plain-file `Secrets` blob. It exists so the eventual keychain
//! integration drops into one place rather than threading through every
//! command.
//!
//! It also owns the one piece of key-model logic every auth path shares:
//! turning a server `memberships` array into DEKs. The primary
//! membership's wrap is under the password (or recovery) KEK and yields
//! the **Home DEK**; every other membership's wrap is under the Home DEK
//! (`spec/encryption.md` §Wrap-states).

use monoplan_core::{AEAD_NONCE_LEN, Dek, DocId, Kek, PasswordMaster, WrappedDek};
use monoplan_protocol::{KdfParams, Membership};

use crate::config::Secrets;

#[derive(Debug, thiserror::Error)]
pub enum KeystoreError {
    #[error(transparent)]
    Crypto(#[from] monoplan_core::CryptoError),
    #[error("invalid hex: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("no key stored for doc {0}; log in again to refresh doc keys")]
    MissingDocKey(uuid::Uuid),
    #[error("server returned malformed doc id {0:?}")]
    MalformedDocId(String),
    #[error("server response has no membership for the primary doc {0}")]
    MissingPrimaryMembership(uuid::Uuid),
    #[error("server returned a wrap nonce of wrong length (expected {AEAD_NONCE_LEN} bytes)")]
    BadNonceLength,
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

pub fn parse_doc_id(hex: &str) -> Result<DocId, KeystoreError> {
    uuid::Uuid::parse_str(hex)
        .map(DocId)
        .map_err(|_| KeystoreError::MalformedDocId(hex.to_string()))
}

fn wrapped(wrapped_dek: &[u8], nonce: &[u8]) -> Result<WrappedDek, KeystoreError> {
    let nonce: [u8; AEAD_NONCE_LEN] = nonce
        .try_into()
        .map_err(|_| KeystoreError::BadNonceLength)?;
    Ok(WrappedDek {
        ciphertext: wrapped_dek.to_vec(),
        nonce,
    })
}

/// Open one server-held KEK wrap (password or recovery KEK). Only the
/// primary membership and the recovery `doc_keys` entry are KEK wraps.
pub fn unwrap_doc_key(kek: &Kek, wrapped_dek: &[u8], nonce: &[u8]) -> Result<Dek, KeystoreError> {
    Ok(kek.unwrap(&wrapped(wrapped_dek, nonce)?)?)
}

/// The Home DEK: the primary membership's wrap opened with the password
/// KEK. Fails if the server's memberships do not include the primary.
pub fn unwrap_home(
    kek: &Kek,
    primary_doc_id: DocId,
    memberships: &[Membership],
) -> Result<Dek, KeystoreError> {
    let primary = memberships
        .iter()
        .find(|m| parse_doc_id(&m.doc_id).is_ok_and(|id| id == primary_doc_id))
        .ok_or(KeystoreError::MissingPrimaryMembership(primary_doc_id.0))?;
    unwrap_doc_key(kek, &primary.wrapped_dek, &primary.wrapped_dek_nonce)
}

/// Every membership's `(doc_id, dek)`, in server order: the primary's
/// is `home` itself, every other wrap is opened with `home`. The same
/// function serves login (memberships from `/login`) and recovery
/// (memberships from `GET /api/docs` after the reset).
pub fn unwrap_memberships(
    home: &Dek,
    primary_doc_id: DocId,
    memberships: &[Membership],
) -> Result<Vec<(DocId, Dek)>, KeystoreError> {
    memberships
        .iter()
        .map(|m| {
            let doc_id = parse_doc_id(&m.doc_id)?;
            let dek = if doc_id == primary_doc_id {
                home.clone()
            } else {
                home.unwrap_dek(&wrapped(&m.wrapped_dek, &m.wrapped_dek_nonce)?)?
            };
            Ok((doc_id, dek))
        })
        .collect()
}
