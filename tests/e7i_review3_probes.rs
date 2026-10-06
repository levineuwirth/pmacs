//! E7i review round 3's two observations, ruled into fix round 3 as
//! findings, as witnesses against PR #309: a parse stopped at its deadline
//! or its memory limit painted the last parse's spans over text an edit
//! had moved (review 2's Medium 1 on the path a big file reaches), and a
//! worker the system killed from outside was told as a crash of its
//! grammar and counted toward the three-crash stop.
//!
//! Each row drives a real worker, the debug build's, through an
//! in-process editor on a real file, and reads the grid `paint_frame`
//! paints, the TUI's own frame, and the semantic frames the GPU frontend
//! receives; the last runs a daemon, whose watchdog the in-process host
//! cannot be made to use. The stops are real: the repository's own
//! markdown with #301's nested openers or #296's underscore paragraph put
//! in it, the regress inputs every fuzz arm replays
//! (`fuzz/regress/markdown/`); the kills are `kill -KILL`, as the
//! system's OOM killer sends it.

#[path = "common/mod.rs"]
mod common;

use std::collections::{HashMap, HashSet};
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

/// The color the rows theme markdown's `@text.*` captures with (#310:
/// the default theme styles none of them), so the parse's spans are told
/// from every other style on screen.
const MARK: Color = Color::Rgb(0x7b, 0x1f, 0xa2);

const ROWS: u32 = 30;
const COLS: u32 = 100;

/// One in-process editor at a time: the parse units' host is the
/// process's (`syntax.parse-unit-path` among its state).
static ONE_EDITOR: Mutex<()> = Mutex::new(());

fn one_editor() -> MutexGuard<'static, ()> {
    ONE_EDITOR.lock().unwrap_or_else(PoisonError::into_inner)
}

