//! T M10.9 — paint other-frontend cursor and selection overlays
//! into a recipient's grid.
//!
//! # Contract
//!
//! Called by [`crate::instance_render::RenderState::render_frame`]
//! AFTER the main paint pass writes the buffer content into the
//! recipient's `next` grid, and BEFORE the grid is diffed against
//! the previous frame. The overlay pass MODIFIES cells in place
//! — it doesn't insert / shift / re-layout. The grid size is
//! unchanged. The diff captures overlay changes as ordinary cell
//! diffs (style changes are visible to `Cell::PartialEq`).
//!
//! # Coordinate resolution
//!
//! **Source's byte position resolves to recipient's grid coordinates
//! via recipient's window layout.** Different recipients with
//! different viewports paint the source cursor at different grid
//! coordinates — that's correct: each recipient sees the overlay
//! where their own view places it.
//!
//! This forecloses the bug shape of "painted at source's coords"
//! (which would assume both ends share a viewport — they don't).
//!
//! # Cursor + label
//!
//! - Cursor cell: foreground color set to the source's assigned
//!   palette color; `reverse: true` makes it stand out against
//!   underlying text. The original glyph is preserved.
//! - Label: a single character (`'A'`..`'Z'` for `FrontendId`s 2–27;
//!   `None` beyond) painted ONE row above the cursor cell. If
//!   `row == 0`, painted ONE row below instead. Label uses the
//!   source's color, no reverse.
//!
//! # Selection
//!
//! For each cell within the source's selection range that's
//! visible in the recipient's window: `underline = Single` plus
//! the source's color. Distinct from local selection's `reverse`
//! styling — a recipient can visually distinguish their own
//! selection from a remote one.
//!
//! # Filtering
//!
//! Overlays paint only when:
//! - Source != recipient (sender exclusion is the caller's
//!   responsibility; we don't recheck here)
//! - The source's buffer matches at least one of the recipient's
//!   windows' buffers
//! - The source's cursor maps to a coord visible in that window's
//!   viewport (within `view_top` + `inner_rows`, within rect cols)
//!
//! Otherwise the overlay is silently skipped — no off-screen
//! indicator; M10.x may add one.

use crate::cell::{CellCoord, CellGrid, CellSize, Color, UnderlineStyle};
use crate::editor::EditorState;
use crate::overlay_color::{color_for_slot, label_for_frontend_id};
use crate::presence::PresenceSnapshot;
use crate::protocol::FrontendId;
use crate::view::View;
use crate::window::Rect;

/// One other-frontend presence, with the daemon's resolved color
/// slot. The dispatcher builds these from
/// [`crate::presence::SessionRegistry::other_presences_for`] +
/// the per-session color slot.
#[derive(Copy, Clone, Debug)]
pub struct OtherPresence {
    /// The source frontend.
    pub frontend_id: FrontendId,
    /// The source's last-broadcast presence snapshot.
    pub snapshot: PresenceSnapshot,
    /// Palette slot index (0..[`crate::overlay_color::PALETTE_LEN`])
    /// for the source's color. Resolved to `Color` via
    /// [`color_for_slot`].
    pub color_slot: u8,
}

