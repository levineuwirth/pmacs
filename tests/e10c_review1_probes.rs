// tests/e10c_review1_probes.rs --- E10c review 1's behavioral probes.

//! States a user reaches that E10c's own witnesses did not choose, each
//! driven through the keys (`EditorState::dispatch_key`, the TUI's path)
//! and read where a user reads it: the painted mode line, `C-x b`'s
//! band, `*buffer-list*` and `*help*`.
//!
//! - A directory rename that brings two alike basenames together and
//!   then parts them (`reconcile_rename`'s doc claims it; the phase's
//!   rename row renames one file).
//! - An explicit name equal to the middle of a file's suffix chain
//!   (`src/a.rs`), which the handoff names as unwitnessed.
//! - `C-x C-w` onto a file another buffer already holds, the one state
//!   that reaches the `<N>` suffix by the keys: which buffer keeps the
//!   plain name.
//! - `*buffer-list*`'s `File` column for a buffer of a megabyte and for
//!   a name holding a multi-byte character.

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
const COLS: u32 = 160;

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

/// The name the active window's mode line shows.
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

/// The name of the buffer holding `path`, if one does; the first such
/// buffer in the registry's order.
fn name_of_path(s: &EditorState, path: &Path) -> Option<String> {
    eval(
        s,
        &format!(
            "for _, id in ipairs(pmacs.buffer.list()) do\n\
               if id:path() == {p:?} then return pmacs.describe.buffer(id).name end\n\
             end\n\
             return nil",
            p = path.display().to_string()
        ),
    )
}

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

/// `C-x b`, `text`, RET.
fn switch_to(s: &mut EditorState, text: &str) {
    ctrl(s, 'x');
    press(s, KeyCode::Char('b'));
    assert!(active(s), "C-x b opens the prompt");
    type_str(s, text);
    press(s, KeyCode::Enter);
    assert!(!active(s), "RET closed the prompt");
}

/// `C-x k` RET on the prefill.
fn kill_this(s: &mut EditorState) {
    ctrl(s, 'x');
    press(s, KeyCode::Char('k'));
    assert!(active(s), "C-x k opens the prompt");
    press(s, KeyCode::Enter);
}

/// `pmacs.fs.rename(from, to)` driven to its end, as a package moves a
/// file or a directory.
fn rename(s: &mut EditorState, from: &Path, to: &Path) {
    exec(
        s,
        &format!(
            "_G.R1_RENAMED = nil\n\
             pmacs.async(function()\n\
               _G.R1_RENAMED = pcall(function() pmacs.fs.rename({from:?}, {to:?}):await() end)\n\
             end)",
            from = from.display().to_string(),
            to = to.display().to_string()
        ),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while eval::<Option<bool>>(s, "return _G.R1_RENAMED").is_none() {
        assert!(Instant::now() < deadline, "the rename settles");
        s.tick_async();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        eval::<bool>(s, "return _G.R1_RENAMED"),
        "fixture: the rename succeeded"
    );
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, bytes).expect("write");
}

/// The last component of `dir`, as a collision that grows into it
/// writes it.
fn last(dir: &Path) -> String {
    dir.file_name()
        .expect("a named directory")
        .to_string_lossy()
        .into_owned()
}

/// P1. A directory rename moves every buffer under it, and the names are
/// recomputed over the whole set: renaming `tmp` to `x/src` brings its
/// `a.rs` beside `src/a.rs`, so both grow a directory further; renaming
/// `x/src` to `x/lib` parts them, and both shrink back.
#[test]
fn r1_a_directory_rename_brings_two_alike_basenames_together_and_parts_them() {
    let td = tempfile::tempdir().expect("tempdir");
    write(&td.path().join("src/a.rs"), b"x\n");
    write(&td.path().join("tmp/a.rs"), b"y\n");
    std::fs::create_dir_all(td.path().join("x")).expect("mkdir");
    let mut s = fresh();
    find_file(&mut s, &td.path().join("src/a.rs"));
    find_file(&mut s, &td.path().join("tmp/a.rs"));
    assert_eq!(mode_line_name(&s), "tmp/a.rs", "fixture: the pair collides");

    rename(&mut s, &td.path().join("tmp"), &td.path().join("x/src"));
    let root = last(td.path());
    assert_eq!(
        mode_line_name(&s),
        "x/src/a.rs",
        "the moved buffer takes the directories its new path needs"
    );
    assert_eq!(
        name_of_path(&s, &td.path().join("src/a.rs")),
        Some(format!("{root}/src/a.rs")),
        "the buffer that did not move grows too: {:?}",
        names(&s)
    );

    rename(&mut s, &td.path().join("x/src"), &td.path().join("x/lib"));
    assert_eq!(mode_line_name(&s), "lib/a.rs", "parted, it shrinks back");
    assert_eq!(
        name_of_path(&s, &td.path().join("src/a.rs")),
        Some("src/a.rs".to_owned()),
        "and so does the other: {:?}",
        names(&s)
    );
}

