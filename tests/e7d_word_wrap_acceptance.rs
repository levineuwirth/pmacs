//! E7d.1 — word wrap on the grid (D35).
//!
//! `ui.line-wrap`'s `wrap` breaks at word boundaries: after a run of
//! spaces, after a tab, after a hyphen, dash or slash, and on either side
//! of a double-width character. A run of spaces hangs past the edge, as
//! cosmic-text's blank word does on the GPU; a word wider than the row
//! starts a fresh row and breaks by glyph, so no character is ever
//! unreachable.
//!
//! Every row here reads the frame through `RenderState::render_frame`,
//! the wire the TUI paints from: the `CellDelta` for the text and the
//! `Cursor` message for the caret. A test that built its own viewport
//! and called `TextView::render` would pass against a driver that never
//! reached the renderer (the gap `line_wrap_acceptance` was written for).

use std::collections::HashMap;
use std::path::Path;

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use pmacs::bootstrap::BootstrapRoots;
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

fn session(name: &str) -> EditorState {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("e7d-word-wrap")
        .join(name);
    let _ = std::fs::remove_dir_all(&base);
    let roots = BootstrapRoots::isolated_under(&base);
    for (_, dir) in roots.child_env() {
        std::fs::create_dir_all(&dir).expect("create controlled root");
    }
    let state = EditorState::new_with_roots(&roots);
    state.install_state_dirs();
    // The gutter eats content columns; these rows are about where the
    // TEXT breaks, so they paint against a bare grid.
    exec(&state, "pmacs.window.set_line_numbers('off')");
    state
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

/// Put `text` in the scratch buffer through the Lua buffer API and leave
/// the caret at byte 0.
fn fill(s: &EditorState, text: &str) {
    let quoted = format!("{text:?}");
    exec(
        s,
        &format!(
            "local b = pmacs.window.buffer(); \
             b:insert(0, {quoted}); \
             pmacs.editor.goto_byte(0)"
        ),
    );
}

fn key(s: &mut EditorState, code: KeyCode, modifiers: KeyModifiers) {
    s.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    );
}

fn ctrl(s: &mut EditorState, c: char) {
    key(s, KeyCode::Char(c), KeyModifiers::CONTROL);
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        key(s, KeyCode::Char(ch), KeyModifiers::NONE);
    }
}

fn click(s: &mut EditorState, row: u16, column: u16, rows: u32, cols: u32) {
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        s.dispatch_mouse(
            FrontendId::LOCAL,
            MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            },
            pmacs::cell::CellSize::new(rows, cols),
        );
    }
}

fn cursor_byte(s: &EditorState) -> u64 {
    eval(s, "return pmacs.editor.cursor()")
}

/// One frame: the text of each grid row and the caret's cell.
struct Frame {
    rows: Vec<String>,
    caret: Option<(u32, u32)>,
}

fn frame(s: &EditorState, rows: u32, cols: u32) -> Frame {
    let size = pmacs::cell::CellSize::new(rows, cols);
    let mut rs = pmacs::instance_render::RenderState::new(size);
    let msgs = rs.render_frame(s, pmacs::protocol::FrontendId::LOCAL, &HashMap::new(), &[]);
    let mut grid = vec![vec![' '; cols as usize]; rows as usize];
    let mut caret = None;
    for msg in &msgs {
        match msg {
            pmacs_protocol::InstanceMessage::CellDelta { spans, .. } => {
                for span in spans {
                    for (i, cell) in span.cells.iter().enumerate() {
                        let r = span.start.row as usize;
                        let c = span.start.col as usize + i;
                        if r < rows as usize
                            && c < cols as usize
                            && let pmacs::cell::Glyph::Char(ch) = cell.glyph
                        {
                            grid[r][c] = ch;
                        }
                    }
                }
            }
            pmacs_protocol::InstanceMessage::Cursor(Some(cs)) if cs.visible => {
                caret = Some((cs.coord.row, cs.coord.col));
            }
            _ => {}
        }
    }
    Frame {
        rows: grid.into_iter().map(|r| r.into_iter().collect()).collect(),
        caret,
    }
}

/// Words of prose, each distinct, so a row's text names where it broke.
const PROSE: &str = "the quick brown fox jumps over the lazy dog";

/// A paragraph wraps at the spaces: no row ends inside a word, and the
/// trailing space of each row is the break. Character wrap --- Q#LL5's
/// rule, what `wrap` meant before D35 --- would cut `quick`'s successor
/// as `the quick br`.
#[test]
fn a_paragraph_of_prose_wraps_at_the_spaces() {
    let s = session("prose");
    fill(&s, PROSE);
    let f = frame(&s, 8, 12);
    assert_eq!(
        &f.rows[..4],
        &[
            "the quick   ",
            "brown fox   ",
            "jumps over  ",
            "the lazy dog"
        ],
        "each row breaks after a space, never inside a word"
    );
    for row in &f.rows[..4] {
        for word in row.split_whitespace() {
            assert!(
                PROSE.split(' ').any(|w| w == word),
                "{word:?} is a cut word in {row:?}"
            );
        }
    }
}

