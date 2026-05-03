// key.rs --- Chord type and Emacs-style parser.

//! Keypress representation and Emacs-style notation parser.
//!
//! Pmacs's keymap (T M2.4) lives over [`crossterm::event::KeyEvent`]
//! but doesn't expose crossterm types to users: a chord is a
//! [`Chord`] (key + modifiers) and a sequence of chords is a
//! [`Sequence`]. The parser reads strings like `"C-x C-s"` into a
//! [`Sequence`]; `Display` prints the canonical form back.
//!
//! # Notation
//!
//! Single chord: zero or more single-letter modifier prefixes followed
//! by a `-` and one named-or-character key.
//!
//! * `C-` --- Control
//! * `M-` --- Alt (Meta in Emacs lineage)
//! * `S-` --- Shift (only meaningful for non-letter keys; letters
//!   should use the literal uppercase character)
//! * `s-` --- Super
//!
//! Modifiers commute (`C-M-x` and `M-C-x` parse identically). Keys are
//! either a single character (`a`, `1`, `/`) or a name:
//!
//! * `RET` (Enter), `SPC` (Space), `TAB`, `ESC`, `BS` (Backspace),
//!   `DEL` (Delete)
//! * `<up>`, `<down>`, `<left>`, `<right>`
//! * `<home>`, `<end>`, `<pageup>`, `<pagedown>`, `<insert>`
//! * `<f1>` ... `<f12>`
//!
//! Sequences are space-separated chords: `C-x C-s` is two chords.

use std::fmt;

use crossterm::event::{KeyCode, KeyModifiers};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Chord and Sequence
// ---------------------------------------------------------------------------

/// A single keypress with modifiers.
///
/// Two `Chord`s are equal when their `code` and `modifiers` match
/// exactly. The keymap trie hashes on this.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct Chord {
    /// The key code (after kitty-keyboard-protocol disambiguation when
    /// the terminal supports it; see frontend setup at T M2.4).
    pub code: KeyCode,
    /// The active modifiers. We canonicalize on construction (e.g.
    /// uppercase letters strip SHIFT) so logically-equivalent input
    /// hashes the same way.
    pub modifiers: KeyModifiers,
}

impl Chord {
    /// Build a chord, canonicalizing modifiers vs code.
    ///
    /// Canonicalization rules:
    /// * `KeyCode::Char('A')` with `SHIFT` --- the SHIFT bit is
    ///   stripped; the uppercase letter already implies it.
    /// * `KeyCode::Char(c)` for any `c` whose lowercase is itself
    ///   (`/`, `1`, ...) leaves modifiers as-is.
    #[must_use]
    pub fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        let mods = match code {
            KeyCode::Char(ch) if ch.is_ascii_uppercase() => modifiers - KeyModifiers::SHIFT,
            _ => modifiers,
        };
        Self {
            code,
            modifiers: mods,
        }
    }

    /// Build a plain unmodified chord.
    #[must_use]
    pub fn plain(code: KeyCode) -> Self {
        Self::new(code, KeyModifiers::NONE)
    }
}

/// A sequence of one or more chords (e.g. `C-x C-s` is two chords).
///
/// Defined as `Vec<Chord>` rather than a slice newtype because most
/// call sites build sequences incrementally (the dispatcher's pending
/// prefix buffer) or pass them as already-owned vectors.
pub type Sequence = Vec<Chord>;

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parser failures.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum KeyParseError {
    /// The input was empty or contained only whitespace.
    #[error("empty key sequence")]
    Empty,

    /// A modifier prefix didn't have a `-` or had a duplicate.
    #[error("bad modifier in chord {chord:?}: {detail}")]
    BadModifier {
        /// The chord substring that failed.
        chord: String,
        /// What specifically was wrong.
        detail: String,
    },

    /// The token after the last `-` (or the bare token) wasn't a known
    /// named key or a single character.
    #[error("unknown key {token:?} in chord {chord:?}")]
    UnknownKey {
        /// The chord substring that failed.
        chord: String,
        /// The unknown token.
        token: String,
    },
}

