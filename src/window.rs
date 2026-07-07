// window.rs --- Window tree, splits, and per-window state (T M2.8).

//! A *window* displays a buffer in a region of the cell grid. The
//! editor maintains a tree of windows: leaves render a single
//! buffer; splits divide their parent's area horizontally
//! (children stack vertically) or vertically (children sit
//! side-by-side). The active window is identified by its
//! [`WindowId`]; key events route to it.
//!
//! # Per-window state
//!
//! [`Window`] owns the cursor, scroll position, sticky goal column,
//! and a [`TextView`] specific to its buffer. Two windows on the
//! same buffer have independent cursors but share buffer content;
//! when the buffer mutates, both windows' text views are notified
//! by [`crate::editor_core::EditorCore`].
//!
//! # Layout
//!
//! [`Layout::compute`] walks the tree given the available [`Rect`]
//! and produces a per-window viewport rectangle. Splits are
//! proportional with integer weights, so a SIGWINCH-driven resize
//! is automatic: the new terminal area is just fed back through
//! `compute` --- ratios are intrinsic to the tree, not derived from
//! the previous absolute sizes.
//!
//! # Threading
//!
//! Single-threaded, like the rest of the editor core. Lives inside
//! [`crate::editor_core::EditorCore`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::buffer::BufferId;
use crate::cell::{CellCoord, CellSize};
use crate::rope::Position;
use crate::text_view::TextView;
use crate::view::View;

// ---------------------------------------------------------------------------
// WindowId
// ---------------------------------------------------------------------------

/// Stable identifier for a window. Allocated in monotonic order;
/// reusing a freed id is not currently supported (window-close just
/// drops the id permanently).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(u64);

impl WindowId {
    /// Mint a new id. Allocates from a process-wide counter; ids are
    /// unique across the lifetime of the process.
    #[must_use]
    pub fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// The raw id, useful for debug formatting and tests.
    #[must_use]
    pub fn raw(self) -> u64 {
        self.0
    }
}

// ---------------------------------------------------------------------------
// Rect
// ---------------------------------------------------------------------------

/// Rectangular region of the cell grid (rows × cols at a given
/// origin). Used for window viewports.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Rect {
    /// Top-left corner.
    pub origin: CellCoord,
    /// Width and height.
    pub size: CellSize,
}

impl Rect {
    /// New rect at `(row, col)` of `(rows, cols)` size.
    #[must_use]
    pub fn new(row: u32, col: u32, rows: u32, cols: u32) -> Self {
        Self {
            origin: CellCoord::new(row, col),
            size: CellSize::new(rows, cols),
        }
    }

    /// True iff the rect has positive area.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.size.rows == 0 || self.size.cols == 0
    }
}

// ---------------------------------------------------------------------------
// Orientation
// ---------------------------------------------------------------------------

/// Which axis a split divides.
///
/// Naming follows Emacs's convention, which can be confusing: a
/// **horizontal** split produces children stacked top-to-bottom (the
/// dividing line is horizontal). A **vertical** split produces
/// children side-by-side (the dividing line is vertical).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Orientation {
    /// Children stack top-to-bottom; rows are divided.
    Horizontal,
    /// Children sit side-by-side; columns are divided.
    Vertical,
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

/// An active selection in a window.
///
/// The region runs from `anchor` to the window's `cursor`. Either
/// endpoint can be the lower bound; [`Selection::range`] returns them
/// in canonical (lo, hi) order. A `Selection` with `anchor == cursor`
/// is *active but empty*: useful for "shift-click extends" semantics.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Where the selection began (mouse-down position, typically).
    pub anchor: Position,
}

/// Line-number display mode for a window's left gutter (UX gutter arc).
/// `Off` reserves no gutter at all — text starts at column 0, and every
/// coordinate is unchanged (the default, matching the Emacs tradition).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum LineNumberMode {
    /// No gutter; zero layout change.
    #[default]
    Off,
    /// Absolute 1-based line numbers, right-aligned in the gutter.
    Absolute,
    /// Distance from the cursor line (the cursor line shows `0`).
    Relative,
    /// Like `Relative`, but the cursor line shows its absolute 1-based
    /// number instead of `0` (Vim `number` + `relativenumber`).
    Hybrid,
}

