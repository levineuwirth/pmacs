// tests/e10c_buffer_names_acceptance.rs --- E10c: a buffer's name is
// not its path (D40).

//! E10c's rows through the keys a user presses (`EditorState::dispatch_key`,
//! the TUI's path) and read where a user reads a name: the mode line of a
//! painted frame (`paint_frame`). A file buffer is named by its file's
//! basename; two that collide each take the fewest trailing directories
//! that tell them apart, written forward (`src/a.rs`, `tests/a.rs`), and
//! a survivor returns to its basename; an explicitly named buffer keeps
//! its name through a collision, and the derived name moves around it.
//! The names are recomputed on open, kill, rename and save-as. A dired
//! listing is named by its directory and a `/` through the same function
//! (`proj/`, `a/proj/`), and never shares a name with a file.
//! Where the name no longer shows a path, `*buffer-list*` shows it
//! beside the name.

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

/// Drive the async runtime until nothing is parked or pending: a dired
/// listing is read on a worker and lands on a later tick.
fn pump(s: &mut EditorState) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut spins = 0u32;
    loop {
        let idle: bool = eval(
            s,
            "return pmacs._async.parked_count() == 0 and pmacs._async.pending_count() == 0",
        );
        if idle {
            return;
        }
        assert!(Instant::now() < deadline, "async pump deadline exceeded");
        s.tick_async();
        spins += 1;
        if spins > 64 {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// `C-x d`, the prefill replaced by `dir`, RET: dired as a user opens it.
fn open_dired(s: &mut EditorState, dir: &Path) {
    ctrl(s, 'x');
    press(s, KeyCode::Char('d'));
    assert!(active(s), "C-x d opens the prompt");
    clear_field(s);
    type_str(s, &dir.display().to_string());
    press(s, KeyCode::Enter);
    pump(s);
    assert_eq!(
        eval::<String>(
            s,
            "return pmacs.buffer.major_mode(pmacs.window.buffer()) or ''"
        ),
        "dired",
        "fixture: C-x d opened a listing"
    );
}

/// E10c.3's witness: a listing is named by its directory and a `/`; two
/// `proj` directories show as `a/proj/` and `b/proj/`, through the
/// function file names use, and a kill returns the survivor to `proj/`.
/// Dired still finds each by its path: `C-x d` on a directory already
/// listed returns to its buffer.
#[test]
fn two_proj_directories_show_as_names_that_tell_them_apart() {
    let td = tempfile::tempdir().expect("tempdir");
    for dir in ["a/proj", "b/proj"] {
        std::fs::create_dir_all(td.path().join(dir)).expect("mkdir");
        std::fs::write(td.path().join(dir).join("f.txt"), b"x\n").expect("write");
    }
    let mut s = fresh();
    open_dired(&mut s, &td.path().join("a/proj"));
    assert_eq!(
        mode_line_name(&s),
        "proj/",
        "alone, a listing is its directory"
    );
    exec(&s, "_G.A = pmacs.window.buffer()");
    open_dired(&mut s, &td.path().join("b/proj"));
    assert_eq!(mode_line_name(&s), "b/proj/");
    assert!(
        names(&s).contains(&"a/proj/".to_owned()),
        "the first listing lengthened too: {:?}",
        names(&s)
    );
    open_dired(&mut s, &td.path().join("a/proj"));
    assert!(
        eval::<bool>(&s, "return pmacs.window.buffer() == _G.A"),
        "dired finds its buffer by its path, not by a name"
    );
    assert_eq!(mode_line_name(&s), "a/proj/");
    kill_this(&mut s);
    assert_eq!(status(&s), "kill-buffer: killed a/proj/");
    assert!(
        names(&s).contains(&"proj/".to_owned()),
        "the survivor returned to proj/: {:?}",
        names(&s)
    );
}

/// E10c.3's second witness: a listing of `proj` and a file named `proj`
/// never share a name, and each is reached by its own: `C-x b proj/ RET`
/// the listing, `C-x b proj RET` the file.
#[test]
fn a_dired_buffer_and_a_file_named_proj_never_share_a_name() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(td.path().join("x/proj")).expect("mkdir");
    std::fs::create_dir_all(td.path().join("y")).expect("mkdir");
    std::fs::write(td.path().join("y/proj"), b"a file\n").expect("write");
    let mut s = fresh();
    find_file(&mut s, &td.path().join("y/proj"));
    assert_eq!(mode_line_name(&s), "proj");
    open_dired(&mut s, &td.path().join("x/proj"));
    assert_eq!(mode_line_name(&s), "proj/");
    switch_to(&mut s, "proj");
    assert_eq!(
        mode_line_name(&s),
        "proj",
        "C-x b proj RET reaches the file"
    );
    switch_to(&mut s, "proj/");
    assert_eq!(
        mode_line_name(&s),
        "proj/",
        "C-x b proj/ RET reaches the listing"
    );
}

/// The census's syntax decision: a listing of a directory named like a
/// file (`x.rs`) is named `x.rs/`, which `Path::extension` reads as `rs`;
/// language detection reads the buffer's path, and a name ending in `/`
/// names no file, so the listing takes no grammar and no parse.
#[test]
fn a_listing_of_a_directory_named_like_a_file_attaches_no_grammar() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(td.path().join("x.rs")).expect("mkdir");
    std::fs::write(td.path().join("x.rs/main.rs"), b"fn main() {}\n").expect("write");
    let mut s = fresh();
    open_dired(&mut s, &td.path().join("x.rs"));
    assert_eq!(mode_line_name(&s), "x.rs/");
    exec(&s, "pmacs.window.switch_buffer(pmacs.window.buffer())");
    assert_eq!(
        eval::<Option<String>>(
            &s,
            "return pmacs.parse.buffer_language(pmacs.window.buffer())"
        ),
        None,
        "a listing has no language"
    );
    assert!(
        !eval::<bool>(&s, "return pmacs.parse._has_view(pmacs.window.buffer())"),
        "and no parse"
    );
}

