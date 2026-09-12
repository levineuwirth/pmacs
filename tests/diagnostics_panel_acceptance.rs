//! E5.4 --- `*diagnostics*`: a listview over `pmacs.diag` for the active
//! buffer and the project, visit on RET, refresh on publish.
//!
//! The fake server publishes two diagnostics on every `didOpen`, so the
//! panel's rows, the visit's landing point and the refresh after a
//! second file's publish are all real store traffic.
//!
//! Bite: drop the `publishDiagnostics` subscription and the refresh row
//! fails; drop `on_visit` and the visit row fails; drop the project
//! section and the refresh row fails on its "Project" line.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

#[path = "common/iso.rs"]
mod iso;
#[path = "common/ready.rs"]
mod ready;

fn exec(state: &EditorState, source: &str) {
    state.lua_host.lua().load(source.to_owned()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(state: &EditorState, source: &str) -> T {
    state.lua_host.lua().load(source.to_owned()).eval().unwrap()
}

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

fn lua_str(path: &Path) -> String {
    format!("{:?}", path.display().to_string())
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        Self { _dir: dir, root }
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }
}

fn editor(fx: &Fixture) -> EditorState {
    let state = EditorState::new_with_roots(&iso::roots());
    // A frame, so the bottom panel has somewhere to be placed: without
    // a geometry declaration a panel request has no rows to take.
    state.sync_frame_geometry(FrontendId::LOCAL, pmacs::protocol::CellSize::new(40, 100));
    let command = format!("{:?}", fake_lsp_path());
    exec(
        &state,
        &format!(
            "pmacs.lsp.config = {{ rust = {{ command = {command} }} }}\n\
             pmacs.project.set_search_boundary({})",
            lua_str(&fx.root)
        ),
    );
    state
}

fn open(state: &EditorState, path: &Path) {
    exec(
        state,
        &format!("pmacs.buffer.find_or_open({})", lua_str(path)),
    );
}

fn wait_diag_count(state: &mut EditorState, path: &Path, n: i64) {
    let expr = format!(
        "return pmacs.diag.count(pmacs.lsp.uri_for_path and pmacs.lsp.uri_for_path({p}) or ('file://' .. {p}))",
        p = lua_str(path)
    );
    ready::tick_until(
        state,
        &format!("{n} diagnostics for {}", path.display()),
        ready::DEADLINE,
        |s| {
            let count: i64 = eval(s, &expr);
            if count >= n {
                ready::Probe::Ready(())
            } else {
                ready::Probe::Pending(format!("{count} diagnostics"))
            }
        },
    );
}

fn press(state: &mut EditorState, code: KeyCode, mods: KeyModifiers) {
    state.dispatch_key(FrontendId::LOCAL, KeyEvent::new(code, mods));
}

/// Run `command` through the M-x prompt by keystrokes.
fn m_x(state: &mut EditorState, command: &str) {
    press(state, KeyCode::Char('x'), KeyModifiers::ALT);
    for ch in command.chars() {
        press(state, KeyCode::Char(ch), KeyModifiers::NONE);
    }
    press(state, KeyCode::Enter, KeyModifiers::NONE);
}

fn panel_text(state: &EditorState) -> String {
    eval(
        state,
        r#"
        for _, id in ipairs(pmacs.buffer.list()) do
          if pmacs.describe.buffer(id).name == "*diagnostics*" then
            return id:slice(0, id:len())
          end
        end
        return ""
        "#,
    )
}

#[test]
fn the_panel_lists_the_buffers_diagnostics_and_ret_visits_the_first() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let file = fx.write("proj/src/main.rs", "fn main() {}\nlet x = 1;\nlet y = 2;\n");
    let mut state = editor(&fx);
    open(&state, &file);
    wait_diag_count(&mut state, &file, 2);

    // Through M-x by keystrokes: the panel's window is reconciled inside
    // key dispatch, so a command invoked from Lua alone opens the panel
    // buffer without a window to show it in.
    m_x(&mut state, "lsp.diagnostics");
    ready::tick_until(
        &mut state,
        "the diagnostics panel as the active window",
        ready::DEADLINE,
        |s| {
            let name: String = eval(s, "return pmacs.window.buffer():name()");
            if name == "*diagnostics*" {
                ready::Probe::Ready(())
            } else {
                ready::Probe::Pending(name)
            }
        },
    );
    let text = panel_text(&state);
    assert!(
        text.contains("This buffer (2):"),
        "the buffer's section counts its diagnostics: {text:?}"
    );
    assert!(
        text.contains("src/main.rs:1:5  error  synthetic error")
            && text.contains("src/main.rs:3:1  warning  synthetic warning"),
        "rows carry path, line, column, severity and message: {text:?}"
    );
    assert!(text.contains("Project (0):"), "{text:?}");
    // RET on the first data row (the cursor is seated there on open)
    // visits the diagnostic's position, through the keymap.
    let seated: i64 = eval(&state, "return pmacs.editor.cursor_line()");
    assert_eq!(seated, 2, "seated on the first row under the section line");
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    let (name, line, col): (String, i64, i64) = eval(
        &state,
        "local b = pmacs.window.buffer(); return b:name(), pmacs.editor.cursor_line(), pmacs.editor.cursor_col()",
    );
    assert!(
        name.ends_with("main.rs"),
        "the visit landed in the file: {name}"
    );
    assert_eq!(
        (line, col),
        (0, 4),
        "at the diagnostic's line and column, 0-based"
    );
}

