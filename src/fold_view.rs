// fold_view.rs --- The visible-line map (Arc 6, Stage 2).

//! The source-line ↔ display-row projection that folding introduces.
//!
//! Before folding, every grid consumer assumed `display_row =
//! source_line − view_top` — an identity map baked into the text walk,
//! the gutter, every overlay, the caret, both selection painters, the
//! mode-line indicator, and the click/scroll/motion inverses.
//! [`DisplayCoord`](crate::view::DisplayCoord) anticipated a
//! non-identity map "once virtual lines, wrapping, and inline
//! expansions appear"; **folding is the first**.
//!
//! This module is that map — **one derivation/query primitive**
//! (`docs/archive/framings/folding-stage2-framing.md` Q#FD12), derived from
//! [`crate::fold::FoldRegistry::folds`] plus the buffer's line offsets
//! and **never stored**: the byte-range store in [`crate::fold`] stays
//! the single source of truth. Instances are short-lived and built
//! **per rendered window** and **per command/event operation**, never
//! once per frame — a frame paints several windows that may show
//! different buffers, so a per-frame singleton would leak one pane's
//! folds into another's (framing round-2 F2).
//!
//! # Hidden components, not folds
//!
//! The unit here is not a fold. [`crate::fold::FoldStore::insert`]
//! accepts any normalized range, so folds may nest, share a head line,
//! or **cross**: with fold `A` hiding lines 1–3 and fold `B` headed on
//! line 2 hiding lines 3–5, a point on line 5 is directly inside only
//! `B` — yet `B`'s own head is hidden by `A`, so projecting to `B`'s
//! `range.start` would land on another *hidden* position (round-3 F2).
//!
//! The derivation therefore unions overlapping **or adjacent** hidden
//! line intervals into sorted, non-overlapping **hidden components**.
//! Adjacent intervals merge because the later fold's head is hidden by
//! the earlier one, so it can never render. Each component keeps the one
//! visible line immediately before it (`head_line`) and that line's exact
//! end-of-content byte (`head_position` — the fold `range.start` Stage 1
//! already moves point to). Resolving through the component is
//! equivalent to repeatedly projecting a hidden fold head until it is
//! visible, and so covers nesting, shared heads, and crossing overlap
//! alike.

use pmacs_protocol::ByteRange;

use crate::fold::FoldRegistry;
use crate::rope::Position;
use crate::window::Window;

/// The visible-line map for one **window's** buffer, or `None` when that
/// buffer has no folds.
///
/// The single construction rule shared by the render path and
/// `EditorCore` (Q#FD12): keyed on *this* window's `buffer_id` and its
/// own [`TextView`](crate::text_view::TextView) line offsets — never the
/// active buffer's — so a split showing two buffers gets two independent
/// maps and neither leaks into the other (round-2 F2). Returning `None`
/// rather than an empty map keeps the unfolded path byte-identical.
#[must_use]
pub fn map_for_window(registry: &FoldRegistry, window: &Window) -> Option<VisibleLineMap> {
    let folds = registry.folds(window.buffer_id);
    if folds.is_empty() {
        return None;
    }
    Some(VisibleLineMap::build(&folds, |off| {
        window.text_view.line_at_offset(off)
    }))
}

/// A maximal run of consecutive hidden source lines, plus the one
/// visible line that heads it.
///
/// `first_hidden >= 1` always: a component's `head_line` is
/// `first_hidden - 1`, and a fold's head line is the line *above* its
/// first hidden line, so line 0 can never be hidden.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HiddenComponent {
    /// First hidden source line (inclusive).
    first_hidden: usize,
    /// Last hidden source line (inclusive).
    last_hidden: usize,
    /// End-of-content byte of `head_line` — the `ByteRange::start` of
    /// the earliest fold participating in this component, which is
    /// exactly where Stage 1 moves point on a fold-at-cursor.
    head_position: Position,
}

impl HiddenComponent {
    /// The one visible line immediately above this component.
    const fn head_line(&self) -> usize {
        self.first_hidden - 1
    }
}

/// A buffer's collapsed regions, projected into line space.
///
/// Derived from a fold list and a byte→line lookup; cheap enough to
/// rebuild per window per frame (**Bet B4**: `O(folds)` with one binary
/// search into the caller's existing line-offset table per fold, and
/// folds are `O(top-level blocks)`).
///
/// An empty map (`is_identity`) means "no folds" — callers pass `None`
/// rather than an empty map so the unfolded path stays byte-identical.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VisibleLineMap {
    /// Sorted by `first_hidden`, non-overlapping, and separated by at
    /// least one visible line (adjacency is merged away at build time).
    components: Vec<HiddenComponent>,
}

