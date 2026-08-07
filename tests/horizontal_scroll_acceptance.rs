//! Horizontal scroll acceptance (`QoL` Stage 4,
//! `docs/horizontal-scroll-framing.md`).
//!
//! Stage 3 shipped `ui.line-wrap`; under `truncate` the text past the
//! right edge was **unreachable**. Stage 4 makes it reachable by moving
//! the cursor — automatic only, no commands (Q#HS2).
//!
//! # The contract these tests are the oracle for
//!
//! `view_left` is an **unsnapped** per-window display column, and each
//! line derives its own **effective edge** (Q#HS7(c′)). A setter-time
//! snap was the first design and cannot exist: one column can bisect a
//! wide glyph on one line and be an ordinary boundary on the next, so no
//! single snapped value is canonical for every visible line.
//!
//! That is why the discriminating witness here is **multi-line with
//! differing glyph widths at the same column**. A single-line sweep
//! passes against the withdrawn design and proves nothing.

use pmacs::buffer::{Buffer, BufferId};
use pmacs::cell::{Cell, CellCoord, CellGrid, CellSize, Glyph};
use pmacs::text_view::TextView;
use pmacs::view::{DisplayCoord, LayoutCtx, View, Viewport, WrapMode};

fn attached(text: &[u8]) -> (Buffer, TextView) {
    let buf = Buffer::from_bytes(BufferId::next(), "test", text);
    let view = TextView::new(&buf);
    (buf, view)
}

fn ctx(cols: u32, wrap: WrapMode, view_left: u32) -> LayoutCtx {
    LayoutCtx {
        cols,
        wrap,
        view_left,
    }
}

/// Render and return each grid row's text.
fn rows_of(text: &[u8], rows: u32, cols: u32, wrap: WrapMode, view_left: u32) -> Vec<String> {
    let (buf, mut view) = attached(text);
    let mut storage = vec![Cell::default(); (rows * cols) as usize];
    let mut grid = CellGrid {
        cells: &mut storage,
        stride: cols,
        size: CellSize::new(rows, cols),
    };
    view.render(
        &buf,
        Viewport {
            buffer_start: 0,
            buffer_end: buf.len(),
            cell_origin: CellCoord::new(0, 0),
            cell_size: CellSize::new(rows, cols),
            gutter_w: 0,
            folds: None,
            wrap,
            view_left,
        },
        &mut grid,
    );
    (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| match storage[(r * cols + c) as usize].glyph {
                    Glyph::Char(ch) => ch,
                    // Rendered as a distinct marker so a test can tell
                    // "wide glyph's second cell" from "blank".
                    Glyph::Continuation => '\u{1}',
                    Glyph::Cluster(_) => ' ',
                })
                .collect()
        })
        .collect()
}

/// The report's own case: text past the edge becomes visible.
#[test]
fn scrolling_right_reveals_text_past_the_edge() {
    let text = b"ABCDEFGHIJKL";
    assert_eq!(rows_of(text, 1, 4, WrapMode::Truncate, 0)[0], "ABCD");
    assert_eq!(rows_of(text, 1, 4, WrapMode::Truncate, 4)[0], "EFGH");
    assert_eq!(
        rows_of(text, 1, 4, WrapMode::Truncate, 8)[0],
        "IJKL",
        "the tail of a long line is reachable, which is the whole of the \
         report Stage 3 could only half-answer"
    );
}

/// `view_left` must be **inert** under `wrap`, not merely harmless.
///
/// A wrapped line has nothing past the right edge, so an offset there
/// could only hide text nothing would scroll back to. Pinned to 0 by
/// `LayoutCtx::effective_left` rather than by callers remembering.
#[test]
fn wrap_ignores_a_horizontal_offset() {
    let text = b"ABCDEFGH";
    let unscrolled = rows_of(text, 2, 4, WrapMode::Wrap, 0);
    for offset in [1, 4, 7, 99] {
        assert_eq!(
            rows_of(text, 2, 4, WrapMode::Wrap, offset),
            unscrolled,
            "offset {offset} changed a wrapped render; it must be inert"
        );
    }
    assert_eq!(unscrolled[0], "ABCD");
    assert_eq!(unscrolled[1], "EFGH");
}

