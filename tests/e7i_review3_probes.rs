//! E7i review round 3's probes against PR #309 at `db697a2`: the
//! behaviour fix round 2 added to the core (`2c3af58`, `34eb21d`,
//! `862b57f`, `b5b5d15`), at the edges its own witnesses did not choose.
//! Left on `e7i/review-3` for the fix round, as earlier rounds left theirs.
//!
//! Each row drives a real worker, the debug build's, through an
//! in-process editor, and reads the grid `paint_frame` paints and the
//! semantic frames the GPU frontend receives. Rows that pass at
//! `db697a2` settle a question the review asked; the one that fails names
//! a finding in its message.

#[path = "common/mod.rs"]
mod common;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use common::ready::{self, Probe};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::cell::{Cell, CellGrid, CellSize, Color, Glyph};
use pmacs::editor::EditorState;
use pmacs::protocol::{ByteRange, FrontendId, InstanceMessage};
use pmacs::semantic_render::SemanticRenderState;

/// The color the rows theme keywords with.
const MARK: Color = Color::Rgb(0x7b, 0x1f, 0xa2);
/// The colors the semantic-token row themes the fake server's `function`
/// and `namespace` tokens with; no grammar capture of the row's file
/// resolves to `namespace`.
const FUNC: Color = Color::Rgb(0x10, 0x80, 0x20);
const NS: Color = Color::Rgb(0x20, 0x40, 0xc0);

const ROWS: u32 = 30;
const COLS: u32 = 100;

/// One in-process editor at a time: the parse units' host is the
/// process's.
static ONE_EDITOR: Mutex<()> = Mutex::new(());

fn one_editor() -> MutexGuard<'static, ()> {
    ONE_EDITOR.lock().unwrap_or_else(PoisonError::into_inner)
}

fn say(line: &str) {
    let _ = writeln!(std::io::stderr(), "e7i review 3: {line}");
}

/// The debug worker beside the test's `pmacs`.
fn real_unit() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_pmacs"))
        .parent()
        .expect("target dir")
        .join("pmacs-parse-unit")
}

