// folding_stage2_acceptance.rs --- Arc 6 Stage 2 acceptance
// (docs/folding-stage2-framing.md, acceptance items 1–14).

//! Grid (daemon-rendered) collapse.
//!
//! Every claim about what the user sees is asserted on the **rendered
//! cell grid** through the real `paint_frame` — the same pipeline the
//! daemon ships to a terminal client — not on the fold store or on a
//! painter in isolation. Folds are created through the real
//! `pmacs.fold` data API so the stored ranges are normalized exactly as
//! a user command would leave them.
//!
//! The fixture buffer is twelve four-byte lines (`L00\n` … `L11\n`), so
//! line `n` starts at `4n` and its content ends at `4n + 3` — which is
//! precisely a fold's `ByteRange::start` for a head line `n`.

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseEvent, MouseEventKind,
};
use pmacs::buffer::{BufferId, EditOp};
use pmacs::cell::{Cell, CellCoord, CellGrid, CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;
use pmacs::window::{FrontendView, Layout, Window, WindowId};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Terminal geometry: 12 rows × 40 cols. `paint_frame` reserves the last
/// row for the status line and each window's last row for its mode line,
/// so a single window paints text into grid rows 0..=9.
const ROWS: u32 = 12;
const COLS: u32 = 40;
const TEXT_ROWS: u32 = 10;

/// Bytes per fixture line (`"Lnn\n"`).
const LINE_BYTES: u64 = 4;

fn fixture() -> String {
    (0..12).fold(String::new(), |mut acc, n| {
        use std::fmt::Write as _;
        let _ = writeln!(acc, "L{n:02}");
        acc
    })
}

/// A taller fixture for paging/scrolling: 80 five-byte lines
/// (`"Mnnn\n"`), so line `n` starts at `5n`.
fn long_fixture() -> String {
    (0..80).fold(String::new(), |mut acc, n| {
        use std::fmt::Write as _;
        let _ = writeln!(acc, "M{n:03}");
        acc
    })
}

/// Content-end byte of fixture line `n` — a fold's head `start`.
fn end_of(line: usize) -> u64 {
    line as u64 * LINE_BYTES + 3
}

fn editor() -> EditorState {
    let s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    s
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn active_id(s: &EditorState) -> BufferId {
    s.core.borrow().active_buffer_id()
}

/// Insert `text` at the start of `id` and notify every window, so the
/// per-window `TextView` line index matches what we are about to paint.
fn seed(s: &EditorState, id: BufferId, text: &str) {
    let edit = {
        let core = s.core.borrow();
        let registry = core.registry.clone();
        let mut reg = registry.borrow_mut();
        reg.get_mut(id)
            .unwrap()
            .apply_edit(EditOp::Insert {
                pos: 0,
                bytes: text.as_bytes(),
            })
            .unwrap()
    };
    s.core.borrow_mut().notify_buffer_edit(id, &edit);
}

/// A buffer holding the fixture, seeded into the active window.
fn seeded() -> (EditorState, BufferId) {
    let s = editor();
    let id = active_id(&s);
    seed(&s, id, &fixture());
    (s, id)
}

/// Collapse fixture lines `head + 1 ..= last_hidden` through the real
/// `pmacs.fold` data API. Panics if the range is rejected.
fn fold_lines(s: &EditorState, buffer: &str, head: usize, last_hidden: usize) {
    let ok: bool = eval(
        s,
        &format!(
            "return pmacs.fold.fold({buffer}, {{ start = {}, ['end'] = {} }})",
            end_of(head),
            end_of(last_hidden)
        ),
    );
    assert!(ok, "fold({head}..={last_hidden}) must be accepted");
}

/// Collapse in the *active* buffer.
fn fold_active(s: &EditorState, head: usize, last_hidden: usize) {
    fold_lines(s, "pmacs.window.buffer()", head, last_hidden);
}

/// Paint the full frame for `fid` and return the backing cells.
fn paint_for(state: &EditorState, fid: FrontendId) -> Vec<Cell> {
    let mut backing = vec![Cell::default(); (ROWS * COLS) as usize];
    let mut grid = CellGrid {
        cells: &mut backing,
        stride: COLS,
        size: CellSize::new(ROWS, COLS),
    };
    let _cursor = pmacs::editor::paint_frame(
        state,
        fid,
        &std::collections::HashMap::new(),
        &mut grid,
        CellSize::new(ROWS, COLS),
    );
    backing
}

fn paint(state: &EditorState) -> Vec<Cell> {
    paint_for(state, FrontendId::LOCAL)
}

/// Paint the full frame and return the terminal caret cell.
fn paint_caret(state: &EditorState) -> Option<CellCoord> {
    let mut backing = vec![Cell::default(); (ROWS * COLS) as usize];
    let mut grid = CellGrid {
        cells: &mut backing,
        stride: COLS,
        size: CellSize::new(ROWS, COLS),
    };
    pmacs::editor::paint_frame(
        state,
        FrontendId::LOCAL,
        &std::collections::HashMap::new(),
        &mut grid,
        CellSize::new(ROWS, COLS),
    )
}

fn at(cells: &[Cell], row: u32, col: u32) -> &Cell {
    &cells[(row * COLS + col) as usize]
}

fn row_text(cells: &[Cell], row: u32) -> String {
    (0..COLS)
        .map(|c| match at(cells, row, c).glyph {
            Glyph::Char(ch) => ch,
            _ => ' ',
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// The text rows of a single-window frame, trimmed.
fn text_rows(cells: &[Cell]) -> Vec<String> {
    (0..TEXT_ROWS).map(|r| row_text(cells, r)).collect()
}

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn ctrl(s: &mut EditorState, c: char) {
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char(c), KeyModifiers::CONTROL),
    );
}

fn alt(s: &mut EditorState, c: char) {
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::ALT));
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
}

fn cursor(s: &EditorState) -> u64 {
    s.core.borrow().active_window().cursor
}

fn cursor_line(s: &EditorState) -> usize {
    s.core.borrow().cursor_line()
}

fn set_cursor(s: &EditorState, pos: u64) {
    s.core.borrow_mut().set_cursor_byte(pos);
}

fn view_top(s: &EditorState) -> usize {
    s.core.borrow().active_window().view_top
}

fn fold_count(s: &EditorState) -> usize {
    let id = active_id(s);
    s.fold_registry.folds(id).len()
}

fn mouse(kind: MouseEventKind, row: u16, col: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

// ---------------------------------------------------------------------------
// 1. Collapse (framing acceptance 1)
// ---------------------------------------------------------------------------

#[test]
fn collapse_omits_hidden_lines_and_shifts_rows_up() {
    let (s, _id) = seeded();
    // Head = line 2, hidden = lines 3..=5.
    fold_active(&s, 2, 5);
    let cells = paint(&s);
    let rows = text_rows(&cells);

    assert_eq!(rows[0], "L00");
    assert_eq!(rows[1], "L01");
    assert_eq!(
        rows[2], "L02 …",
        "the head keeps its text plus the ellipsis"
    );
    // Lines 3..=5 are gone; the rows below shifted up.
    assert_eq!(rows[3], "L06");
    assert_eq!(rows[4], "L07");
    assert_eq!(rows[5], "L08");
    assert!(
        !rows
            .iter()
            .any(|r| r.starts_with("L03") || r.starts_with("L04") || r.starts_with("L05")),
        "no hidden line renders any row: {rows:?}"
    );
    // 12 source lines − 3 hidden = 9 content rows; row 9 is past the end.
    assert_eq!(rows[8], "L11");
    assert_eq!(
        rows[9], "",
        "content row count equals the visible-line count"
    );
}

#[test]
fn unfolded_frame_is_identical_to_the_pre_folding_baseline() {
    // The `None` map path must be byte-identical, not merely equivalent.
    let (s, _id) = seeded();
    let baseline = paint(&s);
    fold_active(&s, 2, 5);
    let folded = paint(&s);
    assert_ne!(baseline, folded, "the fixture actually folds");

    let (s2, _id2) = seeded();
    assert_eq!(baseline, paint(&s2), "no folds ⇒ unchanged rendering");
}

// ---------------------------------------------------------------------------
// 2. Head marker in both gutter states (framing acceptance 2, round-1 F3)
// ---------------------------------------------------------------------------

#[test]
fn head_marker_gutter_off_is_ellipsis_only() {
    let (s, _id) = seeded();
    fold_active(&s, 2, 5);
    // Line numbers default to Off ⇒ gutter_width() == 0 ⇒ no sign cell.
    let cells = paint(&s);
    assert_eq!(row_text(&cells, 2), "L02 …");
    assert_eq!(
        at(&cells, 2, 0).glyph,
        Glyph::Char('L'),
        "with no gutter the text still starts at column 0 — no width change"
    );
}

#[test]
fn head_marker_gutter_on_adds_the_fold_glyph() {
    let (s, _id) = seeded();
    fold_active(&s, 2, 5);
    exec(&s, "pmacs.window.set_line_numbers('absolute')");
    let cells = paint(&s);
    // 12 lines ⇒ 2 digits + 2 pad = 4-cell gutter; the fold glyph takes
    // the leading pad cell, the number is right-aligned before the
    // trailing pad, and the text begins at column 4.
    assert_eq!(
        at(&cells, 2, 0).glyph,
        Glyph::Char('▸'),
        "head row carries the gutter fold glyph"
    );
    assert_eq!(row_text(&cells, 2), "▸ 3 L02 …");
    // A non-head row has no glyph.
    assert_eq!(at(&cells, 1, 0).glyph, Glyph::Char(' '));
    assert_eq!(row_text(&cells, 1), "  2 L01");
}

#[test]
fn a_diagnostic_on_the_head_row_beats_the_fold_glyph() {
    let (s, _id) = seeded();
    fold_active(&s, 2, 5);
    exec(&s, "pmacs.window.set_line_numbers('absolute')");
    attach_diags(
        &s,
        vec![diag_on(pmacs::diag::DiagnosticSeverity::Error, 2, 2)],
    );
    let cells = paint(&s);
    assert_eq!(
        at(&cells, 2, 0).glyph,
        Glyph::Char('E'),
        "the diagnostic sign wins the shared cell (Q#FD20)"
    );
}

// ---------------------------------------------------------------------------
// 3. Line numbers (framing acceptance 3, Q#FD14)
// ---------------------------------------------------------------------------

#[test]
fn absolute_numbers_skip_hidden_lines() {
    let (s, _id) = seeded();
    fold_active(&s, 2, 5);
    exec(&s, "pmacs.window.set_line_numbers('absolute')");
    let cells = paint(&s);
    // Head shows 3, then the column jumps straight to 7 (line 6).
    assert_eq!(row_text(&cells, 2), "▸ 3 L02 …");
    assert_eq!(row_text(&cells, 3), "  7 L06");
    assert_eq!(row_text(&cells, 4), "  8 L07");
}

#[test]
fn relative_numbers_measure_visible_distance_across_a_fold() {
    let (s, _id) = seeded();
    fold_active(&s, 2, 5);
    set_cursor(&s, 0); // cursor on line 0
    exec(&s, "pmacs.window.set_line_numbers('relative')");
    let cells = paint(&s);
    // Visible order: 0, 1, 2(head), 6, 7 … so line 6 is 3 visible steps
    // from the cursor line, not 6 raw lines.
    assert_eq!(row_text(&cells, 0), "  0 L00");
    assert_eq!(row_text(&cells, 1), "  1 L01");
    assert_eq!(row_text(&cells, 2), "▸ 2 L02 …");
    assert_eq!(row_text(&cells, 3), "  3 L06");
    assert_eq!(row_text(&cells, 4), "  4 L07");
}

#[test]
fn hybrid_shows_absolute_on_the_visible_cursor_row() {
    let (s, _id) = seeded();
    fold_active(&s, 2, 5);
    // Cursor hidden on line 4 (a shared fold left it there).
    set_cursor(&s, end_of(4));
    exec(&s, "pmacs.window.set_line_numbers('hybrid')");
    let cells = paint(&s);
    // The anchor is the visible head (line 2), which shows absolute 3.
    assert_eq!(row_text(&cells, 2), "▸ 3 L02 …");
    assert_eq!(row_text(&cells, 1), "  1 L01");
    assert_eq!(row_text(&cells, 3), "  1 L06", "one visible step below");
}

// ---------------------------------------------------------------------------
// 4. Diagnostic clamp (framing acceptance 4, Q#FD15)
// ---------------------------------------------------------------------------

fn diag_on(
    severity: pmacs::diag::DiagnosticSeverity,
    line: u32,
    end_col: u32,
) -> pmacs::diag::Diagnostic {
    pmacs::diag::Diagnostic {
        start_line: line,
        start_col: 0,
        end_line: line,
        end_col,
        severity,
        message: "boom".into(),
        source: None,
        code: None,
    }
}

/// Attach the diagnostic overlay through the real Lua path and publish
/// `diags` for the active buffer.
fn attach_diags(state: &EditorState, diags: Vec<pmacs::diag::Diagnostic>) {
    let uri: String = eval(
        state,
        r#"
        local buf = pmacs.window.buffer()
        local uri = "file:///tmp/folding_stage2_diag.rs"
        assert(pmacs.diag._attach_view(buf, uri))
        return uri
        "#,
    );
    {
        let core = state.core.borrow();
        let registry = core.registry.clone();
        let mut reg = registry.borrow_mut();
        let buf = reg.get_mut(core.active_buffer_id()).unwrap();
        buf.set_file_path(Some(std::path::PathBuf::from(
            "/tmp/folding_stage2_diag.rs",
        )));
    }
    state
        .lsp_manager
        .borrow()
        .diag_store()
        .lock()
        .expect("diag store lock")
        .set(uri, diags);
}

#[test]
fn a_hidden_diagnostic_clamps_to_the_outermost_visible_head() {
    let (s, _id) = seeded();
    // Outer fold: head 0, hides 1..=9. Inner fold: head 3, hides 4..=6.
    fold_active(&s, 0, 9);
    fold_active(&s, 3, 6);
    exec(&s, "pmacs.window.set_line_numbers('absolute')");
    // A warning on the outer body and an ERROR on a NESTED inner line.
    attach_diags(
        &s,
        vec![
            diag_on(pmacs::diag::DiagnosticSeverity::Warning, 2, 3),
            diag_on(pmacs::diag::DiagnosticSeverity::Error, 5, 3),
        ],
    );
    let cells = paint(&s);
    assert_eq!(
        at(&cells, 0, 0).glyph,
        Glyph::Char('E'),
        "most-severe of the head and every line its fold hides, including \
         a nested inner-fold line, surfaces on the OUTERMOST visible head"
    );
    // Nothing leaks onto the rows below the collapse.
    assert_eq!(row_text(&cells, 1), " 11 L10");
    assert_eq!(at(&cells, 1, 0).glyph, Glyph::Char(' '));
}

// ---------------------------------------------------------------------------
// 5. Nested fold / shared cursor (framing acceptance 5, round-1 F1)
// ---------------------------------------------------------------------------

#[test]
fn nested_fold_with_a_deeply_hidden_cursor_resolves_outermost() {
    let (s, _id) = seeded();
    fold_active(&s, 0, 9); // outer: hides 1..=9
    fold_active(&s, 3, 6); // inner: head 3 is itself hidden
    // A second frontend folded through the shared store while this
    // window's logical cursor sat deep inside the nest.
    set_cursor(&s, end_of(5));
    exec(&s, "pmacs.window.set_line_numbers('relative')");

    let caret = paint_caret(&s).expect("caret is on screen");
    assert_eq!(caret.row, 0, "caret renders on the OUTERMOST visible head");

    let cells = paint(&s);
    assert_eq!(row_text(&cells, 0), "▸ 0 L00 …", "relative anchors there");
    assert_eq!(row_text(&cells, 1), "  1 L10");
}

#[test]
fn set_view_top_clamps_before_any_frame_is_painted() {
    // Round-4 F3: the SETTER establishes the invariant, not the renderer.
    // Between a `saveplace` restore (or `pmacs.editor.set_view_top`) and
    // the next frame, `view_top()` must never name a collapsed line and
    // command/event reckoning must never start from one.
    let (s, _id) = seeded();
    fold_active(&s, 2, 5);
    exec(&s, "pmacs.editor.set_view_top(4)"); // a hidden line
    assert_eq!(
        view_top(&s),
        2,
        "clamped backward to the visible head by the setter itself — \
         read here BEFORE any paint_frame call"
    );
    // A visible target is untouched, and the raw line-count clamp still
    // applies (Arc 3 Q#PS1).
    exec(&s, "pmacs.editor.set_view_top(7)");
    assert_eq!(view_top(&s), 7);
    exec(&s, "pmacs.editor.set_view_top(9999)");
    assert_eq!(view_top(&s), 12, "still clamped to the last line");
}

#[test]
fn a_view_top_inside_a_nest_clamps_backward_to_the_head() {
    let (s, _id) = seeded();
    fold_active(&s, 0, 9);
    fold_active(&s, 3, 6);
    s.core.borrow_mut().active_window_mut().view_top = 5;
    let cells = paint(&s);
    assert_eq!(
        view_top(&s),
        0,
        "clamped BACKWARD to the head, not forward past the fold"
    );
    assert_eq!(row_text(&cells, 0), "L00 …");
}

// ---------------------------------------------------------------------------
// 6. Caret / selection / peer column projection
//    (framing acceptance 6, round-2 F3 + round-3 F2)
// ---------------------------------------------------------------------------

#[test]
fn a_hidden_caret_lands_at_the_heads_end_of_content_column() {
    let s = editor();
    let id = active_id(&s);
    // Line 0 is deliberately SHORT and the hidden line is LONG, so a
    // row-only clamp would leave the caret at a column the head does not
    // even have.
    seed(&s, id, "ab\nxxxxxxxxxxxxxxxxxxxx\ncd\nef\n");
    let ok: bool = eval(
        &s,
        "return pmacs.fold.fold(pmacs.window.buffer(), { start = 2, ['end'] = 23 })",
    );
    assert!(ok);
    // Cursor at column 15 of the hidden line 1.
    set_cursor(&s, 3 + 15);
    let caret = paint_caret(&s).expect("caret on screen");
    assert_eq!(caret.row, 0, "the head row");
    assert_eq!(
        caret.col, 2,
        "the head's end-of-content column, never the raw hidden column"
    );
}

#[test]
fn crossing_folds_project_a_point_to_the_first_visible_head() {
    // Round-3 F2. A hides lines 1..=3 (head 0); B is headed on line 2 —
    // itself hidden by A — and hides 3..=5. A point on line 5 is directly
    // inside only B, whose `range.start` is hidden.
    let (s, _id) = seeded();
    fold_active(&s, 0, 3);
    fold_active(&s, 2, 5);
    assert_eq!(fold_count(&s), 2, "both crossing folds are stored");

    set_cursor(&s, end_of(5));
    let caret = paint_caret(&s).expect("caret on screen");
    assert_eq!(caret.row, 0, "A's visible head, never B's hidden start");
    assert_eq!(caret.col, 3, "L00's end-of-content column");

    let cells = paint(&s);
    let rows = text_rows(&cells);
    assert_eq!(rows[0], "L00 …");
    assert_eq!(rows[1], "L06", "lines 1..=5 all collapse under head 0");
}

#[test]
fn a_selection_spanning_a_fold_paints_only_visible_rows() {
    let (s, _id) = seeded();
    fold_active(&s, 2, 5);
    // Select from the middle of line 1 through the middle of line 7.
    {
        let mut core = s.core.borrow_mut();
        let aw = core.active_window_mut();
        aw.selection = Some(pmacs::window::Selection { anchor: 5 });
        aw.cursor = end_of(7) - 1;
    }
    let cells = paint(&s);
    let washed = |row: u32| {
        (0..COLS)
            .filter(|c| at(&cells, row, *c).style.reverse)
            .count()
    };
    assert!(washed(1) > 0, "row 1 (L01) is partly selected");
    assert!(washed(2) > 0, "row 2 (the visible head) is selected");
    assert!(washed(3) > 0, "row 3 (L06, shifted up) is selected");
    // Every painted row is a visible line; nothing renders for 3..=5 at
    // all, so there is no hidden row to wash.
    assert_eq!(row_text(&cells, 3), "L06");
    assert!(washed(5) == 0, "past the selection end");
}

#[test]
fn a_click_never_lands_on_a_hidden_line() {
    let (s, _id) = seeded();
    fold_active(&s, 2, 5);
    let mut s = s;
    // Grid row 3 shows L06 (line 6) after the collapse.
    s.dispatch_mouse(
        FrontendId::LOCAL,
        mouse(
            MouseEventKind::Down(crossterm::event::MouseButton::Left),
            3,
            0,
        ),
        CellSize::new(ROWS, COLS),
    );
    assert_eq!(cursor_line(&s), 6, "the 3rd VISIBLE line, not line 3");
}

#[test]
fn a_peer_cursor_on_a_hidden_line_clamps_to_the_head_position() {
    let (s, _id) = seeded();
    let id = active_id(&s);
    fold_active(&s, 2, 5);
    let presence = pmacs::overlay_paint::OtherPresence {
        frontend_id: FrontendId(2),
        snapshot: pmacs::presence::PresenceSnapshot {
            buffer_id: id,
            cursor: end_of(4), // hidden line 4
            selection: None,
        },
        color_slot: 0,
    };
    let mut backing = vec![Cell::default(); (ROWS * COLS) as usize];
    let mut grid = CellGrid {
        cells: &mut backing,
        stride: COLS,
        size: CellSize::new(ROWS, COLS),
    };
    pmacs::overlay_paint::paint_other_frontend_overlays(
        &s,
        &mut grid,
        CellSize::new(ROWS, COLS),
        &[presence],
    );
    // The peer paints on the head row at the head's end-of-content column.
    assert!(
        backing[(2 * COLS + 3) as usize].style.reverse,
        "peer cursor clamped onto the visible head row/column"
    );
    for row in 3..6u32 {
        assert!(
            (0..COLS).all(|c| !backing[(row * COLS + c) as usize].style.reverse),
            "nothing painted on a row a hidden line would have owned ({row})"
        );
    }
}

/// Paint the presence pass for `presences` over a fresh grid.
fn paint_presence(s: &EditorState, presences: &[pmacs::overlay_paint::OtherPresence]) -> Vec<Cell> {
    let mut backing = vec![Cell::default(); (ROWS * COLS) as usize];
    let mut grid = CellGrid {
        cells: &mut backing,
        stride: COLS,
        size: CellSize::new(ROWS, COLS),
    };
    pmacs::overlay_paint::paint_other_frontend_overlays(
        s,
        &mut grid,
        CellSize::new(ROWS, COLS),
        presences,
    );
    backing
}

fn peer(
    buffer_id: BufferId,
    cursor: u64,
    selection: Option<(u64, u64)>,
) -> pmacs::overlay_paint::OtherPresence {
    pmacs::overlay_paint::OtherPresence {
        frontend_id: FrontendId(2),
        snapshot: pmacs::presence::PresenceSnapshot {
            buffer_id,
            cursor,
            selection: selection
                .map(|(anchor, active)| pmacs::protocol::SelectionSnapshot { anchor, active }),
        },
        color_slot: 0,
    }
}

#[test]
fn peer_selection_endpoints_project_and_hidden_interiors_drop() {
    let (s, id) = seeded();
    fold_active(&s, 2, 5);
    // A peer selects from the middle of line 1 through the middle of
    // line 7 — straddling the whole collapse.
    let cells = paint_presence(&s, &[peer(id, end_of(7) - 1, Some((5, end_of(7) - 1)))]);
    let underlined = |row: u32| {
        (0..COLS)
            .filter(|c| at(&cells, row, *c).style.underline == pmacs::cell::UnderlineStyle::Single)
            .count()
    };
    assert!(underlined(1) > 0, "row 1 (L01) is partly selected");
    assert!(underlined(2) > 0, "row 2 (the visible head) is selected");
    assert!(underlined(3) > 0, "row 3 (L06, shifted up) is selected");
    assert!(underlined(4) > 0, "row 4 (L07) is selected");
    assert_eq!(underlined(5), 0, "past the selection end");
    // Rows 3 and 4 are L06/L07 — the hidden lines contributed no row at
    // all, so nothing could have painted "through" them.
    let frame = paint(&s);
    assert_eq!(row_text(&frame, 3), "L06");
    assert_eq!(row_text(&frame, 4), "L07");
}

#[test]
fn a_peer_inside_crossing_folds_projects_to_the_first_visible_head() {
    // Round-3 F2 repeated for the PEER path (framing acceptance 6):
    // A hides 1..=3 (head 0); B is headed on hidden line 2 and hides
    // 3..=5. A peer cursor and a peer selection endpoint on line 5 must
    // both resolve to A's head position, never B's hidden `range.start`.
    let (s, id) = seeded();
    fold_active(&s, 0, 3);
    fold_active(&s, 2, 5);

    let cells = paint_presence(&s, &[peer(id, end_of(5), None)]);
    assert!(
        at(&cells, 0, 3).style.reverse,
        "peer cursor on A's visible head row at its end-of-content column"
    );
    for row in 1..4u32 {
        assert!(
            (0..COLS).all(|c| !cells[(row * COLS + c) as usize].style.reverse),
            "nothing on a row a hidden line would have owned ({row})"
        );
    }

    // A selection whose START endpoint is hidden inside the crossing
    // region projects the same way. Visible order is 0(head), 6, 7, 8…,
    // so the projected start puts the selection's first cell on ROW 0 —
    // and that row is the discriminator: projecting to B's hidden
    // `range.start` (end of line 2) instead would paint nothing there.
    let sel = paint_presence(
        &s,
        &[peer(id, end_of(7) - 1, Some((end_of(5), end_of(7) - 1)))],
    );
    let underlined = |row: u32| {
        (0..COLS)
            .filter(|c| at(&sel, row, *c).style.underline == pmacs::cell::UnderlineStyle::Single)
            .count()
    };
    assert!(
        underlined(0) > 0,
        "the projected start lands on A's visible head row, not B's hidden one"
    );
    assert!(underlined(1) > 0, "L06, inside the projected span");
    assert!(underlined(2) > 0, "the visible tail (L07)");
    assert_eq!(underlined(3), 0, "L08 is past the selection end");
}

// ---------------------------------------------------------------------------
// 7. Ordinary overlays across a fold (framing acceptance 7)
// ---------------------------------------------------------------------------

#[test]
fn a_style_span_straddling_a_fold_paints_only_visible_rows() {
    let (s, _id) = seeded();
    fold_active(&s, 2, 5);
    // One buffer-byte span from the start of line 1 through the end of
    // line 7 — it covers three hidden lines on the way.
    exec(
        &s,
        &format!(
            "ov = pmacs.buffer.add_style_overlay(pmacs.window.buffer()); \
             ov:add({}, {}, {{ bg = 4 }})",
            LINE_BYTES,
            end_of(7)
        ),
    );
    let cells = paint(&s);
    let washed = |row: u32| {
        (0..COLS)
            .filter(|c| at(&cells, row, *c).style.bg == pmacs::cell::Color::Indexed(4))
            .count()
    };
    assert!(washed(1) > 0, "L01 is inside the span");
    assert!(washed(2) > 0, "the visible head is inside the span");
    assert!(washed(3) > 0, "L06 — correctly aligned on its SHIFTED row");
    assert!(washed(4) > 0, "L07");
    assert_eq!(washed(5), 0, "L08 is past the span's end");
    assert_eq!(washed(0), 0, "L00 is before the span's start");
    // The alignment claim: row 3 really is L06, so the wash landed on
    // the row the collapse put that line on, not on a raw offset.
    assert_eq!(row_text(&cells, 3), "L06");
}

#[test]
fn a_completion_popup_below_a_fold_anchors_on_the_visible_row() {
    let (s, _id) = seeded();
    fold_active(&s, 2, 5);
    let mut s = s;
    // Cursor at the end of line 7 — three collapsed lines above it, so
    // its VISIBLE row is 4 (0, 1, 2·head, 6, 7) though its raw line is 7.
    set_cursor(&s, end_of(7));
    // A separator first, so the word at point is the prefix itself.
    type_str(&mut s, " L1"); // dabbrev prefix of L10 / L11
    let visible: bool = eval(&s, "return pmacs.completion.popup_visible()");
    assert!(visible, "the popup opened off dabbrev");
    assert_eq!(fold_count(&s), 1, "typing below the fold left it closed");

    let cells = paint(&s);
    // Row 5 is where the popup's first row belongs. Without a fold-aware
    // anchor the popup would have gone to raw row 8 and row 5 would still
    // show source line 8.
    assert_ne!(
        row_text(&cells, 5),
        "L08",
        "the popup covers the row directly below the anchor's VISIBLE row"
    );
    assert!(
        row_text(&cells, 5).contains("L1"),
        "and that row holds a candidate: {:?}",
        row_text(&cells, 5)
    );
    assert_eq!(row_text(&cells, 4), "L07 L1", "the anchor row itself");
}

#[test]
fn a_search_wash_across_a_fold_paints_only_visible_rows_correctly() {
    let (s, _id) = seeded();
    fold_active(&s, 2, 5);
    let mut s = s;
    // Search for "L0" — matches on lines 0..=9, several of them hidden.
    ctrl(&mut s, 's');
    type_str(&mut s, "L07");
    press(&mut s, KeyCode::Enter);

    let cells = paint(&s);
    // L07 is on grid row 4 after the collapse (0,1,2,L06,L07).
    assert_eq!(row_text(&cells, 4), "L07");
    let washed_row_4 = (0..COLS)
        .filter(|c| at(&cells, 4, *c).style != pmacs::cell::Style::default())
        .count();
    assert!(
        washed_row_4 > 0,
        "the match washes the row the folded frame actually put it on"
    );
    // Row 1 (L01) holds no match and stays unwashed.
    assert!(
        (0..COLS).all(|c| at(&cells, 1, c).style == pmacs::cell::Style::default()),
        "no wash bleeds onto an unmatched visible row"
    );
}

// ---------------------------------------------------------------------------
// 8. Viewport / paging / indicator (framing acceptance 8, Q#FD18)
// ---------------------------------------------------------------------------

#[test]
fn page_down_advances_by_a_screenful_of_visible_lines() {
    let s = editor();
    let id = active_id(&s);
    let long = long_fixture();
    seed(&s, id, &long);
    // Hide lines 1..=40 under head 0.
    let ok: bool = eval(
        &s,
        "return pmacs.fold.fold(pmacs.window.buffer(), { start = 4, ['end'] = 204 })",
    );
    assert!(ok);
    let _ = paint(&s); // establish last_visible_rows
    let before = cursor_line(&s);
    assert_eq!(before, 0);
    s.core.borrow_mut().move_page_down();
    let after = cursor_line(&s);
    assert!(
        after > 40,
        "a screenful of VISIBLE lines steps past the whole collapse (landed on {after})"
    );
    assert!(
        !s.fold_registry
            .folds(id)
            .iter()
            .any(|f| f.start < end_of_line_start(after) && end_of_line_start(after) <= f.end),
        "the cursor never comes to rest on a hidden line"
    );
}

/// Byte at the start of fixture-independent line `n` for a 5-byte line
/// fixture (`"Mnnn\n"`).
fn end_of_line_start(line: usize) -> u64 {
    line as u64 * 5
}

#[test]
fn the_mode_line_indicator_reckons_in_visible_lines() {
    let (s, _id) = seeded();
    // 12 lines in a 10-row window: not All.
    let plain = paint(&s);
    assert!(
        row_text(&plain, 10).contains("Top"),
        "unfolded 12 lines in 10 rows reads Top: {}",
        row_text(&plain, 10)
    );
    // Collapse 3 lines away → 9 visible lines fit in 10 rows → All.
    fold_active(&s, 2, 5);
    let folded = paint(&s);
    assert!(
        row_text(&folded, 10).contains("All"),
        "9 visible lines fit the viewport: {}",
        row_text(&folded, 10)
    );
}

#[test]
fn goto_line_into_a_fold_leaves_its_head_visible() {
    let s = editor();
    let id = active_id(&s);
    let long = long_fixture();
    seed(&s, id, &long);
    // Hide lines 51..=70 under head 50.
    let ok: bool = eval(
        &s,
        "return pmacs.fold.fold(pmacs.window.buffer(), { start = 254, ['end'] = 354 })",
    );
    assert!(ok);
    s.core.borrow_mut().move_to_line(60); // a hidden line
    let _ = paint(&s);
    let top = view_top(&s);
    assert!(top <= 50, "view_top is at or above the head (got {top})");
    assert!(
        top + 10 > 50,
        "and the head itself is inside the viewport (top {top})"
    );
    assert_eq!(fold_count(&s), 1, "goto-line does NOT auto-unfold");
}

// ---------------------------------------------------------------------------
// 9. Vertical motion (framing acceptance 9, Q#FD17)
// ---------------------------------------------------------------------------

#[test]
fn next_line_steps_across_a_collapsed_region_in_one_motion() {
    let (s, _id) = seeded();
    fold_active(&s, 2, 5);
    let mut s = s;
    set_cursor(&s, 0);
    press(&mut s, KeyCode::Down);
    assert_eq!(cursor_line(&s), 1);
    press(&mut s, KeyCode::Down);
    assert_eq!(cursor_line(&s), 2, "the head");
    press(&mut s, KeyCode::Down);
    assert_eq!(cursor_line(&s), 6, "one step clears the whole collapse");
    press(&mut s, KeyCode::Up);
    assert_eq!(cursor_line(&s), 2, "and one step back returns to the head");
}

/// Lines of deliberately UNEQUAL width, so a normalization that only
/// clamps the row (carrying the hidden line's raw column 12) lands on a
/// different byte than one that projects the whole position (head column
/// 2) — in BOTH directions.
///
/// ```text
/// line 0: "ab"                 bytes  0..=1   end  2
/// line 1: "PQRSTUVWXY"  (10)   bytes  3..=12  end 13
/// line 2: "ef"                 bytes 14..=15  end 16   <- head
/// line 3: "0123456789ABCDEF"   bytes 17..=32  end 33   <- hidden
/// line 4: "wxyz"               bytes 34..=37  end 38   <- hidden
/// line 5: "ghijklmn"    (8)    bytes 39..=46  end 47
/// ```
const RAGGED: &str = "ab\nPQRSTUVWXY\nef\n0123456789ABCDEF\nwxyz\nghijklmn\n";

fn ragged() -> (EditorState, BufferId) {
    let s = editor();
    let id = active_id(&s);
    seed(&s, id, RAGGED);
    (s, id)
}

/// Column 12 of hidden line 3.
const DEEP: u64 = 17 + 12;

#[test]
fn motion_from_a_hidden_cursor_normalizes_the_whole_position() {
    let (s, _id) = ragged();
    // Head = line 2 ("ef", ends at byte 16); hidden = lines 3..=4.
    fold_range(&s, 16, 38);
    let mut s = s;

    // A shared fold left the logical cursor at column 12 of hidden line
    // 3 — a column line 2 does not even have.
    set_cursor(&s, DEEP);
    press(&mut s, KeyCode::Down);
    assert_eq!(cursor_line(&s), 5, "normalize to head 2, then step down");
    assert_eq!(
        cursor(&s),
        41,
        "the goal column came from the HEAD's end of content (2), not the \
         hidden line's raw column (12): line 5 is 8 wide, so a raw column \
         would have clamped to its end (47)"
    );

    set_cursor(&s, DEEP);
    press(&mut s, KeyCode::Up);
    assert_eq!(cursor_line(&s), 1, "normalize to head 2, then step up");
    assert_eq!(
        cursor(&s),
        5,
        "column 2 of line 1; a raw column 12 would have clamped to 13"
    );
}

#[test]
fn a_boundary_step_still_leaves_a_hidden_cursor_visible() {
    // Round-4 F2: with the fold headed on line 0 there is nowhere to step
    // up TO, but the normalization must still have happened — returning
    // early would leave the logical cursor hidden.
    let (s, _id) = ragged();
    // Head = line 0 ("ab", ends at byte 2); hidden = lines 1..=4.
    fold_range(&s, 2, 38);
    let mut s = s;
    set_cursor(&s, DEEP); // deep inside, on hidden line 3

    press(&mut s, KeyCode::Up);
    assert_eq!(
        cursor_line(&s),
        0,
        "no step available, but still normalized"
    );
    assert_eq!(cursor(&s), 2, "at the head's end of content");
    let folds = s.fold_registry.folds(active_id(&s));
    assert!(
        !folds
            .iter()
            .any(|f| f.start < cursor(&s) && cursor(&s) <= f.end),
        "the cursor is no longer inside any fold"
    );
}

#[test]
fn paging_from_a_hidden_cursor_normalizes_too() {
    // Same class: paging shares the normalization, so a page from a
    // hidden cursor also starts from the head's column.
    let (s, _id) = ragged();
    fold_range(&s, 16, 38);
    set_cursor(&s, DEEP);
    s.core.borrow_mut().move_page_up();
    assert_eq!(cursor_line(&s), 0);
    assert_eq!(cursor(&s), 2, "head column 2, not the hidden line's 12");
}

// ---------------------------------------------------------------------------
// 10. Interactive unfold widening (framing acceptance 10, Q#FD19)
// ---------------------------------------------------------------------------

#[test]
fn yank_at_a_point_inside_a_fold_unfolds_it() {
    let (s, _id) = seeded();
    let mut s = s;
    // Kill line 0's text so the kill ring has content.
    set_cursor(&s, 0);
    ctrl(&mut s, 'k');
    // Re-fold with the shortened buffer: head 2, hidden 3..=5 of the
    // remaining text. Recompute from the live line index.
    let (head_end, last_end) = line_ends(&s, 2, 5);
    fold_range(&s, head_end, last_end);
    assert_eq!(fold_count(&s), 1);

    // Put point INSIDE the fold and yank.
    set_cursor(&s, last_end);
    ctrl(&mut s, 'y');
    assert_eq!(
        fold_count(&s),
        0,
        "yank at a point inside a fold unfolds it"
    );
}

#[test]
fn query_replace_inside_a_fold_unfolds_it() {
    let (s, _id) = seeded();
    let mut s = s;
    fold_active(&s, 2, 5);
    assert_eq!(fold_count(&s), 1);
    set_cursor(&s, 0);
    // M-% L04 RET Q04 RET, then `y` to replace the first match — which
    // is on hidden line 4.
    alt(&mut s, '%');
    type_str(&mut s, "L04");
    press(&mut s, KeyCode::Enter);
    type_str(&mut s, "Q04");
    press(&mut s, KeyCode::Enter);
    type_str(&mut s, "y");
    assert_eq!(
        fold_count(&s),
        0,
        "the replacement's edit point was inside the fold"
    );
}

/// Define a command whose body is `body`, then run it the way a user
/// does — `M-x <name> RET`. The **Rust** dispatch path is what installs
/// the `InteractiveCommandOrigin` scope (`pmacs.command.invoke_interactive`
/// alone does not), so the widening must be driven through it.
fn define(s: &EditorState, name: &str, body: &str) {
    exec(
        s,
        &format!(
            "pmacs.command.define {{ name = '{name}', \
             description = 'stage 2 test', fn = function() {body} end }}"
        ),
    );
}

fn m_x(s: &mut EditorState, name: &str) {
    alt(s, 'x');
    type_str(s, name);
    press(s, KeyCode::Enter);
}

#[test]
fn an_interactive_lua_mutator_unfolds_at_the_edit_site() {
    let (s, _id) = seeded();
    let mut s = s;
    fold_active(&s, 2, 5); // head 2, hidden 3..=5; fold range (11, 23]

    // A PROGRAMMATIC data-API edit inside the fold: no interactive
    // command in scope, so it stays hidden (Stage 1's exemption).
    exec(
        &s,
        &format!("pmacs.window.buffer():insert({}, 'x')", end_of(4)),
    );
    assert_eq!(
        fold_count(&s),
        1,
        "a bare data-API mutation stays programmatic — no unfold"
    );

    // The same edit from INSIDE an interactive command unfolds.
    define(
        &s,
        "test.poke",
        &format!("pmacs.window.buffer():insert({}, 'y')", end_of(4)),
    );
    set_cursor(&s, 0); // point OUTSIDE the fold — the edit site is what counts
    m_x(&mut s, "test.poke");
    assert_eq!(
        fold_count(&s),
        0,
        "Q#FD19 keys on edit.range.start: a command editing INTO a fold \
         from an outside point must reveal what it wrote"
    );
}

#[test]
fn an_interactive_edit_outside_the_fold_leaves_it_closed() {
    // The other half of round-4 F1: keying on the POINT would open an
    // unrelated fold whenever a command edits somewhere else.
    let (s, _id) = seeded();
    let mut s = s;
    fold_active(&s, 2, 5);
    set_cursor(&s, end_of(4)); // point INSIDE the fold …
    define(&s, "test.elsewhere", "pmacs.window.buffer():insert(0, 'x')");
    m_x(&mut s, "test.elsewhere"); // … but the edit lands at byte 0
    assert_eq!(
        fold_count(&s),
        1,
        "an edit outside the fold must not open it just because the \
         cursor happens to sit inside"
    );
}

/// Attach a buffer intercept that relocates every `insert` to `to`.
/// This is a supported, documented rewrite (`pos` / `start` / `end` may
/// be overridden), and it is what makes the *requested* op's position an
/// unreliable answer to "where will this edit land?".
fn relocate_inserts_to(s: &EditorState, to: u64) {
    exec(
        s,
        &format!(
            r#"
            pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
              if op.kind == "insert" then
                return {{ kind = "insert", pos = {to} }}
              end
              return nil
            end)
            "#
        ),
    );
}

#[test]
fn an_intercept_relocating_the_edit_into_a_fold_unfolds_it() {
    // Round-5 F1. The command requests an edit OUTSIDE the fold; a
    // managed intercept moves it INSIDE. Keying on the requested op
    // would leave what the user just wrote invisible.
    let (s, _id) = seeded();
    let mut s = s;
    fold_active(&s, 2, 5); // head 2, hidden 3..=5 — fold range (11, 23]
    relocate_inserts_to(&s, end_of(4)); // 19, inside the fold
    set_cursor(&s, 0);
    define(
        &s,
        "test.relocate_in",
        "pmacs.window.buffer():insert(0, 'y')",
    );
    m_x(&mut s, "test.relocate_in");

    let text: String = eval(
        &s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    assert!(
        text.starts_with("L00\nL01\nL02\nL03\nL04y"),
        "the intercept really relocated the insert: {text:?}"
    );
    assert_eq!(
        fold_count(&s),
        0,
        "the EFFECTIVE edit site was inside the fold, so it must be revealed"
    );
}

#[test]
fn an_intercept_relocating_the_edit_out_of_a_fold_leaves_it_closed() {
    // The other direction: requested INSIDE, intercept moves it OUTSIDE.
    // Keying on the requested op would open a fold nothing was written to.
    let (s, _id) = seeded();
    let mut s = s;
    fold_active(&s, 2, 5);
    relocate_inserts_to(&s, 0); // outside the fold
    set_cursor(&s, 0);
    define(
        &s,
        "test.relocate_out",
        &format!("pmacs.window.buffer():insert({}, 'y')", end_of(4)),
    );
    m_x(&mut s, "test.relocate_out");

    let text: String = eval(
        &s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    assert!(
        text.starts_with("yL00"),
        "the intercept really relocated the insert: {text:?}"
    );
    assert_eq!(
        fold_count(&s),
        1,
        "nothing landed inside the fold, so it must stay closed"
    );
}

#[test]
fn a_rejected_intercept_chain_unfolds_nothing() {
    // A chain that raises applies no edit at all, so there is nothing to
    // reveal — the widening must not fire on a request that never lands.
    let (s, _id) = seeded();
    let mut s = s;
    fold_active(&s, 2, 5);
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "insert" then error("nope") end
          return nil
        end)
        "#,
    );
    set_cursor(&s, 0);
    define(
        &s,
        "test.rejected",
        &format!(
            "pcall(function() pmacs.window.buffer():insert({}, 'y') end)",
            end_of(4)
        ),
    );
    m_x(&mut s, "test.rejected");
    assert_eq!(fold_count(&s), 1, "a rejected edit reveals nothing");
}

#[test]
fn a_bypass_intercept_interactive_edit_also_unfolds() {
    // Pins the seam at `run_buffer_edit`, above the managed/bypass split:
    // hooking only `run_managed_edit` would let this one escape.
    let (s, _id) = seeded();
    let mut s = s;
    fold_active(&s, 2, 5);
    set_cursor(&s, 0);
    define(
        &s,
        "test.bypass",
        &format!(
            "pmacs.window.buffer():insert({}, 'z', {{ bypass_intercept = true }})",
            end_of(4)
        ),
    );
    m_x(&mut s, "test.bypass");
    assert_eq!(
        fold_count(&s),
        0,
        "a bypass_intercept interactive edit must not escape the widening"
    );
}

#[test]
fn comment_toggle_inside_a_fold_unfolds_it() {
    // The framing's named Q#FD19 case, through the REAL `M-;` path
    // (comment-toggle is a Lua mutator, so it exercises the
    // `run_buffer_edit` seam end to end rather than a synthetic stand-in).
    let dir = std::env::temp_dir().join(format!("pmacs-fold-s2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fold_comment.rs");
    let body = "fn a() {\n    let x = 1;\n    let y = 2;\n    let z = 3;\n}\n";
    std::fs::write(&path, body).unwrap();

    let s = editor();
    let mut s = s;
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            path.display().to_string()
        ),
    );
    exec(&s, "pmacs.editor.goto_byte(0)");
    // Hide the three `let` lines under the `fn a() {` head.
    let head_end = body.find('\n').unwrap() as u64;
    let last_hidden_end = body.rfind("let z = 3;").unwrap() as u64 + "let z = 3;".len() as u64;
    fold_range(&s, head_end, last_hidden_end);
    assert_eq!(fold_count(&s), 1);

    // Point on a hidden line, then M-; — the toggle rewrites that line.
    let y_line = body.find("    let y").unwrap() as u64;
    set_cursor(&s, y_line + 4);
    alt(&mut s, ';');
    assert_eq!(
        fold_count(&s),
        0,
        "comment-toggle's edit landed inside the fold, so it must reveal it"
    );
    let text: String = eval(
        &s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    assert!(
        text.contains("// "),
        "the toggle actually commented a line: {text:?}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_interactive_edit_to_an_inactive_buffer_does_not_unfold() {
    let (s, _id) = seeded();
    let mut s = s;
    fold_active(&s, 2, 5);
    set_cursor(&s, end_of(4)); // byte 19, inside the ACTIVE buffer's fold

    // A second, NOT-displayed buffer, deliberately shaped so the active
    // window's cursor byte falls inside a fold of ITS OWN: without the
    // active-window-buffer requirement the widening would anchor the
    // wrong buffer's folds on this frontend's point and open it.
    exec(
        &s,
        "other = pmacs.buffer.from_bytes('other.txt', 'aaa\\nbbb\\nccc\\nddd\\neee\\n')",
    );
    let other_folded: bool = eval(
        &s,
        "return pmacs.fold.fold(other, { start = 3, ['end'] = 19 })",
    );
    assert!(other_folded);
    let other_folds = |s: &EditorState| -> usize {
        let core = s.core.borrow();
        let registry = core.registry.clone();
        let reg = registry.borrow();
        let id = reg.find_by_name("other.txt").expect("other.txt");
        s.fold_registry.folds(id).len()
    };
    assert_eq!(other_folds(&s), 1);

    define(&s, "test.other", "other:insert(0, 'q')");
    m_x(&mut s, "test.other");
    assert_eq!(
        fold_count(&s),
        1,
        "the active buffer was not edited, so its fold stands"
    );
    assert_eq!(
        other_folds(&s),
        1,
        "an explicit inactive-buffer mutation stays programmatic — the \
         invoking frontend's point does not name a place in THAT buffer"
    );
}

#[test]
fn undo_and_redo_do_not_unfold() {
    // Explicitly DEFERRED in the framing (round-1 F5 ruling); pinned so
    // the deferral is a decision, not an accident.
    let (s, _id) = seeded();
    let mut s = s;
    set_cursor(&s, end_of(7));
    type_str(&mut s, "Z"); // an edit to undo, outside any fold
    fold_active(&s, 2, 5);
    set_cursor(&s, end_of(4));
    ctrl(&mut s, '_'); // undo
    assert_eq!(fold_count(&s), 1, "undo does not unfold (deferred)");
}

/// Content-end bytes of the current buffer's lines `a` and `b`.
fn line_ends(s: &EditorState, a: usize, b: usize) -> (u64, u64) {
    let core = s.core.borrow();
    let registry = core.registry.clone();
    let reg = registry.borrow();
    let buf = reg.get(core.active_buffer_id()).unwrap();
    let tv = &core.active_window().text_view;
    let end = |line: usize| tv.line_offset(line).unwrap() + tv.line_len(buf, line).unwrap();
    (end(a), end(b))
}

fn fold_range(s: &EditorState, start: u64, end: u64) {
    let ok: bool = eval(
        s,
        &format!(
            "return pmacs.fold.fold(pmacs.window.buffer(), {{ start = {start}, ['end'] = {end} }})"
        ),
    );
    assert!(ok, "fold({start}..{end}) must be accepted");
}

// ---------------------------------------------------------------------------
// 11. No wire / protocol change (framing acceptance 11, Bet B6)
// ---------------------------------------------------------------------------

#[test]
fn stage_2_bumps_no_protocol_version() {
    assert_eq!(
        pmacs::protocol::PROTOCOL_VERSION,
        19,
        "Stage 2 is entirely daemon-side: the TUI collapse ships no new wire data"
    );
    assert_eq!(
        *pmacs::protocol::SUPPORTED_PROTOCOL_VERSIONS.last().unwrap(),
        19
    );
}

// ---------------------------------------------------------------------------
// 12. Shared store, independent viewports (framing acceptance 12)
// ---------------------------------------------------------------------------

#[test]
fn two_windows_on_one_buffer_both_collapse_with_their_own_view_tops() {
    let (s, _id) = seeded();
    fold_active(&s, 2, 5);
    exec(&s, "pmacs.window.split_horizontal()");
    // Give the second window a different view_top (visible line 2 = head).
    let ids: Vec<WindowId> = s.core.borrow().windows.keys().copied().collect();
    assert_eq!(ids.len(), 2, "the split produced two windows");
    s.core
        .borrow_mut()
        .windows
        .get_mut(&ids[1])
        .unwrap()
        .view_top = 1;

    let cells = paint(&s);
    // 12 rows: status row 11; two stacked windows of 5 and 6 rows, each
    // with a mode line. Window A text rows 0..=3, window B text rows
    // 5..=9 (its own mode line last).
    assert_eq!(row_text(&cells, 0), "L00");
    assert_eq!(row_text(&cells, 2), "L02 …", "window A collapses");
    let b_rows: Vec<String> = (5..10).map(|r| row_text(&cells, r)).collect();
    assert!(
        b_rows.iter().any(|r| r == "L02 …"),
        "window B collapses too, from its own view_top: {b_rows:?}"
    );
    assert!(
        !b_rows.iter().any(|r| r.starts_with("L03")),
        "no hidden line renders in the second window either: {b_rows:?}"
    );
    // Both windows keep their own view_top.
    assert_eq!(s.core.borrow().windows[&ids[1]].view_top, 1);
}

// ---------------------------------------------------------------------------
// 13. Frontend-scoped motion (framing acceptance 13, round-2 F1 / Q#FD21)
// ---------------------------------------------------------------------------

/// Register a second frontend on the SAME buffer, with an explicit
/// projection choice — the attach-time decision the daemon makes from the
/// negotiated selected-render bit.
fn attach_frontend(s: &EditorState, fid: FrontendId, fold_projection: bool) -> WindowId {
    let mut core = s.core.borrow_mut();
    let buffer_id = core.active_buffer_id();
    let text_view = {
        let registry = core.registry.clone();
        let reg = registry.borrow();
        pmacs::text_view::TextView::new(reg.get(buffer_id).unwrap())
    };
    let win_id = WindowId::next();
    core.windows
        .insert(win_id, Window::new(win_id, buffer_id, text_view));
    core.register_frontend_view(
        fid,
        FrontendView {
            layout: Layout::single(win_id),
            active: win_id,
            fold_projection,
        },
    );
    win_id
}

/// Move down once as `fid` and report that frontend's resulting line.
fn move_down_as(s: &EditorState, fid: FrontendId, win_id: WindowId) -> usize {
    let mut core = s.core.borrow_mut();
    core.active_frontend = fid;
    core.move_down();
    let win = &core.windows[&win_id];
    win.text_view.line_at_offset(win.cursor)
}

fn move_up_as(s: &EditorState, fid: FrontendId, win_id: WindowId) -> usize {
    let mut core = s.core.borrow_mut();
    core.active_frontend = fid;
    core.move_up();
    let win = &core.windows[&win_id];
    win.text_view.line_at_offset(win.cursor)
}

#[test]
fn a_semantic_frontend_keeps_raw_line_motion_while_the_tui_folds() {
    let (s, _id) = seeded();
    fold_active(&s, 2, 5);
    // A grid session and a semantic session on the same buffer, sharing
    // the fold store (daemon.rs: a frontend holds exactly one of a
    // RenderState or a SemanticRenderState; both may attach at once).
    let grid_win = attach_frontend(&s, FrontendId(2), true);
    let sem_win = attach_frontend(&s, FrontendId(3), false);
    for win in [grid_win, sem_win] {
        s.core.borrow_mut().windows.get_mut(&win).unwrap().cursor = end_of(2);
    }

    assert_eq!(
        move_down_as(&s, FrontendId(2), grid_win),
        6,
        "the grid session steps by VISIBLE lines, skipping the collapse"
    );
    assert_eq!(
        move_down_as(&s, FrontendId(3), sem_win),
        3,
        "the semantic session still DISPLAYS line 3, so it must land there"
    );
    assert_eq!(
        move_up_as(&s, FrontendId(3), sem_win),
        2,
        "and steps back by raw lines too"
    );
}

#[test]
fn semantic_paging_stays_raw_while_the_grid_pages_visibly() {
    let s = editor();
    let id = active_id(&s);
    let long = long_fixture();
    seed(&s, id, &long);
    // Hide lines 1..=40 under head 0.
    let ok: bool = eval(
        &s,
        "return pmacs.fold.fold(pmacs.window.buffer(), { start = 4, ['end'] = 204 })",
    );
    assert!(ok);
    let grid_win = attach_frontend(&s, FrontendId(2), true);
    let sem_win = attach_frontend(&s, FrontendId(3), false);

    let page_as = |fid: FrontendId, win: WindowId| {
        let mut core = s.core.borrow_mut();
        core.active_frontend = fid;
        core.move_page_down();
        let w = &core.windows[&win];
        w.text_view.line_at_offset(w.cursor)
    };
    let grid_line = page_as(FrontendId(2), grid_win);
    let sem_line = page_as(FrontendId(3), sem_win);
    assert!(
        grid_line > 40,
        "a grid page clears the whole collapse (landed on {grid_line})"
    );
    assert!(
        sem_line <= 40,
        "a semantic page counts the lines it still shows (landed on {sem_line})"
    );
}

#[test]
fn detaching_the_semantic_frontend_leaves_the_grid_unaffected() {
    let (s, _id) = seeded();
    fold_active(&s, 2, 5);
    let grid_win = attach_frontend(&s, FrontendId(2), true);
    let sem_win = attach_frontend(&s, FrontendId(3), false);
    s.core
        .borrow_mut()
        .windows
        .get_mut(&sem_win)
        .unwrap()
        .cursor = end_of(2);
    assert_eq!(move_down_as(&s, FrontendId(3), sem_win), 3);

    s.core.borrow_mut().unregister_frontend_view(FrontendId(3));
    s.core
        .borrow_mut()
        .windows
        .get_mut(&grid_win)
        .unwrap()
        .cursor = end_of(2);
    assert_eq!(
        move_down_as(&s, FrontendId(2), grid_win),
        6,
        "the flag is per-FrontendView — the grid never inherited it"
    );
}

// ---------------------------------------------------------------------------
// 14. Split of different buffers, one folded (framing acceptance 14,
//     round-2 F2 + round-3 F1)
// ---------------------------------------------------------------------------

/// A vertical split showing buffer A (folded, active) beside buffer B
/// (unfolded). Returns `(a_window, b_window)`.
fn split_two_buffers(s: &EditorState) -> (WindowId, WindowId) {
    let a_win = s.core.borrow().active_window_id();
    exec(s, "pmacs.window.split_vertical()");
    let ids: Vec<WindowId> = s.core.borrow().windows.keys().copied().collect();
    let b_win = *ids.iter().find(|w| **w != a_win).expect("second window");
    // Point B at a different buffer.
    exec(
        s,
        "other = pmacs.buffer.from_bytes('other.txt', 'B0\\nB1\\nB2\\nB3\\nB4\\nB5\\nB6\\nB7\\n')",
    );
    let other: BufferId = {
        let core = s.core.borrow();
        let registry = core.registry.clone();
        let reg = registry.borrow();
        reg.find_by_name("other.txt").expect("other.txt present")
    };
    let text_view = {
        let core = s.core.borrow();
        let registry = core.registry.clone();
        let reg = registry.borrow();
        pmacs::text_view::TextView::new(reg.get(other).unwrap())
    };
    let mut core = s.core.borrow_mut();
    let w = core.windows.get_mut(&b_win).unwrap();
    w.buffer_id = other;
    w.text_view = text_view;
    w.cursor = 0;
    drop(core);
    s.core.borrow_mut().set_active_window_id(a_win);
    (a_win, b_win)
}

#[test]
fn an_unfolded_pane_beside_a_folded_one_is_byte_identical_to_its_baseline() {
    let (s, _id) = seeded();
    let (_a_win, b_win) = split_two_buffers(&s);
    let baseline = paint(&s);

    // Now fold buffer A only.
    fold_active(&s, 2, 5);
    let folded = paint(&s);

    // Window A collapsed …
    let a_rows: Vec<String> = (0..TEXT_ROWS).map(|r| row_text(&folded, r)).collect();
    assert!(
        a_rows.iter().any(|r| r.contains("L02 …")),
        "the folded pane collapsed: {a_rows:?}"
    );
    // … while every cell of window B is unchanged. B occupies the right
    // half of a vertical split.
    let b_origin = COLS / 2;
    for row in 0..TEXT_ROWS {
        for col in b_origin..COLS {
            assert_eq!(
                at(&baseline, row, col),
                at(&folded, row, col),
                "buffer B's window leaked A's fold map at ({row},{col})"
            );
        }
    }
    assert_eq!(s.core.borrow().windows[&b_win].view_top, 0);
}

#[test]
fn peer_presence_in_the_unfolded_pane_uses_that_windows_map() {
    // Framing acceptance 14's presence clause: the recipient window's
    // map, not the active (folded) buffer's — a peer in B must land on
    // B's RAW row even while A is collapsed.
    let (s, _id) = seeded();
    let (_a_win, b_win) = split_two_buffers(&s);
    fold_active(&s, 2, 5); // buffer A (active) folds; B does not
    let b_buffer = s.core.borrow().windows[&b_win].buffer_id;
    // B is "B0\nB1\n…B7\n" (3 bytes a line). Put the peer on B's line 6 —
    // BELOW where A's collapse sits, so the two maps disagree: B's map
    // (none) says row 6, while A's map — which hides lines 3..=5 — would
    // shift it up to row 3.
    let cells = paint_presence(&s, &[peer(b_buffer, 6 * 3, None)]);

    let b_origin = COLS / 2;
    let peer_cell =
        |row: u32| (b_origin..COLS).any(|c| cells[(row * COLS + c) as usize].style.reverse);
    assert!(
        peer_cell(6),
        "B has no folds, so its peer stays on raw row 6"
    );
    assert!(
        !peer_cell(3),
        "the recipient window's map is B's — projecting through the \
         active (folded) buffer's map would have put it on row 3"
    );
    // And nothing painted into A's half for a peer in B's buffer.
    for row in 0..TEXT_ROWS {
        assert!(
            (0..b_origin).all(|c| !cells[(row * COLS + c) as usize].style.reverse),
            "a peer in buffer B must not paint into buffer A's pane (row {row})"
        );
    }
}

#[test]
fn wheel_over_an_inactive_unfolded_pane_uses_that_windows_map() {
    let (s, _id) = seeded();
    let (a_win, b_win) = split_two_buffers(&s);
    fold_active(&s, 2, 5); // buffer A (active) folds; B does not
    let mut s = s;
    let _ = paint(&s);

    // Wheel down over the RIGHT half — buffer B's pane, which the wheel
    // does NOT activate.
    s.dispatch_mouse(
        FrontendId::LOCAL,
        mouse(MouseEventKind::ScrollDown, 1, (COLS / 2 + 2) as u16),
        CellSize::new(ROWS, COLS),
    );

    assert_eq!(
        s.core.borrow().active_window_id(),
        a_win,
        "a wheel event does not move focus"
    );
    let b_top = s.core.borrow().windows[&b_win].view_top;
    assert_eq!(
        b_top, 3,
        "B scrolled by raw lines through ITS OWN map (SCROLL_LINES), \
         not through the folded active buffer's"
    );
    assert_eq!(
        s.core.borrow().windows[&a_win].view_top,
        0,
        "the folded pane did not scroll"
    );
}