/// The text of the buffer named `name`.
fn text_of(s: &EditorState, name: &str) -> String {
    eval(
        s,
        &format!(
            "for _, id in ipairs(pmacs.buffer.list()) do\n\
               if pmacs.describe.buffer(id).name == {name:?} then return id:slice(0, id:len()) end\n\
             end\n\
             error('no buffer ' .. {name:?})"
        ),
    )
}

/// E10c.5's witness: a name no longer shows the path, so `C-x C-b`'s
/// `*buffer-list*` shows it beside the name: the path of a file buffer
/// named by its basename, the directory of a listing, and none for a
/// buffer that holds neither, whose row ends at its size. HOME is set
/// where no temporary directory sits, so the paths show whole.
#[test]
fn the_buffer_list_shows_the_path_of_a_buffer_named_by_its_basename() {
    let td = project();
    let mut s = fresh();
    pin_home(&s, "/nonexistent-home");
    find_file(&mut s, &td.path().join("src/b.rs"));
    assert_eq!(mode_line_name(&s), "b.rs");
    open_dired(&mut s, &td.path().join("tests"));
    assert_eq!(mode_line_name(&s), "tests/");
    ctrl(&mut s, 'x');
    ctrl(&mut s, 'b');
    let listing = text_of(&s, "*buffer-list*");
    let row = |name: &str| {
        listing
            .lines()
            .find(|l| {
                l.get(3..)
                    .is_some_and(|rest| rest.starts_with(&format!("{name} ")))
            })
            .unwrap_or_else(|| panic!("no row for {name} in {listing}"))
            .to_owned()
    };
    let b = td.path().join("src/b.rs").display().to_string();
    assert!(
        row("b.rs").trim_end().ends_with(&b),
        "b.rs's row shows its path: {listing}"
    );
    let tests = td.path().join("tests").display().to_string();
    assert!(
        row("tests/").trim_end().ends_with(&tests),
        "the listing's row shows its directory: {listing}"
    );
    assert!(
        row("*scratch*").trim_end().ends_with(" 0"),
        "*scratch* holds no path, and its row ends at its size: {listing}"
    );
    assert!(
        listing.lines().next().is_some_and(|h| h.contains("File")),
        "the header names the column: {listing}"
    );
}