/// A hyphen, an em dash and a slash are break points too, and the break
/// is after them: `well-` ends a row and `known` starts the next.
#[test]
fn a_hyphen_a_dash_and_a_slash_break_after_themselves() {
    let s = session("punctuation");
    fill(&s, "a well-known and/or long—winded");
    let f = frame(&s, 8, 8);
    assert_eq!(
        &f.rows[..5],
        &["a well- ", "known   ", "and/or  ", "long—   ", "winded  "],
    );
}

/// The 200-character word: wider than any row, so it starts a fresh row
/// and breaks by glyph, and every one of its characters is reachable ---
/// `C-f` over it puts the caret on a cell showing exactly the character
/// under the caret, at every step.
#[test]
fn a_two_hundred_character_word_wraps_and_every_character_is_reachable() {
    let mut s = session("long_word");
    let word: String = (0..200u8).map(|i| char::from(b'a' + i % 26)).collect();
    fill(&s, &format!("see {word} end"));
    let (rows, cols) = (12, 40);
    let f = frame(&s, rows, cols);
    assert_eq!(f.rows[0].trim_end(), "see", "the word starts a fresh row");
    assert_eq!(
        f.rows[1],
        word[..40],
        "then breaks by glyph, a full row at a time"
    );
    assert_eq!(f.rows[5], word[160..200]);
    assert_eq!(f.rows[6].trim_end(), "end");
    ctrl(&mut s, 'e');
    ctrl(&mut s, 'a');
    for _ in 0..4 {
        ctrl(&mut s, 'f');
    }
    for (i, expected) in word.chars().enumerate() {
        let f = frame(&s, rows, cols);
        let (r, c) = f
            .caret
            .unwrap_or_else(|| panic!("no caret on the word's character {i}"));
        let shown = f.rows[r as usize].chars().nth(c as usize);
        assert_eq!(
            shown,
            Some(expected),
            "character {i} of the word: the caret at ({r}, {c}) must be on the cell drawing it"
        );
        assert_eq!(cursor_byte(&s), 4 + i as u64);
        ctrl(&mut s, 'f');
    }
}

/// The caret is on the row the text is on, below wrapped lines, and
/// `C-a` / `C-e` take it to the line's first cell and past its last
/// character --- which is the first thing the grid's caret got wrong
/// under wrap before D35, whatever the break rule: it reckoned a screen
/// row as a source line.
#[test]
fn the_caret_c_a_and_c_e_land_on_the_wrapped_rows() {
    let mut s = session("caret");
    fill(&s, &format!("{PROSE}\nsecond {PROSE}"));
    let (rows, cols) = (12, 12);
    let _ = frame(&s, rows, cols);
    ctrl(&mut s, 'n');
    let f = frame(&s, rows, cols);
    // Line 0 is four rows plus the row its end sits on (a full last row).
    assert_eq!(f.rows[5].trim_end(), "second the");
    assert_eq!(f.caret, Some((5, 0)), "C-n: the second line's first cell");
    ctrl(&mut s, 'e');
    let f = frame(&s, rows, cols);
    assert_eq!(f.rows[9].trim_end(), "lazy dog");
    assert_eq!(f.caret, Some((9, 8)), "C-e: just past `dog`, on its row");
    ctrl(&mut s, 'a');
    let f = frame(&s, rows, cols);
    assert_eq!(f.caret, Some((5, 0)), "C-a: back to the line's first cell");
    // Into the middle of the paragraph: `fox` opens the line's third
    // row, because `quick brown ` filled the second.
    for _ in 0.."second the quick brown ".len() {
        ctrl(&mut s, 'f');
    }
    let f = frame(&s, rows, cols);
    assert_eq!(f.rows[6].trim_end(), "quick brown");
    assert_eq!(f.caret, Some((7, 0)));
    assert_eq!(&f.rows[7][..3], "fox");
}

/// A space typed at the right edge hangs, as the GPU's blank word does:
/// the caret goes to the next row's first cell, and the next word is
/// drawn there with no blank opening the row.
#[test]
fn a_space_typed_at_the_edge_hangs_and_the_next_word_opens_the_row() {
    let mut s = session("hanging");
    let (rows, cols) = (6, 10);
    let _ = frame(&s, rows, cols);
    type_str(&mut s, "abcd efghi");
    let f = frame(&s, rows, cols);
    assert_eq!(f.rows[0], "abcd efghi");
    type_str(&mut s, " ");
    let f = frame(&s, rows, cols);
    assert_eq!(
        f.caret,
        Some((1, 0)),
        "after the hanging space: the next row"
    );
    type_str(&mut s, "jk");
    let f = frame(&s, rows, cols);
    assert_eq!(f.rows[0], "abcd efghi");
    assert_eq!(
        f.rows[1].trim_end(),
        "jk",
        "the row opens with the word, not a blank"
    );
    assert_eq!(f.caret, Some((1, 2)));
}

