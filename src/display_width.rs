// display_width.rs --- Shared byte-to-display-column accounting.

//! Allocation-free display-column helpers shared by text renderers.
//!
//! Source positions remain byte-addressed. Tabs are expanded only while
//! projecting those bytes into display columns, using the protocol-wide tab
//! stop. Offsets are clamped to the supplied slice and offsets inside a UTF-8
//! code point resolve to the preceding complete-code-point boundary.

use unicode_width::UnicodeWidthChar;

/// Advance `column` past one character.
///
/// A tab reaches the next protocol tab stop; all other characters use their
/// Unicode terminal width. Control and zero-width characters do not advance.
#[must_use]
pub fn advance_char(column: u32, ch: char) -> u32 {
    let width = if ch == '\t' {
        pmacs_protocol::TAB_STOP_COLUMNS - (column % pmacs_protocol::TAB_STOP_COLUMNS)
    } else {
        UnicodeWidthChar::width(ch).unwrap_or(0) as u32
    };
    column.saturating_add(width)
}

/// Display width of the valid UTF-8 prefix of `bytes`.
///
/// Invalid input is conservatively truncated at the first invalid byte. This
/// also floors a trailing partial code point without allocating or replacing
/// source bytes.
#[must_use]
pub fn valid_prefix_width(bytes: &[u8]) -> u32 {
    let valid_len = match std::str::from_utf8(bytes) {
        Ok(_) => bytes.len(),
        Err(error) => error.valid_up_to(),
    };
    let text = std::str::from_utf8(&bytes[..valid_len]).expect("valid_up_to is a UTF-8 boundary");
    text.chars().fold(0, advance_char)
}

/// Display column at the clamped byte boundary `offset`.
///
/// If `offset` splits a code point, the result is the column at that code
/// point's leading boundary.
#[must_use]
pub fn byte_to_column(bytes: &[u8], offset: usize) -> u32 {
    valid_prefix_width(&bytes[..offset.min(bytes.len())])
}

/// Display-column endpoints for the half-open byte range `[start, end)`.
///
/// Each endpoint is independently clamped and conservatively floored to a
/// complete UTF-8 boundary.
#[must_use]
pub fn byte_range_to_columns(bytes: &[u8], start: usize, end: usize) -> (u32, u32) {
    (byte_to_column(bytes, start), byte_to_column(bytes, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_advance_at_zero_before_stop_and_on_stop() {
        assert_eq!(advance_char(0, '\t'), 8);
        assert_eq!(advance_char(7, '\t'), 8);
        assert_eq!(advance_char(8, '\t'), 16);
    }

    #[test]
    fn unicode_widths_include_wide_and_zero_width_characters() {
        assert_eq!(advance_char(3, '中'), 5);
        assert_eq!(advance_char(3, '\u{301}'), 3);
        assert_eq!(valid_prefix_width("a中\u{301}b".as_bytes()), 4);
    }

    #[test]
    fn byte_columns_clamp_and_floor_partial_or_invalid_utf8() {
        let text = "a中b".as_bytes();
        assert_eq!(byte_to_column(text, 0), 0);
        assert_eq!(byte_to_column(text, 1), 1);
        assert_eq!(byte_to_column(text, 2), 1);
        assert_eq!(byte_to_column(text, 3), 1);
        assert_eq!(byte_to_column(text, 4), 3);
        assert_eq!(byte_to_column(text, usize::MAX), 4);

        assert_eq!(valid_prefix_width(b"ab\xffcd"), 2);
        assert_eq!(byte_to_column(b"ab\xe2\x82", 4), 2);
    }

    #[test]
    fn byte_ranges_map_half_open_endpoints_with_tab_expansion() {
        let text = b"a\tb";
        assert_eq!(byte_range_to_columns(text, 0, 1), (0, 1));
        assert_eq!(byte_range_to_columns(text, 1, 2), (1, 8));
        assert_eq!(byte_range_to_columns(text, 2, 3), (8, 9));
        assert_eq!(byte_range_to_columns(text, 99, 99), (9, 9));
    }

    #[test]
    fn range_boundaries_inside_codepoints_are_floored() {
        let text = "a中b".as_bytes();
        assert_eq!(byte_range_to_columns(text, 2, 3), (1, 1));
        assert_eq!(byte_range_to_columns(text, 2, 4), (1, 3));
    }
}
