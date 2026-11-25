// tests/e10v_7_rename_one_undo_acceptance.rs --- E10v.7 (#536): a
// rename is one undo.

//! A language server's workspace edit was applied edit by edit, each by
//! its own `buf:replace` in the answer's callback, where no command is
//! in scope: so each edit was a daemon group of its own, and the user's
//! `C-x u`, which takes the newest group, took back one edit. A rename
//! of five occurrences took five, the caret drifting left on its line
//! by what each restored before it.
//!
//! Now each buffer's edits are one group (`buf:edit_group`), which one
//! undo takes back whole, putting the caret and the view where the edit
//! found them; a redo puts it back whole. The rename, a code action's
//! edit and a server's own `workspace/applyEdit` share the applier.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use serde_json::{Value, json};

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

/// E10a.3's source: forty comment lines, then the function a rename
/// edits on lines 40 to 42.
fn source() -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for i in 0..40 {
        let _ = writeln!(text, "// {i}");
    }
    text.push_str("fn count(total: u32) -> u32 {\n    let x = total + total;\n    total * x + total\n}\n// tail\n");
    text
}

const OTHER: &str = "fn other(total: u32) -> u32 { total }\n";

fn uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn edit(line: u32, start: u32, end: u32) -> Value {
    json!({
        "range": {
            "start": { "line": line, "character": start },
            "end": { "line": line, "character": end }
        },
        "newText": "n"
    })
}

/// The five `total`s of `source()`.
fn five() -> Value {
    json!([
        edit(40, 9, 14),
        edit(41, 12, 17),
        edit(41, 20, 25),
        edit(42, 4, 9),
        edit(42, 16, 21),
    ])
}

struct Fixture {
    dir: PathBuf,
    a: PathBuf,
    b: PathBuf,
    plan: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("pmacs-e10v7-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.rs");
    std::fs::write(&a, source()).unwrap();
    let b = dir.join("b.rs");
    std::fs::write(&b, OTHER).unwrap();
    Fixture {
        plan: dir.join("plan.json"),
        dir,
        a,
        b,
    }
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
    fn write_plan(&self, changes: &Value) {
        std::fs::write(&self.plan, json!({ "changes": changes }).to_string()).unwrap();
    }

