//! Scroll-position classification, shared by both frontends.
//!
//! # Why this lives in the protocol crate
//!
//! It is not a wire type, and it is deliberately not presentation
//! either. The split it encodes (long-lines framing §5d.6,
//! `COHERENCE.md` §16) is:
//!
//! - **each frontend computes its own local layout facts** — whether
//!   the buffer's first or last row is on screen is a question only the
//!   frontend that laid the text out can answer;
//! - **the shared crate owns the semantic decision those facts feed** —
//!   what `All` / `Top` / `Bot` / `NN%` *mean* is one rule, not two.
//!
//! Rendering the outcome to a string stays in each frontend.
//!
//! `pmacs-gpu` depends on `pmacs-protocol` and never on the `pmacs`
//! lib, so before this module the status readout was **duplicated
//! structurally**: `format_scroll_indicator` exists once in
//! `src/editor.rs` and again in `pmacs-gpu/src/main.rs`, each with its
//! own tests. That is not a tidiness complaint — it produced a real
//! defect during this lane's own review, where a fix landed in one copy
//! and the other kept reporting `All` for a wrapped one-line buffer.
//! With one classifier, a frontend *cannot* classify differently.
//!
//! Adding this needs **no wire message and no protocol-version bump**:
//! [`classify`] is a pure function over values each side already holds.
//!
//! # Why the arguments are booleans and bytes
//!
//! The pre-existing formatter took four counts
//! (`view_top, visible, total_lines, cursor_row`) and derived every
//! branch from `total_lines`. Under line wrapping there is no row total
//! to give it: the GPU shapes only its viewport slice, so it cannot
//! count rows it never laid out, and computing a total by arithmetic
//! would disagree with the break points cosmic-text actually chose.
//!
//! Handing that signature byte counts instead would make
//! `view_top + visible >= total_lines` compare **rows against bytes** —
//! plausible strings, meaningless arithmetic. So the mixing is not
//! merely avoided here, it is **unrepresentable**: the caller supplies
//! two decided predicates and a byte pair, and no count of rows enters
//! this module at all.

/// Where the viewport sits in its buffer.
///
/// `Percent` carries whole percent in `0..=100`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScrollPosition {
    /// Every row of the buffer is on screen.
    All,
    /// The first row is on screen and the last is not.
    Top,
    /// The last row is on screen and the first is not.
    Bot,
    /// Neither end is on screen; whole percent through the buffer.
    Percent(u8),
}

/// Classify the viewport's position from local facts.
///
/// `first_visible` / `last_visible` are the frontend's own answers
/// about its current layout. `byte_pos` is the cursor's byte offset and
/// `byte_len` the buffer's length in bytes; the percentage is taken
/// from those rather than from a row ordinal, because no row total
/// exists (see the module docs).
///
/// An empty buffer (`byte_len == 0`) cannot have a meaningful
/// percentage, and division would trap. It only ever reaches the
/// `Percent` arm if the caller claims neither end is on screen, which
/// is already contradictory for an empty buffer — so that combination
/// yields `All`, matching what the caller's own facts would have said.
#[must_use]
pub fn classify(
    first_visible: bool,
    last_visible: bool,
    byte_pos: u64,
    byte_len: u64,
) -> ScrollPosition {
    match (first_visible, last_visible) {
        (true, true) => ScrollPosition::All,
        (true, false) => ScrollPosition::Top,
        (false, true) => ScrollPosition::Bot,
        (false, false) => {
            if byte_len == 0 {
                return ScrollPosition::All;
            }
            // Widen to u128 before scaling. `saturating_mul` was wrong
            // here, not merely inelegant: it *silently undercounts*.
            // `u64::MAX * 100` saturates to `u64::MAX`, so a cursor at
            // the end of a maximal buffer divided out to 1% — a wrong
            // answer that looked safe because it stayed in range.
            //
            // `u64::MAX * 100` fits in u128 with room to spare, so the
            // product is exact and the only clamp left is the genuine
            // one below.
            let pct = u128::from(byte_pos) * 100 / u128::from(byte_len);
            // Clamped for a caller that reports a cursor past the end
            // (a stale readout mid-edit): 100%, never above.
            ScrollPosition::Percent(u8::try_from(pct.min(100)).unwrap_or(100))
        }
    }
}