impl LineNumberMode {
    /// Whether this mode reserves a gutter at all (everything but `Off`).
    #[must_use]
    pub fn is_on(self) -> bool {
        !matches!(self, Self::Off)
    }

    /// The displayed 0-or-1-based number for a buffer `line` given the
    /// cursor's buffer line, or `None` in `Off`. `Relative`/`Hybrid` depend
    /// on `cursor_line`; `Absolute` ignores it.
    #[must_use]
    pub fn number_for(self, line: usize, cursor_line: usize) -> Option<usize> {
        match self {
            Self::Off => None,
            Self::Absolute => Some(line + 1),
            Self::Relative => Some(line.abs_diff(cursor_line)),
            Self::Hybrid => Some(if line == cursor_line {
                line + 1
            } else {
                line.abs_diff(cursor_line)
            }),
        }
    }
}

/// Cells of horizontal padding the line-number gutter adds around the
/// digit field: a leading and a trailing blank, so `gutter_w = digits +
/// PAD` (Q#UX3). Kept as a named constant so both frontends can share the
/// convention (Q#UX7). `u32` to match the cell-grid column type.
pub const LINE_NUMBER_GUTTER_PAD: u32 = 2;

/// Number of decimal digits in `n` (for `n >= 1`). Allocation-free.
#[must_use]
pub fn decimal_digits(mut n: usize) -> u32 {
    let mut d = 1u32;
    while n >= 10 {
        n /= 10;
        d += 1;
    }
    d
}

/// One leaf of the window tree: a buffer plus per-window state.
pub struct Window {
    /// Unique identifier.
    pub id: WindowId,
    /// Buffer displayed in this window.
    pub buffer_id: BufferId,
    /// Plain-text view of the buffer for this window. Each window
    /// owns its own; line offsets are independent.
    pub text_view: TextView,
    /// Composition stack: views that render after `text_view` into the
    /// same cell grid (T M2.9). See [`crate::view::View`] for the
    /// composition contract. Stored as trait objects so user-defined
    /// view kinds can join the stack via Lua in later milestones.
    pub overlays: Vec<Box<dyn View>>,
    /// Byte position of this window's cursor.
    pub cursor: Position,
    /// Active region, if any (T M2.12). Mouse drag sets the anchor
    /// at mouse-down and updates the cursor as the mouse moves; the
    /// selection lives until cleared (mouse-up with no movement, a
    /// keystroke that cancels, or a region-aware command consumes it).
    pub selection: Option<Selection>,
    /// First buffer line shown at the top of this window's viewport.
    pub view_top: usize,
    /// Sticky display column for vertical motion.
    pub goal_col: Option<u32>,
    /// Number of text rows that fit in this window's viewport at last
    /// render. Updated by the renderer; consumed by `cursor.page-down`
    /// / `cursor.page-up`. `0` until the first render lands.
    pub last_visible_rows: u32,
    /// Line-number gutter mode for this window (UX gutter arc). `Off` by
    /// default → no gutter, no coordinate change.
    pub line_numbers: LineNumberMode,
}

impl Window {
    /// New window for `buffer_id`, with an attached `text_view` and
    /// cursor at the start.
    #[must_use]
    pub fn new(id: WindowId, buffer_id: BufferId, text_view: TextView) -> Self {
        Self {
            id,
            buffer_id,
            text_view,
            overlays: Vec::new(),
            cursor: 0,
            selection: None,
            view_top: 0,
            goal_col: None,
            last_visible_rows: 0,
            line_numbers: LineNumberMode::Off,
        }
    }

    /// Width in cells this window's line-number gutter occupies, or `0`
    /// when disabled (UX gutter arc, Q#UX3). `digits(line_count) + PAD`;
    /// the renderer caps this against the window width and applies it as a
    /// left offset to the text area. Every gutter coordinate-math site
    /// reads this one function so the width stays consistent.
    #[must_use]
    pub fn gutter_width(&self) -> u32 {
        // Every on-mode reserves the same width — sized for the largest
        // number any mode could show (the absolute line count, which
        // bounds relative distances and hybrid's cursor-line number). A
        // fixed width keeps the text from jittering as the cursor moves in
        // relative/hybrid modes.
        if self.line_numbers.is_on() {
            decimal_digits(self.text_view.line_count().max(1)) + LINE_NUMBER_GUTTER_PAD
        } else {
            0
        }
    }

