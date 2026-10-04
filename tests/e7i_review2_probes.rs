//! E7i review round 2's findings, as fix round 2's witnesses against PR
//! #309: the states a user reaches after a buffer's parse worker crashed,
//! which the reviewer drove through the TUI with a crash planted in bash's
//! scanner. Each row drives a real worker, the debug build's, which a
//! wrapper starts with `PMACS_PARSE_UNIT_TEST_CRASH` naming a grammar and a
//! marker: the worker aborts as it starts parsing a layer of that grammar
//! in a text that holds the marker, so the buffer is highlighted until an
//! edit brings the marker in, as a grammar whose C crashes on some bytes.
//! Rows read the grid `paint_frame` paints, the TUI's own frame, and the
//! semantic frames the GPU frontend receives.

#[path = "common/mod.rs"]
mod common;

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::ready::{self, Probe};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::cell::{Cell, CellGrid, CellSize, Color, Glyph};
use pmacs::editor::EditorState;
use pmacs::protocol::{ByteRange, FrontendId, InstanceMessage};
use pmacs::semantic_render::SemanticRenderState;

/// The color the rows theme keywords with, so a keyword's span is told
/// from every other style on screen.
const MARK: Color = Color::Rgb(0x7b, 0x1f, 0xa2);

const ROWS: u32 = 30;
const COLS: u32 = 100;

fn say(line: &str) {
    let _ = writeln!(std::io::stderr(), "e7i review 2: {line}");
}

/// The debug worker beside the test's `pmacs`.
fn real_unit() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_pmacs"))
        .parent()
        .expect("target dir")
        .join("pmacs-parse-unit")
}

/// A wrapper that starts the real worker with its crash hook armed: it
/// aborts as it starts parsing a `language` layer of a text holding
/// `marker`.
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

/// Type `text` as keystrokes, `\n` as Enter.
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

/// The open file's unit report as `deaths=… crashes=… busy=… held=…`.
fn report(state: &EditorState) -> String {
    eval(
        state,
        "local r = pmacs.parse._unit_report(pmacs.window.buffer()) \
         if not r then return 'none' end \
         return string.format('deaths=%d crashes=%d busy=%s held=%s pending=%d', \
           r.deaths, r.crashes, tostring(r.busy), tostring(r.held ~= nil), \
           pmacs.parse._pending_edits(pmacs.window.buffer()) or 0)",
    )
}

fn field(report: &str, name: &str) -> String {
    report
        .split_whitespace()
        .find_map(|w| w.strip_prefix(&format!("{name}=")))
        .unwrap_or("")
        .to_owned()
}

/// Paint one grid frame, as the TUI does.
fn paint(state: &EditorState) -> Vec<Cell> {
    let size = CellSize::new(ROWS, COLS);
    let mut cells = vec![Cell::default(); (ROWS * COLS) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: COLS,
        size,
    };
    pmacs::editor::paint_frame(state, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);
    cells
}

fn row_text(cells: &[Cell], row: u32) -> String {
    (0..COLS)
        .map(|col| match cells[(row * COLS + col) as usize].glyph {
            Glyph::Char(c) => c,
            _ => ' ',
        })
        .collect()
}

/// The rows painted in keyword color, with their text.
fn marked_rows(cells: &[Cell]) -> Vec<String> {
    (0..ROWS)
        .filter(|&row| (0..COLS).any(|col| cells[(row * COLS + col) as usize].style.fg == MARK))
        .map(|row| format!("{row}: {}", row_text(cells, row).trim_end()))
        .collect()
}

/// The mode line: the row that carries the buffer's mode segment.
fn mode_line(cells: &[Cell]) -> String {
    (0..ROWS)
        .map(|row| row_text(cells, row))
        .find(|t| t.contains("(rust)"))
        .unwrap_or_default()
        .trim_end()
        .to_owned()
}

/// What a frontend holds of the buffer's styling after applying `msgs`
/// over `held`: a full frame replaces it, a segment clears its range and
/// repaints it.
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

/// A Rust file of `n` functions, a keyword at the head of every line.
fn rust_text(n: usize) -> String {
    let mut text = String::new();
    for i in 0..n {
        let _ = write!(text, "fn f{i}() {{\n    let x = {i};\n}}\n");
    }
    text
}

