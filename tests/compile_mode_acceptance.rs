//! Compile-mode acceptance (Arc 5 stage 1,
//! docs/compile-mode-framing.md, items 1–33; item 34 lives as unit
//! tests in src/process.rs, item 35 in
//! `tests/compile_mode_crdt_acceptance.rs`).
//!
//! Dispatch-driven: every keybinding claim is exercised through
//! `dispatch_key` (never `pmacs.command.invoke`), per the standing
//! discipline — a dead binding must fail these tests. Process output
//! is pumped through `tick_processes` (the production
//! `process.after-tick` path); grep streams through `tick_async`.
//! Fixtures are `/bin/sh` scripts materialized in tempdirs.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;
use std::path::Path;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Harness (auto_pair_acceptance conventions + m6_5 pump pattern)
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

fn ctrl_shift(s: &mut EditorState, c: char) {
    s.dispatch_key(
        FrontendId::LOCAL,
        key(
            KeyCode::Char(c),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
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

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

fn errors_buffer(s: &EditorState) -> String {
    s.lua_host.errors_buffer_text()
}

/// Fresh editor with LSP spawning disabled (language detection still
/// works; the after-load hook must not exec real servers).
fn editor() -> EditorState {
    let s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    s
}

fn write_script(dir: &Path, name: &str, body: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p.display().to_string()
}

/// Text of the buffer named `name`, or empty when absent.
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

fn active_buffer_name(s: &EditorState) -> String {
    eval(
        s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    )
}

fn compilation_text(s: &EditorState) -> String {
    named_text(s, "*compilation*")
}

fn process_count(s: &EditorState) -> i64 {
    eval(s, "return #pmacs.process.list()")
}

/// Drive frames until `pred` holds. Pumps both the process
/// supervisor (compile/shell) and the async runtime (grep workers).
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

/// Start a compile run programmatically with an explicit cwd.
fn compile_run(s: &EditorState, cmdline: &str, cwd: &Path) {
    exec(
        s,
        &format!(
            "pmacs.compile.run({cmdline:?}, {{ cwd = {:?} }})",
            cwd.display().to_string()
        ),
    );
}

/// Run `cmdline` and pump to its exit marker. Panics on timeout.
fn compile_and_finish(s: &mut EditorState, cmdline: &str, cwd: &Path) {
    compile_run(s, cmdline, cwd);
    assert!(
        pump_until(s, 10_000, |s| compilation_text(s).contains("[compile ")),
        "compile run must reach its exit marker; buffer:\n{}",
        compilation_text(s)
    );
}

/// Poll `path` for a pid the fixture script wrote there.
fn wait_pidfile(s: &mut EditorState, path: &Path) -> i32 {
    let mut pid = None;
    pump_until(s, 5_000, |_| {
        if let Ok(body) = std::fs::read_to_string(path)
            && let Ok(p) = body.trim().parse::<i32>()
        {
            pid = Some(p);
            return true;
        }
        false
    });
    pid.expect("fixture pidfile must appear")
}

fn pid_alive(pid: i32) -> bool {
    // `kill -0` probe via /bin/kill: portable across Linux and macOS
    // (a /proc existence check has no macOS equivalent and would
    // make every "descendant is dead" assertion vacuously true
    // there).
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Errors getter marshalled to a comparable Rust shape (encoded as
/// one line per entry — mlua tuples don't implement `FromLua`).
fn compile_errors(s: &EditorState) -> Vec<(String, i64, i64, Option<String>)> {
    let encoded: String = eval(
        s,
        r#"
        local out = {}
        for _, e in ipairs(pmacs.compile.errors()) do
            out[#out + 1] = string.format("%s|%d|%d|%s",
                e.file, e.line, e.col, e.severity or "-")
        end
        return table.concat(out, "\n")
        "#,
    );
    encoded
        .lines()
        .map(|l| {
            let mut parts = l.split('|');
            let file = parts.next().unwrap().to_owned();
            let line = parts.next().unwrap().parse().unwrap();
            let col = parts.next().unwrap().parse().unwrap();
            let sev = match parts.next().unwrap() {
                "-" => None,
                s => Some(s.to_owned()),
            };
            (file, line, col, sev)
        })
        .collect()
}

/// Rendered cells of the active window (copied from the
/// `m4_acceptance` grid helper — cross-crate test code can't import).
fn render_active_window_to_grid(
    state: &mut EditorState,
    rows: u32,
    cols: u32,
) -> Vec<pmacs::cell::Cell> {
    use pmacs::cell::{Cell, CellGrid, CellSize};
    use pmacs::view::{View, Viewport};
    use pmacs::window::Rect;

    let mut core = state.core.borrow_mut();
    let active = core.active_window_id();
    let registry = core.registry.clone();
    let win = core.windows.get_mut(&active).expect("active window");
    let rect = Rect::new(0, 0, rows, cols);
    let cell_count = (rect.size.rows * rect.size.cols) as usize;
    let mut backing = vec![Cell::default(); cell_count];
    let reg = registry.borrow();
    let buf = reg.get(win.buffer_id).expect("buffer in registry");
    let viewport = Viewport {
        buffer_start: 0,
        buffer_end: buf.len(),
        cell_origin: rect.origin,
        cell_size: CellSize::new(rect.size.rows, rect.size.cols),
        gutter_w: 0,
    };
    let mut grid = CellGrid {
        cells: &mut backing,
        stride: rect.size.cols,
        size: CellSize::new(rect.size.rows, rect.size.cols),
    };
    win.text_view.render(buf, viewport, &mut grid);
    for overlay in &mut win.overlays {
        overlay.render(buf, viewport, &mut grid);
    }
    backing
}

fn any_styled_cell(cells: &[pmacs::cell::Cell]) -> bool {
    cells
        .iter()
        .any(|c| c.style != pmacs::cell::Style::default())
}

const DESYNC: &str = "[output desynced by external edit]";

// ---------------------------------------------------------------------------
// 1–4: spawn shape, read-only, merged interleaving, EOF
// ---------------------------------------------------------------------------

#[test]
fn acc01_spawn_streams_header_output_and_exit_marker() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    compile_and_finish(&mut s, "printf 'hello\\nworld\\n'", dir.path());
    let text = compilation_text(&s);
    assert!(
        text.starts_with("$ printf"),
        "header leads with the command; got:\n{text}"
    );
    assert!(
        text.contains(&format!("Directory: {}", dir.path().display())),
        "header names the resolved cwd; got:\n{text}"
    );
    assert!(text.contains("hello\nworld\n"), "output streamed:\n{text}");
    assert!(
        text.contains("[compile exited with code 0]"),
        "exit marker with code:\n{text}"
    );
    assert!(
        status(&s).contains("finished"),
        "completion status; got: {}",
        status(&s)
    );
    assert_eq!(active_buffer_name(&s), "*compilation*", "switch-in-place");
}

#[test]
fn acc02_buffer_is_read_only_under_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    compile_and_finish(&mut s, "echo out", dir.path());
    let before = compilation_text(&s);
    type_str(&mut s, "x");
    assert_eq!(
        compilation_text(&s),
        before,
        "dispatched typing must be rejected by the read-only intercept"
    );
}

#[test]
fn acc03_stderr_interleaves_in_emission_order() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        "mix.sh",
        "echo out1\necho err1 >&2\necho out2\necho err2 >&2\n",
    );
    let mut s = editor();
    compile_and_finish(&mut s, &format!("sh {script}"), dir.path());
    let text = compilation_text(&s);
    assert!(
        text.contains("out1\nerr1\nout2\nerr2\n"),
        "child-boundary merge preserves emission order (per-tick \
         stdout-then-stderr coalescing would reorder); got:\n{text}"
    );
}

#[test]
fn acc04_stdin_eof_lets_cat_terminate() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    let t0 = Instant::now();
    compile_and_finish(&mut s, "cat; echo done", dir.path());
    assert!(
        compilation_text(&s).contains("\ndone\n"),
        "cat must see EOF and fall through (line-start match — the \
         header echoes the command and would match bare 'done')"
    );
    assert!(
        compilation_text(&s).contains("exited with code 0"),
        "clean exit"
    );
    assert!(
        t0.elapsed() < Duration::from_secs(5),
        "must not hang on piped stdin"
    );
}

// ---------------------------------------------------------------------------
// 5–9: group lifecycle through the editor surface
// ---------------------------------------------------------------------------