    /// Push an overlay onto the composition stack. Overlays render
    /// after `text_view`, in the order they were pushed.
    pub fn push_overlay(&mut self, view: Box<dyn View>) {
        self.overlays.push(view);
    }

    /// Stable kind identifiers of every overlay on this window, in
    /// push order. Test seam used by `pmacs.window._overlay_kinds()`
    /// to verify that a specific overlay type actually attached
    /// (e.g. a code-format prompt result buffer expects a
    /// `"syntax-highlight"` overlay after the wire-up step).
    pub fn overlay_kinds(&self) -> Vec<&'static str> {
        self.overlays.iter().map(|v| v.kind()).collect()
    }

    /// Active region as `(lo, hi)` byte positions, if any. Returns
    /// `None` when no selection is active or when the selection is
    /// empty (anchor == cursor).
    #[must_use]
    pub fn region(&self) -> Option<(Position, Position)> {
        let sel = self.selection?;
        match sel.anchor.cmp(&self.cursor) {
            std::cmp::Ordering::Less => Some((sel.anchor, self.cursor)),
            std::cmp::Ordering::Greater => Some((self.cursor, sel.anchor)),
            std::cmp::Ordering::Equal => None,
        }
    }
}

// ---------------------------------------------------------------------------
// LayoutNode + Layout
// ---------------------------------------------------------------------------

/// One node in the window tree.
#[derive(Clone, Debug)]
pub enum LayoutNode {
    /// A single window occupying its parent's area.
    Leaf(WindowId),
    /// A split with proportional integer weights. The weights vector
    /// always has the same length as `children`; weights of `0` are
    /// treated as `1` (defensive against empty weight specs from
    /// Lua).
    Split {
        /// Direction of the dividing line.
        orientation: Orientation,
        /// Per-child weights. Sum of weights determines proportional
        /// allocation across the parent's primary axis.
        weights: Vec<u32>,
        /// Children in display order (left→right or top→bottom).
        children: Vec<LayoutNode>,
    },
}

/// Window tree + active focus.
#[derive(Clone, Debug)]
pub struct Layout {
    /// Root of the tree.
    pub root: LayoutNode,
}

/// T M10.8 — one attached frontend's view of the editor.
///
/// Per-frontend state for multi-frontend operation: the split tree
/// the frontend sees and which window within it is focused.
/// `WindowId`s are globally unique across all frontends — the
/// `EditorCore::windows` flat map holds every window, and each
/// frontend's `FrontendView` references a subset via its `Layout`.
///
/// The buffers themselves remain shared in `EditorCore::registry` —
/// two frontends with windows onto the same `BufferId` see the same
/// content but each window owns its own cursor / `view_top` / `goal_col`.
#[derive(Clone, Debug)]
pub struct FrontendView {
    /// Window tree visible to this frontend.
    pub layout: Layout,
    /// Focused window within `layout`. Always a `WindowId` that
    /// `layout` references (invariant: `layout.iter_ids()` contains
    /// `active`).
    pub active: WindowId,
}

impl Layout {
    /// A trivial single-window layout.
    #[must_use]
    pub fn single(window: WindowId) -> Self {
        Self {
            root: LayoutNode::Leaf(window),
        }
    }

    /// Walk the tree and assign each leaf a viewport rectangle.
    ///
    /// Splits divide proportionally according to their weights. If a
    /// child's allocated extent is `0` (terminal too small for the
    /// split), that child receives an empty rect, and renderers must
    /// skip it.
    #[must_use]
    pub fn compute(&self, area: Rect) -> HashMap<WindowId, Rect> {
        let mut out = HashMap::new();
        compute_node(&self.root, area, &mut out);
        out
    }

    /// All [`WindowId`]s in left→right / top→bottom order.
    #[must_use]
    pub fn iter_ids(&self) -> Vec<WindowId> {
        let mut out = Vec::new();
        collect_ids(&self.root, &mut out);
        out
    }

    /// Replace the leaf currently displaying `target` with a split.
    /// Returns `true` if the leaf was found and replaced.
    pub fn split_window(
        &mut self,
        target: WindowId,
        orientation: Orientation,
        new_window: WindowId,
    ) -> bool {
        split_node(&mut self.root, target, orientation, new_window)
    }

