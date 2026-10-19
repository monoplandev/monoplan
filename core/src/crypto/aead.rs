//! XChaCha20-Poly1305 AEAD wrap/unwrap.
//!
//! Used for three purposes (`spec/encryption.md`):
//! - Wrap the Home DEK with a KEK (one wrap per (account, password) and
//!   one per (account, recovery code)).
//! - Wrap any other doc's DEK with the Home DEK (one wrap per
//!   non-primary membership).
//! - Encrypt every op + snapshot blob with the DEK (per-blob fresh nonce).

use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit},
};

use super::{
    CryptoError, Result, random_bytes,
    secrets::{Dek, Kek},
};

/// 24 bytes — XChaCha20's extended nonce. Random-nonce safe up to ~2^96
/// uses per key, which we will not approach in practice.
pub const AEAD_NONCE_LEN: usize = 24;

/// A DEK ciphertext + the random nonce that produced it.
#[derive(Debug, Clone)]
pub struct WrappedDek {
    pub ciphertext: Vec<u8>,
    pub nonce: [u8; AEAD_NONCE_LEN],
}

impl Kek {
    /// Encrypt the DEK with this KEK. Generates a fresh random nonce.
    pub fn wrap(&self, dek: &Dek) -> Result<WrappedDek> {
        let cipher = XChaCha20Poly1305::new(self.as_bytes().into());
        let nonce: [u8; AEAD_NONCE_LEN] = random_bytes();
        let ciphertext = cipher
            .encrypt(XNonce::from_slice(&nonce), dek.as_bytes().as_slice())
            .map_err(|_| CryptoError::Aead)?;
        Ok(WrappedDek { ciphertext, nonce })
    }

    /// Decrypt a wrapped DEK with this KEK.
    pub fn unwrap(&self, w: &WrappedDek) -> Result<Dek> {
        if w.nonce.len() != AEAD_NONCE_LEN {
            return Err(CryptoError::InvalidNonceLength {
                expected: AEAD_NONCE_LEN,
                actual: w.nonce.len(),
            });
        }
        let cipher = XChaCha20Poly1305::new(self.as_bytes().into());
        let plaintext = cipher
            .decrypt(XNonce::from_slice(&w.nonce), w.ciphertext.as_slice())
            .map_err(|_| CryptoError::Aead)?;
        Dek::from_bytes(&plaintext)
    }
}

impl Dek {
    /// Encrypt an op / snapshot blob. Fresh random nonce per call.
    pub fn seal(&self, plaintext: &[u8]) -> Result<(Vec<u8>, [u8; AEAD_NONCE_LEN])> {
        let cipher = XChaCha20Poly1305::new(self.as_bytes().into());
        let nonce: [u8; AEAD_NONCE_LEN] = random_bytes();
        let ct = cipher
            .encrypt(XNonce::from_slice(&nonce), plaintext)
            .map_err(|_| CryptoError::Aead)?;
        Ok((ct, nonce))
    }

    pub fn open(&self, ciphertext: &[u8], nonce: &[u8]) -> Result<Vec<u8>> {
        if nonce.len() != AEAD_NONCE_LEN {
            return Err(CryptoError::InvalidNonceLength {
                expected: AEAD_NONCE_LEN,
                actual: nonce.len(),
            });
        }
        let cipher = XChaCha20Poly1305::new(self.as_bytes().into());
        cipher
            .decrypt(XNonce::from_slice(nonce), ciphertext)
            .map_err(|_| CryptoError::Aead)
    }

    /// Wrap another doc's DEK under this one. `self` is the Home DEK
    /// (the account key); the result goes on the other doc's
    /// `doc_members` row as `wrapped_dek` / `wrapped_dek_nonce`. Same
    /// AEAD and nonce discipline as `seal`, over the 32 key bytes.
    pub fn wrap_dek(&self, other: &Dek) -> Result<WrappedDek> {
        let (ciphertext, nonce) = self.seal(other.as_bytes())?;
        Ok(WrappedDek { ciphertext, nonce })
    }

    /// Inverse of `wrap_dek`: open a non-primary membership's wrap with
    /// the Home DEK. Wrong key or a KEK wrap handed in by mistake →
    /// `CryptoError::Aead`.
    pub fn unwrap_dek(&self, w: &WrappedDek) -> Result<Dek> {
        let plaintext = self.open(&w.ciphertext, &w.nonce)?;
        Dek::from_bytes(&plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_unwrap_roundtrip() {
        let kek = Kek::from_bytes(&[7u8; 32]).unwrap();
        let dek = Dek::generate();
        let wrapped = kek.wrap(&dek).unwrap();
        let recovered = kek.unwrap(&wrapped).unwrap();
        assert_eq!(dek.as_bytes(), recovered.as_bytes());
    }

    #[test]
    fn unwrap_with_wrong_kek_fails() {
        let kek = Kek::from_bytes(&[7u8; 32]).unwrap();
        let bad = Kek::from_bytes(&[8u8; 32]).unwrap();
        let dek = Dek::generate();
        let wrapped = kek.wrap(&dek).unwrap();
        assert!(matches!(bad.unwrap(&wrapped), Err(CryptoError::Aead)));
    }

    #[test]
    fn op_seal_open_roundtrip() {
        let dek = Dek::generate();
        let payload = b"hello world".to_vec();
        let (ct, nonce) = dek.seal(&payload).unwrap();
        assert_eq!(dek.open(&ct, &nonce).unwrap(), payload);
    }

    #[test]
    fn dek_wraps_another_dek_and_only_the_same_dek_opens_it() {
        let home = Dek::generate();
        let other = Dek::generate();
        let wrapped = home.wrap_dek(&other).unwrap();
        assert_eq!(wrapped.nonce.len(), AEAD_NONCE_LEN);
        let back = home.unwrap_dek(&wrapped).unwrap();
        assert_eq!(back.as_bytes(), other.as_bytes());

        let stranger = Dek::generate();
        assert!(matches!(
            stranger.unwrap_dek(&wrapped),
            Err(CryptoError::Aead)
        ));
        // A KEK wrap is not a Home-DEK wrap even when the raw bytes
        // match: the two key types never share material by accident.
        let kek = Kek::from_bytes(home.as_bytes()).unwrap();
        let kek_wrapped = kek.wrap(&other).unwrap();
        assert_eq!(
            home.unwrap_dek(&kek_wrapped).unwrap().as_bytes(),
            other.as_bytes(),
            "same bytes, same AEAD: the type split is a client-side discipline, not a crypto boundary"
        );
    }
}