#[test]
fn acc05_kill_reaps_backgrounded_descendant() {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("pid");
    let mut s = editor();
    compile_run(
        &s,
        &format!("sleep 30 & echo $! > {}; wait", pidfile.display()),
        dir.path(),
    );
    let pid = wait_pidfile(&mut s, &pidfile);
    ctrl(&mut s, 'c');
    ctrl(&mut s, 'k');
    assert!(
        pump_until(&mut s, 10_000, |s| compilation_text(s)
            .contains("[compile killed by")),
        "kill must produce a signaled exit marker; buffer:\n{}",
        compilation_text(&s)
    );
    assert!(
        pump_until(&mut s, 3_000, |_| !pid_alive(pid)),
        "group-directed kill must reap the backgrounded descendant \
         (positive-pid SIGTERM would strand it)"
    );
    assert!(
        pump_until(&mut s, 3_000, |s| process_count(s) == 0),
        "process list returns to baseline"
    );
}

#[test]
fn acc06_leader_exit_without_wait_completes_promptly_and_reaps() {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("pid");
    let mut s = editor();
    let t0 = Instant::now();
    compile_run(
        &s,
        &format!("sleep 30 & echo $! > {}", pidfile.display()),
        dir.path(),
    );
    assert!(
        pump_until(&mut s, 5_000, |s| compilation_text(s)
            .contains("[compile exited")),
        "leader exit must be observed without waiting on the descendant"
    );
    assert!(
        t0.elapsed() < Duration::from_millis(2500),
        "the run must not ride the 2s drain timeout; took {:?}",
        t0.elapsed()
    );
    let pid = wait_pidfile(&mut s, &pidfile);
    assert!(
        pump_until(&mut s, 3_000, |_| !pid_alive(pid)),
        "leader-exit reap must kill the pipe-holding descendant"
    );
    assert!(
        pump_until(&mut s, 3_000, |s| process_count(s) == 0),
        "process list returns to baseline"
    );
}

#[test]
fn acc07_term_trapping_child_falls_to_sigkill() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    compile_run(&s, "trap '' TERM; echo ready; sleep 30", dir.path());
    // "\nready\n" — the OUTPUT line, not the header's echo of the
    // command (matching the header raced the kill ahead of the trap
    // installation and let plain SIGTERM win).
    assert!(
        pump_until(&mut s, 5_000, |s| compilation_text(s).contains("\nready\n")),
        "trap must be installed before we kill"
    );
    let t0 = Instant::now();
    ctrl(&mut s, 'c');
    ctrl(&mut s, 'k');
    assert!(
        pump_until(&mut s, 5_000, |s| compilation_text(s)
            .contains("killed by SIGKILL")),
        "TERM-trapping child must fall to the ledger's SIGKILL; buffer:\n{}",
        compilation_text(&s)
    );
    assert!(
        t0.elapsed() < Duration::from_secs(2),
        "escalation lands near the 500ms grace, not the drain timeout; took {:?}",
        t0.elapsed()
    );
    assert!(
        pump_until(&mut s, 3_000, |s| process_count(s) == 0),
        "baseline restored"
    );
}

/// Fixture: background a TERM-ignoring survivor; the leader exits
/// only after the survivor's trap is INSTALLED (readiness file).
/// Without the gate, a slow scheduler (macOS CI, observed) delivers
/// the leader-exit group-TERM before the subshell's `trap` runs and
/// kills the "survivor" — making these assertions vacuous. Mirrors
/// the process.rs unit fixture.
fn survivor_cmdline(dir: &Path, redirect: bool) -> (String, std::path::PathBuf) {
    let pidfile = dir.join("pid");
    let ready = dir.join("ready");
    let redirect_part = if redirect {
        "exec >/dev/null 2>&1; "
    } else {
        ""
    };
    let cmdline = format!(
        "( trap '' TERM; : > {ready}; {redirect_part}sleep 30 ) & echo $! > {pid}; \
         while [ ! -e {ready} ]; do sleep 0.01; done",
        ready = ready.display(),
        pid = pidfile.display(),
    );
    (cmdline, pidfile)
}

#[test]
fn acc08_ledger_reaps_term_ignoring_redirected_survivor() {
    // The bite: the survivor ignores TERM and sheds its output, so
    // the terminal event arrives AND the readers finish — only the
    // liveness probe can catch it.
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    let (cmdline, pidfile) = survivor_cmdline(dir.path(), true);
    compile_run(&s, &cmdline, dir.path());
    assert!(
        pump_until(&mut s, 5_000, |s| compilation_text(s)
            .contains("exited with code 0")),
        "leader exits cleanly"
    );
    let pid = wait_pidfile(&mut s, &pidfile);
    assert!(
        pump_until(&mut s, 3_000, |_| !pid_alive(pid)),
        "the kill(-pgid, 0) probe must reap the redirected survivor \
         (leader- or reader-conditioned escalation never fires here)"
    );
}

#[test]
fn acc09_pipe_holding_survivor_bounded_tick_latency() {
    // Non-redirected twin of acc08: the descendant KEEPS fd1, so the
    // readers stay alive — in-drain ledger enforcement must SIGKILL
    // at the grace bound instead of blocking ~2s.
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    let (cmdline, _pidfile) = survivor_cmdline(dir.path(), false);
    compile_run(&s, &cmdline, dir.path());
    let stop = Instant::now() + Duration::from_secs(6);
    let mut max_tick = Duration::ZERO;
    let mut done = false;
    while Instant::now() < stop {
        let t = Instant::now();
        s.tick_processes();
        max_tick = max_tick.max(t.elapsed());
        if compilation_text(&s).contains("[compile exited") {
            done = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(done, "run must complete; buffer:\n{}", compilation_text(&s));
    assert!(
        max_tick < Duration::from_millis(1200),
        "blocking tick bounded by ~grace + 2 poll intervals, not the \
         2s drain timeout; max tick {max_tick:?}"
    );
}

// ---------------------------------------------------------------------------
// 10–14: error parsing
// ---------------------------------------------------------------------------

#[test]
fn acc10_starter_rules_parse_and_normalize() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        "diag.sh",
        concat!(
            "printf 'error[E0308]: mismatched types\\n'\n",
            "printf '  --> src/foo.rs:3:5\\n'\n",
            "printf 'foo.c:7:2: warning: unused variable\\n'\n",
            "printf 'Traceback (most recent call last):\\n'\n",
            "printf '  File \"bar.py\", line 9\\n'\n",
        ),
    );
    let mut s = editor();
    compile_and_finish(&mut s, &format!("sh {script}"), dir.path());
    let errors = compile_errors(&s);
    assert_eq!(
        errors,
        vec![
            // rustc arrow: 1-based 3:5 → 0-based (2,4); the arrow
            // line carries no severity token → nil (navigable but
            // uncolored, per Q#CM4).
            ("src/foo.rs".to_owned(), 2, 4, None),
            // gcc-style colocates the keyword → sniffed severity.
            ("foo.c".to_owned(), 6, 1, Some("warning".to_owned())),
            // Python frame: no column → col 0; no severity token.
            ("bar.py".to_owned(), 8, 0, None),
        ],
        "starter-rule parse + 0-based normalization"
    );
}

#[test]
fn acc11_sub_one_coordinates_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    compile_and_finish(&mut s, "printf 'foo.rs:0:0: error: boom\\n'", dir.path());
    assert!(
        compile_errors(&s).is_empty(),
        "a 0:0 capture must be discarded, not stored as -1; got {:?}",
        compile_errors(&s)
    );
}

#[test]
fn acc12_custom_rule_severity_override_and_malformed_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    exec(
        &s,
        r#"
        pmacs.compile.rules = {
            { pattern = "x", file = 1, line = 1, severity = "fatal" },
            { pattern = "(z%.txt):(%d+):", file = 1, line = 2, severity = "warning" },
        }
        "#,
    );
    // The skip note is a transient status set at run start; capture
    // it before the completion status overwrites it.
    compile_run(&s, "printf 'z.txt:5: error: boom\\n'", dir.path());
    assert!(
        status(&s).contains("skipped 1 malformed"),
        "the severity=\"fatal\" entry is rejected and counted; got: {}",
        status(&s)
    );
    assert!(pump_until(&mut s, 10_000, |s| compilation_text(s)
        .contains("[compile exited")));
    let errors = compile_errors(&s);
    assert_eq!(
        errors,
        vec![("z.txt".to_owned(), 4, 0, Some("warning".to_owned()))],
        "the severity field overrides the sniffed 'error' keyword"
    );
}