fn say(line: &str) {
    let _ = writeln!(std::io::stderr(), "e7i review 3: {line}");
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The debug worker beside the test's `pmacs`.
fn real_unit() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_pmacs"))
        .parent()
        .expect("target dir")
        .join("pmacs-parse-unit")
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

/// The open buffer's unit report, `key=value` words.
fn report(state: &EditorState) -> String {
    eval(
        state,
        "local b = pmacs.window.buffer() local r = pmacs.parse._unit_report(b) \
         if not r then return 'none' end \
         return string.format('deaths=%d crashes=%d busy=%s held=%s pending=%d stopped=%s tree=%s unit=%s death=%s', \
           r.deaths, r.crashes, tostring(r.busy), tostring(r.held ~= nil), \
           pmacs.parse._pending_edits(b) or 0, tostring(r.stopped), \
           tostring(pmacs.parse.tree(b) ~= nil), (r.unit:gsub(' ', '_')), \
           ((tostring(r.last_death):match('^(%a+):')) or 'nil'))",
    )
}

fn field(report: &str, name: &str) -> String {
    report
        .split_whitespace()
        .find_map(|w| w.strip_prefix(&format!("{name}=")))
        .unwrap_or("")
        .to_owned()
}

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

/// The mode line: the row above the echo line.
fn mode_line(cells: &[Cell]) -> String {
    row_text(cells, ROWS - 2).trim_end().to_owned()
}

/// Which text cells are painted in `MARK`.
fn marks(cells: &[Cell]) -> Vec<Vec<bool>> {
    (0..ROWS - 2)
        .map(|row| {
            (0..COLS)
                .map(|col| cells[(row * COLS + col) as usize].style.fg == MARK)
                .collect()
        })
        .collect()
}

fn marked(mask: &[Vec<bool>]) -> usize {
    mask.iter().flatten().filter(|&&m| m).count()
}

/// How many rows the edit pushed the text down: where the first line
/// with text on it before the edit is painted now.
fn shift(before: &[Cell], after: &[Cell]) -> u32 {
    // Past the line-number gutter, whose numbers the edit changed.
    let text = |cells: &[Cell], row: u32| {
        let t = row_text(cells, row);
        let t = t.trim_start();
        t.trim_start_matches(|c: char| c.is_ascii_digit())
            .trim()
            .to_owned()
    };
    let from = (0..ROWS - 2)
        .find(|&row| !text(before, row).is_empty())
        .expect("a line with text on it before the edit");
    let first = text(before, from);
    let Some(row) = (from..ROWS - 2).find(|&row| text(after, row) == first) else {
        let rows: Vec<String> = (0..ROWS).map(|r| row_text(after, r)).collect();
        panic!(
            "the file's first line {first:?} is on screen after the edit:\n{}",
            rows.join("\n")
        )
    };
    row - from
}

/// Cells painted in `MARK` where no parse of the text they show would put
/// it: on a row the edit inserted, or on a moved row where the same line
/// was not painted so before the edit. What a parse's spans paint over
/// text that moved under them.
fn stale_cells(before: &[Vec<bool>], after: &[Vec<bool>], rows_down: u32) -> usize {
    let mut stale = 0;
    for (row, cols) in after.iter().enumerate() {
        for (col, &m) in cols.iter().enumerate() {
            if !m {
                continue;
            }
            let was = (row as u32)
                .checked_sub(rows_down)
                .is_some_and(|r| before[r as usize][col]);
            if !was {
                stale += 1;
            }
        }
    }
    stale
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

/// `MARK` spans the frontend holds that are not where the spans it held
/// before the edit stand now, `inserted` bytes further on: spans placed
/// by the bytes of a text the edit moved.
fn stale_spans(before: &[(u64, u64, Color)], held: &[(u64, u64, Color)], inserted: u64) -> usize {
    let moved: HashSet<(u64, u64)> = before
        .iter()
        .filter(|s| s.2 == MARK)
        .map(|&(s, e, _)| (s + inserted, e + inserted))
        .collect();
    held.iter()
        .filter(|s| s.2 == MARK && !moved.contains(&(s.0, s.1)))
        .count()
}

/// An editor on a copy of the repository's `docs/divergences.md`, a
/// markdown file whose first screen holds a heading, strong text and
/// code spans, with the worker `unit` and `settings` (Lua) applied before
/// it opens; after its first parse installed.
fn editor_on_notes(dir: &Path, unit: &Path, settings: &str) -> (EditorState, PathBuf) {
    let path = dir.join("divergences.md");
    std::fs::copy(repo().join("docs/divergences.md"), &path).expect("copy the notes");
    let mut state = EditorState::new_with_roots(&common::iso::roots());
    exec(
        &state,
        &format!(
            "pmacs.lsp.config = {{}}\n\
             pmacs.config.set('syntax.isolation', 'process')\n\
             pmacs.config.set('syntax.parse-unit-path', {:?})\n\
             pmacs.config.set('ui.line-wrap', 'truncate')\n\
             pmacs.theme.merge {{ text = {{ fg = {{ 0x7b, 0x1f, 0xa2 }} }} }}\n\
             {settings}\n\
             pmacs.buffer.find_or_open({:?})",
            unit.display().to_string(),
            path.display().to_string()
        ),
    );
    wait_installed(&mut state, "the first parse installed");
    (state, path)
}

fn wait_installed(state: &mut EditorState, what: &str) {
    ready::tick_until(state, what, Duration::from_mins(1), |s| {
        let r = report(s);
        if field(&r, "busy") == "false" && field(&r, "tree") == "true" {
            Probe::Ready(())
        } else {
            Probe::Pending(r)
        }
    });
}

fn one_tick(state: &mut EditorState) {
    state.tick_processes();
    state.tick_lsp();
    state.tick_async();
}

/// Tick until no parse of the open buffer is running or waiting, then a
/// few frames more, so a settled parse has been installed or told.
fn quiesce(state: &mut EditorState) {
    ready::tick_until(
        state,
        "the buffer's parses settled",
        Duration::from_mins(1),
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

/// #296's paragraph as the issue generates it, `lines` lines long.
fn underscores(lines: usize) -> String {
    let line = format!("{}a `_`_", "_".repeat(582));
    vec![line; lines].join("\n") + "\n"
}

/// Which stop a row reaches, and how.
#[derive(Clone, Copy, Debug)]
enum Stop {
    /// #301's nested openers, past a 300 ms deadline.
    Deadline,
    /// #296's underscore paragraph, past a 64 MiB worker.
    Memory,
}

impl Stop {
    /// What is put at the head of the file: the input and a blank line,
    /// so the paragraphs after it are their own.
    fn input(self) -> String {
        match self {
            Self::Deadline => {
                std::fs::read_to_string(
                    repo().join("fuzz/regress/markdown/301-nested-openers-98.input"),
                )
                .expect("#301's input")
                    + "\n\n"
            }
            Self::Memory => underscores(14) + "\n",
        }
    }

    /// Set before the file opens: the memory limit is the worker's from
    /// its start.
    fn settings(self) -> &'static str {
        match self {
            Self::Deadline => "",
            Self::Memory => {
                "pmacs.config.set('syntax.parse-memory-limit-mb', 64)\n\
                 pmacs.config.set('syntax.parse-deadline-ms', 20000)"
            }
        }
    }

    /// Set after the first parse installed, under the default deadline.
    fn armed(self) -> &'static str {
        match self {
            Self::Deadline => "pmacs.config.set('syntax.parse-deadline-ms', 300)",
            Self::Memory => "",
        }
    }

    /// The death the slot records for it.
    fn death(self) -> &'static str {
        match self {
            Self::Deadline => "time",
            Self::Memory => "memory",
        }
    }

    /// The mode line's mark while it stands.
    fn mark(self) -> &'static str {
        match self {
            Self::Deadline => "parse:timeout",
            Self::Memory => "parse:memory",
        }
    }

    /// How the notice in `*errors*` says it.
    fn told(self) -> &'static str {
        match self {
            Self::Deadline => "ran past syntax.parse-deadline-ms",
            Self::Memory => "reached syntax.parse-memory-limit-mb",
        }
    }
}