#[test]
fn a_publish_while_the_panel_is_open_refreshes_it_in_place() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let a = fx.write("proj/src/main.rs", "fn main() {}\nlet x = 1;\nlet y = 2;\n");
    let b = fx.write(
        "proj/src/lib.rs",
        "pub fn lib() {}\nlet z = 3;\nlet w = 4;\n",
    );
    let mut state = editor(&fx);
    open(&state, &a);
    wait_diag_count(&mut state, &a, 2);
    m_x(&mut state, "lsp.diagnostics");
    assert!(panel_text(&state).contains("Project (0):"));

    // A second file opens in the document window (the panel stays), the
    // fake publishes for it, and the panel's project section grows
    // without anyone pressing `g`.
    exec(
        &state,
        &format!(
            "pmacs.window.display_file({}, {{ select = true }})",
            lua_str(&b)
        ),
    );
    wait_diag_count(&mut state, &b, 2);
    let text = ready::tick_until(
        &mut state,
        "the panel re-rendered with the second file's diagnostics",
        ready::DEADLINE,
        |s| {
            let text = panel_text(s);
            if text.contains("Project (2):") {
                ready::Probe::Ready(text)
            } else {
                ready::Probe::Pending(text)
            }
        },
    );
    assert!(
        text.contains("lib.rs:1:5  error  synthetic error"),
        "the project section names the other file's rows: {text:?}"
    );
}

#[test]
fn review_g_preserves_the_source_buffer_section_and_ret_target() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let file = fx.write("proj/src/main.rs", "fn main() {}\nlet x = 1;\nlet y = 2;\n");
    let mut state = editor(&fx);
    open(&state, &file);
    wait_diag_count(&mut state, &file, 2);
    m_x(&mut state, "lsp.diagnostics");
    assert!(panel_text(&state).contains("This buffer (2):"));
    assert_eq!(
        eval::<String>(&state, "return pmacs.window.buffer():name()"),
        "*diagnostics*"
    );
    press(&mut state, KeyCode::Char('g'), KeyModifiers::NONE);
    let text = panel_text(&state);
    eprintln!("AFTER G: {text}");
    press(&mut state, KeyCode::Enter, KeyModifiers::NONE);
    let destination: String = eval(&state, "return pmacs.window.buffer():name()");
    eprintln!("RET DESTINATION: {destination}");
    assert!(
        text.contains("This buffer (2):"),
        "refresh lost the source buffer: {text}"
    );
    assert!(destination.ends_with("main.rs"));
}

#[test]
fn review_publish_preserves_the_source_while_panel_is_focused() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let file = fx.write("proj/src/main.rs", "fn main() {}\nlet x = 1;\nlet y = 2;\n");
    let mut state = editor(&fx);
    open(&state, &file);
    wait_diag_count(&mut state, &file, 2);
    exec(&state, "REVIEW_ATTACHMENT = pmacs.lsp.active_attachment()");
    m_x(&mut state, "lsp.diagnostics");
    assert!(panel_text(&state).contains("This buffer (2):"));
    exec(
        &state,
        r"
        REVIEW_PUBLISH = false
        pmacs.lsp.on_notification('textDocument/publishDiagnostics', function() REVIEW_PUBLISH = true end)
        pmacs.lsp.did_open(REVIEW_ATTACHMENT.server, REVIEW_ATTACHMENT.uri, 2, 'fn main() {}\nlet x = 1;\nlet y = 2;\n')
    ",
    );
    ready::tick_until(&mut state, "republish", ready::DEADLINE, |s| {
        if eval::<bool>(s, "return REVIEW_PUBLISH") {
            ready::Probe::Ready(())
        } else {
            ready::Probe::Pending("no publish yet".to_owned())
        }
    });
    assert_eq!(
        eval::<String>(&state, "return pmacs.window.buffer():name()"),
        "*diagnostics*"
    );
    let text = panel_text(&state);
    assert!(
        text.contains("This buffer (2):"),
        "publish lost source: {text}"
    );
}