    fn editor(&self, extra: &str) -> EditorState {
        let s = EditorState::new_with_roots(&iso::roots());
        s.lua_host.lua().remove_app_data::<StateDir>();
        s.lua_host.lua().set_app_data(StateDir(self.dir.clone()));
        exec(
            &s,
            &format!(
                "pmacs.lsp.config = {{}}\n\
                 pmacs.project.set_search_boundary({dir:?})\n\
                 pmacs.lsp.config.rust = {{ command = '{cmd}', env = {{ \
                   PMACS_FAKE_LSP_MODE = 'applyeditplan', PMACS_FAKE_LSP_EDIT_PLAN = '{plan}', \
                   PMACS_FAKE_LSP_APPLYEDIT_SINK = '{sink}' {extra} }} }}\n",
                dir = self.dir.display().to_string(),
                cmd = fake_lsp_path(),
                plan = self.plan.display(),
                sink = self.dir.join("sink.json").display(),
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

/// The caret on line 41, column 22, the view from line 30.
fn seat(s: &EditorState) {
    let byte: usize = source()
        .split_inclusive('\n')
        .take(41)
        .map(str::len)
        .sum::<usize>()
        + 22;
    exec(s, &format!("pmacs.editor.goto_byte({byte})"));
    exec(s, "pmacs.editor.set_view_top(30)");
}

fn rename(s: &mut EditorState) {
    press(s, KeyCode::Char('c'), KeyModifiers::CONTROL);
    press(s, KeyCode::Char('r'), KeyModifiers::NONE);
    press(s, KeyCode::Char('n'), KeyModifiers::NONE);
    press(s, KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        pump_until(s, 10, |s| status(s).starts_with("LSP: renamed")),
        "premise: the rename was applied; status {:?}",
        status(s)
    );
}

/// `C-x u`.
fn undo(s: &mut EditorState) {
    press(s, KeyCode::Char('x'), KeyModifiers::CONTROL);
    press(s, KeyCode::Char('u'), KeyModifiers::NONE);
}

/// `C-x r`.
fn redo(s: &mut EditorState) {
    press(s, KeyCode::Char('x'), KeyModifiers::CONTROL);
    press(s, KeyCode::Char('r'), KeyModifiers::NONE);
}

/// Review 1's `r1_p4`, kept and tightened: after a rename of five
/// occurrences, one `C-x u` restores the whole text, and the caret's
/// line and column and the view's first line are the values captured
/// before the rename. At the base one undo took back one edit (line 41
/// restored, 42 and 43 still renamed); as written the probe accepted any
/// of three lines and never read the column.
#[test]
fn e10v_7_r1_p4_one_undo_takes_back_a_rename_and_puts_the_place_back() {
    let fx = fixture("rename");
    fx.write_plan(&json!({ uri(&fx.a): five() }));
    let mut s = fx.editor("");
    seat(&s);
    let before = place(&s);
    assert_eq!(before, ("a.rs".to_owned(), 41, 22, 30), "premise");
    rename(&mut s);
    assert_ne!(named_text(&s, "a.rs"), source(), "premise: renamed");
    undo(&mut s);
    assert_eq!(
        named_text(&s, "a.rs"),
        source(),
        "one undo takes the whole rename back"
    );
    assert_eq!(
        place(&s),
        before,
        "the caret's line and column and the view, as before the rename"
    );
}

/// A redo after it puts the rename back whole.
#[test]
fn e10v_7_a_redo_puts_the_whole_rename_back() {
    let fx = fixture("redo");
    fx.write_plan(&json!({ uri(&fx.a): five() }));
    let mut s = fx.editor("");
    seat(&s);
    rename(&mut s);
    let renamed = named_text(&s, "a.rs");
    undo(&mut s);
    assert_eq!(named_text(&s, "a.rs"), source(), "premise: undone whole");
    redo(&mut s);
    assert_eq!(
        named_text(&s, "a.rs"),
        renamed,
        "one redo puts the whole rename back"
    );
}

/// A rename across two buffers: one undo in each takes back that
/// buffer's edits, and only those.
#[test]
fn e10v_7_a_rename_across_two_buffers_is_one_undo_in_each() {
    let fx = fixture("two");
    fx.write_plan(&json!({
        uri(&fx.a): five(),
        uri(&fx.b): [edit(0, 9, 14), edit(0, 30, 35)],
    }));
    let mut s = fx.editor("");
    seat(&s);
    let before = place(&s);
    rename(&mut s);
    let other_renamed = "fn other(n: u32) -> u32 { n }\n";
    assert_eq!(named_text(&s, "b.rs"), other_renamed, "premise");
    undo(&mut s);
    assert_eq!(
        named_text(&s, "a.rs"),
        source(),
        "one undo in a.rs takes its edits back"
    );
    assert_eq!(place(&s), before);
    assert_eq!(named_text(&s, "b.rs"), other_renamed, "and not b.rs's");
    open(&s, &fx.b);
    undo(&mut s);
    assert_eq!(
        named_text(&s, "b.rs"),
        OTHER,
        "one undo in b.rs takes its edits back"
    );
}

/// A code action's edit of five occurrences: one undo, the place back.
#[test]
fn e10v_7_a_code_actions_edit_is_one_undo() {
    let fx = fixture("action");
    let results = fx.dir.join("results.json");
    std::fs::write(
        &results,
        json!({ "textDocument/codeAction": [{
            "title": "rename total", "kind": "refactor",
            "edit": { "changes": { uri(&fx.a): five() } }
        }] })
        .to_string(),
    )
    .unwrap();
    let mut s = fx.editor(&format!(
        ", PMACS_FAKE_LSP_RESULT_PLAN = '{}'",
        results.display()
    ));
    seat(&s);
    let before = place(&s);
    press(&mut s, KeyCode::Char('c'), KeyModifiers::CONTROL);
    press(&mut s, KeyCode::Char('a'), KeyModifiers::NONE);
    let _ = pump_until(&mut s, 10, |s| {
        s.core.borrow().minibuffer.is_active() || named_text(s, "a.rs") != source()
    });
    if s.core.borrow().minibuffer.is_active() {
        press(&mut s, KeyCode::Enter, KeyModifiers::NONE);
    }
    assert!(
        pump_until(&mut s, 10, |s| named_text(s, "a.rs") != source()),
        "premise: the action's edit landed; status {:?}",
        status(&s)
    );
    undo(&mut s);
    assert_eq!(
        named_text(&s, "a.rs"),
        source(),
        "one undo takes the action's edit back"
    );
    assert_eq!(place(&s), before);
}

/// A server's own `workspace/applyEdit` of five occurrences (the fake's
/// command action makes it send the plan): one undo, the place back.
#[test]
fn e10v_7_a_servers_apply_edit_is_one_undo() {
    let fx = fixture("applyedit");
    fx.write_plan(&json!({ uri(&fx.a): five() }));
    let mut s = fx.editor("");
    seat(&s);
    let before = place(&s);
    press(&mut s, KeyCode::Char('c'), KeyModifiers::CONTROL);
    press(&mut s, KeyCode::Char('a'), KeyModifiers::NONE);
    assert!(
        pump_until(&mut s, 10, |s| s.core.borrow().minibuffer.is_active()),
        "premise: the action prompt; status {:?}",
        status(&s)
    );
    press(&mut s, KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        pump_until(&mut s, 10, |s| named_text(s, "a.rs") != source()),
        "premise: the server's edit landed; status {:?}",
        status(&s)
    );
    undo(&mut s);
    assert_eq!(
        named_text(&s, "a.rs"),
        source(),
        "one undo takes the server's edit back"
    );
    assert_eq!(place(&s), before);
}
