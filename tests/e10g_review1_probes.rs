//! E10g review 1's probes: the states a user reaches that the handoff's
//! witnesses did not choose, committed by fix round 1 as witnesses
//! (resume §4). Each is named for the row it holds the head to, and
//! fails at the reviewed head `c71dad7` for the reason the review gives.
//!
//! - P1 and P1b (E10g.6): the line-number gutter, on by default and a
//!   chrome surface with a face of its own (`ui.gutter`), held to the
//!   second clause as ruled, in process and through a real daemon and the
//!   attach client's own reduction. The owner ruled on 2026-10-29 that
//!   `default-dark`'s gutter is a known failure of that clause, #428, so
//!   both pin it under that issue rather than require it to pass (review
//!   1's way (a)).

use std::collections::HashMap;

use pmacs::cell::{Cell, CellSize, Color, Style};
use pmacs::color_depth::{ColorDepth, legibility_violations};
use pmacs::editor::EditorState;
use pmacs::instance_render::RenderState;
use pmacs::protocol::{FrontendId, InstanceMessage};

#[path = "common/editor.rs"]
mod editor;
use editor::{eval, exec};

mod common;
use common::iso;

fn fresh() -> EditorState {
    EditorState::new_with_roots(&iso::roots())
}

/// One whole frame painted at `depth`, as a row-major grid (the first
/// frame diffs against a blank grid, so every cell is named or default).
fn frame_at(s: &EditorState, size: CellSize, depth: ColorDepth) -> Vec<Vec<Cell>> {
    let mut r = RenderState::new(size);
    r.set_color_depth(depth);
    let mut grid = vec![vec![Cell::default(); size.cols as usize]; size.rows as usize];
    for message in r.render_frame(s, FrontendId::LOCAL, &HashMap::new(), &[]) {
        if let InstanceMessage::CellDelta { spans, .. } = message {
            for span in spans {
                for (i, cell) in span.cells.into_iter().enumerate() {
                    grid[span.start.row as usize][span.start.col as usize + i] = cell;
                }
            }
        }
    }
    grid
}

fn glyphs(cells: &[Cell]) -> String {
    cells
        .iter()
        .map(|c| match c.glyph {
            pmacs::cell::Glyph::Char(ch) => ch,
            _ => '\0',
        })
        .collect()
}

/// The first `(row, col)` where `text` is drawn.
fn find_anywhere(grid: &[Vec<Cell>], text: &str) -> (usize, usize) {
    for (row, cells) in grid.iter().enumerate() {
        let line = glyphs(cells);
        if let Some(byte) = line.find(text) {
            return (row, line[..byte].chars().count());
        }
    }
    panic!("{text:?} is not drawn");
}

// ---------------------------------------------------------------------------
// P1, P1b — E10g.6: the gutter at 16 colors
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Path16 {
    /// In process: each theme color its authored 16.
    InProcess,
    /// Attached, as the suite's row simulates it: the frame at 256, then
    /// the client's table.
    Attached,
}

fn frame_16(s: &EditorState, size: CellSize, path: Path16) -> Vec<Vec<Cell>> {
    match path {
        Path16::InProcess => frame_at(s, size, ColorDepth::Sixteen),
        Path16::Attached => {
            let mut grid = frame_at(s, size, ColorDepth::TwoFiftySix);
            for cell in grid.iter_mut().flatten() {
                let at16 = |c| pmacs::color_depth::reduce(c, ColorDepth::Sixteen);
                cell.style.fg = at16(cell.style.fg);
                cell.style.bg = at16(cell.style.bg);
            }
            grid
        }
    }
}

/// The gutter's cells on a frame of a window showing "hello": the cell
/// holding line 1's number, and a gutter cell on a row past the last
/// line. Asserts both are in the gutter (left of the text).
fn gutter_surfaces(frame: &[Vec<Cell>]) -> Vec<(String, Style)> {
    let (row, text_col) = find_anywhere(frame, "hello");
    assert_eq!(row, 0, "the text is on the first row");
    let number_col = glyphs(&frame[0])
        .chars()
        .position(|c| c == '1')
        .expect("line 1's number is drawn");
    assert!(number_col < text_col, "the number sits in the gutter");
    vec![
        ("gutter, a number".to_owned(), frame[0][number_col].style),
        ("gutter, past the last line".to_owned(), frame[8][0].style),
    ]
}

fn buffer_face_set(s: &EditorState) -> bool {
    s.syntax_registry
        .theme()
        .lock()
        .unwrap()
        .buffer_face()
        .is_some()
}

/// #428: `default-dark`'s gutter, `Indexed(8)` on the terminal's own
/// background, which the second clause cannot judge (the owner's ruling
/// of 2026-10-29 on review 1's Medium 1).
const GUTTER_428: [&str; 2] = [
    "gutter, a number: its foreground does not differ from its background (Index(8) on DefaultBg)",
    "gutter, past the last line: its foreground does not differ from its background (Index(8) on DefaultBg)",
];

