// tests/dired_acceptance.rs --- dired arc Stage 1 acceptance.

//! Acceptance for the dired view (`docs/dired-framing.md` §14 items
//! 1-16, Q#DR2-DR10). Item 17 --- "the fixture still passes" --- is a
//! gate item rather than a test here: `m8_1`/`m8_2`/`m8_3` prove the
//! `read_dir` opt is additive by continuing to pass unchanged.
//!
//! Discipline, following the Stage 0 suite:
//!
//! * every in-buffer claim is driven by a **real key** through
//!   `dispatch_key`, so a dead mode-keymap entry cannot pass vacuously;
//! * `pmacs.dired.open` is called directly only where a test needs an
//!   opt the interactive command does not carry (`display = "panel"`),
//!   and it is the documented public entry point in those cases;
//! * every listing is async, so each dispatch is followed by `pump`,
//!   which drives `tick_async` until the coroutine and its worker job
//!   have both settled.
//!
//! Fixtures use `.txt` files and empty `pmacs.lsp.config`, so no
//! `buffer.after-load` hook spawns a language server. Note the suite
//! asserts nothing about LSP, so the wipe cannot make an assertion
//! vacuous (the Lean 4 round-1 trap).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::cell::{CellGrid, CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::editor_core::normalize_buffer_path;
use pmacs::protocol::FrontendId;
use pmacs::window::WindowId;
use tempfile::TempDir;

const ROWS: u32 = 24;
const COLS: u32 = 100;

// ---------------------------------------------------------------------------
// Harness
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

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn type_char(s: &mut EditorState, c: char) {
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::NONE));
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        type_char(s, ch);
    }
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

/// A fresh editor with a declared frame geometry (a grid frontend's real
/// frame size *is* its geometry declaration, and the panel tests need
/// one before any side window can be placed).
fn editor() -> EditorState {
    let s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(ROWS, COLS));
    s
}

/// An editor whose active buffer is a real file inside `dir`, so the
/// `C-x d` prompt prefills with that directory and `C-x C-j` has a file
/// to jump from.
fn editor_in(dir: &Path) -> (EditorState, PathBuf) {
    let anchor = dir.join("anchor.txt");
    std::fs::write(&anchor, b"anchor\n").expect("write anchor");
    let s = editor();
    let anchor_str = anchor.display().to_string();
    exec(&s, &format!("pmacs.buffer.find_or_open({anchor_str:?})"));
    (s, anchor)
}

/// Drive the async runtime until no coroutine is parked and no worker
/// job is pending. Every dired command dispatches `read_dir` on a
/// worker and resumes on a later tick, so nothing dired does is
/// observable until this returns.
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

