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

use crate::buffer::Buffer;
use crate::cell::{Cell, CellCoord, CellGrid, Style};
use crate::display_width::byte_range_to_columns;
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

/// Identity of a span store: the allocation address, stable for the
/// `Arc`'s lifetime. Every overlay/translator over the same store
/// reports this via [`View::overlay_identity`], which is what makes
/// per-window attachment idempotent and disposal able to find every
/// window copy (PR #113 round-6 findings 1 and 3).
#[must_use]
pub fn style_store_identity(spans: &SharedBufferStyleSpans) -> usize {
    Arc::as_ptr(spans) as usize
}

/// View that renders buffer-byte style annotations.
///
/// Unlike [`StyleSpanOverlay`], this overlay stores byte ranges rather
/// than viewport cell ranges. That is the right shape for stream
/// consumers such as the REPL: ANSI SGR applies to bytes as they land
/// in the rope, and render maps the surviving ranges into visible cells.
///
/// RENDER-ONLY (PR #113 round-5 finding 1): this view deliberately
/// does not implement `on_edit`. Every window showing the buffer
/// holds its own copy over the SAME shared store, so a per-view
/// translation runs once per attached window — twice under a split,
/// zero times while the buffer is hidden. Coordinate translation
/// belongs to [`BufferStyleSpanTranslator`], attached to the buffer
/// itself.
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

/// Buffer-attached edit translator for a shared span store.
///
/// Keeps the byte coordinates in a [`SharedBufferStyleSpans`] store
/// in sync with buffer edits, EXACTLY ONCE per edit, independent of
/// how many windows currently render the buffer (PR #113 round-5
/// finding 1). Buffer-attached views receive `on_edit` on every
/// mutation path — intercept-skipping Lua writes, undo/redo, and
/// remote CRDT ops — whether or not the buffer is displayed
/// anywhere; window-attached [`BufferStyleOverlay`] copies are
/// render-only.
///
/// Translation preserves the untouched fragments of a span that
/// partially overlaps the edit (round-5 finding 2): bytes before the
/// replaced range keep their styling, bytes at/after it keep theirs
/// shifted by the edit's length delta, and only the bytes actually
/// replaced lose styling — the writer styles what it writes.
pub struct BufferStyleSpanTranslator {
    spans: SharedBufferStyleSpans,
}

impl BufferStyleSpanTranslator {
    /// Construct a translator over `spans`.
    #[must_use]
    pub fn new(spans: SharedBufferStyleSpans) -> Self {
        Self { spans }
    }
}

impl View for BufferStyleSpanTranslator {
    fn on_edit(&mut self, _buf: &Buffer, edit: &Edit) -> Result<(), crate::buffer::BufferError> {
        let old_start = edit.range.start;
        let old_end = edit.range.end;
        let old_len = old_end - old_start;
        let new_len = edit.inserted_len;
        // Buffers deliberately broadcast no-op edits (empty insert /
        // empty-range delete — buffer.rs's "callers that count the
        // call" contract). Nothing moved, so there is nothing to
        // translate; falling through would split any span containing
        // the position into two adjacent fragments per call —
        // unbounded growth for repeated no-ops, and a fragment
        // boundary mid-codepoint for a no-op at a continuation byte
        // (round-6 finding 2).
        if old_len == 0 && new_len == 0 {
            return Ok(());
        }
        let mut spans = self.spans.lock().expect("style spans mutex poisoned");
        let mut adjusted = Vec::with_capacity(spans.len());
        for span in spans.drain(..) {
            // Left fragment: bytes strictly before the replaced
            // range are untouched by the edit.
            if span.start < old_start {
                adjusted.push(BufferStyleSpan {
                    start: span.start,
                    end: span.end.min(old_start),
                    style: span.style,
                });
            }
            // Right fragment: bytes at/after the replaced range
            // survive, shifted by the length delta. (`pos >= old_end
            // >= old_len`, so the subtraction cannot underflow.)
            if span.end > old_end {
                adjusted.push(BufferStyleSpan {
                    start: span.start.max(old_end) - old_len + new_len,
                    end: span.end - old_len + new_len,
                    style: span.style,
                });
            }
            // A span entirely inside the replaced range produces
            // neither fragment and is dropped.
        }
        *spans = adjusted;
        Ok(())
    }

