//! Fixed-cell terminal paint planning (Vterm Stage 3).
//!
//! The document renderer derives geometry from shaped text: a glyph's
//! advance decides where the next glyph starts. A terminal cannot work
//! that way. Its column origins are defined by the CHILD, and the
//! frontend's font has no say in them — a wide glyph the font renders
//! 1.9 cells wide still occupies exactly two columns, and the cell after
//! it still starts at exactly `col * advance`.
//!
//! So this module resolves a [`TerminalFrame`] into a plan expressed in
//! CELLS, never pixels. Row/column rectangles own backgrounds,
//! underlines, selection, the cursor, and clipping; the renderer
//! multiplies by its own metrics at the end. That split is also what
//! makes the paint rules testable without a GPU: everything here is a
//! pure function of the frame plus two default colors.

use pmacs_protocol::{Cell, CellSize, Color, Glyph, Style, TerminalFrame, UnderlineStyle};

/// A resolved 24-bit color. The plan carries no `Default` sentinel:
/// resolution happens once, up front, because `reverse` swaps the
/// RESOLVED pair — a reversed default-on-default cell must come out as
/// dark-on-light, which is impossible if `Default` survives into the
/// swap.
pub type Rgb = [u8; 3];

/// The frontend defaults `Color::Default` resolves to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalPalette {
    /// The GPU's plain-text color.
    pub default_fg: Rgb,
    /// The GPU's window background.
    pub default_bg: Rgb,
}

/// A half-open run of cells on one row: `[start_col, end_col)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellRun {
    /// Row within the frame.
    pub row: u32,
    /// Inclusive first column.
    pub start_col: u32,
    /// Exclusive last column.
    pub end_col: u32,
}

/// A background run with its resolved (post-`reverse`) color.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackgroundRun {
    /// Cells covered.
    pub run: CellRun,
    /// Resolved fill.
    pub color: Rgb,
}

/// An underline run with its resolved color and form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnderlineRun {
    /// Cells covered.
    pub run: CellRun,
    /// Resolved stroke color.
    pub color: Rgb,
    /// Which underline form to draw.
    pub style: UnderlineStyle,
}

/// One shaped text run pinned to an explicit cell origin.
///
/// `cells` is the run's declared footprint. The renderer clips to it, so
/// a font whose glyph is wider than the cells the child allocated
/// overflows into a clip, never into the next column's origin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextRun {
    /// Row within the frame.
    pub row: u32,
    /// Origin column.
    pub col: u32,
    /// Declared width in cells.
    pub cells: u32,
    /// Text to shape.
    pub text: String,
    /// Resolved (post-`reverse`) foreground.
    pub color: Rgb,
    /// Bold via font attributes.
    pub bold: bool,
    /// Italic via font attributes.
    pub italic: bool,
}

/// Everything one terminal frame paints, in cell coordinates.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerminalPaintPlan {
    /// The frame's declared grid.
    pub size: CellSize,
    /// Background fills, coalesced across adjacent equal colors.
    pub backgrounds: Vec<BackgroundRun>,
    /// Text runs in row-major order.
    pub runs: Vec<TextRun>,
    /// Underline strokes, coalesced across adjacent equal color+form.
    pub underlines: Vec<UnderlineRun>,
    /// Editor-owned selection wash, one run per selected row.
    pub selection: Vec<CellRun>,
    /// The child's cursor cell, when visible.
    pub cursor: Option<CellRun>,
}

/// Resolve a cell's foreground and background, applying `reverse` after
/// both defaults have been substituted.
fn resolved_colors(style: &Style, palette: TerminalPalette) -> (Rgb, Rgb) {
    let fg = resolve_color(style.fg, palette.default_fg);
    let bg = resolve_color(style.bg, palette.default_bg);
    if style.reverse { (bg, fg) } else { (fg, bg) }
}

/// Map one wire color through the frontend's defaults and palette.
fn resolve_color(color: Color, default: Rgb) -> Rgb {
    match color {
        Color::Default => default,
        Color::Rgb(r, g, b) => [r, g, b],
        Color::Indexed(index) => indexed_rgb(index),
    }
}

