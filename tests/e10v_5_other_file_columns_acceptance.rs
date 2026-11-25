// tests/e10v_5_other_file_columns_acceptance.rs --- E10v.5 (#533): a
// position in another file lands at its own columns.

//! A server that counts columns in UTF-16, the spec's default, names
//! positions in files other than the one a request was about: a rename's
//! or a code action's edits, a definition, references, a workspace
//! symbol, and a `workspace/applyEdit` the server sends of its own
//! accord. The client converted every position in an answer against the
//! one document the request named (`inbound_converted`), so a column in
//! another file was read against the requesting document's line of the
//! same number; and a server's own `applyEdit` reached the applier with
//! its columns not converted at all, UTF-16 units read as bytes.
//!
//! Now each position is converted against its own document: the text
//! the server holds for it (an open buffer's, unsaved edits and all, as
//! last sent), else the file on disk. Each row's other file holds two
//! emoji before the text the server names, each two UTF-16 units and
//! four bytes, so a column read in the wrong units or against the wrong
//! line misses.

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

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
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

/// The buffer the window shows, the caret's 0-based line and byte column.
fn place(s: &EditorState) -> (String, i64, i64) {
    let name: String = eval(
        s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    );
    (
        name,
        eval(s, "return pmacs.editor.cursor_line()"),
        eval(s, "return pmacs.editor.cursor_col()"),
    )
}

/// The UTF-16 column of byte `byte` of `line`.
fn utf16(line: &str, byte: usize) -> u32 {
    u32::try_from(line[..byte].encode_utf16().count()).unwrap()
}

/// The requesting file: short lines, so a column read against its line
/// of the same number is clamped.
const A: &str = "// a\nx\nyy\n";
/// The other file, as on disk.
const B_LINE: &str = "let e = \"\u{1F600}\u{1F600}\"; fn other(total: u32) -> u32 { total }";

fn uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn range(line: u32, text: &str, from: usize, to: usize) -> Value {
    json!({
        "start": { "line": line, "character": utf16(text, from) },
        "end": { "line": line, "character": utf16(text, to) }
    })
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

struct Fixture {
    dir: PathBuf,
    a: PathBuf,
    b: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("pmacs-e10v5-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.rs");
    let b = dir.join("b.rs");
    std::fs::write(&a, A).unwrap();
    std::fs::write(&b, format!("// b\n{B_LINE}\n")).unwrap();
    Fixture { dir, a, b }
}

impl Fixture {
    /// An editor on `a.rs` with the fake: UTF-16 columns (no encoding
    /// negotiated), `env` beside the mode.
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

    fn result_plan(&self, plan: &Value) -> String {
        let p = self.dir.join("results.json");
        std::fs::write(&p, plan.to_string()).unwrap();
        format!("PMACS_FAKE_LSP_RESULT_PLAN = '{}'", p.display())
    }
}

/// A server's own `workspace/applyEdit` into another file that is open
/// with unsaved text differing from disk: two lines above the one the
/// server names, one holding an emoji. The edit renames both `total`s
/// of that line, columns in UTF-16 counted in the unsaved text, the text
/// the server was sent. The code action's command on `a.rs` makes the
/// fake send it. At the base the columns reached the applier unconverted,
/// UTF-16 units read as bytes, and the edits landed four bytes early.
#[test]
fn e10v_5_a_servers_own_apply_edit_lands_at_the_other_files_columns() {
    let fx = fixture("applyedit");
    let unsaved = format!("// \u{1F600} unsaved\n// b\n{B_LINE}\n");
    let line = B_LINE;
    let first = line.find("total").unwrap();
    let second = line.rfind("total").unwrap();
    let plan = fx.dir.join("plan.json");
    std::fs::write(
        &plan,
        json!({ "changes": { uri(&fx.b): [
            { "range": range(2, line, first, first + 5), "newText": "n" },
            { "range": range(2, line, second, second + 5), "newText": "n" },
        ] } })
        .to_string(),
    )
    .unwrap();
    let sink = fx.dir.join("changes.jsonl");
    let mut s = fx.editor(&format!(
        "PMACS_FAKE_LSP_MODE = 'applyeditplan', PMACS_FAKE_LSP_EDIT_PLAN = '{}', \
         PMACS_FAKE_LSP_APPLYEDIT_SINK = '{}', PMACS_FAKE_LSP_CHANGE_SINK = '{}'",
        plan.display(),
        fx.dir.join("sink.json").display(),
        sink.display()
    ));
    // b.rs open, its unsaved text sent to the server; then back to a.rs.
    open(&s, &fx.b);
    exec(&s, "pmacs.editor.goto_byte(0)");
    exec(
        &s,
        &format!("pmacs.editor.clipboard_set({:?})", "// \u{1F600} unsaved\n"),
    );
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('y'), KeyModifiers::CONTROL),
    );
    exec(&s, "pmacs.lsp._attachment_for_command()");
    assert!(
        pump_until(&mut s, 10, |_| std::fs::read_to_string(&sink)
            .unwrap_or_default()
            .contains("unsaved")),
        "premise: the server holds b.rs's unsaved text"
    );
    assert_eq!(named_text(&s, "b.rs"), unsaved, "premise");
    open(&s, &fx.a);
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('a'), KeyModifiers::NONE),
    );
    assert!(
        pump_until(&mut s, 10, |s| s.core.borrow().minibuffer.is_active()),
        "premise: the action prompt; status {:?}",
        status(&s)
    );
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Enter, KeyModifiers::NONE));
    let expected = unsaved.replace("total", "n");
    let landed = pump_until(&mut s, 10, |s| named_text(s, "b.rs") != unsaved);
    assert!(
        landed,
        "premise: the server's edit landed; status {:?}",
        status(&s)
    );
    assert_eq!(
        named_text(&s, "b.rs"),
        expected,
        "both totals renamed, at b.rs's own columns in its unsaved text; status {:?}",
        status(&s)
    );
    assert_eq!(
        std::fs::read_to_string(&fx.b).unwrap(),
        format!("// b\n{B_LINE}\n"),
        "premise: the disk was not the text converted against"
    );
}

