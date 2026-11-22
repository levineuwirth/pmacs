// tests/e10a_2_panic_location_acceptance.rs --- E10a.2: a Rust panic's
// place is visited.

//! `cargo test` prints a failing test's place as `thread 'm::t' (N)
//! panicked at src/a.rs:29:5:`. Until E10a.2 no compile rule read that
//! line, and the `file:line:col:` rule, which allowed spaces anywhere in
//! a path, took everything after the test name's last `::` as the file:
//! `RET` or `M-g n` then opened a new, empty buffer named `t' (N)
//! panicked at src/a.rs` in place of the test.
//!
//! These rows run a compilation as a user does, by keys, and visit its
//! locations by `M-g n` and by `RET` in the compilation buffer. The
//! first takes its output from a real `cargo test` over a crate it
//! writes; the rest print the lines a row needs, so that each form is
//! there whatever toolchain builds the suite.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;
use std::path::Path;
use std::time::{Duration, Instant};

mod common;
#[path = "support/mod.rs"]
mod support;

use common::iso;

#[path = "common/editor.rs"]
mod editor;
use editor::{eval, exec};

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
        press(s, KeyCode::Char(ch));
    }
}

/// `M-g n`, the unified next-error chord.
fn next_error(s: &mut EditorState) {
    alt(s, 'g');
    press(s, KeyCode::Char('n'));
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

/// An editor with isolated roots, no language server, and a declared
/// frame, so the compilation panel is shown and can take the focus.
fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    exec(&s, "pmacs.lsp.config = {}");
    s.sync_frame_geometry(FrontendId::LOCAL, pmacs::protocol::CellSize::new(24, 80));
    s
}

fn named_text(s: &EditorState, name: &str) -> String {
    let b: mlua::String = eval(
        s,
        &format!(
            r#"
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == {name:?} then
                    return id:slice(0, id:len())
                end
            end
            return ""
            "#
        ),
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
}

fn compilation_text(s: &EditorState) -> String {
    named_text(s, "*compilation*")
}

/// Every buffer's name, sorted: a stray buffer shows as one more.
fn buffer_names(s: &EditorState) -> Vec<String> {
    let joined: String = eval(
        s,
        r#"
        local out = {}
        for _, id in ipairs(pmacs.buffer.list()) do
            out[#out + 1] = pmacs.describe.buffer(id).name
        end
        table.sort(out)
        return table.concat(out, "\n")
        "#,
    );
    joined.lines().map(str::to_owned).collect()
}

/// The file the active window shows, and its caret as the 1-based line
/// and column a compiler prints.
fn place(s: &EditorState) -> (String, i64, i64) {
    let path: Option<String> = eval(s, "return pmacs.editor.file_path()");
    let line: i64 = eval(s, "return pmacs.editor.cursor_line()");
    let col: i64 = eval(s, "return pmacs.editor.cursor_col()");
    (path.unwrap_or_default(), line + 1, col + 1)
}

fn pump_until(
    s: &mut EditorState,
    timeout_ms: u64,
    mut pred: impl FnMut(&EditorState) -> bool,
) -> bool {
    let stop = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if pred(s) {
            return true;
        }
        if Instant::now() >= stop {
            return false;
        }
        s.tick_processes();
        s.tick_async();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Open `file`, then `C-c c`, the offered command erased a key at a
/// time, `cmdline` typed and `RET`: a compilation started as a user
/// starts one, in the file's project, shown in the panel with the focus
/// left in the file. Returns once the run has written its exit marker.
fn compile_by_keys(s: &mut EditorState, file: &Path, cmdline: &str, timeout_ms: u64) {
    exec(
        s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    ctrl(s, 'c');
    press(s, KeyCode::Char('c'));
    // The prompt offers `cargo build` in a Rust project, and no key
    // clears a prompt.
    for _ in 0..32 {
        press(s, KeyCode::Backspace);
    }
    type_str(s, cmdline);
    press(s, KeyCode::Enter);
    assert!(
        pump_until(s, timeout_ms, |s| compilation_text(s)
            .contains("\n[compile ")),
        "the run must reach its exit marker; status {:?}; buffer:\n{}",
        status(s),
        compilation_text(s)
    );
}

/// A crate of one library and one failing test, `ledger_tests.rs:5:5`.
fn write_crate(dir: &Path) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"panicfix\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/lib.rs"),
        "pub fn average(a: i64, b: i64) -> i64 {\n    (a + b) / 2\n}\n\n#[cfg(test)]\nmod ledger_tests;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/ledger_tests.rs"),
        "use super::average;\n\n#[test]\nfn average_rounds_half_up() {\n    assert_eq!(average(1, 2), 2);\n}\n",
    )
    .unwrap();
}