/// Parse a single chord, e.g. `C-x` or `M-RET` or `<f5>` or `a`.
///
/// # Errors
///
/// Returns [`KeyParseError`] for empty input, malformed modifier
/// prefixes, or unknown keys.
pub fn parse_chord(s: &str) -> Result<Chord, KeyParseError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(KeyParseError::Empty);
    }

    let mut modifiers = KeyModifiers::NONE;
    let mut rest = trimmed;

    // Modifier prefixes: while the next two chars are "X-" with X in
    // {C, M, S, s} and there's something after, peel it off.
    loop {
        let mut chars = rest.chars();
        let first = chars.next();
        let second = chars.next();
        let after = chars.as_str();
        match (first, second) {
            (Some(c), Some('-')) if "CMSs".contains(c) && !after.is_empty() => {
                let bit = match c {
                    'C' => KeyModifiers::CONTROL,
                    'M' => KeyModifiers::ALT,
                    'S' => KeyModifiers::SHIFT,
                    's' => KeyModifiers::SUPER,
                    _ => unreachable!(),
                };
                if modifiers.contains(bit) {
                    return Err(KeyParseError::BadModifier {
                        chord: trimmed.to_owned(),
                        detail: format!("modifier `{c}-` repeated"),
                    });
                }
                modifiers |= bit;
                rest = after;
            }
            _ => break,
        }
    }

    let code = parse_key_code(rest, trimmed)?;
    Ok(Chord::new(code, modifiers))
}

/// Parse a sequence: whitespace-separated chords. `C-x C-s` is two
/// chords; `C-x  f` is two (extra spaces are fine).
///
/// # Errors
///
/// Returns [`KeyParseError::Empty`] if no chords parse out, or any
/// per-chord error from [`parse_chord`].
pub fn parse_sequence(s: &str) -> Result<Sequence, KeyParseError> {
    let mut out = Vec::new();
    for tok in s.split_whitespace() {
        out.push(parse_chord(tok)?);
    }
    if out.is_empty() {
        return Err(KeyParseError::Empty);
    }
    Ok(out)
}