// Review-3 python fixture: exchanges real framed JSON-RPC through the
// manager, so publications are synchronized by notification callback.
// Its didOpen text selects the next diagnostic snapshot.
const REVIEW3_LSP: &str = r"
import json, sys

def send(value):
    body = json.dumps(dict(jsonrpc='2.0', **value)).encode()
    sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n' % len(body)).encode() + body)
    sys.stdout.buffer.flush()

def diag(line, col, message, severity=1):
    return dict(range=dict(start=dict(line=line, character=col),
                           end=dict(line=line, character=col+1)),
                message=message, severity=severity, source='review-fixture', code=message)

while True:
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        if line in (b'\r\n', b'\n'): break
        key, value = line.decode().split(':', 1)
        headers[key.lower()] = value.strip()
    msg = json.loads(sys.stdin.buffer.read(int(headers['content-length'])))
    method = msg.get('method')
    if method == 'initialize':
        send(dict(id=msg['id'], result=dict(capabilities=dict(textDocumentSync=1))))
    elif method == 'exit':
        break
    elif method == 'textDocument/didOpen':
        doc = msg['params']['textDocument']
        text = doc.get('text', '')
        if 'duplicate' in text:
            diagnostics = [diag(0,4,'first at shared position'), diag(0,4,'second at shared position',2)]
        else:
            diagnostics = [diag(2,0,'selected target')]
            if 'insert' in text:
                diagnostics.insert(0, diag(0,4,'new earlier diagnostic'))
        send(dict(method='textDocument/publishDiagnostics', params=dict(uri=doc['uri'], diagnostics=diagnostics)))
    elif 'id' in msg:
        send(dict(id=msg['id'], result=None))
";

fn review3_editor(fx: &Fixture) -> EditorState {
    let state = EditorState::new_with_roots(&iso::roots());
    state.sync_frame_geometry(FrontendId::LOCAL, pmacs::protocol::CellSize::new(40, 100));
    let server = fx.write("review_lsp.py", REVIEW3_LSP);
    let script = lua_str(&server);
    exec(
        &state,
        &format!(
            "pmacs.lsp.config = {{ rust = {{ command = \"python3\", args = {{ {script} }} }} }}\n\
             pmacs.project.set_search_boundary({})",
            lua_str(&fx.root)
        ),
    );
    state
}

#[test]
fn review3_diagnostics_at_the_same_position_can_open() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let file = fx.write(
        "proj/src/main.rs",
        "fn main() {} // duplicate\nlet x = 1;\nlet y = 2;\n",
    );
    let mut state = review3_editor(&fx);
    open(&state, &file);
    wait_diag_count(&mut state, &file, 2);
    m_x(&mut state, "lsp.diagnostics");
    let text = panel_text(&state);
    assert!(
        text.contains("This buffer (2):")
            && text.contains("first at shared position")
            && text.contains("second at shared position"),
        "both legitimate diagnostics must appear; panel={text:?}; errors={:?}; status={:?}",
        state.lua_host.errors_buffer_text(),
        state.core.borrow().status
    );
}

#[test]
fn review3_duplicate_positions_do_not_freeze_an_open_panel() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let file = fx.write("proj/src/main.rs", "fn main() {}\nlet x = 1;\nlet y = 2;\n");
    let mut state = review3_editor(&fx);
    open(&state, &file);
    wait_diag_count(&mut state, &file, 1);
    exec(&state, "REVIEW_ATTACHMENT = pmacs.lsp.active_attachment()");
    m_x(&mut state, "lsp.diagnostics");
    assert!(panel_text(&state).contains("This buffer (1):"));
    exec(
        &state,
        r"
        REVIEW_PUBLISH = false
        pmacs.lsp.on_notification('textDocument/publishDiagnostics', function() REVIEW_PUBLISH = true end)
        pmacs.lsp.did_open(REVIEW_ATTACHMENT.server, REVIEW_ATTACHMENT.uri, 2, 'fn main() {} // duplicate\nlet x = 1;\nlet y = 2;\n')
    ",
    );
    ready::tick_until(
        &mut state,
        "duplicate-position publication",
        ready::DEADLINE,
        |s| {
            if eval::<bool>(s, "return REVIEW_PUBLISH") {
                ready::Probe::Ready(())
            } else {
                ready::Probe::Pending("awaiting publish".to_owned())
            }
        },
    );
    let text = panel_text(&state);
    assert!(
        text.contains("This buffer (2):")
            && text.contains("first at shared position")
            && text.contains("second at shared position"),
        "an overlapping diagnostic publication must refresh the already-open panel: {text}"
    );
}

