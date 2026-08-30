// view.rs --- The View trait. Polymorphic interpretive layer over a buffer.

//! Views: the polymorphic interpretive layer attached to a buffer.
//!
//! Implements the view contract from spec §3.3 (Views). A view subscribes
//! to edits, maintains derived state, contributes to rendering, and may
//! intercept edits before they reach the rope.
//!
//! # Re-entry by construction
//!
//! Views never hold a `&Buffer` or `&mut Buffer` of their own. The buffer
//! passes itself into every callback (per spec §2.6, Checkpoint 6). While a
//! callback runs, the buffer's `&mut self` is held by the buffer's own
//! stack frame; no aliasing reference exists for the view to use, so a view
//! cannot recursively call `buffer.apply_edit` from inside its own
//! `on_edit` --- the borrow checker rejects it.
//!
//! # Composition (T M2.9)
//!
//! A window holds one base [`crate::text_view::TextView`] plus a stack
//! of overlay views (`window.overlays`). The frontend renders them in
//! a deterministic order:
//!
//! 1. The base text view runs first. It clears every cell in the
//!    window's viewport and paints buffer text + default style.
//! 2. Overlays then run in attach order (FIFO). Each overlay
//!    observes the cells already written and may read, mutate in
//!    place, or replace any cell inside the viewport.
//! 3. At any cell two views both touch, the later writer wins on the
//!    fields it chooses to write. Overlays are free to read existing
//!    fields and merge — see [`crate::overlay::StyleSpanOverlay`] for
//!    a style-merging overlay and [`crate::overlay::VirtualCellOverlay`]
//!    for a whole-cell-replacement overlay.
//!
//! Cursor placement and scrolling use the base text view's
//! [`View::pos_to_display`] only; overlays do not move the cursor.
//! Composition adds well under 10% overhead over a single-view render
//! when overlays touch only the cells they declare (see
//! `composition_overhead_under_ten_percent` in `editor.rs`).

use crate::buffer::{Buffer, BufferError, BufferId, EditOp};
use crate::cell::{CellCoord, CellGrid, CellSize};
use crate::fold_view::VisibleLineMap;
use crate::rope::{Edit, Position};

// ---------------------------------------------------------------------------
// InterceptContext (T M7.4)
// ---------------------------------------------------------------------------

/// Snapshot of the buffer's identity and shape, passed to
/// [`View::intercept_edit`] in lieu of a `&Buffer` reference.
///
/// # Why a snapshot, not a `&Buffer`
///
/// Pre-M7.4, `intercept_edit` received `&Buffer`. The Lua-bindings
/// layer held the registry's `RefCell::borrow_mut` for the full
/// duration of the call, so an intercept body that re-entered
/// `pmacs.buffer.X` synchronously hit a recursive-borrow error
/// (`BindingError::Reentrant`); the M6.10 audit fix surfaced this as
/// a typed error rather than a panic but did not enable the re-entry.
///
/// M7.4 splits the edit flow into three phases. Phase 2 runs the
/// intercept chain with the registry borrow released, so an intercept
/// body may call back into `pmacs.buffer.X` on any buffer. The cost:
/// the intercept can no longer hold a `&Buffer` (the buffer might be
/// mutated by the re-entrant call). Instead, we hand it an
/// `InterceptContext` snapshot taken at the start of the edit; the
/// fields cover every read the intercept needs (`buf_id` for routing,
/// `buf_len` for clamping positions, `buf_name` for diagnostic
/// messages, `revision` for "did this snapshot drift?" checks).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterceptContext {
    /// The buffer's identifier. Stable across the edit.
    pub buf_id: BufferId,
    /// Buffer length in bytes at the moment the snapshot was taken.
    pub buf_len: u64,
    /// Buffer name at the moment the snapshot was taken.
    pub buf_name: String,
    /// Buffer revision at the moment the snapshot was taken. Useful
    /// for re-entrant cross-buffer edits to detect whether the parent
    /// buffer was mutated during their lifetime.
    pub revision: u64,
}