#[test]
fn acc13_unterminated_final_line_still_parses() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    // No trailing newline: complete at EOF, parsed at the terminal
    // event (bite: fails without the finalization pass).
    compile_and_finish(&mut s, "printf 'x.c:3:1: error: no newline'", dir.path());
    assert_eq!(
        compile_errors(&s),
        vec![("x.c".to_owned(), 2, 0, Some("error".to_owned()))],
        "final unterminated diagnostic must not be dropped"
    );
}

#[test]
fn acc14_malformed_rule_containers_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    // (a) top-level non-table degrades to the built-in defaults.
    let mut s = editor();
    exec(&s, "pmacs.compile.rules = 42");
    compile_run(&s, "printf 'a.c:1:1: error: e\\n'", dir.path());
    assert!(
        status(&s).contains("not a table"),
        "one degradation note at run start; got: {}",
        status(&s)
    );
    assert!(pump_until(&mut s, 10_000, |s| compilation_text(s)
        .contains("[compile exited")));
    assert_eq!(
        compile_errors(&s).len(),
        1,
        "built-in defaults still parse under a non-table container"
    );
    assert!(
        errors_buffer(&s).is_empty(),
        "no error spam: {}",
        errors_buffer(&s)
    );

    // (b) an invalid-pattern entry is skipped; a later valid entry
    // still matches; one status note counts the skip.
    let mut s = editor();
    exec(
        &s,
        r#"
        pmacs.compile.rules = {
            { pattern = "([", file = 1, line = 2 },
            { pattern = "(b%.c):(%d+):", file = 1, line = 2 },
        }
        "#,
    );
    compile_run(&s, "printf 'b.c:4: error: e\\n'", dir.path());
    assert!(
        status(&s).contains("skipped 1 malformed"),
        "one status note at run start; got: {}",
        status(&s)
    );
    assert!(pump_until(&mut s, 10_000, |s| compilation_text(s)
        .contains("[compile exited")));
    assert_eq!(
        compile_errors(&s),
        vec![("b.c".to_owned(), 3, 0, Some("error".to_owned()))],
        "the valid entry still matches after the malformed one"
    );
    assert!(
        errors_buffer(&s).is_empty(),
        "no error spam: {}",
        errors_buffer(&s)
    );
}

// ---------------------------------------------------------------------------
// 15–18: navigation
// ---------------------------------------------------------------------------

/// Fixture: a target file plus a compile run reporting one error at
/// target.c:3:2. Returns the editor, finished, in *compilation*.
fn error_fixture(dir: &Path) -> EditorState {
    std::fs::write(dir.join("target.c"), "l1\nl2\nl3 body\nl4\n").unwrap();
    let mut s = editor();
    compile_and_finish(&mut s, "printf 'target.c:3:2: error: boom\\n'", dir);
    s
}

#[test]
fn acc15_ret_visits_error_and_jump_back_returns() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = error_fixture(dir.path());
    // Cursor starts on the header (row 0): RET there reports and
    // stays.
    press(&mut s, KeyCode::Enter);
    assert_eq!(status(&s), "no error on this line");
    assert_eq!(active_buffer_name(&s), "*compilation*");
    // n lands on the diagnostic row; RET visits at 0-based (2,1).
    press(&mut s, KeyCode::Char('n'));
    press(&mut s, KeyCode::Enter);
    assert!(
        active_buffer_name(&s).ends_with("target.c"),
        "RET visits the file; active: {}",
        active_buffer_name(&s)
    );
    let line: i64 = eval(&s, "return pmacs.editor.cursor_line()");
    let col: i64 = eval(&s, "return pmacs.editor.cursor_col()");
    assert_eq!((line, col), (2, 1), "0-based landing from 1-based 3:2");
    // M-, returns to the compilation buffer (jump ring).
    alt(&mut s, ',');
    assert_eq!(active_buffer_name(&s), "*compilation*");
}

#[test]
fn acc16_n_p_walk_error_lines_without_wrap() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    compile_and_finish(
        &mut s,
        "printf 'a.c:1:1: error: one\\nplain line\\nb.c:2:2: error: two\\n'",
        dir.path(),
    );
    let row_of = |s: &EditorState| -> i64 { eval(s, "return pmacs.editor.cursor_line()") };
    press(&mut s, KeyCode::Char('n'));
    let first = row_of(&s);
    press(&mut s, KeyCode::Char('n'));
    let second = row_of(&s);
    assert!(second > first, "n walks forward between error lines");
    press(&mut s, KeyCode::Char('n'));
    assert_eq!(status(&s), "no more errors", "no wrap at the end");
    assert_eq!(row_of(&s), second, "cursor stays");
    press(&mut s, KeyCode::Char('p'));
    assert_eq!(row_of(&s), first, "p walks back");
    press(&mut s, KeyCode::Char('p'));
    assert_eq!(status(&s), "no more errors", "no wrap at the start");
}

#[test]
fn acc17_chords_walk_compile_errors_across_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("one.c"), "a\nb\n").unwrap();
    std::fs::write(dir.path().join("two.c"), "c\nd\ne\n").unwrap();
    let mut s = editor();
    compile_and_finish(
        &mut s,
        "printf 'one.c:1:1: error: e1\\ntwo.c:3:1: error: e2\\n'",
        dir.path(),
    );
    // M-g n visits the first error, then the second, then reports.
    alt(&mut s, 'g');
    press(&mut s, KeyCode::Char('n'));
    assert!(active_buffer_name(&s).ends_with("one.c"), "first error");
    alt(&mut s, 'g');
    press(&mut s, KeyCode::Char('n'));
    assert!(active_buffer_name(&s).ends_with("two.c"), "second error");
    alt(&mut s, 'g');
    press(&mut s, KeyCode::Char('n'));
    assert_eq!(status(&s), "no more errors", "no wrap past the last");
    assert!(active_buffer_name(&s).ends_with("two.c"), "stays put");
    // M-g p walks back.
    alt(&mut s, 'g');
    press(&mut s, KeyCode::Char('p'));
    assert!(active_buffer_name(&s).ends_with("one.c"), "previous error");
    // C-x ` is the classic chord for the same dispatcher.
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('`'));
    assert!(
        active_buffer_name(&s).ends_with("two.c"),
        "C-x ` = error.next"
    );
}

#[test]
fn acc18_dispatcher_falls_back_to_diagnostics_without_a_claim() {
    let mut s = editor();
    alt(&mut s, 'g');
    press(&mut s, KeyCode::Char('n'));
    assert_eq!(
        status(&s),
        "diag: no LSP server for active buffer",
        "with no compile/grep claim, M-g n must reach diag.next \
         (today's behavior preserved exactly)"
    );
}

// ---------------------------------------------------------------------------
// 19–22: recompile, q-target, kill, supersede
// ---------------------------------------------------------------------------

#[test]
fn acc19_g_recompiles_and_clears_overlay_spans() {
    let dir = tempfile::tempdir().unwrap();
    let counter = dir.path().join("count");
    let mut s = editor();
    // First run emits a severity-colored diagnostic (overlay span);
    // each run appends to the counter file.
    compile_and_finish(
        &mut s,
        &format!(
            "echo run >> {}; printf 'a.c:1:1: error: colored\\n'",
            counter.display()
        ),
        dir.path(),
    );
    assert!(any_styled_cell(&render_active_window_to_grid(
        &mut s, 8, 60
    )));
    assert_eq!(
        std::fs::read_to_string(&counter).unwrap().lines().count(),
        1
    );
    // g re-runs the stored command.
    press(&mut s, KeyCode::Char('g'));
    assert!(
        pump_until(&mut s, 10_000, |_| {
            std::fs::read_to_string(&counter).is_ok_and(|c| c.lines().count() == 2)
        }),
        "recompile must actually re-execute the command"
    );
    // The rerun executes the SAME stored command, so its output
    // carries the same diagnostic; per-run reset is pinned by
    // checking the fresh run reached its own marker with
    // exactly one diagnostic parsed (not accumulated).
    assert!(pump_until(&mut s, 10_000, |s| compilation_text(s)
        .contains("[compile exited")));
    assert_eq!(
        compile_errors(&s).len(),
        1,
        "error list resets per run (not accumulated)"
    );
}