/// Paint other-frontend cursor + selection overlays into the
/// recipient's grid.
///
/// `state.core.active_frontend` is the recipient. `grid` is the
/// recipient's `next` grid post-`paint_frame`. `term_size` is the
/// recipient's terminal dimensions.
///
/// See module docs for the painting semantics. This function is
/// idempotent within a single tick — calling it twice produces
/// the same final grid; the per-tick coalescing happens via
/// `SessionRegistry::sweep`.
pub fn paint_other_frontend_overlays(
    state: &EditorState,
    grid: &mut CellGrid,
    term_size: CellSize,
    other_presences: &[OtherPresence],
) {
    if other_presences.is_empty() {
        return;
    }
    if term_size.rows < 2 || term_size.cols == 0 {
        return;
    }

    let core = state.core.borrow();
    // Reserve the last row for status / minibuffer (same convention
    // as `paint_frame`); overlays only paint into the text area.
    let text_rows = term_size.rows.saturating_sub(1);
    if text_rows == 0 {
        return;
    }
    let text_area = Rect::new(0, 0, text_rows, term_size.cols);
    let placements = core.active_layout().compute(text_area);

    let registry = core.registry.clone();
    let reg = registry.borrow();

    for presence in other_presences {
        let color = color_for_slot(presence.color_slot);
        let label = label_for_frontend_id(presence.frontend_id);

        // For each of the recipient's windows whose buffer matches
        // the source's snapshot.buffer_id, paint the cursor in
        // that window's viewport.
        for (win_id, window) in &core.windows {
            if window.buffer_id != presence.snapshot.buffer_id {
                continue;
            }
            let Some(rect) = placements.get(win_id).copied() else {
                continue;
            };
            let inner_rows = inner_rows_of(&rect);
            if inner_rows == 0 || rect.size.cols == 0 {
                continue;
            }
            let Ok(buf) = reg.get(window.buffer_id) else {
                continue;
            };
            // UX gutter: this window may reserve a left strip for line
            // numbers; a remote cursor/selection is a text-relative column
            // shifted right by that width (0 when the gutter is off). This
            // pass runs after `paint_frame`, so it learns the width here.
            let gutter_w = {
                let g = window.gutter_width();
                if g >= rect.size.cols { 0 } else { g }
            };
            let text_cols = rect.size.cols.saturating_sub(gutter_w);
            // Source's byte position → display coords via THIS
            // recipient window's text_view (the recipient's view
            // of the buffer).
            let Some(disp) = window
                .text_view
                .pos_to_display(buf, presence.snapshot.cursor)
            else {
                continue;
            };
            // Filter to viewport visible range. `view_top` is the
            // top visible buffer-line; cells below it are in-frame
            // until `view_top + inner_rows`.
            let row_in_window = match (disp.row as usize).checked_sub(window.view_top) {
                Some(r) if r < inner_rows as usize => r,
                _ => continue,
            };
            // Column bounds: disp.col is the buffer column; window
            // doesn't horizontally scroll in v1.0, so cells past
            // rect.size.cols are simply off-grid for this window.
            if disp.col >= text_cols {
                continue;
            }
            let cursor_grid_row = rect.origin.row + row_in_window as u32;
            let cursor_grid_col = rect.origin.col + gutter_w + disp.col;
            paint_cursor_cell(grid, cursor_grid_row, cursor_grid_col, color);
            if let Some(label_ch) = label {
                paint_label_cell(grid, cursor_grid_row, cursor_grid_col, label_ch, color);
            }
            // Selection overlay. Iterate cells in the selection
            // range visible in this window.
            if let Some(sel) = presence.snapshot.selection {
                let (lo, hi) = if sel.anchor <= sel.active {
                    (sel.anchor, sel.active)
                } else {
                    (sel.active, sel.anchor)
                };
                paint_selection_in_window(
                    grid, buf, window, rect, inner_rows, gutter_w, lo, hi, color,
                );
            }
        }
    }
}

/// Compute the inner rows of a window's rect — the text area,
/// excluding the bottom mode-line row.
fn inner_rows_of(rect: &Rect) -> u32 {
    rect.size.rows.saturating_sub(1)
}

/// Paint the cursor cell for the source: set fg to the source's
/// color and toggle reverse. Preserves the underlying glyph.
fn paint_cursor_cell(grid: &mut CellGrid, row: u32, col: u32, color: Color) {
    if row >= grid.size.rows || col >= grid.size.cols {
        return;
    }
    let cell = grid.at(CellCoord::new(row, col));
    cell.style.fg = color;
    cell.style.reverse = !cell.style.reverse;
}

/// Paint the label character one row above the cursor (or below
/// if `row == 0`). The label cell uses the source's color
/// without reverse, so it's distinct from the cursor cell.
fn paint_label_cell(grid: &mut CellGrid, cursor_row: u32, col: u32, label: char, color: Color) {
    let label_row = if cursor_row == 0 {
        // Top edge: paint below.
        cursor_row + 1
    } else {
        cursor_row - 1
    };
    if label_row >= grid.size.rows || col >= grid.size.cols {
        return;
    }
    let cell = grid.at(CellCoord::new(label_row, col));
    cell.glyph = crate::cell::Glyph::Char(label);
    cell.style.fg = color;
    cell.style.bold = true;
}

