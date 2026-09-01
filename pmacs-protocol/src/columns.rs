// columns.rs --- The display-column rule both frontends reckon in.

//! One definition of "a display column", shared across the wire.
//!
//! The TUI and the GPU both need to answer "how wide is this line?" —
//! for the caret follow, for the minimap, and for GUI Stage 1b's B7
//! right bound. **Two copies of that answer is a defect waiting to
//! happen**: a bound computed one way and a follow computed the other
//! disagree about where the document ends, and the disagreement is
//! invisible until a tab or a wide character reaches the edge.
//!
//! It lives here for the same reason [`crate::scroll::follow_left`]
//! does — the protocol crate is the one place both frontends already
//! depend on.
//!
//! **Scope: SOURCE-TEXT columns.** Tab stops and Unicode terminal
//! width. Rendered projections — inline adornments, math substitutions
//! — can occupy a different width on screen and are deliberately not
//! counted here.

use unicode_width::UnicodeWidthChar;

/// Advance `column` past one character.
///
/// A tab reaches the next [`crate::TAB_STOP_COLUMNS`] stop; every other
/// character contributes its Unicode terminal width, so control and
/// zero-width characters do not advance.
#[must_use]
pub fn advance_char(column: u32, ch: char) -> u32 {
    let width = if ch == '\t' {
        crate::TAB_STOP_COLUMNS - (column % crate::TAB_STOP_COLUMNS)
    } else {
        UnicodeWidthChar::width(ch).unwrap_or(0) as u32
    };
    column.saturating_add(width)
}

/// Display width of one line, in columns.
#[must_use]
pub fn line_columns(line: &str) -> u32 {
    line.chars().fold(0, advance_char)
}

/// Widest line in `text`, in display columns — B7's right-bound input.
#[must_use]
pub fn widest_line_columns(text: &str) -> u32 {
    text.split('\n').map(line_columns).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{advance_char, line_columns, widest_line_columns};

    #[test]
    fn a_tab_reaches_the_next_stop_rather_than_advancing_one() {
        assert_eq!(advance_char(0, '\t'), crate::TAB_STOP_COLUMNS);
        assert_eq!(advance_char(1, '\t'), crate::TAB_STOP_COLUMNS);
        assert_eq!(
            advance_char(crate::TAB_STOP_COLUMNS, '\t'),
            crate::TAB_STOP_COLUMNS * 2
        );
    }

    #[test]
    fn wide_and_zero_width_characters_are_measured_not_counted() {
        assert_eq!(line_columns("ab"), 2);
        assert_eq!(line_columns("漢字"), 4, "wide characters take two columns");
        assert_eq!(line_columns("a\u{200b}b"), 2, "zero-width adds nothing");
    }

    /// The widest line, not the last one and not the first.
    #[test]
    fn widest_line_is_the_maximum_over_all_lines() {
        assert_eq!(widest_line_columns("a\nbbbb\ncc"), 4);
        assert_eq!(widest_line_columns(""), 0);
        assert_eq!(
            widest_line_columns("\tx"),
            crate::TAB_STOP_COLUMNS + 1,
            "tabs count toward the bound"
        );
    }
}
