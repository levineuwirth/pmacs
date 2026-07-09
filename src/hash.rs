// hash.rs --- shared content hashing (Arc 3 phase 3, Q#AS9).

//! One `sha256_hex` for the whole crate. Three call sites want a stable,
//! filename-safe digest of a string:
//!
//! * [`crate::desktop`] — the desktop session key,
//! * [`crate::autosave`] — the recovery-file key (a hash of the path),
//! * `crate::packages::fetcher` — the package cache/mirror key.
//!
//! Each had grown (or was about to grow) its own private copy. A
//! *cryptographic* digest matters for the fetcher: a non-cryptographic
//! hash is trivially collidable, and a deliberate collision would make
//! two URLs share one bare mirror + lock file.
//!
//! Lowercase hex, so the output passes
//! [`crate::state::validate_name`]'s `[A-Za-z0-9._-]` charset.

use sha2::{Digest, Sha256};

/// Lowercase-hex SHA-256 of `s`.
pub(crate) fn sha256_hex(s: &str) -> String {
    use std::fmt::Write as _;
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector_and_charset() {
        // The canonical empty-string SHA-256.
        assert_eq!(
            sha256_hex(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let h = sha256_hex("/home/u/a.rs");
        assert_eq!(h.len(), 64);
        assert!(
            h.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        );
        // Filename-safe: passes the state-key charset.
        assert!(crate::state::validate_name(&format!("autosave/{h}")).is_ok());
    }

    #[test]
    fn distinct_inputs_distinct_digests() {
        assert_ne!(sha256_hex("a"), sha256_hex("b"));
    }
}
