// tests/e7d_diagnostic_at_point_acceptance.rs --- E7d.2, the
// diagnostic's message at point.

//! When the caret is inside a diagnostic's range its message shows on
//! the status line, and leaving clears it: the severity and the first
//! line of the message, the most severe where several overlap, then
//! the innermost. Daemon-side and no wire change --- the grid paints it
//! on its status row and a semantic frontend reads it from
//! `StatusFacts.message`, which already existed.
//!
//! The diagnostics are E7c's: the fake in `didsave` mode publishes, on
//! a save, one `rustc` error per `CHECKME` in the saved text (`e7c:
//! check`) and on every change one native warning (`e7c: held`), so
//! after a save both sit on the same word and the error is the more
//! severe. The caret enters, moves within and leaves through the
//! production key dispatch.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::cell::{Cell, CellGrid, CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::{ByteRange, FrontendId, InstanceMessage};
use pmacs::semantic_render::SemanticRenderState;

#[path = "common/iso.rs"]
mod iso;

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn tick(s: &mut EditorState) {
    s.tick_processes();
    s.tick_lsp();
    s.tick_async();
}

fn pump_lua_flag(s: &mut EditorState, flag: &str, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        tick(s);
        let done: bool = s
            .lua_host
            .lua()
            .load(format!("return ({flag}) == true"))
            .eval()
            .unwrap_or(false);
        if done {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

const INITIALIZED: &str = "(function() \
   for _,r in ipairs(pmacs.lsp.list()) do \
     if r.state and r.state.kind=='initialized' then return true end \
   end \
   return false \
 end)()";

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmacs-e7d-dap-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const BODY: &str = "fn main() {\n    let x = CHECKME;\n}\n";

/// The fake in `didsave` mode on `<dir>/a.rs` holding `BODY`, the
/// handshake answered, with `check_sources` as given.
fn open_with_fake(tag: &str, check_sources: &str) -> (EditorState, PathBuf) {
    let dir = temp_dir(tag);
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.clone()));
    exec(&s, "pmacs.lsp.config = {}");
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust = {{
               command = {:?},
               env = {{ PMACS_FAKE_LSP_MODE = 'didsave' }},
               check_sources = {check_sources},
             }}",
            fake_lsp_path()
        ),
    );
    let file = dir.join("a.rs");
    std::fs::write(&file, BODY).unwrap();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    let mut s = s;
    assert!(pump_lua_flag(&mut s, INITIALIZED, 10), "fake server init");
    (s, file)
}

/// The diagnostics for the active document, one line each:
/// `<source> <severity> <line>:<col>-<end_line>:<end_col>`, in the
/// order the store holds them.
fn positions(s: &EditorState) -> Vec<String> {
    eval(
        s,
        "local rec = pmacs.lsp.active_attachment()
         local out = {}
         for _, d in ipairs(pmacs.diag.list(rec.uri)) do
           out[#out + 1] = string.format('%s %s %d:%d-%d:%d', tostring(d.source), tostring(d.severity),
             d.start_line, d.start_col, d.end_line, d.end_col)
         end
         return out",
    )
}

fn buffer_text(s: &EditorState) -> String {
    eval(
        s,
        "local buf = pmacs.window.buffer() return buf:slice(0, buf:len())",
    )
}

