// theme_faces_acceptance.rs --- Themes Arc 4 stage 1 acceptance
// (docs/theme-faces-framing.md, acceptance items 1–19, 24–26, and
// 28–29; the GPU routes — 20–23, 27, and 30 — live in pmacs-gpu's
// headless suite).

//! Named UI faces (`ui` / `ui.*` theme entries) + the `ThemeFacts`
//! wire channel (protocol v16).
//!
//! Grid-path rendering drives the full `paint_frame` (mode line,
//! status row, gutter, minibuffer, selection, search, diagnostics all
//! paint there); keybinding claims dispatch keys per the standing
//! discipline; wire claims drive a `SemanticRenderState` frame by
//! frame, and the version gate exercises a real daemon.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::cell::{Cell, CellGrid, CellSize, Color, Style, UnderlineStyle};
use pmacs::editor::EditorState;
use pmacs::protocol::{ByteRange, FrontendId, InstanceMessage, ThemeFace};
use pmacs::semantic_render::SemanticRenderState;
use std::time::{Duration, Instant};

#[cfg(feature = "crdt")]
mod common;

// ---------------------------------------------------------------------------
// Harness (compile_mode_acceptance conventions)
// ---------------------------------------------------------------------------

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
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

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn exec_err(s: &EditorState, src: &str) -> mlua::Error {
    s.lua_host
        .lua()
        .load(src.to_string())
        .exec()
        .expect_err("chunk must error")
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

/// Fresh editor with LSP spawning disabled.
fn editor() -> EditorState {
    let s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    s
}

/// Paint the FULL frame (windows + mode lines + status row /
/// minibuffer) — the chrome surfaces under test all render here.
fn paint_full_frame(state: &EditorState, rows: u32, cols: u32) -> Vec<Cell> {
    let mut backing = vec![Cell::default(); (rows * cols) as usize];
    let mut grid = CellGrid {
        cells: &mut backing,
        stride: cols,
        size: CellSize::new(rows, cols),
    };
    let _cursor = pmacs::editor::paint_frame(state, &mut grid, CellSize::new(rows, cols));
    backing
}

fn at(cells: &[Cell], cols: u32, row: u32, col: u32) -> &Cell {
    &cells[(row * cols + col) as usize]
}

fn row_text(cells: &[Cell], cols: u32, row: u32) -> String {
    (0..cols)
        .map(|c| match &at(cells, cols, row, c).glyph {
            pmacs::cell::Glyph::Char(ch) => *ch,
            _ => ' ',
        })
        .collect()
}

// --- Wire helpers -----------------------------------------------------------

fn active_buffer(state: &EditorState) -> pmacs::buffer::BufferId {
    state.core.borrow().active_window().buffer_id
}

fn semantic(state: &EditorState) -> SemanticRenderState {
    let buffer_id = active_buffer(state);
    let mut s = SemanticRenderState::new(FrontendId::LOCAL);
    s.set_viewport(
        buffer_id,
        ByteRange {
            start: 0,
            end: 1 << 20,
        },
        0,
    );
    s
}

fn theme_facts_of(msgs: &[InstanceMessage]) -> Option<Vec<ThemeFace>> {
    msgs.iter().find_map(|m| match m {
        InstanceMessage::ThemeFacts { faces } => Some(faces.clone()),
        _ => None,
    })
}

fn has_style_spans(msgs: &[InstanceMessage]) -> bool {
    msgs.iter()
        .any(|m| matches!(m, InstanceMessage::StyleSpans { .. }))
}

fn summary_of(msgs: &[InstanceMessage]) -> Option<Vec<Style>> {
    msgs.iter().find_map(|m| match m {
        InstanceMessage::FileStyleSummary { lines, .. } => Some(lines.clone()),
        _ => None,
    })
}

// --- Grammar fixtures (m4_acceptance conventions) ---------------------------

fn pump_async<F: Fn(&EditorState) -> bool>(state: &mut EditorState, predicate: F) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate(state) {
        assert!(Instant::now() < deadline, "async pump deadline exceeded");
        state.tick_async();
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn current_tree_language(state: &EditorState) -> Option<String> {
    let chunk = r"
        local buf = pmacs.window.buffer()
        if not buf then return nil end
        local tree = pmacs.parse.tree(buf)
        if not tree then return nil end
        return tree:language()
    ";
    state
        .lua_host
        .lua()
        .load(chunk)
        .eval::<Option<String>>()
        .ok()
        .flatten()
}

/// Open `path` and pump until its parse settles (highlights attached).
fn open_and_wait_for_parse(path: std::path::PathBuf) -> EditorState {
    let mut state = EditorState::open(path).expect("open file");
    exec(&state, "pmacs.lsp.config = {}");
    pump_async(&mut state, |s| current_tree_language(s).is_some());
    state
}

fn rust_fixture(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("faces.rs");
    std::fs::write(&path, b"fn main() {}\n").expect("write fixture");
    path
}

// --- Diagnostics fixture -----------------------------------------------------

fn diag(severity: pmacs::diag::DiagnosticSeverity) -> pmacs::diag::Diagnostic {
    pmacs::diag::Diagnostic {
        start_line: 0,
        start_col: 0,
        end_line: 0,
        end_col: 3,
        severity,
        message: "boom".into(),
        source: None,
        code: None,
    }
}

/// Give the active buffer a file path, publish `diags` for it, and
/// attach the diagnostic overlay through the REAL Lua path
/// (`pmacs.diag._attach_view` — `install_diag`), never a bare
/// constructor (Q#TH9).
fn attach_diags(state: &EditorState, diags: Vec<pmacs::diag::Diagnostic>) -> String {
    let uri: String = eval(
        state,
        r#"
        local buf = pmacs.window.buffer()
        local uri = "file:///tmp/theme_faces_diag.rs"
        assert(pmacs.diag._attach_view(buf, uri))
        return uri
        "#,
    );
    {
        let core = state.core.borrow();
        let registry = core.registry.clone();
        let mut reg = registry.borrow_mut();
        let buf = reg.get_mut(core.active_buffer_id()).unwrap();
        buf.set_file_path(Some(std::path::PathBuf::from("/tmp/theme_faces_diag.rs")));
    }
    let store = state.lsp_manager.borrow().diag_store();
    store
        .lock()
        .expect("diag store lock")
        .set(uri.clone(), diags);
    uri
}

// --- Daemon wire helpers (item 29; CRDT suites only) -------------------------

#[cfg(feature = "crdt")]
fn wire_viewport(
    stream: &mut std::os::unix::net::UnixStream,
    fid: FrontendId,
    buffer_id: pmacs::buffer::BufferId,
) {
    pmacs::transport::write_message(
        stream,
        &pmacs::protocol::FrontendEvent::Viewport {
            frontend_id: fid,
            buffer_id,
            visible: ByteRange {
                start: 0,
                end: 4096,
            },
            generation: 0,
        },
    )
    .expect("write Viewport");
}

#[cfg(feature = "crdt")]
fn wire_key(
    stream: &mut std::os::unix::net::UnixStream,
    fid: FrontendId,
    key: pmacs::protocol::Key,
) {
    pmacs::transport::write_message(
        stream,
        &pmacs::protocol::FrontendEvent::Key(pmacs::protocol::KeyEvent {
            frontend_id: fid,
            key,
            mods: pmacs::protocol::Modifiers::NONE,
            timestamp_ns: 0,
        }),
    )
    .expect("write Key");
}

/// Read wire messages until `pick` returns, or panic at the deadline.
#[cfg(feature = "crdt")]
fn wire_wait_for<T>(
    stream: &mut std::os::unix::net::UnixStream,
    what: &str,
    mut pick: impl FnMut(InstanceMessage) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(msg) = pmacs::transport::read_message::<InstanceMessage>(stream)
            && let Some(t) = pick(msg)
        {
            return t;
        }
    }
    panic!("timeout waiting for {what}");
}

// ---------------------------------------------------------------------------
// 1 — unset faces: byte-identical chrome; default_style never leaks
// ---------------------------------------------------------------------------

#[test]
fn unset_faces_keep_todays_chrome_and_syntax_default_never_leaks() {
    let mut state = editor();
    type_str(&mut state, "hello");
    let (rows, cols) = (8u32, 20u32);
    let base = paint_full_frame(&state, rows, cols);

    // Today's literals, spot-pinned: mode line (window's last row) and
    // status row are reverse video with no colors.
    let mode_row = rows - 2;
    let status_row = rows - 1;
    for col in 0..cols {
        let m = at(&base, cols, mode_row, col);
        assert!(m.style.reverse, "mode line is reverse video when unthemed");
        assert_eq!(m.style.fg, Color::Default);
        let s = at(&base, cols, status_row, col);
        assert!(s.style.reverse, "status row is reverse video when unthemed");
    }

    // A loud SYNTAX default must not bleed into chrome (Q#TH4:
    // face resolution returns None, never default_style).
    exec(
        &state,
        "pmacs.theme.default { fg = 5, bg = 3, bold = true }",
    );
    let loud = paint_full_frame(&state, rows, cols);
    assert_eq!(base, loud, "pmacs.theme.default must change no chrome cell");
}

// ---------------------------------------------------------------------------
// 2 + 3 — surface faces apply per cell; owns-surface resets to plain
// ---------------------------------------------------------------------------

#[test]
fn surface_faces_apply_and_partial_faces_reset_to_plain() {
    let mut state = editor();
    type_str(&mut state, "one\ntwo\nthree");
    let (rows, cols) = (8u32, 20u32);
    let mode_row = rows - 2;
    let status_row = rows - 1;

    // Full modeline face: fg + bg, no reverse.
    exec(
        &state,
        r#"pmacs.theme.merge { ["ui.modeline"] = { fg = 252, bg = 236 },
                              ["ui.statusline"] = { fg = 4 } }"#,
    );
    let themed = paint_full_frame(&state, rows, cols);
    for col in 0..cols {
        let m = at(&themed, cols, mode_row, col);
        assert_eq!(m.style.fg, Color::Indexed(252), "modeline fg at {col}");
        assert_eq!(m.style.bg, Color::Indexed(236), "modeline bg at {col}");
        assert!(!m.style.reverse, "a set face owns the surface: no reverse");
        let s = at(&themed, cols, status_row, col);
        assert_eq!(s.style.fg, Color::Indexed(4), "statusline fg at {col}");
        assert!(!s.style.reverse, "statusline surface resets to plain");
    }

    // Owns-surface (Q#TH5): an fg-only modeline face still drops the
    // reverse video — partial faces reset the rest to plain.
    exec(
        &state,
        r#"pmacs.theme.merge { ["ui.modeline"] = { fg = 252 } }"#,
    );
    let partial = paint_full_frame(&state, rows, cols);
    let m = at(&partial, cols, mode_row, 0);
    assert_eq!(m.style.fg, Color::Indexed(252));
    assert_eq!(m.style.bg, Color::Default, "unset bg is plain");
    assert!(!m.style.reverse, "partial face resets reverse to plain");

    // Gutter face ({fg} mask) on the line-number strip.
    exec(&state, r#"pmacs.window.set_line_numbers("absolute")"#);
    exec(
        &state,
        r#"pmacs.theme.merge { ["ui.gutter"] = { fg = 13 } }"#,
    );
    let with_gutter = paint_full_frame(&state, rows, cols);
    let g = at(&with_gutter, cols, 0, 0);
    assert_eq!(g.style.fg, Color::Indexed(13), "gutter digits recolor");
}

// ---------------------------------------------------------------------------
// 3 (rest) — ui.selection = {} disables the wash on the grid
// ---------------------------------------------------------------------------

#[test]
fn empty_selection_face_disables_the_wash() {
    let mut state = editor();
    type_str(&mut state, "alpha beta");
    let (rows, cols) = (6u32, 20u32);

    let set_selection = |state: &EditorState| {
        let mut core = state.core.borrow_mut();
        let win = core.active_window_mut();
        win.selection = Some(pmacs::window::Selection { anchor: 0 });
        win.cursor = 5;
    };
    let clear_selection = |state: &EditorState| {
        state.core.borrow_mut().active_window_mut().selection = None;
    };

    // Unset face: reverse-video wash, exactly today.
    set_selection(&state);
    let washed = paint_full_frame(&state, rows, cols);
    assert!(
        at(&washed, cols, 0, 0).style.reverse,
        "unthemed selection is reverse video"
    );

    // Empty face: the wash disappears — selected cells render exactly
    // like unselected ones (Q#TH5, all-default overlay).
    exec(&state, r#"pmacs.theme.merge { ["ui.selection"] = {} }"#);
    let disabled = paint_full_frame(&state, rows, cols);
    clear_selection(&state);
    let unselected = paint_full_frame(&state, rows, cols);
    assert_eq!(
        disabled, unselected,
        "an all-default selection face must disable the wash"
    );
}

// ---------------------------------------------------------------------------
// 4 — mask enforcement: out-of-mask components are ignored
// ---------------------------------------------------------------------------

#[test]
fn out_of_mask_components_are_ignored_on_the_grid() {
    let mut state = editor();
    type_str(&mut state, "alpha beta");
    let (rows, cols) = (8u32, 20u32);
    {
        let mut core = state.core.borrow_mut();
        let win = core.active_window_mut();
        win.selection = Some(pmacs::window::Selection { anchor: 0 });
        win.cursor = 5;
    }

    // ui.selection mask is {bg}: fg + reverse must be ignored.
    exec(
        &state,
        r#"pmacs.theme.merge { ["ui.selection"] = { bg = 17 } }"#,
    );
    let bg_only = paint_full_frame(&state, rows, cols);
    exec(
        &state,
        r#"pmacs.theme.merge { ["ui.selection"] = { fg = 1, reverse = true, bg = 17 } }"#,
    );
    let with_extras = paint_full_frame(&state, rows, cols);
    assert_eq!(
        bg_only, with_extras,
        "fg/reverse on a wash face must render exactly as the bg-only face"
    );
    assert_eq!(
        at(&bg_only, cols, 0, 0).style.bg,
        Color::Indexed(17),
        "the in-mask bg applies"
    );
    assert_eq!(
        at(&bg_only, cols, 0, 0).style.fg,
        Color::Default,
        "the out-of-mask fg does not"
    );

    // ui.gutter mask is {fg}: bg + reverse must be ignored.
    exec(&state, r#"pmacs.window.set_line_numbers("absolute")"#);
    exec(
        &state,
        r#"pmacs.theme.merge { ["ui.gutter"] = { fg = 9 } }"#,
    );
    let fg_only = paint_full_frame(&state, rows, cols);
    exec(
        &state,
        r#"pmacs.theme.merge { ["ui.gutter"] = { fg = 9, bg = 1, reverse = true } }"#,
    );
    let gutter_extras = paint_full_frame(&state, rows, cols);
    assert_eq!(
        fg_only, gutter_extras,
        "bg/reverse on a foreground-only site must render as the fg-only face"
    );
}

// ---------------------------------------------------------------------------
// 5 — wash faces merge over syntax-styled cells; the real search path
// ---------------------------------------------------------------------------

#[test]
fn search_wash_faces_merge_over_syntax_and_split_active_from_lazy() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Two `fn` occurrences so a lazy and an active match coexist.
    let path = dir.path().join("faces.rs");
    std::fs::write(&path, b"fn main() { fn_helper(); }\n").expect("write");
    let mut state = open_and_wait_for_parse(path);
    let (rows, cols) = (8u32, 40u32);

    exec(
        &state,
        r#"pmacs.theme.merge {
            ["ui.search.match"] = { bg = 17 },
            ["ui.search.match.active"] = { bg = 22 },
        }"#,
    );
    // The syntax-styled frame BEFORE any search: the per-cell fg
    // baseline the wash must preserve (`fn` at 0 is a keyword; the
    // `fn` inside `fn_helper` is function-colored — the wash must
    // keep each as-is, whatever the grammar decided).
    let before = paint_full_frame(&state, rows, cols);

    // The REAL path: dispatched C-s reaching ensure_search_overlay.
    ctrl(&mut state, 's');
    type_str(&mut state, "fn");
    let cells = paint_full_frame(&state, rows, cols);

    // Both `fn` occurrences: cols 0..2 and 12..14 on row 0.
    let first = at(&cells, cols, 0, 0);
    let second = at(&cells, cols, 0, 12);
    let bgs = [first.style.bg, second.style.bg];
    assert!(
        bgs.contains(&Color::Indexed(22)),
        "one match is active (got {bgs:?})"
    );
    assert!(
        bgs.contains(&Color::Indexed(17)),
        "one match is lazy (got {bgs:?})"
    );
    // Merge semantics: each cell's syntax fg survives under the
    // bg-only wash (per-cell fg assertion, not any-styled-cell).
    for col in [0u32, 1, 12, 13] {
        assert_eq!(
            at(&cells, cols, 0, col).style.fg,
            at(&before, cols, 0, col).style.fg,
            "a bg-only wash keeps the syntax fg underneath (col {col})"
        );
        assert_ne!(
            at(&before, cols, 0, col).style.bg,
            at(&cells, cols, 0, col).style.bg,
            "the wash bg landed (col {col})"
        );
    }
}

// ---------------------------------------------------------------------------
// 6 + 7 — diag faces on every grid surface + inheritance + empty child
// ---------------------------------------------------------------------------

#[test]
fn diag_faces_recolor_squiggle_and_marker_with_inheritance_and_empty_child_reset() {
    use pmacs::diag::DiagnosticSeverity;

    let mut state = editor();
    type_str(&mut state, "boom\nfine\n");
    attach_diags(
        &state,
        vec![
            diag(DiagnosticSeverity::Error),
            pmacs::diag::Diagnostic {
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 3,
                severity: DiagnosticSeverity::Warning,
                message: "warn".into(),
                source: None,
                code: None,
            },
        ],
    );
    let (rows, cols) = (8u32, 20u32);

    // Unset: built-in severity colors (error Indexed(1) squiggle,
    // warning Indexed(3)).
    let base = paint_full_frame(&state, rows, cols);
    assert_eq!(at(&base, cols, 0, 0).style.underline, UnderlineStyle::Curly);
    assert_eq!(
        at(&base, cols, 0, 0).style.underline_color,
        Color::Indexed(1)
    );
    assert_eq!(
        at(&base, cols, 1, 0).style.underline_color,
        Color::Indexed(3)
    );

    // Inheritance: a themed ui.diag parent colors all severities.
    exec(&state, r#"pmacs.theme.merge { ["ui.diag"] = { fg = 93 } }"#);
    let inherited = paint_full_frame(&state, rows, cols);
    assert_eq!(
        at(&inherited, cols, 0, 0).style.underline_color,
        Color::Indexed(93),
        "error inherits ui.diag"
    );
    assert_eq!(
        at(&inherited, cols, 1, 0).style.underline_color,
        Color::Indexed(93),
        "warning inherits ui.diag"
    );

    // An exact EMPTY child blocks inheritance and resets errors to
    // the built-in color; warnings keep the parent's (Q#TH5, round 3
    // finding 4).
    exec(&state, r#"pmacs.theme.merge { ["ui.diag.error"] = {} }"#);
    let reset = paint_full_frame(&state, rows, cols);
    assert_eq!(
        at(&reset, cols, 0, 0).style.underline_color,
        Color::Indexed(1),
        "empty child resets errors to the built-in"
    );
    assert_eq!(
        at(&reset, cols, 1, 0).style.underline_color,
        Color::Indexed(93),
        "warnings still inherit"
    );

    // An explicit colored child wins over the parent.
    exec(
        &state,
        r#"pmacs.theme.merge { ["ui.diag.error"] = { fg = 45 } }"#,
    );
    let explicit = paint_full_frame(&state, rows, cols);
    assert_eq!(
        at(&explicit, cols, 0, 0).style.underline_color,
        Color::Indexed(45)
    );
}

#[test]
fn diag_face_recolors_the_minimap_marks_and_reships_the_summary() {
    use pmacs::diag::DiagnosticSeverity;

    let mut state = editor();
    type_str(&mut state, "boom\nfine\n");
    attach_diags(&state, vec![diag(DiagnosticSeverity::Error)]);

    let mut sem = semantic(&state);
    let first = sem.render_frame(&state);
    let lines = summary_of(&first).expect("first frame ships the summary");
    assert_eq!(
        lines[0].underline_color,
        Color::Indexed(1),
        "unthemed mark carries the built-in error color"
    );

    // A diag-face change with NO buffer edit re-ships the summary
    // with the resolved color (the minimap twin of the staleness
    // bite).
    exec(
        &state,
        r#"pmacs.theme.merge { ["ui.diag.error"] = { fg = 45 } }"#,
    );
    let next = sem.render_frame(&state);
    let lines = summary_of(&next).expect("diag-face change re-ships the summary");
    assert_eq!(
        lines[0].underline_color,
        Color::Indexed(45),
        "the mark recolors through ui.diag.error"
    );
    // The mark is PRESENT (never Default — the diag Default policy
    // keeps presence representable).
    assert_ne!(lines[0].underline_color, Color::Default);
}

// ---------------------------------------------------------------------------
// 8 — bare ui is a face key
// ---------------------------------------------------------------------------

#[test]
fn bare_ui_merge_ships_the_catch_all_without_touching_spans() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = open_and_wait_for_parse(rust_fixture(&dir));
    let mut sem = semantic(&state);
    let first = sem.render_frame(&state);
    assert!(has_style_spans(&first), "grammar buffer ships spans");
    assert_eq!(theme_facts_of(&first), Some(Vec::new()));

    exec(&state, r"pmacs.theme.merge { ui = { fg = 3 } }");
    let next = sem.render_frame(&state);
    let facts = theme_facts_of(&next).expect("bare ui bumps face_epoch and emits");
    assert_eq!(facts.len(), 12, "the catch-all resolves every stage-1 face");
    assert!(
        facts
            .iter()
            .all(|f| f.style.fg == Color::Indexed(3) && f.name.starts_with("ui")),
        "each face resolved through the catch-all"
    );
    assert!(
        !has_style_spans(&next),
        "a face key must classify as face, not syntax — no span re-emission"
    );
}

// ---------------------------------------------------------------------------
// 9 + 10 — the staleness bite + consecutive-set monotonicity
// ---------------------------------------------------------------------------

#[test]
fn mid_session_recolor_reships_spans_without_an_edit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = open_and_wait_for_parse(rust_fixture(&dir));
    let mut sem = semantic(&state);
    assert!(has_style_spans(&sem.render_frame(&state)));
    assert!(
        !has_style_spans(&sem.render_frame(&state)),
        "an unchanged tick is span-silent (the gate holds)"
    );

    // Zero buffer edits; a capture recolor alone must re-ship. This
    // is the pre-existing GPU staleness bug's bite: pre-arc, the
    // StyleGate ignored the theme and this frame shipped nothing.
    exec(&state, r"pmacs.theme.set { keyword = { fg = 99 } }");
    assert!(
        has_style_spans(&sem.render_frame(&state)),
        "a mid-session pmacs.theme.set must re-ship StyleSpans"
    );
}

#[test]
fn consecutive_sets_each_reship_and_coalesce_within_one_frame() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = open_and_wait_for_parse(rust_fixture(&dir));
    let mut sem = semantic(&state);
    let _ = sem.render_frame(&state);

    // set → observe → set → observe (round 2 finding 4's shape): each
    // mutation is observed by a render. Fails if wholesale
    // replacement resets the counters (the second set would share the
    // first's epoch and ship nothing).
    exec(&state, r"pmacs.theme.set { keyword = { fg = 99 } }");
    assert!(
        has_style_spans(&sem.render_frame(&state)),
        "first set re-ships"
    );
    exec(&state, r"pmacs.theme.set { keyword = { fg = 111 } }");
    assert!(
        has_style_spans(&sem.render_frame(&state)),
        "second set re-ships"
    );

    // Companion: two mutations inside one frame legitimately coalesce
    // into one emission carrying the SECOND set's color.
    exec(&state, r"pmacs.theme.set { keyword = { fg = 120 } }");
    exec(&state, r"pmacs.theme.set { keyword = { fg = 130 } }");
    let frame = sem.render_frame(&state);
    let span_styles: Vec<Style> = frame
        .iter()
        .find_map(|m| match m {
            InstanceMessage::StyleSpans { segments, .. } => Some(
                segments
                    .iter()
                    .flat_map(|s| s.spans.iter().map(|sp| sp.style))
                    .collect(),
            ),
            _ => None,
        })
        .expect("coalesced frame ships once");
    assert!(
        span_styles.iter().any(|s| s.fg == Color::Indexed(130)),
        "the coalesced emission reflects the final set"
    );
}