/// A project of plain files whose "build" is a script printing
/// `output`: `Cargo.toml` marks the root the compilation runs in.
fn write_project(dir: &Path, output: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"p\"\n").unwrap();
    std::fs::write(dir.join("src/lib.rs"), "// lib\n").unwrap();
    std::fs::write(
        dir.join("src/ledger_tests.rs"),
        "line one\nline two\nline three\nline four\nline five\n",
    )
    .unwrap();
    std::fs::write(dir.join("out.txt"), output).unwrap();
}

fn canonical(path: &Path) -> String {
    std::fs::canonicalize(path).unwrap().display().to_string()
}

fn assert_no_stray_buffer(s: &EditorState, before: &[String], opened: &str) {
    let mut expected: Vec<String> = before.to_vec();
    if !expected.iter().any(|n| n == opened) {
        expected.push(opened.to_owned());
        expected.sort();
    }
    assert_eq!(
        buffer_names(s),
        expected,
        "the visit opens the file it names and no other buffer"
    );
}

// ---------------------------------------------------------------------------
// A real `cargo test`
// ---------------------------------------------------------------------------

/// The baseline's failed step. `cargo test` over a crate with one
/// failing test, started by `C-c c`; `M-g n` from the source lands on
/// the test's file at the panic's line and column, and so does `RET` on
/// the panic's line in the compilation buffer. At the base both open an
/// empty buffer named from the test's name to the path.
#[test]
fn e10a_2_a_failing_test_from_a_real_cargo_run_is_visited_by_next_error_and_by_ret() {
    let dir = tempfile::tempdir().unwrap();
    write_crate(dir.path());
    let mut s = editor();
    // The run's own target directory and no compiler wrapper: the test
    // binary's environment names this build's.
    compile_by_keys(
        &mut s,
        &dir.path().join("src/lib.rs"),
        "CARGO_TARGET_DIR=target RUSTC_WRAPPER= cargo test --offline",
        300_000,
    );
    let output = compilation_text(&s);
    assert!(
        output.contains("panicked at src/ledger_tests.rs:5:5:"),
        "premise: cargo printed the panic's place; buffer:\n{output}"
    );
    let before = buffer_names(&s);
    let test_file = canonical(&dir.path().join("src/ledger_tests.rs"));

    next_error(&mut s);
    assert_eq!(
        place(&s),
        (test_file.clone(), 5, 5),
        "M-g n lands on the test's file at the panic's line and column; status {:?}",
        status(&s)
    );
    assert_no_stray_buffer(&s, &before, "ledger_tests.rs");

    // The same line by `RET`, from the compilation buffer: back to the
    // top of the test's file first, so the landing is the visit's.
    exec(&s, "pmacs.editor.goto_byte(0)");
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('o'));
    let focused: String = eval(
        &s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    );
    assert_eq!(focused, "*compilation*", "premise: C-x o reaches the panel");
    exec(&s, "pmacs.editor.goto_byte(0)");
    press(&mut s, KeyCode::Char('n'));
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        place(&s),
        (test_file, 5, 5),
        "RET on the panic's line lands on the same place; status {:?}",
        status(&s)
    );
    assert_no_stray_buffer(&s, &before, "ledger_tests.rs");
}

// ---------------------------------------------------------------------------
// The line's forms
// ---------------------------------------------------------------------------

/// What the compile run parsed, as `file|line|col|severity` rows.
fn parsed(s: &EditorState) -> Vec<String> {
    let joined: String = eval(
        s,
        r#"
        local out = {}
        for _, e in ipairs(pmacs.compile.errors()) do
            out[#out + 1] = string.format("%s|%d|%d|%s",
                e.file, e.line + 1, e.col + 1, e.severity or "-")
        end
        return table.concat(out, "\n")
        "#,
    );
    joined.lines().map(str::to_owned).collect()
}