fn parse_key_code(token: &str, full_chord: &str) -> Result<KeyCode, KeyParseError> {
    // Named keys (case-sensitive uppercase canonical, but accept lowercase).
    let upper = token.to_ascii_uppercase();
    let named = match upper.as_str() {
        "RET" | "RETURN" | "ENTER" => Some(KeyCode::Enter),
        "TAB" => Some(KeyCode::Tab),
        "SPC" | "SPACE" => Some(KeyCode::Char(' ')),
        "ESC" | "ESCAPE" => Some(KeyCode::Esc),
        "BS" | "BACKSPACE" => Some(KeyCode::Backspace),
        "DEL" | "DELETE" => Some(KeyCode::Delete),
        _ => None,
    };
    if let Some(c) = named {
        return Ok(c);
    }

    // Bracketed names: <up>, <f5>, <pageup>, ...
    if token.starts_with('<') && token.ends_with('>') && token.len() >= 3 {
        let inner = &token[1..token.len() - 1].to_ascii_lowercase();
        let code = match inner.as_str() {
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" => KeyCode::PageDown,
            "insert" | "ins" => KeyCode::Insert,
            other if other.starts_with('f') => {
                let n: u8 = other[1..].parse().map_err(|_| KeyParseError::UnknownKey {
                    chord: full_chord.to_owned(),
                    token: token.to_owned(),
                })?;
                if !(1..=12).contains(&n) {
                    return Err(KeyParseError::UnknownKey {
                        chord: full_chord.to_owned(),
                        token: token.to_owned(),
                    });
                }
                KeyCode::F(n)
            }
            _ => {
                return Err(KeyParseError::UnknownKey {
                    chord: full_chord.to_owned(),
                    token: token.to_owned(),
                });
            }
        };
        return Ok(code);
    }

    // Single character.
    let mut chars = token.chars();
    let ch = chars.next().ok_or_else(|| KeyParseError::UnknownKey {
        chord: full_chord.to_owned(),
        token: token.to_owned(),
    })?;
    if chars.next().is_some() {
        // Multi-char token that didn't match any rule above.
        return Err(KeyParseError::UnknownKey {
            chord: full_chord.to_owned(),
            token: token.to_owned(),
        });
    }
    Ok(KeyCode::Char(ch))
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            f.write_str("C-")?;
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            f.write_str("M-")?;
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            f.write_str("S-")?;
        }
        if self.modifiers.contains(KeyModifiers::SUPER) {
            f.write_str("s-")?;
        }
        match self.code {
            KeyCode::Char(' ') => f.write_str("SPC"),
            KeyCode::Char(c) => write!(f, "{c}"),
            KeyCode::Enter => f.write_str("RET"),
            KeyCode::Tab => f.write_str("TAB"),
            KeyCode::Esc => f.write_str("ESC"),
            KeyCode::Backspace => f.write_str("BS"),
            KeyCode::Delete => f.write_str("DEL"),
            KeyCode::Up => f.write_str("<up>"),
            KeyCode::Down => f.write_str("<down>"),
            KeyCode::Left => f.write_str("<left>"),
            KeyCode::Right => f.write_str("<right>"),
            KeyCode::Home => f.write_str("<home>"),
            KeyCode::End => f.write_str("<end>"),
            KeyCode::PageUp => f.write_str("<pageup>"),
            KeyCode::PageDown => f.write_str("<pagedown>"),
            KeyCode::Insert => f.write_str("<insert>"),
            KeyCode::F(n) => write!(f, "<f{n}>"),
            other => write!(f, "{other:?}"),
        }
    }
}