impl InterceptContext {
    /// Build a context snapshot from a buffer reference.
    #[must_use]
    pub fn snapshot(buf: &Buffer) -> Self {
        Self {
            buf_id: buf.id(),
            buf_len: buf.len(),
            buf_name: buf.name().to_string(),
            revision: buf.revision(),
        }
    }
}

// ---------------------------------------------------------------------------
// Coordinates
// ---------------------------------------------------------------------------

/// Coordinate in display space (row, col), measured in cells.
///
/// Distinct from [`CellCoord`] in name only at this stage, but kept separate
/// for documentation: `DisplayCoord` is a *content* coordinate (reported by
/// `pos_to_display`), `CellCoord` is a *grid* coordinate (where to draw).
/// They coincide for plain text but diverge once virtual lines, wrapping,
/// and inline expansions appear.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
pub struct DisplayCoord {
    /// 0-based **source line** index.
    ///
    /// Deliberately still the source line under wrapping, not a visual
    /// row. Redefining it would have broken every existing consumer —
    /// `overlay_paint`'s `row - view_top`, vertical motion's bounds
    /// check — with no compile error. Adding [`Self::sub_row`] beside it
    /// instead leaves those consumers *correct*, not merely findable.
    pub row: u32,
    /// Which visual row **within** `row`, when the line wraps.
    ///
    /// `0` for every unwrapped line and under
    /// [`WrapMode::Truncate`](crate::view::WrapMode::Truncate), which is
    /// what makes this field additive: code that has never heard of
    /// wrapping keeps computing the right answer.
    pub sub_row: u32,
    /// 0-based column, within the visual row named by `sub_row`.
    pub col: u32,
}

impl DisplayCoord {
    /// Construct a display coordinate on a line's first visual row.
    #[must_use]
    pub const fn new(row: u32, col: u32) -> Self {
        Self {
            row,
            sub_row: 0,
            col,
        }
    }

    /// Construct a display coordinate on a specific visual row of a
    /// wrapped line.
    #[must_use]
    pub const fn wrapped(row: u32, sub_row: u32, col: u32) -> Self {
        Self { row, sub_row, col }
    }
}

/// The layout facts a coordinate mapping needs, which the mapping
/// itself cannot know.
///
/// # Why this is a required parameter
///
/// `pos_to_display` took `(&self, buf, pos)` and had no notion of the
/// grid at all — so under wrapping it could not compute a visual row,
/// and `display_to_pos` could not invert one. Passing the missing
/// facts as a required argument is deliberate: it makes the compiler
/// enumerate every call site rather than leaving an audit to grep.
///
/// That is the opposite choice from [`DisplayCoord::sub_row`], and for
/// the opposite reason. Enforcement is possible on the way in, so it is
/// taken; it is not possible on the way out, so the output is made
/// correct-by-default instead.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
pub struct LayoutCtx {
    /// Width in cells of the content area the text wraps within.
    ///
    /// `0` means "not rendered yet" — the same convention
    /// `Window::last_visible_rows` uses — and is treated as unwrapped,
    /// since a viewport with no columns has no rows to distinguish.
    pub cols: u32,
    /// The window's resolved wrap mode.
    pub wrap: WrapMode,
    /// First visible display column — the window's horizontal scroll
    /// offset (Stage 4, framing Q#HS7).
    ///
    /// **Stored unsnapped.** It is one per-window column, while "does
    /// this column bisect a wide glyph?" is a *per-line* question, so no
    /// single snapped value could be canonical for every visible line.
    /// Each line derives its own **effective edge** during the walk it
    /// already performs from column 0 (framing Q#HS7(c′)).
    ///
    /// Inert under [`WrapMode::Wrap`]: a wrapped line has nothing past
    /// the right edge to scroll toward, so the wrap path ignores this
    /// entirely and stays byte-identical to Stage 3.
    pub view_left: u32,
}