/// What one look at the screen and the frontend's spans found.
struct Look {
    at_ms: u128,
    stale_cells: usize,
    marked_cells: usize,
    stale_spans: usize,
    mode_line: String,
    report: String,
}

/// Fix round 3, item 1: a parse stopped at its deadline or memory limit
/// installs nothing, and at `db697a2` the buffer kept painting the last
/// parse's spans where they had been, so every line the edit moved took
/// its predecessor's colors, on the grid and in the GPU's frames, for as
/// long as the input that stops the parse stays: each keystroke's parse
/// is stopped again. The row puts #301's openers (or #296's paragraph) at
/// the head of a real markdown file, as a paste, types three more
/// characters a second apart, then takes the input out, and looks at the
/// screen after every stop.
#[allow(clippy::too_many_lines)] // one user's path, stop by stop
fn a_stop_over_moved_text(stop: Stop) {
    let _one = one_editor();
    let dir = tempfile::tempdir().expect("tempdir");
    let (mut state, _path) = editor_on_notes(dir.path(), &real_unit(), stop.settings());
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
    let before_cells = paint(&state);
    let before = marks(&before_cells);
    let before_spans = held.clone();
    assert!(
        marked(&before) >= 20,
        "control: the notes are painted ({} cells)",
        marked(&before)
    );
    assert!(
        before_spans.iter().filter(|s| s.2 == MARK).count() >= 5,
        "control: the frontend holds the notes' spans"
    );

    exec(&state, stop.armed());
    let input = stop.input();
    exec(
        &state,
        &format!(
            "pmacs.editor.goto_byte(0)\n\
             pmacs.window.buffer():insert(0, {input:?})"
        ),
    );
    // The paste's parse, dispatched by the keystroke after it, as the
    // after-edit hook dispatches every edit's.
    type_keys(&mut state, "x");
    let mut inserted = input.len() as u64 + 1;
    let pasted = Instant::now();
    let mut looks: Vec<Look> = Vec::new();
    let mut look = |state: &mut EditorState, inserted: u64, looks: &mut Vec<Look>| {
        quiesce(state);
        apply_style_spans(&mut held, &semantic.render_frame(state));
        let cells = paint(state);
        let after = marks(&cells);
        looks.push(Look {
            at_ms: pasted.elapsed().as_millis(),
            stale_cells: stale_cells(&before, &after, shift(&before_cells, &cells)),
            marked_cells: marked(&after),
            stale_spans: stale_spans(&before_spans, &held, inserted),
            mode_line: mode_line(&cells),
            report: report(state),
        });
    };
    ready::tick_until(&mut state, "the stop told", Duration::from_mins(1), |s| {
        if errors_text(s).contains(stop.told()) {
            Probe::Ready(())
        } else {
            Probe::Pending(report(s))
        }
    });
    look(&mut state, inserted, &mut looks);
    for _ in 0..3 {
        std::thread::sleep(Duration::from_secs(1));
        type_keys(&mut state, "x");
        inserted += 1;
        look(&mut state, inserted, &mut looks);
    }
    let stopped_looks = looks.len();

    // The input taken out, and the keystrokes with it: the text is the
    // file again, and its parse installs.
    exec(
        &state,
        &format!(
            "local b = pmacs.window.buffer()\n\
             b:delete(0, {inserted})\n\
             pmacs.parse._dispatch(b, 'markdown')"
        ),
    );
    ready::tick_until(
        &mut state,
        "a parse installs once the input is out",
        Duration::from_mins(1),
        |s| {
            let r = report(s);
            let cells = paint(s);
            if field(&r, "busy") == "false"
                && field(&r, "tree") == "true"
                && marked(&marks(&cells)) == marked(&before)
            {
                Probe::Ready(())
            } else {
                Probe::Pending(r)
            }
        },
    );
    look(&mut state, 0, &mut looks);
    for (i, l) in looks.iter().enumerate() {
        say(&format!(
            "{stop:?} look {i} at {} ms: stale cells {}, marked cells {}, stale frontend spans {}, mode line {:?}, {}",
            l.at_ms, l.stale_cells, l.marked_cells, l.stale_spans, l.mode_line, l.report
        ));
    }
    let told = errors_text(&state).matches(stop.told()).count();
    say(&format!("{stop:?}: told {told} time(s)"));

    for l in &looks[..stopped_looks] {
        assert_eq!(
            field(&l.report, "death"),
            stop.death(),
            "each look follows a {stop:?} stop: {}",
            l.report
        );
    }
    for (i, l) in looks[..stopped_looks].iter().enumerate() {
        assert_eq!(
            (l.stale_cells, l.stale_spans),
            (0, 0),
            "look {i}, {} ms after the paste: no span of the last parse is painted over the moved text, \
             on the grid or in the frontend's frames",
            l.at_ms
        );
        assert!(
            l.mode_line.contains(stop.mark()),
            "look {i}: the mode line says the parse was stopped, in a word that is not \
             `parse:stopped` (the next edit parses it again): {:?}",
            l.mode_line
        );
    }
    let last = looks.last().expect("the last look");
    assert_eq!(
        (last.stale_cells, last.marked_cells),
        (0, marked(&before)),
        "once the input is out a parse installs and the notes are painted again"
    );
    assert!(
        !last.mode_line.contains("parse:"),
        "a parse that installs clears the mark: {:?}",
        last.mode_line
    );
    let notice = errors_text(&state);
    assert!(
        notice.contains("the next edit parses it again") && !notice.contains("stays as it was"),
        "the notice says what follows the stop, not that the highlighting stays: {notice}"
    );
    assert_eq!(told, 1, "told once while the input stays");
}