#[test]
fn review3_duplicate_republish_keeps_selection_on_the_same_row() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let file = fx.write(
        "proj/src/main.rs",
        "fn main() {} // duplicate\nlet x = 1;\nlet y = 2;\n",
    );
    let mut state = review3_editor(&fx);
    open(&state, &file);
    wait_diag_count(&mut state, &file, 2);
    exec(&state, "REVIEW_ATTACHMENT = pmacs.lsp.active_attachment()");
    m_x(&mut state, "lsp.diagnostics");
    assert!(panel_text(&state).contains("This buffer (2):"));
    press(&mut state, KeyCode::Char('n'), KeyModifiers::NONE);
    let seated: i64 = eval(&state, "return pmacs.editor.cursor_line()");
    assert_eq!(seated, 3, "seated on the second colocated row");
    exec(
        &state,
        r"
        REVIEW_PUBLISH = false
        pmacs.lsp.on_notification('textDocument/publishDiagnostics', function() REVIEW_PUBLISH = true end)
        pmacs.lsp.did_open(REVIEW_ATTACHMENT.server, REVIEW_ATTACHMENT.uri, 2, 'fn main() {} // duplicate\nlet x = 1;\nlet y = 2;\n')
    ",
    );
    ready::tick_until(&mut state, "same-set republication", ready::DEADLINE, |s| {
        if eval::<bool>(s, "return REVIEW_PUBLISH") {
            ready::Probe::Ready(())
        } else {
            ready::Probe::Pending("awaiting publish".to_owned())
        }
    });
    let after: i64 = eval(&state, "return pmacs.editor.cursor_line()");
    let item_line: i64 = eval(&state, "return pmacs.listview.current_item().line");
    assert_eq!(
        after, 3,
        "content-derived ids keep the same row selected across a republication"
    );
    assert_eq!(item_line, 0);
}

#[test]
fn review3_publish_inserting_a_row_preserves_the_selected_diagnostic() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let file = fx.write("proj/src/main.rs", "fn main() {}\nlet x = 1;\nlet y = 2;\n");
    let mut state = review3_editor(&fx);
    open(&state, &file);
    wait_diag_count(&mut state, &file, 1);
    exec(&state, "REVIEW_ATTACHMENT = pmacs.lsp.active_attachment()");
    m_x(&mut state, "lsp.diagnostics");
    assert!(panel_text(&state).contains("This buffer (1):"));
    assert_eq!(
        eval::<i64>(&state, "return pmacs.listview.current_item().line"),
        2
    );
    exec(
        &state,
        r"
        REVIEW_PUBLISH = false
        pmacs.lsp.on_notification('textDocument/publishDiagnostics', function() REVIEW_PUBLISH = true end)
        pmacs.lsp.did_open(REVIEW_ATTACHMENT.server, REVIEW_ATTACHMENT.uri, 2, 'fn main() {} // insert\nlet x = 1;\nlet y = 2;\n')
    ",
    );
    ready::tick_until(
        &mut state,
        "changed diagnostic publication",
        ready::DEADLINE,
        |s| {
            if eval::<bool>(s, "return REVIEW_PUBLISH") {
                ready::Probe::Ready(())
            } else {
                ready::Probe::Pending("awaiting publish".to_owned())
            }
        },
    );
    assert!(panel_text(&state).contains("This buffer (2):"));
    assert_eq!(
        eval::<String>(&state, "return pmacs.window.buffer():name()"),
        "*diagnostics*"
    );
    press(&mut state, KeyCode::Enter, KeyModifiers::NONE);
    let (name, line): (String, i64) = eval(
        &state,
        "return pmacs.window.buffer():name(), pmacs.editor.cursor_line()",
    );
    assert!(name.ends_with("main.rs"));
    assert_eq!(
        line, 2,
        "RET must still visit the selected diagnostic after an earlier row is inserted"
    );
}

