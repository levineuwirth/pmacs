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
//! - P2 and P2b (E10g.2): a theme that sets `ui.menu` alone, or only the
//!   bare `ui` catch-all, keeps the selected row of the candidate band,
//!   the completion popup and the context menu apart from the other rows,
//!   in the grid and in the faces the daemon ships to the GPU.
//! - P3 (E10g.4): a prompt that opens over the picker does not leave the
//!   picker's preview in force.

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

// ---------------------------------------------------------------------------
// P2, P2b — E10g.2: the selected row under a theme that sets only ui.menu
// ---------------------------------------------------------------------------

/// The selected row's style and an unselected row's style, read from one
/// truecolor frame each, for the completion popup, the minibuffer's
/// candidate band and the context menu.
fn selected_and_row(s: &EditorState) -> Vec<(&'static str, Style, Style)> {
    use pmacs::completion::{
        CompletionEditsCarry, CompletionItemKind, CompletionPopupState, PopupCandidate,
    };
    let size = CellSize::new(16, 60);
    let mut out = Vec::new();
    let candidate = |label: &str| PopupCandidate {
        carry: CompletionEditsCarry::default(),
        label: label.to_owned(),
        kind: CompletionItemKind::Text,
        detail: None,
        insert_text: label.to_owned(),
    };
    let popup = {
        let core = s.core.borrow();
        CompletionPopupState::new(
            core.active_buffer_id(),
            0,
            String::new(),
            vec![candidate("gamma"), candidate("delta")],
            2,
        )
        .expect("a popup")
    };
    s.core.borrow_mut().completion_popup_open(popup);
    exec(
        s,
        r#"pmacs.minibuffer.read {
             prompt = "Pick: ",
             source = function() return { "one", "two" } end,
             on_accept = function() end,
           }"#,
    );
    let frame = frame_at(s, size, ColorDepth::TrueColor);
    let (row, col) = find_anywhere(&frame, "gamma");
    out.push((
        "completion popup",
        frame[row][col].style,
        frame[row + 1][col].style,
    ));
    let (row, col) = find_anywhere(&frame, "one");
    out.push((
        "candidate band",
        frame[row][col].style,
        frame[row + 1][col].style,
    ));
    exec(s, "pmacs.minibuffer.cancel()");
    s.core.borrow_mut().completion_popup_close();

    s.core.borrow_mut().menu_open(
        vec![
            pmacs::menu::MenuRow::Item {
                label: "Alpha".into(),
                command: "editor.copy".into(),
            },
            pmacs::menu::MenuRow::Item {
                label: "Beta".into(),
                command: "editor.paste".into(),
            },
        ],
        (2, 20),
    );
    let frame = frame_at(s, size, ColorDepth::TrueColor);
    let (row, col) = find_anywhere(&frame, "Alpha");
    out.push((
        "context menu",
        frame[row][col].style,
        frame[row + 1][col].style,
    ));
    s.core.borrow_mut().menu_close();
    out
}