/// The panic's line with the thread's id, as Rust prints it now, and
/// without it, as it did; a test in a nested module and one at the
/// crate's root, whose name holds no `::`. Each is read as the path,
/// line and column after `panicked at`, and visited.
#[test]
fn e10a_2_a_panic_is_read_with_and_without_the_thread_id() {
    let lines = [
        "thread 'ledger::ledger_tests::average_rounds_half_up' (1491614) panicked at src/ledger_tests.rs:3:6:",
        "thread 'ledger::ledger_tests::average_rounds_half_up' panicked at src/ledger_tests.rs:3:6:",
        "thread 'it_works' (77) panicked at src/ledger_tests.rs:3:6:",
        "thread 'main' panicked at src/ledger_tests.rs:3:6:",
    ];
    for line in lines {
        let dir = tempfile::tempdir().unwrap();
        write_project(
            dir.path(),
            &format!("running 1 test\n{line}\nassertion failed\n"),
        );
        let mut s = editor();
        compile_by_keys(
            &mut s,
            &dir.path().join("src/lib.rs"),
            "cat out.txt",
            10_000,
        );
        assert_eq!(
            parsed(&s),
            vec!["src/ledger_tests.rs|3|6|error".to_owned()],
            "the place after `panicked at` is the line's location: {line}"
        );
        let before = buffer_names(&s);
        next_error(&mut s);
        assert_eq!(
            place(&s),
            (canonical(&dir.path().join("src/ledger_tests.rs")), 3, 6),
            "M-g n lands there: {line}"
        );
        assert_no_stray_buffer(&s, &before, "ledger_tests.rs");
    }
}