/// P2. An explicit name equal to the middle link of a file's suffix
/// chain: a script's buffer named `src/a.rs` beside the files `src/a.rs`
/// and `tests/a.rs`. The explicit name is never rewritten; only the file
/// it equals grows past it, the other keeps the shortest name that tells
/// it apart; `C-x b src/a.rs RET` reaches the explicit buffer, whose name
/// it is; and killing that buffer returns the file to `src/a.rs`.
#[test]
fn r1_an_explicit_name_equal_to_a_middle_suffix_moves_only_the_file_it_equals() {
    let td = tempfile::tempdir().expect("tempdir");
    write(&td.path().join("src/a.rs"), b"x\n");
    write(&td.path().join("tests/a.rs"), b"y\n");
    let mut s = fresh();
    exec(&s, "_G.MINE = pmacs.buffer.create('src/a.rs')");
    find_file(&mut s, &td.path().join("src/a.rs"));
    find_file(&mut s, &td.path().join("tests/a.rs"));
    let root = last(td.path());
    assert_eq!(
        name_of_path(&s, &td.path().join("src/a.rs")),
        Some(format!("{root}/src/a.rs")),
        "the file the explicit name equals takes one directory more: {:?}",
        names(&s)
    );
    assert_eq!(
        mode_line_name(&s),
        "tests/a.rs",
        "the other file needs no more than its own directory"
    );
    assert_eq!(
        eval::<String>(&s, "return pmacs.describe.buffer(_G.MINE).name"),
        "src/a.rs",
        "the explicit name is never rewritten"
    );

    switch_to(&mut s, "src/a.rs");
    assert_eq!(status(&s), "switch-buffer: showing src/a.rs");
    assert!(
        eval::<bool>(&s, "return pmacs.window.buffer() == _G.MINE"),
        "C-x b src/a.rs RET reaches the buffer named src/a.rs, the explicit one"
    );

    kill_this(&mut s);
    assert_eq!(status(&s), "kill-buffer: killed src/a.rs");
    assert_eq!(
        name_of_path(&s, &td.path().join("src/a.rs")),
        Some("src/a.rs".to_owned()),
        "with the explicit name gone the file returns to src/a.rs: {:?}",
        names(&s)
    );
}

/// P3. `C-x C-w` onto a file another buffer already holds, answered
/// `yes`: two buffers then hold one path, the one state the keys reach
/// in which no directory tells two names apart, so one takes `<2>`.
/// Emacs leaves the buffer that already shows `a.rs` its name and gives
/// the newcomer `a.rs<2>` (D40 follows Emacs); a name the user is
/// reading should not move because another buffer was saved. The kill
/// of the newcomer returns nothing to anyone, the survivor having kept
/// its name throughout.
#[test]
fn r1_save_as_onto_an_open_file_leaves_the_open_buffer_its_name() {
    let td = tempfile::tempdir().expect("tempdir");
    let file = td.path().join("a.rs");
    write(&file, b"on disk\n");
    let mut s = fresh();
    find_file(&mut s, &file);
    exec(&s, "_G.OPEN = pmacs.window.buffer()");
    assert_eq!(mode_line_name(&s), "a.rs", "fixture: the file's buffer");

    switch_to(&mut s, "*scratch*");
    assert_eq!(mode_line_name(&s), "*scratch*", "fixture: in *scratch*");
    type_str(&mut s, "from scratch");
    ctrl(&mut s, 'x');
    ctrl(&mut s, 'w');
    assert!(active(&s), "C-x C-w opens the prompt");
    clear_field(&mut s);
    type_str(&mut s, &file.display().to_string());
    press(&mut s, KeyCode::Enter);
    assert!(active(&s), "the existing file is a question");
    type_str(&mut s, "yes");
    press(&mut s, KeyCode::Enter);
    assert!(!active(&s), "answered");
    assert_eq!(
        eval::<String>(&s, "return pmacs.window.buffer():path()"),
        file.display().to_string(),
        "fixture: *scratch* now holds the file"
    );

    assert_eq!(
        eval::<String>(&s, "return pmacs.describe.buffer(_G.OPEN).name"),
        "a.rs",
        "the buffer that already showed a.rs keeps its name: {:?}",
        names(&s)
    );
    assert_eq!(
        mode_line_name(&s),
        "a.rs<2>",
        "the newcomer on the same file takes the suffix"
    );
}