// ---------------------------------------------------------------------------
// 11 — malformed merge is atomic (Lua contract; the deterministic
// bite is the commit-boundary unit in lua_bindings)
// ---------------------------------------------------------------------------

#[test]
fn malformed_merge_is_atomic_from_lua() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = open_and_wait_for_parse(rust_fixture(&dir));
    let mut sem = semantic(&state);
    let _ = sem.render_frame(&state);

    exec(&state, r"pmacs.theme.merge { keyword = { fg = 42 } }");
    let _ = sem.render_frame(&state);

    // One merge carrying a valid entry AND a malformed one (a string
    // is not a style table). Lua table iteration order is unspecified
    // (round 2 finding 8), so this pins only the order-independent
    // user contract: error, nothing landed, nothing emitted.
    let err = exec_err(
        &state,
        r#"pmacs.theme.merge { string = { fg = 7 }, ["ui.modeline"] = "nope" }"#,
    );
    let msg = format!("{err}");
    assert!(!msg.is_empty());

    let fg: i64 = eval(&state, r#"return pmacs.theme.get("keyword").fg"#);
    assert_eq!(fg, 42, "prior entries survive");
    let string_fg: i64 = eval(&state, r#"return pmacs.theme.get("string").fg"#);
    assert_ne!(
        string_fg, 7,
        "the valid entry of the failed merge is absent"
    );

    let frame = sem.render_frame(&state);
    assert!(
        !has_style_spans(&frame) && theme_facts_of(&frame).is_none(),
        "a failed merge bumps nothing and emits nothing"
    );
}

#[test]
fn raising_index_metamethods_fail_the_merge_transactionally() {
    // PR #120 round 1 finding 2: `Table::get` runs __index, so a
    // raising metatable must error the whole merge. Pre-fix, the
    // trapped lookup silently parsed as an all-default style and the
    // merge SUCCEEDED, committing the valid sibling — against the
    // Q#TH6 all-or-nothing contract.
    let mut state = editor();
    type_str(&mut state, "hello");
    let mut sem = semantic(&state);
    let _ = sem.render_frame(&state);

    let err = exec_err(
        &state,
        r#"
        local trap = setmetatable({}, { __index = function() error("trapdoor") end })
        pmacs.theme.merge { ["ui.gutter"] = trap, zebra = { fg = 42 } }
        "#,
    );
    assert!(
        format!("{err}").contains("trapdoor"),
        "the metatable's own error surfaces: {err}"
    );
    // `zebra` is not in default_dark, so its presence would prove the
    // valid sibling leaked through the failed merge.
    let (zebra, gutter): (bool, bool) = eval(
        &state,
        r#"
        local t = pmacs.theme.current()
        return t["zebra"] ~= nil, t["ui.gutter"] ~= nil
        "#,
    );
    assert!(
        !zebra && !gutter,
        "nothing from the failed merge landed (zebra={zebra}, gutter={gutter})"
    );
    let frame = sem.render_frame(&state);
    assert!(
        theme_facts_of(&frame).is_none() && !has_style_spans(&frame),
        "a failed merge bumps nothing and emits nothing"
    );
    // (Boolean fields deliberately follow Lua truthiness — mlua's
    // bool conversion — so `reverse = "yes"` is Some(true), not an
    // error; the transactional contract is about lookups that RAISE.)
}

#[test]
fn v15_peers_get_no_face_derived_summary_marks() {
    // PR #120 round 1 finding 3: ui.diag.* colors reach the minimap
    // through FileStyleSummary — an ungated pre-v16 channel. A v15
    // semantic peer must keep built-in marks (its squiggles, signs,
    // and counters are unthemed too); a v16 peer gets the face.
    use pmacs::diag::DiagnosticSeverity;

    let mut state = editor();
    type_str(&mut state, "boom\nfine\n");
    attach_diags(&state, vec![diag(DiagnosticSeverity::Error)]);
    exec(
        &state,
        r#"pmacs.theme.merge { ["ui.diag.error"] = { fg = 45 } }"#,
    );
    let buffer_id = active_buffer(&state);
    let viewport = ByteRange {
        start: 0,
        end: 1 << 20,
    };

    let mut v15 = SemanticRenderState::for_peer(FrontendId::LOCAL, 15);
    v15.set_viewport(buffer_id, viewport, 0);
    let frame = v15.render_frame(&state);
    let lines = summary_of(&frame).expect("v15 still receives the summary");
    assert_eq!(
        lines[0].underline_color,
        Color::Indexed(1),
        "a v15 peer's marks keep the built-in severity color"
    );
    assert!(
        theme_facts_of(&frame).is_none(),
        "and no ThemeFacts is produced for it at all"
    );

    let mut v16 = SemanticRenderState::for_peer(FrontendId::LOCAL, 16);
    v16.set_viewport(buffer_id, viewport, 0);
    let frame = v16.render_frame(&state);
    assert_eq!(
        summary_of(&frame).expect("summary ships")[0].underline_color,
        Color::Indexed(45),
        "a v16 peer's marks resolve the face"
    );
    assert!(theme_facts_of(&frame).is_some());
}

// ---------------------------------------------------------------------------
// 28 + 29 — snapshot/baseline reset: the A → B → A round trip
// (PR #120 round 2 finding 1)
// ---------------------------------------------------------------------------

#[test]
fn snapshot_round_trip_restores_the_themed_summary_at_one_generation() {
    // A `BufferSnapshot` wipes the frontend's buffer-scoped render
    // state, so the producer's emission baselines must die with the
    // send (`on_buffer_snapshot_sent`). Pre-fix, revisiting A at an
    // unchanged generation matched `last_summary[A]` and the
    // frontend never regained the themed minimap — or A's
    // StatusFacts — until an edit, republish, or theme mutation
    // happened to move the key.
    use pmacs::diag::DiagnosticSeverity;

    let mut state = editor();
    type_str(&mut state, "boom\nfine\n");
    attach_diags(&state, vec![diag(DiagnosticSeverity::Error)]);
    exec(
        &state,
        r#"pmacs.theme.merge { ["ui.diag.error"] = { fg = 45 } }"#,
    );
    exec(&state, "_G.__A = pmacs.window.buffer()");

    let summary_msg = |msgs: &[InstanceMessage]| {
        msgs.iter().find_map(|m| match m {
            InstanceMessage::FileStyleSummary {
                generation, lines, ..
            } => Some((*generation, lines.clone())),
            _ => None,
        })
    };
    let has_status = |msgs: &[InstanceMessage]| {
        msgs.iter()
            .any(|m| matches!(m, InstanceMessage::StatusFacts { .. }))
    };
    let viewport = ByteRange {
        start: 0,
        end: 1 << 20,
    };

    let a = active_buffer(&state);
    let mut sem = semantic(&state);
    let first = sem.render_frame(&state);
    let (gen_a, themed) = summary_msg(&first).expect("first frame ships the summary");
    assert_eq!(
        themed[0].underline_color,
        Color::Indexed(45),
        "the mark is themed"
    );
    assert!(has_status(&first), "first frame ships StatusFacts");
    let quiet = sem.render_frame(&state);
    assert!(
        summary_msg(&quiet).is_none() && !has_status(&quiet),
        "an unchanged tick is suppressed"
    );

    // The daemon switches this session to B: it writes snapshot(B)
    // (resetting B's baselines) and the frontend re-declares.
    exec(
        &state,
        "local id = pmacs.instance.show(); pmacs.window.switch_buffer(id)",
    );
    let b = active_buffer(&state);
    assert_ne!(a, b, "the instance buffer is a distinct buffer");
    sem.on_buffer_snapshot_sent(b);
    sem.set_viewport(b, viewport, 0);
    let _ = sem.render_frame(&state);

    // ... and back to A. Zero edits: same CRDT generation.
    exec(&state, "pmacs.window.switch_buffer(_G.__A)");
    assert_eq!(active_buffer(&state), a);
    sem.on_buffer_snapshot_sent(a);
    sem.set_viewport(a, viewport, 0);
    let back = sem.render_frame(&state);
    let (gen_back, lines_back) = summary_msg(&back).expect("the revisit re-ships the summary");
    assert_eq!(
        gen_back, gen_a,
        "at the SAME generation — no edit forced it"
    );
    assert_eq!(lines_back, themed, "the identical themed payload returns");
    assert!(has_status(&back), "StatusFacts returns with it");
}

#[cfg(feature = "crdt")]
#[test]
fn daemon_reships_the_summary_after_a_real_buffer_round_trip() {
    // Item 29 — the daemon wiring for finding 1: a REAL daemon whose
    // session navigates A → B → A (via dispatched keys, so the
    // active-buffer-follow path writes the snapshots) must re-ship
    // `FileStyleSummary` for A at its unchanged generation. Pre-fix,
    // the producer baselines survived the snapshot and the revisit
    // was summary-silent forever.
    use common::daemon::{TestDaemon, build_default_caps};
    use pmacs::protocol::{AttachRequest, FrontendCapabilities, Hello, Key};
    use pmacs::transport::{read_message, write_message};

    let daemon = TestDaemon::spawn_with_config(
        r#"
        pmacs.command.define {
          name = "test.go-instance",
          description = "themes round 2: switch to the instance buffer",
          fn = function()
            _G.__orig = pmacs.window.buffer()
            local id = pmacs.instance.show()
            pmacs.window.switch_buffer(id)
          end,
        }
        pmacs.command.define {
          name = "test.go-back",
          description = "themes round 2: switch back to the original buffer",
          fn = function() pmacs.window.switch_buffer(_G.__orig) end,
        }
        pmacs.keymap.bind { scope = "global", sequence = "<f6>", command = "test.go-instance" }
        pmacs.keymap.bind { scope = "global", sequence = "<f7>", command = "test.go-back" }
        "#,
    );

    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_millis(250)))
        .unwrap();
    let hello: Hello = read_message(&mut stream).expect("read Hello");
    let fid = hello.assigned_frontend_id;
    write_message(
        &mut stream,
        &AttachRequest {
            protocol_version: 16,
            frontend_capabilities: FrontendCapabilities {
                multi_frontend: true,
                crdt_replica: true,
                semantic_render: true,
                ..build_default_caps()
            },
            initial_size: CellSize::new(24, 80),
        },
    )
    .expect("write AttachRequest");

    // Learn buffer A from the bootstrap snapshot, declare, and note
    // the first summary's generation.
    let a = wire_wait_for(&mut stream, "bootstrap BufferSnapshot", |m| match m {
        InstanceMessage::BufferSnapshot { buffer_id, .. } => Some(buffer_id),
        _ => None,
    });
    wire_viewport(&mut stream, fid, a);
    let gen_a = wire_wait_for(&mut stream, "first FileStyleSummary(A)", |m| match m {
        InstanceMessage::FileStyleSummary {
            buffer_id,
            generation,
            ..
        } if buffer_id == a => Some(generation),
        _ => None,
    });

    // A → B: the follow path writes snapshot(B); re-declare for B.
    wire_key(&mut stream, fid, Key::F(6));
    let b = wire_wait_for(&mut stream, "BufferSnapshot(B)", |m| match m {
        InstanceMessage::BufferSnapshot { buffer_id, .. } if buffer_id != a => Some(buffer_id),
        _ => None,
    });
    wire_viewport(&mut stream, fid, b);

    // B → A: the follow path writes snapshot(A) — the reset under
    // test — and the revisit must re-ship A's summary, unchanged
    // generation included.
    wire_key(&mut stream, fid, Key::F(7));
    wire_wait_for(&mut stream, "BufferSnapshot(A) on revisit", |m| match m {
        InstanceMessage::BufferSnapshot { buffer_id, .. } if buffer_id == a => Some(()),
        _ => None,
    });
    wire_viewport(&mut stream, fid, a);
    let gen_back = wire_wait_for(&mut stream, "re-shipped FileStyleSummary(A)", |m| match m {
        InstanceMessage::FileStyleSummary {
            buffer_id,
            generation,
            ..
        } if buffer_id == a => Some(generation),
        _ => None,
    });
    assert_eq!(
        gen_back, gen_a,
        "the revisit summary arrives at A's unchanged generation"
    );
}

