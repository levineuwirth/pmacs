// instance_render.rs --- Instance-side cell-buffer ownership and paint-and-diff.

//! Instance-side rendering (T M5.2).
//!
//! Spec §sec:m5-remote, §sec:v01-remote-scope deliverable 1.
//!
//! The `prev`/`next` cell buffers live on the instance, not on the frontend.
//! [`paint_frame`](crate::editor::paint_frame) writes into `next`; the diff
//! against `prev` produces a [`InstanceMessage::CellDelta`] the transport
//! ships to the frontend. The frontend (whether in-process TUI or network)
//! consumes the messages and emits whatever it needs to display them.
//!
//! This factoring is the architectural payoff promised by the spec
//! (§sec:remote): a frontend that diffs full grids locally cannot exist
//! over a network without making the network case pathologically expensive.
//! Doing the diff once, on the instance side, is what makes the SSH
//! transport (T M5.7) cheap.

use crate::cell::{Cell, CellGrid, CellSize, diff};
use crate::editor::{EditorState, paint_frame};
use crate::protocol::{CursorState, FrontendId, InstanceMessage};
use crate::terminal::TerminalSnapshot;
use crate::window::WindowId;
use std::collections::HashMap;

/// Owns the cell buffers and runs the paint-and-diff cycle.
pub struct RenderState {
    size: CellSize,
    /// Last frame as shipped to the frontend. After [`Self::render_frame`],
    /// this is the most recent painted state.
    prev: Vec<Cell>,
    /// Next frame being assembled. Cleared between frames so each render
    /// starts from a blank slate (windows that shrink on resize do not
    /// leak old contents).
    next: Vec<Cell>,
    /// True until the first frame is emitted, and after every resize.
    /// Tells [`Self::render_frame`] to mark the next [`InstanceMessage::CellDelta`]
    /// as a full-grid sync. Remote frontends use this flag to know they
    /// must blank their local buffer before applying the deltas.
    needs_full_grid: bool,
}

impl RenderState {
    /// Construct a render state for a grid of the given dimensions.
    #[must_use]
    pub fn new(size: CellSize) -> Self {
        let cells = (size.rows as usize) * (size.cols as usize);
        Self {
            size,
            prev: vec![Cell::default(); cells],
            next: vec![Cell::default(); cells],
            needs_full_grid: true,
        }
    }

    /// Current grid size in cells.
    #[must_use]
    pub fn size(&self) -> CellSize {
        self.size
    }

    /// Resize the grid. Reallocates `prev`/`next` and flags the next
    /// emitted [`InstanceMessage::CellDelta`] as a full-grid sync, since
    /// the old buffer's contents are no longer applicable.
    ///
    /// No-op if `new_size` matches the current size.
    pub fn resize(&mut self, new_size: CellSize) {
        if new_size == self.size {
            return;
        }
        self.size = new_size;
        let cells = (new_size.rows as usize) * (new_size.cols as usize);
        self.prev = vec![Cell::default(); cells];
        self.next = vec![Cell::default(); cells];
        self.needs_full_grid = true;
    }

    /// Force the next emitted [`InstanceMessage::CellDelta`] to be a
    /// full-grid sync, even when the buffer has not changed shape.
    /// Used on fresh attach when a frontend connects to an
    /// already-running instance and needs an authoritative starting
    /// state.
    pub fn force_full_grid_resync(&mut self) {
        self.needs_full_grid = true;
    }