/// The display column at which `needle` begins in `line`.
fn column_of(line: &str, needle: &str) -> Option<usize> {
    line.find(needle).map(|i| line[..i].chars().count())
}

/// `C-x C-b`, and the listing's text.
fn buffer_list(s: &mut EditorState) -> String {
    ctrl(s, 'x');
    ctrl(s, 'b');
    eval(
        s,
        "for _, id in ipairs(pmacs.buffer.list()) do\n\
           if pmacs.describe.buffer(id).name == '*buffer-list*' then\n\
             return id:slice(0, id:len())\n\
           end\n\
         end\n\
         error('no *buffer-list*')",
    )
}

/// Every row that shows one of `paths` begins it in the column the
/// header's `File` begins in.
fn assert_file_column_in_line(listing: &str, paths: &[&Path]) {
    let header = listing.lines().next().expect("a header");
    let file_col = column_of(header, "File").expect("the header names File");
    for path in paths {
        let shown = path.display().to_string();
        let row = listing
            .lines()
            .find(|l| l.trim_end().ends_with(&shown))
            .unwrap_or_else(|| panic!("no row shows {shown}:\n{listing}"));
        assert_eq!(
            column_of(row, &shown),
            Some(file_col),
            "the path begins under File:\n{listing}"
        );
    }
}

/// P4. E10c.5's `File` column for a buffer of a megabyte: its size,
/// `1048576 bytes`, is thirteen characters in a twelve-wide field.
#[test]
fn r1_the_buffer_list_keeps_a_megabyte_buffer_s_path_under_file() {
    let td = tempfile::tempdir().expect("tempdir");
    let small = td.path().join("small.txt");
    let big = td.path().join("big.txt");
    write(&small, b"x\n");
    let line = format!("{}\n", "x".repeat(1023));
    write(&big, line.repeat(1024).as_bytes());
    assert_eq!(
        std::fs::metadata(&big).expect("stat").len(),
        1_048_576,
        "fixture: a megabyte"
    );
    let mut s = fresh();
    pin_home(&s, "/nonexistent-home");
    find_file(&mut s, &small);
    find_file(&mut s, &big);
    let listing = buffer_list(&mut s);
    assert_file_column_in_line(&listing, &[&small, &big]);
}

/// P4b. The `File` column for a name holding a multi-byte character:
/// `%-30s` pads by bytes, so `résumé.md` (nine characters, eleven bytes)
/// ends two columns short and its size and path with it.
#[test]
fn r1_the_buffer_list_keeps_a_non_ascii_name_s_path_under_file() {
    let td = tempfile::tempdir().expect("tempdir");
    let plain = td.path().join("plain.md");
    let accented = td.path().join("résumé.md");
    write(&plain, b"x\n");
    write(&accented, b"y\n");
    let mut s = fresh();
    pin_home(&s, "/nonexistent-home");
    find_file(&mut s, &plain);
    find_file(&mut s, &accented);
    let listing = buffer_list(&mut s);
    assert_file_column_in_line(&listing, &[&plain, &accented]);
}

#[path = "common/home.rs"]
mod home;
use home::pin_home;

#[path = "common/iso.rs"]
mod iso;