/// The generic rule itself, with the panic's rule taken out of the
/// table: a path that follows prose on its line ends at a space, so the
/// rule reads `src/ledger_tests.rs` and not the line from the test's
/// name on, whether or not the name holds a `::`.
#[test]
fn e10a_2_the_generic_rule_takes_no_quoted_name_and_space_as_part_of_a_path() {
    for line in [
        "thread 'ledger::ledger_tests::average_rounds_half_up' (1491614) panicked at src/ledger_tests.rs:3:6:",
        "thread 'main' panicked at src/ledger_tests.rs:3:6:",
    ] {
        let dir = tempfile::tempdir().unwrap();
        write_project(dir.path(), &format!("{line}\n"));
        let mut s = editor();
        let removed: i64 = eval(
            &s,
            r#"
            local n = 0
            for i = #pmacs.compile.rules, 1, -1 do
                if pmacs.compile.rules[i].pattern:find("panicked at", 1, true) then
                    table.remove(pmacs.compile.rules, i)
                    n = n + 1
                end
            end
            return n
            "#,
        );
        assert!(
            removed <= 1,
            "at most one rule names a panic; removed {removed}"
        );
        compile_by_keys(
            &mut s,
            &dir.path().join("src/lib.rs"),
            "cat out.txt",
            10_000,
        );
        let got = parsed(&s);
        assert_eq!(got.len(), 1, "one location on the line: {got:?}");
        assert!(
            got[0].starts_with("src/ledger_tests.rs|3|6|"),
            "the generic rule reads the path alone: {got:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Paths with spaces
// ---------------------------------------------------------------------------

/// A path with spaces still parses where a compiler prints one: at the
/// start of its line, in the three-part and the two-part form, and after
/// rustc's arrow. Each is visited.
#[test]
fn e10a_2_a_path_with_spaces_at_the_start_of_its_line_keeps_parsing() {
    let dir = tempfile::tempdir().unwrap();
    write_project(
        dir.path(),
        "my dir/a b.c:2:3: error: one\n  my dir/a b.c:4: note: two\n  --> my dir/a b.c:1:2\n",
    );
    std::fs::create_dir_all(dir.path().join("my dir")).unwrap();
    std::fs::write(dir.path().join("my dir/a b.c"), "abc\ndef\nghi\njkl\n").unwrap();
    let mut s = editor();
    compile_by_keys(
        &mut s,
        &dir.path().join("src/lib.rs"),
        "cat out.txt",
        10_000,
    );
    assert_eq!(
        parsed(&s),
        vec![
            "my dir/a b.c|2|3|error".to_owned(),
            "my dir/a b.c|4|1|-".to_owned(),
            "my dir/a b.c|1|2|-".to_owned(),
        ]
    );
    let file = canonical(&dir.path().join("my dir/a b.c"));
    for (line, col) in [(2, 3), (4, 1), (1, 2)] {
        next_error(&mut s);
        assert_eq!(place(&s), (file.clone(), line, col));
    }
}

// ---------------------------------------------------------------------------
// A location that names no file
// ---------------------------------------------------------------------------

/// A location whose file does not exist opens no buffer and says so in
/// the echo line, by `M-g n` and by `RET`; the walk goes on to the next
/// location. At the base each visit made a new, empty buffer.
#[test]
fn e10a_2_a_location_that_names_no_file_says_so_and_opens_no_buffer() {
    let dir = tempfile::tempdir().unwrap();
    write_project(
        dir.path(),
        "gone/never.rs:3:1: error: one\nsrc/ledger_tests.rs:2:2: error: two\n",
    );
    let mut s = editor();
    compile_by_keys(
        &mut s,
        &dir.path().join("src/lib.rs"),
        "cat out.txt",
        10_000,
    );
    assert_eq!(parsed(&s).len(), 2, "premise: both lines are locations");
    let before = buffer_names(&s);
    let lib = canonical(&dir.path().join("src/lib.rs"));

    next_error(&mut s);
    assert_eq!(buffer_names(&s), before, "no buffer is opened");
    // Compared resolved: the source was opened by the path as given, and
    // on macOS the temporary directory is under the `/var` symlink.
    assert_eq!(
        canonical(Path::new(&place(&s).0)),
        lib,
        "the window still shows the source"
    );
    assert!(
        status(&s).contains("no such file") && status(&s).contains("gone/never.rs"),
        "the echo line says which file is missing: {:?}",
        status(&s)
    );

    // The walk goes on.
    next_error(&mut s);
    assert_eq!(
        place(&s),
        (canonical(&dir.path().join("src/ledger_tests.rs")), 2, 2),
        "the next M-g n visits the location after it"
    );

    // And by RET on the first line of the compilation buffer.
    let with_test = buffer_names(&s);
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('o'));
    exec(&s, "pmacs.editor.goto_byte(0)");
    press(&mut s, KeyCode::Char('n'));
    press(&mut s, KeyCode::Enter);
    assert_eq!(buffer_names(&s), with_test, "RET opens no buffer either");
    assert!(
        status(&s).contains("no such file") && status(&s).contains("gone/never.rs"),
        "RET says so too: {:?}",
        status(&s)
    );
}

// ---------------------------------------------------------------------------
// The GPU window
// ---------------------------------------------------------------------------

/// The same visit as the GPU window sees it: a daemon holding the
/// project, the production `App` in `pmacs-gpu --headless-probe`
/// pressing `C-c c`, `RET` on the offered command and `M-g n`, and where
/// the window then is. At the base the window showed an empty buffer
/// under the test's name, its caret at line 1.
#[cfg(feature = "crdt")]
#[test]
fn e10a_2_the_gpu_window_lands_on_the_panics_place() {
    use common::daemon::TestDaemon;

    let dir = tempfile::tempdir().unwrap();
    write_project(
        dir.path(),
        "running 1 test\nthread 'ledger::ledger_tests::average_rounds_half_up' (1491614) \
         panicked at src/ledger_tests.rs:3:6:\nassertion failed\n",
    );
    // The compile prompt offers this project's default command, which
    // the probe accepts as it stands.
    let init = format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.compile.defaults.rust = 'cat out.txt'\n\
         pmacs.buffer.find_or_open({:?})\n",
        dir.path().join("src/lib.rs").display().to_string()
    );
    let daemon = TestDaemon::spawn_with_env_and_init(&[], &init);
    let steps = "shows:lib.rs place:before key:C-c key:c key:ret quiet:1500 \
                 key:M-g key:n shows:ledger_tests.rs quiet:500 place:after";
    let Some(facts) = common::gpu_probe::run_script_probe(daemon.socket_path(), "e10a2", steps)
    else {
        support::record_skip("the GPU probe did not run");
        return;
    };
    let fact = |key: &str| common::gpu_probe::fact(&facts, key).to_owned();
    assert_eq!(
        fact("before.name"),
        "lib.rs",
        "premise: the window starts on the source; facts {facts:?}"
    );
    assert_eq!(
        (
            fact("after.name"),
            fact("after.line"),
            fact("after.col"),
            fact("after.text_len"),
        ),
        (
            "ledger_tests.rs".to_owned(),
            "3".to_owned(),
            "6".to_owned(),
            "line one\nline two\nline three\nline four\nline five\n"
                .len()
                .to_string(),
        ),
        "the window shows the test's file, all of it, with the caret at the panic's place; \
         facts {facts:?}"
    );
}