impl VisibleLineMap {
    /// Derive the map from a buffer's folds.
    ///
    /// `line_at_offset` is the caller's own line-offset lookup (the
    /// rendering window's [`TextView`](crate::text_view::TextView), the
    /// only table guaranteed to agree with the rows being painted). A
    /// fold's stored range is `[end of head line, end of last hidden
    /// line]`, so `head_line = line_at_offset(start)` and `last_hidden =
    /// line_at_offset(end)`; a fold that no longer spans a whole line
    /// (mid-edit drift) contributes nothing.
    #[must_use]
    pub fn build<F>(folds: &[ByteRange], line_at_offset: F) -> Self
    where
        F: Fn(Position) -> usize,
    {
        let mut raw: Vec<HiddenComponent> = folds
            .iter()
            .filter_map(|f| {
                let head_line = line_at_offset(f.start);
                let last_hidden = line_at_offset(f.end);
                (last_hidden > head_line).then_some(HiddenComponent {
                    first_hidden: head_line + 1,
                    last_hidden,
                    head_position: f.start,
                })
            })
            .collect();
        raw.sort_by(|a, b| {
            a.first_hidden
                .cmp(&b.first_hidden)
                .then(a.last_hidden.cmp(&b.last_hidden))
        });
        let mut components: Vec<HiddenComponent> = Vec::with_capacity(raw.len());
        for c in raw {
            match components.last_mut() {
                // Overlapping OR adjacent: `c`'s head line is itself
                // hidden by `prev`, so it can never render — the merged
                // component keeps `prev`'s (visible) head.
                Some(prev) if c.first_hidden <= prev.last_hidden + 1 => {
                    prev.last_hidden = prev.last_hidden.max(c.last_hidden);
                }
                _ => components.push(c),
            }
        }
        Self { components }
    }