// ---------------------------------------------------------------------------
// 12 + 16 + 19 — ThemeFacts emission discipline; late join; set wipes
// ---------------------------------------------------------------------------

#[test]
fn theme_facts_emission_discipline_and_set_wipes_faces() {
    let mut state = editor();
    type_str(&mut state, "hello");
    let mut sem = semantic(&state);

    // Exactly one authoritative EMPTY table for an unthemed session.
    assert_eq!(theme_facts_of(&sem.render_frame(&state)), Some(Vec::new()));
    assert_eq!(theme_facts_of(&sem.render_frame(&state)), None);

    // A face merge ships the resolved sorted table exactly once.
    exec(
        &state,
        r#"pmacs.theme.merge { ["ui.gutter"] = { fg = 13 }, ["ui.modeline"] = { bg = 236 } }"#,
    );
    let facts = theme_facts_of(&sem.render_frame(&state)).expect("face merge emits");
    let names: Vec<&str> = facts.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["ui.gutter", "ui.modeline"], "sorted, concrete");
    assert_eq!(theme_facts_of(&sem.render_frame(&state)), None);

    // An identical re-merge emits nothing.
    exec(
        &state,
        r#"pmacs.theme.merge { ["ui.gutter"] = { fg = 13 } }"#,
    );
    assert_eq!(
        theme_facts_of(&sem.render_frame(&state)),
        None,
        "identical re-merge is suppressed"
    );

    // Q#TH10: a captures-only wholesale set wipes the faces — the
    // wire ships the empty table and the chrome returns to defaults.
    exec(&state, r"pmacs.theme.set { keyword = { fg = 1 } }");
    assert_eq!(
        theme_facts_of(&sem.render_frame(&state)),
        Some(Vec::new()),
        "set wipes faces along with captures"
    );
    let cells = paint_full_frame(&state, 8, 20);
    assert!(
        at(&cells, 20, 6, 0).style.reverse,
        "chrome is back to the reverse-video default"
    );

    // pmacs.theme.clear also ships the empty table when faces existed.
    exec(
        &state,
        r#"pmacs.theme.merge { ["ui.gutter"] = { fg = 13 } }"#,
    );
    let _ = sem.render_frame(&state);
    exec(&state, "pmacs.theme.clear()");
    assert_eq!(
        theme_facts_of(&sem.render_frame(&state)),
        Some(Vec::new()),
        "clear ships the empty table"
    );
}