/// Paint the source's selection cells visible in this window.
/// Each cell within `[lo, hi)` that maps to a visible coord gets
/// `underline = Single` + the source's color.
#[allow(clippy::too_many_arguments)]
fn paint_selection_in_window(
    grid: &mut CellGrid,
    buf: &crate::buffer::Buffer,
    window: &crate::window::Window,
    rect: Rect,
    inner_rows: u32,
    gutter_w: u32,
    lo: crate::rope::Position,
    hi: crate::rope::Position,
    color: Color,
) {
    if lo >= hi {
        return;
    }
    let text_cols = rect.size.cols.saturating_sub(gutter_w);
    // Walk byte positions from lo to hi, mapping each to a
    // display coord. Step in single-byte increments; pos_to_display
    // tolerates byte-boundary positions and returns None for
    // positions outside the buffer.
    //
    // This is O(N) in the selection's byte length. For typical
    // selections (5–100 cells), microseconds. For large selections
    // (multi-megabyte), this would be too expensive — but the
    // viewport bound makes this naturally cheap: cells outside
    // the viewport are skipped via the row-bounds check, and the
    // selection only PAINTS for cells in the viewport. We could
    // restrict iteration to viewport-byte-range; for v1.0 the
    // straightforward walk is fine.
    let mut pos = lo;
    while pos < hi {
        let Some(disp) = window.text_view.pos_to_display(buf, pos) else {
            break;
        };
        match (disp.row as usize).checked_sub(window.view_top) {
            Some(r) if r < inner_rows as usize && disp.col < text_cols => {
                let grid_row = rect.origin.row + r as u32;
                let grid_col = rect.origin.col + gutter_w + disp.col;
                if grid_row < grid.size.rows && grid_col < grid.size.cols {
                    let cell = grid.at(CellCoord::new(grid_row, grid_col));
                    cell.style.underline = UnderlineStyle::Single;
                    // Use the source's color for the underline; if
                    // the cell already has a foreground style, the
                    // underline color comes from the fg. We don't
                    // override fg to preserve the cell's existing
                    // glyph appearance.
                    if cell.style.fg == Color::Default {
                        cell.style.fg = color;
                    }
                }
            }
            _ => {
                // Below viewport — no further cells in this window
                // are visible if we're past `view_top + inner_rows`.
                // But the selection might have skipped some bytes
                // (multibyte char boundaries); we keep walking.
            }
        }
        pos += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::BufferId;
    use crate::cell::{Cell, CellSize, Glyph};
    use crate::editor::EditorState;
    use crate::protocol::SelectionSnapshot;

    fn empty_grid(size: CellSize) -> Vec<Cell> {
        vec![Cell::default(); (size.rows * size.cols) as usize]
    }

    fn make_grid(cells: &mut [Cell], size: CellSize) -> CellGrid<'_> {
        CellGrid {
            cells,
            stride: size.cols,
            size,
        }
    }

    fn dummy_presence(
        fid: FrontendId,
        buffer_id: BufferId,
        cursor: u64,
        color_slot: u8,
    ) -> OtherPresence {
        OtherPresence {
            frontend_id: fid,
            snapshot: PresenceSnapshot {
                buffer_id,
                cursor,
                selection: None,
            },
            color_slot,
        }
    }

    #[test]
    fn empty_presences_no_paint() {
        let state = EditorState::new();
        let size = CellSize::new(24, 80);
        let mut cells = empty_grid(size);
        let original = cells.clone();
        let mut grid = make_grid(&mut cells, size);
        paint_other_frontend_overlays(&state, &mut grid, size, &[]);
        assert_eq!(cells, original, "empty presences leaves grid unchanged");
    }

    #[test]
    fn presence_in_different_buffer_no_paint() {
        let state = EditorState::new();
        let size = CellSize::new(24, 80);
        let mut cells = empty_grid(size);
        let original = cells.clone();
        let mut grid = make_grid(&mut cells, size);
        // BufferId::next() produces a unique id that doesn't match
        // the in-process scratch buffer.
        let presence = dummy_presence(FrontendId(2), BufferId::next(), 0, 0);
        paint_other_frontend_overlays(&state, &mut grid, size, &[presence]);
        assert_eq!(
            cells, original,
            "presence in different buffer leaves grid unchanged"
        );
    }

    #[test]
    fn cursor_at_origin_paints_cell_and_label_below() {
        let state = EditorState::new();
        let size = CellSize::new(24, 80);
        let mut cells = empty_grid(size);
        let mut grid = make_grid(&mut cells, size);
        // Get the scratch buffer's id so the presence matches.
        let active_buf = state.core.borrow().active_buffer_id();
        let presence = dummy_presence(FrontendId(2), active_buf, 0, 0);
        paint_other_frontend_overlays(&state, &mut grid, size, &[presence]);

        // Cursor at row 0, col 0: cell modified (reverse toggled,
        // fg = palette[0]).
        let cursor_cell = &cells[0];
        assert_eq!(cursor_cell.style.fg, color_for_slot(0));
        assert!(cursor_cell.style.reverse);

        // Label painted BELOW the cursor since row 0 is the top.
        let label_cell = &cells[(size.cols) as usize];
        assert!(matches!(label_cell.glyph, Glyph::Char('A')));
        assert_eq!(label_cell.style.fg, color_for_slot(0));
    }

    #[test]
    fn frontend_beyond_26_paints_cursor_no_label() {
        let state = EditorState::new();
        let size = CellSize::new(24, 80);
        let mut cells = empty_grid(size);
        let mut grid = make_grid(&mut cells, size);
        let active_buf = state.core.borrow().active_buffer_id();
        let presence = dummy_presence(FrontendId(28), active_buf, 0, 3);
        paint_other_frontend_overlays(&state, &mut grid, size, &[presence]);

        // Cursor cell painted with slot 3's color.
        let cursor_cell = &cells[0];
        assert_eq!(cursor_cell.style.fg, color_for_slot(3));
        assert!(cursor_cell.style.reverse);

        // Label cell NOT painted (FrontendId 28 has no label).
        let label_cell = &cells[(size.cols) as usize];
        assert_eq!(*label_cell, Cell::default(), "no label for FrontendId(28)");
    }

    #[test]
    fn cursor_off_grid_no_paint() {
        // The scratch buffer is empty; pos_to_display for cursor
        // beyond the buffer returns None → no paint.
        let state = EditorState::new();
        let size = CellSize::new(24, 80);
        let mut cells = empty_grid(size);
        let original = cells.clone();
        let mut grid = make_grid(&mut cells, size);
        let active_buf = state.core.borrow().active_buffer_id();
        // Cursor at byte 999 — far past the empty scratch buffer.
        let presence = dummy_presence(FrontendId(2), active_buf, 999, 0);
        paint_other_frontend_overlays(&state, &mut grid, size, &[presence]);
        assert_eq!(
            cells, original,
            "cursor at out-of-buffer position leaves grid unchanged"
        );
    }

    #[test]
    fn selection_paints_underline_on_visible_cells() {
        // Construct a state with some buffer content so selection
        // has cells to paint.
        let state = EditorState::new();
        // Insert a few chars so cursor positions 0..5 are valid.
        state.core.borrow_mut().insert_char('h');
        state.core.borrow_mut().insert_char('i');
        state.core.borrow_mut().insert_char('!');
        let size = CellSize::new(24, 80);
        let mut cells = empty_grid(size);
        let mut grid = make_grid(&mut cells, size);
        let active_buf = state.core.borrow().active_buffer_id();

        let mut presence = dummy_presence(FrontendId(2), active_buf, 0, 0);
        presence.snapshot.selection = Some(SelectionSnapshot {
            anchor: 0,
            active: 3,
        });
        paint_other_frontend_overlays(&state, &mut grid, size, &[presence]);

        // Cells 0, 1, 2 should have UnderlineStyle::Single.
        for (col, cell) in cells.iter().enumerate().take(3) {
            assert_eq!(
                cell.style.underline,
                UnderlineStyle::Single,
                "cell {col} should be underlined"
            );
        }
        // Cell 3 should NOT be underlined (selection is [0, 3) exclusive).
        assert_eq!(cells[3].style.underline, UnderlineStyle::None);
    }
}
