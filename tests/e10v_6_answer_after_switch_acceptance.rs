// tests/e10v_6_answer_after_switch_acceptance.rs --- E10v.6 (#535): an
// answer lands in the buffer it was asked for.

//! `C-c f` asked the server to format the shown buffer and applied the
//! answer, whenever it landed, through `apply_text_edits`, which edited
//! `pmacs.window.buffer()`: a switch while the answer was pending put the
//! requested document's edits into whatever the window showed then. The
//! same-file branch of go-to-definition moved the caret of whatever
//! buffer was shown. Now an answer is applied to the buffer it was asked
//! for, shown or not, no other buffer's text, caret or view moving; a
//! buffer killed before the answer drops it, and the echo line says so.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use serde_json::json;

mod common;
#[path = "support/mod.rs"]
mod support;

use common::iso;

#[path = "common/editor.rs"]
mod editor;
use editor::{eval, exec};

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

fn press(s: &mut EditorState, code: KeyCode, mods: KeyModifiers) {
    s.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent {
            code,
            modifiers: mods,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    );
}

fn tick(s: &mut EditorState) {
    s.tick_processes();
    s.tick_lsp();
    s.tick_async();
}

fn pump_until(s: &mut EditorState, secs: u64, mut done: impl FnMut(&EditorState) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        tick(s);
        if done(s) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

fn named_text(s: &EditorState, name: &str) -> String {
    let b: mlua::String = eval(
        s,
        &format!(
            r#"
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == {name:?} then
                    return id:slice(0, id:len())
                end
            end
            return ""
            "#
        ),
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
}

/// The shown buffer, the caret's 0-based line and column, the view's
/// first line.
fn place(s: &EditorState) -> (String, i64, i64, i64) {
    let name: String = eval(
        s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    );
    (
        name,
        eval(s, "return pmacs.editor.cursor_line()"),
        eval(s, "return pmacs.editor.cursor_col()"),
        eval(s, "return pmacs.editor.view_top()"),
    )
}

const A: &str = "    fn a() {}\nfn b() {}\nlet here = 1;\n0123456789\ntail\n";
/// The fake's formatting answer applied to `A`: line 0's first four
/// characters gone, a `;` at line 3, column 7.
const A_FORMATTED: &str = "fn a() {}\nfn b() {}\nlet here = 1;\n0123456;789\ntail\n";
const NOTES: &str = "alpha beta\ngamma delta\nepsilon zeta\neta theta iota\n";

struct Fixture {
    dir: PathBuf,
    a: PathBuf,
    notes: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("pmacs-e10v6-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.rs");
    std::fs::write(&a, A).unwrap();
    let notes = dir.join("notes.txt");
    std::fs::write(&notes, NOTES).unwrap();
    Fixture { dir, a, notes }
}

fn open(s: &EditorState, path: &Path) {
    exec(
        s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            path.display().to_string()
        ),
    );
}

impl Fixture {
    fn editor(&self, env: &str) -> EditorState {
        let s = EditorState::new_with_roots(&iso::roots());
        s.lua_host.lua().remove_app_data::<StateDir>();
        s.lua_host.lua().set_app_data(StateDir(self.dir.clone()));
        exec(
            &s,
            &format!(
                "pmacs.lsp.config = {{}}\n\
                 pmacs.project.set_search_boundary({dir:?})\n\
                 pmacs.lsp.config.rust = {{ command = '{cmd}', env = {{ {env} }} }}\n",
                dir = self.dir.display().to_string(),
                cmd = fake_lsp_path(),
            ),
        );
        open(&s, &self.a);
        let mut s = s;
        assert!(
            pump_until(&mut s, 10, |s| eval::<bool>(
                s,
                "for _, r in ipairs(pmacs.lsp.list()) do \
                   if r.state and r.state.kind == 'initialized' then return true end \
                 end return false"
            )),
            "premise: the fake initialized"
        );
        s
    }
}

/// Review 1's `r1_p3`, kept and tightened: `C-c f` in `a.rs`, then a
/// switch to `notes.txt`, its caret put on line 2 and its view from line
/// 1, while the answer is held. The answer formats `a.rs`, to exactly the
/// text it asked for, though no window shows it; `notes.txt`'s text,
/// caret and view are as they were. At the base `notes.txt` read `a beta
/// … eta ;theta iota` and `a.rs` was unchanged.
#[test]
fn e10v_6_r1_p3_a_format_answered_after_a_switch_edits_only_its_own_buffer() {
    let fx = fixture("format");
    let mut s = fx.editor("PMACS_FAKE_LSP_FORMAT_HOLD_MS = '1500'");
    press(&mut s, KeyCode::Char('c'), KeyModifiers::CONTROL);
    press(&mut s, KeyCode::Char('f'), KeyModifiers::NONE);
    open(&s, &fx.notes);
    exec(&s, "pmacs.editor.goto_byte(14)");
    exec(&s, "pmacs.editor.set_view_top(1)");
    let before = place(&s);
    assert_eq!(before, ("notes.txt".to_owned(), 1, 3, 1), "premise");
    assert!(
        pump_until(&mut s, 10, |s| status(s).starts_with("LSP: applied")
            || status(s).starts_with("LSP: no formatting")),
        "premise: the format's answer landed; status {:?}",
        status(&s)
    );
    assert_eq!(status(&s), "LSP: applied 2 edits");
    assert_eq!(
        named_text(&s, "a.rs"),
        A_FORMATTED,
        "a.rs holds exactly the format it asked for"
    );
    assert_eq!(named_text(&s, "notes.txt"), NOTES, "notes.txt is untouched");
    assert_eq!(
        place(&s),
        before,
        "notes.txt's caret and view are as they were"
    );
}

/// A format whose buffer is killed and its file opened again while the
/// answer is pending: the answer was the killed buffer's, so the new one
/// (the window shows it, from disk) is untouched and the echo line says
/// the answer was dropped. At the base the shown buffer took the edits.
#[test]
fn e10v_6_a_format_whose_buffer_was_killed_and_reopened_is_dropped() {
    let fx = fixture("reopen");
    let mut s = fx.editor("PMACS_FAKE_LSP_FORMAT_HOLD_MS = '1500'");
    press(&mut s, KeyCode::Char('c'), KeyModifiers::CONTROL);
    press(&mut s, KeyCode::Char('f'), KeyModifiers::NONE);
    exec(&s, "pmacs.buffer.kill(pmacs.window.buffer())");
    open(&s, &fx.a);
    assert_eq!(place(&s).0, "a.rs", "premise: the file opened again");
    assert!(
        pump_until(&mut s, 10, |s| status(s).contains("format")
            || status(s).starts_with("LSP: applied")),
        "premise: the answer landed; status {:?}",
        status(&s)
    );
    assert_eq!(
        named_text(&s, "a.rs"),
        A,
        "the reopened buffer is untouched"
    );
    assert!(
        status(&s).contains("dropped") && status(&s).contains("a.rs"),
        "the echo line says the answer was dropped: {:?}",
        status(&s)
    );
}

/// A definition in the same file, answered after a switch to
/// `notes.txt`: the window goes back to `a.rs` at the definition, as a
/// cross-file definition goes to its file; `notes.txt`'s caret is not
/// moved. At the base the window stayed on `notes.txt` with its caret
/// moved to the definition's line and column.
#[test]
fn e10v_6_a_definition_answered_after_a_switch_goes_to_its_own_buffer() {
    let fx = fixture("definition");
    let plan = fx.dir.join("results.json");
    let a_uri = format!("file://{}", fx.a.display());
    std::fs::write(
        &plan,
        json!({ "textDocument/definition": [{ "uri": a_uri, "range": {
            "start": { "line": 3, "character": 2 }, "end": { "line": 3, "character": 4 }
        } }] })
        .to_string(),
    )
    .unwrap();
    let mut s = fx.editor(&format!(
        "PMACS_FAKE_LSP_RESULT_PLAN = '{}', PMACS_FAKE_LSP_RESULT_HOLD_MS = '1500'",
        plan.display()
    ));
    exec(&s, "pmacs.command.invoke('lsp.go-to-definition')");
    open(&s, &fx.notes);
    exec(&s, "pmacs.editor.goto_byte(14)");
    assert!(
        pump_until(&mut s, 10, |s| status(s).contains("definition")),
        "premise: the answer landed; status {:?}",
        status(&s)
    );
    let (name, line, col, _) = place(&s);
    assert_eq!(
        (name.as_str(), line, col),
        ("a.rs", 3, 2),
        "the window shows the requested buffer at the definition; status {:?}",
        status(&s)
    );
    open(&s, &fx.notes);
    assert_eq!(named_text(&s, "notes.txt"), NOTES);
}