#[test]
fn late_joiner_receives_the_face_table_without_new_mutations() {
    let mut state = editor();
    type_str(&mut state, "hello");
    exec(
        &state,
        r#"pmacs.theme.merge { ["ui.gutter"] = { fg = 13 } }"#,
    );

    // First session consumes the table.
    let mut a = semantic(&state);
    assert!(theme_facts_of(&a.render_frame(&state)).is_some());
    assert_eq!(theme_facts_of(&a.render_frame(&state)), None);

    // A second, later attachment gets the authoritative table on its
    // first frame with zero mutations since (Q#TH7 None seeding).
    let mut b = semantic(&state);
    let facts = theme_facts_of(&b.render_frame(&state)).expect("late joiner is corrected");
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].name, "ui.gutter");
}

// ---------------------------------------------------------------------------
// 13 — face-only mutations stay off the syntax paths
// ---------------------------------------------------------------------------

#[test]
fn face_only_mutations_ship_facts_but_no_spans_or_summary() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = open_and_wait_for_parse(rust_fixture(&dir));
    let mut sem = semantic(&state);
    let first = sem.render_frame(&state);
    assert!(has_style_spans(&first) && summary_of(&first).is_some());

    exec(
        &state,
        r#"pmacs.theme.merge { ["ui.modeline"] = { bg = 236 } }"#,
    );
    let frame = sem.render_frame(&state);
    assert!(theme_facts_of(&frame).is_some(), "the face table ships");
    assert!(
        !has_style_spans(&frame),
        "a face-only merge never re-runs the tree-sitter path"
    );
    assert!(
        summary_of(&frame).is_none(),
        "the unchanged summary is payload-suppressed"
    );
}