    /// Remove the leaf for `target`. Returns `true` if removed.
    /// Collapses single-child splits in the cleanup pass.
    pub fn close_window(&mut self, target: WindowId) -> bool {
        let removed = remove_leaf(&mut self.root, target).is_some();
        if removed {
            collapse_single_child_splits(&mut self.root);
        }
        removed
    }

    /// Collapse the layout to just `keep`. Returns `false` if `keep`
    /// is not a leaf in the tree.
    pub fn keep_only(&mut self, keep: WindowId) -> bool {
        if !self.iter_ids().contains(&keep) {
            return false;
        }
        self.root = LayoutNode::Leaf(keep);
        true
    }

    /// Step focus from `current` to the next window in iteration
    /// order, wrapping around. Returns the new focus, or `current`
    /// if the layout has only one window.
    #[must_use]
    pub fn focus_next(&self, current: WindowId) -> WindowId {
        let ids = self.iter_ids();
        match ids.iter().position(|&id| id == current) {
            Some(i) => ids[(i + 1) % ids.len()],
            None => *ids.first().unwrap_or(&current),
        }
    }

    /// Step focus to the previous window.
    #[must_use]
    pub fn focus_prev(&self, current: WindowId) -> WindowId {
        let ids = self.iter_ids();
        match ids.iter().position(|&id| id == current) {
            Some(i) => ids[(i + ids.len() - 1) % ids.len()],
            None => *ids.first().unwrap_or(&current),
        }
    }
}

fn compute_node(node: &LayoutNode, area: Rect, out: &mut HashMap<WindowId, Rect>) {
    match node {
        LayoutNode::Leaf(id) => {
            out.insert(*id, area);
        }
        LayoutNode::Split {
            orientation,
            weights,
            children,
        } => {
            let total: u32 = weights.iter().map(|w| (*w).max(1)).sum();
            let primary = match orientation {
                Orientation::Horizontal => area.size.rows,
                Orientation::Vertical => area.size.cols,
            };
            let mut cursor: u32 = 0;
            for (i, child) in children.iter().enumerate() {
                let w = weights.get(i).copied().unwrap_or(1).max(1);
                let extent = if i + 1 == children.len() {
                    primary - cursor
                } else {
                    primary * w / total
                };
                let child_area = match orientation {
                    Orientation::Horizontal => Rect {
                        origin: CellCoord::new(area.origin.row + cursor, area.origin.col),
                        size: CellSize::new(extent, area.size.cols),
                    },
                    Orientation::Vertical => Rect {
                        origin: CellCoord::new(area.origin.row, area.origin.col + cursor),
                        size: CellSize::new(area.size.rows, extent),
                    },
                };
                compute_node(child, child_area, out);
                cursor += extent;
            }
        }
    }
}

fn collect_ids(node: &LayoutNode, out: &mut Vec<WindowId>) {
    match node {
        LayoutNode::Leaf(id) => out.push(*id),
        LayoutNode::Split { children, .. } => {
            for c in children {
                collect_ids(c, out);
            }
        }
    }
}

fn split_node(
    node: &mut LayoutNode,
    target: WindowId,
    orientation: Orientation,
    new_window: WindowId,
) -> bool {
    match node {
        LayoutNode::Leaf(id) if *id == target => {
            let original = *id;
            *node = LayoutNode::Split {
                orientation,
                weights: vec![1, 1],
                children: vec![LayoutNode::Leaf(original), LayoutNode::Leaf(new_window)],
            };
            true
        }
        LayoutNode::Leaf(_) => false,
        LayoutNode::Split { children, .. } => children
            .iter_mut()
            .any(|c| split_node(c, target, orientation, new_window)),
    }
}

fn remove_leaf(node: &mut LayoutNode, target: WindowId) -> Option<()> {
    match node {
        LayoutNode::Leaf(_) => None,
        LayoutNode::Split {
            children, weights, ..
        } => {
            // Direct child match?
            if let Some(idx) = children
                .iter()
                .position(|c| matches!(c, LayoutNode::Leaf(id) if *id == target))
            {
                children.remove(idx);
                if idx < weights.len() {
                    weights.remove(idx);
                }
                return Some(());
            }
            // Recurse into split children.
            for c in children.iter_mut() {
                if remove_leaf(c, target).is_some() {
                    return Some(());
                }
            }
            None
        }
    }
}