    /// Paint one frame and return the messages to ship.
    ///
    /// Returns a `CellDelta` (with the changed spans) followed by a
    /// `Cursor` message. Returns an empty vec if the grid is too small
    /// to render meaningfully (`rows < 2` or `cols == 0`).
    ///
    /// `other_presences` (T M10.9): other attached frontends' cursor
    ///   and selection snapshots, with their assigned color slots.
    ///   The overlay paint pass modifies cells in `next` AFTER the
    ///   main paint and BEFORE the diff. Empty slice → no overlays
    ///   (in-process TUI use; M10.6/7 daemon use).
    pub fn render_frame(
        &mut self,
        state: &EditorState,
        frontend_id: FrontendId,
        terminal_snapshots: &HashMap<WindowId, TerminalSnapshot>,
        other_presences: &[crate::overlay_paint::OtherPresence],
    ) -> Vec<InstanceMessage> {
        if self.size.rows < 2 || self.size.cols == 0 {
            return Vec::new();
        }

        let cursor_coord = {
            let mut grid = CellGrid {
                cells: &mut self.next,
                stride: self.size.cols,
                size: self.size,
            };
            let coord = paint_frame(
                state,
                frontend_id,
                terminal_snapshots,
                &mut grid,
                self.size,
            );
            // T M10.9 — overlay paint after main paint, before diff.
            // Modifies cells in `next`; diff captures the changes
            // as ordinary style updates.
            crate::overlay_paint::paint_other_frontend_overlays(
                state,
                &mut grid,
                self.size,
                other_presences,
            );
            coord
        };

        // Full-grid sync semantics (T M5.3): when `needs_full_grid` is
        // set, the `prev` buffer no longer represents what the consumer
        // sees on screen — either it is freshly-allocated (just-resized
        // or just-constructed), or a new frontend is attaching to a
        // running instance whose `prev` reflects the previous
        // frontend's view, not the new one's. Compare `next` against an
        // all-default grid so the consumer receives every non-default
        // cell of the current screen, regardless of what the previous
        // frontend was shown.
        let spans = if self.needs_full_grid {
            let blank = vec![Cell::default(); self.next.len()];
            diff(&blank, &self.next, self.size.cols, self.size)
        } else {
            diff(&self.prev, &self.next, self.size.cols, self.size)
        };
        let full_grid = self.needs_full_grid;
        self.needs_full_grid = false;

        std::mem::swap(&mut self.prev, &mut self.next);
        for cell in &mut self.next {
            *cell = Cell::default();
        }

        let cursor_state = cursor_coord.map(|coord| CursorState {
            coord,
            visible: true,
        });

        vec![
            InstanceMessage::CellDelta { spans, full_grid },
            InstanceMessage::Cursor(cursor_state),
        ]
    }
}

#[cfg(test)]
mod tests {
    // Acceptance home for T M5.2 (TUI re-architected as protocol consumer)
    // and T M5.3 (cell-delta diffing happens instance-side). The M5.2
    // contract — `RenderState` lives instance-side, `Frontend` becomes
    // a transport sink — is structural and verified by the existing
    // `paint_frame` tests passing unchanged. The M5.3 contract is
    // exercised by the `m5_3_*`-prefixed tests below: full-grid sync on
    // fresh attach, differential subsequent frames, fresh full-grid on
    // re-attach. See tests/INDEX.md for the full M5.x → coverage map.

    use super::*;
    use crate::cell::CellCoord;
    use crate::editor::EditorState;
    use crate::protocol::FrontendId;

    fn empty_state() -> EditorState {
        EditorState::new()
    }

    #[test]
    fn new_allocates_full_buffers() {
        let r = RenderState::new(CellSize::new(24, 80));
        assert_eq!(r.prev.len(), 24 * 80);
        assert_eq!(r.next.len(), 24 * 80);
        assert!(r.needs_full_grid);
    }

    #[test]
    fn render_returns_cell_delta_and_cursor() {
        let mut r = RenderState::new(CellSize::new(24, 80));
        let msgs = r.render_frame(&empty_state(), FrontendId::LOCAL, &HashMap::new(), &[]);
        assert_eq!(msgs.len(), 2);
        assert!(matches!(msgs[0], InstanceMessage::CellDelta { .. }));
        assert!(matches!(msgs[1], InstanceMessage::Cursor(_)));
    }

    #[test]
    fn first_frame_is_full_grid_sync() {
        let mut r = RenderState::new(CellSize::new(24, 80));
        let msgs = r.render_frame(&empty_state(), FrontendId::LOCAL, &HashMap::new(), &[]);
        match &msgs[0] {
            InstanceMessage::CellDelta { full_grid, .. } => assert!(*full_grid),
            _ => panic!("expected CellDelta first"),
        }
    }

    #[test]
    fn second_frame_is_differential() {
        let mut r = RenderState::new(CellSize::new(24, 80));
        let _ = r.render_frame(&empty_state(), FrontendId::LOCAL, &HashMap::new(), &[]);
        let msgs = r.render_frame(&empty_state(), FrontendId::LOCAL, &HashMap::new(), &[]);
        match &msgs[0] {
            InstanceMessage::CellDelta { full_grid, .. } => assert!(!*full_grid),
            _ => panic!("expected CellDelta first"),
        }
    }