// ---------------------------------------------------------------------------
// 15 — resolution is daemon-side
// ---------------------------------------------------------------------------

#[test]
fn resolution_is_daemon_side_for_the_diag_family() {
    let mut state = editor();
    type_str(&mut state, "hello");
    let mut sem = semantic(&state);
    let _ = sem.render_frame(&state);

    exec(&state, r#"pmacs.theme.merge { ["ui.diag"] = { fg = 93 } }"#);
    let facts = theme_facts_of(&sem.render_frame(&state)).expect("emits");
    let names: Vec<&str> = facts.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "ui.diag.error",
            "ui.diag.hint",
            "ui.diag.info",
            "ui.diag.warning"
        ],
        "the walk happens daemon-side: concrete children ship, the parent doesn't"
    );
}

// ---------------------------------------------------------------------------
// 17 — the daemon version gate (v15 peer never receives ThemeFacts)
// ---------------------------------------------------------------------------

#[cfg(feature = "crdt")]
#[test]
fn v15_peer_never_receives_theme_facts_and_v16_does() {
    use common::daemon::{TestDaemon, build_default_caps};
    use pmacs::protocol::{AttachRequest, FrontendCapabilities, FrontendEvent, Hello};
    use pmacs::transport::{read_message, write_message};

    fn semantic_caps() -> FrontendCapabilities {
        FrontendCapabilities {
            multi_frontend: true,
            crdt_replica: true,
            semantic_render: true,
            ..build_default_caps()
        }
    }

    /// Attach a semantic session at `version`, declare a viewport,
    /// and report `(saw_theme_facts, saw_style_spans)` within the
    /// deadline.
    fn probe(daemon: &TestDaemon, version: u32) -> (bool, bool) {
        let mut stream = daemon.connect();
        stream
            .set_read_timeout(Some(Duration::from_millis(250)))
            .unwrap();
        let hello: Hello = read_message(&mut stream).expect("read Hello");
        let fid = hello.assigned_frontend_id;
        write_message(
            &mut stream,
            &AttachRequest {
                protocol_version: version,
                frontend_capabilities: semantic_caps(),
                initial_size: CellSize::new(24, 80),
            },
        )
        .expect("write AttachRequest");
        // Learn a buffer id from the bootstrap snapshot, declare the
        // viewport, then classify what arrives.
        let mut buf = None;
        let learn_by = Instant::now() + Duration::from_secs(2);
        while Instant::now() < learn_by && buf.is_none() {
            if let Ok(InstanceMessage::BufferSnapshot { buffer_id, .. }) =
                read_message::<InstanceMessage>(&mut stream)
            {
                buf = Some(buffer_id);
            }
        }
        let buffer_id = buf.expect("received a BufferSnapshot");
        write_message(
            &mut stream,
            &FrontendEvent::Viewport {
                frontend_id: fid,
                buffer_id,
                visible: ByteRange {
                    start: 0,
                    end: 4096,
                },
                generation: 0,
            },
        )
        .expect("write Viewport");
        let deadline = Instant::now() + Duration::from_secs(3);
        let (mut saw_facts, mut saw_spans) = (false, false);
        while Instant::now() < deadline && !(saw_facts && saw_spans) {
            match read_message::<InstanceMessage>(&mut stream) {
                Ok(InstanceMessage::ThemeFacts { .. }) => saw_facts = true,
                Ok(InstanceMessage::StyleSpans { .. }) => saw_spans = true,
                Ok(_) | Err(_) => {}
            }
        }
        (saw_facts, saw_spans)
    }

    let daemon = TestDaemon::spawn();
    let (v16_facts, v16_spans) = probe(&daemon, 16);
    assert!(v16_spans, "a v16 semantic session receives StyleSpans");
    assert!(
        v16_facts,
        "a v16 semantic session receives the authoritative ThemeFacts"
    );
    let (v15_facts, v15_spans) = probe(&daemon, 15);
    assert!(v15_spans, "a v15 peer still receives StyleSpans");
    assert!(
        !v15_facts,
        "the daemon skip arm must keep ThemeFacts off a v15 wire"
    );
}

