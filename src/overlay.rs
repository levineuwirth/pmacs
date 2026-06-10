// overlay.rs --- Reference overlay views for multi-view composition (T M2.9).

//! Overlay views for multi-view composition.
//!
//! T M2.9 / spec §3.3.2 calls for multiple [`View`]s per buffer
//! contributing to the same cell grid. The base layer is the
//! [`crate::text_view::TextView`] that paints buffer text and the
//! default style; overlays render *after* the base, layering on top.
//!
//! # Composition rules
//!
//! See [`crate::view::View`] for the canonical statement. Briefly:
//!
//! 1. The window's base text view renders first and clears every cell
//!    in its viewport.
//! 2. The window's overlays then render in attach order (FIFO). Each
//!    overlay observes the cells already written and may read, mutate
//!    in place, or replace any cell inside its viewport.
//! 3. Later overlays win against earlier ones at any cell they both
//!    touch — composition is "last writer wins" for the *fields the
//!    overlay chooses to write*, with overlays free to read existing
//!    fields and merge.
//!
//! Cursor placement uses the base view's `pos_to_display`; overlays do
//! not move the cursor.
//!
//! # Two reference overlays
//!
//! [`StyleSpanOverlay`] modifies the [`Style`] of cells covered by a
//! list of (row, col-range) spans without touching their glyphs ---
//! the canonical syntax-highlight / diagnostics-underline pattern.
//!
//! [`VirtualCellOverlay`] writes whole cells (glyph + style +
//! attachment) at declared (row, col) positions --- the canonical
//! inline-blame / completion-ghost pattern.
//!
//! Both express positions in *cell coordinates* (relative to the
//! viewport's `cell_origin`), not buffer coordinates. Mapping from
//! buffer bytes to cell coordinates is the caller's responsibility
//! (typically via the base text view's `pos_to_display`); pushing the
//! mapping out keeps overlays cheap and stateless. A future
//! buffer-coord overlay would compose on top of these the same way.

use std::sync::{Arc, Mutex};

use unicode_width::UnicodeWidthChar;

use crate::buffer::Buffer;
use crate::cell::{Cell, CellCoord, CellGrid, Style};
use crate::rope::Edit;
use crate::view::{View, Viewport};

// ---------------------------------------------------------------------------
// StyleSpanOverlay
// ---------------------------------------------------------------------------

/// A run of cells that should have a [`Style`] merged in.
///
/// Coordinates are *cell-grid relative* to the rendering viewport's
/// `cell_origin`: `row` is the offset from the top of the viewport,
/// `start_col`/`end_col` are columns from the viewport's left edge.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StyleSpan {
    /// Row offset within the viewport. Spans whose row is outside the
    /// viewport are silently skipped at render time.
    pub row: u32,
    /// First column the span covers.
    pub start_col: u32,
    /// Column one past the end of the span.
    pub end_col: u32,
    /// Style to merge into each cell the span covers.
    pub style: Style,
}

/// View that merges [`StyleSpan`]s into the existing cell grid.
///
/// Glyphs and attachments are preserved verbatim. The merge replaces
/// `fg` and `bg` only when the span requested a non-default value, and
/// OR-blends the boolean attributes (bold, italic, reverse) so two
/// stacked overlays can both contribute. Underline replacement
/// follows the same "non-default wins" rule.
#[derive(Clone, Debug, Default)]
pub struct StyleSpanOverlay {
    /// Spans to merge. Render order within this overlay is the order
    /// of this `Vec`; later entries override earlier ones at the cells
    /// they share.
    pub spans: Vec<StyleSpan>,
}

impl StyleSpanOverlay {
    /// Empty overlay.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append `span`. Returns `&mut self` for chaining.
    pub fn add(&mut self, span: StyleSpan) -> &mut Self {
        self.spans.push(span);
        self
    }
}

impl View for StyleSpanOverlay {
    fn render(&mut self, _buf: &Buffer, viewport: Viewport, cells: &mut CellGrid<'_>) {
        for span in &self.spans {
            if span.row >= viewport.cell_size.rows {
                continue;
            }
            let max_col = viewport.cell_size.cols;
            let start = span.start_col.min(max_col);
            let end = span.end_col.min(max_col);
            let row = viewport.cell_origin.row + span.row;
            let col_origin = viewport.cell_origin.col;
            // Single mutable cell borrow per iteration: avoids the
            // double bounds-check pair (`get` then `at`) that pushed
            // overlay overhead past the 10% spec budget.
            for col in start..end {
                let cell = cells.at(CellCoord::new(row, col_origin + col));
                cell.style = merge_styles(cell.style, span.style);
            }
        }
    }
}