    /// Whether this map hides nothing — the identity projection.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.components.is_empty()
    }

    /// The component hiding `line`, if any.
    fn component_of(&self, line: usize) -> Option<&HiddenComponent> {
        let after = self.components.partition_point(|c| c.first_hidden <= line);
        let c = self.components.get(after.checked_sub(1)?)?;
        (line <= c.last_hidden).then_some(c)
    }

    /// Whether `line` is collapsed away and renders no row.
    #[must_use]
    pub fn is_hidden(&self, line: usize) -> bool {
        self.component_of(line).is_some()
    }

    /// Whether `line` is the visible head of a collapsed region — the
    /// row that carries the ellipsis and the gutter fold glyph.
    #[must_use]
    pub fn is_head(&self, line: usize) -> bool {
        self.components
            .binary_search_by(|c| c.first_hidden.cmp(&(line + 1)))
            .is_ok()
    }

    /// The **outermost visible head** of `line`: for a hidden line, its
    /// component's head line; for a visible line, itself.
    ///
    /// The **row-only** clamp — diagnostic signs, the relative-number
    /// cursor anchor, and the backward `view_top` clamp. Positions that
    /// carry a column use [`Self::visible_position`] instead.
    #[must_use]
    pub fn visible_head_of(&self, line: usize) -> usize {
        self.component_of(line)
            .map_or(line, HiddenComponent::head_line)
    }

    /// The **position** projection of a byte on `line`: for a hidden
    /// line, its component's `head_position` (the head line's
    /// end-of-content byte); for a visible line, `pos` unchanged.
    ///
    /// Used wherever a clamp carries a column — the local caret, peer
    /// cursors, and selection endpoints — so a hidden point lands at the
    /// head's end of content rather than at an arbitrary column on the
    /// head (round-2 F3) or at a still-hidden crossing fold's start
    /// (round-3 F2).
    #[must_use]
    pub fn visible_position(&self, line: usize, pos: Position) -> Position {
        self.component_of(line).map_or(pos, |c| c.head_position)
    }

    /// Clamp a candidate `view_top` **backward** to a visible line, so a
    /// fold at the top of the viewport shows its head rather than being
    /// skipped past (framing acceptance 8).
    #[must_use]
    pub fn clamp_view_top(&self, line: usize) -> usize {
        self.visible_head_of(line)
    }

    /// The next visible line strictly after `line`, skipping whole
    /// collapsed regions. May exceed the buffer's line count; callers
    /// bound it themselves.
    #[must_use]
    pub fn next_visible(&self, line: usize) -> usize {
        let next = line + 1;
        self.component_of(next)
            .map_or(next, |c| c.last_hidden.saturating_add(1))
    }

    /// The previous visible line strictly before `line`, or `0` when
    /// `line` is already the first line.
    #[must_use]
    pub fn prev_visible(&self, line: usize) -> usize {
        match line.checked_sub(1) {
            Some(prev) => self.visible_head_of(prev),
            None => 0,
        }
    }

    /// Number of visible lines in the half-open range `[from, to)`;
    /// `0` when `to <= from`.
    ///
    /// This is the framing's `visible_between` — exposed unsigned and
    /// half-open (plus the symmetric [`Self::visible_distance`]) because
    /// no consumer reads the sign: row offsets always measure forward
    /// from `view_top`, and relative line numbers want a magnitude.
    #[must_use]
    pub fn visible_rows_between(&self, from: usize, to: usize) -> usize {
        if to <= from {
            return 0;
        }
        (to - from) - self.hidden_in(from, to)
    }

    /// Visible-line distance between `a` and `b`, either order — the
    /// relative/hybrid gutter number measured across collapses.
    #[must_use]
    pub fn visible_distance(&self, a: usize, b: usize) -> usize {
        if a <= b {
            self.visible_rows_between(a, b)
        } else {
            self.visible_rows_between(b, a)
        }
    }

    /// Hidden lines within the half-open range `[from, to)`.
    fn hidden_in(&self, from: usize, to: usize) -> usize {
        self.components
            .iter()
            .filter(|c| c.first_hidden < to && c.last_hidden >= from)
            .map(|c| {
                // `lo <= hi` holds under the filter, so this cannot
                // underflow.
                let lo = c.first_hidden.max(from);
                let hi = c.last_hidden.min(to - 1);
                hi + 1 - lo
            })
            .sum()
    }

    /// Total visible lines in a buffer of `total_lines` source lines —
    /// the denominator the mode-line scroll indicator reckons in.
    #[must_use]
    pub fn visible_line_count(&self, total_lines: usize) -> usize {
        total_lines - self.hidden_in(0, total_lines).min(total_lines)
    }

    /// The line `n` visible steps forward from `from` (which is first
    /// normalized to its visible head). `n == 0` yields that head.
    #[must_use]
    pub fn nth_visible_from(&self, from: usize, n: usize) -> usize {
        let mut line = self.visible_head_of(from);
        for _ in 0..n {
            line = self.next_visible(line);
        }
        line
    }

    /// The line `n` visible steps back from `from` (first normalized to
    /// its visible head), saturating at line 0.
    #[must_use]
    pub fn nth_visible_back(&self, from: usize, n: usize) -> usize {
        let mut line = self.visible_head_of(from);
        for _ in 0..n {
            if line == 0 {
                break;
            }
            line = self.prev_visible(line);
        }
        line
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A 40-line buffer of `"L<n>\n"`-ish rows, 8 bytes each, so line
    /// `n` starts at `8n` and its content ends at `8n + 7`.
    fn line_of(offset: Position) -> usize {
        (offset / 8) as usize
    }

    /// The fold that hides lines `first..=last` in that fixture.
    fn fold(head: usize, last_hidden: usize) -> ByteRange {
        ByteRange {
            start: (head as u64) * 8 + 7,
            end: (last_hidden as u64) * 8 + 7,
        }
    }

    fn map(folds: &[ByteRange]) -> VisibleLineMap {
        VisibleLineMap::build(folds, line_of)
    }

    #[test]
    fn empty_map_is_identity() {
        let m = map(&[]);
        assert!(m.is_identity());
        assert!(!m.is_hidden(5));
        assert_eq!(m.visible_head_of(5), 5);
        assert_eq!(m.next_visible(5), 6);
        assert_eq!(m.visible_rows_between(0, 10), 10);
    }

    #[test]
    fn single_fold_hides_its_interior_only() {
        // head 2, hidden 3..=6.
        let m = map(&[fold(2, 6)]);
        assert!(!m.is_hidden(2));
        assert!(m.is_head(2));
        for line in 3..=6 {
            assert!(m.is_hidden(line), "line {line} should be hidden");
            assert_eq!(m.visible_head_of(line), 2);
        }
        assert!(!m.is_hidden(7));
        assert_eq!(m.next_visible(2), 7);
        assert_eq!(m.prev_visible(7), 2);
        // 0,1,2,7,8,9 visible in [0,10).
        assert_eq!(m.visible_rows_between(0, 10), 6);
        assert_eq!(m.visible_line_count(10), 6);
    }

    #[test]
    fn nested_folds_resolve_to_the_outermost_visible_head() {
        // Outer: head 0, hidden 1..=9. Inner: head 3, hidden 4..=6.
        let m = map(&[fold(0, 9), fold(3, 6)]);
        assert_eq!(m.visible_head_of(5), 0, "inner head 3 is itself hidden");
        assert_eq!(m.visible_head_of(3), 0);
        assert!(m.is_head(0));
        assert!(!m.is_head(3), "a hidden head renders no row");
        assert_eq!(m.next_visible(0), 10);
    }

    #[test]
    fn shared_head_folds_merge_to_the_longer_reach() {
        // Two folds on head 4: one hides 5..=6, the other 5..=9.
        let m = map(&[fold(4, 6), fold(4, 9)]);
        assert_eq!(m.visible_head_of(9), 4);
        assert_eq!(m.next_visible(4), 10);
        assert_eq!(m.visible_position(9, 9 * 8 + 3), fold(4, 6).start);
    }

    #[test]
    fn crossing_folds_project_to_the_first_visible_head() {
        // Round-3 F2: A hides 1..=3 (head 0); B is headed on line 2 and
        // hides 3..=5. A point on line 5 is directly inside only B, but
        // B's head is hidden by A — it must resolve to A's head.
        let m = map(&[fold(0, 3), fold(2, 5)]);
        assert!(m.is_hidden(5));
        assert_eq!(m.visible_head_of(5), 0);
        assert_eq!(
            m.visible_position(5, 5 * 8 + 4),
            fold(0, 3).start,
            "never B's still-hidden range.start"
        );
        assert_eq!(m.next_visible(0), 6);
        assert!(!m.is_head(2), "B's head is hidden, so it heads nothing");
    }

    #[test]
    fn adjacent_folds_merge_because_the_later_head_is_hidden() {
        // A hides 1..=3 (head 0); B is headed on line 3 (hidden by A)
        // and hides 4..=5. Lines 1..=5 collapse under head 0.
        let m = map(&[fold(0, 3), fold(3, 5)]);
        for line in 1..=5 {
            assert_eq!(m.visible_head_of(line), 0, "line {line}");
        }
        assert_eq!(m.next_visible(0), 6);
    }

    #[test]
    fn a_visible_line_between_two_folds_keeps_them_separate() {
        // A hides 1..=3 (head 0); B hides 5..=6 (head 4). Line 4 stays
        // visible, so the components do not merge.
        let m = map(&[fold(0, 3), fold(4, 6)]);
        assert!(!m.is_hidden(4));
        assert_eq!(m.visible_head_of(3), 0);
        assert_eq!(m.visible_head_of(6), 4);
        assert_eq!(m.next_visible(0), 4);
        assert_eq!(m.next_visible(4), 7);
    }

    #[test]
    fn visible_position_leaves_a_visible_byte_alone() {
        let m = map(&[fold(2, 6)]);
        assert_eq!(m.visible_position(7, 7 * 8 + 2), 7 * 8 + 2);
    }

    #[test]
    fn clamp_view_top_goes_backward_to_the_head() {
        let m = map(&[fold(2, 6)]);
        assert_eq!(m.clamp_view_top(5), 2);
        assert_eq!(m.clamp_view_top(2), 2);
        assert_eq!(m.clamp_view_top(7), 7);
    }

    #[test]
    fn visible_distance_is_symmetric_and_skips_folds() {
        let m = map(&[fold(2, 6)]);
        // Visible order: 0,1,2,7,8 — line 8 is 4 visible steps from 0.
        assert_eq!(m.visible_distance(0, 8), 4);
        assert_eq!(m.visible_distance(8, 0), 4);
        assert_eq!(m.visible_distance(2, 7), 1);
    }

    #[test]
    fn nth_visible_walks_forward_and_back_over_folds() {
        let m = map(&[fold(2, 6)]);
        assert_eq!(m.nth_visible_from(0, 3), 7);
        assert_eq!(m.nth_visible_back(8, 4), 0);
        // A hidden origin normalizes to its head first.
        assert_eq!(m.nth_visible_from(5, 1), 7);
        assert_eq!(m.nth_visible_back(5, 1), 1);
    }

    #[test]
    fn build_drops_a_fold_that_no_longer_spans_a_line() {
        // start and end inside one line: nothing to hide.
        let degenerate = ByteRange { start: 10, end: 12 };
        assert!(map(&[degenerate]).is_identity());
    }
}