#[test]
fn e7i_review3_a_deadline_stop_paints_no_span_over_moved_text() {
    a_stop_over_moved_text(Stop::Deadline);
}

#[test]
fn e7i_review3_a_memory_stop_paints_no_span_over_moved_text() {
    a_stop_over_moved_text(Stop::Memory);
}

/// The outcome item 1 must not touch: a deadline that cuts only injected
/// layers installs the root and the layers parsed before it, a parse of
/// the current text, so it keeps its spans and carries no mark. A real
/// file of 70 KB, the repository's own markdown, whose 180 inline layers
/// take most of a parse: with the deadline set past the root's parse and
/// short of the layers', an edit's parse is cut in its layers.
#[test]
#[allow(clippy::too_many_lines)] // one user's path, frame by frame
fn e7i_review3_a_deadline_that_cuts_only_layers_keeps_its_current_spans() {
    let _one = one_editor();
    let dir = tempfile::tempdir().expect("tempdir");
    let mut text = String::new();
    for file in [
        "README.md",
        "docs/divergences.md",
        "docs/invariants.md",
        "CLAUDE.md",
        "fuzz/regress/README.md",
    ] {
        text.push_str(
            &std::fs::read_to_string(repo().join(file)).expect("the repository's markdown"),
        );
        text.push('\n');
    }
    let path = dir.path().join("docs.md");
    std::fs::write(&path, &text).expect("docs.md");
    let mut state = EditorState::new_with_roots(&common::iso::roots());
    exec(
        &state,
        &format!(
            "pmacs.lsp.config = {{}}\n\
             pmacs.config.set('syntax.isolation', 'process')\n\
             pmacs.config.set('syntax.parse-unit-path', {:?})\n\
             pmacs.config.set('ui.line-wrap', 'truncate')\n\
             pmacs.theme.merge {{ text = {{ fg = {{ 0x7b, 0x1f, 0xa2 }} }} }}\n\
             pmacs.buffer.find_or_open({:?})",
            real_unit().display().to_string(),
            path.display().to_string()
        ),
    );
    wait_installed(&mut state, "the first parse installed");
    let cut_told = |s: &EditorState| {
        errors_text(s)
            .matches("an embedded region's parse ran past")
            .count()
    };
    let current = |s: &EditorState| {
        eval(
            s,
            "local b = pmacs.window.buffer() local t = pmacs.parse.tree(b) \
             return tostring(t ~= nil and t:source_len() == b:len())",
        ) == "true"
    };
    let mut cut = None;
    let mut deadline = 0;
    for attempt in 0..4 {
        // An edit at the head parsed whole first, so the next parse is
        // incremental and its root takes what the warm one took.
        exec(
            &state,
            "pmacs.config.set('syntax.parse-deadline-ms', 0) pmacs.editor.goto_byte(0)",
        );
        type_keys(&mut state, "\n");
        quiesce(&mut state);
        assert!(current(&state), "the warm parse installed");
        let root_ms: u64 = eval(
            &state,
            "return tostring(math.floor(pmacs.parse.tree(pmacs.window.buffer()):parse_duration_ms()))",
        )
        .parse()
        .expect("the root's parse time");
        deadline = (root_ms * 2 + 10) << attempt;
        let before_cells = paint(&state);
        let told_before = cut_told(&state);
        exec(
            &state,
            &format!(
                "pmacs.config.set('syntax.parse-deadline-ms', {deadline}) pmacs.editor.goto_byte(0)"
            ),
        );
        type_keys(&mut state, "\n");
        quiesce(&mut state);
        say(&format!(
            "attempt {attempt}: root {root_ms} ms, deadline {deadline} ms, cut told {}, current {}, {}",
            cut_told(&state),
            current(&state),
            report(&state)
        ));
        if cut_told(&state) > told_before {
            cut = Some(before_cells);
            break;
        }
    }
    let before_cells = cut.unwrap_or_else(|| {
        panic!(
            "no deadline cut only layers in four attempts: {}",
            errors_text(&state)
        )
    });
    assert!(
        current(&state),
        "the cut parse is installed, of the buffer's text: {}",
        report(&state)
    );
    let before = marks(&before_cells);
    let cells = paint(&state);
    let after = marks(&cells);
    let stale = stale_cells(&before, &after, shift(&before_cells, &cells));
    say(&format!(
        "a parse cut in its layers at {deadline} ms: marked {} before, {} after, stale {stale}, mode line {:?}",
        marked(&before),
        marked(&after),
        mode_line(&cells)
    ));
    assert!(
        marked(&before) > 0 && marked(&after) > 0,
        "the cut parse's spans are painted ({} cells)",
        marked(&after)
    );
    assert_eq!(stale, 0, "and painted where its text stands now");
    assert!(
        !mode_line(&cells).contains("parse:"),
        "a parse that installed carries no mark: {:?}",
        mode_line(&cells)
    );
}