/// Merge `overlay` over `base`, producing a new [`Style`].
///
/// `fg`/`bg`/`underline`/`underline_color` use "non-default wins"
/// --- the overlay's value applies only when it is non-default;
/// otherwise the base value is preserved. Boolean attributes
/// (`bold`, `italic`, `reverse`) OR-blend so that two stacked
/// overlays can each contribute. Pulled out of [`StyleSpanOverlay`]
/// so [`crate::highlight::SyntaxHighlightView`] can reuse the same
/// composition rule (T M4.3).
#[must_use]
pub fn merge_styles(base: Style, overlay: Style) -> Style {
    use crate::cell::{Color, UnderlineStyle};
    Style {
        fg: if overlay.fg == Color::Default {
            base.fg
        } else {
            overlay.fg
        },
        bg: if overlay.bg == Color::Default {
            base.bg
        } else {
            overlay.bg
        },
        bold: base.bold || overlay.bold,
        italic: base.italic || overlay.italic,
        underline: if overlay.underline == UnderlineStyle::None {
            base.underline
        } else {
            overlay.underline
        },
        reverse: base.reverse || overlay.reverse,
        underline_color: if overlay.underline_color == Color::Default {
            base.underline_color
        } else {
            overlay.underline_color
        },
    }
}

// ---------------------------------------------------------------------------
// BufferStyleOverlay
// ---------------------------------------------------------------------------

/// A style annotation expressed in buffer byte coordinates.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BufferStyleSpan {
    /// First byte covered by the style.
    pub start: u64,
    /// Byte one past the styled range.
    pub end: u64,
    /// Style to merge over the base text.
    pub style: Style,
}

/// Shared span store used by Lua handles and render overlays.
pub type SharedBufferStyleSpans = Arc<Mutex<Vec<BufferStyleSpan>>>;

/// View that renders buffer-byte style annotations.
///
/// Unlike [`StyleSpanOverlay`], this overlay stores byte ranges rather
/// than viewport cell ranges. That is the right shape for stream
/// consumers such as the REPL: ANSI SGR applies to bytes as they land
/// in the rope, and render maps the surviving ranges into visible cells.
#[derive(Clone, Debug)]
pub struct BufferStyleOverlay {
    spans: SharedBufferStyleSpans,
}

impl BufferStyleOverlay {
    /// Construct an overlay backed by `spans`.
    #[must_use]
    pub fn new(spans: SharedBufferStyleSpans) -> Self {
        Self { spans }
    }
}

impl View for BufferStyleOverlay {
    fn on_edit(&mut self, _buf: &Buffer, edit: &Edit) -> Result<(), crate::buffer::BufferError> {
        let old_start = edit.range.start;
        let old_end = edit.range.end;
        let old_len = old_end - old_start;
        let new_len = edit.inserted_len;
        let mut spans = self.spans.lock().expect("style spans mutex poisoned");
        let mut adjusted = Vec::with_capacity(spans.len());
        for mut span in spans.drain(..) {
            if span.end <= old_start {
                adjusted.push(span);
            } else if span.start >= old_end {
                span.start = shift_pos(span.start, old_end, old_len, new_len);
                span.end = shift_pos(span.end, old_end, old_len, new_len);
                adjusted.push(span);
            }
            // Overlapping spans are dropped. REPL style spans are append-only
            // and scrollback truncation deletes whole old blocks, so a
            // conservative drop is simpler and avoids half-styled fragments.
        }
        *spans = adjusted;
        Ok(())
    }

    fn render(&mut self, buf: &Buffer, viewport: Viewport, cells: &mut CellGrid<'_>) {
        let spans = self
            .spans
            .lock()
            .expect("style spans mutex poisoned")
            .clone();
        if spans.is_empty() {
            return;
        }
        let line_offsets = compute_line_offsets(buf);
        if line_offsets.is_empty() {
            return;
        }
        let start_line = line_at_offset(&line_offsets, viewport.buffer_start);
        for span in spans {
            render_buffer_style_span(buf, &line_offsets, start_line, viewport, cells, span);
        }
    }
}

fn shift_pos(pos: u64, old_end: u64, old_len: u64, new_len: u64) -> u64 {
    if new_len >= old_len {
        pos + (new_len - old_len)
    } else {
        pos.saturating_sub(old_end)
            .saturating_add(old_end - (old_len - new_len))
    }
}