/// A definition in another file, not open in any buffer: the window
/// goes to `b.rs` with the caret on `other`, its column converted
/// against the file as read from disk. At the base it was converted
/// against `a.rs`'s line 1, `x`, and clamped to its end.
#[test]
fn e10v_5_a_definition_in_another_file_lands_on_its_column() {
    let fx = fixture("definition");
    let at = B_LINE.find("other").unwrap();
    let env = fx.result_plan(&json!({
        "textDocument/definition": [{ "uri": uri(&fx.b), "range": range(1, B_LINE, at, at + 5) }]
    }));
    let mut s = fx.editor(&env);
    exec(&s, "pmacs.command.invoke('lsp.go-to-definition')");
    assert!(
        pump_until(&mut s, 10, |s| place(s).0 == "b.rs"),
        "premise: the window went to b.rs; status {:?}",
        status(&s)
    );
    assert_eq!(
        place(&s),
        ("b.rs".to_owned(), 1, i64::try_from(at).unwrap()),
        "the caret on `other`, at its byte column"
    );
}

/// References in another file, asked by `M-?`: the panel's row names
/// `b.rs`'s line and the column of the second `total`, and the location
/// a visit goes to holds that byte column (the visit's placement is the
/// definition row's). At the base the column was `a.rs`'s line 1's
/// length.
#[test]
fn e10v_5_references_in_another_file_name_its_column() {
    let fx = fixture("references");
    let at = B_LINE.rfind("total").unwrap();
    let env = fx.result_plan(&json!({
        "textDocument/references": [{ "uri": uri(&fx.b), "range": range(1, B_LINE, at, at + 5) }]
    }));
    let mut s = fx.editor(&env);
    // `M-?`, as a user asks.
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('?'), KeyModifiers::ALT),
    );
    assert!(
        pump_until(&mut s, 10, |s| named_text(s, "*references*")
            .contains("b.rs")),
        "premise: the references panel; status {:?}",
        status(&s)
    );
    let panel = named_text(&s, "*references*");
    assert!(
        panel.contains(&format!("b.rs:2:{}", at + 1)),
        "the row names the second total's column, {}: {panel:?}",
        at + 1
    );
    // The column a visit places the caret at, as the panel's record holds it.
    let col: i64 = eval(
        &s,
        "for _, r in ipairs(pmacs.lsp.list()) do \
           local l = pmacs.references.locations(r.id, pmacs.lsp.active_attachment().uri) \
           if l and l[1] then return l[1].col end \
         end return -1",
    );
    assert_eq!(
        col,
        i64::try_from(at).unwrap(),
        "the location's byte column"
    );
}

/// A workspace symbol, whose request names no document, in another
/// file, asked by `C-x p s`: the panel's row names `b.rs`'s line, and
/// the symbol a visit goes to holds `other`'s byte column. At the base
/// its column was left in UTF-16 units and read as bytes, four short.
#[test]
fn e10v_5_a_workspace_symbol_in_another_file_holds_its_column() {
    let fx = fixture("wsymbol");
    let at = B_LINE.find("other").unwrap();
    let env = fx.result_plan(&json!({
        "workspace/symbol": [{
            "name": "other", "kind": 12,
            "location": { "uri": uri(&fx.b), "range": range(1, B_LINE, at, at + 5) }
        }]
    }));
    let mut s = fx.editor(&env);
    // `C-x p s`, then the query at its prompt.
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('x'), KeyModifiers::CONTROL),
    );
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('p'), KeyModifiers::NONE),
    );
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('s'), KeyModifiers::NONE),
    );
    assert!(
        pump_until(&mut s, 5, |s| s.core.borrow().minibuffer.is_active()),
        "premise: the query prompt"
    );
    for ch in "other".chars() {
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        pump_until(&mut s, 10, |s| named_text(s, "*workspace-symbols*")
            .contains("other")),
        "premise: the symbols panel; status {:?}",
        status(&s)
    );
    let panel = named_text(&s, "*workspace-symbols*");
    assert!(
        panel.contains("b.rs:2"),
        "the row names b.rs's line: {panel:?}"
    );
    // The column a visit places the caret at, as the panel's record holds it.
    let col: i64 = eval(
        &s,
        "for _, r in ipairs(pmacs.lsp.list()) do \
           local syms = pmacs.workspace_symbol.symbols(r.id, 'other') \
           if syms and syms[1] then return syms[1].col end \
         end return -1",
    );
    assert_eq!(col, i64::try_from(at).unwrap(), "the symbol's byte column");
}