/// E10g.6 for the gutter. Line numbers are on by default
/// (`LineNumberMode::Absolute`, `src/window.rs`), and the gutter is a
/// `ui.*` surface with its own face (`ui.gutter`, Q#TH5), so it is a
/// chrome surface the row's "each chrome surface as actually painted"
/// names; the suite's `chrome_at_16` samples eleven surfaces and not it.
/// Every shipped theme must pass the second clause there on both paths,
/// but for `default-dark`'s gutter, color 8 on the terminal's own
/// background, which is #428 (the owner's ruling of 2026-10-29) and is
/// pinned here by its two lines on each path. The control is a theme
/// with a buffer face, whose gutter passes.
#[test]
fn e10g_review1_p1_the_gutter_is_legible_at_16_colors_on_both_paths() {
    let size = CellSize::new(16, 60);
    for path in [Path16::InProcess, Path16::Attached] {
        let s = fresh();
        exec(
            &s,
            r#"pmacs.theme.define("control", { faces = {
                 ["ui.default"] = { fg = 7, bg = 0 },
                 ["ui.modeline"] = { fg = 0, bg = 6 },
               } })
               pmacs.theme.load("control")
               pmacs.window.buffer():insert(0, "hello")"#,
        );
        let frame = frame_16(&s, size, path);
        let found = legibility_violations(
            &frame[8][30].style,
            &gutter_surfaces(&frame),
            buffer_face_set(&s),
        );
        assert!(found.is_empty(), "{path:?} control: {found:#?}");
    }

    let shipped: Vec<String> = eval(&fresh(), "return pmacs.theme.names()");
    let mut found = Vec::new();
    for path in [Path16::InProcess, Path16::Attached] {
        for theme in &shipped {
            let s = fresh();
            exec(&s, &format!("pmacs.theme.load({theme:?})"));
            exec(&s, r#"pmacs.window.buffer():insert(0, "hello")"#);
            let frame = frame_16(&s, size, path);
            let lines = legibility_violations(
                &frame[8][30].style,
                &gutter_surfaces(&frame),
                buffer_face_set(&s),
            );
            if theme == "default-dark" {
                assert_eq!(lines, GUTTER_428, "{path:?} {theme}: #428, pinned");
                continue;
            }
            for v in lines {
                found.push(format!("{path:?} {theme}: {v}"));
            }
        }
    }
    assert!(found.is_empty(), "{found:#?}");
}

/// P1 through the attached path itself rather than the suite's
/// simulation of it: a real daemon running the default theme paints an
/// attachment whose `true_color` is false, and the attach client's own
/// reduction (`reduce_spans_to_16`, what `frontend::write_frame` runs at
/// 16) takes it to the sixteen. The modeline and the status line, which
/// the suite samples, are the control; the gutter is the probe, and on
/// this path too it is #428's two lines, pinned. The suite's own attached
/// path (`frame_16`'s `Attached`) is a simulation, the frame at 256 and
/// the table in the test; this row is the real one.
#[test]
fn e10g_review1_p1b_the_attached_path_through_a_daemon_and_the_client() {
    use pmacs_protocol::transport::{read_message, write_message};
    let size = CellSize::new(16, 60);
    let daemon = common::daemon::TestDaemon::spawn_with_config(
        r#"pmacs.window.buffer():insert(0, "hello")"#,
    );
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .expect("read timeout");
    let _hello: pmacs_protocol::message::Hello = read_message(&mut stream).expect("Hello");
    let request = pmacs_protocol::message::AttachRequest {
        protocol_version: pmacs_protocol::message::PROTOCOL_VERSION,
        frontend_capabilities: pmacs_protocol::message::FrontendCapabilities {
            true_color: false,
            ..common::daemon::build_default_caps()
        },
        initial_size: size,
    };
    write_message(&mut stream, &request).expect("AttachRequest");
    let mut grid = vec![vec![Cell::default(); size.cols as usize]; size.rows as usize];
    loop {
        let message: InstanceMessage = read_message(&mut stream).expect("a frame");
        if let InstanceMessage::CellDelta {
            spans,
            full_grid: true,
        } = message
        {
            for span in pmacs::color_depth::reduce_spans_to_16(&spans) {
                for (i, cell) in span.cells.into_iter().enumerate() {
                    grid[span.start.row as usize][span.start.col as usize + i] = cell;
                }
            }
            break;
        }
    }
    for cell in grid.iter().flatten() {
        for color in [cell.style.fg, cell.style.bg] {
            assert!(
                matches!(color, Color::Default | Color::Indexed(0..=15)),
                "a 16-color frame: {color:?}"
            );
        }
    }
    let buffer = grid[8][30].style;
    let control = vec![
        ("modeline".to_owned(), grid[14][1].style),
        ("status line".to_owned(), grid[15][1].style),
    ];
    assert_eq!(
        legibility_violations(&buffer, &control, false),
        Vec::<String>::new(),
        "the control: the surfaces the suite samples pass here too"
    );
    let found = legibility_violations(&buffer, &gutter_surfaces(&grid), false);
    assert_eq!(found, GUTTER_428, "#428, pinned, through the real path");
}
