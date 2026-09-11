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
    assert_eq!(eval::<String>(&state, "return pmacs.window.buffer():name()"), "*diagnostics*");
    press(&mut state, KeyCode::Char('g'), KeyModifiers::NONE);
    let text = panel_text(&state);
    eprintln!("AFTER G: {text}");
    press(&mut state, KeyCode::Enter, KeyModifiers::NONE);
    let destination: String = eval(&state, "return pmacs.window.buffer():name()");
    eprintln!("RET DESTINATION: {destination}");
    assert!(text.contains("This buffer (2):"), "refresh lost the source buffer: {text}");
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
    exec(&state, r#"
        REVIEW_PUBLISH = false
        pmacs.lsp.on_notification('textDocument/publishDiagnostics', function() REVIEW_PUBLISH = true end)
        pmacs.lsp.did_open(REVIEW_ATTACHMENT.server, REVIEW_ATTACHMENT.uri, 2, 'fn main() {}\nlet x = 1;\nlet y = 2;\n')
    "#);
    ready::tick_until(&mut state, "republish", ready::DEADLINE, |s| {
        if eval::<bool>(s, "return REVIEW_PUBLISH") { ready::Probe::Ready(()) }
        else { ready::Probe::Pending("no publish yet".to_owned()) }
    });
    assert_eq!(eval::<String>(&state, "return pmacs.window.buffer():name()"), "*diagnostics*");
    let text=panel_text(&state);
    assert!(text.contains("This buffer (2):"), "publish lost source: {text}");
}

#[test]
fn review_project_section_excludes_an_unrelated_root() {
    let fx = Fixture::new();
    fx.write("proj_a/Cargo.toml", "[package]\nname = \"a\"\n");
    fx.write("proj_b/Cargo.toml", "[package]\nname = \"b\"\n");
    let a=fx.write("proj_a/src/main.rs", "fn main() {}\nlet x = 1;\nlet y = 2;\n");
    let b=fx.write("proj_b/src/unrelated.rs", "fn main() {}\nlet x = 1;\nlet y = 2;\n");
    let mut state=editor(&fx);
    open(&state,&b);
    wait_diag_count(&mut state,&b,2);
    open(&state,&a);
    wait_diag_count(&mut state,&a,2);
    assert_eq!(eval::<usize>(&state,"return #pmacs.lsp.list()"),2, "two roots have independent servers");
    m_x(&mut state,"lsp.diagnostics");
    let text=panel_text(&state);
    assert!(!text.contains("unrelated.rs"), "other root leaked into Project: {text}");
}
