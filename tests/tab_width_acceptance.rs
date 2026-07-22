//! Cross-frontend tab-stop acceptance for core/TUI rendering.

use std::sync::{Arc, Mutex};

use pmacs::buffer::{Buffer, BufferId};
use pmacs::cell::{Cell, CellCoord, CellGrid, CellSize, Glyph, Style};
use pmacs::overlay::{BufferStyleOverlay, BufferStyleSpan, SharedBufferStyleSpans};
use pmacs::text_view::TextView;
use pmacs::view::{DisplayCoord, View, Viewport};

fn viewport(rows: u32, cols: u32, buffer_end: u64) -> Viewport {
    Viewport {
        buffer_start: 0,
        buffer_end,
        cell_origin: CellCoord::new(0, 0),
        cell_size: CellSize::new(rows, cols),
        gutter_w: 0,
    }
}

fn render_text(buf: &Buffer, rows: u32, cols: u32) -> Vec<Cell> {
    let mut cells = vec![Cell::default(); (rows * cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: cols,
        size: CellSize::new(rows, cols),
    };
    TextView::new(buf).render(buf, viewport(rows, cols, buf.len()), &mut grid);
    cells
}

#[test]
fn plain_text_projects_tabs_without_changing_source_bytes() {
    let source = b"\tx\n1234567\ty\n12345678\tz";
    let buf = Buffer::from_bytes(BufferId::next(), "tabs", source);
    let cells = render_text(&buf, 3, 20);
    let view = TextView::new(&buf);
    assert_eq!(view.pos_to_display(&buf, 1), Some(DisplayCoord::new(0, 8)));
    assert_eq!(view.pos_to_display(&buf, 11), Some(DisplayCoord::new(1, 8)));
    assert_eq!(
        view.pos_to_display(&buf, 22),
        Some(DisplayCoord::new(2, 16))
    );

    for cell in cells.iter().take(8) {
        assert_eq!(cell.glyph, Glyph::Char(' '));
    }
    assert_eq!(cells[8].glyph, Glyph::Char('x'));
    assert_eq!(cells[20 + 8].glyph, Glyph::Char('y'));
    assert_eq!(cells[40 + 16].glyph, Glyph::Char('z'));

    let mut retained = vec![0; buf.len() as usize];
    buf.snapshot_rope().slice(0, buf.len(), &mut retained);
    assert_eq!(retained, source, "rendering must not replace source tabs");
}

#[test]
fn buffer_style_overlay_covers_the_same_expanded_tab_columns_as_plain_text() {
    let source = b"a\tb";
    let buf = Buffer::from_bytes(BufferId::next(), "styled-tab", source);
    let mut cells = render_text(&buf, 1, 12);
    let spans: SharedBufferStyleSpans = Arc::new(Mutex::new(vec![BufferStyleSpan {
        start: 1,
        end: 2,
        style: Style {
            bold: true,
            ..Style::default()
        },
    }]));
    let mut overlay = BufferStyleOverlay::new(spans);
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: 12,
        size: CellSize::new(1, 12),
    };
    overlay.render(&buf, viewport(1, 12, buf.len()), &mut grid);

    assert_eq!(grid.get(CellCoord::new(0, 0)).glyph, Glyph::Char('a'));
    assert!(!grid.get(CellCoord::new(0, 0)).style.bold);
    for col in 1..8 {
        let cell = grid.get(CellCoord::new(0, col));
        assert_eq!(cell.glyph, Glyph::Char(' '), "expanded tab column {col}");
        assert!(cell.style.bold, "overlay missed expanded tab column {col}");
    }
    assert_eq!(grid.get(CellCoord::new(0, 8)).glyph, Glyph::Char('b'));
    assert!(!grid.get(CellCoord::new(0, 8)).style.bold);
}
