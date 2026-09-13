// tests/minibuffer_band_acceptance.rs --- E6.2: the grid's candidate band.

//! The minibuffer's candidates as the grid frontend paints them: a
//! band of rows directly above the prompt, painted through one real
//! `paint_frame`, with the selection in reverse video and `Up`/`Down`
//! moving it visibly. Every row here reads the painted cells, not the
//! session, because the row's claim is about what is on screen.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::cell::{CellGrid, CellSize, Glyph, Style};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

const ROWS: u32 = 24;
const COLS: u32 = 80;

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

fn fresh() -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    s
}

fn candidates(s: &EditorState) -> Vec<String> {
    eval(s, "return pmacs.minibuffer.candidates()")
}

/// One painted frame: the text of every row and the style of every
/// cell, at `rows` x `cols`.
struct Frame {
    text: Vec<String>,
    styles: Vec<Vec<Style>>,
}

fn paint(s: &EditorState, rows: u32, cols: u32) -> Frame {
    let size = CellSize::new(rows, cols);
    let mut cells = vec![pmacs::cell::Cell::default(); (rows * cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: cols,
        size,
    };
    pmacs::editor::paint_frame(s, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);
    let text = (0..rows)
        .map(|row| {
            (0..cols)
                .map(|col| match &cells[(row * cols + col) as usize].glyph {
                    Glyph::Char(ch) => *ch,
                    Glyph::Cluster(_) => '?',
                    Glyph::Continuation => ' ',
                })
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect();
    let styles = (0..rows)
        .map(|row| {
            (0..cols)
                .map(|col| cells[(row * cols + col) as usize].style)
                .collect()
        })
        .collect();
    Frame { text, styles }
}

impl Frame {
    /// Whether row `row` is painted in the band's selected style: every
    /// cell of the row in reverse video.
    fn row_is_selected(&self, row: u32) -> bool {
        self.styles[row as usize].iter().all(|st| st.reverse)
    }

    /// The label painted on band row `row` (the text after the glyph
    /// column, up to the two-space detail separator).
    fn label(&self, row: u32) -> String {
        let line = &self.text[row as usize];
        let body = line.get(2..).unwrap_or("");
        body.split("  ").next().unwrap_or("").to_owned()
    }

    /// The rows above the prompt that are painted as the band: contiguous
    /// rows ending at `rows - 2` whose cells carry the popup background
    /// or reverse video AND whose label is one of `cands`. The label
    /// test is what tells a band row from the mode line, which is also
    /// reverse video and sits exactly where a one-row band would.
    fn band_rows(&self, cands: &[String]) -> Vec<u32> {
        let rows = self.text.len() as u32;
        let mut out = Vec::new();
        let mut row = rows - 2;
        loop {
            let st = self.styles[row as usize][1];
            let styled = st.reverse || st.bg == pmacs::cell::Color::Indexed(236);
            if !styled || !cands.contains(&self.label(row)) {
                break;
            }
            out.push(row);
            if row == 0 {
                break;
            }
            row -= 1;
        }
        out.reverse();
        out
    }

    /// Whether any cell above the prompt carries the popup background.
    fn any_popup_background_above_the_prompt(&self) -> bool {
        let rows = self.styles.len();
        self.styles[..rows - 1]
            .iter()
            .flatten()
            .any(|st| st.bg == pmacs::cell::Color::Indexed(236))
    }
}

/// The band paints the candidates above the prompt, best first, with
/// the selection in reverse video; `Down` and `Up` move it visibly.
#[test]
fn band_paints_the_candidates_above_the_prompt_and_the_selection_moves() {
    let mut s = fresh();
    alt(&mut s, 'x');
    type_str(&mut s, "hel");
    let cands = candidates(&s);
    assert!(
        cands.len() >= 3,
        "fixture premise: `hel` matches several commands; got {cands:?}"
    );

    let f = paint(&s, ROWS, COLS);
    assert!(
        f.text[(ROWS - 1) as usize].starts_with("M-x hel"),
        "the prompt row is the last; got {:?}",
        f.text[(ROWS - 1) as usize]
    );
    let band = f.band_rows(&cands);
    let shown = cands.len().min(10);
    assert_eq!(
        band.len(),
        shown,
        "one band row per candidate up to ten, directly above the prompt; got {band:?}"
    );
    for (i, row) in band.iter().enumerate() {
        assert_eq!(
            f.label(*row),
            cands[i],
            "band row {i} carries candidate {i}"
        );
    }
    assert!(
        f.row_is_selected(band[0]),
        "the first candidate is selected"
    );
    assert!(!f.row_is_selected(band[1]));
    assert_eq!(f.label(band[0]), "help");

    press(&mut s, KeyCode::Down);
    let f = paint(&s, ROWS, COLS);
    let band = f.band_rows(&cands);
    assert!(
        !f.row_is_selected(band[0]),
        "Down: the first row is no longer selected"
    );
    assert!(f.row_is_selected(band[1]), "Down: the second row is");

    press(&mut s, KeyCode::Up);
    let f = paint(&s, ROWS, COLS);
    let band = f.band_rows(&cands);
    assert!(f.row_is_selected(band[0]), "Up: back to the first");
    assert!(!f.row_is_selected(band[1]));
}

/// The command source's detail rides the row, first line only, as the
/// wire's row does.
#[test]
fn band_rows_carry_the_command_description() {
    let mut s = fresh();
    alt(&mut s, 'x');
    type_str(&mut s, "hel");
    let cands = candidates(&s);
    let f = paint(&s, ROWS, COLS);
    let band = f.band_rows(&cands);
    let first = &f.text[band[0] as usize];
    let description: String = eval(&s, "return pmacs.describe.command('help').description");
    let first_line = description.lines().next().unwrap_or("");
    assert!(
        first.contains(first_line),
        "the `help` row carries its description; row {first:?}, description {first_line:?}"
    );
}

/// Probe: a long list windows around the selection, so a selection
/// past the tenth candidate is still on screen and highlighted.
#[test]
fn band_windows_around_a_selection_past_the_visible_rows() {
    let mut s = fresh();
    alt(&mut s, 'x');
    let cands = candidates(&s);
    assert!(
        cands.len() > 20,
        "fixture premise: more than twenty commands"
    );
    for _ in 0..15 {
        press(&mut s, KeyCode::Down);
    }
    let f = paint(&s, ROWS, COLS);
    let band = f.band_rows(&cands);
    assert_eq!(band.len(), 10, "ten rows shown of a longer list");
    let highlighted: Vec<u32> = band
        .iter()
        .copied()
        .filter(|r| f.row_is_selected(*r))
        .collect();
    assert_eq!(
        highlighted.len(),
        1,
        "exactly one selected row; got {highlighted:?}"
    );
    assert_eq!(
        f.label(highlighted[0]),
        cands[15],
        "the selected row is the sixteenth candidate"
    );
    // The window is centered on the selection, as the wire's slice for
    // a semantic frontend is (`minibuffer_window`): ten rows starting
    // five above the selection, so both frontends show the same rows.
    assert_eq!(
        f.label(band[0]),
        cands[10],
        "the band's first row is five candidates above the selection"
    );
    assert_eq!(
        highlighted[0], band[5],
        "the selection sits on the sixth band row"
    );
}

/// Probe: `Up` from the top wraps to the last candidate, and the band
/// scrolls so the last is visible and highlighted.
#[test]
fn up_from_the_top_wraps_to_the_last_candidate_and_shows_it() {
    let mut s = fresh();
    alt(&mut s, 'x');
    let cands = candidates(&s);
    press(&mut s, KeyCode::Up);
    let f = paint(&s, ROWS, COLS);
    let band = f.band_rows(&cands);
    let highlighted: Vec<u32> = band
        .iter()
        .copied()
        .filter(|r| f.row_is_selected(*r))
        .collect();
    assert_eq!(highlighted.len(), 1);
    assert_eq!(f.label(highlighted[0]), *cands.last().expect("candidates"));
}

/// Probe: a prompt with no candidates paints no band; the rows above
/// the prompt are what they were before it opened.
#[test]
fn a_prompt_without_candidates_paints_no_band() {
    let mut s = fresh();
    let before = paint(&s, ROWS, COLS);
    exec(
        &s,
        "pmacs.minibuffer.read { prompt = 'Free: ', on_accept = function() end }",
    );
    type_str(&mut s, "text");
    let f = paint(&s, ROWS, COLS);
    assert!(
        candidates(&s).is_empty(),
        "fixture premise: a free-text prompt"
    );
    assert!(
        !f.any_popup_background_above_the_prompt(),
        "no candidates, no band"
    );
    for row in 0..(ROWS - 2) {
        assert_eq!(
            f.text[row as usize], before.text[row as usize],
            "row {row} above the prompt is untouched"
        );
    }
    assert!(f.text[(ROWS - 1) as usize].starts_with("Free: text"));
}

/// Probe: on a short terminal the band takes what fits and keeps the
/// selection inside it.
#[test]
fn band_fits_a_short_terminal_and_keeps_the_selection_visible() {
    let mut s = fresh();
    alt(&mut s, 'x');
    for _ in 0..5 {
        press(&mut s, KeyCode::Down);
    }
    let cands = candidates(&s);
    let f = paint(&s, 4, COLS);
    let band = f.band_rows(&cands);
    assert!(
        !band.is_empty() && band.len() <= 3,
        "at most three rows fit above the prompt on four; got {band:?}"
    );
    let highlighted: Vec<u32> = band
        .iter()
        .copied()
        .filter(|r| f.row_is_selected(*r))
        .collect();
    assert_eq!(
        highlighted.len(),
        1,
        "the selection is visible; band {band:?}"
    );
    assert_eq!(f.label(highlighted[0]), cands[5]);
}

/// Probe: in a files prompt a directory's row carries `/` in the glyph
/// column and a file's a blank.
#[test]
fn files_band_marks_directories_with_a_slash() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(td.path().join("sub")).expect("mkdir");
    std::fs::write(td.path().join("anchor.txt"), b"a\n").expect("write");
    let mut s = fresh();
    let anchor = td.path().join("anchor.txt").display().to_string();
    exec(&s, &format!("pmacs.buffer.find_or_open({anchor:?})"));
    ctrl(&mut s, 'x');
    ctrl(&mut s, 'f');
    let cands = candidates(&s);
    let f = paint(&s, ROWS, COLS);
    let band = f.band_rows(&cands);
    let mut seen = 0;
    for row in band {
        let line = &f.text[row as usize];
        let label = f.label(row);
        let glyph = line.chars().next().unwrap_or(' ');
        match label.as_str() {
            "sub" => {
                assert_eq!(glyph, '/', "a directory row carries the slash");
                seen += 1;
            }
            "anchor.txt" => {
                assert_eq!(glyph, ' ', "a file row carries a blank");
                seen += 1;
            }
            _ => {}
        }
    }
    assert_eq!(seen, 2, "both entries are in the band");
}

#[path = "common/iso.rs"]
mod iso;