/// A Rust file of `n` functions, a keyword at the head of every line.
fn rust_text(n: usize) -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for i in 0..n {
        let _ = write!(text, "fn f{i}() {{\n    let x = {i};\n}}\n");
    }
    text
}

/// `kill -KILL <pid>`, as the system's out-of-memory killer sends it.
fn kill_from_outside(pid: &str) {
    let killed = std::process::Command::new("kill")
        .args(["-KILL", pid])
        .status()
        .expect("kill");
    assert!(killed.success(), "killed the worker {pid} from outside");
}

/// What `*errors*` says of kills and of crashes.
fn told_killed(state: &EditorState) -> usize {
    errors_text(state)
        .matches("killed from outside pmacs (SIGKILL)")
        .count()
}

/// Fix round 3, item 2: a worker killed by a `SIGKILL` the editor did not
/// send died of something outside pmacs (under memory pressure the
/// system's OOM killer takes the largest processes first, and the parse
/// workers are among them), not of its grammar, which cannot raise
/// `SIGKILL`. At `db697a2` it was told "rust crashed (SIGKILL)", backed off
/// and counted toward the three-crash stop, so three such kills ended the
/// buffer's highlighting for good. Here the idle worker is killed three
/// times, each found by the parse a buffer switch dispatches: each is told
/// as a kill, counts no crash and holds nothing, so the parse the switch
/// requested after it starts a worker at once and installs, as does the
/// next edit's; the buffer never stops.
#[test]
#[allow(clippy::too_many_lines)] // one user's path, kill by kill
fn e7i_review3_a_worker_killed_from_outside_is_not_a_crash_and_never_stops_the_buffer() {
    let _one = one_editor();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("a.rs");
    std::fs::write(&path, rust_text(20)).expect("a.rs");
    let other = dir.path().join("notes.txt");
    std::fs::write(&other, "plain\n").expect("notes.txt");
    let mut state = EditorState::new_with_roots(&common::iso::roots());
    exec(
        &state,
        &format!(
            "pmacs.lsp.config = {{}}\n\
             pmacs.config.set('syntax.isolation', 'process')\n\
             pmacs.config.set('syntax.parse-unit-path', {:?})\n\
             pmacs.theme.merge {{ keyword = {{ fg = {{ 0x7b, 0x1f, 0xa2 }} }} }}\n\
             pmacs.buffer.find_or_open({:?})",
            real_unit().display().to_string(),
            path.display().to_string()
        ),
    );
    wait_installed(&mut state, "the first parse installed");
    let mut rows = Vec::new();
    for kill in 1..=3 {
        let r = report(&state);
        let pid = field(&r, "unit").trim_start_matches("pid_").to_owned();
        assert!(!pid.is_empty(), "kill {kill}: a worker lives: {r}");
        kill_from_outside(&pid);
        std::thread::sleep(Duration::from_millis(200));
        // A switch away and back dispatches a parse with no edit, which
        // finds the worker gone.
        exec(
            &state,
            &format!(
                "pmacs.window.switch_buffer(pmacs.buffer.find_or_open({:?}))\n\
                 pmacs.window.switch_buffer(pmacs.buffer.find_or_open({:?}))",
                other.display().to_string(),
                path.display().to_string()
            ),
        );
        let deaths = kill.to_string();
        ready::tick_until(&mut state, "the kill found", Duration::from_mins(1), |s| {
            let r = report(s);
            if field(&r, "deaths") == deaths && field(&r, "busy") == "false" {
                Probe::Ready(())
            } else {
                Probe::Pending(r)
            }
        });
        quiesce(&mut state);
        let found = report(&state);
        let found_line = mode_line(&paint(&state));
        // The next edit: a line entered at the head.
        exec(&state, "pmacs.editor.goto_byte(0)");
        type_keys(&mut state, "\n");
        let edited = Instant::now();
        let installed = ready::tick_until(
            &mut state,
            "the edit's parse installs",
            Duration::from_mins(1),
            |s| {
                let r = report(s);
                let current = eval(
                    s,
                    "local b = pmacs.window.buffer() local t = pmacs.parse.tree(b) \
                     return tostring(t ~= nil and t:source_len() == b:len())",
                );
                if current == "true" && field(&r, "busy") == "false" {
                    Probe::Ready(r)
                } else {
                    Probe::Pending(r)
                }
            },
        );
        let install_ms = edited.elapsed().as_millis();
        quiesce(&mut state);
        let cells = paint(&state);
        say(&format!(
            "kill {kill}: found {found}, mode line {found_line:?}; the edit's parse installed in \
             {install_ms} ms: {installed}, mode line {:?}",
            mode_line(&cells)
        ));
        rows.push((found, found_line, install_ms, installed, mode_line(&cells)));
    }
    let errors = errors_text(&state);
    say(&format!("*errors*:\n{errors}"));

    for (i, (found, found_line, install_ms, installed, line)) in rows.iter().enumerate() {
        let kill = i + 1;
        assert_eq!(
            (
                field(found, "death").as_str(),
                field(found, "crashes").as_str(),
                field(found, "held").as_str(),
                field(found, "stopped").as_str()
            ),
            ("killed", "0", "false", "false"),
            "kill {kill} is told as a kill, not a crash: no streak, no back-off: {found}"
        );
        assert!(
            field(found, "unit").starts_with("pid_") && !found_line.contains("parse:"),
            "kill {kill}: nothing held the switch's follow-up parse, which started a worker \
             at once and installed (the mark is witnessed where no parse follows, below): \
             {found}, {found_line:?}"
        );
        assert!(
            *install_ms < 900,
            "kill {kill}: the next edit's parse starts a worker at once, not after a back-off \
             ({install_ms} ms): {installed}"
        );
        assert!(
            !line.contains("parse:"),
            "kill {kill}: the install clears the mark: {line:?}"
        );
    }
    assert!(
        !errors.contains("crashed"),
        "no kill is told as a crash: {errors}"
    );
    assert_eq!(
        told_killed(&state),
        3,
        "each kill told once, the install between them re-arming the notice: {errors}"
    );
    assert!(
        errors.contains("not a crash of rust"),
        "the notice clears the grammar: {errors}"
    );
}