    fn kind(&self) -> &'static str {
        "buffer_style_span_translator"
    }

    fn overlay_identity(&self) -> Option<usize> {
        Some(style_store_identity(&self.spans))
    }
}

impl View for BufferStyleOverlay {
    fn kind(&self) -> &'static str {
        "buffer_style_overlay"
    }

    fn overlay_identity(&self) -> Option<usize> {
        Some(style_store_identity(&self.spans))
    }

    fn clone_for_split(&self) -> Option<Box<dyn View>> {
        Some(Box::new(self.clone()))
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
        let mut line_prefix = vec![0; (style_end - line_start) as usize];
        buf.snapshot_rope()
            .slice(line_start, style_end, &mut line_prefix);
        let (start_col, end_col) = byte_range_to_columns(
            &line_prefix,
            (style_start - line_start) as usize,
            line_prefix.len(),
        );
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
            gutter_w: 0,
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

    fn red() -> Style {
        Style {
            fg: crate::cell::Color::Indexed(1),
            ..Default::default()
        }
    }

    fn spans_of(store: &SharedBufferStyleSpans) -> Vec<(u64, u64)> {
        store
            .lock()
            .unwrap()
            .iter()
            .map(|s| (s.start, s.end))
            .collect()
    }

    #[test]
    fn translator_shifts_spans_exactly_once_regardless_of_render_views() {
        // PR #113 round-5 finding 1: N windows over the same store
        // must not translate N times, and zero windows must not mean
        // zero translations. The render-only overlays contribute
        // nothing to on_edit; the single buffer-attached translator
        // does it all.
        use crate::buffer::EditOp;
        use crate::rope::Range;
        let mut buf = Buffer::from_bytes(BufferId::next(), "t", b"abc");
        let store: SharedBufferStyleSpans = Arc::new(Mutex::new(vec![BufferStyleSpan {
            start: 1,
            end: 3,
            style: red(),
        }]));
        buf.attach_view(Box::new(BufferStyleSpanTranslator::new(Arc::clone(&store))));
        // Two render copies attached to the same buffer — the
        // split-window shape. Their on_edit is the default no-op.
        buf.attach_view(Box::new(BufferStyleOverlay::new(Arc::clone(&store))));
        buf.attach_view(Box::new(BufferStyleOverlay::new(Arc::clone(&store))));
        // Replace byte 0 with two bytes: delta +1, span after the
        // edit shifts by exactly one.
        buf.apply_edit(EditOp::Replace {
            range: Range::new(0, 1),
            bytes: b"XY",
        })
        .expect("edit applies");
        assert_eq!(
            spans_of(&store),
            vec![(2, 4)],
            "one translator, one shift — attachment count is irrelevant"
        );
    }

    #[test]
    fn translator_preserves_untouched_span_fragments() {
        // PR #113 round-5 finding 2: a partial overwrite must keep
        // styling on the bytes it never wrote. Replacing byte 0 of a
        // red [0,3) span (same length) leaves [1,3) red; the written
        // byte's styling is the writer's business.
        use crate::buffer::EditOp;
        use crate::rope::Range;
        let mut buf = Buffer::from_bytes(BufferId::next(), "t", b"abc");
        let store: SharedBufferStyleSpans = Arc::new(Mutex::new(vec![BufferStyleSpan {
            start: 0,
            end: 3,
            style: red(),
        }]));
        buf.attach_view(Box::new(BufferStyleSpanTranslator::new(Arc::clone(&store))));
        buf.apply_edit(EditOp::Replace {
            range: Range::new(0, 1),
            bytes: b"X",
        })
        .expect("edit applies");
        assert_eq!(
            spans_of(&store),
            vec![(1, 3)],
            "the untouched right fragment survives a same-length rewrite"
        );
    }

    #[test]
    fn translator_splits_a_span_around_an_interior_edit() {
        // Both fragments survive an interior replacement; the
        // replaced middle loses styling. Also pins the insertion
        // case: bytes inserted INSIDE a span do not inherit style.
        use crate::buffer::EditOp;
        use crate::rope::Range;
        let mut buf = Buffer::from_bytes(BufferId::next(), "t", b"abcdef");
        let store: SharedBufferStyleSpans = Arc::new(Mutex::new(vec![BufferStyleSpan {
            start: 0,
            end: 6,
            style: red(),
        }]));
        buf.attach_view(Box::new(BufferStyleSpanTranslator::new(Arc::clone(&store))));
        // Replace "cd" with "Z": left [0,2) intact, right [4,6)
        // shifts to [3,5).
        buf.apply_edit(EditOp::Replace {
            range: Range::new(2, 4),
            bytes: b"Z",
        })
        .expect("edit applies");
        assert_eq!(
            spans_of(&store),
            vec![(0, 2), (3, 5)],
            "left kept, right shifted by the length delta"
        );
        // A span wholly inside a replaced range is dropped.
        {
            let mut spans = store.lock().unwrap();
            spans.clear();
            spans.push(BufferStyleSpan {
                start: 1,
                end: 2,
                style: red(),
            });
        }
        buf.apply_edit(EditOp::Replace {
            range: Range::new(0, 4),
            bytes: b"....",
        })
        .expect("edit applies");
        assert_eq!(
            spans_of(&store),
            Vec::<(u64, u64)>::new(),
            "a fully-overwritten span produces no fragments"
        );
    }

    #[test]
    fn translator_splits_a_span_around_a_genuine_insertion() {
        // A real EditOp::Insert (not a replacement) inside a span:
        // the left fragment stays, the right fragment shifts by the
        // inserted length, and the inserted bytes inherit nothing.
        use crate::buffer::EditOp;
        let mut buf = Buffer::from_bytes(BufferId::next(), "t", b"abcdef");
        let store: SharedBufferStyleSpans = Arc::new(Mutex::new(vec![BufferStyleSpan {
            start: 0,
            end: 6,
            style: red(),
        }]));
        buf.attach_view(Box::new(BufferStyleSpanTranslator::new(Arc::clone(&store))));
        buf.apply_edit(EditOp::Insert {
            pos: 3,
            bytes: b"XY",
        })
        .expect("insert applies");
        assert_eq!(
            spans_of(&store),
            vec![(0, 3), (5, 8)],
            "insertion splits the span; inserted bytes are unstyled"
        );
    }

    #[test]
    fn translator_ignores_pure_noop_edits() {
        // Buffers deliberately broadcast no-op edits (empty insert /
        // empty-range delete). PR #113 round-6 finding 2: falling
        // through split a containing span into two adjacent
        // fragments per call — unbounded growth for repeated no-ops
        // at distinct positions, and a fragment boundary
        // mid-codepoint for a no-op at a UTF-8 continuation byte.
        use crate::buffer::EditOp;
        use crate::rope::Range;
        let mut buf = Buffer::from_bytes(BufferId::next(), "t", "ab\u{e9}def".as_bytes());
        let store: SharedBufferStyleSpans = Arc::new(Mutex::new(vec![BufferStyleSpan {
            start: 0,
            end: 7,
            style: red(),
        }]));
        buf.attach_view(Box::new(BufferStyleSpanTranslator::new(Arc::clone(&store))));
        // Distinct interior positions, including 3 — the é's
        // continuation byte.
        for pos in [1, 2, 3, 4, 5] {
            buf.apply_edit(EditOp::Insert { pos, bytes: b"" })
                .expect("no-op insert applies");
            buf.apply_edit(EditOp::Delete {
                range: Range::new(pos, pos),
            })
            .expect("no-op delete applies");
        }
        assert_eq!(
            spans_of(&store),
            vec![(0, 7)],
            "no-op edits must not fragment or move spans"
        );
    }
}