#[test]
fn acc20_compile_g_q_restores_the_original_buffer() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("origin.txt"), "home\n").unwrap();
    let mut s = editor();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            dir.path().join("origin.txt").display().to_string()
        ),
    );
    compile_and_finish(&mut s, "echo one", dir.path());
    press(&mut s, KeyCode::Char('g'));
    assert!(pump_until(&mut s, 10_000, |s| compilation_text(s)
        .contains("[compile exited")));
    press(&mut s, KeyCode::Char('q'));
    assert!(
        active_buffer_name(&s).ends_with("origin.txt"),
        "q restores the pre-compile buffer even after g (q-target \
         not re-captured); active: {}",
        active_buffer_name(&s)
    );
}

#[test]
fn acc21_kill_produces_signaled_marker() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    compile_run(&s, "echo running; sleep 30", dir.path());
    assert!(pump_until(&mut s, 5_000, |s| compilation_text(s)
        .contains("\nrunning\n")));
    ctrl(&mut s, 'c');
    ctrl(&mut s, 'k');
    assert!(
        pump_until(&mut s, 5_000, |s| compilation_text(s)
            .contains("[compile killed by SIGTERM]")),
        "plain kill: SIGTERM marker; buffer:\n{}",
        compilation_text(&s)
    );
}

#[test]
fn acc22_supersede_resets_and_returns_to_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    assert_eq!(process_count(&s), 0);
    compile_run(&s, "echo first-run-output; sleep 30", dir.path());
    assert!(pump_until(&mut s, 5_000, |s| compilation_text(s)
        .contains("\nfirst-run-output\n")));
    compile_run(&s, "echo second-run-output", dir.path());
    assert!(pump_until(&mut s, 10_000, |s| compilation_text(s)
        .contains("[compile exited with code 0]")));
    let text = compilation_text(&s);
    assert!(
        !text.contains("\nfirst-run-output\n"),
        "old-run output must not land after the reset:\n{text}"
    );
    assert!(
        !text.contains("killed by"),
        "the superseded run's exit marker must not land either:\n{text}"
    );
    assert!(
        text.contains("\nsecond-run-output\n"),
        "new run streams:\n{text}"
    );
    assert!(
        pump_until(&mut s, 5_000, |s| process_count(s) == 0),
        "both generations forgotten once drained"
    );
}

// ---------------------------------------------------------------------------
// 23–25: undo surfaces and the revision guard
// ---------------------------------------------------------------------------

#[test]
fn acc23_all_seven_undo_redo_chords_are_status_noops() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    compile_and_finish(&mut s, "echo out", dir.path());
    let before = compilation_text(&s);

    let check = |s: &mut EditorState, label: &str| {
        assert_eq!(
            status(s),
            "generated buffer: undo disabled",
            "{label} must reach the buffer-local no-op"
        );
        assert_eq!(compilation_text(s), before, "{label} must not edit");
        exec(s, "pmacs.editor.set_status('')");
    };

    ctrl(&mut s, '/');
    check(&mut s, "C-/");
    ctrl(&mut s, '_');
    check(&mut s, "C-_");
    ctrl(&mut s, '4');
    check(&mut s, "C-4 (raw-terminal single-key undo)");
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('u'));
    check(&mut s, "C-x u");
    ctrl(&mut s, '?');
    check(&mut s, "C-? (redo)");
    ctrl_shift(&mut s, '_');
    check(&mut s, "C-S-_ (redo)");
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('r'));
    check(&mut s, "C-x r (redo)");
}

#[test]
fn acc24_command_path_undo_after_completed_run_recovers_immediately() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    exec(
        &s,
        &format!(
            "pmacs.shell.command('echo shell-out', {{ cwd = {:?} }})",
            dir.path().display().to_string()
        ),
    );
    assert!(pump_until(&mut s, 10_000, |s| named_text(
        s,
        "*shell-command*"
    )
    .contains("[shell exited with code 0]")));
    // M-x buffer.undo: the command path rebinding cannot reach. No
    // pump event will ever arrive — recovery must come from the
    // buffer.after-edit subscription, synchronously.
    alt(&mut s, 'x');
    type_str(&mut s, "buffer.undo");
    press(&mut s, KeyCode::Enter);
    let text = named_text(&s, "*shell-command*");
    assert!(
        text.contains(DESYNC),
        "desync marker must appear immediately (bite: fails when \
         recovery only runs at pump/anchor time); buffer:\n{text}"
    );
    assert!(
        errors_buffer(&s).is_empty(),
        "clean *errors*: {}",
        errors_buffer(&s)
    );
}

#[test]
fn acc25a_no_hook_shrink_mid_stream_recovers_and_reanchors() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("b.c"), "x\ny\nz\n").unwrap();
    let script = write_script(
        dir.path(),
        "slow.sh",
        "printf 'a.c:1:1: error: one\\n'\nsleep 1\nprintf 'b.c:2:1: error: two\\n'\n",
    );
    let mut s = editor();
    compile_run(&s, &format!("sh {script}"), dir.path());
    assert!(pump_until(&mut s, 5_000, |s| compilation_text(s).contains(": one")));
    let first_row: i64 = {
        // The first diagnostic's row, while its anchor is live.
        press(&mut s, KeyCode::Char('n'));
        eval(&s, "return pmacs.editor.cursor_line()")
    };
    // Harness-eval mutation: outside dispatch, outside
    // with_after_edit_check — the actual no-hook producer the pump
    // guard owns.
    exec(
        &s,
        r#"
        for _, id in ipairs(pmacs.buffer.list()) do
            if pmacs.describe.buffer(id).name == "*compilation*" then
                id:delete(id:len() - 3, id:len(), { bypass_intercept = true })
            end
        end
        "#,
    );
    assert!(
        pump_until(&mut s, 10_000, |s| compilation_text(s)
            .contains("[compile exited")),
        "the pump must survive and finish; buffer:\n{}",
        compilation_text(&s)
    );
    let text = compilation_text(&s);
    assert!(
        text.contains(&format!("\n{DESYNC}\n")),
        "newline-delimited marker; buffer:\n{text}"
    );
    assert!(
        text.find(DESYNC).unwrap() < text.find("two").unwrap(),
        "streaming continued after the marker:\n{text}"
    );
    assert!(
        errors_buffer(&s).is_empty(),
        "no spam: {}",
        errors_buffer(&s)
    );
    // Pre-marker anchor dropped: RET on the old row reports.
    exec(&s, "pmacs.editor.goto_byte(0)");
    let target = first_row;
    exec(
        &s,
        &format!("for _ = 1, {target} do pmacs.editor.move_down() end"),
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(status(&s), "no error on this line", "stale anchor dropped");
    // Fresh epoch: the post-marker diagnostic gets a working anchor.
    exec(&s, "pmacs.editor.goto_byte(0)");
    press(&mut s, KeyCode::Char('n'));
    press(&mut s, KeyCode::Enter);
    assert!(
        active_buffer_name(&s).ends_with("b.c"),
        "post-marker diagnostic navigates; active: {}",
        active_buffer_name(&s)
    );
}

#[test]
fn acc25b_same_length_newline_moving_replace_is_caught() {
    // The length-guard killer: content changes, length doesn't, and
    // a newline moves so every anchor stays in bounds but rows lie.
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        "slow.sh",
        "printf 'a.c:1:1: error: one\\n'\nsleep 1\nprintf 'done\\n'\n",
    );
    let mut s = editor();
    compile_run(&s, &format!("sh {script}"), dir.path());
    assert!(pump_until(&mut s, 5_000, |s| compilation_text(s).contains(": one")));
    // Replace "one\n" with "one!" — same length, newline moved.
    exec(
        &s,
        r#"
        for _, id in ipairs(pmacs.buffer.list()) do
            if pmacs.describe.buffer(id).name == "*compilation*" then
                local text = id:slice(0, id:len())
                local at = text:find("one\n", 1, true)
                id:replace(at - 1, at + 3, "one!", { bypass_intercept = true })
            end
        end
        "#,
    );
    assert!(
        pump_until(&mut s, 10_000, |s| compilation_text(s)
            .contains("[compile exited")),
        "pump survives; buffer:\n{}",
        compilation_text(&s)
    );
    assert!(
        compilation_text(&s).contains(DESYNC),
        "revision guard catches what a length guard provably misses:\n{}",
        compilation_text(&s)
    );
}

// ---------------------------------------------------------------------------
// 26: ANSI + rendered styling + M-, retention
// ---------------------------------------------------------------------------