/// A code buffer with a long line wraps somewhere sensible: after the
/// spaces between arguments, never inside an identifier.
#[test]
fn a_long_line_of_code_wraps_between_its_arguments() {
    let s = session("code");
    let line = "let result = compute(first_argument, second_argument, third);";
    fill(&s, line);
    let f = frame(&s, 6, 30);
    assert_eq!(
        &f.rows[..3],
        &[
            "let result =                  ",
            "compute(first_argument,       ",
            "second_argument, third);      ",
        ],
    );
}

/// The window follows the caret in screen rows. Ten paragraphs, each
/// wrapping to five rows, in a twelve-row window: reckoned in lines the
/// ninth paragraph is "on screen" at the top, and the caret was drawn
/// nowhere while the text scrolled nowhere.
#[test]
fn the_window_follows_the_caret_down_wrapped_paragraphs() {
    let mut s = session("follow");
    let text: Vec<String> = (0..10).map(|i| format!("para{i} {PROSE}")).collect();
    fill(&s, &text.join("\n"));
    let (rows, cols) = (12, 12);
    let _ = frame(&s, rows, cols);
    for _ in 0..8 {
        ctrl(&mut s, 'n');
    }
    let f = frame(&s, rows, cols);
    let (r, c) = f
        .caret
        .expect("the caret on screen after moving down eight paragraphs");
    assert_eq!(c, 0);
    assert!(
        f.rows[r as usize].starts_with("para8 the"),
        "the caret's row shows para8's first row: {:?} at {r}",
        f.rows
    );
}

/// A click on a continuation row lands on the character drawn there,
/// and a click past the end of a row the line continues from lands on
/// that row's last character, not a row below.
#[test]
fn a_click_on_a_wrapped_row_lands_where_it_points() {
    let mut s = session("click");
    fill(&s, &format!("{PROSE}\nsecond {PROSE}"));
    let (rows, cols) = (12, 12);
    let _ = frame(&s, rows, cols);
    // `fox` opens row 7 (the caret row above): its `o` is column 1.
    click(&mut s, 7, 1, rows, cols);
    let second = PROSE.len() as u64 + 1;
    assert_eq!(
        cursor_byte(&s),
        second + "second the quick brown f".len() as u64
    );
    // Row 5 is `second the`; column 11 is past it.
    click(&mut s, 5, 11, rows, cols);
    assert_eq!(
        cursor_byte(&s),
        second + "second the".len() as u64,
        "past the row's end: its trailing space, still on row 5"
    );
    let f = frame(&s, rows, cols);
    assert_eq!(f.caret, Some((5, 10)));
}

/// The grid's layout cost on a large markdown file, per painted frame:
/// E7d.1's measurement, taken before and after the word-wrap walk
/// (`--ignored`, a measurement and not a gate). The file is named by
/// `PMACS_E7D_MARKDOWN`; the frame is 50 x 120, and the caret walks the
/// file a line at a time with `C-n`, one frame each, so every paint
/// lays out a screenful of wrapped prose.
#[test]
#[ignore = "measurement: set PMACS_E7D_MARKDOWN and run with --ignored"]
fn measure_grid_frames_on_a_large_markdown_file() {
    let Ok(path) = std::env::var("PMACS_E7D_MARKDOWN") else {
        eprintln!("PMACS_E7D_MARKDOWN unset; nothing measured");
        return;
    };
    let text = std::fs::read_to_string(&path).expect("read the markdown file");
    let mut s = session("measure");
    s.lua_host
        .lua()
        .globals()
        .set("E7D_TEXT", text.as_str())
        .unwrap();
    exec(
        &s,
        "pmacs.window.buffer():insert(0, E7D_TEXT); pmacs.editor.goto_byte(0)",
    );
    let (rows, cols) = (50, 120);
    let size = pmacs::cell::CellSize::new(rows, cols);
    let mut rs = pmacs::instance_render::RenderState::new(size);
    let _ = rs.render_frame(&s, FrontendId::LOCAL, &HashMap::new(), &[]);
    let lines = text.lines().count();
    let mut samples = Vec::with_capacity(lines);
    for _ in 0..lines {
        ctrl(&mut s, 'n');
        let t = std::time::Instant::now();
        let _ = rs.render_frame(&s, FrontendId::LOCAL, &HashMap::new(), &[]);
        samples.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    let pick = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize];
    eprintln!(
        "E7D-MEASURE file={path} bytes={} lines={lines} frames={} p50={:.3}ms p90={:.3}ms p99={:.3}ms max={:.3}ms",
        text.len(),
        samples.len(),
        pick(0.5),
        pick(0.9),
        pick(0.99),
        samples[samples.len() - 1],
    );
}