    #[test]
    fn unchanged_state_produces_empty_spans_after_first_frame() {
        let state = empty_state();
        let mut r = RenderState::new(CellSize::new(24, 80));
        let _ = r.render_frame(&state, FrontendId::LOCAL, &HashMap::new(), &[]);
        let msgs = r.render_frame(&state, FrontendId::LOCAL, &HashMap::new(), &[]);
        match &msgs[0] {
            InstanceMessage::CellDelta { spans, .. } => assert!(
                spans.is_empty(),
                "expected no changes between identical frames; got {spans:?}"
            ),
            _ => unreachable!(),
        }
    }

    #[test]
    fn resize_reallocates_and_flags_full_grid() {
        let mut r = RenderState::new(CellSize::new(24, 80));
        let _ = r.render_frame(&empty_state(), FrontendId::LOCAL, &HashMap::new(), &[]);
        assert!(!r.needs_full_grid);

        r.resize(CellSize::new(40, 120));
        assert_eq!(r.size(), CellSize::new(40, 120));
        assert_eq!(r.prev.len(), 40 * 120);
        assert_eq!(r.next.len(), 40 * 120);
        assert!(r.needs_full_grid);

        let msgs = r.render_frame(&empty_state(), FrontendId::LOCAL, &HashMap::new(), &[]);
        match &msgs[0] {
            InstanceMessage::CellDelta { full_grid, .. } => assert!(*full_grid),
            _ => unreachable!(),
        }
    }

    #[test]
    fn resize_to_same_size_is_noop() {
        let mut r = RenderState::new(CellSize::new(24, 80));
        let _ = r.render_frame(&empty_state(), FrontendId::LOCAL, &HashMap::new(), &[]);
        assert!(!r.needs_full_grid);
        r.resize(CellSize::new(24, 80));
        // No reallocation, no full-grid flip.
        assert!(!r.needs_full_grid);
    }

    #[test]
    fn force_full_grid_resync_flips_flag() {
        let mut r = RenderState::new(CellSize::new(24, 80));
        let _ = r.render_frame(&empty_state(), FrontendId::LOCAL, &HashMap::new(), &[]);
        assert!(!r.needs_full_grid);
        r.force_full_grid_resync();
        assert!(r.needs_full_grid);
    }

    #[test]
    fn too_small_grid_returns_empty_messages() {
        // rows < 2 means we can't paint a text-area + status row.
        let mut r = RenderState::new(CellSize::new(1, 80));
        assert!(
            r.render_frame(&empty_state(), FrontendId::LOCAL, &HashMap::new(), &[])
                .is_empty()
        );

        let mut r = RenderState::new(CellSize::new(24, 0));
        assert!(
            r.render_frame(&empty_state(), FrontendId::LOCAL, &HashMap::new(), &[])
                .is_empty()
        );
    }

    #[test]
    fn cursor_message_carries_coord_when_paint_returns_one() {
        let mut r = RenderState::new(CellSize::new(24, 80));
        let msgs = r.render_frame(&empty_state(), FrontendId::LOCAL, &HashMap::new(), &[]);
        match &msgs[1] {
            InstanceMessage::Cursor(Some(cs)) => {
                assert!(cs.visible);
                // Empty *scratch* buffer puts the cursor at row 0, col 0.
                assert_eq!(cs.coord, CellCoord::new(0, 0));
            }
            InstanceMessage::Cursor(None) => {
                // Acceptable too — empty state may suppress cursor.
            }
            _ => panic!("expected Cursor message"),
        }
    }

    // -------------------------------------------------------------------
    // T M5.3 acceptance criteria
    //
    // Spec §sec:v01-remote-scope deliverable 2:
    //   1. A fresh attach receives a full-grid CellDelta on the first frame.
    //   2. A subsequent paint with one changed line emits only the changed
    //      cells (not a full grid).
    //   3. Detach + re-attach produces a fresh full-grid sync (the instance
    //      does not preserve frontend-specific state).
    // -------------------------------------------------------------------