/// E10g.2 keeps today's constant for every surface whose face the theme
/// does not set. A theme that sets `ui.menu` alone does not set
/// `ui.menu.selected`, yet the theme's walk (`Theme::face`: an absent
/// name falls back to its parent, then to bare `ui`) resolves the
/// selected row to `ui.menu`'s own pair, and the selection disappears
/// from the M-x band, the completion popup and the context menu. The
/// same for a theme or `init.lua` that sets only the bare `ui`
/// catch-all, which at `5ae5a27` reached none of these surfaces. The
/// control is the unset theme, where each selected row is distinct.
#[test]
fn e10g_review1_p2_a_theme_setting_only_ui_menu_keeps_the_selection_visible() {
    let s = fresh();
    exec(&s, r#"pmacs.window.buffer():insert(0, "hello")"#);
    for (what, selected, row) in selected_and_row(&s) {
        assert_ne!(selected, row, "the control, unset: {what}");
    }

    let mut found = Vec::new();
    for (case, merge) in [
        (
            "ui.menu alone",
            r#"pmacs.theme.merge { ["ui.menu"] = { fg = {220, 220, 220}, bg = {40, 40, 48} } }"#,
        ),
        (
            "the bare ui catch-all alone",
            "pmacs.theme.merge { ui = { fg = {220, 220, 220}, bg = {40, 40, 48} } }",
        ),
    ] {
        let s = fresh();
        exec(&s, r#"pmacs.window.buffer():insert(0, "hello")"#);
        exec(&s, merge);
        for (what, selected, row) in selected_and_row(&s) {
            if selected == row {
                found.push(format!(
                    "{case}: {what}: the selected row is painted as every other row ({selected:?})"
                ));
            }
        }
    }
    assert!(found.is_empty(), "{found:#?}");
}

/// P2 on the GPU's side of the wire: what the daemon ships in
/// `ThemeFacts` for a theme that sets `ui.menu` alone (and, likewise,
/// `ui.scrollbar` alone). The GPU paints the selected row's wash and
/// glyphs from `ui.menu.selected` whenever that face arrives
/// (`face_wash_or("ui.menu.selected", MENU_SELECTED_BG)`,
/// `selected_row_color`), and the scrollbar thumb from
/// `ui.scrollbar.thumb`; a child that arrives as its parent's copy paints
/// the selection, and the thumb, in the row's and the track's own color.
#[test]
fn e10g_review1_p2b_the_daemon_does_not_ship_a_selected_face_copied_from_its_parent() {
    use pmacs::semantic_render::SemanticRenderState;
    let s = fresh();
    exec(
        &s,
        r#"pmacs.theme.merge {
             ["ui.menu"] = { fg = {220, 220, 220}, bg = {40, 40, 48} },
             ["ui.scrollbar"] = { bg = {40, 40, 48} },
           }"#,
    );
    let mut r = SemanticRenderState::new(FrontendId::LOCAL);
    let buffer_id = s.core.borrow().active_window().buffer_id;
    r.set_viewport(
        buffer_id,
        pmacs::protocol::ByteRange { start: 0, end: 64 },
        0,
    );
    let faces = r
        .render_frame(&s)
        .into_iter()
        .find_map(|m| match m {
            InstanceMessage::ThemeFacts { faces } => Some(faces),
            _ => None,
        })
        .expect("the first frame carries the face table");
    let style_of = |name: &str| faces.iter().find(|f| f.name == name).map(|f| f.style);
    let mut found = Vec::new();
    for (child, parent) in [
        ("ui.menu.selected", "ui.menu"),
        ("ui.scrollbar.thumb", "ui.scrollbar"),
    ] {
        let parent_style = style_of(parent).unwrap_or_else(|| panic!("{parent} ships"));
        if style_of(child) == Some(parent_style) {
            found.push(format!(
                "{child} ships as {parent}'s copy ({parent_style:?}); the GPU paints it in place of its constant"
            ));
        }
    }
    assert!(found.is_empty(), "{found:#?}");
}

// ---------------------------------------------------------------------------
// P3 — E10g.4: a prompt that opens over the picker
// ---------------------------------------------------------------------------

fn roots_for(name: &str) -> pmacs::bootstrap::BootstrapRoots {
    let base = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("e10g-review1")
        .join(name);
    let _ = std::fs::remove_dir_all(&base);
    let roots = pmacs::bootstrap::BootstrapRoots::isolated_under(&base);
    for (_, dir) in roots.child_env() {
        std::fs::create_dir_all(&dir).expect("create a controlled root");
    }
    for dir in [roots.state_dir(), roots.config_dir()] {
        std::fs::create_dir_all(dir.expect("isolated roots name it")).expect("create");
    }
    roots
}

fn press(
    s: &mut EditorState,
    code: crossterm::event::KeyCode,
    mods: crossterm::event::KeyModifiers,
) {
    s.dispatch_key(
        FrontendId::LOCAL,
        crossterm::event::KeyEvent {
            code,
            modifiers: mods,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        },
    );
}

fn loaded(s: &EditorState) -> Option<String> {
    eval(s, "return pmacs.theme.loaded()")
}

/// E10g.4: the picker's preview lasts while the picker does; `RET` keeps
/// it and `C-g` restores the theme in force at open. A prompt that opens
/// while the picker is up replaces it (`Minibuffer::begin` drops the
/// open session without calling its `on_cancel`, as `finder.lua:316`
/// records), so neither runs: `lsp.code-actions`' chooser, which opens
/// when the server's answer arrives (`lsp.lua:4060`), reached from the
/// context menu's "Quick Fix" by a right click, which `dispatch_mouse`
/// takes while the minibuffer is up. Its `C-g` then leaves the previewed
/// theme loaded, `ui.theme` naming the theme before, nothing kept.
#[test]
fn e10g_review1_p3_a_prompt_that_opens_over_the_picker_does_not_keep_its_preview() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let roots = roots_for("replaced");
    let config = roots.config_dir().expect("config dir");
    std::fs::write(
        config.join("init.lua"),
        r#"pmacs.theme.define("plain-a", { faces = { ["ui.default"] = { bg = {1, 1, 1} } } })
           pmacs.theme.define("plain-b", { faces = { ["ui.default"] = { bg = {2, 2, 2} } } })"#,
    )
    .expect("write init.lua");
    let mut s = EditorState::new_with_roots(&roots);
    s.install_state_dirs();
    assert_eq!(loaded(&s).as_deref(), Some("default-dark"));

    press(&mut s, KeyCode::Char('x'), KeyModifiers::ALT);
    for ch in "theme-select".chars() {
        press(&mut s, KeyCode::Char(ch), KeyModifiers::NONE);
    }
    press(&mut s, KeyCode::Enter, KeyModifiers::NONE);
    press(&mut s, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(loaded(&s).as_deref(), Some("plain-a"), "the preview");

    exec(
        &s,
        r#"pmacs.minibuffer.read {
             prompt = "Code action (2): ",
             source = function() return { "1: first", "2: second" } end,
             on_accept = function() end,
           }"#,
    );
    press(&mut s, KeyCode::Char('g'), KeyModifiers::CONTROL);
    assert!(
        !s.core.borrow().minibuffer.is_active(),
        "every prompt closed"
    );
    let setting: String = eval(&s, "return pmacs.config.get('ui.theme')");
    assert_eq!(setting, "default-dark", "nothing was accepted");
    assert_eq!(
        loaded(&s).as_deref(),
        Some("default-dark"),
        "the preview outlived the picker"
    );
    assert!(
        !roots.state_dir().unwrap().join("theme").exists(),
        "nothing kept"
    );
}
