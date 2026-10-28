//! Random token generation + server-side hashing.
//!
//! Tokens are 32 random bytes presented to clients as 64-char lowercase
//! hex. The server stores `SHA-256(raw_bytes)`. SHA-256 is sufficient
//! because the input already has 256 bits of entropy — no rainbow-table
//! risk, and constant-time hash is fast enough at any traffic level we
//! expect here.

use rand::Rng;
use sha2::{Digest, Sha256};

pub const TOKEN_LEN: usize = 32;

/// 32 fresh random bytes.
pub fn generate_token() -> [u8; TOKEN_LEN] {
    let mut buf = [0u8; TOKEN_LEN];
    rand::rng().fill_bytes(&mut buf);
    buf
}

/// Hex-encode a token for client-side storage / transmission.
pub fn encode_token(token: &[u8; TOKEN_LEN]) -> String {
    hex::encode(token)
}

/// Decode a hex token back to bytes. Returns `None` on bad encoding or
/// wrong length — callers should treat both as "invalid token" without
/// distinguishing.
pub fn decode_token(s: &str) -> Option<[u8; TOKEN_LEN]> {
    let bytes = hex::decode(s.trim()).ok()?;
    bytes.try_into().ok()
}

/// SHA-256 of arbitrary bytes. Used for both `auth_secret` and tokens.
pub fn sha256(input: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(input);
    h.finalize().into()
}
