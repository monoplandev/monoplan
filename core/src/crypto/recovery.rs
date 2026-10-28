//! BIP39 12-word recovery code (English wordlist, 128 bits + 4-bit checksum).

use bip39::{Language, Mnemonic};

use super::{CryptoError, Result};

/// A normalized 12-word phrase. `Display` emits the canonical
/// space-separated form. The string itself is what gets fed to Argon2id
/// at the recovery path (per `spec/encryption.md`).
#[derive(Debug, Clone)]
pub struct RecoveryCode(String);

impl RecoveryCode {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RecoveryCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub fn generate_recovery_code() -> Result<RecoveryCode> {
    let mut entropy = [0u8; 16];
    rand::Rng::fill_bytes(&mut rand::rng(), &mut entropy);
    let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy)
        .map_err(|e| CryptoError::InvalidRecoveryCode(e.to_string()))?;
    Ok(RecoveryCode(mnemonic.to_string()))
}

/// Validate + normalize a user-typed recovery phrase. Tolerates extra
/// whitespace and case variation.
pub fn parse_recovery_code(input: &str) -> Result<RecoveryCode> {
    let normalized = input
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    let mnemonic = Mnemonic::parse_in_normalized(Language::English, &normalized)
        .map_err(|e| CryptoError::InvalidRecoveryCode(e.to_string()))?;
    Ok(RecoveryCode(mnemonic.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_code_parses_back() {
        let code = generate_recovery_code().unwrap();
        let parsed = parse_recovery_code(code.as_str()).unwrap();
        assert_eq!(code.as_str(), parsed.as_str());
        assert_eq!(code.as_str().split_whitespace().count(), 12);
    }

    #[test]
    fn whitespace_tolerant() {
        let code = generate_recovery_code().unwrap();
        let messy = format!("  {}\n", code.as_str().replace(' ', "   "));
        let parsed = parse_recovery_code(&messy).unwrap();
        assert_eq!(code.as_str(), parsed.as_str());
    }

    #[test]
    fn case_insensitive() {
        let code = generate_recovery_code().unwrap();
        let parsed = parse_recovery_code(&code.as_str().to_uppercase()).unwrap();
        assert_eq!(code.as_str(), parsed.as_str());
    }

    #[test]
    fn rejects_invalid_word() {
        let bad = "this is not a valid bip39 phrase at all please reject";
        assert!(parse_recovery_code(bad).is_err());
    }
}
