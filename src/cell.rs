// cell.rs --- Cell-grid rendering target. TUI-shaped, GUI-extensible.

//! Cell-grid rendering target.
//!
//! Implements the cell-grid types from spec §3.3 (Cell Grid). The grid is a
//! `rows × cols` 2D array of [`Cell`]s; each cell carries a [`Glyph`], a
//! [`Style`], and an optional [`Attachment`]. The TUI ignores `Attachment`;
//! a future GUI backend interprets it.
//!
//! ## Module split (session 1 of the `pmacs-gpu` arc)
//!
//! The data types — `Cell`, `Glyph`, `Style`, `Color`, `UnderlineStyle`,
//! `CellCoord`, `CellSize`, `Attachment`, `DiffSpan` — moved to
//! `pmacs-protocol::cell` and are re-exported here so existing
//! `crate::cell::Cell` import paths keep resolving. [`CellGrid`] and
//! [`diff`] stay in this module — they are instance-side rendering
//! machinery, not wire shapes. See `docs/pmacs-gpu-design.md`.

pub use pmacs_protocol::{
    Attachment, Cell, CellCoord, CellSize, Color, DiffSpan, Glyph, Style, UnderlineStyle,
};

// ---------------------------------------------------------------------------
// Grid (instance-side render surface; borrowed slice; does not move)
// ---------------------------------------------------------------------------

/// A mutable view onto a row-major cell buffer.
///
/// The grid does not own its memory: the frontend owns a `Vec<Cell>` and
/// hands a borrow to views during render. Cells are addressed via
/// [`CellCoord`].
pub struct CellGrid<'a> {
    /// Backing cell buffer, length `stride * size.rows` cells, row-major.
    pub cells: &'a mut [Cell],
    /// Stride in cells per row. Often equals `size.cols`, but allows the
    /// frontend to keep extra columns for double-buffering.
    pub stride: u32,
    /// Visible rows × cols.
    pub size: CellSize,
}