/// Move a horizontal viewport's left edge so `cursor_col` is visible,
/// returning the new edge. All three arguments and the result are
/// **columns**.
///
/// # Why this is shared, and why in columns
///
/// This is the horizontal twin of [`classify`], and it is here for the
/// same reason spelled out in the module docs: the two frontends were
/// about to hold one rule twice. The TUI stores its edge as a column
/// (`Window::view_left`); `pmacs-gpu` stores pixels, because its
/// geometry comes from cosmic-text advances. Long-lines Stage 5 Q#G1
/// settles that difference as a **conversion, not a second rule** — the
/// GPU rejects non-monospace code fonts (Q#G3), so px ↔ column is exact
/// through the resolved advance, and the GPU divides on the way in and
/// multiplies on the way out.
///
/// Columns, not pixels, is the shared unit because it is the one both
/// sides can name. A pixel rule would force the TUI into float
/// arithmetic over a quantity that is integral by construction, and an
/// off-by-one from rounding there is a character the user cannot read —
/// the whole complaint this arc answers.
///
/// # The rule
///
/// Scroll the minimum distance that puts the cursor back inside, so a
/// cursor already visible never moves the view. `width == 0` means
/// nothing has been laid out yet: no column is visible, so no edge is
/// better than another and the current one stands.
#[must_use]
pub fn follow_left(left: u32, cursor_col: u32, width: u32) -> u32 {
    if width == 0 {
        return left;
    }
    if cursor_col < left {
        cursor_col
    } else if cursor_col >= left.saturating_add(width) {
        // `+ 1` puts the cursor's own column at the right edge rather
        // than one past it. No underflow: this arm implies
        // `cursor_col >= width`.
        cursor_col.saturating_add(1) - width
    } else {
        left
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four outcomes are decided by the two predicates alone.
    #[test]
    fn the_two_predicates_decide_the_three_named_states() {
        assert_eq!(classify(true, true, 0, 100), ScrollPosition::All);
        assert_eq!(classify(true, false, 0, 100), ScrollPosition::Top);
        assert_eq!(classify(false, true, 99, 100), ScrollPosition::Bot);
        assert!(matches!(
            classify(false, false, 50, 100),
            ScrollPosition::Percent(_)
        ));
    }

    /// The case the whole of framing §5d exists for.
    ///
    /// A one-source-line buffer that wraps to more rows than fit must
    /// report `Top`, not `All`. The pre-existing formatter returned
    /// `All` here unconditionally — its first branch was
    /// `if total_lines <= 1 { return "All" }`, and a wrapped single line
    /// still has `total_lines == 1`.
    ///
    /// This classifier cannot reproduce that bug, because it is never
    /// told how many lines there are.
    #[test]
    fn a_wrapped_single_line_is_top_not_all() {
        // One line, first row on screen, last row far below.
        assert_eq!(classify(true, false, 0, 4_000), ScrollPosition::Top);
    }

    /// Percent comes from bytes, and only when neither end shows.
    #[test]
    fn percent_is_byte_based_and_only_in_the_middle() {
        assert_eq!(
            classify(false, false, 250, 1_000),
            ScrollPosition::Percent(25)
        );
        // Even at a byte position that would read as "the end", an
        // explicit `last_visible` wins — the predicate is the fact, the
        // percentage is only a readout.
        assert_eq!(classify(false, true, 1_000, 1_000), ScrollPosition::Bot);
    }

    /// Degenerate inputs stay total: no panic, no wrap, no divide by zero.
    #[test]
    fn degenerate_inputs_are_total() {
        assert_eq!(classify(false, false, 0, 0), ScrollPosition::All);
        assert_eq!(
            classify(false, false, 10, 5),
            ScrollPosition::Percent(100),
            "a cursor past the end clamps rather than exceeding 100"
        );
        assert_eq!(
            classify(false, false, u64::MAX, 1),
            ScrollPosition::Percent(100),
            "saturating multiply, so a huge offset cannot wrap into a small percent"
        );
    }

    /// Percent never leaves `0..=100`, for any input.
    ///
    /// **In range is not the same as correct**, which is why
    /// [`large_byte_counts_stay_accurate`] exists beside this. This
    /// sweep passed against a `saturating_mul` that silently reported
    /// 1% for a cursor at the end of a maximal buffer — a wrong answer
    /// that satisfies every assertion here.
    #[test]
    fn percent_is_always_in_range() {
        for pos in [0_u64, 1, 7, 99, 100, 1_000, u64::MAX / 2, u64::MAX] {
            for len in [1_u64, 3, 100, 9_999, u64::MAX] {
                if let ScrollPosition::Percent(p) = classify(false, false, pos, len) {
                    assert!(p <= 100, "pos={pos} len={len} gave {p}%");
                }
            }
        }
    }

    /// The percentage stays *accurate* where `u64` arithmetic would
    /// overflow, not merely bounded.
    ///
    /// `byte_pos * 100` exceeds `u64::MAX` for any position above
    /// `u64::MAX / 100`. Saturating there collapses the numerator to a
    /// constant, so the quotient stops tracking the position at all:
    /// `u64::MAX / u64::MAX` is 1, and the readout said **1%** at the
    /// very end of the buffer.
    #[test]
    fn large_byte_counts_stay_accurate() {
        assert_eq!(
            classify(false, false, u64::MAX, u64::MAX),
            ScrollPosition::Percent(100),
            "the end of a maximal buffer is 100%, not 1%"
        );
        assert_eq!(
            classify(false, false, u64::MAX / 2, u64::MAX),
            ScrollPosition::Percent(49),
            "halfway through a maximal buffer, floored"
        );
        assert_eq!(
            classify(false, false, u64::MAX / 4, u64::MAX),
            ScrollPosition::Percent(24),
            "a quarter through, floored"
        );
        // The smallest position whose scaling overflows u64 — the first
        // input the old implementation got wrong.
        let first_overflowing = u64::MAX / 100 + 1;
        assert_eq!(
            classify(false, false, first_overflowing, u64::MAX),
            ScrollPosition::Percent(1),
            "correct by arithmetic here, not by saturation"
        );
    }

    /// A cursor already inside the window never moves the edge — the
    /// property that separates "follow" from "center".
    #[test]
    fn a_visible_cursor_leaves_the_edge_alone() {
        for col in 10..90 {
            assert_eq!(follow_left(10, col, 80), 10, "column {col} is visible");
        }
    }

    /// Both edges, minimally.
    #[test]
    fn the_edge_moves_the_minimum_distance_in_each_direction() {
        // Left: the cursor's own column becomes the first visible one.
        assert_eq!(follow_left(10, 4, 80), 4);
        // Right: the cursor's own column becomes the LAST visible one,
        // which is `+ 1 - width`, not `- width`. Dropping the `+ 1`
        // parks the caret one column off the right edge — invisible,
        // and the exact defect this arc reports.
        assert_eq!(follow_left(10, 90, 80), 11);
        assert_eq!(follow_left(0, 79, 80), 0, "the last column still fits");
        assert_eq!(follow_left(0, 80, 80), 1, "one past it scrolls by one");
    }

    /// Nothing laid out yet: no column is visible, so the edge stands
    /// rather than snapping to a cursor whose geometry is unknown.
    #[test]
    fn a_zero_width_viewport_holds_its_edge() {
        assert_eq!(follow_left(7, 0, 0), 7);
        assert_eq!(follow_left(7, 9999, 0), 7);
    }

    /// The saturating arms are reachable arithmetic, not decoration.
    ///
    /// At `cursor_col == u32::MAX` the saturation absorbs the `+ 1`, so
    /// the edge lands one column short of showing that column. Asserted
    /// as the value it actually produces rather than the value the rule
    /// would like: a line 4·10⁹ columns wide does not occur, and a test
    /// that lied about this arm to look tidy would be worse than the
    /// one-column imprecision it hid.
    #[test]
    fn extreme_columns_do_not_panic() {
        assert_eq!(follow_left(0, u32::MAX, 80), u32::MAX - 80);
        assert_eq!(follow_left(u32::MAX, 0, 80), 0);
        // `left + width` overflows; the cursor is nonetheless left of
        // the edge, so the first arm decides and nothing wraps.
        assert_eq!(follow_left(u32::MAX - 1, 5, 80), 5);
    }
}
