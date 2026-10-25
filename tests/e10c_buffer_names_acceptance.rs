// tests/e10c_buffer_names_acceptance.rs --- E10c: a buffer's name is
// not its path (D40).

//! E10c's rows through the keys a user presses (`EditorState::dispatch_key`,
//! the TUI's path) and read where a user reads a name: the mode line of a
//! painted frame (`paint_frame`). A file buffer is named by its file's
//! basename; two that collide each take the fewest trailing directories
//! that tell them apart, written forward (`src/a.rs`, `tests/a.rs`), and
//! a survivor returns to its basename; an explicitly named buffer keeps
//! its name through a collision, and the derived name moves around it.
//! The names are recomputed on open, kill, rename and save-as.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::cell::{CellGrid, Glyph};
use pmacs::editor::EditorState;
use pmacs::protocol::{CellSize, FrontendId};

#[path = "common/editor.rs"]
mod editor;
use editor::{eval, exec};

const ROWS: u32 = 30;
const COLS: u32 = 120;

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

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        press(s, KeyCode::Char(ch));
    }
}

fn fresh() -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    exec(&s, "pmacs.lsp.config = {}");
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(ROWS, COLS));
    s
}

fn active(s: &EditorState) -> bool {
    eval(s, "return pmacs.minibuffer.is_active()")
}

fn contents(s: &EditorState) -> String {
    eval(s, "return pmacs.minibuffer.contents()")
}

fn candidates(s: &EditorState) -> Vec<String> {
    eval(s, "return pmacs.minibuffer.candidates()")
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

/// Paint one real frame and return its rows as text.
fn painted_rows(s: &EditorState) -> Vec<String> {
    let size = CellSize::new(ROWS, COLS);
    let mut cells = vec![pmacs::cell::Cell::default(); (ROWS * COLS) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: COLS,
        size,
    };
    pmacs::editor::paint_frame(s, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);
    (0..ROWS)
        .map(|row| {
            (0..COLS)
                .map(|col| match &cells[(row * COLS + col) as usize].glyph {
                    Glyph::Char(ch) => *ch,
                    Glyph::Cluster(_) => '?',
                    Glyph::Continuation => ' ',
                })
                .collect::<String>()
        })
        .collect()
}

/// The name the active window's mode line shows: the token after its
/// ` +` (focused) and modified markers.
fn mode_line_name(s: &EditorState) -> String {
    let rows = painted_rows(s);
    let rest = rows
        .iter()
        .find_map(|row| row.strip_prefix(" + ").or_else(|| row.strip_prefix(" +*")))
        .unwrap_or_else(|| panic!("no focused mode line in {rows:#?}"));
    rest.trim_start().split(' ').next().unwrap_or("").to_owned()
}

/// Every buffer's name, in the registry's order.
fn names(s: &EditorState) -> Vec<String> {
    eval(
        s,
        "local t = {}\n\
         for _, id in ipairs(pmacs.buffer.list()) do\n\
           t[#t + 1] = pmacs.describe.buffer(id).name\n\
         end\n\
         return t",
    )
}

/// Empty a prefilled field as a user does, by holding backspace.
fn clear_field(s: &mut EditorState) {
    for _ in 0..contents(s).chars().count() {
        press(s, KeyCode::Backspace);
    }
    assert_eq!(contents(s), "", "fixture: backspace empties the field");
}

/// `C-x C-f`, the prefill replaced by `path`, RET.
fn find_file(s: &mut EditorState, path: &Path) {
    ctrl(s, 'x');
    ctrl(s, 'f');
    assert!(active(s), "C-x C-f opens the prompt");
    clear_field(s);
    type_str(s, &path.display().to_string());
    press(s, KeyCode::Enter);
    assert!(!active(s), "RET closed the prompt");
}

/// `C-x k` RET on the prefill: kill the buffer the window shows.
fn kill_this(s: &mut EditorState) {
    ctrl(s, 'x');
    press(s, KeyCode::Char('k'));
    assert!(active(s), "C-x k opens the prompt");
    press(s, KeyCode::Enter);
    assert!(!active(s), "RET closed the prompt");
}

/// `C-x b`, `text`, RET.
fn switch_to(s: &mut EditorState, text: &str) {
    ctrl(s, 'x');
    press(s, KeyCode::Char('b'));
    assert!(active(s), "C-x b opens the prompt");
    type_str(s, text);
    press(s, KeyCode::Enter);
    assert!(!active(s), "RET closed the prompt");
}

/// A project with `src/a.rs`, `tests/a.rs` and `src/b.rs`.
fn project() -> tempfile::TempDir {
    let td = tempfile::tempdir().expect("tempdir");
    for rel in ["src/a.rs", "tests/a.rs", "src/b.rs"] {
        let path = td.path().join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, b"x\n").expect("write");
    }
    td
}