/// Standard xterm-style 256-color palette: 16 base colors, the 6×6×6
/// cube (16..=231), then the 24-step grayscale ramp (232..=255).
///
/// Deliberately the same table the document path's `indexed_to_glyphon`
/// uses. Two palettes in one frontend would make an indexed diagnostic
/// and an indexed terminal cell disagree on what "red" is.
pub fn indexed_rgb(index: u8) -> Rgb {
    const ANSI16: [Rgb; 16] = [
        [0, 0, 0],
        [205, 49, 49],
        [13, 188, 121],
        [229, 229, 16],
        [36, 114, 200],
        [188, 63, 188],
        [17, 168, 205],
        [229, 229, 229],
        [102, 102, 102],
        [241, 76, 76],
        [35, 209, 139],
        [245, 245, 67],
        [59, 142, 234],
        [214, 112, 214],
        [41, 184, 219],
        [255, 255, 255],
    ];
    if index < 16 {
        return ANSI16[index as usize];
    }
    if (16..=231).contains(&index) {
        const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        let i = index - 16;
        return [
            STEPS[(i / 36) as usize],
            STEPS[((i / 6) % 6) as usize],
            STEPS[(i % 6) as usize],
        ];
    }
    let level = 8 + 10 * (index - 232);
    [level, level, level]
}

/// The style a cell paints with.
///
/// A continuation has no paint identity of its own: it is the second
/// column of the preceding glyph, so it inherits that lead's style. A
/// continuation that carried its own style would let the two halves of
/// one wide character disagree on background or underline.
fn paint_style(cells: &[Cell], index: usize, row_start: usize) -> &Style {
    if matches!(cells[index].glyph, Glyph::Continuation) && index > row_start {
        &cells[index - 1].style
    } else {
        &cells[index].style
    }
}