impl LayoutCtx {
    /// The identity context: no wrapping, width irrelevant.
    ///
    /// Every pre-wrap caller means this, and saying so explicitly is
    /// what makes those call sites readable as decisions rather than
    /// oversights.
    #[must_use]
    pub const fn truncated() -> Self {
        Self {
            cols: 0,
            wrap: WrapMode::Truncate,
            view_left: 0,
        }
    }

    /// Whether this context actually wraps.
    #[must_use]
    pub const fn wrapping(self) -> bool {
        matches!(self.wrap, WrapMode::Wrap) && self.cols > 0
    }

    /// The horizontal offset that actually applies.
    ///
    /// Always `0` when wrapping, which is what makes `view_left` inert
    /// under `wrap` **by construction** rather than by every caller
    /// remembering to check. A wrapped line has no content past the
    /// right edge, so a non-zero offset there could only hide text that
    /// nothing would ever scroll back to.
    #[must_use]
    pub const fn effective_left(self) -> u32 {
        if self.wrapping() { 0 } else { self.view_left }
    }
}

/// How a line wider than the viewport is shown --- the long-lines
/// stage, `docs/long-lines-framing.md`.
///
/// # Why the renderer is told, rather than asking
///
/// This is the *resolved* mode, not the setting's name. `ui.line-wrap`
/// is **buffer-local** (framing Q#LL2), and the config registry has no
/// ambient current buffer by design (`config_registry.rs`, Q#CR4) — a
/// caller wanting buffer-aware behavior must pass the `BufferId`. The
/// render driver holds both the registry and the buffer, resolves once
/// per window per frame, and puts the answer here. Views stay
/// config-agnostic, exactly as they do for folds.
///
/// # `Truncate` is the identity case, deliberately
///
/// Every behavior predating this type is `Truncate`, and it must stay
/// byte-identical under it — which is what lets the wrap work be
/// verified against the existing suite rather than against new
/// assertions.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Default, Hash)]
pub enum WrapMode {
    /// One source line per row; the remainder is clipped at the right
    /// edge. What every pre-Stage-3 caller did.
    #[default]
    Truncate,
    /// A line longer than the viewport continues on the following rows.
    ///
    /// Character wrap, not word wrap (framing Q#LL5): it matches
    /// Emacs's default, and it is the only break rule both frontends
    /// can implement identically without pulling UAX #14 into the grid.
    Wrap,
}

/// What to render and where.
///
/// The frontend computes the viewport (which buffer range maps to which
/// cell origin) and hands it to the view. The view fills cells inside that
/// origin/size window.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Viewport<'a> {
    /// First byte in the buffer to consider rendering.
    pub buffer_start: Position,
    /// One past the last byte to consider.
    pub buffer_end: Position,
    /// Top-left cell in the grid where rendering should start.
    pub cell_origin: CellCoord,
    /// Number of cells the viewport occupies.
    pub cell_size: CellSize,
    /// Width of the line-number gutter reserved to the *left* of
    /// `cell_origin` (UX gutter arc). `0` when no gutter. Overlays that
    /// want to draw in the gutter (e.g. the diagnostic sign) reach it at
    /// `cell_origin.col - gutter_w`; overlays that only touch the text area
    /// ignore it (the origin is already shifted past the gutter).
    pub gutter_w: u32,
    /// The rendering window's collapsed regions (Arc 6 Stage 2, Q#FD12),
    /// or `None` when this window's buffer has no folds — the unfolded
    /// path then stays byte-identical to the pre-folding renderer.
    ///
    /// Borrowed rather than owned so `Viewport` stays [`Copy`]: the frame
    /// builds **one map per rendered window** (never one per frame — a
    /// split may show different buffers) and hands the same shared
    /// reference to every painter of that window.
    pub folds: Option<&'a VisibleLineMap>,
    /// How lines wider than `cell_size.cols` are shown, already
    /// resolved for this window's buffer (see [`WrapMode`]).
    ///
    /// A required field rather than a defaulted one on purpose: every
    /// construction site has to state which behavior it means, so the
    /// pre-Stage-3 sites read as *deliberately* unwrapped rather than
    /// merely untouched.
    pub wrap: WrapMode,
    /// First visible display column (Stage 4). See
    /// [`LayoutCtx::view_left`]; required rather than defaulted for the
    /// same reason `wrap` is.
    pub view_left: u32,
}