/// Fix round 3, item 2: the editor's own kills send `SIGKILL` too, and
/// know they did, so the witness tells the two sources apart rather than
/// two signals. #296's paragraph is pasted at the head of a real markdown
/// file under a 20 s deadline, and each time its parse grows past 64 MiB
/// the worker is killed from outside, as the system's OOM killer would
/// take it, three times in a row with no parse installed between: each is
/// told as a kill (once), its moved spans dropped, and none counts toward
/// the three-crash stop, which at `db697a2` the third reached. Then, the
/// paragraph replaced by #301's nested openers, whose parse no deadline
/// races, the editor kills the next worker itself under a 300 ms deadline
/// (`Death::Time`, a `SIGKILL` it sent): told as the deadline it is. Once
/// the openers are out a parse installs.
#[test]
#[allow(clippy::too_many_lines)] // one user's path, kill by kill
fn e7i_review3_the_editor_s_own_sigkill_and_an_outside_one_are_told_apart() {
    let _one = one_editor();
    let dir = tempfile::tempdir().expect("tempdir");
    let (mut state, _path) = editor_on_notes(
        dir.path(),
        &real_unit(),
        "pmacs.config.set('syntax.parse-deadline-ms', 20000)",
    );
    let before_cells = paint(&state);
    let before = marks(&before_cells);
    let paragraph = underscores(14) + "\n";
    exec(
        &state,
        &format!(
            "pmacs.editor.goto_byte(0)\n\
             pmacs.window.buffer():insert(0, {paragraph:?})"
        ),
    );
    let mut inserted = paragraph.len();
    let settled = |state: &mut EditorState, deaths: usize, what: &str| {
        let deaths = deaths.to_string();
        ready::tick_until(state, what, Duration::from_mins(1), |s| {
            let r = report(s);
            if field(&r, "deaths") == deaths
                && field(&r, "busy") == "false"
                && field(&r, "pending") == "0"
            {
                Probe::Ready(())
            } else {
                Probe::Pending(r)
            }
        });
        quiesce(state);
    };
    for kill in 1..=3 {
        // The next edit dispatches the paragraph's parse.
        type_keys(&mut state, "x");
        inserted += 1;
        let pid = ready::tick_until(
            &mut state,
            "the paragraph's parse grows past 64 MiB",
            Duration::from_mins(1),
            |s| {
                let grown = eval(
                    s,
                    "local r = pmacs.parse._unit_report(pmacs.window.buffer()) \
                     if r and r.busy and (r.memory or 0) > 64 * 1024 * 1024 then \
                       return (r.unit:match('%d+')) end return ''",
                );
                if grown.is_empty() {
                    Probe::Pending(report(s))
                } else {
                    Probe::Ready(grown)
                }
            },
        );
        kill_from_outside(&pid);
        settled(&mut state, kill, "the kill found");
        let found = report(&state);
        let cells = paint(&state);
        let line = mode_line(&cells);
        let stale = stale_cells(&before, &marks(&cells), shift(&before_cells, &cells));
        say(&format!(
            "kill {kill} from outside at 64 MiB: {found}, mode line {line:?}, stale {stale}"
        ));
        assert_eq!(
            (
                field(&found, "death").as_str(),
                field(&found, "crashes").as_str(),
                field(&found, "held").as_str(),
                field(&found, "stopped").as_str()
            ),
            ("killed", "0", "false", "false"),
            "kill {kill}: an outside SIGKILL is a kill, not a crash: no streak, no back-off, no stop"
        );
        assert!(
            line.contains("parse:killed") && stale == 0,
            "kill {kill}: the kill drops the moved spans and marks the buffer: {line:?}, {stale} stale"
        );
    }

    // The editor's own kill: the deadline's, 100 ms past 300 ms, on #301's
    // nested openers put in the paragraph's place. The paragraph raced it
    // (#322): its expense is markdown's inline layer, and where that layer
    // was still within the progress callback's reach at the deadline the
    // worker cut it and answered with the root, so a tree installed and no
    // kill came, on the macOS luajit runner and here at a 100 ms deadline or
    // with the worker throttled to a quarter of its speed. The openers'
    // parse is bounded below instead: it runs on, out of the callback's
    // reach (`HARD_GRACE`'s "#301's condensation"), for about 23 s natively
    // (`fuzz/regress/README.md`), so the editor's kill is the only way it
    // ends, as the deadline stop above relies on.
    let openers = Stop::Deadline.input();
    exec(
        &state,
        &format!(
            "pmacs.config.set('syntax.parse-deadline-ms', 300)\n\
             local b = pmacs.window.buffer()\n\
             b:delete(0, {inserted})\n\
             b:insert(0, {openers:?})\n\
             pmacs.editor.goto_byte(0)"
        ),
    );
    type_keys(&mut state, "x");
    inserted = openers.len() + 1;
    settled(&mut state, 4, "the deadline's kill");
    let own = report(&state);
    let own_line = mode_line(&paint(&state));
    say(&format!(
        "killed by the deadline: {own}, mode line {own_line:?}"
    ));
    assert_eq!(
        (
            field(&own, "death").as_str(),
            field(&own, "crashes").as_str()
        ),
        ("time", "0"),
        "the editor's own SIGKILL at the deadline stays the deadline's: {own}"
    );
    assert!(
        own_line.contains("parse:timeout"),
        "and is marked as one: {own_line:?}"
    );

    // The openers out: a parse installs, and the mark goes.
    exec(
        &state,
        &format!(
            "local b = pmacs.window.buffer()\n\
             b:delete(0, {inserted})\n\
             pmacs.parse._dispatch(b, 'markdown')"
        ),
    );
    wait_installed(&mut state, "a parse installs once the openers are out");
    quiesce(&mut state);
    let cells = paint(&state);
    let errors = errors_text(&state);
    say(&format!(
        "the openers out: {}, mode line {:?}\n*errors*:\n{errors}",
        report(&state),
        mode_line(&cells)
    ));
    assert_eq!(
        marked(&marks(&cells)),
        marked(&before),
        "the notes are painted again"
    );
    assert!(
        !mode_line(&cells).contains("parse:"),
        "the install clears the mark: {:?}",
        mode_line(&cells)
    );
    assert!(
        told_killed(&state) == 1
            && errors.contains("ran past syntax.parse-deadline-ms")
            && !errors.contains("crashed"),
        "the three kills told once, the deadline once, no crash: {errors}"
    );
}

