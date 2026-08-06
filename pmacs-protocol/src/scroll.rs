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
            // Saturating, then clamped: a caller that reports a cursor
            // past the end (a stale readout mid-edit) gets 100%, not a
            // wrapped or panicking percent.
            let pct = byte_pos.saturating_mul(100) / byte_len;
            ScrollPosition::Percent(u8::try_from(pct.min(100)).unwrap_or(100))
        }
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
}