/// An editor showing `path` with the given worker, keywords in `MARK`,
/// after its first parse installed.
fn editor_on(path: &Path, unit: &Path) -> EditorState {
    let mut state = EditorState::new_with_roots(&common::iso::roots());
    exec(
        &state,
        &format!(
            "pmacs.lsp.config = {{}}\n\
             pmacs.config.set('syntax.isolation', 'process')\n\
             pmacs.config.set('syntax.parse-unit-path', {:?})\n\
             pmacs.theme.merge {{ keyword = {{ fg = {{ 0x7b, 0x1f, 0xa2 }} }} }}\n\
             pmacs.buffer.find_or_open({:?})",
            unit.display().to_string(),
            path.display().to_string()
        ),
    );
    ready::tick_until(
        &mut state,
        "the first parse installed",
        Duration::from_mins(1),
        |s| {
            let seen = eval(
                s,
                "local b = pmacs.window.buffer() local r = pmacs.parse._unit_report(b) \
                 return tostring(r ~= nil and not r.busy and pmacs.parse.tree(b) ~= nil)",
            );
            if seen == "true" {
                Probe::Ready(())
            } else {
                Probe::Pending(report(s))
            }
        },
    );
    state
}

/// Tick until no parse of the open buffer is running or waiting, then a
/// few frames more, so a settled parse has been installed or told.
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
        state.tick_processes();
        state.tick_async();
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Review 2, Medium 1, the half that misleads. After a buffer's worker
/// crashed, no parse replaces the installed one until the back-off ends,
/// and none ever once its parsing stopped; at `cd6cd52` the grid kept
/// painting that parse's spans where they were, by line and column, so
/// lines inserted after the crash took their predecessors' colors, and the
/// semantic path shipped them to the GPU frontend by byte. And the buffer
/// said it had given up only through the unread error, which reading
/// `*errors*` ends: after it the mode line read `(rust)`, the echo line was
/// empty. Fix round 2 drops the spans when a crash leaves them over moved
/// text, and always at the stop, and marks the buffer's mode line for as
/// long as no parse installs: `parse:crashed`, then `parse:stopped`.
#[test]
#[allow(clippy::too_many_lines)] // one user's path, frame by frame
fn e7i_review2_a_crashed_buffer_paints_no_stale_span_and_keeps_its_mark() {
    let dir = tempfile::tempdir().expect("tempdir");
    let unit = crashing_on(dir.path(), "rust", "CRASHME");
    let path = dir.path().join("a.rs");
    std::fs::write(&path, rust_text(20)).expect("a.rs");
    let mut state = editor_on(&path, &unit);
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
    let before = marked_rows(&cells);
    assert!(
        before.len() >= 10,
        "control: the parsed file paints its keywords: {before:?}"
    );
    assert!(
        held.iter().any(|&(_, _, fg)| fg == MARK),
        "control: the semantic frame carries the keywords' spans"
    );
    assert!(
        !mode_line(&cells).contains("parse:"),
        "control: no mark on a parsed buffer: {}",
        mode_line(&cells)
    );

    // The marker typed at the top crashes the worker; three lines entered
    // after it, inside the back-off, move every line below.
    type_keys(&mut state, "CRASHME");
    ready::tick_until(&mut state, "the crash told", Duration::from_secs(30), |s| {
        let errors = s.lua_host.errors_buffer_text();
        if errors.contains("crashed") {
            Probe::Ready(())
        } else {
            Probe::Pending(report(s))
        }
    });
    type_keys(&mut state, "\n\n\n");
    quiesce(&mut state);
    apply_style_spans(&mut held, &semantic.render_frame(&state));
    let cells = paint(&state);
    let after_crash = marked_rows(&cells);
    let r = report(&state);
    say(&format!(
        "after the crash and three lines: {r}; mode line {:?}; rows in keyword color {after_crash:?}",
        mode_line(&cells)
    ));
    assert!(
        after_crash.is_empty(),
        "no span of the parse before the crash is painted over the moved text: {after_crash:?}"
    );
    assert!(
        !held.iter().any(|&(_, _, fg)| fg == MARK),
        "the GPU frontend holds no keyword span after the crash: {held:?}"
    );
    assert!(
        mode_line(&cells).contains("parse:crashed"),
        "the mode line says the buffer's parse crashed: {:?}",
        mode_line(&cells)
    );

    // On to the stop: an edit after each back-off crashes the next worker,
    // until the third crash in a row stops the buffer's parsing.
    ready::tick_until(
        &mut state,
        "the third crash in a row",
        Duration::from_secs(30),
        |s| {
            let r = report(s);
            let deaths: u32 = field(&r, "deaths").parse().unwrap_or(0);
            if deaths >= 3 && field(&r, "busy") == "false" {
                return Probe::Ready(());
            }
            if field(&r, "held") == "false" && field(&r, "busy") == "false" {
                type_keys(s, " ");
            }
            Probe::Pending(r)
        },
    );
    quiesce(&mut state);
    let cells = paint(&state);
    let stopped_line = mode_line(&cells);
    assert!(
        stopped_line.contains("parse:stopped"),
        "the mode line says the buffer's parsing stopped: {stopped_line:?}"
    );

    // The user reads `*errors*`, which marks every error read, and comes
    // back to the buffer.
    exec(
        &state,
        "for _, x in ipairs(pmacs.buffer.list()) do \
           if x:name() == '*errors*' then pmacs.window.switch_buffer(x) end \
         end",
    );
    let _ = paint(&state);
    assert_eq!(
        state.lua_host.unread_errors(),
        0,
        "showing *errors* read every error"
    );
    exec(
        &state,
        &format!(
            "pmacs.window.switch_buffer(pmacs.buffer.find_or_open({:?}))",
            path.display().to_string()
        ),
    );
    type_keys(&mut state, "\n\n");
    quiesce(&mut state);
    apply_style_spans(&mut held, &semantic.render_frame(&state));
    let cells = paint(&state);
    let read_line = mode_line(&cells);
    let echo = row_text(&cells, ROWS - 1);
    let after_stop = marked_rows(&cells);
    say(&format!(
        "after *errors* was read and two more lines: mode line {read_line:?}; echo {:?}; rows in keyword color {after_stop:?}",
        echo.trim_end()
    ));
    assert!(
        read_line.contains("parse:stopped"),
        "the mark outlives reading *errors*: {read_line:?}"
    );
    assert!(
        after_stop.is_empty(),
        "lines inserted after the stop take no stale color: {after_stop:?}"
    );
    assert!(
        !held.iter().any(|&(_, _, fg)| fg == MARK),
        "the GPU frontend holds no keyword span after the stop: {held:?}"
    );
}