/// E10c.5 as the owner amended it (2026-10-25, on review 1's Low 1):
/// `*buffer-list*` writes a path under `$HOME` in `~` form; its name
/// column is as wide as the longest name up to forty columns, a longer
/// name cut with `…`; and its size is a bare number right-aligned in a
/// column as wide as the largest, the header's `Size` above it. Each row
/// is read whole, so a column out of line fails it.
#[test]
fn the_buffer_list_writes_tilde_paths_cuts_a_long_name_and_aligns_its_sizes() {
    let td = tempfile::tempdir().expect("tempdir");
    let small = td.path().join("src/small.rs");
    let big = td.path().join("big.txt");
    let long_name = format!("{}.md", "n".repeat(50));
    let long = td.path().join(&long_name);
    for (path, bytes) in [
        (&small, b"x".to_vec()),
        (&big, vec![b'y'; 1_048_576]),
        (&long, b"z\n".to_vec()),
    ] {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }
    let mut s = fresh();
    pin_home(&s, &td.path().display().to_string());
    for path in [&small, &big, &long] {
        find_file(&mut s, path);
    }
    ctrl(&mut s, 'x');
    ctrl(&mut s, 'b');
    let listing = text_of(&s, "*buffer-list*");
    let cut = format!("{}…", "n".repeat(39));
    for expected in [
        format!(
            "   {:<40} {:>7} File   RET visit  d/u mark  x kill marked  k kill  g refresh  q quit",
            "Buffer", "Size"
        ),
        format!("   {:<40} {:>7}", "*scratch*", 0),
        format!("   {:<40} {:>7} ~/src/small.rs", "small.rs", 1),
        format!("   {:<40} {:>7} ~/big.txt", "big.txt", 1_048_576),
        format!("   {cut} {:>7} ~/{long_name}", 2),
    ] {
        assert!(
            listing.lines().any(|l| l == expected),
            "no row {expected:?} in:\n{listing}"
        );
    }
    assert!(
        !listing.contains(&td.path().display().to_string()),
        "every path in ~ form:\n{listing}"
    );
}

/// Open `paths` in one fresh session through `pmacs.buffer.find_or_open`
/// in one Lua loop, as E10c review 1's measurement did, and say how long
/// the loop took.
fn time_opening(paths: &[std::path::PathBuf]) -> Duration {
    let s = fresh();
    let before: usize = eval(&s, "return #pmacs.buffer.list()");
    let list = paths
        .iter()
        .map(|p| format!("{:?}", p.display().to_string()))
        .collect::<Vec<_>>()
        .join(",");
    exec(&s, &format!("_G.E10C_PATHS = {{ {list} }}"));
    let t0 = Instant::now();
    exec(
        &s,
        "for _, p in ipairs(_G.E10C_PATHS) do pmacs.buffer.find_or_open(p) end",
    );
    let elapsed = t0.elapsed();
    let after: usize = eval(&s, "return #pmacs.buffer.list()");
    assert_eq!(after - before, paths.len(), "fixture: every file opened");
    elapsed
}

/// The budget for opening 2,000 files in one session (D12), by profile:
/// see [`opening_two_thousand_files_stays_within_budget`].
const OPEN_2000_BUDGET: Duration = if cfg!(debug_assertions) {
    Duration::from_secs(10)
} else {
    Duration::from_millis(2500)
};

/// E10c review 1's Medium 1, as a wall-clock budget (D12): 2,000 files
/// opened in one session, once with 2,000 basenames and once with one
/// (`d<i>/mod.txt`, each name growing one directory), no grammar and no
/// language server, as the review's measurement opened them. At
/// `0adb83e` every open renamed every buffer three times, so the opens
/// were quadratic in the session: the review measured 22 to 29 s in
/// release and 118 to 132 s in debug, against 0.4 to 0.5 s and 2.1 s at
/// `1eea479`. Now an open names its buffer once, and an event renames
/// only the paths whose names it shares. The bound is `1eea479`'s figure
/// on the laptop five times over, rounded up, for a run on a loaded or
/// slower machine: about 2 s a shape in debug (1,845 and 1,930 ms again at
/// fix round 1), so 10 s, and about 0.5 s in release (386 and 493 ms), so
/// 2.5 s. At fix round 1, on a quieter machine than the review's,
/// `0adb83e` took 7.2 to 11.3 s in release and 42 to 47 s in debug.
#[test]
#[ignore = "wall-clock budget; runs under --ignored in the perf jobs and scripts/gate --perf"]
fn opening_two_thousand_files_stays_within_budget() {
    let td = tempfile::tempdir().expect("tempdir");
    let make = |rel: String| {
        let path = td.path().join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, b"x\n").expect("write");
        path
    };
    let unique: Vec<_> = (0..2000).map(|i| make(format!("f{i}.txt"))).collect();
    let alike: Vec<_> = (0..2000).map(|i| make(format!("d{i}/mod.txt"))).collect();
    let unique_took = time_opening(&unique);
    let alike_took = time_opening(&alike);
    eprintln!(
        "e10c: opening 2,000 files took {unique_took:?} with 2,000 basenames \
         and {alike_took:?} with one (budget {OPEN_2000_BUDGET:?} each)"
    );
    assert!(
        unique_took < OPEN_2000_BUDGET && alike_took < OPEN_2000_BUDGET,
        "opening 2,000 files must stay within {OPEN_2000_BUDGET:?}; \
         took {unique_took:?} and {alike_took:?}"
    );
}

#[path = "common/home.rs"]
mod home;
use home::pin_home;

#[path = "common/iso.rs"]
mod iso;