/// E10c.2's witness: two `a.rs` show as `src/a.rs` and `tests/a.rs`, on
/// the mode line and in `C-x b`'s candidates, and killing one returns the
/// other to `a.rs`.
#[test]
fn two_a_rs_show_as_src_a_rs_and_tests_a_rs_and_a_kill_returns_the_survivor() {
    let td = project();
    let mut s = fresh();
    find_file(&mut s, &td.path().join("src/a.rs"));
    assert_eq!(mode_line_name(&s), "a.rs", "alone, a file is its basename");
    find_file(&mut s, &td.path().join("tests/a.rs"));
    assert_eq!(mode_line_name(&s), "tests/a.rs");
    find_file(&mut s, &td.path().join("src/b.rs"));
    assert_eq!(mode_line_name(&s), "b.rs", "a basename no other holds");
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "a.rs");
    let offered = candidates(&s);
    assert!(
        offered.contains(&"src/a.rs".to_owned()) && offered.contains(&"tests/a.rs".to_owned()),
        "C-x b offers both by the names that tell them apart: {offered:?}"
    );
    clear_field(&mut s);
    type_str(&mut s, "src/a");
    press(&mut s, KeyCode::Enter);
    assert_eq!(mode_line_name(&s), "src/a.rs", "C-x b src/a RET reaches it");
    assert_eq!(status(&s), "switch-buffer: showing src/a.rs");

    kill_this(&mut s);
    assert_eq!(status(&s), "kill-buffer: killed src/a.rs");
    let left = names(&s);
    assert!(
        left.contains(&"a.rs".to_owned()) && !left.iter().any(|n| n.ends_with("/a.rs")),
        "the survivor returned to its basename: {left:?}"
    );
    switch_to(&mut s, "a.rs");
    assert_eq!(mode_line_name(&s), "a.rs", "and the mode line says so");
    assert_eq!(
        eval::<String>(&s, "return pmacs.window.buffer():path()"),
        td.path().join("tests/a.rs").display().to_string(),
        "it is the other file"
    );
}

/// E10c.2's second witness: a buffer a user or a script named
/// explicitly keeps its name through a collision, and the derived name
/// takes the directories that tell it apart from it; the explicit name
/// stays when the derived one leaves, and a file buffer renamed
/// explicitly is never rewritten by a later open.
#[test]
fn an_explicitly_named_buffer_keeps_its_name_through_a_collision() {
    let td = project();
    let mut s = fresh();
    exec(&s, "_G.MINE = pmacs.buffer.create('a.rs')");
    find_file(&mut s, &td.path().join("src/a.rs"));
    assert_eq!(
        mode_line_name(&s),
        "src/a.rs",
        "the file's name moves around the explicit one"
    );
    assert_eq!(
        eval::<String>(&s, "return pmacs.describe.buffer(_G.MINE).name"),
        "a.rs",
        "the explicit name is never rewritten"
    );
    exec(
        &s,
        "pmacs.buffer.set_name(pmacs.window.buffer(), 'chosen.rs')",
    );
    find_file(&mut s, &td.path().join("tests/a.rs"));
    assert_eq!(
        mode_line_name(&s),
        "tests/a.rs",
        "a second a.rs meets only the explicit a.rs"
    );
    let all = names(&s);
    assert!(
        all.contains(&"chosen.rs".to_owned()) && all.contains(&"a.rs".to_owned()),
        "the file renamed by hand keeps its name: {all:?}"
    );
    exec(&s, "pmacs.buffer.kill(_G.MINE)");
    assert_eq!(
        mode_line_name(&s),
        "a.rs",
        "with the explicit a.rs gone, the file takes its basename"
    );
}

/// The names follow a save-as and a rename: `C-x C-w` moves `tests/a.rs`
/// to `tests/c.rs`, so `src/a.rs` returns to `a.rs`; a rename through
/// `pmacs.fs.rename` brings `src/b.rs` beside `tests/c.rs` as `src/c.rs`,
/// and both take their directories.
#[test]
fn save_as_and_rename_recompute_every_name() {
    let td = project();
    let mut s = fresh();
    find_file(&mut s, &td.path().join("src/a.rs"));
    find_file(&mut s, &td.path().join("src/b.rs"));
    find_file(&mut s, &td.path().join("tests/a.rs"));
    assert_eq!(mode_line_name(&s), "tests/a.rs");

    let moved = td.path().join("tests/c.rs");
    ctrl(&mut s, 'x');
    ctrl(&mut s, 'w');
    assert!(active(&s), "C-x C-w opens the prompt");
    clear_field(&mut s);
    type_str(&mut s, &moved.display().to_string());
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        mode_line_name(&s),
        "c.rs",
        "save-as names it by its new file"
    );
    let after_save = names(&s);
    assert!(
        after_save.contains(&"a.rs".to_owned()) && after_save.contains(&"b.rs".to_owned()),
        "src/a.rs returned to a.rs: {after_save:?}"
    );

    let from = td.path().join("src/b.rs").display().to_string();
    let to = td.path().join("src/c.rs").display().to_string();
    exec(
        &s,
        &format!(
            "_G.RENAMED = nil\n\
             pmacs.async(function()\n\
               _G.RENAMED = pcall(function() pmacs.fs.rename({from:?}, {to:?}):await() end)\n\
             end)"
        ),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while eval::<Option<bool>>(&s, "return _G.RENAMED").is_none() {
        assert!(Instant::now() < deadline, "the rename settles");
        s.tick_async();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        eval::<bool>(&s, "return _G.RENAMED"),
        "the rename succeeded"
    );
    assert_eq!(
        mode_line_name(&s),
        "tests/c.rs",
        "the shown buffer took its directory beside the renamed one"
    );
    let after_rename = names(&s);
    assert!(
        after_rename.contains(&"src/c.rs".to_owned()) && after_rename.contains(&"a.rs".to_owned()),
        "the renamed buffer followed its file: {after_rename:?}"
    );
}

#[path = "common/iso.rs"]
mod iso;