/// The canonical form of `path` — the core's own normalizer, which is
/// exactly what `pmacs.path.canonicalize` calls.
fn canon(path: &Path) -> String {
    normalize_buffer_path(path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn active_text(s: &EditorState) -> String {
    eval(
        s,
        "local b = pmacs.window.buffer()\nreturn b:slice(0, b:len())",
    )
}

fn active_lines(s: &EditorState) -> Vec<String> {
    active_text(s).lines().map(str::to_owned).collect()
}

fn active_name(s: &EditorState) -> String {
    eval(
        s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    )
}

fn active_path(s: &EditorState) -> Option<String> {
    eval(
        s,
        "local b = pmacs.window.buffer()\n\
         if b == nil then return nil end\n\
         local ok, p = pcall(function() return b:path() end)\n\
         if ok then return p end\n\
         return nil",
    )
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

fn buffer_names(s: &EditorState) -> Vec<String> {
    eval(
        s,
        "local out = {}\n\
         for _, id in ipairs(pmacs.buffer.list()) do\n\
           out[#out + 1] = pmacs.describe.buffer(id).name\n\
         end\n\
         return out",
    )
}

fn dired_buffer_names(s: &EditorState) -> Vec<String> {
    let mut names: Vec<String> = buffer_names(s)
        .into_iter()
        .filter(|n| n.starts_with("*dired:"))
        .collect();
    names.sort();
    names
}

/// One layout offset from dired's own constants, so column assertions
/// cannot drift from the module that computes them.
fn layout(s: &EditorState, field: &str) -> usize {
    let value: i64 = eval(s, &format!("return pmacs.dired._layout.{field}"));
    usize::try_from(value).expect("layout offsets are non-negative")
}

/// The rendered name column of one listing line.
fn line_name(s: &EditorState, line: &str) -> String {
    let start = layout(s, "NAME_START");
    line.get(start..).unwrap_or("").to_owned()
}

/// The 0-based line the entry named `name` renders on.
fn line_of(s: &EditorState, name: &str) -> usize {
    let lines = active_lines(s);
    for (index, line) in lines.iter().enumerate().skip(1) {
        let rendered = line_name(s, line);
        if rendered == name || rendered.starts_with(&format!("{name} -> ")) {
            return index;
        }
    }
    panic!("no listing line for {name:?} in {lines:#?}");
}

/// Seat the cursor on `name`'s line. Test scaffolding: the *keys* that
/// move by line are exercised separately (acceptance 6).
fn seat_on(s: &EditorState, name: &str) {
    let line = line_of(s, name);
    exec(s, &format!("pmacs.editor.move_to_line({line})"));
}

fn cursor_line(s: &EditorState) -> usize {
    let value: i64 = eval(s, "return pmacs.editor.cursor_line()");
    usize::try_from(value).expect("cursor lines are non-negative")
}

/// The entry name under the cursor, or `None` on the header/footer.
fn cursor_entry(s: &EditorState) -> Option<String> {
    let line = cursor_line(s);
    if line == 0 {
        return None;
    }
    let lines = active_lines(s);
    lines.get(line).map(|text| line_name(s, text))
}

/// Open `path` through the public entry point, pumping to settle.
/// Returns the raised message, if it raised.
fn open_dired(s: &mut EditorState, path: &str, opts: &str) -> Option<String> {
    exec(
        s,
        &format!(
            "_G.DIRED_ERR = nil\n\
             pmacs.async(function()\n\
               local ok, err = pcall(pmacs.dired.open, {path:?}, {opts})\n\
               if not ok then\n\
                 _G.DIRED_ERR = type(err) == 'table' and tostring(err.message) or tostring(err)\n\
               end\n\
             end)"
        ),
    );
    pump(s);
    eval(s, "return _G.DIRED_ERR")
}

fn open_ok(s: &mut EditorState, path: &Path, opts: &str) {
    let raised = open_dired(s, &path.display().to_string(), opts);
    assert!(
        raised.is_none(),
        "dired.open must succeed; raised {raised:?}"
    );
}

fn side_window(s: &EditorState) -> Option<WindowId> {
    s.core.borrow().side_window_for(FrontendId::LOCAL)
}

fn window_buffer_name(s: &EditorState, window: WindowId) -> String {
    let buffer_id = s
        .core
        .borrow()
        .windows
        .get(&window)
        .map(|w| w.buffer_id)
        .expect("window is live");
    let registry = s.lua_host.registry().borrow();
    registry
        .get(buffer_id)
        .expect("buffer is live")
        .name()
        .to_owned()
}

fn active_window(s: &EditorState) -> WindowId {
    s.core.borrow().active_window_id()
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
                .trim_end()
                .to_owned()
        })
        .collect()
}

/// `a.txt` (5 bytes), `b.txt` (6 bytes), `subdir/`, and `link ->
/// a.txt`.
fn fixture_dir() -> TempDir {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::write(td.path().join("a.txt"), b"hello").expect("write a");
    std::fs::write(td.path().join("b.txt"), b"world!").expect("write b");
    std::fs::create_dir(td.path().join("subdir")).expect("mkdir");
    std::fs::write(td.path().join("subdir").join("inner.txt"), b"deep\n").expect("write inner");
    std::os::unix::fs::symlink("a.txt", td.path().join("link")).expect("symlink");
    td
}

// ---------------------------------------------------------------------------
// 1 --- listing shape
// ---------------------------------------------------------------------------

/// Header line plus one line per entry, with kind char, perms, size,
/// mtime, and name; a symlink renders `l` with ` -> target`; the entry
/// count matches `read_dir`. Driven through the real `C-x d`, accepting
/// the prefilled directory.
#[test]
fn dired_renders_a_header_and_one_line_per_entry() {
    let td = fixture_dir();
    let (mut s, _anchor) = editor_in(td.path());

    ctrl(&mut s, 'x');
    type_char(&mut s, 'd');
    assert!(
        eval::<bool>(&s, "return pmacs.minibuffer.is_active()"),
        "C-x d must open a prompt"
    );
    assert_eq!(
        eval::<String>(&s, "return pmacs.minibuffer.contents()"),
        canon(td.path()),
        "the prompt prefills with the current buffer's directory, so RET \
         opens where you are"
    );
    press(&mut s, KeyCode::Enter);
    pump(&mut s);

    let lines = active_lines(&s);
    assert_eq!(
        lines[0],
        format!("{}:", canon(td.path())),
        "line 0 is the header"
    );
    let on_disk = std::fs::read_dir(td.path()).expect("read_dir").count();
    assert_eq!(
        lines.len() - 1,
        on_disk,
        "one line per entry, no footer on a clean listing: {lines:#?}"
    );

    let kind_start = layout(&s, "KIND_START");
    let perms_start = layout(&s, "PERMS_START");
    let perms_end = layout(&s, "PERMS_END");
    let size_start = layout(&s, "SIZE_START");

    let a = &lines[line_of(&s, "a.txt")];
    assert_eq!(&a[kind_start..=kind_start], "-", "a regular file: {a:?}");
    let perms = &a[perms_start..perms_end];
    assert_eq!(perms.len(), 9, "nine permission characters: {perms:?}");
    assert!(
        perms.starts_with("rw"),
        "owner may read and write a file we just wrote: {perms:?}"
    );
    assert_eq!(
        a[size_start..size_start + 10].trim(),
        "5",
        "the size column carries a.txt's five bytes: {a:?}"
    );
    assert!(
        a[perms_end..size_start].chars().all(char::is_whitespace),
        "columns are space-separated: {a:?}"
    );

    let sub = &lines[line_of(&s, "subdir")];
    assert_eq!(&sub[kind_start..=kind_start], "d", "a directory: {sub:?}");

    let link = &lines[line_of(&s, "link")];
    assert_eq!(&link[kind_start..=kind_start], "l", "a symlink: {link:?}");
    assert_eq!(
        line_name(&s, link),
        "link -> a.txt",
        "a symlink shows its target"
    );

    // The mark column is reserved and blank in Stage 1 (Q#DR4): filling
    // it in is Stage 2's job, and reserving it now is what keeps Stage
    // 2 from moving every column right of it.
    let mark_start = layout(&s, "MARK_START");
    for line in &lines[1..] {
        assert_eq!(
            &line[mark_start..kind_start],
            "  ",
            "the mark column renders blank: {line:?}"
        );
    }

    let mtime_start = layout(&s, "MTIME_START");
    let name_start = layout(&s, "NAME_START");
    let stamp = &a[mtime_start..name_start - 1];
    assert_eq!(stamp.len(), 16, "fixed-width mtime: {stamp:?}");
    assert!(
        stamp.starts_with("20") && stamp.contains('-') && stamp.contains(':'),
        "an ISO-ish minute-precision timestamp: {stamp:?}"
    );
}

// ---------------------------------------------------------------------------
// 2 --- visit dispatches on kind, through the panel-safe primitive
// ---------------------------------------------------------------------------

/// `RET` on a directory descends; on a file it opens the file; on the
/// header it does nothing.
#[test]
fn dired_visit_dispatches_on_entry_kind() {
    let td = fixture_dir();
    let mut s = editor();
    open_ok(&mut s, td.path(), "nil");

    // Header: no entry, so nothing happens.
    exec(&s, "pmacs.editor.move_to_line(0)");
    let before = active_name(&s);
    press(&mut s, KeyCode::Enter);
    pump(&mut s);
    assert_eq!(
        active_name(&s),
        before,
        "RET on the header must not visit anything"
    );

    // Directory: descend into its own dired buffer.
    seat_on(&s, "subdir");
    press(&mut s, KeyCode::Enter);
    pump(&mut s);
    assert_eq!(
        active_name(&s),
        format!("*dired:{}*", canon(&td.path().join("subdir"))),
        "RET on a directory opens that directory's dired buffer"
    );
    assert_eq!(
        line_name(&s, &active_lines(&s)[1]),
        "inner.txt",
        "the descended listing is the subdirectory's"
    );

    // File: the fixture's "requires the buffer-from-file API" error is
    // gone --- `f` is the same command as RET.
    seat_on(&s, "inner.txt");
    type_char(&mut s, 'f');
    pump(&mut s);
    assert_eq!(
        active_path(&s).map(PathBuf::from),
        Some(PathBuf::from(canon(
            &td.path().join("subdir").join("inner.txt")
        ))),
        "RET/f on a file opens the file bound to its path"
    );
    assert_eq!(
        eval::<String>(&s, "return pmacs.window.buffer():slice(0, 4)"),
        "deep",
        "the file's real contents load"
    );
}

/// The panel case, which is the real assertion (Q#DR10): with dired
/// displayed as a panel, `RET` on a file leaves the dired panel alive
/// and puts the file in the document window. Falsified by swapping
/// `display_file` for `find_or_open`, which switches the active window
/// in both branches before firing hooks --- the panel swallows itself.
#[test]
fn dired_visit_from_a_panel_keeps_the_panel_and_uses_the_document_window() {
    let td = fixture_dir();
    let (mut s, anchor) = editor_in(td.path());
    let document = active_window(&s);
    open_ok(&mut s, td.path(), r#"{ display = "panel" }"#);

    let panel = side_window(&s).expect("display = panel must create a side window");
    assert_eq!(active_window(&s), panel, "the panel is selected");
    assert_eq!(
        window_buffer_name(&s, panel),
        format!("*dired:{}*", canon(td.path())),
        "the panel shows dired"
    );

    seat_on(&s, "a.txt");
    press(&mut s, KeyCode::Enter);
    pump(&mut s);

    let panel_after = side_window(&s).expect("the dired panel must survive a file visit");
    assert_eq!(panel_after, panel, "the same side window, not a new one");
    assert_eq!(
        window_buffer_name(&s, panel_after),
        format!("*dired:{}*", canon(td.path())),
        "the panel still shows dired"
    );
    assert_eq!(
        window_buffer_name(&s, document),
        canon(&td.path().join("a.txt")),
        "the visited file lands in the document window"
    );
    assert_eq!(
        active_path(&s).map(PathBuf::from),
        Some(PathBuf::from(canon(&td.path().join("a.txt")))),
        "and it is what the visit selected"
    );
    assert!(
        anchor.exists(),
        "fixture sanity: the anchor file was never touched"
    );
}

// ---------------------------------------------------------------------------
// 3 --- one buffer per directory, canonicalized
// ---------------------------------------------------------------------------

/// Descending twice then ascending twice yields the *same* buffers as
/// the first visit, and every dired buffer's name describes the
/// directory it displays.
#[test]
fn dired_navigation_reuses_one_buffer_per_directory() {
    let td = tempfile::tempdir().expect("tempdir");
    let deep = td.path().join("one").join("two");
    std::fs::create_dir_all(&deep).expect("mkdir -p");
    std::fs::write(deep.join("leaf.txt"), b"leaf\n").expect("write leaf");

    let mut s = editor();
    open_ok(&mut s, td.path(), "nil");
    exec(&s, "_G.ROOT = pmacs.window.buffer()");

    seat_on(&s, "one");
    press(&mut s, KeyCode::Enter);
    pump(&mut s);
    exec(&s, "_G.ONE = pmacs.window.buffer()");
    seat_on(&s, "two");
    press(&mut s, KeyCode::Enter);
    pump(&mut s);
    exec(&s, "_G.TWO = pmacs.window.buffer()");
    assert_eq!(
        active_name(&s),
        format!("*dired:{}*", canon(&deep)),
        "each buffer's name describes the directory it displays"
    );

    // Back up, with `^`.
    type_char(&mut s, '^');
    pump(&mut s);
    assert!(
        eval::<bool>(&s, "return pmacs.window.buffer() == _G.ONE"),
        "ascending returns to the SAME buffer, not a fresh one"
    );
    assert_eq!(
        cursor_entry(&s).as_deref(),
        Some("two"),
        "`^` seats the cursor on the directory it came from"
    );
    type_char(&mut s, '^');
    pump(&mut s);
    assert!(
        eval::<bool>(&s, "return pmacs.window.buffer() == _G.ROOT"),
        "and again at the next level up"
    );

    // Down again: still the same two buffers.
    seat_on(&s, "one");
    press(&mut s, KeyCode::Enter);
    pump(&mut s);
    assert!(eval::<bool>(&s, "return pmacs.window.buffer() == _G.ONE"));
    seat_on(&s, "two");
    press(&mut s, KeyCode::Enter);
    pump(&mut s);
    assert!(eval::<bool>(&s, "return pmacs.window.buffer() == _G.TWO"));
    assert_eq!(
        dired_buffer_names(&s).len(),
        3,
        "three directories visited, three dired buffers: {:?}",
        dired_buffer_names(&s)
    );
}

/// Three spellings of one directory yield ONE buffer, because names and
/// lookups both go through the canonical form (Q#DR2).
#[test]
fn dired_canonicalizes_before_naming_and_lookup() {
    let td = fixture_dir();
    let base = td.path().display().to_string();
    let name = td
        .path()
        .file_name()
        .expect("tempdir has a basename")
        .to_string_lossy()
        .into_owned();

    let mut s = editor();
    open_ok(&mut s, td.path(), "nil");
    let raised = open_dired(&mut s, &format!("{base}/"), "nil");
    assert!(raised.is_none(), "trailing slash must open: {raised:?}");
    let raised = open_dired(&mut s, &format!("{base}/../{name}"), "nil");
    assert!(raised.is_none(), "a `..` round trip must open: {raised:?}");

    assert_eq!(
        dired_buffer_names(&s),
        vec![format!("*dired:{}*", canon(td.path()))],
        "three spellings, one buffer"
    );
}

/// `dired.kill-when-opening` (Emacs 28's opt-out): the departed buffer
/// is gone after a descent.
#[test]
fn dired_kill_when_opening_kills_the_departed_buffer() {
    let td = fixture_dir();
    let mut s = editor();
    exec(&s, "pmacs.config.set('dired.kill-when-opening', true)");
    open_ok(&mut s, td.path(), "nil");
    assert_eq!(dired_buffer_names(&s).len(), 1);

    seat_on(&s, "subdir");
    press(&mut s, KeyCode::Enter);
    pump(&mut s);

    assert_eq!(
        dired_buffer_names(&s),
        vec![format!("*dired:{}*", canon(&td.path().join("subdir")))],
        "descending killed the buffer it left"
    );
    // And the setting is what did it: the default keeps both.
    exec(&s, "pmacs.config.set('dired.kill-when-opening', false)");
    type_char(&mut s, '^');
    pump(&mut s);
    assert_eq!(
        dired_buffer_names(&s).len(),
        2,
        "with the setting off, the departed buffer survives: {:?}",
        dired_buffer_names(&s)
    );
}

// ---------------------------------------------------------------------------
// 3b --- canonicalization parity
// ---------------------------------------------------------------------------

/// The Lua canonicalizer and the core normalizer agree on every edge in
/// one shared list --- because they are the *same function*
/// (`pmacs.path.canonicalize` is `normalize_buffer_path`). Stage 1
/// deliberately did not mirror the normalizer in Lua: a second
/// implementation that disagreed on `//tmp` or a `..` at root would
/// mint two buffers for one directory with no error anywhere, and the
/// mirror would then owe Stage 2 a removal.
#[test]
fn dired_canonicalization_is_the_cores_own_normalizer() {
    let s = editor();
    let cases = [
        "//tmp",
        "/tmp/",
        "/tmp/../tmp",
        "/tmp/./x/../y",
        "/../..",
        "/",
        ".",
        "relative/path",
        "~",
        "~/inside",
        "~notauser/x",
    ];
    for case in cases {
        let from_lua: String = eval(&s, &format!("return pmacs.path.canonicalize({case:?})"));
        let from_rust = normalize_buffer_path(PathBuf::from(case))
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            from_lua, from_rust,
            "canonicalization must not fork for {case:?}"
        );
    }

    // And the form dired names buffers with is that same form.
    let td = fixture_dir();
    let mut s = s;
    open_ok(&mut s, td.path(), "nil");
    assert_eq!(active_name(&s), format!("*dired:{}*", canon(td.path())));
}

// ---------------------------------------------------------------------------
// 3c --- panel descent
// ---------------------------------------------------------------------------

/// A directory descent in a panel-displayed dired stays in the *same*
/// side window (Q#DR10): the next directory is the same kind of thing as
/// the current one and belongs in the same slot. Neither replaced by a
/// document window nor duplicated.
#[test]
fn dired_directory_descent_stays_in_its_side_window() {
    let td = fixture_dir();
    let (mut s, anchor) = editor_in(td.path());
    let document = active_window(&s);
    open_ok(&mut s, td.path(), r#"{ display = "panel" }"#);
    let panel = side_window(&s).expect("a side window");

    seat_on(&s, "subdir");
    press(&mut s, KeyCode::Enter);
    pump(&mut s);

    assert_eq!(
        side_window(&s),
        Some(panel),
        "the same side window, not a second one"
    );
    assert_eq!(
        window_buffer_name(&s, panel),
        format!("*dired:{}*", canon(&td.path().join("subdir"))),
        "showing the new directory"
    );
    assert_eq!(
        window_buffer_name(&s, document),
        canon(&anchor),
        "the document window is untouched"
    );
    assert_eq!(active_window(&s), panel, "and dired keeps the focus");
}

/// A **dedicated** panel is a different story, and the framing's R2-3
/// expectation ("the new dired buffer inherits the dedication") is
/// falsified by the substrate: `display_buffer` never replaces the
/// buffer in a slot dedicated to another one --- it discards every
/// side-specific parameter and falls back to the document window
/// (Q#BP3 2.iii). Dired does not try to unpin the user's panel, so the
/// pin holds and the new directory appears in the document area, which
/// is also what Emacs's `display-buffer` does with a dedicated window.
#[test]
fn dired_descent_from_a_dedicated_panel_leaves_the_pin_alone() {
    let td = fixture_dir();
    let (mut s, _anchor) = editor_in(td.path());
    let document = active_window(&s);
    open_ok(&mut s, td.path(), r#"{ display = "panel" }"#);
    let panel = side_window(&s).expect("a side window");
    exec(
        &s,
        &format!(
            "pmacs.window.set_params({}, {{ dedicated = true }})",
            panel.raw()
        ),
    );

    seat_on(&s, "subdir");
    press(&mut s, KeyCode::Enter);
    pump(&mut s);

    assert_eq!(
        side_window(&s),
        Some(panel),
        "no second side window is created"
    );
    assert_eq!(
        window_buffer_name(&s, panel),
        format!("*dired:{}*", canon(td.path())),
        "the dedicated slot keeps the buffer it was pinned to"
    );
    assert!(
        eval::<bool>(
            &s,
            &format!("return pmacs.window.params({}).dedicated", panel.raw())
        ),
        "and it is still dedicated afterward"
    );
    assert_eq!(
        window_buffer_name(&s, document),
        format!("*dired:{}*", canon(&td.path().join("subdir"))),
        "the new directory falls back to the document window"
    );
}

// ---------------------------------------------------------------------------
// 4 --- ownership check
// ---------------------------------------------------------------------------

/// A foreign buffer that merely *has* dired's name is not adopted (F7):
/// `pmacs.buffer.create` takes any caller-chosen name, and dired paints
/// with `bypass_intercept`, so adopting one would silently clobber a
/// user's data.
#[test]
fn dired_does_not_adopt_a_foreign_buffer_with_its_name() {
    let td = fixture_dir();
    let mut s = editor();
    let name = format!("*dired:{}*", canon(td.path()));
    exec(
        &s,
        &format!(
            "local b = pmacs.buffer.create({name:?})\n\
             b:insert(0, 'FOREIGN CONTENTS')\n\
             _G.FOREIGN = b"
        ),
    );
    // Even with the major mode set, which is the weaker ownership test
    // the framing floated: the handle table is the authority.
    exec(&s, "pmacs.buffer.set_major_mode(_G.FOREIGN, 'dired')");

    open_ok(&mut s, td.path(), "nil");

    assert_eq!(
        eval::<String>(&s, "return _G.FOREIGN:slice(0, _G.FOREIGN:len())"),
        "FOREIGN CONTENTS",
        "the foreign buffer's contents must be byte-identical"
    );
    assert!(
        !eval::<bool>(&s, "return pmacs.window.buffer() == _G.FOREIGN"),
        "dired must not display the foreign buffer"
    );
    assert_eq!(
        active_name(&s),
        format!("{name}<2>"),
        "dired opens under a disambiguated name instead"
    );
    assert!(
        active_lines(&s)[0].ends_with(':'),
        "and it is a real listing: {:?}",
        active_lines(&s)[0]
    );
}

// ---------------------------------------------------------------------------
// 5 --- read-only discipline
// ---------------------------------------------------------------------------

/// An ordinary self-insert is rejected by the intercept and leaves the
/// text byte-identical, while dired's own repaint succeeds through
/// `bypass_intercept`. `set_round_trip_input` is pinned through the
/// **production** seam a semantic frontend reads (`dispatch_idle_for`,
/// published as `DispatchIdle`) rather than by a direct-call assertion:
/// without it, a GPU session would optimistically apply `g` as an
/// insert instead of letting it reach the revert binding.
#[test]
fn dired_buffer_is_read_only_and_round_trips_input() {
    let td = fixture_dir();
    let mut s = editor();
    open_ok(&mut s, td.path(), "nil");
    let before = active_text(&s);

    // A document window, deliberately: the panel arm of the same gate
    // (`!window.is_side()`) would otherwise be what makes this pass.
    assert!(
        !s.core
            .borrow()
            .windows
            .get(&active_window(&s))
            .expect("live window")
            .is_side(),
        "fixture premise: dired is in a document window here"
    );
    assert!(
        !s.dispatch_idle_for(FrontendId::LOCAL),
        "a round-trip buffer must turn optimistic apply OFF"
    );

    // `z` is bound nowhere in dired mode, so it reaches self-insert.
    type_char(&mut s, 'z');
    assert_eq!(
        active_text(&s),
        before,
        "the read-only intercept must reject a self-insert"
    );
    assert!(
        status(&s).contains("read-only"),
        "and say so; got {:?}",
        status(&s)
    );

    // Dired's own writes still land: revert repaints the whole buffer.
    std::fs::write(td.path().join("c.txt"), b"new\n").expect("write c");
    type_char(&mut s, 'g');
    pump(&mut s);
    assert!(
        active_text(&s).contains("c.txt"),
        "dired's own repaint bypasses the intercept: {:?}",
        active_text(&s)
    );
}

// ---------------------------------------------------------------------------
// 6 --- mode keymap
// ---------------------------------------------------------------------------

/// The keys resolve through `scope = "mode"` with no per-buffer
/// binding: a *second* dired buffer, created by a descent that calls no
/// `keymap.bind` of its own, still responds to `n`, `g`, and `^`. The
/// mode also shows in the statusline, through a real painted frame.
#[test]
fn dired_keys_resolve_through_the_mode_keymap() {
    let td = fixture_dir();
    let mut s = editor();
    open_ok(&mut s, td.path(), "nil");
    seat_on(&s, "subdir");
    press(&mut s, KeyCode::Enter);
    pump(&mut s);

    assert_eq!(
        eval::<Option<String>>(&s, "return pmacs.buffer.major_mode(pmacs.window.buffer())"),
        Some("dired".to_owned()),
        "the descended buffer carries the mode"
    );
    assert_eq!(
        eval::<i64>(
            &s,
            "local n = 0\n\
             for _, entry in ipairs(pmacs.keymap.list()) do\n\
               if entry.scope:find('buffer') then n = n + 1 end\n\
             end\n\
             return n"
        ),
        0,
        "and no buffer-scoped binding exists anywhere"
    );

    // `n` moves by line through the mode binding.
    exec(&s, "pmacs.editor.move_to_line(0)");
    type_char(&mut s, 'n');
    assert_eq!(cursor_line(&s), 1, "`n` moves down one line");

    // `g` reverts: a file added externally appears.
    std::fs::write(td.path().join("subdir").join("second.txt"), b"x\n").expect("write second");
    type_char(&mut s, 'g');
    pump(&mut s);
    assert!(
        active_text(&s).contains("second.txt"),
        "`g` re-read the directory: {:?}",
        active_text(&s)
    );

    // `^` ascends.
    type_char(&mut s, '^');
    pump(&mut s);
    assert_eq!(
        active_name(&s),
        format!("*dired:{}*", canon(td.path())),
        "`^` ascends from the second buffer too"
    );

    let rows = painted_rows(&s);
    let mode_line = rows
        .iter()
        .rev()
        .find(|row| row.contains("dired"))
        .unwrap_or_else(|| panic!("no painted row mentions the mode: {rows:#?}"));
    assert!(
        mode_line.contains("dired"),
        "the major mode shows in the statusline: {mode_line:?}"
    );
}

// ---------------------------------------------------------------------------
// 7 --- cursor preservation
// ---------------------------------------------------------------------------

/// The cursor is re-seated by BASENAME across a repaint (Q#DR9), and
/// falls back to the nearest surviving line when the entry is gone.
/// Every repaint is wholesale, so a dired that dropped to line 0 after
/// each revert would be unusable.
#[test]
fn dired_revert_reseats_the_cursor_by_basename() {
    let td = tempfile::tempdir().expect("tempdir");
    for name in ["c.txt", "d.txt", "e.txt"] {
        std::fs::write(td.path().join(name), b"x").expect("write");
    }
    let mut s = editor();
    open_ok(&mut s, td.path(), "nil");
    seat_on(&s, "d.txt");
    let line_before = cursor_line(&s);

    // Two files that sort BEFORE it, so its line index has to change.
    std::fs::write(td.path().join("a.txt"), b"x").expect("write a");
    std::fs::write(td.path().join("b.txt"), b"x").expect("write b");
    type_char(&mut s, 'g');
    pump(&mut s);

    assert_ne!(
        cursor_line(&s),
        line_before,
        "fixture premise: the line index moved"
    );
    assert_eq!(
        cursor_entry(&s).as_deref(),
        Some("d.txt"),
        "the cursor follows the basename, not the line"
    );

    // Now the entry disappears: land on the nearest surviving line.
    let vanished_line = cursor_line(&s);
    std::fs::remove_file(td.path().join("d.txt")).expect("rm d");
    type_char(&mut s, 'g');
    pump(&mut s);
    assert!(
        cursor_line(&s) > 0,
        "a vanished entry must not drop the cursor to the header"
    );
    assert_eq!(
        cursor_line(&s),
        vanished_line.min(active_lines(&s).len() - 1),
        "it lands on the nearest surviving line"
    );
}

// ---------------------------------------------------------------------------
// 8 --- sort modes
// ---------------------------------------------------------------------------

/// `s` cycles name -> mtime -> size -> name; mtime sorts newest first
/// and size largest first, each with a stable name tiebreak; the cursor
/// stays on its basename across the reorder.
#[test]
fn dired_sort_cycles_name_then_mtime_then_size() {
    let td = tempfile::tempdir().expect("tempdir");
    // Explicit sizes and mtimes, so neither order depends on the
    // filesystem's timestamp resolution or on write ordering.
    let plan = [
        ("a.txt", 3usize, 1_000u64),
        ("b.txt", 1, 3_000),
        ("c.txt", 2, 2_000),
    ];
    for (name, size, mtime) in plan {
        let path = td.path().join(name);
        std::fs::write(&path, vec![b'x'; size]).expect("write");
        let file = std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("open for set_modified");
        file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(mtime))
            .expect("set mtime");
    }

    let mut s = editor();
    open_ok(&mut s, td.path(), "nil");
    let names = |s: &EditorState| -> Vec<String> {
        active_lines(s)
            .iter()
            .skip(1)
            .map(|line| line_name(s, line))
            .collect()
    };
    assert_eq!(
        names(&s),
        vec!["a.txt", "b.txt", "c.txt"],
        "the initial order is by name"
    );

    seat_on(&s, "c.txt");
    type_char(&mut s, 's');
    assert_eq!(
        names(&s),
        vec!["b.txt", "c.txt", "a.txt"],
        "mtime sorts newest first"
    );
    assert!(
        status(&s).contains("mtime"),
        "and reports the new mode: {:?}",
        status(&s)
    );
    assert_eq!(
        cursor_entry(&s).as_deref(),
        Some("c.txt"),
        "the cursor stays on its basename across the reorder"
    );

    type_char(&mut s, 's');
    assert_eq!(
        names(&s),
        vec!["a.txt", "c.txt", "b.txt"],
        "size sorts largest first"
    );
    type_char(&mut s, 's');
    assert_eq!(
        names(&s),
        vec!["a.txt", "b.txt", "c.txt"],
        "and cycles back"
    );
}

// ---------------------------------------------------------------------------
// 9 --- tolerant listing
// ---------------------------------------------------------------------------

/// A child whose `lstat` fails no longer fails the whole listing
/// (Q#DR6): the readable entries render, the footer counts what could
/// not be read, and the default (non-opt) call still returns a bare
/// array — both forms are exercised here, so the frozen fixture's
/// contract cannot regress unnoticed.
#[test]
fn dired_tolerant_listing_renders_what_it_can_and_counts_the_rest() {
    use std::os::unix::fs::PermissionsExt;
    let td = tempfile::tempdir().expect("tempdir");
    let dir = td.path().join("no-search");
    std::fs::create_dir(&dir).expect("mkdir");
    std::fs::write(dir.join("readable.txt"), b"x").expect("write readable");
    std::fs::write(dir.join("blocked.txt"), b"x").expect("write blocked");
    // Readable but not searchable: `readdir` yields the names, every
    // child `lstat` fails.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o400)).expect("chmod 400");
    if std::fs::symlink_metadata(dir.join("readable.txt")).is_ok() {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("restore");
        eprintln!("lstat still succeeds without search permission (root?); skipping");
        return;
    }

    let mut s = editor();
    let raised = open_dired(&mut s, &dir.display().to_string(), "nil");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("restore");
    assert!(
        raised.is_none(),
        "a per-entry failure must not fail the listing: {raised:?}"
    );

    let lines = active_lines(&s);
    assert_eq!(
        lines.last().map(String::as_str),
        Some("2 entries unreadable"),
        "the footer names how much of the view is missing: {lines:#?}"
    );

    // Both call shapes, in one test: the bare array is what the frozen
    // M8.2 fixture consumes with `ipairs`.
    let shapes: Vec<i64> = eval(
        &s,
        &format!(
            "local out = {{}}\n\
             pmacs.async(function()\n\
               local bare = pmacs.fs.read_dir({:?}):await()\n\
               local tolerant = pmacs.fs.read_dir({:?}, {{ tolerant = true }}):await()\n\
               _G.SHAPES = {{\n\
                 #bare,\n\
                 bare.entries == nil and 1 or 0,\n\
                 #tolerant.entries,\n\
                 tolerant.errors ~= nil and 1 or 0,\n\
                 #tolerant.errors,\n\
               }}\n\
             end)\n\
             return out",
            td.path().display().to_string(),
            td.path().display().to_string()
        ),
    );
    assert!(shapes.is_empty(), "the async body has not run yet");
    pump(&mut s);
    let shapes: Vec<i64> = eval(&s, "return _G.SHAPES");
    assert_eq!(
        shapes,
        vec![1, 1, 1, 1, 0],
        "bare: one entry and no `entries` field; tolerant: one entry plus \
         an empty error channel"
    );

    // A failure on the parent itself is still fatal.
    let missing = td.path().join("does-not-exist");
    let raised = open_dired(&mut s, &missing.display().to_string(), "nil");
    assert!(
        raised.is_some(),
        "an unopenable directory has no partial answer"
    );
}

// ---------------------------------------------------------------------------
// 10 --- tolerant symlink targets
// ---------------------------------------------------------------------------

/// A symlink whose target is not UTF-8 lists successfully with the
/// entry present and its target reported unknown (F5). Falsified by
/// reverting the `read_link`/target arm in `read_dir_blocking`, which
/// takes the whole listing down.
#[cfg(not(target_os = "macos"))]
#[test]
fn dired_lists_a_symlink_whose_target_is_not_utf8() {
    use std::os::unix::ffi::OsStrExt;
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::write(td.path().join("real.txt"), b"x").expect("write real");
    std::os::unix::fs::symlink(
        std::ffi::OsStr::from_bytes(b"target-\xff"),
        td.path().join("weird"),
    )
    .expect("symlink");

    let mut s = editor();
    let raised = open_dired(&mut s, &td.path().display().to_string(), "nil");
    assert!(
        raised.is_none(),
        "one weird symlink must not take the directory down: {raised:?}"
    );

    let lines = active_lines(&s);
    let weird = &lines[line_of(&s, "weird")];
    assert_eq!(
        line_name(&s, weird),
        "weird -> ?",
        "the entry is listed with an unknown target"
    );
    assert!(
        lines.iter().any(|line| line_name(&s, line) == "real.txt"),
        "and the readable sibling is still there: {lines:#?}"
    );
    assert_eq!(
        lines.last().map(String::as_str),
        Some("1 entries unreadable"),
        "the footer counts it: {lines:#?}"
    );
}

// ---------------------------------------------------------------------------
// 11 --- unknown opts keys
// ---------------------------------------------------------------------------

/// A typo'd opt errors naming the key instead of silently listing in
/// fatal mode (framing §8, minor c). Silently ignoring it is exactly
/// how a tolerant listing would degrade with no signal at all.
#[test]
fn read_dir_rejects_an_unknown_opts_key() {
    let td = fixture_dir();
    let s = editor();
    let message: String = eval(
        &s,
        &format!(
            "local ok, err = pcall(pmacs.fs.read_dir, {:?}, {{ tolerat = true }})\n\
             if ok then return 'NO ERROR' end\n\
             return tostring(err)",
            td.path().display().to_string()
        ),
    );
    assert!(
        message.contains("tolerat") && message.contains("unknown opts key"),
        "the error must name the offending key; got {message:?}"
    );

    // A wrongly-typed known key is rejected too.
    let message: String = eval(
        &s,
        &format!(
            "local ok, err = pcall(pmacs.fs.read_dir, {:?}, {{ tolerant = 'yes' }})\n\
             if ok then return 'NO ERROR' end\n\
             return tostring(err)",
            td.path().display().to_string()
        ),
    );
    assert!(
        message.contains("tolerant must be a boolean"),
        "got {message:?}"
    );
}

// ---------------------------------------------------------------------------
// 12 --- non-UTF-8 names stay fatal
// ---------------------------------------------------------------------------

/// A non-UTF-8 *name* is a path-representation problem, not a listing
/// one: dired reports the structured error and creates no buffer.
/// Rendering it tolerantly would hand dired a name it could not pass
/// back through `rename`.
#[cfg(not(target_os = "macos"))]
#[test]
fn dired_reports_a_non_utf8_name_and_creates_no_buffer() {
    use std::os::unix::ffi::OsStrExt;
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        td.path()
            .join(std::ffi::OsStr::from_bytes(b"bad-\xff-name")),
        b"",
    )
    .expect("write entry");

    let mut s = editor();
    let before = active_name(&s);
    // Through the real command, so the reporting path is the one a user
    // hits rather than `pmacs.dired.open`'s raise.
    exec(
        &s,
        &format!(
            "pmacs.async(function()\n\
               local ok, err = pcall(pmacs.dired.open, {:?})\n\
               if not ok then\n\
                 pmacs.editor.set_status('dired: ' .. tostring(err.message))\n\
               end\n\
             end)",
            td.path().display().to_string()
        ),
    );
    pump(&mut s);

    let line = status(&s);
    assert!(
        line.contains("non-UTF-8") && line.contains("255"),
        "the structured error must surface with the offending raw bytes; \
         got {line:?}"
    );
    assert!(
        dired_buffer_names(&s).is_empty(),
        "and no dired buffer was created: {:?}",
        dired_buffer_names(&s)
    );
    assert_eq!(active_name(&s), before, "the active buffer is untouched");
}

// ---------------------------------------------------------------------------
// 13 --- dired-jump
// ---------------------------------------------------------------------------

/// `C-x C-j` opens dired on this file's directory with the cursor on
/// that file's line; from a buffer with no path it reports and creates
/// nothing.
#[test]
fn dired_jump_seats_the_cursor_on_the_visited_file() {
    let td = fixture_dir();
    let (mut s, anchor) = editor_in(td.path());

    ctrl(&mut s, 'x');
    ctrl(&mut s, 'j');
    pump(&mut s);

    assert_eq!(
        active_name(&s),
        format!("*dired:{}*", canon(td.path())),
        "dired opens on the file's directory"
    );
    assert_eq!(
        cursor_entry(&s).as_deref(),
        Some("anchor.txt"),
        "with the cursor on the file we jumped from"
    );
    assert!(anchor.exists());

    // From a pathless buffer: report, create nothing.
    exec(
        &s,
        "pmacs.window.switch_buffer(pmacs.buffer.create('*pathless*'))",
    );
    let before = dired_buffer_names(&s);
    ctrl(&mut s, 'x');
    ctrl(&mut s, 'j');
    pump(&mut s);
    assert!(
        status(&s).contains("no file"),
        "the reason must surface; got {:?}",
        status(&s)
    );
    assert_eq!(
        dired_buffer_names(&s),
        before,
        "and nothing new was created"
    );
    assert_eq!(active_name(&s), "*pathless*", "nor was anything displayed");
}

// ---------------------------------------------------------------------------
// 14 --- quit
// ---------------------------------------------------------------------------

/// `q` restores the previously active buffer; in a side window it
/// routes through `pmacs.window.quit`, matching `listview.quit`'s
/// Q#BP11b split.
#[test]
fn dired_quit_restores_the_previous_buffer_and_closes_a_panel() {
    let td = fixture_dir();
    let (mut s, anchor) = editor_in(td.path());
    open_ok(&mut s, td.path(), "nil");
    type_char(&mut s, 'q');
    assert_eq!(
        active_name(&s),
        canon(&anchor),
        "`q` returns to the buffer dired was opened from"
    );

    // The panel arm: `q` deletes the side window rather than switching
    // the buffer inside it.
    open_ok(&mut s, td.path(), r#"{ display = "panel" }"#);
    assert!(side_window(&s).is_some(), "fixture premise: a side window");
    type_char(&mut s, 'q');
    assert_eq!(
        side_window(&s),
        None,
        "`q` in a side window routes through window.quit"
    );
    assert_eq!(
        active_name(&s),
        canon(&anchor),
        "and focus lands back in the document window"
    );
}

// ---------------------------------------------------------------------------
// 15 --- failure leaves nothing behind
// ---------------------------------------------------------------------------

/// `C-x d` on a nonexistent directory creates no buffer, switches no
/// window, and reports the reason (the fixture's
/// `dired_open_failure_leaves_editor_unchanged` invariant), driven
/// through the real prompt.
#[test]
fn dired_open_failure_leaves_the_editor_unchanged() {
    let td = fixture_dir();
    let (mut s, anchor) = editor_in(td.path());
    let before_window = active_window(&s);
    let before_names = buffer_names(&s);

    ctrl(&mut s, 'x');
    type_char(&mut s, 'd');
    // The field prefills with the anchor's directory; append a path
    // component that does not exist.
    type_str(&mut s, "/nope");
    press(&mut s, KeyCode::Enter);
    pump(&mut s);

    let line = status(&s);
    assert!(
        line.starts_with("dired: "),
        "the failure surfaces as dired's own status message; got {line:?}"
    );
    assert_eq!(
        buffer_names(&s),
        before_names,
        "no buffer was created: {:?}",
        buffer_names(&s)
    );
    assert_eq!(active_window(&s), before_window, "no window changed");
    assert_eq!(
        active_name(&s),
        canon(&anchor),
        "the active buffer is intact"
    );
}

// ---------------------------------------------------------------------------
// 16 --- scale
// ---------------------------------------------------------------------------

/// A 10,000-entry directory renders within the fixture's established
/// 200 ms budget, on the builtin path. Carries the fixture's macOS
/// ignore gate: hosted macOS debug runners do not consistently satisfy
/// it.
#[test]
#[cfg_attr(
    target_os = "macos",
    ignore = "hosted macOS debug runners do not consistently satisfy this timing gate"
)]
fn dired_renders_10k_entries_within_200ms() {
    let td = tempfile::tempdir().expect("tempdir");
    for i in 0..10_000 {
        std::fs::write(td.path().join(format!("f{i:05}")), b"").expect("write fixture entry");
    }

    let mut s = editor();
    let started = Instant::now();
    open_ok(&mut s, td.path(), "nil");
    let elapsed = started.elapsed();

    assert_eq!(
        active_lines(&s).len(),
        10_001,
        "header plus one line per entry"
    );
    assert!(
        elapsed < Duration::from_millis(200),
        "10K entries must render within 200ms; took {elapsed:?}"
    );
}