/// **The discriminating witness** (framing Q#HS7(c′)/(d)).
///
/// At one `view_left`, one line takes the straddle path and another the
/// ordinary path. A setter-time snap has a single value to choose and
/// must be wrong for one of these two lines; a per-line effective edge
/// is right for both.
#[test]
fn one_offset_straddles_on_one_line_and_not_another() {
    // Line 0: a wide glyph occupying columns 1-2, so column 2 bisects it.
    // Line 1: all narrow, so column 2 is an ordinary boundary.
    let text = "a\u{4e00}bcd\nabcd".as_bytes();
    let out = rows_of(text, 2, 3, WrapMode::Truncate, 2);

    assert_eq!(
        out[0].chars().next(),
        Some(' '),
        "the bisected glyph's trailing cell is a styled BLANK — not a \
         Continuation, which would name a leading cell nobody painted"
    );
    assert_eq!(
        &out[0][1..],
        "bc",
        "and the rest of that line follows it normally"
    );
    assert_eq!(
        out[1], "cd ",
        "the same offset on an all-narrow line is an ordinary boundary"
    );
}

/// The bisected glyph's trailing cell is designated to the glyph's
/// **start** byte, so clicking it selects the character it belongs to.
///
/// Forward-rounding here would designate the NEXT character and leave
/// the straddling glyph with no visible cell mapping to it at all —
/// unreachable exactly when it is what the user scrolled toward.
#[test]
fn the_bisected_cell_maps_back_to_its_own_glyph() {
    let (buf, view) = attached("a\u{4e00}bcd".as_bytes());
    let c = ctx(3, WrapMode::Truncate, 2);
    // 'a' is byte 0; the wide glyph is bytes 1..4.
    assert_eq!(
        view.display_to_pos(&buf, DisplayCoord::new(0, 0), c),
        Some(1),
        "screen column 0 is the wide glyph's trailing cell"
    );
    // …and the round trip: the glyph reports that same cell.
    assert_eq!(
        view.pos_to_display(&buf, 1, c).map(|d| d.col),
        Some(0),
        "place_of_byte designates cell 0, so byte_at_place inverts it"
    );
}

/// A tab straddling the edge keeps **forward** rounding (Q#HS7(c″)).
///
/// This is a REGRESSION witness, not a new claim: `display_to_pos`
/// already rounds forward for a column inside a tab's expansion. Stage 4
/// must not perturb it — and it would, if the walk were "optimized" to
/// start at the effective edge instead of column 0, because tab stops
/// are computed from the line start.
#[test]
fn a_straddling_tab_still_rounds_forward() {
    // Tab expands to columns 0..8 at the default tab width; 'x' is byte 1.
    let (buf, view) = attached(b"\txyz");
    let unscrolled =
        view.display_to_pos(&buf, DisplayCoord::new(0, 4), ctx(8, WrapMode::Truncate, 0));
    assert_eq!(unscrolled, Some(1), "precondition: forward rounding today");

    // Same absolute column 4, now reached as screen column 0 with the
    // expansion's leading cells scrolled off.
    assert_eq!(
        view.display_to_pos(&buf, DisplayCoord::new(0, 0), ctx(8, WrapMode::Truncate, 4)),
        Some(1),
        "scroll must not change where a tab-interior column lands"
    );
}