impl CellGrid<'_> {
    /// Borrow the cell at `coord` mutably.
    ///
    /// Panics if `coord` is outside the grid.
    pub fn at(&mut self, coord: CellCoord) -> &mut Cell {
        debug_assert!(coord.row < self.size.rows);
        debug_assert!(coord.col < self.size.cols);
        let idx = coord.row as usize * self.stride as usize + coord.col as usize;
        &mut self.cells[idx]
    }

    /// Read the cell at `coord`.
    ///
    /// Panics if `coord` is outside the grid.
    #[must_use]
    pub fn get(&self, coord: CellCoord) -> &Cell {
        debug_assert!(coord.row < self.size.rows);
        debug_assert!(coord.col < self.size.cols);
        let idx = coord.row as usize * self.stride as usize + coord.col as usize;
        &self.cells[idx]
    }

    /// Reset every visible cell to [`Cell::default`].
    pub fn clear(&mut self) {
        for row in 0..self.size.rows {
            for col in 0..self.size.cols {
                *self.at(CellCoord::new(row, col)) = Cell::default();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Diff (instance-side renderer helper; does not move)
// ---------------------------------------------------------------------------

/// Compute the diff between two cell buffers of identical layout.
///
/// `prev` and `next` are row-major slices, each of length at least
/// `stride * size.rows`; `stride` is the cells-per-row stride (typically
/// equals `size.cols`). The result is a list of [`DiffSpan`]s, one per
/// contiguous run of changed cells, in row-major order. Identical buffers
/// yield an empty `Vec`.
///
/// Threading: any thread.
#[must_use]
pub fn diff(prev: &[Cell], next: &[Cell], stride: u32, size: CellSize) -> Vec<DiffSpan> {
    debug_assert!(prev.len() >= (stride as usize) * (size.rows as usize));
    debug_assert!(next.len() >= (stride as usize) * (size.rows as usize));

    let mut spans = Vec::new();
    for row in 0..size.rows {
        let row_offset = (row as usize) * (stride as usize);
        let mut col = 0u32;
        while col < size.cols {
            let idx = row_offset + col as usize;
            if prev[idx] == next[idx] {
                col += 1;
                continue;
            }
            let start = CellCoord::new(row, col);
            let mut cells = Vec::new();
            while col < size.cols {
                let idx = row_offset + col as usize;
                if prev[idx] == next[idx] {
                    break;
                }
                cells.push(next[idx].clone());
                col += 1;
            }
            spans.push(DiffSpan { start, cells });
        }
    }
    spans
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(c: char) -> Cell {
        Cell {
            glyph: Glyph::Char(c),
            style: Style::default(),
            attachment: None,
        }
    }

    fn cont() -> Cell {
        Cell {
            glyph: Glyph::Continuation,
            style: Style::default(),
            attachment: None,
        }
    }

    #[test]
    fn cell_grid_at_round_trip() {
        let mut storage = vec![Cell::default(); 12];
        let mut grid = CellGrid {
            cells: &mut storage,
            stride: 4,
            size: CellSize::new(3, 4),
        };
        *grid.at(CellCoord::new(1, 2)) = ch('X');
        assert_eq!(grid.get(CellCoord::new(1, 2)).glyph, Glyph::Char('X'));
        // The cell at (1, 2) lives at offset row*stride + col = 1*4 + 2 = 6.
        assert_eq!(storage[6].glyph, Glyph::Char('X'));
    }

    #[test]
    fn cell_grid_clear_resets() {
        let mut storage = vec![ch('Z'); 6];
        let mut grid = CellGrid {
            cells: &mut storage,
            stride: 3,
            size: CellSize::new(2, 3),
        };
        grid.clear();
        assert!(storage.iter().all(|c| *c == Cell::default()));
    }

    // ----- diff -----

    #[test]
    fn diff_identical_is_empty() {
        let prev = vec![ch('a'); 4];
        let next = vec![ch('a'); 4];
        assert!(diff(&prev, &next, 4, CellSize::new(1, 4)).is_empty());
    }

    #[test]
    fn diff_single_cell_change() {
        let prev = vec![ch('a'), ch('b'), ch('c'), ch('d')];
        let next = vec![ch('a'), ch('B'), ch('c'), ch('d')];
        let spans = diff(&prev, &next, 4, CellSize::new(1, 4));
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start, CellCoord::new(0, 1));
        assert_eq!(spans[0].cells, vec![ch('B')]);
    }

    #[test]
    fn diff_contiguous_run_is_one_span() {
        let prev = vec![ch('a'), ch('b'), ch('c'), ch('d')];
        let next = vec![ch('a'), ch('B'), ch('C'), ch('d')];
        let spans = diff(&prev, &next, 4, CellSize::new(1, 4));
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start, CellCoord::new(0, 1));
        assert_eq!(spans[0].cells, vec![ch('B'), ch('C')]);
    }

    #[test]
    fn diff_split_by_unchanged_cell_is_two_spans() {
        let prev = vec![ch('a'), ch('b'), ch('c'), ch('d')];
        let next = vec![ch('A'), ch('b'), ch('C'), ch('d')];
        let spans = diff(&prev, &next, 4, CellSize::new(1, 4));
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].start, CellCoord::new(0, 0));
        assert_eq!(spans[0].cells, vec![ch('A')]);
        assert_eq!(spans[1].start, CellCoord::new(0, 2));
        assert_eq!(spans[1].cells, vec![ch('C')]);
    }

    #[test]
    fn diff_separate_rows_produce_separate_spans() {
        // 2 rows × 3 cols, stride 3.
        let prev = vec![ch('a'), ch('b'), ch('c'), ch('d'), ch('e'), ch('f')];
        let next = vec![ch('a'), ch('B'), ch('c'), ch('d'), ch('e'), ch('F')];
        let spans = diff(&prev, &next, 3, CellSize::new(2, 3));
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].start, CellCoord::new(0, 1));
        assert_eq!(spans[1].start, CellCoord::new(1, 2));
    }

    #[test]
    fn diff_handles_wide_char_continuations() {
        // Old: narrow 'a', 'b'. New: wide '中' + Continuation.
        // Both cells differ, span includes both.
        let prev = vec![ch('a'), ch('b'), ch('c')];
        let next = vec![ch('中'), cont(), ch('c')];
        let spans = diff(&prev, &next, 3, CellSize::new(1, 3));
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start, CellCoord::new(0, 0));
        assert_eq!(spans[0].cells, vec![ch('中'), cont()]);
    }

    #[test]
    fn diff_respects_stride_with_padding() {
        // 1 row × 2 cols, but stride is 4 (extra padding columns ignored).
        let prev = vec![ch('a'), ch('b'), ch('!'), ch('!')];
        let next = vec![ch('A'), ch('b'), ch('?'), ch('?')];
        let spans = diff(&prev, &next, 4, CellSize::new(1, 2));
        // Padding columns 2..4 are not in size and must not be reported.
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start, CellCoord::new(0, 0));
        assert_eq!(spans[0].cells, vec![ch('A')]);
    }
}
