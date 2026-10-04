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
//! receives. The stops are real: the repository's own markdown with
//! #301's nested openers or #296's underscore paragraph put in it, the
//! regress inputs every fuzz arm replays (`fuzz/regress/markdown/`).

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