/// Tick until `pred` holds or `secs` pass.
fn wait(s: &mut EditorState, secs: u64, pred: impl Fn(&EditorState) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        tick(s);
        if pred(s) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
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

/// The grid frontend's status row at 8 x 120, trimmed.
fn status_row(s: &EditorState) -> String {
    let (rows, cols) = (8u32, 120u32);
    let size = CellSize::new(rows, cols);
    let mut cells = vec![Cell::default(); (rows * cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: cols,
        size,
    };
    pmacs::editor::paint_frame(s, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);
    (0..cols)
        .map(
            |col| match &cells[((rows - 1) * cols + col) as usize].glyph {
                Glyph::Char(c) => *c,
                _ => ' ',
            },
        )
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// A semantic frontend's `StatusFacts.message` after one frame, carried
/// forward in `last` while the message is suppressed as unchanged.
fn wire_message(s: &EditorState, r: &mut SemanticRenderState, last: &mut Option<String>) {
    for m in r.render_frame(s) {
        if let InstanceMessage::StatusFacts { message, .. } = m {
            *last = message;
        }
    }
}

fn semantic(s: &EditorState) -> SemanticRenderState {
    let buffer_id = s.core.borrow().active_window().buffer_id;
    let mut r = SemanticRenderState::new(FrontendId::LOCAL);
    r.set_viewport(
        buffer_id,
        ByteRange {
            start: 0,
            end: 1 << 20,
        },
        0,
    );
    r
}

/// Save so the check's error lands on the marker beside the native
/// warning, every arrival drained; the caret at byte 0.
fn error_and_warning_on_the_marker(tag: &str) -> EditorState {
    let (mut s, _file) = open_with_fake(tag, "{ 'rustc' }");
    assert!(
        wait(&mut s, 5, |s| positions(s).len() == 1),
        "the open's warning arrived: {:?}",
        positions(&s)
    );
    exec(
        &s,
        "_G.__e7d_saved = false
         pmacs.hook.add('buffer.after-save', function() _G.__e7d_saved = true end)
         pmacs.command.invoke('buffer.save')",
    );
    assert!(pump_lua_flag(&mut s, "_G.__e7d_saved", 10), "saved");
    assert!(
        wait(&mut s, 5, |s| positions(s).len() == 2),
        "the check's error arrived beside the warning: {:?}",
        positions(&s)
    );
    for _ in 0..20 {
        tick(&mut s);
    }
    exec(&s, "pmacs.editor.goto_byte(0)");
    // A save's own message would outrank the diagnostic; the rows are
    // about the caret, so the command text is cleared as the next
    // keystroke would.
    s.core.borrow_mut().status.clear();
    s
}

/// The row: entering the check's error shows `error: e7c: check` on
/// both frontends --- the error and not the warning under it, the more
/// severe --- moving within it keeps it, and leaving it on either side
/// clears it.
#[test]
fn e7d_2_the_caret_entering_moving_within_and_leaving_a_diagnostic() {
    let mut s = error_and_warning_on_the_marker("enter");
    let mut r = semantic(&s);
    let mut wire = Some("unset".to_owned());
    let marker = buffer_text(&s).find("CHECKME").expect("the marker") as u64;
    let expected = "error: e7c: check";

    wire_message(&s, &mut r, &mut wire);
    assert!(
        !status_row(&s).contains("e7c:"),
        "at byte 0: {:?}",
        status_row(&s)
    );
    assert_eq!(wire, None, "at byte 0, the wire's message is clear");

    // One short of the marker: still outside.
    exec(&s, &format!("pmacs.editor.goto_byte({})", marker - 1));
    wire_message(&s, &mut r, &mut wire);
    assert!(!status_row(&s).contains("e7c:"));
    assert_eq!(wire, None);

    // C-f enters it.
    key(&mut s, KeyCode::Char('f'), KeyModifiers::CONTROL);
    assert_eq!(s.core.borrow().active_window().cursor, marker);
    wire_message(&s, &mut r, &mut wire);
    assert_eq!(status_row(&s), expected, "entered: the grid's status row");
    assert_eq!(wire.as_deref(), Some(expected), "entered: StatusFacts");

    // Within: to its last character.
    for _ in 0..6 {
        key(&mut s, KeyCode::Char('f'), KeyModifiers::CONTROL);
        wire_message(&s, &mut r, &mut wire);
        assert_eq!(
            status_row(&s),
            expected,
            "within, at {}",
            s.core.borrow().active_window().cursor
        );
        assert_eq!(wire.as_deref(), Some(expected));
    }

    // One more leaves it: the caret is past `CHECKME`.
    key(&mut s, KeyCode::Char('f'), KeyModifiers::CONTROL);
    assert_eq!(s.core.borrow().active_window().cursor, marker + 7);
    wire_message(&s, &mut r, &mut wire);
    assert!(
        !status_row(&s).contains("e7c:"),
        "left: {:?}",
        status_row(&s)
    );
    assert_eq!(wire, None, "left: the wire's message clears");

    // And back in from the right, with C-b.
    key(&mut s, KeyCode::Char('b'), KeyModifiers::CONTROL);
    wire_message(&s, &mut r, &mut wire);
    assert_eq!(status_row(&s), expected);
    assert_eq!(wire.as_deref(), Some(expected));
}

/// Before the save only the native warning is on the marker, and it
/// shows under its own severity.
#[test]
fn e7d_2_a_warning_alone_shows_as_a_warning() {
    let (mut s, _file) = open_with_fake("warning", "{ 'rustc' }");
    assert!(
        wait(&mut s, 5, |s| positions(s).len() == 1),
        "the open's warning arrived: {:?}",
        positions(&s)
    );
    let marker = buffer_text(&s).find("CHECKME").expect("the marker") as u64;
    exec(&s, &format!("pmacs.editor.goto_byte({})", marker + 3));
    s.core.borrow_mut().status.clear();
    let mut r = semantic(&s);
    let mut wire = None;
    wire_message(&s, &mut r, &mut wire);
    assert_eq!(status_row(&s), "warning: e7c: held");
    assert_eq!(wire.as_deref(), Some("warning: e7c: held"));
}

/// A command's text outranks the diagnostic at point, and the
/// diagnostic returns when the text clears: the derived message never
/// overwrote it.
#[test]
fn e7d_2_a_command_message_outranks_it_and_it_returns() {
    let mut s = error_and_warning_on_the_marker("outrank");
    let marker = buffer_text(&s).find("CHECKME").expect("the marker") as u64;
    exec(&s, &format!("pmacs.editor.goto_byte({})", marker + 2));
    exec(&s, "pmacs.editor.set_status('12 references')");
    let mut r = semantic(&s);
    let mut wire = None;
    wire_message(&s, &mut r, &mut wire);
    assert_eq!(status_row(&s), "12 references");
    assert_eq!(wire.as_deref(), Some("12 references"));
    s.core.borrow_mut().status.clear();
    wire_message(&s, &mut r, &mut wire);
    assert_eq!(status_row(&s), "error: e7c: check");
    assert_eq!(wire.as_deref(), Some("error: e7c: check"));
    let _ = &mut s;
}