#[test]
fn acc26_ansi_renders_styled_cells_and_survives_jump_back() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("x.c"), "a\nb\n").unwrap();
    let script = write_script(
        dir.path(),
        "color.sh",
        concat!(
            "printf '\\033[31mredtext\\033[0m plain\\n'\n",
            "printf 'progress 1\\rprogress 2\\n'\n",
            "printf 'x.c:1:1: error: e\\n'\n",
        ),
    );
    let mut s = editor();
    compile_and_finish(&mut s, &format!("sh {script}"), dir.path());
    let text = compilation_text(&s);
    assert!(!text.contains('\u{1b}'), "no escape bytes:\n{text}");
    assert!(text.contains("redtext"), "SGR text survives:\n{text}");
    assert!(
        text.contains("progress 2") && !text.contains("progress 1"),
        "CR progress collapsed in place:\n{text}"
    );
    // Attachment proven: a rendered cell in the ACTIVE WINDOW
    // carries style (handle spans alone would pass even when
    // attach_style_overlay was never called).
    assert!(
        any_styled_cell(&render_active_window_to_grid(&mut s, 10, 60)),
        "rendered TUI cell must carry the SGR span's style"
    );
    // RET to the file and M-, back must retain styling (rides the
    // jump_back after-switch parity + re-attach subscription).
    press(&mut s, KeyCode::Char('n'));
    press(&mut s, KeyCode::Enter);
    assert!(active_buffer_name(&s).ends_with("x.c"));
    alt(&mut s, ',');
    assert_eq!(active_buffer_name(&s), "*compilation*");
    assert!(
        any_styled_cell(&render_active_window_to_grid(&mut s, 10, 60)),
        "styling must survive the RET → M-, round trip"
    );
}

// ---------------------------------------------------------------------------
// 27: killed buffer mid-run
// ---------------------------------------------------------------------------

#[test]
fn acc27_killed_buffer_terminates_run_and_recreates() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    compile_run(&s, "echo alive; sleep 30", dir.path());
    assert!(pump_until(&mut s, 5_000, |s| compilation_text(s).contains("\nalive\n")));
    exec(
        &s,
        r#"
        for _, id in ipairs(pmacs.buffer.list()) do
            if pmacs.describe.buffer(id).name == "*compilation*" then
                pmacs.buffer.remove(id)
            end
        end
        "#,
    );
    assert!(
        pump_until(&mut s, 5_000, |s| process_count(s) == 0),
        "run terminated and forgotten after buffer death"
    );
    assert!(
        errors_buffer(&s).is_empty(),
        "no spam: {}",
        errors_buffer(&s)
    );
    // The next run recreates the buffer and completes.
    compile_and_finish(&mut s, "echo reborn", dir.path());
    assert!(compilation_text(&s).contains("reborn"));
}

// ---------------------------------------------------------------------------
// 28–30: grep-mode
// ---------------------------------------------------------------------------

fn grep_fixture(dir: &Path) {
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::write(
        dir.join("f.txt"),
        "top\nzqxvbn_needle_77 here\nzqxvbn_needle_77 again\n",
    )
    .unwrap();
}

fn search(s: &EditorState, query: &str, root: &Path) {
    exec(
        s,
        &format!(
            "pmacs.project.search({query:?}, {{ root = {:?} }})",
            root.display().to_string()
        ),
    );
}

fn search_done(s: &EditorState) -> bool {
    named_text(s, "*search-results*").contains("-- search ")
}

#[test]
fn acc28_grep_panel_is_a_locations_buffer() {
    let dir = tempfile::tempdir().unwrap();
    grep_fixture(dir.path());
    let mut s = editor();
    search(&s, "zqxvbn_needle_77", dir.path());
    assert!(pump_until(&mut s, 10_000, search_done), "search completes");
    assert_eq!(active_buffer_name(&s), "*search-results*");
    let text = named_text(&s, "*search-results*");
    assert!(
        text.contains("f.txt:2:0:"),
        "structured match line:\n{text}"
    );
    // Read-only under dispatch.
    type_str(&mut s, "x");
    assert_eq!(named_text(&s, "*search-results*"), text, "read-only");
    // RET visits the first match (line 2 → 0-based 1; col 0).
    press(&mut s, KeyCode::Char('n'));
    press(&mut s, KeyCode::Enter);
    assert!(active_buffer_name(&s).ends_with("f.txt"));
    let line: i64 = eval(&s, "return pmacs.editor.cursor_line()");
    assert_eq!(line, 1, "grep 1-based line normalized");
    // The search claimed the error source: M-g n continues the walk
    // (RET re-seated the index at match 1, so next = match 2).
    alt(&mut s, ',');
    alt(&mut s, 'g');
    press(&mut s, KeyCode::Char('n'));
    assert!(
        active_buffer_name(&s).ends_with("f.txt"),
        "M-g n walks grep matches (source claimed)"
    );
    let line: i64 = eval(&s, "return pmacs.editor.cursor_line()");
    assert_eq!(line, 2, "M-g n advanced to the second match");
    // A second search supersedes: fresh page.
    alt(&mut s, ',');
    search(&s, "top", dir.path());
    assert!(pump_until(&mut s, 10_000, search_done));
    let text2 = named_text(&s, "*search-results*");
    assert!(
        text2.contains("Searching for: top") && !text2.contains("zqxvbn_needle_77 here"),
        "supersede gives a fresh page:\n{text2}"
    );
}

#[test]
fn acc29_grep_kill_mid_search_is_safe_and_masking_is_prevented() {
    // A wide fixture keeps the stream alive across many ticks so a
    // mid-stream window deterministically exists.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    for i in 0..2000 {
        std::fs::write(
            dir.path().join(format!("f{i:04}.txt")),
            "zqxvbn_needle_77\n",
        )
        .unwrap();
    }
    // (a) masking prevention: a no-hook edit between producer writes
    // is detected by the NEXT producer write (a later batch or the
    // close trailer), not silently absorbed.
    let mut s = editor();
    search(&s, "zqxvbn_needle_77", dir.path());
    assert!(
        pump_until(&mut s, 10_000, |s| {
            let t = named_text(s, "*search-results*");
            t.contains(":1:0:") && !t.contains("-- search ")
        }),
        "must observe a mid-stream state (matches landed, not closed)"
    );
    exec(
        &s,
        r#"
        for _, id in ipairs(pmacs.buffer.list()) do
            if pmacs.describe.buffer(id).name == "*search-results*" then
                id:insert(id:len(), "INTRUDER", { bypass_intercept = true })
            end
        end
        "#,
    );
    assert!(pump_until(&mut s, 15_000, search_done));
    let text = named_text(&s, "*search-results*");
    let marker_at = text.find(DESYNC);
    let trailer_at = text.find("-- search ").unwrap();
    assert!(
        marker_at.is_some() && marker_at.unwrap() < trailer_at,
        "the next producer write must mark the mismatch before \
         appending (not mask it); tail:\n…{}",
        &text[text.len().saturating_sub(400)..]
    );
    assert!(
        errors_buffer(&s).is_empty(),
        "no spam: {}",
        errors_buffer(&s)
    );

    // (b) killing the panel mid-search: no stale-handle writes, and
    // the next search recreates the buffer.
    let mut s = editor();
    search(&s, "zqxvbn_needle_77", dir.path());
    assert!(pump_until(&mut s, 10_000, |s| named_text(
        s,
        "*search-results*"
    )
    .contains(":1:0:")));
    exec(
        &s,
        r#"
        for _, id in ipairs(pmacs.buffer.list()) do
            if pmacs.describe.buffer(id).name == "*search-results*" then
                pmacs.buffer.remove(id)
            end
        end
        "#,
    );
    // Drain whatever the worker still delivers.
    let _ = pump_until(&mut s, 1_000, |_| false);
    assert!(
        errors_buffer(&s).is_empty(),
        "no spam: {}",
        errors_buffer(&s)
    );
    search(&s, "zqxvbn_needle_77", dir.path());
    assert!(
        pump_until(&mut s, 15_000, |s| named_text(s, "*search-results*")
            .contains(":1:0:")),
        "a subsequent search recreates the panel and works"
    );
}