fn compute_line_offsets(buf: &Buffer) -> Vec<u64> {
    let mut offsets = vec![0];
    let rope = buf.snapshot_rope();
    let mut pos = 0;
    let len = rope.len();
    for chunk in rope.chunks(0, len) {
        for (i, b) in chunk.iter().enumerate() {
            if *b == b'\n' {
                offsets.push(pos + i as u64 + 1);
            }
        }
        pos += chunk.len() as u64;
    }
    offsets
}

fn line_at_offset(line_offsets: &[u64], offset: u64) -> usize {
    match line_offsets.binary_search(&offset) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    }
}

fn line_end(buf: &Buffer, line_offsets: &[u64], line: usize) -> u64 {
    let start = line_offsets[line];
    let raw_end = line_offsets.get(line + 1).copied().unwrap_or(buf.len());
    if line + 1 < line_offsets.len() && raw_end > start {
        raw_end - 1
    } else {
        raw_end
    }
}

fn display_col_for_range(buf: &Buffer, start: u64, end: u64) -> u32 {
    if end <= start {
        return 0;
    }
    let mut bytes = vec![0u8; (end - start) as usize];
    buf.snapshot_rope().slice(start, end, &mut bytes);
    while !bytes.is_empty() && std::str::from_utf8(&bytes).is_err() {
        bytes.pop();
    }
    let Ok(s) = std::str::from_utf8(&bytes) else {
        return 0;
    };
    let mut col = 0;
    for ch in s.chars() {
        let width = if ch == '\t' {
            8 - (col % 8)
        } else {
            UnicodeWidthChar::width(ch).unwrap_or(0) as u32
        };
        col += width;
    }
    col
}

fn render_buffer_style_span(
    buf: &Buffer,
    line_offsets: &[u64],
    start_line: usize,
    viewport: Viewport,
    cells: &mut CellGrid<'_>,
    span: BufferStyleSpan,
) {
    if span.start >= span.end {
        return;
    }
    let first_line = line_at_offset(line_offsets, span.start);
    let last_line = line_at_offset(line_offsets, span.end.saturating_sub(1));
    for line in first_line..=last_line {
        if line < start_line {
            continue;
        }
        let row_offset = (line - start_line) as u32;
        if row_offset >= viewport.cell_size.rows {
            break;
        }
        let line_start = line_offsets[line];
        let line_end = line_end(buf, line_offsets, line);
        let style_start = span.start.max(line_start).min(line_end);
        let style_end = span.end.min(line_end);
        if style_start >= style_end {
            continue;
        }
        let start_col = display_col_for_range(buf, line_start, style_start);
        let end_col = display_col_for_range(buf, line_start, style_end);
        let start_col = start_col.min(viewport.cell_size.cols);
        let end_col = end_col.min(viewport.cell_size.cols);
        for col in start_col..end_col {
            let coord = CellCoord::new(
                viewport.cell_origin.row + row_offset,
                viewport.cell_origin.col + col,
            );
            let cell = cells.at(coord);
            cell.style = merge_styles(cell.style, span.style);
        }
    }
}

// ---------------------------------------------------------------------------
// VirtualCellOverlay
// ---------------------------------------------------------------------------

/// One virtual cell to paint at a viewport-relative coordinate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VirtualCell {
    /// Row offset within the viewport.
    pub row: u32,
    /// Column offset within the viewport.
    pub col: u32,
    /// The cell to write.
    pub cell: Cell,
}

/// View that writes whole [`Cell`]s at declared positions, replacing
/// whatever the base view (or earlier overlays) put there.
///
/// Use this for inline blame, completion ghost text, breakpoint
/// markers, anything where the overlay genuinely owns the cell rather
/// than just decorating an existing glyph.
#[derive(Clone, Debug, Default)]
pub struct VirtualCellOverlay {
    /// Cells to paint. Later entries override earlier ones at shared
    /// coordinates.
    pub cells: Vec<VirtualCell>,
}

impl VirtualCellOverlay {
    /// Empty overlay.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append `vc`. Returns `&mut self` for chaining.
    pub fn add(&mut self, vc: VirtualCell) -> &mut Self {
        self.cells.push(vc);
        self
    }
}