fn collapse_single_child_splits(node: &mut LayoutNode) {
    if let LayoutNode::Split { children, .. } = node {
        for c in children.iter_mut() {
            collapse_single_child_splits(c);
        }
        if children.len() == 1 {
            let only = children.remove(0);
            *node = only;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_number_mode_number_for_covers_all_modes() {
        use LineNumberMode::{Absolute, Hybrid, Off, Relative};
        // Cursor on buffer line 5 (0-based). Lines 3 and 7 are 2 away.
        assert_eq!(Off.number_for(3, 5), None);
        // Absolute ignores the cursor line: 1-based.
        assert_eq!(Absolute.number_for(3, 5), Some(4));
        assert_eq!(Absolute.number_for(5, 5), Some(6));
        // Relative: distance from the cursor line; cursor line is 0.
        assert_eq!(Relative.number_for(3, 5), Some(2));
        assert_eq!(Relative.number_for(7, 5), Some(2));
        assert_eq!(Relative.number_for(5, 5), Some(0));
        // Hybrid: absolute on the cursor line, relative elsewhere.
        assert_eq!(Hybrid.number_for(5, 5), Some(6));
        assert_eq!(Hybrid.number_for(3, 5), Some(2));
        // Every on-mode reserves a gutter; Off does not.
        assert!(!Off.is_on());
        assert!(Absolute.is_on() && Relative.is_on() && Hybrid.is_on());
    }

    #[test]
    fn decimal_digits_counts_correctly() {
        assert_eq!(decimal_digits(1), 1);
        assert_eq!(decimal_digits(9), 1);
        assert_eq!(decimal_digits(10), 2);
        assert_eq!(decimal_digits(99), 2);
        assert_eq!(decimal_digits(100), 3);
        assert_eq!(decimal_digits(1000), 4);
        // A 6-digit file → 6 digits + PAD gutter.
        assert_eq!(decimal_digits(123_456), 6);
    }

    fn id() -> WindowId {
        WindowId::next()
    }

    fn rect_24x80() -> Rect {
        Rect::new(0, 0, 24, 80)
    }

    #[test]
    fn single_window_takes_full_area() {
        let w = id();
        let layout = Layout::single(w);
        let placements = layout.compute(rect_24x80());
        assert_eq!(placements.get(&w), Some(&rect_24x80()));
    }

    #[test]
    fn vertical_split_divides_columns() {
        let a = id();
        let b = id();
        let mut layout = Layout::single(a);
        assert!(layout.split_window(a, Orientation::Vertical, b));
        let placements = layout.compute(rect_24x80());
        let ra = placements[&a];
        let rb = placements[&b];
        assert_eq!(ra.size.rows, 24);
        assert_eq!(rb.size.rows, 24);
        assert_eq!(ra.size.cols + rb.size.cols, 80);
        assert_eq!(ra.origin.col, 0);
        assert_eq!(rb.origin.col, ra.size.cols);
    }

    #[test]
    fn horizontal_split_divides_rows() {
        let a = id();
        let b = id();
        let mut layout = Layout::single(a);
        assert!(layout.split_window(a, Orientation::Horizontal, b));
        let placements = layout.compute(rect_24x80());
        let ra = placements[&a];
        let rb = placements[&b];
        assert_eq!(ra.size.cols, 80);
        assert_eq!(rb.size.cols, 80);
        assert_eq!(ra.size.rows + rb.size.rows, 24);
    }

    #[test]
    fn ratios_are_preserved_under_resize() {
        // 2:1 horizontal split. Resizing should preserve ratio.
        let a = id();
        let b = id();
        let mut layout = Layout::single(a);
        layout.split_window(a, Orientation::Vertical, b);
        if let LayoutNode::Split { weights, .. } = &mut layout.root {
            *weights = vec![2, 1];
        } else {
            panic!("expected split");
        }
        let p1 = layout.compute(Rect::new(0, 0, 24, 90));
        assert_eq!(p1[&a].size.cols, 60);
        assert_eq!(p1[&b].size.cols, 30);
        // Resize down by 1/3.
        let p2 = layout.compute(Rect::new(0, 0, 24, 60));
        assert_eq!(p2[&a].size.cols, 40);
        assert_eq!(p2[&b].size.cols, 20);
        // Resize wide.
        let p3 = layout.compute(Rect::new(0, 0, 24, 300));
        assert_eq!(p3[&a].size.cols, 200);
        assert_eq!(p3[&b].size.cols, 100);
    }

    #[test]
    fn eight_splits_render_in_distinct_rects() {
        // Build an 8-way layout: vertical-of-4 over horizontal-of-2,
        // achieved by 3 vertical splits then 1 horizontal split per
        // column. Verify all 8 leaves get unique non-empty rects.
        let initial = id();
        let mut layout = Layout::single(initial);
        let mut leaves = vec![initial];
        // Split each existing leaf vertically until we have 4.
        for _ in 0..3 {
            let pivot = *leaves.last().unwrap();
            let new = id();
            assert!(layout.split_window(pivot, Orientation::Vertical, new));
            leaves.push(new);
        }
        // Now horizontally split each leaf.
        let mut more = Vec::new();
        for &l in &leaves {
            let new = id();
            assert!(layout.split_window(l, Orientation::Horizontal, new));
            more.push(new);
        }
        leaves.extend(more);
        assert_eq!(leaves.len(), 8);
        let placements = layout.compute(rect_24x80());
        assert_eq!(placements.len(), 8);
        // Every rect must be non-empty (terminal large enough).
        for id in &leaves {
            let r = placements[id];
            assert!(!r.is_empty(), "rect for {id:?} was empty");
        }
        // No two rects overlap (compare pairwise).
        let rects: Vec<_> = leaves.iter().map(|id| placements[id]).collect();
        for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                assert!(!rects_overlap(&rects[i], &rects[j]));
            }
        }
    }

    fn rects_overlap(a: &Rect, b: &Rect) -> bool {
        let a_r0 = a.origin.row;
        let a_r1 = a.origin.row + a.size.rows;
        let a_c0 = a.origin.col;
        let a_c1 = a.origin.col + a.size.cols;
        let b_r0 = b.origin.row;
        let b_r1 = b.origin.row + b.size.rows;
        let b_c0 = b.origin.col;
        let b_c1 = b.origin.col + b.size.cols;
        a_r0 < b_r1 && b_r0 < a_r1 && a_c0 < b_c1 && b_c0 < a_c1
    }

    #[test]
    fn focus_next_walks_in_iteration_order() {
        let a = id();
        let b = id();
        let c = id();
        let mut layout = Layout::single(a);
        layout.split_window(a, Orientation::Vertical, b);
        layout.split_window(b, Orientation::Horizontal, c);
        let order = layout.iter_ids();
        assert_eq!(order.len(), 3);
        let mut cur = order[0];
        for expected in &[order[1], order[2], order[0], order[1]] {
            cur = layout.focus_next(cur);
            assert_eq!(&cur, expected);
        }
    }

    #[test]
    fn focus_prev_is_inverse_of_focus_next() {
        let a = id();
        let b = id();
        let c = id();
        let mut layout = Layout::single(a);
        layout.split_window(a, Orientation::Vertical, b);
        layout.split_window(b, Orientation::Horizontal, c);
        let order = layout.iter_ids();
        let mut cur = order[0];
        cur = layout.focus_next(cur);
        cur = layout.focus_prev(cur);
        assert_eq!(cur, order[0]);
    }

    #[test]
    fn close_window_collapses_single_child_split() {
        let a = id();
        let b = id();
        let mut layout = Layout::single(a);
        layout.split_window(a, Orientation::Vertical, b);
        assert_eq!(layout.iter_ids().len(), 2);
        assert!(layout.close_window(b));
        assert_eq!(layout.iter_ids(), vec![a]);
        assert!(matches!(layout.root, LayoutNode::Leaf(_)));
    }

    #[test]
    fn keep_only_collapses_to_target() {
        let a = id();
        let b = id();
        let c = id();
        let mut layout = Layout::single(a);
        layout.split_window(a, Orientation::Vertical, b);
        layout.split_window(a, Orientation::Horizontal, c);
        assert!(layout.keep_only(c));
        assert_eq!(layout.iter_ids(), vec![c]);
    }

    #[test]
    fn keep_only_returns_false_for_unknown_id() {
        let a = id();
        let bogus = id();
        let mut layout = Layout::single(a);
        assert!(!layout.keep_only(bogus));
        assert_eq!(layout.iter_ids(), vec![a]);
    }
}