/// The echo line of an 80-column grid: its last row.
fn echo_line_at_80(state: &EditorState) -> String {
    let (rows, cols) = (24u32, 80u32);
    let size = CellSize::new(rows, cols);
    let mut cells = vec![Cell::default(); (rows * cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: cols,
        size,
    };
    pmacs::editor::paint_frame(state, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);
    (0..cols)
        .map(
            |col| match cells[((rows - 1) * cols + col) as usize].glyph {
                Glyph::Char(c) => c,
                _ => ' ',
            },
        )
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// Review 2, Medium 1, the half that follows. The echo line shows the
/// first 80 columns of the last unread error, and at `cd6cd52` the notice
/// led with the buffer's absolute path, so a path of 63 characters left
/// `parse unit crashed: signal` on 120 columns, without the signal's number
/// or the back-off, and the stop's `parsing stopped` fitted only under 54.
/// Fix round 2 leads with the grammar, the signal and what follows, under
/// the file's short name, and parses the buffer again when each back-off
/// ends, so the stop arrives with no edit after the crash's.
#[test]
fn e7i_review2_the_echo_line_keeps_the_signal_and_the_state_at_80_columns() {
    let dir = tempfile::tempdir().expect("tempdir");
    let unit = crashing_on(dir.path(), "rust", "CRASHME");
    let deep = dir
        .path()
        .join("a-project-with-a-long-name/src/and/a/nested/module");
    std::fs::create_dir_all(&deep).expect("deep dir");
    let path = deep.join("a.rs");
    assert!(
        path.display().to_string().len() >= 63,
        "the path is at least the reviewer's 63 characters: {}",
        path.display()
    );
    std::fs::write(&path, rust_text(5)).expect("a.rs");
    let mut state = editor_on(&path, &unit);
    type_keys(&mut state, "CRASHME");
    let crashed = ready::tick_until(
        &mut state,
        "the crash told on the echo line",
        Duration::from_secs(30),
        |s| {
            let echo = echo_line_at_80(s);
            if echo.contains("crashed") {
                Probe::Ready(echo)
            } else {
                Probe::Pending(format!("{} echo {echo:?}", report(s)))
            }
        },
    );
    let stopped = ready::tick_until(
        &mut state,
        "the stop told on the echo line, with no edit",
        Duration::from_secs(30),
        |s| {
            let echo = echo_line_at_80(s);
            if echo.contains("stopped") {
                Probe::Ready(echo)
            } else {
                Probe::Pending(format!("{} echo {echo:?}", report(s)))
            }
        },
    );
    say(&format!(
        "the echo line at 80 columns: after the crash {crashed:?}; at the stop {stopped:?}"
    ));
    assert!(
        crashed.contains("rust crashed (SIGABRT) parsing a.rs; parsing again in 1 s"),
        "the grammar, the signal and the back-off survive the width: {crashed:?}"
    );
    assert!(
        stopped.contains("parsing stopped: rust crashed (SIGABRT) 3 times in a row"),
        "the stop and the signal survive the width: {stopped:?}"
    );
}
