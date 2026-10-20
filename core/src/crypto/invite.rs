//! Invite secrets: the one-time sharing link (`spec/sharing-plan.md`
//! §Invites).
//!
//! An invite is a single 32-byte random secret that travels only in a
//! URL fragment. Two HKDF subkeys are derived from it:
//!
//! - the **invite id**, which the client sends to the server to create,
//!   fetch and accept the invite (the server stores the id, never the
//!   secret, and cannot invert it);
//! - the **invite KEK**, under which the inviter wraps the doc's DEK.
//!
//! The server therefore holds a wrap it cannot open and an id that
//! reveals nothing about the wrap key. Anyone holding the URL holds
//! both, which is the bearer-credential nature of the link; the server
//! bounds it with expiry and single use.

use zeroize::{Zeroize, ZeroizeOnDrop};

use super::{
    Result, WrappedDek,
    kdf::hkdf_expand,
    random_bytes,
    secrets::{Dek, Kek},
};

pub const INVITE_SECRET_LEN: usize = 32;
pub const INVITE_ID_LEN: usize = 32;

pub const INVITE_KEK_INFO: &[u8] = b"monoplan/invite_kek/v1";
pub const INVITE_ID_INFO: &[u8] = b"monoplan/invite_id/v1";

/// The fragment secret of a sharing link. Never sent to the server.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct InviteSecret([u8; INVITE_SECRET_LEN]);

impl std::fmt::Debug for InviteSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("InviteSecret").field(&"<redacted>").finish()
    }
}

impl InviteSecret {
    /// Fresh random secret for a new link.
    pub fn generate() -> Self {
        Self(random_bytes())
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        if b.len() != INVITE_SECRET_LEN {
            return Err(super::CryptoError::InvalidKeyLength {
                expected: INVITE_SECRET_LEN,
                actual: b.len(),
            });
        }
        let mut k = [0u8; INVITE_SECRET_LEN];
        k.copy_from_slice(b);
        Ok(Self(k))
    }

    pub fn as_bytes(&self) -> &[u8; INVITE_SECRET_LEN] {
        &self.0
    }

    /// The server-side lookup id. One-way: the server cannot recover the
    /// secret or the KEK from it.
    pub fn id(&self) -> Result<[u8; INVITE_ID_LEN]> {
        hkdf_expand(&self.0, INVITE_ID_INFO)
    }

    /// The key the doc's DEK is wrapped under for this invite.
    pub fn kek(&self) -> Result<Kek> {
        Ok(Kek(hkdf_expand(&self.0, INVITE_KEK_INFO)?))
    }

    /// Inviter side: wrap the doc's DEK for the link.
    pub fn wrap(&self, dek: &Dek) -> Result<WrappedDek> {
        self.kek()?.wrap(dek)
    }

    /// Invitee side: open the wrap the server handed back for this id.
    pub fn unwrap(&self, w: &WrappedDek) -> Result<Dek> {
        self.kek()?.unwrap(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_and_kek_are_distinct_and_deterministic() {
        let s = InviteSecret::generate();
        let id1 = s.id().unwrap();
        let id2 = s.id().unwrap();
        assert_eq!(id1, id2);
        assert_ne!(&id1, s.kek().unwrap().as_bytes());
        assert_ne!(&id1, s.as_bytes());
    }

    #[test]
    fn wrap_roundtrips_and_a_different_secret_fails() {
        let s = InviteSecret::generate();
        let dek = Dek::generate();
        let w = s.wrap(&dek).unwrap();
        assert_eq!(s.unwrap(&w).unwrap().as_bytes(), dek.as_bytes());
        let other = InviteSecret::generate();
        assert!(other.unwrap(&w).is_err());
    }

    #[test]
    fn from_bytes_rejects_wrong_length() {
        assert!(InviteSecret::from_bytes(&[1u8; 31]).is_err());
        let s = InviteSecret::from_bytes(&[1u8; 32]).unwrap();
        assert_eq!(s.as_bytes(), &[1u8; 32]);
    }
}