/// Render a sequence as a space-separated chord string.
#[must_use]
pub fn display_sequence(seq: &[Chord]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for (i, c) in seq.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{c}");
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl(c: char) -> Chord {
        Chord::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }
    fn plain(c: char) -> Chord {
        Chord::plain(KeyCode::Char(c))
    }

    #[test]
    fn parses_plain_char() {
        assert_eq!(parse_chord("a").unwrap(), plain('a'));
        assert_eq!(parse_chord("/").unwrap(), plain('/'));
    }

    #[test]
    fn parses_control_modifier() {
        assert_eq!(parse_chord("C-x").unwrap(), ctrl('x'));
    }

    #[test]
    fn parses_meta_and_combinations() {
        assert_eq!(
            parse_chord("M-x").unwrap(),
            Chord::new(KeyCode::Char('x'), KeyModifiers::ALT)
        );
        assert_eq!(
            parse_chord("C-M-x").unwrap(),
            Chord::new(
                KeyCode::Char('x'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )
        );
        // Modifiers commute.
        assert_eq!(parse_chord("C-M-x").unwrap(), parse_chord("M-C-x").unwrap());
    }

    #[test]
    fn parses_named_keys() {
        assert_eq!(parse_chord("RET").unwrap(), Chord::plain(KeyCode::Enter));
        assert_eq!(
            parse_chord("SPC").unwrap(),
            Chord::plain(KeyCode::Char(' '))
        );
        assert_eq!(parse_chord("TAB").unwrap(), Chord::plain(KeyCode::Tab));
        assert_eq!(parse_chord("ESC").unwrap(), Chord::plain(KeyCode::Esc));
        assert_eq!(parse_chord("BS").unwrap(), Chord::plain(KeyCode::Backspace));
        assert_eq!(parse_chord("DEL").unwrap(), Chord::plain(KeyCode::Delete));
    }

    #[test]
    fn parses_bracketed_keys() {
        assert_eq!(parse_chord("<up>").unwrap(), Chord::plain(KeyCode::Up));
        assert_eq!(parse_chord("<f1>").unwrap(), Chord::plain(KeyCode::F(1)));
        assert_eq!(parse_chord("<f12>").unwrap(), Chord::plain(KeyCode::F(12)));
        assert_eq!(
            parse_chord("<pageup>").unwrap(),
            Chord::plain(KeyCode::PageUp)
        );
        assert_eq!(
            parse_chord("C-<up>").unwrap(),
            Chord::new(KeyCode::Up, KeyModifiers::CONTROL)
        );
    }

    #[test]
    fn parses_sequences() {
        let seq = parse_sequence("C-x C-s").unwrap();
        assert_eq!(seq, vec![ctrl('x'), ctrl('s')]);
        let seq = parse_sequence("C-x  f  RET").unwrap();
        assert_eq!(seq.len(), 3);
        assert_eq!(seq[2].code, KeyCode::Enter);
    }

    #[test]
    fn empty_input_errors() {
        assert_eq!(parse_chord("").unwrap_err(), KeyParseError::Empty);
        assert_eq!(parse_chord("   ").unwrap_err(), KeyParseError::Empty);
        assert_eq!(parse_sequence("").unwrap_err(), KeyParseError::Empty);
    }

    #[test]
    fn unknown_key_errors() {
        match parse_chord("<wat>") {
            Err(KeyParseError::UnknownKey { token, .. }) => assert_eq!(token, "<wat>"),
            other => panic!("expected UnknownKey, got {other:?}"),
        }
        match parse_chord("<f99>") {
            Err(KeyParseError::UnknownKey { token, .. }) => assert_eq!(token, "<f99>"),
            other => panic!("expected UnknownKey, got {other:?}"),
        }
        // Multi-char token that's not bracketed and not named.
        match parse_chord("hello") {
            Err(KeyParseError::UnknownKey { token, .. }) => assert_eq!(token, "hello"),
            other => panic!("expected UnknownKey, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_modifier_errors() {
        match parse_chord("C-C-x") {
            Err(KeyParseError::BadModifier { detail, .. }) => {
                assert!(detail.contains("repeated"), "detail: {detail}");
            }
            other => panic!("expected BadModifier, got {other:?}"),
        }
    }

    #[test]
    fn uppercase_letter_strips_shift_bit() {
        // A user typing "A" (literal) doesn't carry the SHIFT modifier;
        // the uppercase letter implies it. We canonicalize so a chord
        // built from KeyCode::Char('A') + SHIFT hashes the same as the
        // bare uppercase chord.
        let with_shift = Chord::new(KeyCode::Char('A'), KeyModifiers::SHIFT);
        let bare = Chord::plain(KeyCode::Char('A'));
        assert_eq!(with_shift, bare);
    }

    #[test]
    fn display_round_trips_canonical_form() {
        let cases = [
            ("a", plain('a')),
            ("C-x", ctrl('x')),
            (
                "C-M-x",
                Chord::new(
                    KeyCode::Char('x'),
                    KeyModifiers::CONTROL | KeyModifiers::ALT,
                ),
            ),
            ("RET", Chord::plain(KeyCode::Enter)),
            ("SPC", Chord::plain(KeyCode::Char(' '))),
            ("<up>", Chord::plain(KeyCode::Up)),
            ("<f5>", Chord::plain(KeyCode::F(5))),
        ];
        for (text, chord) in cases {
            assert_eq!(format!("{chord}"), text, "display of {chord:?}");
            assert_eq!(parse_chord(text).unwrap(), chord, "parse of {text:?}");
        }
    }

    #[test]
    fn display_sequence_round_trips() {
        let seq = vec![ctrl('x'), ctrl('s')];
        let s = display_sequence(&seq);
        assert_eq!(s, "C-x C-s");
        assert_eq!(parse_sequence(&s).unwrap(), seq);
    }
}
