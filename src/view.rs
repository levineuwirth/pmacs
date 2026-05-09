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
    /// 0-based row.
    pub row: u32,
    /// 0-based column.
    pub col: u32,
}

impl DisplayCoord {
    /// Construct a display coordinate.
    #[must_use]
    pub const fn new(row: u32, col: u32) -> Self {
        Self { row, col }
    }
}

/// What to render and where.
///
/// The frontend computes the viewport (which buffer range maps to which
/// cell origin) and hands it to the view. The view fills cells inside that
/// origin/size window.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct Viewport {
    /// First byte in the buffer to consider rendering.
    pub buffer_start: Position,
    /// One past the last byte to consider.
    pub buffer_end: Position,
    /// Top-left cell in the grid where rendering should start.
    pub cell_origin: CellCoord,
    /// Number of cells the viewport occupies.
    pub cell_size: CellSize,
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
    fn render(&mut self, _buf: &Buffer, _viewport: Viewport, _cells: &mut CellGrid<'_>) {}

    /// Translate a buffer byte position to a display coordinate, if the
    /// view holds a meaningful mapping for that position.
    ///
    /// Default: returns `None` (view has no opinion).
    fn pos_to_display(&self, _buf: &Buffer, _pos: Position) -> Option<DisplayCoord> {
        None
    }

    /// Translate a display coordinate back to a buffer byte position.
    ///
    /// Default: returns `None`.
    fn display_to_pos(&self, _buf: &Buffer, _coord: DisplayCoord) -> Option<Position> {
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
        // hand the same descriptor to multiple views without ceremony.
        fn assert_copy<T: Copy>() {}
        assert_copy::<Viewport>();
    }
}