impl Viewport<'_> {
    /// The horizontal offset that actually applies — `0` when wrapping,
    /// for the reason [`LayoutCtx::effective_left`] gives.
    #[must_use]
    pub const fn left_edge(&self) -> u32 {
        if matches!(self.wrap, WrapMode::Wrap) {
            0
        } else {
            self.view_left
        }
    }

    /// Clip a **line**-column range to what is on screen, returning
    /// **screen** columns — or `None` when none of it is visible.
    ///
    /// # Why every buffer-coordinate decorator must use this
    ///
    /// Stage 4 translated the base text walk and nothing else, which
    /// split the frame in half: at `view_left = 10` a glyph at source
    /// column 10 painted at screen column 0 while its syntax style,
    /// diagnostic underline, search wash and selection painted at screen
    /// column 10 — or vanished. Decorations drifted off the characters
    /// they describe, silently, and only once a window was scrolled.
    ///
    /// Every such site had the same two lines (`start_col.min(max_cols)`,
    /// `end_col.min(max_cols)`) — correct only while the left edge was
    /// pinned at zero. One helper replaces all of them so a future
    /// decorator inherits the translation instead of re-deriving it.
    ///
    /// **Five adopters**, and the count is the point: syntax/LSP
    /// styling, diagnostic underlines, search washes,
    /// [`crate::overlay::BufferStyleOverlay`], and the selection
    /// painter. The selection was nearly the exception — Stage 4's first
    /// version duplicated the rule there, justified by a width this
    /// painter supposedly needed and the viewport lacked. That was
    /// false: the render viewport's `cell_size.cols` is already
    /// `rect.size.cols - gutter_w`, and its origin already sits past the
    /// gutter. A canonical rule with one honest exception is not
    /// canonical, so the exception went.
    ///
    /// **Not for [`crate::overlay::StyleSpanOverlay`] or
    /// [`crate::overlay::VirtualCellOverlay`]**: those are documented as
    /// viewport-relative, so their columns are already screen columns
    /// and translating them twice would be the mirror defect.
    #[must_use]
    pub fn visible_cols(&self, start_col: u32, end_col: u32) -> Option<(u32, u32)> {
        let left = self.left_edge();
        let right = left.saturating_add(self.cell_size.cols);
        let start = start_col.max(left);
        let end = end_col.min(right);
        // A range that begins off-screen left and reaches past the edge
        // is CLIPPED, not skipped — that is the selection defect this
        // returns `Some` for.
        (end > start).then(|| (start - left, end - left))
    }

    /// Row offset within this viewport for source `line`, given the
    /// viewport's first (visible) source line.
    ///
    /// `None` when `line` is above `start_line` or collapsed away — a
    /// hidden line simply has no row, so its painter skips it. Without a
    /// map this is the identity `line - start_line` every consumer used
    /// before folding.
    ///
    /// The result is monotonically non-decreasing in `line`, so a caller
    /// that bounds its walk with `row_offset >= rows` may still `break`.
    #[must_use]
    pub fn row_offset_of(self, start_line: usize, line: usize) -> Option<u32> {
        let raw = line.checked_sub(start_line)?;
        let rows = match self.folds {
            Some(map) if !map.is_identity() => {
                if map.is_hidden(line) {
                    return None;
                }
                map.visible_rows_between(start_line, line)
            }
            _ => raw,
        };
        u32::try_from(rows).ok()
    }

    /// The inverse of [`Self::row_offset_of`]: the source line rendered
    /// at `row_offset`, given the viewport's first (visible) source line.
    ///
    /// Painters that walk rows rather than spans (the syntax and LSP
    /// style views) use this; the result may exceed the buffer's line
    /// count, which those callers already bound.
    #[must_use]
    pub fn line_at_row_offset(self, start_line: usize, row_offset: u32) -> usize {
        match self.folds {
            Some(map) if !map.is_identity() => {
                map.nth_visible_from(start_line, row_offset as usize)
            }
            _ => start_line + row_offset as usize,
        }
    }
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Polymorphic interpretive layer attached to a buffer.
///
/// All callbacks receive `&Buffer` rather than `&mut Buffer`: views observe
/// and contribute, they do not mutate the buffer mid-callback. To produce
/// edits, a view returns a transformed [`EditOp`] from `intercept_edit` ---
/// or none of the above, and instead schedules a worker to do expensive work
/// asynchronously (M3+).
///
/// # Threading
///
/// Views run on the main thread. The buffer they attach to lives behind
/// `Rc<RefCell<BufferRegistry>>` and is not sent across threads (workers
/// receive rope snapshots, not buffer or view references). The trait
/// therefore does not require `Send`; this lets views hold types that
/// are not `Send` themselves --- notably `mlua` handles, used by the
/// M6.4 REPL package's `LuaInterceptView` to attach a Lua callback as
/// an intercept-edit chain entry.
pub trait View {
    /// Validate or transform a proposed edit before it reaches the rope.
    ///
    /// Returning the input as-is passes through; returning a different
    /// [`EditOp`] rewrites the edit (used by, e.g., a directory-listing
    /// view that translates filename edits into rename operations).
    /// Returning an error rejects the edit.
    ///
    /// The view receives a [`InterceptContext`] snapshot of the
    /// buffer's identity and shape at the moment the edit began
    /// (T M7.4). Intercept bodies needing live state on another
    /// buffer can re-enter `pmacs.buffer.X` (or the in-process
    /// equivalent); the registry borrow is released for the duration
    /// of this call.
    ///
    /// Default: pass through.
    fn intercept_edit<'a>(
        &mut self,
        _ctx: &InterceptContext,
        op: EditOp<'a>,
    ) -> Result<EditOp<'a>, BufferError> {
        Ok(op)
    }

    /// Called after every successful edit.
    ///
    /// The view updates its derived state. Must be cheap; expensive work
    /// is dispatched to a worker (M3+) and reflected back via a separate
    /// update path.
    ///
    /// Default: no-op.
    fn on_edit(&mut self, _buf: &Buffer, _edit: &Edit) -> Result<(), BufferError> {
        Ok(())
    }

    /// Render the buffer region named by `viewport` into `cells`.
    ///
    /// Pure-ish: same inputs produce the same outputs. The view writes only
    /// inside `viewport.cell_origin .. cell_origin + cell_size`; cells
    /// outside that window are left untouched (composition is the
    /// frontend's job).
    ///
    /// Default: no-op (used by views that participate in `on_edit` but not
    /// in rendering).
    fn render(&mut self, _buf: &Buffer, _viewport: Viewport<'_>, _cells: &mut CellGrid<'_>) {}

    /// Translate a buffer byte position to a display coordinate, if the
    /// view holds a meaningful mapping for that position.
    ///
    /// Default: returns `None` (view has no opinion).
    fn pos_to_display(
        &self,
        _buf: &Buffer,
        _pos: Position,
        _ctx: LayoutCtx,
    ) -> Option<DisplayCoord> {
        None
    }

    /// Translate a display coordinate back to a buffer byte position.
    ///
    /// Default: returns `None`.
    fn display_to_pos(
        &self,
        _buf: &Buffer,
        _coord: DisplayCoord,
        _ctx: LayoutCtx,
    ) -> Option<Position> {
        None
    }

    /// Stable identifier for this view's *kind*. Used by introspection
    /// seams (e.g. `pmacs.window._overlay_kinds()`) to verify that a
    /// specific overlay type actually attached after a wire-up step,
    /// without needing `Any` downcasts. The default `"unknown"` is
    /// fine for views that no test cares about; views that participate
    /// in cross-package wiring (syntax highlight, style overlays)
    /// should override.
    fn kind(&self) -> &'static str {
        "unknown"
    }

    /// Identity of the shared resource this overlay renders, if any
    /// (PR #113 round-6 findings 1 and 3). Two overlay instances
    /// backed by the same store report the same value, which lets a
    /// window attach a resource-backed overlay AT MOST once
    /// ([`crate::window::Window::ensure_overlay`]) and lets disposal
    /// remove every window copy. The default `None` opts out: such
    /// views are never deduplicated or bulk-removed.
    fn overlay_identity(&self) -> Option<usize> {
        None
    }

    /// A copy of this overlay for a freshly split window showing the
    /// same buffer (round-6 finding 1: a same-buffer split starts
    /// with an empty overlay list and fires no switch hook, so
    /// without this the new pane rendered unstyled). `None` (the
    /// default) means the view does not carry across splits;
    /// store-backed render overlays return a clone.
    fn clone_for_split(&self) -> Option<Box<dyn View>> {
        None
    }

    /// Retarget this overlay from `old_uri` to `new_uri` after a
    /// resource rename (dired Stage 2a, §5). Default: no-op — a view
    /// that renders nothing URI-keyed is unaffected.
    ///
    /// Mutates **in place**, so the overlay keeps its position in the
    /// window's composition order. That is the reason this is a trait
    /// hook rather than a remove-and-re-push at the call site: overlays
    /// are an ordered `Vec` merged in sequence, and re-pushing would
    /// move a diagnostic underline to the end of the stack. It is also
    /// how *passive* windows are reached at all — the Lua attach path
    /// (`pmacs.diag._attach_view`) can only touch the active window,
    /// while the sweep that drives this walks every window.
    fn rename_resource(&mut self, _old_uri: &str, _new_uri: &str) {}
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_coord_basics() {
        let c = DisplayCoord::new(3, 5);
        assert_eq!(c.row, 3);
        assert_eq!(c.col, 5);
    }

    #[test]
    fn viewport_is_copy() {
        // Compile-time assertion: Viewport must be Copy so the frontend can
        // hand the same descriptor to multiple views without ceremony. Arc 6
        // Stage 2 (Bet B7) keeps this true by borrowing the fold map — a
        // shared reference is itself `Copy`.
        fn assert_copy<T: Copy>() {}
        assert_copy::<Viewport<'_>>();
    }

    #[test]
    fn row_offset_without_folds_is_the_identity_map() {
        let vp = Viewport {
            buffer_start: 0,
            buffer_end: 0,
            cell_origin: CellCoord::new(0, 0),
            cell_size: CellSize::new(10, 10),
            gutter_w: 0,
            folds: None,
            wrap: WrapMode::Truncate,
            view_left: 0,
        };
        assert_eq!(vp.row_offset_of(4, 4), Some(0));
        assert_eq!(vp.row_offset_of(4, 9), Some(5));
        assert_eq!(vp.row_offset_of(4, 3), None, "above the viewport");
    }

    #[test]
    fn row_offset_skips_hidden_lines_and_compacts_rows() {
        // Fold heading line 1, hiding lines 2..=4 (8-byte lines).
        let map =
            VisibleLineMap::build(&[pmacs_protocol::ByteRange { start: 15, end: 39 }], |off| {
                (off / 8) as usize
            });
        let vp = Viewport {
            buffer_start: 0,
            buffer_end: 0,
            cell_origin: CellCoord::new(0, 0),
            cell_size: CellSize::new(10, 10),
            gutter_w: 0,
            folds: Some(&map),
            wrap: WrapMode::Truncate,
            view_left: 0,
        };
        assert_eq!(vp.row_offset_of(0, 1), Some(1), "the head keeps its row");
        assert_eq!(vp.row_offset_of(0, 3), None, "hidden lines have no row");
        assert_eq!(vp.row_offset_of(0, 5), Some(2), "rows below shift up");
    }

    /// C9: §4 of `docs/crdt-identity-undo-framing.md` enumerates every
    /// in-tree `on_edit` override so that the consumers of a
    /// version-only history `Edit` are a closed set. This asserts the
    /// set is still what the census measured.
    ///
    /// **It asserts PAIRS, not a file set and a count.** Replacing
    /// `ParseView`'s override with an unclassified type in the same
    /// file leaves both the file set and the total unchanged, and only
    /// the pair set catches it.
    ///
    /// **Its reach is in-tree, and that is a real limit.** [`View`] and
    /// `Buffer::attach_view` are both public, so a downstream crate may
    /// implement `on_edit` and attach it; no in-tree measurement can
    /// enumerate that. What speaks to those implementors is the
    /// documented contract on `Edit` itself.
    ///
    /// It also guards only the census's CLOSURE condition — that the
    /// override set is unchanged — not the classifications inside it.
    /// Those are executed by
    /// `identity_replace_history_op_leaves_classified_consumers_unchanged`.
    #[test]
    fn every_in_tree_on_edit_override_is_one_the_census_classified() {
        /// `(file, impl target)`, as measured by the census.
        const CLASSIFIED: [(&str, &str); 4] = [
            ("fold.rs", "FoldStoreTranslator"),
            ("overlay.rs", "BufferStyleSpanTranslator"),
            ("syntax.rs", "ParseView"),
            ("text_view.rs", "TextView"),
        ];

        fn rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("src is readable") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    rs_files(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }

        let mut files = Vec::new();
        rs_files(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        files.sort();

        let mut found: Vec<(String, String)> = Vec::new();
        for path in &files {
            let name = path
                .file_name()
                .expect("file name")
                .to_string_lossy()
                .into_owned();
            let text = std::fs::read_to_string(path).expect("source is readable");
            let lines: Vec<&str> = text.lines().collect();
            // The test module boundary: the first `#[cfg(test)]` that
            // introduces a `mod`. Overrides below it are test fixtures
            // and are out of scope, as the census says.
            let boundary = lines.iter().enumerate().find_map(|(i, l)| {
                (l.trim() == "#[cfg(test)]"
                    && lines
                        .get(i + 1)
                        .is_some_and(|n| n.trim_start().starts_with("mod ")))
                .then_some(i)
            });
            for (i, line) in lines.iter().enumerate() {
                if boundary.is_some_and(|b| i > b) {
                    break;
                }
                if !line.contains("fn on_edit") {
                    continue;
                }
                // Walk back to the enclosing `impl … for <Type>`. A hit
                // on a trait declaration first means this is the
                // trait's own default, which is not an override.
                for j in (0..=i).rev() {
                    let l = lines[j].trim_start();
                    if let Some(rest) = l.strip_prefix("impl")
                        && let Some(after) = rest.split(" for ").nth(1)
                    {
                        let target = after
                            .split_whitespace()
                            .next()
                            .unwrap_or("")
                            .trim_end_matches('{')
                            .rsplit("::")
                            .next()
                            .unwrap_or("")
                            .to_owned();
                        found.push((name.clone(), target));
                        break;
                    }
                    if l.starts_with("trait ") || l.starts_with("pub trait ") {
                        break;
                    }
                }
            }
        }
        found.sort();

        let expected: Vec<(String, String)> = CLASSIFIED
            .iter()
            .map(|(f, t)| ((*f).to_owned(), (*t).to_owned()))
            .collect();
        assert_eq!(
            found, expected,
            "C9: the in-tree `on_edit` override set no longer matches the \
             census in docs/crdt-identity-undo-framing.md §4. Every \
             override is a consumer of the version-only history `Edit` \
             and needs classifying there."
        );
    }
}