/// Tab stops are preserved because the walk still starts at column 0.
#[test]
fn tab_stops_survive_a_horizontal_offset() {
    // "a\tb": the tab advances to the next multiple of the tab width, so
    // 'b' sits at column 8 regardless of what is scrolled off.
    let full = rows_of(b"a\tb", 1, 12, WrapMode::Truncate, 0);
    assert_eq!(full[0].chars().nth(8), Some('b'), "precondition");

    let scrolled = rows_of(b"a\tb", 1, 12, WrapMode::Truncate, 6);
    assert_eq!(
        scrolled[0].chars().next(),
        Some(' '),
        "column 6 is still inside the tab's expansion"
    );
    assert_eq!(
        scrolled[0].chars().nth(2),
        Some('b'),
        "'b' is at absolute column 8, so screen column 8-6=2 — a walk \
         restarted at the edge would put it at 0"
    );
}

/// Round-trip identity at a non-zero offset, walked exhaustively — the
/// Q#HS7(d) invariant, on the ordinary (non-straddling) path.
#[test]
fn round_trip_is_identity_at_a_non_zero_offset() {
    let (buf, view) = attached(b"abcdefghij");
    let c = ctx(4, WrapMode::Truncate, 3);
    // Bytes 3.. are at or right of the edge; earlier ones are off-screen
    // and report None rather than clamping.
    for pos in 0..3u64 {
        assert_eq!(
            view.pos_to_display(&buf, pos, c),
            None,
            "byte {pos} is left of the edge: not visible, never clamped \
             to column 0 — clamping would make many bytes share one cell"
        );
    }
    for pos in 3..=10u64 {
        let coord = view.pos_to_display(&buf, pos, c).expect("visible");
        assert_eq!(
            view.display_to_pos(&buf, coord, c),
            Some(pos),
            "round trip must be identity at byte {pos}"
        );
    }
}

// ---------------------------------------------------------------------------
// Decorations must travel WITH the text (review P1)
//
// Stage 4's first commit translated the base glyph walk and nothing
// else. Every buffer-coordinate decorator — syntax/LSP styling,
// diagnostic underlines, search washes, `BufferStyleOverlay`, and the
// selection painter — kept clamping `start_col..end_col` straight onto
// `cell_origin.col`. At `view_left = 10` the glyph from source column 10
// painted at screen column 0 while its style painted at screen column 10
// or vanished: decorations drifting off the characters they describe,
// silently, and only once a window had been scrolled.
//
// `Viewport::visible_cols` is the one rule they now share, across FIVE
// adopters: four decorator families — syntax/LSP styling, diagnostic
// underlines, search washes, `BufferStyleOverlay` — plus the selection
// painter, whose own witnesses live in `src/editor.rs` because
// `paint_local_selection` is private.
//
// These witnesses pin the rule from three directions, because the
// decorator sites were textually identical and a single test would have
// let a missed adopter through.
// ---------------------------------------------------------------------------

use std::sync::{Arc, Mutex};

use pmacs::cell::Style;
use pmacs::overlay::{BufferStyleOverlay, BufferStyleSpan};

/// `(glyph, is_styled)` per cell of row 0 — decoration read against the
/// character it is supposed to be describing.
fn row0_with_styles(
    text: &[u8],
    cols: u32,
    view_left: u32,
    spans: Vec<BufferStyleSpan>,
) -> Vec<(char, bool)> {
    let (buf, mut view) = attached(text);
    let mut storage = vec![Cell::default(); cols as usize];
    let mut grid = CellGrid {
        cells: &mut storage,
        stride: cols,
        size: CellSize::new(1, cols),
    };
    let viewport = Viewport {
        buffer_start: 0,
        buffer_end: buf.len(),
        cell_origin: CellCoord::new(0, 0),
        cell_size: CellSize::new(1, cols),
        gutter_w: 0,
        folds: None,
        wrap: WrapMode::Truncate,
        view_left,
    };
    view.render(&buf, viewport, &mut grid);
    let store: pmacs::overlay::SharedBufferStyleSpans = Arc::new(Mutex::new(spans));
    let mut overlay = BufferStyleOverlay::new(store);
    overlay.render(&buf, viewport, &mut grid);
    (0..cols as usize)
        .map(|c| {
            let g = match storage[c].glyph {
                Glyph::Char(ch) => ch,
                _ => ' ',
            };
            (g, storage[c].style != Style::default())
        })
        .collect()
}