// ---------------------------------------------------------------------------
// 24 — the Lua surface is unchanged
// ---------------------------------------------------------------------------

#[test]
fn lua_surface_lists_faces_alongside_captures() {
    let state = editor();
    exec(
        &state,
        r#"pmacs.theme.merge { ui = { fg = 3 }, ["ui.modeline"] = { fg = 252, bg = 236 },
                              keyword = { fg = 1 } }"#,
    );
    let (has_ui, has_modeline, has_keyword): (bool, bool, bool) = eval(
        &state,
        r#"
        local t = pmacs.theme.current()
        return t["ui"] ~= nil, t["ui.modeline"] ~= nil, t["keyword"] ~= nil
        "#,
    );
    assert!(has_ui && has_modeline && has_keyword);
    let (fg, bg): (i64, i64) = eval(
        &state,
        r#"
        local s = pmacs.theme.get("ui.modeline")
        return s.fg, s.bg
        "#,
    );
    assert_eq!((fg, bg), (252, 236), "get returns the set face style");
}

// ---------------------------------------------------------------------------
// Minibuffer faces (items 2's remaining surfaces): prompt/input/fill
// through the dispatched M-x path; the candidate suffix through
// selection.
// ---------------------------------------------------------------------------

#[test]
fn minibuffer_and_candidate_faces_apply_through_m_x() {
    let mut state = editor();
    let (rows, cols) = (8u32, 40u32);
    exec(
        &state,
        r#"pmacs.theme.merge {
            ["ui.minibuffer"] = { fg = 6 },
            ["ui.minibuffer.candidate"] = { fg = 11 },
        }"#,
    );

    alt(&mut state, 'x');
    type_str(&mut state, "window");
    press(&mut state, KeyCode::Down);
    let cells = paint_full_frame(&state, rows, cols);
    let bottom = rows - 1;
    let text = row_text(&cells, cols, bottom);
    assert!(text.starts_with("M-x window"), "minibuffer open: {text:?}");

    // Prompt + input carry the minibuffer face, plain surface.
    let prompt_cell = at(&cells, cols, bottom, 0);
    assert_eq!(prompt_cell.style.fg, Color::Indexed(6));
    assert!(!prompt_cell.style.reverse);

    // The selected-candidate suffix (`  [cand]`) carries its own face.
    if let Some(bracket) = text.find('[') {
        let cand_cell = at(&cells, cols, bottom, bracket as u32);
        assert_eq!(
            cand_cell.style.fg,
            Color::Indexed(11),
            "candidate suffix face"
        );
        assert!(
            !cand_cell.style.reverse,
            "candidate surface resets to plain"
        );
    } else {
        panic!("no candidate suffix rendered: {text:?}");
    }
}
