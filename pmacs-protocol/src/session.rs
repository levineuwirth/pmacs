//! Session identity (E9, protocol v27): the token a daemon mints for a
//! semantic session once its handshake has settled, and that the
//! frontend presents to resume the session's view on a later socket.
//!
//! # Why the token is minted after negotiation, never in `Hello`
//!
//! `Hello` is the server-first, pre-negotiation message: the daemon
//! writes it before the frontend has said which version it speaks, so
//! a widened `Hello` would take effect before either peer knows which
//! version is in force and is unmeetable at every version, not only at
//! v27 (D34, ruled 2026-09-10). The token therefore rides an appended
//! [`crate::InstanceMessage::SessionToken`], sent to a session that
//! negotiated [`SESSION_MIN_VERSION`] or later, after its bootstrap and
//! before any frame. The frontend keeps the latest token it was given
//! and presents it in [`crate::FrontendEvent::Resume`], the one message
//! of its handshake step at v27: `Some(token)` to take back the view the
//! daemon retained for that token, `None` for a fresh view.
//!
//! The token is a bearer credential within the socket's own trust: the
//! daemon's socket directory is the user's (mode 0700), so a peer that
//! can present a token could already attach as the same user. It is
//! random so that a view is never resumed by accident --- by a second
//! window of the same user attaching while the first is reconnecting,
//! say --- not to authenticate across users.

/// First protocol version carrying the session family:
/// [`crate::InstanceMessage::SessionToken`],
/// [`crate::FrontendEvent::Resume`] and
/// [`crate::GoodbyeReason::Detached`].
///
/// Read as a constant rather than compared against a literal, and
/// **not** derived from [`crate::PROTOCOL_VERSION`]: expressing the
/// boundary against a moving ceiling would silently drag this family
/// forward on the next bump.
pub const SESSION_MIN_VERSION: u32 = 27;

/// Length of a [`SessionToken`] in bytes.
pub const SESSION_TOKEN_BYTES: usize = 16;

/// A daemon-minted session token: sixteen random bytes.
///
/// Fixed-width on purpose. postcard encodes a `[u8; N]` as its bytes
/// with no length prefix, so the token's wire size is a constant and a
/// peer cannot send a longer one; and a newtype keeps a token from being
/// passed where any other sixteen bytes would do.
///
/// `Debug` and `Display` print the token as hex, so a log line can be
/// matched against another by eye; nothing about a token is secret
/// beyond the socket it crossed (see the module doc).
#[derive(Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SessionToken(pub [u8; SESSION_TOKEN_BYTES]);

impl SessionToken {
    /// The all-zero token, which a daemon never mints: a default-built
    /// or half-initialized sender produces it, so the daemon treats it
    /// as "no token" rather than as a view to look up.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        let mut i = 0;
        while i < SESSION_TOKEN_BYTES {
            if self.0[i] != 0 {
                return false;
            }
            i += 1;
        }
        true
    }

    /// Lower-case hex, thirty-two characters.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(SESSION_TOKEN_BYTES * 2);
        for byte in self.0 {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
        }
        out
    }

    /// Parse the form [`Self::to_hex`] prints; `None` for anything else.
    #[must_use]
    pub fn from_hex(text: &str) -> Option<Self> {
        if text.len() != SESSION_TOKEN_BYTES * 2 || !text.is_ascii() {
            return None;
        }
        let mut bytes = [0u8; SESSION_TOKEN_BYTES];
        for (i, pair) in text.as_bytes().chunks_exact(2).enumerate() {
            let hex = std::str::from_utf8(pair).ok()?;
            bytes[i] = u8::from_str_radix(hex, 16).ok()?;
        }
        Some(Self(bytes))
    }
}

impl std::fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SessionToken({})", self.to_hex())
    }
}

impl std::fmt::Display for SessionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_sixteen_bytes_on_the_wire_with_no_length_prefix() {
        let token = SessionToken([7; SESSION_TOKEN_BYTES]);
        let bytes = postcard::to_allocvec(&token).expect("encode");
        assert_eq!(bytes, vec![7u8; SESSION_TOKEN_BYTES]);
        let decoded: SessionToken = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(decoded, token);
    }

    #[test]
    fn hex_round_trips_and_rejects_the_wrong_length_or_alphabet() {
        let mut bytes = [0u8; SESSION_TOKEN_BYTES];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::try_from(i * 17).unwrap_or(255);
        }
        let token = SessionToken(bytes);
        let hex = token.to_hex();
        assert_eq!(hex.len(), 32);
        assert_eq!(SessionToken::from_hex(&hex), Some(token));
        assert_eq!(format!("{token}"), hex);
        assert_eq!(format!("{token:?}"), format!("SessionToken({hex})"));
        assert_eq!(SessionToken::from_hex(&hex[1..]), None);
        assert_eq!(SessionToken::from_hex(&format!("{}zz", &hex[..30])), None);
        assert_eq!(SessionToken::from_hex("ééééééééééééééééé"), None);
    }

    #[test]
    fn zero_is_the_one_token_a_daemon_never_mints() {
        assert!(SessionToken([0; SESSION_TOKEN_BYTES]).is_zero());
        let mut one = [0u8; SESSION_TOKEN_BYTES];
        one[SESSION_TOKEN_BYTES - 1] = 1;
        assert!(!SessionToken(one).is_zero());
    }
}