#[test]
fn acc30_grep_root_retained_across_interactive_supersede() {
    let dir = tempfile::tempdir().unwrap();
    grep_fixture(dir.path());
    let mut s = editor();
    // First search from a project file: root comes from
    // pmacs.project.detect (the .git marker).
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            dir.path().join("f.txt").display().to_string()
        ),
    );
    exec(&s, "pmacs.project.search('zqxvbn_needle_77')");
    assert!(pump_until(&mut s, 10_000, |s| named_text(
        s,
        "*search-results*"
    )
    .contains("f.txt:2:0:")));
    // Second search issued from inside the pathless panel, no
    // opts.root: the panel's stored root must be reused (the "."
    // fallback would search the test process's cwd and find
    // nothing).
    exec(&s, "pmacs.project.search('zqxvbn_needle_77')");
    assert!(
        pump_until(&mut s, 10_000, |s| {
            let t = named_text(s, "*search-results*");
            t.contains("f.txt:2:0:") && t.contains("-- search ")
        }),
        "second interactive search must run with the first search's \
         root; panel:\n{}",
        named_text(&s, "*search-results*")
    );
    // And RET still resolves against that root.
    press(&mut s, KeyCode::Char('n'));
    press(&mut s, KeyCode::Enter);
    assert!(active_buffer_name(&s).ends_with("f.txt"));
}

// ---------------------------------------------------------------------------
// 31–33: shell-command, q, round-trip flag
// ---------------------------------------------------------------------------

#[test]
fn acc31_shell_command_via_m_bang_does_not_claim_errors() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("one.c"), "a\n").unwrap();
    let mut s = editor();
    // Compile first so the error source is claimed by compile.
    compile_and_finish(&mut s, "printf 'one.c:1:1: error: e\\n'", dir.path());
    // M-! prompts; type the command; RET runs it.
    alt(&mut s, '!');
    type_str(&mut s, "echo shellout");
    press(&mut s, KeyCode::Enter);
    assert!(
        pump_until(&mut s, 10_000, |s| named_text(s, "*shell-command*")
            .contains("[shell exited with code 0]")),
        "M-! output + exit marker; buffer:\n{}",
        named_text(&s, "*shell-command*")
    );
    assert!(named_text(&s, "*shell-command*").contains("\nshellout\n"));
    // The shell run must NOT have stolen the claim: M-g n still
    // walks the compile errors.
    alt(&mut s, 'g');
    press(&mut s, KeyCode::Char('n'));
    assert!(
        active_buffer_name(&s).ends_with("one.c"),
        "M-g n after M-! still walks the prior compile; active: {}",
        active_buffer_name(&s)
    );
}

#[test]
fn acc32_q_restores_previous_buffer_from_all_three() {
    let dir = tempfile::tempdir().unwrap();
    grep_fixture(dir.path());
    std::fs::write(dir.path().join("home.txt"), "hi\n").unwrap();
    let mut s = editor();
    let open_home = format!(
        "pmacs.buffer.find_or_open({:?})",
        dir.path().join("home.txt").display().to_string()
    );
    exec(&s, &open_home);
    // *compilation*
    compile_and_finish(&mut s, "echo x", dir.path());
    press(&mut s, KeyCode::Char('q'));
    assert!(
        active_buffer_name(&s).ends_with("home.txt"),
        "q from compile"
    );
    // *shell-command*
    exec(
        &s,
        &format!(
            "pmacs.shell.command('echo y', {{ cwd = {:?} }})",
            dir.path().display().to_string()
        ),
    );
    assert!(pump_until(&mut s, 10_000, |s| named_text(
        s,
        "*shell-command*"
    )
    .contains("[shell exited")));
    press(&mut s, KeyCode::Char('q'));
    assert!(active_buffer_name(&s).ends_with("home.txt"), "q from shell");
    // *search-results*
    search(&s, "zqxvbn_needle_77", dir.path());
    assert!(pump_until(&mut s, 10_000, search_done));
    press(&mut s, KeyCode::Char('q'));
    assert!(
        active_buffer_name(&s).ends_with("home.txt"),
        "q from search"
    );
}

#[test]
fn acc33_round_trip_input_is_set_on_generated_buffers() {
    let dir = tempfile::tempdir().unwrap();
    grep_fixture(dir.path());
    let mut s = editor();
    compile_and_finish(&mut s, "echo x", dir.path());
    assert!(
        s.core.borrow().active_buffer_round_trips(),
        "*compilation* must round-trip (semantic frontends gate their \
         optimistic path on this)"
    );
    exec(
        &s,
        &format!(
            "pmacs.shell.command('echo y', {{ cwd = {:?} }})",
            dir.path().display().to_string()
        ),
    );
    assert!(
        s.core.borrow().active_buffer_round_trips(),
        "*shell-command*"
    );
    search(&s, "zqxvbn_needle_77", dir.path());
    assert!(
        s.core.borrow().active_buffer_round_trips(),
        "*search-results*"
    );
}

// ---------------------------------------------------------------------------
// PR #113 round 1 — bite tests (one per finding; each observed
// failing against the pre-fix tree via scripts/bite)
// ---------------------------------------------------------------------------

#[test]
fn r1f1_non_finite_coordinates_fail_closed() {
    // A 400-digit line capture tonumbers to math.huge; pre-fix it
    // was stored and any visit walked forever.
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    let digits = "9".repeat(400);
    compile_and_finish(
        &mut s,
        &format!("printf 'h.c:{digits}:1: error: e\\n'"),
        dir.path(),
    );
    assert!(
        compile_errors(&s).is_empty(),
        "a non-finite line coordinate must be discarded; got {:?}",
        compile_errors(&s)
    );
}

#[test]
fn r1f1_beyond_eol_column_clamps_to_the_target_row() {
    // Column past EOL: pre-fix the walk marched move_right across
    // newlines and landed rows away from the diagnostic's line (and
    // an astronomical value walked effectively forever).
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("w.c"), "ab\ncd\nef\n").unwrap();
    let mut s = editor();
    compile_and_finish(&mut s, "printf 'w.c:1:500: error: e\\n'", dir.path());
    press(&mut s, KeyCode::Char('n'));
    press(&mut s, KeyCode::Enter);
    assert!(active_buffer_name(&s).ends_with("w.c"));
    let line: i64 = eval(&s, "return pmacs.editor.cursor_line()");
    assert_eq!(
        line, 0,
        "the column walk must clamp at the target row's EOL, not run \
         onto later rows"
    );
}

#[test]
fn r1f2_grep_command_path_undo_after_completed_search_recovers() {
    let dir = tempfile::tempdir().unwrap();
    grep_fixture(dir.path());
    let mut s = editor();
    search(&s, "zqxvbn_needle_77", dir.path());
    assert!(pump_until(&mut s, 10_000, search_done), "search completes");
    // M-x buffer.undo in the completed panel: no producer write or
    // navigation may ever come — recovery must be immediate via the
    // buffer.after-edit subscription.
    alt(&mut s, 'x');
    type_str(&mut s, "buffer.undo");
    press(&mut s, KeyCode::Enter);
    let text = named_text(&s, "*search-results*");
    assert!(
        text.contains(DESYNC),
        "desync marker must appear immediately in the grep panel; \
         buffer:\n{text}"
    );
    assert!(
        errors_buffer(&s).is_empty(),
        "clean *errors*: {}",
        errors_buffer(&s)
    );
}

#[test]
fn r1f3_rustc_arrow_paths_with_spaces_parse() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        "space.sh",
        "printf '  --> /tmp/my dir/foo.rs:12:4\\n'\n",
    );
    let mut s = editor();
    compile_and_finish(&mut s, &format!("sh {script}"), dir.path());
    assert_eq!(
        compile_errors(&s),
        vec![("/tmp/my dir/foo.rs".to_owned(), 11, 3, None)],
        "the rustc rule must capture space-containing paths whole \
         (pre-fix the two-part fallback recorded arrow junk at col 0)"
    );
}