impl View for VirtualCellOverlay {
    fn render(&mut self, _buf: &Buffer, viewport: Viewport, cells: &mut CellGrid<'_>) {
        for vc in &self.cells {
            if vc.row >= viewport.cell_size.rows || vc.col >= viewport.cell_size.cols {
                continue;
            }
            let coord = CellCoord::new(
                viewport.cell_origin.row + vc.row,
                viewport.cell_origin.col + vc.col,
            );
            *cells.at(coord) = vc.cell.clone();
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{Buffer, BufferId};
    use crate::cell::{Cell, CellSize, Glyph, UnderlineStyle};
    use crate::text_view::TextView;
    use crate::view::Viewport;

    fn make_grid(rows: u32, cols: u32) -> Vec<Cell> {
        vec![Cell::default(); (rows * cols) as usize]
    }

    fn viewport(rows: u32, cols: u32) -> Viewport {
        Viewport {
            buffer_start: 0,
            buffer_end: u64::MAX,
            cell_origin: CellCoord::new(0, 0),
            cell_size: CellSize::new(rows, cols),
        }
    }

    #[test]
    fn style_overlay_merges_into_existing_glyphs() {
        let buf = Buffer::from_bytes(BufferId::next(), "t", b"hello world\nsecond line\n");
        let mut text_view = TextView::new(&buf);
        let mut backing = make_grid(2, 20);
        let mut grid = CellGrid {
            cells: &mut backing,
            stride: 20,
            size: CellSize::new(2, 20),
        };
        text_view.render(&buf, viewport(2, 20), &mut grid);
        // Sanity: base view drew the glyphs.
        assert!(matches!(
            grid.get(CellCoord::new(0, 0)).glyph,
            Glyph::Char('h')
        ));

        let mut overlay = StyleSpanOverlay::new();
        overlay.add(StyleSpan {
            row: 0,
            start_col: 0,
            end_col: 5,
            style: Style {
                bold: true,
                underline: UnderlineStyle::Curly,
                ..Default::default()
            },
        });
        overlay.render(&buf, viewport(2, 20), &mut grid);

        // Glyph preserved, style merged in.
        for col in 0..5 {
            let c = grid.get(CellCoord::new(0, col));
            assert_eq!(
                c.glyph,
                Glyph::Char(['h', 'e', 'l', 'l', 'o'][col as usize])
            );
            assert!(c.style.bold);
            assert_eq!(c.style.underline, UnderlineStyle::Curly);
        }
        // Outside the span, style untouched.
        let c = grid.get(CellCoord::new(0, 5));
        assert_eq!(c.glyph, Glyph::Char(' '));
        assert!(!c.style.bold);
    }

    #[test]
    fn virtual_cell_overlay_replaces_existing_cells() {
        let buf = Buffer::from_bytes(BufferId::next(), "t", b"hi\n");
        let mut text_view = TextView::new(&buf);
        let mut backing = make_grid(1, 10);
        let mut grid = CellGrid {
            cells: &mut backing,
            stride: 10,
            size: CellSize::new(1, 10),
        };
        text_view.render(&buf, viewport(1, 10), &mut grid);

        let mut overlay = VirtualCellOverlay::new();
        overlay.add(VirtualCell {
            row: 0,
            col: 5,
            cell: Cell {
                glyph: Glyph::Char('V'),
                style: Style {
                    italic: true,
                    ..Default::default()
                },
                attachment: None,
            },
        });
        overlay.render(&buf, viewport(1, 10), &mut grid);

        let c = grid.get(CellCoord::new(0, 5));
        assert_eq!(c.glyph, Glyph::Char('V'));
        assert!(c.style.italic);
        // Adjacent cells untouched.
        assert_eq!(grid.get(CellCoord::new(0, 0)).glyph, Glyph::Char('h'));
        assert_eq!(grid.get(CellCoord::new(0, 1)).glyph, Glyph::Char('i'));
    }

    #[test]
    fn three_views_compose_deterministically() {
        // Acceptance bullet 1: text + style overlay + virtual cells
        // render correctly into the same grid.
        let buf = Buffer::from_bytes(BufferId::next(), "t", b"hello world\n");
        let mut text_view = TextView::new(&buf);
        let mut backing = make_grid(1, 20);
        let mut grid = CellGrid {
            cells: &mut backing,
            stride: 20,
            size: CellSize::new(1, 20),
        };

        // Layer 1: text.
        text_view.render(&buf, viewport(1, 20), &mut grid);

        // Layer 2: style overlay covering "hello".
        let mut style = StyleSpanOverlay::new();
        style.add(StyleSpan {
            row: 0,
            start_col: 0,
            end_col: 5,
            style: Style {
                bold: true,
                ..Default::default()
            },
        });
        style.render(&buf, viewport(1, 20), &mut grid);

        // Layer 3: virtual cell at col 12 (past "hello world").
        let mut virt = VirtualCellOverlay::new();
        virt.add(VirtualCell {
            row: 0,
            col: 12,
            cell: Cell {
                glyph: Glyph::Char('★'),
                style: Style {
                    italic: true,
                    ..Default::default()
                },
                attachment: None,
            },
        });
        virt.render(&buf, viewport(1, 20), &mut grid);

        // "hello" is bold, glyphs preserved.
        for col in 0..5 {
            assert!(grid.get(CellCoord::new(0, col)).style.bold);
        }
        // " world" is plain.
        for col in 5..11 {
            assert!(!grid.get(CellCoord::new(0, col)).style.bold);
        }
        // Virtual cell took col 12.
        assert_eq!(grid.get(CellCoord::new(0, 12)).glyph, Glyph::Char('★'));
        assert!(grid.get(CellCoord::new(0, 12)).style.italic);
    }

    #[test]
    fn later_overlay_wins_against_earlier_at_shared_cells() {
        // Acceptance bullet 2: composition order is deterministic.
        // Two style overlays, the second wins on overlap.
        let buf = Buffer::from_bytes(BufferId::next(), "t", b"abc\n");
        let mut tv = TextView::new(&buf);
        let mut backing = make_grid(1, 10);
        let mut grid = CellGrid {
            cells: &mut backing,
            stride: 10,
            size: CellSize::new(1, 10),
        };
        tv.render(&buf, viewport(1, 10), &mut grid);

        let mut first = StyleSpanOverlay::new();
        first.add(StyleSpan {
            row: 0,
            start_col: 0,
            end_col: 3,
            style: Style {
                underline: UnderlineStyle::Single,
                ..Default::default()
            },
        });
        let mut second = StyleSpanOverlay::new();
        second.add(StyleSpan {
            row: 0,
            start_col: 0,
            end_col: 3,
            style: Style {
                underline: UnderlineStyle::Curly,
                ..Default::default()
            },
        });
        first.render(&buf, viewport(1, 10), &mut grid);
        second.render(&buf, viewport(1, 10), &mut grid);

        // Second overlay's underline wins (later renders override).
        for col in 0..3 {
            assert_eq!(
                grid.get(CellCoord::new(0, col)).style.underline,
                UnderlineStyle::Curly
            );
        }
    }

    #[test]
    fn merge_styles_underline_color_non_default_wins() {
        use crate::cell::Color;
        let base = Style {
            fg: Color::Indexed(2),
            underline: UnderlineStyle::Single,
            underline_color: Color::Indexed(6),
            ..Default::default()
        };
        // Overlay sets its own underline color: it wins, while the
        // base's fg (syntax color) survives untouched.
        let diag_overlay = Style {
            underline: UnderlineStyle::Curly,
            underline_color: Color::Indexed(1),
            ..Default::default()
        };
        let merged = merge_styles(base, diag_overlay);
        assert_eq!(merged.underline_color, Color::Indexed(1));
        assert_eq!(merged.fg, Color::Indexed(2));
        // Overlay with default underline color: base's is preserved.
        let plain_overlay = Style {
            bold: true,
            ..Default::default()
        };
        let merged = merge_styles(base, plain_overlay);
        assert_eq!(merged.underline_color, Color::Indexed(6));
    }

    #[test]
    fn out_of_viewport_spans_are_silently_skipped() {
        let buf = Buffer::from_bytes(BufferId::next(), "t", b"x\n");
        let mut tv = TextView::new(&buf);
        let mut backing = make_grid(1, 5);
        let mut grid = CellGrid {
            cells: &mut backing,
            stride: 5,
            size: CellSize::new(1, 5),
        };
        tv.render(&buf, viewport(1, 5), &mut grid);

        let mut overlay = StyleSpanOverlay::new();
        overlay.add(StyleSpan {
            row: 99,
            start_col: 0,
            end_col: 100,
            style: Style {
                bold: true,
                ..Default::default()
            },
        });
        // Must not panic.
        overlay.render(&buf, viewport(1, 5), &mut grid);

        let mut virt = VirtualCellOverlay::new();
        virt.add(VirtualCell {
            row: 99,
            col: 99,
            cell: Cell::default(),
        });
        virt.render(&buf, viewport(1, 5), &mut grid);
    }
}