fn styled(start: u64, end: u64) -> BufferStyleSpan {
    BufferStyleSpan {
        start,
        end,
        style: Style {
            bold: true,
            ..Style::default()
        },
    }
}

/// A style span sits on the characters it names, at a non-zero offset.
#[test]
fn a_style_span_travels_with_its_characters() {
    // Style covers bytes 4..6 ("EF"), which scroll to screen columns 0..2.
    let out = row0_with_styles(b"ABCDEFGHIJ", 4, 4, vec![styled(4, 6)]);
    let text: String = out.iter().map(|(g, _)| *g).collect();
    assert_eq!(text, "EFGH", "precondition: the glyphs did translate");
    assert_eq!(
        out.iter().map(|(_, s)| *s).collect::<Vec<_>>(),
        vec![true, true, false, false],
        "the style must land on E and F — before the fix it painted at \
         absolute columns 4..6, i.e. screen columns 4..6, off this window"
    );
}

/// A span beginning off-screen and reaching into view is CLIPPED, not
/// dropped — the boundary the selection painter got wrong.
#[test]
fn a_span_starting_off_screen_still_paints_its_visible_tail() {
    // Style covers bytes 2..6 ("CDEF"); C and D are scrolled off.
    let out = row0_with_styles(b"ABCDEFGHIJ", 4, 4, vec![styled(2, 6)]);
    assert_eq!(
        out.iter().map(|(_, s)| *s).collect::<Vec<_>>(),
        vec![true, true, false, false],
        "the visible tail (E, F) must still be styled; skipping the whole \
         span because it starts left of the edge is the defect"
    );
}

/// And a span entirely left of the edge paints nothing.
#[test]
fn a_span_entirely_off_screen_paints_nothing() {
    let out = row0_with_styles(b"ABCDEFGHIJ", 4, 4, vec![styled(0, 3)]);
    assert!(
        out.iter().all(|(_, s)| !*s),
        "a span that ends before the edge must not paint — clamping it to \
         column 0 instead would smear it onto unrelated text"
    );
}

/// Under `wrap` the decorator translation is inert too, matching the
/// base walk.
#[test]
fn decorations_ignore_the_offset_under_wrap() {
    let (buf, _) = attached(b"ABCDEFGH");
    let _ = buf;
    let a = row0_with_styles(b"ABCDEFGH", 4, 0, vec![styled(0, 2)]);
    // Same span, non-zero offset, wrapping: `left_edge()` pins to 0.
    let (buf2, mut view2) = attached(b"ABCDEFGH");
    let mut storage = vec![Cell::default(); 4];
    let mut grid = CellGrid {
        cells: &mut storage,
        stride: 4,
        size: CellSize::new(1, 4),
    };
    let viewport = Viewport {
        buffer_start: 0,
        buffer_end: buf2.len(),
        cell_origin: CellCoord::new(0, 0),
        cell_size: CellSize::new(1, 4),
        gutter_w: 0,
        folds: None,
        wrap: WrapMode::Wrap,
        view_left: 4,
    };
    view2.render(&buf2, viewport, &mut grid);
    let store: pmacs::overlay::SharedBufferStyleSpans = Arc::new(Mutex::new(vec![styled(0, 2)]));
    let mut overlay = BufferStyleOverlay::new(store);
    overlay.render(&buf2, viewport, &mut grid);
    let wrapped: Vec<bool> = (0..4)
        .map(|c| storage[c].style != Style::default())
        .collect();
    assert_eq!(
        wrapped,
        a.iter().map(|(_, s)| *s).collect::<Vec<_>>(),
        "a wrapped render must ignore the offset for decorations exactly \
         as it does for glyphs"
    );
}
