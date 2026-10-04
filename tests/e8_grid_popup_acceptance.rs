// tests/e8_grid_popup_acceptance.rs --- E8.4, the popup in the grid.

//! The hover and signature popup painted by the grid (E8.4), through
//! the TUI's production path: keys through `EditorState::dispatch_key`,
//! the frame through `pmacs::editor::paint_frame`, the painter every
//! grid frontend's frames come from. The fake language server answers.
//!
//! The grid shows the rows that fit, at most half the window, and
//! closes with a row counting what it did not show and naming `C-c H`;
//! `*lsp-help*` keeps the whole text, written through
//! `Buffer::set_generated_contents`.

#[path = "common/iso.rs"]
mod iso;

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pmacs::cell::{Cell, CellGrid, CellSize, Color, Glyph};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

const ROWS: u32 = 24;
const COLS: u32 = 80;

/// An editor visiting `a.rs` (in a cargo project) holding `source`, the
/// rust server pointed at the fake in `mode` with `env`.
fn editor(source: &str, mode: &str, env: &[(&str, &str)]) -> (EditorState, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("Cargo.toml"), b"[package]\nname=\"x\"\n").expect("write");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, source).expect("write a.rs");
    let state = EditorState::new_with_roots(&iso::roots());
    let fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp");
    let mut env_lua = format!("PMACS_FAKE_LSP_MODE = '{mode}',");
    for (k, v) in env {
        let _ = write!(env_lua, " {k} = '{v}',");
    }
    exec(
        &state,
        &format!(
            "pmacs.lsp.config = {{ rust = {{ command = {fake:?}, env = {{ {env_lua} }} }} }}\n\
             pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    (state, dir)
}

fn exec(state: &EditorState, src: &str) {
    state.lua_host.lua().load(src).exec().expect("lua exec");
}

fn key(state: &mut EditorState, k: char, mods: KeyModifiers) {
    state.dispatch_key(FrontendId::LOCAL, KeyEvent::new(KeyCode::Char(k), mods));
}

fn chord(state: &mut EditorState, ch: char) {
    key(state, 'c', KeyModifiers::CONTROL);
    key(state, ch, KeyModifiers::NONE);
}

fn tick(state: &mut EditorState) {
    state.tick_processes();
    state.tick_lsp();
    state.tick_async();
}

fn paint(state: &EditorState) -> Vec<Cell> {
    let mut backing = vec![Cell::default(); (ROWS * COLS) as usize];
    let mut grid = CellGrid {
        cells: &mut backing,
        stride: COLS,
        size: CellSize::new(ROWS, COLS),
    };
    let _ = pmacs::editor::paint_frame(
        state,
        FrontendId::LOCAL,
        &std::collections::HashMap::new(),
        &mut grid,
        CellSize::new(ROWS, COLS),
    );
    backing
}

fn row_text(cells: &[Cell], row: u32) -> String {
    (0..COLS)
        .map(|c| match &cells[(row * COLS + c) as usize].glyph {
            Glyph::Char(ch) => *ch,
            _ => ' ',
        })
        .collect()
}

/// The row a painted frame shows `needle` on, if any.
fn find_row(cells: &[Cell], needle: &str) -> Option<u32> {
    (0..ROWS).find(|&r| row_text(cells, r).contains(needle))
}

/// Ask with `C-c ch` every half second until a painted frame shows
/// `needle`; the server answers only once it has initialized.
fn ask_until_painted(state: &mut EditorState, ch: char, needle: &str) -> Vec<Cell> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        chord(state, ch);
        let half = Instant::now() + Duration::from_millis(500);
        while Instant::now() < half {
            tick(state);
            let cells = paint(state);
            if find_row(&cells, needle).is_some() {
                return cells;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    panic!("the grid never painted {needle:?}");
}

/// E8.4's gate row: `C-c h` paints the hover in the grid below the
/// caret's line, in the completion popup's cells, the TUI no longer
/// echoing one line on the status line; motion out of its range, `C-g`
/// and an edit each take it off the grid.
#[test]
fn e8_4_the_grid_paints_the_hover_below_the_caret_and_clears_it() {
    let (mut state, _dir) = editor("fn main() { let value = 1; }\n", "hover", &[]);
    let cells = ask_until_painted(&mut state, 'h', "Synthetic hover content");
    let title = find_row(&cells, "# pmacs-fake-lsp").expect("the first line");
    let body = find_row(&cells, "Synthetic hover content").expect("the body");
    assert_eq!(title, 1, "below the caret's line (row 0)");
    assert_eq!(body, title + 2, "the blank line between them kept");
    assert!(
        !state.core.borrow().status.contains("pmacs-fake-lsp"),
        "the one-line echo is gone"
    );

    key(&mut state, 'e', KeyModifiers::CONTROL);
    assert!(
        find_row(&paint(&state), "Synthetic hover content").is_none(),
        "motion out of the range clears it"
    );
    key(&mut state, 'a', KeyModifiers::CONTROL);
    assert!(
        find_row(&paint(&state), "Synthetic hover content").is_none(),
        "and coming back does not restore it"
    );

    ask_until_painted(&mut state, 'h', "Synthetic hover content");
    key(&mut state, 'g', KeyModifiers::CONTROL);
    assert!(
        find_row(&paint(&state), "Synthetic hover content").is_none(),
        "C-g"
    );

    ask_until_painted(&mut state, 'h', "Synthetic hover content");
    key(&mut state, 'x', KeyModifiers::NONE);
    assert!(
        find_row(&paint(&state), "Synthetic hover content").is_none(),
        "an edit"
    );
}

/// The bound in the grid: two hundred lines show the rows that fit in
/// half the window and close with a row counting the rest; three
/// hundred, past the wire's bound, count what the bound left out too;
/// and `*lsp-help*` holds every line.
#[test]
fn e8_4_a_long_hover_shows_what_fits_counts_the_rest_and_lsp_help_holds_it_all() {
    let (mut state, _dir) = editor(
        "fn main() {}\n",
        "hover",
        &[("PMACS_FAKE_LSP_HOVER_LINES", "200")],
    );
    let cells = ask_until_painted(&mut state, 'h', "more lines");
    // Half the window's rows: the lines that fit from row 1 (below the
    // caret's line), then the closing row counting the rest of 202.
    let footer = find_row(&cells, "more lines").expect("the closing row");
    let shown = footer - 1;
    assert!((3..=12).contains(&shown), "half a 24-row window: {shown}");
    assert!(
        row_text(&cells, footer).contains(&format!(
            "… {} more lines · C-c H opens *lsp-help*",
            202 - shown
        )),
        "202 lines, {shown} shown: {:?}",
        row_text(&cells, footer)
    );
    drop(state);

    let (mut state, _dir) = editor(
        "fn main() {}\n",
        "hover",
        &[("PMACS_FAKE_LSP_HOVER_LINES", "300")],
    );
    let cells = ask_until_painted(&mut state, 'h', "more lines");
    let footer = find_row(&cells, "more lines").expect("the closing row");
    let shown = footer - 1;
    assert!(
        row_text(&cells, footer).contains(&format!("… {} more lines", 256 - shown + 46)),
        "the 256 sent less the {shown} shown, and the 46 the bound left out: {:?}",
        row_text(&cells, footer)
    );

    // `C-c H`: the whole text, unbounded, in *lsp-help*.
    key(&mut state, 'c', KeyModifiers::CONTROL);
    key(&mut state, 'H', KeyModifiers::SHIFT);
    let deadline = Instant::now() + Duration::from_secs(10);
    let lines = loop {
        tick(&mut state);
        let n: i64 = state
            .lua_host
            .lua()
            .load(
                "local b = pmacs.window.buffer()\n\
                 if b:name() ~= '*lsp-help*' then return -1 end\n\
                 local n = 0\n\
                 for _ in b:slice(0, b:len()):gmatch('line %d+ of the long hover') do n = n + 1 end\n\
                 return n",
            )
            .eval()
            .unwrap_or(-1);
        if n >= 0 || Instant::now() >= deadline {
            break n;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(lines, 300, "*lsp-help* holds every line the server sent");
}

/// The one placement rule in cells: with no room below the caret's line
/// the popup goes above it.
#[test]
fn e8_4_the_grid_popup_flips_above_on_the_last_row() {
    let mut source = String::new();
    for i in 0..40 {
        let _ = writeln!(source, "let line_{i} = {i};");
    }
    let (mut state, _dir) = editor(&source, "hover", &[("PMACS_FAKE_LSP_HOVER_LINES", "4")]);
    // Down to line 20, near the window's last text row, with six popup
    // lines to show and no room for them below.
    for _ in 0..20 {
        key(&mut state, 'n', KeyModifiers::CONTROL);
    }
    let cells = ask_until_painted(&mut state, 'h', "line 3 of the long hover");
    let last = find_row(&cells, "line 3 of the long hover").expect("the last line");
    let caret_row = find_row(&cells, "let line_20").expect("the caret's line");
    assert!(
        ROWS - 2 - caret_row < 6,
        "the setup must leave no room below: caret row {caret_row}"
    );
    assert!(
        last < caret_row,
        "above the caret's line: {last} vs {caret_row}"
    );
}

/// Themed: a `ui.popup` background the theme sets is the cells'.
#[test]
fn e8_4_the_grid_popup_takes_the_ui_popup_face() {
    let (mut state, _dir) = editor("fn main() {}\n", "hover", &[]);
    exec(
        &state,
        "pmacs.theme.set { [\"ui.popup\"] = { fg = { 255, 255, 255 }, bg = { 200, 0, 0 } } }",
    );
    let cells = ask_until_painted(&mut state, 'h', "Synthetic hover content");
    let row = find_row(&cells, "Synthetic hover content").expect("the body");
    let col = row_text(&cells, row).find('S').expect("the text") as u32;
    assert_eq!(
        cells[(row * COLS + col) as usize].style.bg,
        Color::Rgb(200, 0, 0),
        "the ui.popup face paints the popup's cells"
    );
}

/// The signature in the grid: `C-c s` inside a call paints its label.
#[test]
fn e8_4_the_grid_paints_the_signature() {
    let (mut state, _dir) = editor("fn main() { let value = 1; }\n", "sighelp", &[]);
    for _ in 0..8 {
        key(&mut state, 'f', KeyModifiers::CONTROL);
    }
    let cells = ask_until_painted(&mut state, 's', "fn echo(name: &str, count: usize)");
    assert!(find_row(&cells, "Echoes `name` `count` times.").is_some());
}

/// E8.5 in the grid: the signature's active parameter is drawn in its
/// own face (bold and underlined when the theme sets none), and nothing
/// else on the label is.
#[test]
fn e8_5_the_grid_marks_the_active_parameter() {
    let (mut state, _dir) = editor("fn main() { let value = 1; }\n", "sighelp", &[]);
    for _ in 0..8 {
        key(&mut state, 'f', KeyModifiers::CONTROL);
    }
    let cells = ask_until_painted(&mut state, 's', "fn echo(name: &str, count: usize)");
    let row = find_row(&cells, "fn echo(").expect("the label");
    let text = row_text(&cells, row);
    let start = text.find("count: usize").expect("the parameter") as u32;
    let marked = |col: u32| {
        let style = cells[(row * COLS + col) as usize].style;
        style.bold && style.underline != pmacs::cell::UnderlineStyle::None
    };
    assert!(
        (start..start + 12).all(marked),
        "every cell of the active parameter is marked: {text:?}"
    );
    let name = text.find("name: &str").expect("the other parameter") as u32;
    assert!(
        !(name..name + 10).any(marked),
        "the other parameter is not: {text:?}"
    );
}

/// E8.5: the mark survives a theme. A theme that sets only `ui.popup`
/// resolves `ui.popup.active-parameter` to it (the faces' dotted-prefix
/// walk), and the active parameter must still be marked.
#[test]
fn e8_5_the_active_parameter_stays_marked_under_a_theme_that_sets_only_ui_popup() {
    let (mut state, _dir) = editor("fn main() { let value = 1; }\n", "sighelp", &[]);
    exec(
        &state,
        "pmacs.theme.set { [\"ui.popup\"] = { fg = { 255, 255, 255 }, bg = { 0, 0, 120 } } }",
    );
    for _ in 0..8 {
        key(&mut state, 'f', KeyModifiers::CONTROL);
    }
    let cells = ask_until_painted(&mut state, 's', "fn echo(name: &str, count: usize)");
    let row = find_row(&cells, "fn echo(").expect("the label");
    let text = row_text(&cells, row);
    let start = text.find("count: usize").expect("the parameter") as u32;
    let style = cells[(row * COLS + start) as usize].style;
    assert!(
        style.bold && style.underline != pmacs::cell::UnderlineStyle::None,
        "the active parameter stays marked under ui.popup alone: {style:?}"
    );
    assert_eq!(
        style.bg,
        Color::Rgb(0, 0, 120),
        "and takes the popup's face"
    );
}

/// Under word wrap a screen row is not a line: a 300-column line above
/// the caret's takes four rows of an 80-column window, and the popup must
/// open below the row the caret's line is drawn on, not below the row a
/// line count would put it on, which lands it in the wrapped text above
/// and over the caret's own line. Found probing the prose path the brief
/// named; the completion popup shared the walk.
#[test]
fn e8_4_under_wrap_the_popup_opens_below_the_carets_drawn_row() {
    let long = "x".repeat(300);
    let (mut state, _dir) = editor(&format!("{long}\nfn main() {{}}\n"), "hover", &[]);
    key(&mut state, 'n', KeyModifiers::CONTROL);
    let cells = ask_until_painted(&mut state, 'h', "Synthetic hover content");
    let caret = find_row(&cells, "fn main").expect("the caret's line is not covered");
    let title = find_row(&cells, "# pmacs-fake-lsp").expect("the popup");
    assert!(
        caret >= 3,
        "the long line wraps over the rows above it: {caret}"
    );
    assert_eq!(
        title,
        caret + 1,
        "the popup opens on the row below the caret's"
    );
}