#[test]
fn r1f4_capture_indexes_above_three_and_absent_columns() {
    let dir = tempfile::tempdir().unwrap();
    // (a) a valid four-capture rule with col = 4 must store the
    // fourth capture, not silently column 0.
    let mut s = editor();
    exec(
        &s,
        r#"
        pmacs.compile.rules = {
            { pattern = "(q%.c):(%d+):(x):(%d+)", file = 1, line = 2, col = 4 },
        }
        "#,
    );
    compile_and_finish(&mut s, "printf 'q.c:7:x:9 error\\n'", dir.path());
    assert_eq!(
        compile_errors(&s),
        vec![("q.c".to_owned(), 6, 8, Some("error".to_owned()))],
        "capture index 4 must be honored"
    );

    // (b) fractional indexes are malformed; (c) a rule that names a
    // column its match didn't produce rejects the match.
    let mut s = editor();
    exec(
        &s,
        r#"
        pmacs.compile.rules = {
            { pattern = "(a%.c):(%d+):", file = 1, line = 2, col = 1.5 },
            { pattern = "(r%.c):(%d+):?(%d*)", file = 1, line = 2, col = 3 },
        }
        "#,
    );
    compile_run(&s, "printf 'a.c:3: e\\nr.c:5: e\\n'", dir.path());
    assert!(
        status(&s).contains("skipped 1 malformed"),
        "the fractional index is rejected at validation; got: {}",
        status(&s)
    );
    assert!(pump_until(&mut s, 10_000, |s| compilation_text(s)
        .contains("[compile exited")));
    assert!(
        compile_errors(&s).is_empty(),
        "an empty column capture under a col-naming rule must reject \
         the match, not store column 0; got {:?}",
        compile_errors(&s)
    );
}

#[test]
fn r1f5_user_global_cannot_shadow_the_marker_helper() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    // A hostile (or merely colliding) user global with the helper's
    // old name: pre-fix this replaced the module's function and its
    // error consumed the terminal event before forget ran.
    exec(&s, "_G.emit_text_raw = function() error('shadowed') end");
    compile_and_finish(&mut s, "echo fine", dir.path());
    assert!(
        compilation_text(&s).contains("[compile exited with code 0]"),
        "the exit marker must come from the module's local helper"
    );
    assert!(
        errors_buffer(&s).is_empty(),
        "no error spam: {}",
        errors_buffer(&s)
    );
    assert!(
        pump_until(&mut s, 3_000, |s| process_count(s) == 0),
        "terminal-event cleanup (forget) must have run"
    );
}

#[test]
fn r1f6_wrong_spec_types_error_instead_of_defaulting() {
    let s = editor();
    let (ok, err): (bool, String) = eval(
        &s,
        r#"
        local ok, err = pcall(pmacs.process.spawn,
            { label = "t", command = "/bin/true", stdin = true })
        return ok, tostring(err)
        "#,
    );
    assert!(!ok, "stdin = true (boolean) must be a hard error");
    assert!(err.contains("stdin must be"), "pointed message; got: {err}");
    let (ok, err): (bool, String) = eval(
        &s,
        r#"
        local ok, err = pcall(pmacs.process.spawn,
            { label = "t", command = "/bin/true", group = "true" })
        return ok, tostring(err)
        "#,
    );
    assert!(!ok, "group = \"true\" (string) must be a hard error");
    assert!(
        err.contains("group must be a boolean"),
        "pointed message; got: {err}"
    );
}

#[test]
fn r1f7_resync_invalidates_the_public_byte_anchor() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    compile_and_finish(&mut s, "printf 'a.c:1:1: error: e\\n'", dir.path());
    let has_anchor: bool = eval(
        &s,
        "return pmacs.compile.errors()[1].line_start_byte ~= nil",
    );
    assert!(has_anchor, "pre-desync entries carry the byte anchor");
    // Trigger the guard through the command path.
    alt(&mut s, 'x');
    type_str(&mut s, "buffer.undo");
    press(&mut s, KeyCode::Enter);
    assert!(compilation_text(&s).contains(DESYNC), "marker appended");
    let has_anchor: bool = eval(
        &s,
        "return pmacs.compile.errors()[1].line_start_byte ~= nil",
    );
    assert!(
        !has_anchor,
        "total pre-marker anchor invalidation includes the public \
         line_start_byte, not just the display row"
    );
}

#[test]
fn r1f8_inherited_cwd_resolves_to_the_daemon_working_directory() {
    // Pathless scratch buffer, no opts.cwd, no project: the header
    // must print the real working directory, and relative error
    // paths must resolve against it explicitly.
    let mut s = editor();
    exec(&s, "pmacs.compile.run('echo hi')");
    assert!(pump_until(&mut s, 10_000, |s| compilation_text(s)
        .contains("[compile exited")));
    let cwd = std::env::current_dir().unwrap().display().to_string();
    let text = compilation_text(&s);
    assert!(
        text.contains(&format!("Directory: {cwd}")),
        "the header must name the daemon's actual working directory; \
         got:\n{text}"
    );
    assert!(
        !text.contains("(inherited)") && !text.contains("(unknown)"),
        "no placeholder when the identity API is available:\n{text}"
    );
}

#[test]
fn r1f9_truncated_utf8_at_eof_becomes_the_replacement_character() {
    let dir = tempfile::tempdir().unwrap();
    // \303 (0xC3) opens a two-byte sequence that never completes:
    // the parser's cross-feed buffer holds it, and only the new
    // stream-end finish() can flush it as U+FFFD.
    let script = write_script(dir.path(), "trunc.sh", "printf 'abc\\303'\n");
    let mut s = editor();
    compile_and_finish(&mut s, &format!("sh {script}"), dir.path());
    let text = compilation_text(&s);
    assert!(
        text.contains("abc\u{FFFD}"),
        "the truncated sequence must surface as U+FFFD, not vanish; \
         buffer:\n{text}"
    );
}

// ---------------------------------------------------------------------------
// PR #113 round 2 — bite tests
// ---------------------------------------------------------------------------

#[test]
fn r2f1_rule_validation_is_a_stable_snapshot() {
    // Mutating the user's rule object AFTER compile.run() must not
    // alter the in-flight run: validation copies scalar fields into
    // per-run plain tables.
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(dir.path(), "slow.sh", "sleep 0.5\nprintf 'm.c:3: e\\n'\n");
    let mut s = editor();
    exec(
        &s,
        r#"pmacs.compile.rules = { { pattern = "(m%.c):(%d+):", file = 1, line = 2 } }"#,
    );
    compile_run(&s, &format!("sh {script}"), dir.path());
    // The output hasn't arrived yet; sabotage the live rule object.
    exec(&s, "pmacs.compile.rules[1].pattern = 'nevermatch'");
    assert!(pump_until(&mut s, 10_000, |s| compilation_text(s)
        .contains("[compile exited")));
    assert_eq!(
        compile_errors(&s),
        vec![("m.c".to_owned(), 2, 0, None)],
        "the run must parse with its validated snapshot, not the \
         mutated object"
    );
}

#[test]
fn r2f1_metatable_backed_rules_cannot_raise_through_the_pump() {
    // A rule whose field reads raise (hostile __index) and a
    // container whose traversal raises: both must degrade cleanly —
    // no error thrown through the per-frame pump, terminal cleanup
    // intact.
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    exec(
        &s,
        r#"
        local hostile = setmetatable({}, { __index = function() error("boom") end })
        pmacs.compile.rules = { hostile,
            { pattern = "(k%.c):(%d+):", file = 1, line = 2 } }
        "#,
    );
    compile_run(&s, "printf 'k.c:4: e\\n'", dir.path());
    assert!(
        status(&s).contains("skipped 1 malformed"),
        "the hostile entry is a counted skip; got: {}",
        status(&s)
    );
    assert!(pump_until(&mut s, 10_000, |s| compilation_text(s)
        .contains("[compile exited with code 0]")));
    assert_eq!(
        compile_errors(&s),
        vec![("k.c".to_owned(), 3, 0, None)],
        "the valid entry still parses"
    );
    assert!(
        errors_buffer(&s).is_empty(),
        "nothing raised through the pump: {}",
        errors_buffer(&s)
    );
    assert!(
        pump_until(&mut s, 3_000, |s| process_count(s) == 0),
        "terminal cleanup ran"
    );

    // Hostile CONTAINER whose traversal raises. Flavor-dependent by
    // Lua semantics: 5.2+ `ipairs` consults __index (the raise fires
    // and the pcall degrades to defaults with a note); LuaJIT/5.1
    // reads raw (the container is simply empty — no rules, no note).
    // Both flavors must complete cleanly with nothing thrown
    // through the pump.
    let mut s = editor();
    exec(
        &s,
        r#"
        pmacs.compile.rules = setmetatable({}, {
            __index = function() error("container boom") end,
        })
        "#,
    );
    compile_run(&s, "printf 'a.c:1:1: error: e\\n'", dir.path());
    let is_lua54: bool = eval(&s, "return _VERSION ~= 'Lua 5.1'");
    if is_lua54 {
        assert!(
            status(&s).contains("raised during traversal"),
            "degradation note under 5.2+ ipairs semantics; got: {}",
            status(&s)
        );
    }
    assert!(pump_until(&mut s, 10_000, |s| compilation_text(s)
        .contains("[compile exited")));
    if is_lua54 {
        assert_eq!(
            compile_errors(&s).len(),
            1,
            "built-in defaults still parse after container degradation"
        );
    } else {
        assert!(
            compile_errors(&s).is_empty(),
            "under raw-ipairs flavors the hostile container reads as \
             an (empty) rule table"
        );
    }
    assert!(
        errors_buffer(&s).is_empty(),
        "no spam: {}",
        errors_buffer(&s)
    );
}