/// The text a cell contributes, or `None` for a continuation.
fn cell_text(cell: &Cell) -> Option<String> {
    match &cell.glyph {
        Glyph::Char(ch) => Some(ch.to_string()),
        // Validation already proved the cluster is UTF-8; a defensive
        // lossy decode keeps a future validator change from panicking
        // the renderer.
        Glyph::Cluster(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        Glyph::Continuation => None,
    }
}

/// Whether a cell can join an ASCII run.
///
/// Only single-column ASCII qualifies. Everything else — clusters, wide
/// leads, non-ASCII — gets its own explicitly positioned run, because
/// only for ASCII in a monospace face does the shaped advance reliably
/// equal the cell advance.
fn ascii_runnable(cell: &Cell) -> bool {
    matches!(&cell.glyph, Glyph::Char(ch) if ch.is_ascii_graphic() || *ch == ' ')
}

/// Whether a cell is a wide lead (its continuation follows).
fn is_wide_lead(cells: &[Cell], index: usize) -> bool {
    cells
        .get(index + 1)
        .is_some_and(|next| matches!(next.glyph, Glyph::Continuation))
}

impl TerminalPaintPlan {
    /// Resolve a validated frame into cell-space paint data.
    ///
    /// The frame must already have passed
    /// [`TerminalFrame::validate`]; this assumes the cell count, the
    /// wide-continuation topology, and the selection spans are sound.
    #[must_use]
    pub fn build(frame: &TerminalFrame, palette: TerminalPalette) -> Self {
        let cols = frame.size.cols as usize;
        let mut plan = Self {
            size: frame.size,
            ..Self::default()
        };
        if cols == 0 {
            return plan;
        }

        for row in 0..frame.size.rows {
            let row_start = row as usize * cols;
            let row_cells = &frame.cells[row_start..row_start + cols];
            plan.plan_row(row, row_cells, palette);
        }

        plan.selection = frame
            .selection
            .iter()
            .map(|span| CellRun {
                row: span.row,
                start_col: span.start_col,
                end_col: span.end_col,
            })
            .collect();

        plan.cursor = frame.cursor.map(|cursor| CellRun {
            row: cursor.row,
            start_col: cursor.col,
            end_col: cursor.col + 1,
        });

        plan
    }

    /// Plan one row's backgrounds, underlines, and text runs.
    #[allow(clippy::too_many_lines, reason = "one row's paint state machine")]
    fn plan_row(&mut self, row: u32, cells: &[Cell], palette: TerminalPalette) {
        // Backgrounds and underlines coalesce across the whole row;
        // text runs break on any attribute change AND on any glyph that
        // cannot be positioned by shaping.
        let mut bg_open: Option<(u32, Rgb)> = None;
        let mut ul_open: Option<(u32, Rgb, UnderlineStyle)> = None;
        let mut text_open: Option<(u32, String, Rgb, bool, bool)> = None;

        for col in 0..cells.len() {
            let style = paint_style(cells, col, 0);
            let (fg, bg) = resolved_colors(style, palette);
            let col_u32 = col as u32;

            match bg_open {
                Some((start, color)) if color != bg => {
                    self.backgrounds.push(BackgroundRun {
                        run: CellRun {
                            row,
                            start_col: start,
                            end_col: col_u32,
                        },
                        color,
                    });
                    bg_open = Some((col_u32, bg));
                }
                Some(_) => {}
                None => bg_open = Some((col_u32, bg)),
            }

            // A `Default` underline color follows the POST-reverse
            // foreground: on a reversed cell the underline must track
            // the color the glyph actually drew in, not the one the
            // child nominally set.
            let underline = (style.underline != UnderlineStyle::None).then(|| {
                let color = match style.underline_color {
                    Color::Default => fg,
                    other => resolve_color(other, fg),
                };
                (color, style.underline)
            });
            match (ul_open, underline) {
                (Some((start, color, form)), Some((next_color, next_form)))
                    if color != next_color || form != next_form =>
                {
                    self.underlines.push(UnderlineRun {
                        run: CellRun {
                            row,
                            start_col: start,
                            end_col: col_u32,
                        },
                        color,
                        style: form,
                    });
                    ul_open = Some((col_u32, next_color, next_form));
                }
                (Some((start, color, form)), None) => {
                    self.underlines.push(UnderlineRun {
                        run: CellRun {
                            row,
                            start_col: start,
                            end_col: col_u32,
                        },
                        color,
                        style: form,
                    });
                    ul_open = None;
                }
                (None, Some((color, form))) => ul_open = Some((col_u32, color, form)),
                (Some(_), Some(_)) | (None, None) => {}
            }

            let cell = &cells[col];
            let joinable = ascii_runnable(cell) && !is_wide_lead(cells, col);
            if joinable {
                match text_open.as_mut() {
                    Some((_, text, color, bold, italic))
                        if *color == fg && *bold == style.bold && *italic == style.italic =>
                    {
                        text.push_str(&cell_text(cell).unwrap_or_default());
                    }
                    _ => {
                        self.flush_text_run(row, text_open.take());
                        text_open = Some((
                            col_u32,
                            cell_text(cell).unwrap_or_default(),
                            fg,
                            style.bold,
                            style.italic,
                        ));
                    }
                }
                continue;
            }

            self.flush_text_run(row, text_open.take());
            let Some(text) = cell_text(cell) else {
                // A continuation draws nothing: its lead already
                // covers both columns.
                continue;
            };
            let cells_wide = if is_wide_lead(cells, col) { 2 } else { 1 };
            self.runs.push(TextRun {
                row,
                col: col_u32,
                cells: cells_wide,
                text,
                color: fg,
                bold: style.bold,
                italic: style.italic,
            });
        }

        let end = cells.len() as u32;
        if let Some((start, color)) = bg_open {
            self.backgrounds.push(BackgroundRun {
                run: CellRun {
                    row,
                    start_col: start,
                    end_col: end,
                },
                color,
            });
        }
        if let Some((start, color, form)) = ul_open {
            self.underlines.push(UnderlineRun {
                run: CellRun {
                    row,
                    start_col: start,
                    end_col: end,
                },
                color,
                style: form,
            });
        }
        self.flush_text_run(row, text_open);
    }

    fn flush_text_run(&mut self, row: u32, open: Option<(u32, String, Rgb, bool, bool)>) {
        let Some((col, text, color, bold, italic)) = open else {
            return;
        };
        let cells = text.chars().count() as u32;
        self.runs.push(TextRun {
            row,
            col,
            cells,
            text,
            color,
            bold,
            italic,
        });
    }
}

/// The terminal cell viewport a drawable rectangle admits.
///
/// Rows and columns are `floor(extent / metric)`, clamped through the
/// shared protocol limits so an enormous window cannot declare a grid
/// the daemon would reject. A rectangle too small for one whole cell
/// yields `None`: a zero-area declaration is not sent at all, and the
/// next geometry change that produces a valid size sends one.
#[must_use]
pub fn cell_viewport(
    width_px: f32,
    height_px: f32,
    advance_px: f32,
    line_px: f32,
) -> Option<CellSize> {
    if !(advance_px.is_finite() && line_px.is_finite()) || advance_px <= 0.0 || line_px <= 0.0 {
        return None;
    }
    if !(width_px.is_finite() && height_px.is_finite()) || width_px <= 0.0 || height_px <= 0.0 {
        return None;
    }
    let cols = (width_px / advance_px).floor();
    let rows = (height_px / line_px).floor();
    if cols < 1.0 || rows < 1.0 {
        return None;
    }
    let cols = (cols as u32).min(u32::from(pmacs_protocol::MAX_TERMINAL_COLS));
    let rows = (rows as u32).min(u32::from(pmacs_protocol::MAX_TERMINAL_ROWS));
    // The area bound can still bite at the extremes (512x512 is exactly
    // the cap, but a future limit change need not keep that true), so
    // shed rows rather than emit a size the daemon must reject.
    let max_rows = (pmacs_protocol::MAX_TERMINAL_VISIBLE_CELLS / cols as usize) as u32;
    let rows = rows.min(max_rows);
    if rows == 0 {
        return None;
    }
    Some(CellSize::new(rows, cols))
}

/// Hit-test a pixel inside the terminal rectangle to a cell.
///
/// Returns `None` outside the declared grid, so the status band, the
/// padding past the last full column, and any point above the terminal
/// origin are never terminal hits.
#[must_use]
pub fn hit_test_cell(
    x_px: f32,
    y_px: f32,
    origin: (f32, f32),
    advance_px: f32,
    line_px: f32,
    size: CellSize,
) -> Option<pmacs_protocol::CellCoord> {
    if advance_px <= 0.0 || line_px <= 0.0 {
        return None;
    }
    let dx = x_px - origin.0;
    let dy = y_px - origin.1;
    if dx < 0.0 || dy < 0.0 {
        return None;
    }
    let col = (dx / advance_px).floor();
    let row = (dy / line_px).floor();
    if col < 0.0 || row < 0.0 {
        return None;
    }
    let col = col as u32;
    let row = row as u32;
    if row >= size.rows || col >= size.cols {
        return None;
    }
    Some(pmacs_protocol::CellCoord::new(row, col))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pmacs_protocol::{BufferId, CellCoord, TerminalProcessState, TerminalSelectionSpan};

    const PALETTE: TerminalPalette = TerminalPalette {
        default_fg: [230, 230, 235],
        default_bg: [13, 13, 18],
    };

    fn styled(glyph: Glyph, style: Style) -> Cell {
        Cell {
            glyph,
            style,
            attachment: None,
        }
    }

    fn plain(ch: char) -> Cell {
        styled(Glyph::Char(ch), Style::default())
    }

    fn frame(rows: u32, cols: u32, cells: Vec<Cell>) -> TerminalFrame {
        TerminalFrame {
            buffer_id: BufferId::from_raw(1),
            size: CellSize::new(rows, cols),
            cells,
            cursor: None,
            title: None,
            screen_generation: 1,
            selection: Vec::new(),
            scroll_offset: 0,
            at_bottom: true,
            pid: 1,
            process: TerminalProcessState::Running,
        }
    }

    #[test]
    fn ascii_coalesces_by_attributes_and_keeps_explicit_origins() {
        let red = Style {
            fg: Color::Indexed(1),
            ..Style::default()
        };
        let cells = vec![
            plain('a'),
            plain('b'),
            styled(Glyph::Char('c'), red),
            plain('d'),
        ];
        let plan = TerminalPaintPlan::build(&frame(1, 4, cells), PALETTE);
        assert_eq!(plan.runs.len(), 3);
        assert_eq!(plan.runs[0].text, "ab");
        assert_eq!(plan.runs[0].col, 0);
        assert_eq!(plan.runs[0].cells, 2);
        assert_eq!(plan.runs[1].text, "c");
        assert_eq!(plan.runs[1].col, 2);
        assert_eq!(plan.runs[1].color, indexed_rgb(1));
        // The run after the attribute change is positioned by its own
        // column, not by where the previous run's glyphs happened to end.
        assert_eq!(plan.runs[2].col, 3);
    }

    #[test]
    fn wide_lead_owns_two_cells_and_its_continuation_draws_nothing() {
        let cyan = Style {
            bg: Color::Rgb(0, 40, 40),
            ..Style::default()
        };
        let cells = vec![
            styled(Glyph::Char('\u{4e00}'), cyan),
            styled(Glyph::Continuation, Style::default()),
            plain('x'),
            plain('y'),
        ];
        let plan = TerminalPaintPlan::build(&frame(1, 4, cells), PALETTE);
        let wide = &plan.runs[0];
        assert_eq!(wide.text, "\u{4e00}");
        assert_eq!(wide.col, 0);
        assert_eq!(wide.cells, 2, "a wide lead declares a two-cell footprint");
        assert_eq!(
            plan.runs[1].col, 2,
            "the run after a wide glyph starts at its own column"
        );
        assert!(
            plan.runs.iter().all(|run| run.col != 1),
            "a continuation contributes no text run"
        );
        // The continuation carries a DEFAULT style on the wire but must
        // paint with its lead's background.
        let covering = plan
            .backgrounds
            .iter()
            .find(|bg| bg.run.start_col == 0)
            .expect("lead background");
        assert_eq!(covering.color, [0, 40, 40]);
        assert_eq!(
            covering.run.end_col, 2,
            "the lead's background covers both of its columns"
        );
    }

    #[test]
    fn reverse_swaps_resolved_defaults_and_drives_the_underline_color() {
        let style = Style {
            reverse: true,
            underline: UnderlineStyle::Single,
            ..Style::default()
        };
        let plan =
            TerminalPaintPlan::build(&frame(1, 1, vec![styled(Glyph::Char('z'), style)]), PALETTE);
        assert_eq!(plan.backgrounds[0].color, PALETTE.default_fg);
        assert_eq!(plan.runs[0].color, PALETTE.default_bg);
        assert_eq!(
            plan.underlines[0].color, PALETTE.default_bg,
            "a default underline color follows the post-reverse foreground"
        );
    }

    #[test]
    fn explicit_underline_color_and_form_coalesce_then_break() {
        let curly = Style {
            underline: UnderlineStyle::Curly,
            underline_color: Color::Rgb(200, 0, 0),
            ..Style::default()
        };
        let mut dotted = curly;
        dotted.underline = UnderlineStyle::Dotted;
        let cells = vec![
            styled(Glyph::Char('a'), curly),
            styled(Glyph::Char('b'), curly),
            styled(Glyph::Char('c'), dotted),
            plain('d'),
        ];
        let plan = TerminalPaintPlan::build(&frame(1, 4, cells), PALETTE);
        assert_eq!(plan.underlines.len(), 2);
        assert_eq!(plan.underlines[0].run.start_col, 0);
        assert_eq!(plan.underlines[0].run.end_col, 2);
        assert_eq!(plan.underlines[0].style, UnderlineStyle::Curly);
        assert_eq!(plan.underlines[0].color, [200, 0, 0]);
        assert_eq!(plan.underlines[1].run.start_col, 2);
        assert_eq!(plan.underlines[1].run.end_col, 3);
        assert_eq!(plan.underlines[1].style, UnderlineStyle::Dotted);
    }

    #[test]
    fn bold_italic_and_truecolor_reach_the_run() {
        let style = Style {
            bold: true,
            italic: true,
            fg: Color::Rgb(1, 2, 3),
            ..Style::default()
        };
        let plan =
            TerminalPaintPlan::build(&frame(1, 1, vec![styled(Glyph::Char('q'), style)]), PALETTE);
        assert!(plan.runs[0].bold);
        assert!(plan.runs[0].italic);
        assert_eq!(plan.runs[0].color, [1, 2, 3]);
    }

    #[test]
    fn selection_and_cursor_come_only_from_the_frame() {
        let mut f = frame(2, 4, vec![plain('.'); 8]);
        f.selection = vec![TerminalSelectionSpan {
            row: 1,
            start_col: 1,
            end_col: 3,
        }];
        f.cursor = Some(CellCoord::new(0, 2));
        let plan = TerminalPaintPlan::build(&f, PALETTE);
        assert_eq!(
            plan.selection,
            vec![CellRun {
                row: 1,
                start_col: 1,
                end_col: 3
            }]
        );
        assert_eq!(
            plan.cursor,
            Some(CellRun {
                row: 0,
                start_col: 2,
                end_col: 3
            })
        );

        // A frame with no cursor paints none: visibility is the child's
        // decision, never the frontend's.
        f.cursor = None;
        assert!(TerminalPaintPlan::build(&f, PALETTE).cursor.is_none());
    }

    #[test]
    fn cluster_cells_get_their_own_positioned_run() {
        let cells = vec![
            plain('a'),
            styled(
                Glyph::Cluster("e\u{301}".as_bytes().to_vec().into_boxed_slice()),
                Style::default(),
            ),
            plain('b'),
        ];
        let plan = TerminalPaintPlan::build(&frame(1, 3, cells), PALETTE);
        assert_eq!(plan.runs.len(), 3);
        assert_eq!(plan.runs[1].text, "e\u{301}");
        assert_eq!(plan.runs[1].col, 1);
        assert_eq!(plan.runs[1].cells, 1);
        assert_eq!(plan.runs[2].col, 2);
    }

    #[test]
    fn rows_never_share_runs_or_background_spans() {
        let plan = TerminalPaintPlan::build(&frame(2, 2, vec![plain('x'); 4]), PALETTE);
        assert_eq!(
            plan.runs.len(),
            2,
            "one run per row, never wrapped together"
        );
        assert_eq!(plan.runs[0].row, 0);
        assert_eq!(plan.runs[1].row, 1);
        assert!(plan.backgrounds.iter().all(|bg| bg.run.end_col <= 2));
    }

    #[test]
    fn cell_viewport_floors_clamps_and_refuses_a_degenerate_rectangle() {
        assert_eq!(
            cell_viewport(100.0, 50.0, 10.0, 20.0),
            Some(CellSize::new(2, 10))
        );
        // Partial cells are dropped, never rounded up into a column the
        // child would write past.
        assert_eq!(
            cell_viewport(109.0, 59.0, 10.0, 20.0),
            Some(CellSize::new(2, 10))
        );
        assert_eq!(cell_viewport(9.0, 20.0, 10.0, 20.0), None);
        assert_eq!(cell_viewport(100.0, 19.0, 10.0, 20.0), None);
        assert_eq!(cell_viewport(100.0, 50.0, 0.0, 20.0), None);
        assert_eq!(cell_viewport(f32::NAN, 50.0, 10.0, 20.0), None);
        let huge = cell_viewport(1_000_000.0, 1_000_000.0, 1.0, 1.0).expect("clamped");
        assert_eq!(huge.rows, u32::from(pmacs_protocol::MAX_TERMINAL_ROWS));
        assert_eq!(huge.cols, u32::from(pmacs_protocol::MAX_TERMINAL_COLS));
    }

    #[test]
    fn hit_test_yields_only_in_bounds_cells() {
        let size = CellSize::new(3, 4);
        assert_eq!(
            hit_test_cell(16.0, 16.0, (16.0, 16.0), 10.0, 20.0, size),
            Some(CellCoord::new(0, 0))
        );
        assert_eq!(
            hit_test_cell(16.0 + 25.0, 16.0 + 41.0, (16.0, 16.0), 10.0, 20.0, size),
            Some(CellCoord::new(2, 2))
        );
        // Above / left of the origin, and past the last declared cell —
        // the status band and the trailing padding are not terminal hits.
        assert_eq!(
            hit_test_cell(15.0, 16.0, (16.0, 16.0), 10.0, 20.0, size),
            None
        );
        assert_eq!(
            hit_test_cell(16.0, 15.0, (16.0, 16.0), 10.0, 20.0, size),
            None
        );
        assert_eq!(
            hit_test_cell(16.0 + 40.0, 16.0, (16.0, 16.0), 10.0, 20.0, size),
            None
        );
        assert_eq!(
            hit_test_cell(16.0, 16.0 + 60.0, (16.0, 16.0), 10.0, 20.0, size),
            None
        );
    }
}