/// Fix round 3, item 2, the two sources through one door. A worker that
/// dies under the editor's own kill and one killed from outside both die
/// by `SIGKILL`, and both reach `ProcessUnit::death` when the parse thread
/// finds the worker gone: the editor's watchdog kills the largest worker
/// from its own thread once the units pass their total, as it does where
/// no cgroup holds them (macOS, an undelegated Linux; the hook keeps the
/// daemon from making one here). One daemon holds three buffers of #296's
/// paragraph, which the watchdog stops at a 256 MiB total, and a Rust
/// buffer whose idle worker is then killed from outside and found by its
/// next parse. The watchdog's kills are the total's; the outside one is a
/// kill, and no crash is counted.
#[test]
#[allow(clippy::too_many_lines)] // one daemon's script and its reading
fn e7i_review3_the_watchdog_s_sigkill_and_an_outside_one_reach_death_and_are_told_apart() {
    use std::fmt::Write as _;
    let dir = tempfile::tempdir().expect("tempdir").keep();
    let mut opens = String::new();
    for i in 0..3 {
        let path = dir.join(format!("v{i}.md"));
        std::fs::write(&path, underscores(28)).expect("victim");
        let _ = writeln!(
            opens,
            "victims[#victims + 1] = pmacs.buffer.find_or_open({:?})",
            path.display().to_string()
        );
    }
    let typed = dir.join("typed.rs");
    std::fs::write(&typed, rust_text(5)).expect("typed.rs");
    let report = dir.join("report.txt");
    let init = format!(
        "pmacs.lsp.config = {{}}\n\
         pmacs.config.set('syntax.isolation', 'process')\n\
         pmacs.config.set('syntax.parse-memory-limit-mb', 2048)\n\
         pmacs.config.set('syntax.parse-memory-total-mb', 256)\n\
         pmacs.config.set('syntax.parse-deadline-ms', 20000)\n\
         local typed = pmacs.buffer.find_or_open({typed:?})\n\
         local victims = {{}}\n\
         {opens}\
         local function write(text)\n\
           local f = assert(io.open({report:?}, 'w')); f:write(text); f:close()\n\
         end\n\
         pmacs.async(function()\n\
           local t0 = pmacs.editor.monotonic_ms()\n\
           local function wait(ok)\n\
             while not ok() do\n\
               if pmacs.editor.monotonic_ms() - t0 > 60000 then return false end\n\
               pmacs.workers.sleep(50):await()\n\
             end\n\
             return true\n\
           end\n\
           local function settled(b, deaths)\n\
             local r = pmacs.parse._unit_report(b)\n\
             return r and not r.busy and r.deaths >= deaths\n\
           end\n\
           if not wait(function()\n\
             local deaths, busy = 0, false\n\
             for _, v in ipairs(victims) do\n\
               local r = pmacs.parse._unit_report(v)\n\
               if r then deaths = deaths + r.deaths; busy = busy or r.busy end\n\
             end\n\
             return deaths >= 1 and not busy\n\
           end) then write('the watchdog stopped nothing\\n') return end\n\
           if not wait(function() local r = pmacs.parse._unit_report(typed); return r and r.unit ~= '' and not r.busy end) then\n\
             write('the Rust buffer has no worker\\n') return\n\
           end\n\
           local pid = pmacs.parse._unit_report(typed).unit:match('%d+')\n\
           local p = io.popen('kill -KILL ' .. pid .. '; echo $?'); local code = p:read('*l'); p:close()\n\
           pmacs.workers.sleep(200):await()\n\
           typed:insert(0, '// a line\\n')\n\
           pmacs.parse._dispatch(typed, 'rust')\n\
           if not wait(function() return settled(typed, 1) end) then write('the kill was never found\\n') return end\n\
           local lines = {{}}\n\
           for _, v in ipairs(victims) do\n\
             local r = pmacs.parse._unit_report(v)\n\
             lines[#lines + 1] = string.format('victim crashes=%d death=%s', r.crashes, tostring(r.last_death))\n\
           end\n\
           local r = pmacs.parse._unit_report(typed)\n\
           lines[#lines + 1] = string.format('typed kill=%s crashes=%d held=%s death=%s', code, r.crashes,\n\
             tostring(r.held ~= nil), tostring(r.last_death))\n\
           lines[#lines + 1] = 'report ' .. tostring(pmacs.parse._isolation_report())\n\
           write(table.concat(lines, '\\n') .. '\\n')\n\
         end)\n",
        typed = typed.display().to_string(),
        report = report.display().to_string(),
    );
    let mut daemon = common::daemon::TestDaemon::spawn_with_env_and_init(
        &[("PMACS_PARSE_UNIT_CGROUP", "off")],
        &init,
    );
    let started = Instant::now();
    let text = loop {
        if let Ok(text) = std::fs::read_to_string(&report)
            && text.ends_with('\n')
        {
            break text;
        }
        assert!(
            started.elapsed() < Duration::from_mins(2),
            "the daemon's report never came: {}",
            std::fs::read_to_string(format!("{}.stderr.log", daemon.socket_path().display()))
                .unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    say(&format!("one daemon, two sources:\n{text}"));
    let victims: Vec<&str> = text.lines().filter(|l| l.starts_with("victim ")).collect();
    let typed_line = text
        .lines()
        .find(|l| l.starts_with("typed "))
        .unwrap_or_else(|| panic!("no Rust buffer's line: {text}"));
    assert!(
        victims
            .iter()
            .any(|l| l.contains("death=total:") && l.contains("(by watchdog)")),
        "the watchdog's own SIGKILL is the total's: {text}"
    );
    assert!(
        victims
            .iter()
            .all(|l| l.contains("crashes=0") && !l.contains("death=killed:")),
        "no watchdog kill is told as an outside one or counted as a crash: {text}"
    );
    assert!(
        typed_line.starts_with("typed kill=0 crashes=0 held=false death=killed: "),
        "the outside SIGKILL is a kill, holds nothing and counts no crash: {typed_line}"
    );
    assert!(daemon.is_alive(), "the daemon outlived both");
}