    #[test]
    fn m5_3_fresh_attach_receives_full_grid_celldelta() {
        // Criterion 1: the first frame after construction is a full-grid
        // CellDelta carrying every non-default cell.
        let mut r = RenderState::new(CellSize::new(24, 80));
        let msgs = r.render_frame(&empty_state(), FrontendId::LOCAL, &HashMap::new(), &[]);
        match &msgs[0] {
            InstanceMessage::CellDelta { full_grid, spans } => {
                assert!(*full_grid, "first frame must be flagged full_grid=true");
                // Empty *scratch* still paints a status row + cursor.
                assert!(
                    !spans.is_empty(),
                    "fresh attach must surface the current screen, got empty spans"
                );
            }
            _ => panic!("expected CellDelta as the first message"),
        }
    }

    #[test]
    fn m5_3_differential_frame_after_small_edit_is_proportionally_small() {
        // Criterion 2: a state change that only affects a small region
        // of the grid produces a delta that touches only a small fraction
        // of cells, not the full grid.
        use crate::editor::EditorState;
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

        let size = CellSize::new(24, 80);
        let mut state = EditorState::new();
        let mut r = RenderState::new(size);
        // Seat the prev buffer.
        let _ = r.render_frame(&state, FrontendId::LOCAL, &HashMap::new(), &[]);

        // Single character insert.
        state.dispatch_key(
            FrontendId::LOCAL,
            KeyEvent {
                code: KeyCode::Char('x'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::empty(),
            },
        );
        let msgs = r.render_frame(&state, FrontendId::LOCAL, &HashMap::new(), &[]);
        match &msgs[0] {
            InstanceMessage::CellDelta { full_grid, spans } => {
                assert!(!*full_grid, "differential frame must not flag full_grid");
                let changed: usize = spans.iter().map(|s| s.cells.len()).sum();
                let total = (size.rows * size.cols) as usize;
                assert!(
                    changed * 10 < total,
                    "differential delta must be much smaller than full grid; \
                     got {changed} changed cells of {total} total"
                );
            }
            _ => panic!("expected CellDelta first"),
        }
    }

    #[test]
    fn m5_3_force_full_grid_resync_replays_full_screen_against_blank() {
        // Criterion 3: detach + re-attach produces a fresh full-grid
        // sync. The instance does not preserve frontend-specific state,
        // so the next frame after `force_full_grid_resync` reflects the
        // full screen against a blank baseline (not against what the
        // previous frontend saw).
        let size = CellSize::new(5, 20);
        let mut r = RenderState::new(size);

        // First render: seats prev with the painted frame.
        let first = r.render_frame(&empty_state(), FrontendId::LOCAL, &HashMap::new(), &[]);
        let baseline_changed: usize = match &first[0] {
            InstanceMessage::CellDelta { spans, .. } => spans.iter().map(|s| s.cells.len()).sum(),
            _ => unreachable!(),
        };
        assert!(baseline_changed > 0, "baseline must paint some cells");

        // A second render with no state change normally produces zero
        // spans (the state matches prev exactly).
        let unchanged = r.render_frame(&empty_state(), FrontendId::LOCAL, &HashMap::new(), &[]);
        match &unchanged[0] {
            InstanceMessage::CellDelta { full_grid, spans } => {
                assert!(!*full_grid);
                assert!(spans.is_empty(), "unchanged state should diff to nothing");
            }
            _ => unreachable!(),
        }

        // Now simulate detach + re-attach: the new frontend has no idea
        // what's on screen. force_full_grid_resync flags the next frame
        // for full sync.
        r.force_full_grid_resync();
        let resync = r.render_frame(&empty_state(), FrontendId::LOCAL, &HashMap::new(), &[]);
        match &resync[0] {
            InstanceMessage::CellDelta { full_grid, spans } => {
                assert!(*full_grid, "post-resync frame must be full_grid=true");
                let resync_changed: usize = spans.iter().map(|s| s.cells.len()).sum();
                assert_eq!(
                    resync_changed, baseline_changed,
                    "fresh-attach resync must surface the same number of cells \
                     as the original full-grid render; \
                     baseline={baseline_changed}, resync={resync_changed}"
                );
            }
            _ => panic!("expected CellDelta first"),
        }
    }
}