#[test]
fn r2f2_infinite_capture_index_is_counted_malformed() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    exec(
        &s,
        r#"
        pmacs.compile.rules = {
            { pattern = "(z%.c):(%d+):", file = 1, line = 2, col = math.huge },
        }
        "#,
    );
    compile_run(&s, "printf 'z.c:2: e\\n'", dir.path());
    assert!(
        status(&s).contains("skipped 1 malformed"),
        "math.huge is not a capture index (floor(huge) == huge, so \
         integrality alone passes it); got: {}",
        status(&s)
    );
    assert!(pump_until(&mut s, 10_000, |s| compilation_text(s)
        .contains("[compile exited")));
}

#[test]
fn r2f3_shell_command_ignores_the_compile_rule_table() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    // Both degradation shapes at once: a non-table container would
    // warn, a raising container would abort — shell-command performs
    // no parsing and must see neither.
    exec(&s, "pmacs.compile.rules = 42");
    exec(
        &s,
        &format!(
            "pmacs.shell.command('echo shellok', {{ cwd = {:?} }})",
            dir.path().display().to_string()
        ),
    );
    assert!(
        !status(&s).contains("not a table"),
        "no compile-rule warning on a shell run; got: {}",
        status(&s)
    );
    assert!(
        pump_until(&mut s, 10_000, |s| named_text(s, "*shell-command*")
            .contains("[shell exited with code 0]")),
        "shell-command runs regardless of rule-table state"
    );
}

#[test]
fn r2f4_parser_finish_resets_for_a_fresh_stream() {
    // Lua-driven twin of the ansi.rs units (which live inside the
    // file a scripts/bite swap replaces): after finish(), a feed
    // must parse a NEW stream — not continue a pre-EOF escape, not
    // stay alt-screen-suppressed.
    let s = editor();
    let (after_csi, after_alt): (String, String) = eval(
        &s,
        r#"
        local function text_of(evs)
            local out = {}
            for _, ev in ipairs(evs) do
                if ev.kind == "text" then out[#out + 1] = ev.text end
            end
            return table.concat(out)
        end
        local p = pmacs.ansi.parser()
        p:feed("\27[3")   -- incomplete CSI at stream end
        p:finish()
        local a = text_of(p:feed("plain"))
        local q = pmacs.ansi.parser()
        q:feed("\27[?1049hhidden")  -- alt screen active at stream end
        q:finish()
        local b = text_of(q:feed("visible"))
        return a, b
        "#,
    );
    assert_eq!(
        after_csi, "plain",
        "post-finish feed must not continue the pre-EOF CSI"
    );
    assert_eq!(
        after_alt, "visible",
        "stream end must end alt-screen suppression"
    );
}

#[test]
fn r1f10_builtin_default_rules_survive_in_place_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = editor();
    // Corrupt an entry IN PLACE, then degrade the container: the
    // "built-in defaults" fallback must be a true copy, not an alias
    // of the mutated table.
    exec(
        &s,
        "pmacs.compile.rules[2] = 'junk'; pmacs.compile.rules = 42",
    );
    compile_and_finish(&mut s, "printf 'foo.c:7:2: warning: w\\n'", dir.path());
    assert_eq!(
        compile_errors(&s),
        vec![("foo.c".to_owned(), 6, 1, Some("warning".to_owned()))],
        "the gcc three-part rule from the TRUE defaults must match \
         (the aliased pre-fix table lost it and the two-part fallback \
         stored column 0)"
    );
}

// ---------------------------------------------------------------------------
// PR #113 round 3 — bite tests
// ---------------------------------------------------------------------------

#[test]
fn r3f1_cr_and_backspace_are_utf8_safe() {
    // Overwrites must consume whole existing codepoints in one
    // atomic replace, and backspace must step to the previous UTF-8
    // boundary. Pre-fix, é\rX left "X\xA9" (malformed) and é\bX left
    // "\xC3X"; under CRDT the mid-codepoint edit rejects and aborts
    // the pump (see the CRDT twin).
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        "uni.sh",
        concat!(
            "printf '\\303\\251\\rX\\n'\n", // é\rX  → X
            "printf 'X\\r\\303\\251\\n'\n", // X\ré  → é
            "printf '\\303\\251\\bX\\n'\n", // é\bX  → X
        ),
    );
    let mut s = editor();
    compile_and_finish(&mut s, &format!("sh {script}"), dir.path());
    let text = compilation_text(&s);
    assert!(
        text.contains("\nX\n\u{e9}\nX\n"),
        "each overwrite must yield exactly the replacing character, \
         valid UTF-8, no residue; buffer:\n{text:?}"
    );
    assert!(
        text.contains("[compile exited with code 0]"),
        "terminal event must survive the unicode batch:\n{text}"
    );
    assert!(
        errors_buffer(&s).is_empty(),
        "no pump aborts: {}",
        errors_buffer(&s)
    );
    assert!(
        pump_until(&mut s, 3_000, |s| process_count(s) == 0),
        "process record forgotten (cleanup ran)"
    );
}

#[test]
fn r3f2_parser_finish_emits_balancing_state_events() {
    // Consumers mirror parser state from the event stream alone: an
    // unclosed alt-screen enter must be balanced by an exit, and a
    // non-default running style by a default SetStyle — applied to
    // consumer state, not just observed as later text.
    let s = editor();
    let (alt_balanced, style_reset): (bool, bool) = eval(
        &s,
        r#"
        local p = pmacs.ansi.parser()
        p:feed("\27[31mred")
        p:feed("\27[?1049h")  -- enter alt screen, never exited
        local alt = true      -- consumer mirror of the enter
        local style = { fg = 1 }
        for _, ev in ipairs(p:finish()) do
            if ev.kind == "alt_screen_exit" then alt = false end
            if ev.kind == "set_style" then style = ev.style end
        end
        local style_is_default = style.fg == "default"
            and style.bg == "default" and not style.bold
        return alt == false, style_is_default
        "#,
    );
    assert!(
        alt_balanced,
        "finish must emit alt_screen_exit for an unclosed enter"
    );
    assert!(
        style_reset,
        "finish must emit a default set_style when the running style \
         was non-default"
    );
}

#[test]
fn r3f3_spec_fields_are_raw_reads_metatables_not_honored() {
    let s = editor();
    // A metatable that provides group = true: honoring it would be
    // silent spec-by-metatable; raw reads ignore it (the compile.lua
    // rawget posture), so the child must NOT lead its own group.
    let pid: i64 = eval(
        &s,
        r#"
        local spec = setmetatable(
            { label = "mt", command = "/bin/sh", args = { "-c", "sleep 30" } },
            { __index = function(_, k)
                if k == "group" then return true end
                return nil
            end })
        local id = pmacs.process.spawn(spec)
        for _, row in ipairs(pmacs.process.list()) do
            if row.state and row.state.pid then return row.state.pid end
        end
        return -1
        "#,
    );
    assert!(pid > 0, "metatable-backed spec must spawn");
    let out = std::process::Command::new("ps")
        .args(["-o", "pgid=", "-p", &pid.to_string()])
        .output()
        .expect("ps");
    let pgid: i64 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("pgid");
    assert_ne!(
        pgid, pid,
        "metatable-provided group=true must not be honored (raw reads)"
    );
    // A RAISING __index must not be silently absorbed either — with
    // raw reads it simply never fires; the spawn succeeds cleanly.
    let ok: bool = eval(
        &s,
        r#"
        local spec = setmetatable(
            { label = "mt2", command = "/bin/sh", args = { "-c", "exit 0" } },
            { __index = function() error("hostile spec metatable") end })
        local ok = pcall(pmacs.process.spawn, spec)
        return ok
        "#,
    );
    assert!(ok, "raw reads must not trip a raising __index");
    exec(
        &s,
        r"
        for _, row in ipairs(pmacs.process.list()) do
            pcall(pmacs.process.terminate, row.id)
        end
        ",
    );
}
