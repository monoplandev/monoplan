//! Email normalisation. Email is purely an account identifier (it never
//! feeds the KDF or any wrap), so the server owns its canonical form and
//! every credential endpoint runs through here before touching the db.
//!
//! Canonical form: surrounding whitespace trimmed, whole address
//! lowercased. RFC 5321 technically allows a case-sensitive local part,
//! but no real provider honours it, and treating `Dan@x` and `dan@x` as
//! different accounts only ever produces a confusing "unknown email" on
//! the second device. Provider-specific rewrites (Gmail dot / plus-tag
//! stripping) are deliberately not applied.

/// Returns the canonical form, or `None` if nothing is left after
/// trimming.
pub fn normalize_email(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::normalize_email;

    #[test]
    fn trims_and_lowercases() {
        assert_eq!(
            normalize_email("  Dan.Gormly@Example.COM \n").as_deref(),
            Some("dan.gormly@example.com")
        );
    }

    #[test]
    fn blank_is_none() {
        assert_eq!(normalize_email(""), None);
        assert_eq!(normalize_email("   \t"), None);
    }

    #[test]
    fn leaves_dots_and_plus_tags_alone() {
        assert_eq!(
            normalize_email("d.an+tag@gmail.com").as_deref(),
            Some("d.an+tag@gmail.com")
        );
    }
}