#[test]
fn review3_publish_removing_the_selected_row_falls_back_to_its_line() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let file = fx.write("proj/src/main.rs", "fn main() {}\nlet x = 1;\nlet y = 2;\n");
    let mut state = review3_editor(&fx);
    open(&state, &file);
    wait_diag_count(&mut state, &file, 1);
    exec(&state, "REVIEW_ATTACHMENT = pmacs.lsp.active_attachment()");
    m_x(&mut state, "lsp.diagnostics");
    assert!(panel_text(&state).contains("This buffer (1):"));
    assert_eq!(eval::<i64>(&state, "return pmacs.editor.cursor_line()"), 2);
    exec(
        &state,
        "pmacs.diag.clear(REVIEW_ATTACHMENT.uri); pmacs.listview.rerender('*diagnostics*')",
    );
    let (line, no_item): (i64, bool) = eval(
        &state,
        "return pmacs.editor.cursor_line(), pmacs.listview.current_item() == nil",
    );
    assert_eq!(
        line, 2,
        "a vanished diagnostic falls back to its numeric line, clamped"
    );
    assert!(no_item, "the fallback line carries no diagnostic to visit");
    assert!(panel_text(&state).contains("This buffer (0):"));
}

#[test]
fn review3_background_publish_keeps_the_panel_selection_and_leaves_the_document_cursor() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let file = fx.write("proj/src/main.rs", "fn main() {}\nlet x = 1;\nlet y = 2;\n");
    let mut state = review3_editor(&fx);
    open(&state, &file);
    wait_diag_count(&mut state, &file, 1);
    exec(&state, "REVIEW_ATTACHMENT = pmacs.lsp.active_attachment()");
    m_x(&mut state, "lsp.diagnostics");
    assert!(panel_text(&state).contains("This buffer (1):"));
    exec(&state, "REVIEW_PANEL = pmacs.window.buffer()");
    exec(
        &state,
        &format!(
            "pmacs.window.display_file({}, {{ select = true }})",
            lua_str(&file)
        ),
    );
    assert!(eval::<String>(&state, "return pmacs.window.buffer():name()").ends_with("main.rs"));
    let doc_line: i64 = eval(&state, "return pmacs.editor.cursor_line()");
    exec(
        &state,
        r"
        REVIEW_PUBLISH = false
        pmacs.lsp.on_notification('textDocument/publishDiagnostics', function() REVIEW_PUBLISH = true end)
        pmacs.lsp.did_open(REVIEW_ATTACHMENT.server, REVIEW_ATTACHMENT.uri, 2, 'fn main() {} // insert\nlet x = 1;\nlet y = 2;\n')
    ",
    );
    ready::tick_until(
        &mut state,
        "background diagnostic publication",
        ready::DEADLINE,
        |s| {
            if eval::<bool>(s, "return REVIEW_PUBLISH") {
                ready::Probe::Ready(())
            } else {
                ready::Probe::Pending("awaiting publish".to_owned())
            }
        },
    );
    assert_eq!(
        eval::<i64>(&state, "return pmacs.editor.cursor_line()"),
        doc_line,
        "refreshing a background panel must not move the document window"
    );
    assert!(
        eval::<String>(&state, "return pmacs.window.buffer():name()").ends_with("main.rs"),
        "the background refresh leaves the document focused"
    );
    // Focusing re-seats the cursor through the display transaction, so
    // the background window's retained selection is read in place.
    assert_eq!(
        eval::<i64>(
            &state,
            "return pmacs.window._cursor_line_on_buffer(REVIEW_PANEL)"
        ),
        3,
        "the background panel kept the selected diagnostic across the insertion"
    );
    exec(
        &state,
        "pmacs.window.display(REVIEW_PANEL, { select = true })",
    );
    assert!(
        eval::<String>(&state, "return pmacs.window.buffer():name()").ends_with("*diagnostics*"),
        "switching back focuses the panel"
    );
}

#[test]
fn review_project_section_excludes_an_unrelated_root() {
    let fx = Fixture::new();
    fx.write("proj_a/Cargo.toml", "[package]\nname = \"a\"\n");
    fx.write("proj_b/Cargo.toml", "[package]\nname = \"b\"\n");
    let a = fx.write(
        "proj_a/src/main.rs",
        "fn main() {}\nlet x = 1;\nlet y = 2;\n",
    );
    let b = fx.write(
        "proj_b/src/unrelated.rs",
        "fn main() {}\nlet x = 1;\nlet y = 2;\n",
    );
    let mut state = editor(&fx);
    open(&state, &b);
    wait_diag_count(&mut state, &b, 2);
    open(&state, &a);
    wait_diag_count(&mut state, &a, 2);
    assert_eq!(
        eval::<usize>(&state, "return #pmacs.lsp.list()"),
        2,
        "two roots have independent servers"
    );
    m_x(&mut state, "lsp.diagnostics");
    let text = panel_text(&state);
    assert!(
        !text.contains("unrelated.rs"),
        "other root leaked into Project: {text}"
    );
}