/// A wrapper that starts the real worker with its debug crash hook armed.
fn crashing_on(dir: &Path, language: &str, marker: &str) -> PathBuf {
    let unit = real_unit();
    assert!(unit.exists(), "the worker is built: {}", unit.display());
    let wrapper = dir.join("crashing-on-marker");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nPMACS_PARSE_UNIT_TEST_CRASH='{language}:{marker}' exec '{}' \"$@\"\n",
            unit.display()
        ),
    )
    .expect("wrapper");
    std::fs::set_permissions(
        &wrapper,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("chmod");
    wrapper
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn type_keys(state: &mut EditorState, text: &str) {
    for ch in text.chars() {
        let code = if ch == '\n' {
            KeyCode::Enter
        } else {
            KeyCode::Char(ch)
        };
        state.dispatch_key(FrontendId::LOCAL, key(code));
    }
}

fn eval(state: &EditorState, lua: &str) -> String {
    state
        .lua_host
        .lua()
        .load(lua)
        .eval::<String>()
        .unwrap_or_else(|e| format!("<error: {e}>"))
}

fn exec(state: &EditorState, lua: &str) {
    state.lua_host.lua().load(lua).exec().expect("lua");
}

fn report(state: &EditorState) -> String {
    eval(
        state,
        "local b = pmacs.window.buffer() local r = pmacs.parse._unit_report(b) \
         if not r then return 'none' end \
         return string.format('deaths=%d crashes=%d busy=%s held=%s pending=%d stopped=%s tree=%s unit=%s', \
           r.deaths, r.crashes, tostring(r.busy), tostring(r.held ~= nil), \
           pmacs.parse._pending_edits(b) or 0, tostring(r.stopped), \
           tostring(pmacs.parse.tree(b) ~= nil), (r.unit:gsub(' ', '_')))",
    )
}

fn field(report: &str, name: &str) -> String {
    report
        .split_whitespace()
        .find_map(|w| w.strip_prefix(&format!("{name}=")))
        .unwrap_or("")
        .to_owned()
}

fn paint_at(state: &EditorState, rows: u32, cols: u32) -> Vec<Cell> {
    let size = CellSize::new(rows, cols);
    let mut cells = vec![Cell::default(); (rows * cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: cols,
        size,
    };
    pmacs::editor::paint_frame(state, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);
    cells
}

fn paint(state: &EditorState) -> Vec<Cell> {
    paint_at(state, ROWS, COLS)
}

fn row_text_at(cells: &[Cell], cols: u32, row: u32) -> String {
    (0..cols)
        .map(|col| match cells[(row * cols + col) as usize].glyph {
            Glyph::Char(c) => c,
            _ => ' ',
        })
        .collect()
}

fn row_text(cells: &[Cell], row: u32) -> String {
    row_text_at(cells, COLS, row)
}

fn mode_line(cells: &[Cell]) -> String {
    row_text(cells, ROWS - 2).trim_end().to_owned()
}

/// Cells painted in keyword color whose glyph is not part of a keyword
/// of the row's text where it stands: what a parse's spans paint over
/// text that has moved under them.
fn stale_cells(cells: &[Cell]) -> usize {
    let mut stale = 0;
    for row in 0..ROWS - 2 {
        let text: Vec<char> = row_text(cells, row).chars().collect();
        for col in 0..COLS as usize {
            if cells[row as usize * COLS as usize + col].style.fg != MARK {
                continue;
            }
            let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
            if !word(text[col]) {
                stale += 1;
                continue;
            }
            let mut s = col;
            while s > 0 && word(text[s - 1]) {
                s -= 1;
            }
            let mut e = col;
            while e < text.len() && word(text[e]) {
                e += 1;
            }
            let w: String = text[s..e].iter().collect();
            if w != "fn" && w != "let" {
                stale += 1;
            }
        }
    }
    stale
}

fn marked_rows(cells: &[Cell]) -> usize {
    (0..ROWS - 2)
        .filter(|&row| (0..COLS).any(|col| cells[(row * COLS + col) as usize].style.fg == MARK))
        .count()
}

fn apply_style_spans(held: &mut Vec<(u64, u64, Color)>, msgs: &[InstanceMessage]) {
    for msg in msgs {
        let InstanceMessage::StyleSpans { full, segments, .. } = msg else {
            continue;
        };
        if *full {
            held.clear();
        }
        for segment in segments {
            let (lo, hi) = (segment.range.start, segment.range.end);
            let mut kept = Vec::new();
            for &(s, e, fg) in held.iter() {
                if e <= lo || s >= hi {
                    kept.push((s, e, fg));
                    continue;
                }
                if s < lo {
                    kept.push((s, lo, fg));
                }
                if e > hi {
                    kept.push((hi, e, fg));
                }
            }
            *held = kept;
            held.extend(
                segment
                    .spans
                    .iter()
                    .map(|span| (span.range.start, span.range.end, span.style.fg)),
            );
        }
    }
}

fn rust_text(n: usize) -> String {
    let mut text = String::new();
    for i in 0..n {
        let _ = write!(text, "fn f{i}() {{\n    let x = {i};\n}}\n");
    }
    text
}

fn open_with(state: &EditorState, path: &Path, unit: &Path) {
    exec(
        state,
        &format!(
            "pmacs.lsp.config = pmacs.lsp.config or {{}}\n\
             pmacs.config.set('syntax.isolation', 'process')\n\
             pmacs.config.set('syntax.parse-unit-path', {:?})\n\
             pmacs.theme.merge {{ keyword = {{ fg = {{ 0x7b, 0x1f, 0xa2 }} }} }}\n\
             pmacs.buffer.find_or_open({:?})",
            unit.display().to_string(),
            path.display().to_string()
        ),
    );
}

fn wait_parsed(state: &mut EditorState, what: &str) {
    ready::tick_until(state, what, Duration::from_mins(1), |s| {
        let r = report(s);
        if field(&r, "busy") == "false" && field(&r, "tree") == "true" {
            Probe::Ready(())
        } else {
            Probe::Pending(r)
        }
    });
}

fn editor_on(path: &Path, unit: &Path) -> EditorState {
    let mut state = EditorState::new_with_roots(&common::iso::roots());
    exec(&state, "pmacs.lsp.config = {}");
    open_with(&state, path, unit);
    wait_parsed(&mut state, "the first parse installed");
    state
}

fn one_tick(state: &mut EditorState) {
    state.tick_processes();
    state.tick_lsp();
    state.tick_async();
}

fn quiesce(state: &mut EditorState) {
    ready::tick_until(
        state,
        "the buffer's parses settled",
        Duration::from_secs(30),
        |s| {
            let r = report(s);
            if field(&r, "busy") == "false" && field(&r, "pending") == "0" {
                Probe::Ready(())
            } else {
                Probe::Pending(r)
            }
        },
    );
    for _ in 0..5 {
        one_tick(state);
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn errors_text(state: &EditorState) -> String {
    state.lua_host.errors_buffer_text()
}

/// Live `pmacs-parse-unit` processes whose parent is this test process
/// (a worker is the editor's child; the crash wrapper `exec`s it).
fn live_workers() -> Vec<u32> {
    let me = std::process::id();
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return out;
    };
    for entry in dir.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else {
            continue;
        };
        let comm = &stat[open + 1..close];
        let rest: Vec<&str> = stat[close + 1..].split_whitespace().collect();
        let (Some(state), Some(ppid)) = (rest.first(), rest.get(1)) else {
            continue;
        };
        if comm.starts_with("pmacs-parse-u") && *state != "Z" && *ppid == me.to_string() {
            out.push(pid);
        }
    }
    out
}

/// Workers started since `before` was taken. The units' host is the
/// process's, so the workers of an earlier row's editor (dropped, its
/// buffers' units not) are still alive in this process, and are not this
/// row's to count.
fn new_workers(before: &std::collections::HashSet<u32>) -> usize {
    live_workers()
        .into_iter()
        .filter(|pid| !before.contains(pid))
        .count()
}

/// Charge 1. `drop_current(false)` keeps the installed parse when its text
/// is still the buffer's, so a crash on a parse no edit requested keeps
/// colors that are still right. The question: does an edit made during
/// that crash's back-off paint the kept spans over moved text, since the
/// grid keys its cache on the bundle and not the source? Reached here: an
/// idle worker killed from outside (as an OOM killer would), then a buffer
/// switch, which dispatches a parse with no edit; the crash is found with
/// the text unmoved. Then three lines entered at the top inside the
/// back-off. The edit's own dispatch is refused at once ("held"), and its
/// settle drops the parse because the text has moved; the stale paint
/// lasts until that settle, one tick, where an ordinary edit's lasts until
/// its parse returns. The back-off then ends in a parse of its own that
/// succeeds in a fresh worker: the colors come back, the mark goes, the
/// streak resets (charge 2).
#[test]
#[allow(clippy::too_many_lines)] // one user's path, frame by frame
fn e7i_review3_an_edit_in_the_back_off_of_a_crash_over_unmoved_text() {
    let _one = one_editor();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("a.rs");
    std::fs::write(&path, rust_text(20)).expect("a.rs");
    let other = dir.path().join("notes.txt");
    std::fs::write(&other, "plain\n").expect("notes.txt");
    let mut state = editor_on(&path, &real_unit());
    let buffer = state.core.borrow().active_window().buffer_id;
    let mut semantic = SemanticRenderState::new(FrontendId::LOCAL);
    semantic.set_viewport(
        buffer,
        ByteRange {
            start: 0,
            end: 1 << 20,
        },
        0,
    );
    let mut held = Vec::new();
    apply_style_spans(&mut held, &semantic.render_frame(&state));
    let cells = paint(&state);
    assert!(marked_rows(&cells) >= 10, "control: keywords painted");
    assert_eq!(stale_cells(&cells), 0, "control: nothing stale");

    // The baseline: an ordinary edit in a healthy buffer, painted before
    // its parse returns, and the ticks until no cell is stale.
    exec(&state, "pmacs.editor.goto_byte(0)");
    type_keys(&mut state, "\n\n\n");
    let base_before_tick = stale_cells(&paint(&state));
    let base_start = Instant::now();
    let mut base_ticks = 0;
    while stale_cells(&paint(&state)) > 0 && base_start.elapsed() < Duration::from_secs(10) {
        one_tick(&mut state);
        base_ticks += 1;
        std::thread::sleep(Duration::from_millis(1));
    }
    let base_ms = base_start.elapsed().as_millis();
    quiesce(&mut state);

    // The idle worker killed from outside; a switch away and back
    // dispatches a parse with no edit, which finds the crash.
    let r = report(&state);
    let pid = field(&r, "unit").trim_start_matches("pid_").to_owned();
    assert!(!pid.is_empty(), "the worker's pid: {r}");
    let killed = std::process::Command::new("kill")
        .args(["-KILL", &pid])
        .status()
        .expect("kill");
    assert!(killed.success(), "killed the idle worker {pid}");
    std::thread::sleep(Duration::from_millis(200));
    exec(
        &state,
        &format!(
            "pmacs.window.switch_buffer(pmacs.buffer.find_or_open({:?}))\n\
             pmacs.window.switch_buffer(pmacs.buffer.find_or_open({:?}))",
            other.display().to_string(),
            path.display().to_string()
        ),
    );
    let crash_seen = Instant::now();
    // The slot counts the crash on the job's thread; the settle that
    // installs the mark and the notice comes at a later tick.
    ready::tick_until(&mut state, "the crash told", Duration::from_secs(30), |s| {
        if errors_text(s).contains("crashed") {
            Probe::Ready(())
        } else {
            Probe::Pending(report(s))
        }
    });
    let cells = paint(&state);
    apply_style_spans(&mut held, &semantic.render_frame(&state));
    let at_crash = report(&state);
    say(&format!(
        "crash over unmoved text: {at_crash}; mode line {:?}; marked rows {}; stale {}",
        mode_line(&cells),
        marked_rows(&cells),
        stale_cells(&cells)
    ));
    assert_eq!(
        field(&at_crash, "tree"),
        "true",
        "a crash over unmoved text keeps its parse"
    );
    assert!(
        marked_rows(&cells) >= 10 && stale_cells(&cells) == 0,
        "the kept colors are right: the text has not moved"
    );
    assert!(
        mode_line(&cells).contains("parse:crashed"),
        "the mark stands: {:?}",
        mode_line(&cells)
    );

    // The edit inside the back-off.
    exec(&state, "pmacs.editor.goto_byte(0)");
    type_keys(&mut state, "\n\n\n");
    let in_backoff_ms = crash_seen.elapsed().as_millis();
    let corner_before_tick = stale_cells(&paint(&state));
    let corner_start = Instant::now();
    let mut corner_ticks = 0;
    while stale_cells(&paint(&state)) > 0 && corner_start.elapsed() < Duration::from_secs(10) {
        one_tick(&mut state);
        corner_ticks += 1;
        std::thread::sleep(Duration::from_millis(1));
    }
    let corner_ms = corner_start.elapsed().as_millis();
    let after_edit = report(&state);
    apply_style_spans(&mut held, &semantic.render_frame(&state));
    let cells = paint(&state);
    say(&format!(
        "baseline edit: {base_before_tick} stale cells before a tick, clean after {base_ticks} ticks, {base_ms} ms; \
         edit {in_backoff_ms} ms into the back-off: {corner_before_tick} stale before a tick, clean after {corner_ticks} ticks, {corner_ms} ms; {after_edit}"
    ));
    assert_eq!(
        field(&after_edit, "held"),
        "true",
        "the edit landed inside the back-off: {after_edit}"
    );
    assert_eq!(
        stale_cells(&cells),
        0,
        "no kept span outlives the edit's held parse"
    );
    assert!(
        corner_ticks <= base_ticks.max(2),
        "the corner's stale window ({corner_ticks} ticks) is no longer than an ordinary edit's ({base_ticks})"
    );
    assert!(
        !held.iter().any(|&(_, _, fg)| fg == MARK),
        "the GPU frontend holds no keyword span after the edit"
    );

    // The back-off ends in a parse of its own, in a fresh worker that
    // does not crash: the colors return over the moved text.
    let retried = ready::tick_until(
        &mut state,
        "the retry installs",
        Duration::from_secs(10),
        |s| {
            let r = report(s);
            if field(&r, "tree") == "true" && field(&r, "crashes") == "0" {
                Probe::Ready(r)
            } else {
                Probe::Pending(r)
            }
        },
    );
    let retry_ms = crash_seen.elapsed().as_millis();
    quiesce(&mut state);
    let cells = paint(&state);
    say(&format!(
        "the retry {retry_ms} ms after the crash: {retried}; mode line {:?}; marked rows {}",
        mode_line(&cells),
        marked_rows(&cells)
    ));
    assert!(
        marked_rows(&cells) >= 10 && stale_cells(&cells) == 0,
        "the retry repaints the moved text"
    );
    assert!(
        !mode_line(&cells).contains("parse:"),
        "a retry that installs clears the mark: {:?}",
        mode_line(&cells)
    );
}

/// Charges 2 and 3. A buffer killed inside its back-off: the retry finds
/// no view and starts no worker, nothing more is told, and the file
/// opened again is a new buffer (ids come from a process-wide counter and
/// are never reused) with no mark. `syntax.lua` registers no
/// `on_removed` for its crash tables, as `lsp.lua` does for
/// `failed_attachments`; the dead buffer's entries stay for the session,
/// unreachable.
#[test]
fn e7i_review3_a_buffer_killed_in_its_back_off_starts_no_worker() {
    let _one = one_editor();
    let dir = tempfile::tempdir().expect("tempdir");
    let unit = crashing_on(dir.path(), "rust", "CRASHME");
    let path = dir.path().join("a.rs");
    std::fs::write(&path, rust_text(5)).expect("a.rs");
    let before: std::collections::HashSet<u32> = live_workers().into_iter().collect();
    let mut state = editor_on(&path, &unit);
    let old_key = eval(&state, "return tostring(pmacs.window.buffer())");
    type_keys(&mut state, "CRASHME");
    ready::tick_until(&mut state, "the crash told", Duration::from_secs(30), |s| {
        if errors_text(s).contains("crashed") {
            Probe::Ready(())
        } else {
            Probe::Pending(report(s))
        }
    });
    // The window moves to a file with no grammar first, so no worker of
    // another buffer is counted below; then a.rs is killed.
    let notes = dir.path().join("notes.txt");
    std::fs::write(&notes, "plain\n").expect("notes.txt");
    exec(
        &state,
        &format!(
            "local doomed = pmacs.window.buffer()\n\
             pmacs.window.switch_buffer(pmacs.buffer.find_or_open({:?}))\n\
             pmacs.buffer.kill(doomed)",
            notes.display().to_string()
        ),
    );
    let lines_at_kill = errors_text(&state).lines().count();
    let started = Instant::now();
    let mut most = 0;
    while started.elapsed() < Duration::from_millis(3500) {
        one_tick(&mut state);
        most = most.max(new_workers(&before));
        std::thread::sleep(Duration::from_millis(10));
    }
    let errors = errors_text(&state);
    let new_lines: Vec<&str> = errors.lines().skip(lines_at_kill).collect();
    say(&format!(
        "killed in the back-off: most live workers over 3.5 s {most}; new *errors* lines {new_lines:?}"
    ));
    assert_eq!(most, 0, "no worker starts for a killed buffer");
    assert!(new_lines.is_empty(), "nothing more is told: {new_lines:?}");

    exec(
        &state,
        &format!(
            "pmacs.window.switch_buffer(pmacs.buffer.find_or_open({:?}))",
            path.display().to_string()
        ),
    );
    wait_parsed(&mut state, "the reopened file parsed");
    quiesce(&mut state);
    let new_key = eval(&state, "return tostring(pmacs.window.buffer())");
    let cells = paint(&state);
    say(&format!(
        "reopened: {old_key} -> {new_key}; mode line {:?}",
        mode_line(&cells)
    ));
    assert_ne!(old_key, new_key, "the reopened file is a new buffer id");
    assert!(
        !mode_line(&cells).contains("parse:"),
        "the reopened buffer has no mark: {:?}",
        mode_line(&cells)
    );
    assert!(marked_rows(&cells) >= 3, "and is highlighted");
}

/// Charge 3. The mark in narrow windows, beside a stand-in for the LSP
/// label at the LSP segment's priority and the unread count, and its
/// absence from buffers with no grammar. Right segments are laid out
/// priority-ascending and overflow to the left, so the activity
/// indicator (-10) goes first, then the LSP label (0), then the mark (5),
/// then the unread count (10).
#[test]
#[allow(clippy::too_many_lines)]
fn e7i_review3_the_mark_in_narrow_windows_and_never_on_a_grammarless_buffer() {
    let _one = one_editor();
    let dir = tempfile::tempdir().expect("tempdir");
    let unit = crashing_on(dir.path(), "rust", "CRASHME");
    let path = dir.path().join("a.rs");
    std::fs::write(&path, rust_text(5)).expect("a.rs");
    let notes = dir.path().join("notes.txt");
    std::fs::write(&notes, "plain words\n").expect("notes.txt");
    let mut state = editor_on(&path, &unit);
    type_keys(&mut state, "CRASHME");
    ready::tick_until(
        &mut state,
        "the stop, with no edit after the crash's",
        Duration::from_secs(30),
        |s| {
            let r = report(s);
            if field(&r, "stopped") == "true" && field(&r, "busy") == "false" {
                Probe::Ready(())
            } else {
                Probe::Pending(r)
            }
        },
    );
    quiesce(&mut state);
    exec(
        &state,
        "pmacs.statusline.register { name = 'r3-lsp-stand-in', side = 'right', priority = 0, \
           fn = function(ctx) return 'LSP:ready' end }",
    );
    let mut table = String::new();
    let mut mark_whole_down_to = 0;
    let mut lsp_whole_down_to = 0;
    let mut mark_cut_while_lsp_whole = false;
    for cols in [
        120u32, 100, 80, 60, 50, 44, 40, 36, 34, 32, 30, 28, 26, 24, 22, 20,
    ] {
        let cells = paint_at(&state, ROWS, cols);
        let line = row_text_at(&cells, cols, ROWS - 2);
        let mark = line.contains("parse:stopped");
        let lsp = line.contains("LSP:ready");
        if mark {
            mark_whole_down_to = cols;
        }
        if lsp {
            lsp_whole_down_to = cols;
        }
        if lsp && !mark {
            mark_cut_while_lsp_whole = true;
        }
        let _ = writeln!(table, "{cols:>4} |{}|", line.trim_end());
    }
    say(&format!(
        "the mode line by width, the mark whole down to {mark_whole_down_to} columns, the LSP label down to {lsp_whole_down_to}:\n{table}"
    ));
    assert!(
        !mark_cut_while_lsp_whole,
        "the LSP label is never whole where the mark is cut"
    );
    assert!(
        mark_whole_down_to <= 40,
        "the mark survives a half of an 80-column terminal: whole down to {mark_whole_down_to}"
    );

    // Three windows stacked: the stopped buffer, *errors*, and a file with
    // no grammar. Only the first carries the mark.
    exec(
        &state,
        &format!(
            "pmacs.window.split_horizontal()\n\
             for _, x in ipairs(pmacs.buffer.list()) do \
               if x:name() == '*errors*' then pmacs.window.switch_buffer(x) end \
             end\n\
             pmacs.window.split_horizontal()\n\
             pmacs.window.switch_buffer(pmacs.buffer.find_or_open({:?}))",
            notes.display().to_string()
        ),
    );
    for _ in 0..5 {
        one_tick(&mut state);
    }
    let (rows, wide) = (45u32, 200u32);
    let cells = paint_at(&state, rows, wide);
    let modes: Vec<String> = (0..rows)
        .map(|row| row_text_at(&cells, wide, row).trim_end().to_owned())
        .filter(|t| t.contains(" L") && t.contains(":C"))
        .collect();
    say(&format!("three windows' mode lines: {modes:#?}"));
    let rs = modes
        .iter()
        .find(|t| t.contains("a.rs"))
        .expect("a.rs's mode line");
    let er = modes
        .iter()
        .find(|t| t.contains("*errors*"))
        .expect("*errors*'s mode line");
    let nt = modes
        .iter()
        .find(|t| t.contains("notes.txt"))
        .expect("notes.txt's mode line");
    assert!(rs.contains("parse:stopped"), "the stopped buffer: {rs:?}");
    assert!(!er.contains("parse:"), "*errors* carries no mark: {er:?}");
    assert!(
        !nt.contains("parse:"),
        "a file with no grammar carries none: {nt:?}"
    );
}

/// Charge 4. A dropped parse with `ui.semantic-styling` on ships the
/// server's tokens alone: no grammar span, each token once, and placed by
/// the buffer's current text, not by the dropped parse's bytes. The fake
/// server's tokens are read from the store and placed by the current
/// text: its full answer has a `function` at line 0, columns 0-4, a
/// `variable` (unstyled) and a `namespace` at line 2, columns 2-9, and
/// its delta answer after an edit makes the third a `function` at line 3,
/// columns 0-9.
#[test]
#[allow(clippy::too_many_lines)]
fn e7i_review3_a_dropped_parse_ships_the_server_s_tokens_alone() {
    let _one = one_editor();
    let dir = tempfile::tempdir().expect("tempdir");
    let unit = crashing_on(dir.path(), "rust", "CRASHME");
    let path = dir.path().join("a.rs");
    let text = "fn alpha() {}\nfn beta() {}\nfn gamma_delta() {}\nfn epsilon() {}\n";
    std::fs::write(&path, text).expect("a.rs");
    let fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp");
    let mut state = EditorState::new_with_roots(&common::iso::roots());
    exec(
        &state,
        &format!(
            "pmacs.lsp.config = {{}}\n\
             pmacs.lsp.config.rust = {{ command = {fake:?} }}\n\
             pmacs.theme.merge {{ ['function'] = {{ fg = {{ 0x10, 0x80, 0x20 }} }}, \
               namespace = {{ fg = {{ 0x20, 0x40, 0xc0 }} }} }}"
        ),
    );
    open_with(&state, &path, &unit);
    exec(&state, "pmacs.editor.goto_byte(0)");
    let buffer = state.core.borrow().active_window().buffer_id;
    let mut semantic = SemanticRenderState::new(FrontendId::LOCAL);
    semantic.set_viewport(
        buffer,
        ByteRange {
            start: 0,
            end: 1 << 20,
        },
        0,
    );
    let mut held = Vec::new();
    let mut frames = |state: &mut EditorState, held: &mut Vec<(u64, u64, Color)>| {
        one_tick(state);
        apply_style_spans(held, &semantic.render_frame(state));
    };
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(30) {
        frames(&mut state, &mut held);
        let has = |c: Color| held.iter().any(|&(_, _, fg)| fg == c);
        if has(MARK) && has(NS) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let before = held.clone();
    assert!(
        before.iter().any(|&(_, _, fg)| fg == MARK) && before.iter().any(|&(_, _, fg)| fg == NS),
        "control: grammar and server spans both reach the frontend: {before:?} {}",
        report(&state)
    );

    type_keys(&mut state, "CRASHME");
    // The server's styled tokens as the store holds them, placed by the
    // buffer's current text: what the frontend must hold once the parse
    // is dropped, and nothing else.
    let expected = |state: &EditorState| -> Vec<(u64, u64, Color)> {
        let raw = eval(
            state,
            "local rec = pmacs.lsp.active_attachment() \
             if not rec then return '' end \
             local legend = pmacs.lsp.capabilities(rec.server).semanticTokensProvider.legend.tokenTypes \
             local out = {} \
             for _, t in ipairs(pmacs.semantic_tokens.tokens(rec.server, rec.uri) or {}) do \
               out[#out + 1] = t.line .. ':' .. t.start .. ':' .. t.length .. ':' .. tostring(legend[t.token_type + 1]) \
             end \
             return table.concat(out, ' ')",
        );
        let text = eval(
            state,
            "local b = pmacs.window.buffer() return b:slice(0, b:len())",
        );
        let starts: Vec<u64> = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i as u64 + 1))
            .collect();
        let mut out = Vec::new();
        for tok in raw.split_whitespace() {
            let f: Vec<&str> = tok.split(':').collect();
            let (line, col, len): (usize, u64, u64) = (
                f[0].parse().unwrap(),
                f[1].parse().unwrap(),
                f[2].parse().unwrap(),
            );
            let color = match f[3] {
                "function" => FUNC,
                "namespace" => NS,
                _ => continue,
            };
            let at = starts[line] + col;
            out.push((at, at + len, color));
        }
        out.sort_by_key(|&(s, e, _)| (s, e));
        out
    };
    let started = Instant::now();
    let mut dropped_seen = false;
    let mut want = Vec::new();
    while started.elapsed() < Duration::from_secs(30) {
        frames(&mut state, &mut held);
        let r = report(&state);
        if field(&r, "crashes") != "0" && field(&r, "tree") == "false" {
            dropped_seen = true;
        }
        held.sort_by_key(|&(s, e, _)| (s, e));
        want = expected(&state);
        if dropped_seen && !want.is_empty() && held == want {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let cells = paint(&state);
    let grid_mark = cells.iter().filter(|c| c.style.fg == MARK).count();
    let grid_tok = cells
        .iter()
        .filter(|c| c.style.fg == FUNC || c.style.fg == NS)
        .count();
    let want_cells: u64 = want.iter().map(|&(s, e, _)| e - s).sum();
    say(&format!(
        "after the crash: {}; the frontend holds {held:?}; the server's tokens by the current text {want:?}; grid cells keyword {grid_mark}, token {grid_tok} of {want_cells}",
        report(&state)
    ));
    assert!(dropped_seen, "the parse was dropped");
    assert!(
        !held.iter().any(|&(_, _, fg)| fg == MARK),
        "no grammar span reaches the frontend"
    );
    assert_eq!(
        held, want,
        "the frontend holds exactly the server's tokens, each once, placed by the current text"
    );
    assert_eq!(grid_mark, 0, "the grid paints no grammar span either");
    assert_eq!(
        grid_tok as u64, want_cells,
        "the grid paints the same tokens, cell for cell"
    );
}

/// Charge 5, `862b57f`. The worker writes "parsing an injected layer of
/// X" as each layer starts and "injected layers parsed" after
/// `run_parse_observed` returns, and the root grammar's
/// `tree_sitter::Parser` lives until that function returns: read alone,
/// a root scanner dying at the parser's teardown would be told under the
/// last layer's grammar. Run, it is not: tree-sitter 0.27 destroys a
/// parser's external scanner at the end of every parse (`ts_parser_parse`
/// exits through `ts_parser_reset`, which calls the scanner's `destroy`
/// and clears the payload), so the root grammar's scanner runs nothing
/// after the root parse, before the first layer's line. The planted
/// worker (`probes/e7i-review3/plant-md-destroy.patch`: `abort()` in
/// tree-sitter-markdown's `external_scanner_destroy`, named by
/// `PMACS_R3_PLANTED_WORKER`) writes no layer line before it dies, and the
/// notice names markdown. Kept as a witness for that order; without the
/// planted worker the row says so and passes.
#[test]
fn e7i_review3_a_root_scanner_dying_at_teardown_is_not_blamed_on_a_layer() {
    let Ok(planted) = std::env::var("PMACS_R3_PLANTED_WORKER") else {
        say("PMACS_R3_PLANTED_WORKER unset: the teardown row did not run");
        return;
    };
    let _one = one_editor();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("README.md");
    std::fs::write(
        &path,
        "# Deploy\n\nRun it:\n\n```bash\necho hi\n```\n\nThen wait.\n",
    )
    .expect("README.md");
    let mut state = EditorState::new_with_roots(&common::iso::roots());
    exec(&state, "pmacs.lsp.config = {}");
    open_with(&state, &path, Path::new(&planted));
    let errors = ready::tick_until(&mut state, "the crash told", Duration::from_secs(30), |s| {
        let errors = errors_text(s);
        if errors.contains("crashed") {
            Probe::Ready(errors)
        } else {
            Probe::Pending(report(s))
        }
    });
    let layer = eval(
        &state,
        "return pmacs.parse._unit_report(pmacs.window.buffer()).crash_layer",
    );
    say(&format!(
        "markdown's own scanner aborting at teardown: layer {layer:?}; {}",
        errors.trim()
    ));
    assert_eq!(
        layer, "",
        "the crash was markdown's own scanner, after its layers; the notice blames the last injected layer: {errors}"
    );
}

/// Charge 2. Twelve buffers whose workers crash in the same tick: each
/// retry is keyed by its buffer, so they come due together and start
/// twelve workers at once, each still under the same spawn path (its
/// limits, the recycle threshold, the total), and each streak runs to its
/// own stop at the right exponent: three crashes a buffer, back-offs of
/// 1,000 then 2,000 ms, then nothing more. Nothing keeps the editor busy
/// after the last stop.
#[test]
#[allow(clippy::too_many_lines)]
fn e7i_review3_twelve_buffers_back_off_together_and_each_stops() {
    let _one = one_editor();
    let dir = tempfile::tempdir().expect("tempdir");
    let unit = crashing_on(dir.path(), "rust", "CRASHME");
    let paths: Vec<PathBuf> = (0..12)
        .map(|i| {
            let p = dir.path().join(format!("m{i:02}.rs"));
            std::fs::write(&p, rust_text(3)).expect("file");
            p
        })
        .collect();
    let before: std::collections::HashSet<u32> = live_workers().into_iter().collect();
    let mut state = EditorState::new_with_roots(&common::iso::roots());
    exec(&state, "pmacs.lsp.config = {}");
    for p in &paths {
        open_with(&state, p, &unit);
        exec(
            &state,
            &format!(
                "pmacs.window.switch_buffer(pmacs.buffer.find_or_open({:?}))",
                p.display().to_string()
            ),
        );
        wait_parsed(&mut state, "each file parsed");
    }
    let names: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    let lua_list = names
        .iter()
        .map(|n| format!("{n:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    // Every buffer takes the marker and is dispatched in one chunk.
    exec(
        &state,
        &format!(
            "R3_BUFS = {{}}\n\
             for _, n in ipairs({{ {lua_list} }}) do \
               local b = pmacs.buffer.find_or_open(n) \
               b:insert(0, 'CRASHME') \
               R3_BUFS[#R3_BUFS + 1] = b \
             end\n\
             for _, b in ipairs(R3_BUFS) do pmacs.parse._dispatch(b, 'rust') end"
        ),
    );
    let started = Instant::now();
    let mut most_live = 0;
    let mut crashes_seen: Vec<Vec<(u64, u64)>> = vec![Vec::new(); 12];
    let all_stopped = loop {
        one_tick(&mut state);
        most_live = most_live.max(new_workers(&before));
        let rows = eval(
            &state,
            "local out = {} \
             for _, b in ipairs(R3_BUFS) do \
               local r = pmacs.parse._unit_report(b) \
               out[#out + 1] = r and string.format('%d:%d:%d:%s', r.crashes, r.crashed_at_ms, r.backoff_ms, tostring(r.stopped)) or 'none' \
             end \
             return table.concat(out, ' ')",
        );
        let mut stopped = 0;
        for (i, row) in rows.split_whitespace().enumerate() {
            let f: Vec<&str> = row.split(':').collect();
            if f.len() < 4 {
                continue;
            }
            let (n, at, backoff): (u64, u64, u64) = (
                f[0].parse().unwrap_or(0),
                f[1].parse().unwrap_or(0),
                f[2].parse().unwrap_or(0),
            );
            if n > 0 && crashes_seen[i].last().map(|&(a, _)| a) != Some(at) {
                crashes_seen[i].push((at, backoff));
            }
            if f[3] == "true" {
                stopped += 1;
            }
        }
        if stopped == 12 {
            break true;
        }
        if started.elapsed() > Duration::from_secs(30) {
            break false;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let to_stop_ms = started.elapsed().as_millis();
    // The slots say "stopped" on the jobs' threads a tick before the last
    // stop is told in Lua: let the settles land, then a quiet second and a
    // half: no worker, no crash, no notice.
    for _ in 0..20 {
        one_tick(&mut state);
        std::thread::sleep(Duration::from_millis(10));
    }
    let notices_at_stop = errors_text(&state).matches("[syntax]").count();
    let quiet = Instant::now();
    let mut live_after = 0;
    while quiet.elapsed() < Duration::from_millis(1500) {
        one_tick(&mut state);
        live_after = live_after.max(new_workers(&before));
        std::thread::sleep(Duration::from_millis(10));
    }
    let notices = errors_text(&state).matches("[syntax]").count();
    let spread = |k: usize| -> (u64, u64) {
        let ats: Vec<u64> = crashes_seen
            .iter()
            .filter_map(|c| c.get(k).map(|&(a, _)| a))
            .collect();
        (
            ats.iter().copied().min().unwrap_or(0),
            ats.iter().copied().max().unwrap_or(0),
        )
    };
    let (s0, s1, s2) = (spread(0), spread(1), spread(2));
    say(&format!(
        "twelve together: all stopped {all_stopped} in {to_stop_ms} ms; most live workers {most_live}, after the stops {live_after}; \
         crash instants (min, max) ms: first {s0:?}, second {s1:?}, third {s2:?}; per buffer {crashes_seen:?}; \
         [syntax] notices {notices_at_stop} at the stops, {notices} a second and a half later"
    ));
    assert!(all_stopped, "every buffer reaches its stop with no edit");
    for (i, c) in crashes_seen.iter().enumerate() {
        assert_eq!(c.len(), 3, "buffer {i} crashed three times: {c:?}");
        assert_eq!(
            c.iter().map(|&(_, b)| b).collect::<Vec<_>>(),
            vec![1000, 2000, 0],
            "buffer {i}'s back-offs: 1,000, 2,000, then the stop"
        );
        assert!(
            c[1].0 >= c[0].0 + 1000 && c[2].0 >= c[1].0 + 2000,
            "buffer {i} waited out each back-off: {c:?}"
        );
    }
    assert_eq!(live_after, 0, "no worker after the stops");
    assert_eq!(
        notices_at_stop, 24,
        "each buffer told twice: its first crash and its stop"
    );
    assert_eq!(
        notices, notices_at_stop,
        "nothing more is told after the stops"
    );
}

/// Charge 5, `862b57f`, the sink. `death()` reads the worker's stderr
/// sink after `wait()`, and the thread that fills the sink is never
/// joined, so the layer line the worker wrote just before it died can be
/// missing from what the editor reads (the notice then names the
/// buffer's grammar). The debug hook aborts right after the layer line,
/// the narrowest window there is. Forty fresh buffers, one crash each;
/// `PMACS_R3_SINK_ROUNDS` sets the count. The row reports how many
/// crashes were told without their layer and fails on any.
#[test]
fn e7i_review3_a_crashed_layer_s_name_reaches_the_editor_every_time() {
    let _one = one_editor();
    let rounds: usize = std::env::var("PMACS_R3_SINK_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let dir = tempfile::tempdir().expect("tempdir");
    let unit = crashing_on(dir.path(), "bash", "CRASHME");
    let mut state = EditorState::new_with_roots(&common::iso::roots());
    exec(&state, "pmacs.lsp.config = {}");
    let mut missing = Vec::new();
    for i in 0..rounds {
        let path = dir.path().join(format!("r{i:03}.md"));
        std::fs::write(&path, "# T\n\n```bash\necho CRASHME\n```\n").expect("md");
        open_with(&state, &path, &unit);
        exec(
            &state,
            &format!(
                "pmacs.window.switch_buffer(pmacs.buffer.find_or_open({:?}))",
                path.display().to_string()
            ),
        );
        let layer = ready::tick_until(&mut state, "the crash", Duration::from_secs(30), |s| {
            let r = eval(
                s,
                "local r = pmacs.parse._unit_report(pmacs.window.buffer()) \
                 if r and r.crashes > 0 then return 'L' .. r.crash_layer end return ''",
            );
            match r.strip_prefix('L') {
                Some(layer) => Probe::Ready(layer.to_owned()),
                None => Probe::Pending(report(s)),
            }
        });
        if layer != "bash" {
            missing.push((i, layer));
        }
    }
    say(&format!(
        "{rounds} crashes in a bash layer: told without their layer {}: {missing:?}",
        missing.len()
    ));
    assert!(
        missing.is_empty(),
        "every crash names its layer: {missing:?}"
    );
}
