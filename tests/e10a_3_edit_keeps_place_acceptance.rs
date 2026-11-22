// tests/e10a_3_edit_keeps_place_acceptance.rs --- E10a.3: a rename
// keeps the place.

//! `C-c r` applied the server's edit and left the caret at `L1:C1` with
//! the view at the top, in both frontends. The applier switches the
//! window to each edited buffer and back, and a switch puts the caret
//! and the view at the top even when the window already shows that
//! buffer; nothing carried the place across. Every user of the applier
//! lost it the same way (a code action, a server's own
//! `workspace/applyEdit`), and a format, which switches nothing, left
//! the caret's byte where it was while the text moved under it.
//!
//! These rows press the keys a user presses, against the fake server
//! whose `WorkspaceEdit` each row writes beside its assertions
//! (`PMACS_FAKE_LSP_MODE=applyeditplan`), and read where the caret and
//! the view are afterwards: in the editor, on a real TUI's screen, and
//! in the GPU's window through the headless probe's production `App`.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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

fn ctrl(s: &mut EditorState, c: char) {
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char(c), KeyModifiers::CONTROL),
    );
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        press(s, KeyCode::Char(ch));
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

fn buffer_text(s: &EditorState) -> String {
    let b: mlua::String = eval(
        s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
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

/// The buffer the window shows, the caret's 0-based line and column,
/// and the view's first line.
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

/// Put the caret at `(line, col)`, 0-based, with the view's first line
/// at `top`.
fn put_caret(s: &EditorState, text: &str, line: usize, col: usize, top: usize) {
    let byte: usize = text
        .split_inclusive('\n')
        .take(line)
        .map(str::len)
        .sum::<usize>()
        + col;
    exec(s, &format!("pmacs.editor.goto_byte({byte})"));
    exec(s, &format!("pmacs.editor.set_view_top({top})"));
}

const INITIALIZED: &str = "(function() \
   for _,r in ipairs(pmacs.lsp.list()) do \
     if r.state and r.state.kind=='initialized' then return true end \
   end \
   return false \
 end)()";

/// Forty comment lines, then the function a rename edits on lines 40 to
/// 42: long enough that the view has a first line to lose.
fn source() -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for i in 0..40 {
        let _ = writeln!(text, "// {i}");
    }
    text.push_str("fn count(total: u32) -> u32 {\n    let x = total + total;\n    total * x + total\n}\n// tail\n");
    text
}

fn renamed() -> String {
    source()
        .replace("fn count(total: u32)", "fn count(n: u32)")
        .replace("let x = total + total;", "let x = n + n;")
        .replace("total * x + total", "n * x + n")
}

const OTHER: &str = "// another file\nfn other(total: u32) -> u32 { total }\n";
const OTHER_RENAMED: &str = "// another file\nfn other(n: u32) -> u32 { n }\n";

fn edit(line: u32, start: u32, end: u32) -> serde_json::Value {
    serde_json::json!({
        "range": {
            "start": { "line": line, "character": start },
            "end": { "line": line, "character": end }
        },
        "newText": "n"
    })
}

/// The five occurrences of `total` in `a.rs`: one above the caret's
/// line, two on it, two below.
fn edits_in_source() -> serde_json::Value {
    serde_json::json!([
        edit(40, 9, 14),
        edit(41, 12, 17),
        edit(41, 20, 25),
        edit(42, 4, 9),
        edit(42, 16, 21),
    ])
}

fn edits_in_other() -> serde_json::Value {
    serde_json::json!([edit(1, 9, 14), edit(1, 30, 35)])
}

fn uri(path: &Path) -> String {
    format!("file://{}", std::fs::canonicalize(path).unwrap().display())
}

/// `a.rs` and `b.rs` in a fresh directory, and the plan file the fake
/// reads its `WorkspaceEdit` from.
struct Fixture {
    dir: PathBuf,
    a: PathBuf,
    b: PathBuf,
    plan: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("pmacs-e10a3-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b) = (dir.join("a.rs"), dir.join("b.rs"));
    std::fs::write(&a, source()).unwrap();
    std::fs::write(&b, OTHER).unwrap();
    let plan = dir.join("plan.json");
    Fixture { dir, a, b, plan }
}

impl Fixture {
    fn write_plan(&self, plan: &serde_json::Value) {
        std::fs::write(&self.plan, serde_json::to_string(plan).unwrap()).unwrap();
    }

    /// The edit of `a.rs` alone.
    fn plan_one(&self) {
        let mut changes = serde_json::Map::new();
        changes.insert(uri(&self.a), edits_in_source());
        self.write_plan(&serde_json::json!({ "changes": changes }));
    }

    /// The edits of both files, in the order given.
    fn plan_two(&self, source_first: bool) {
        let source = serde_json::json!({
            "textDocument": { "uri": uri(&self.a), "version": null },
            "edits": edits_in_source()
        });
        let other = serde_json::json!({
            "textDocument": { "uri": uri(&self.b), "version": null },
            "edits": edits_in_other()
        });
        let changes = if source_first {
            vec![source, other]
        } else {
            vec![other, source]
        };
        self.write_plan(&serde_json::json!({ "documentChanges": changes }));
    }

    /// The Lua that points the rust server at the fake carrying the
    /// plan, as an `init.lua` or a daemon's init holds it. The fake
    /// negotiates UTF-8 columns, as rust-analyzer does: under UTF-16 an
    /// edit in a file other than the one a request names has its
    /// columns converted against the wrong document, which is its own
    /// defect and not this suite's subject.
    fn server_config(&self) -> String {
        format!(
            "pmacs.lsp.config = {{}}\n\
             pmacs.lsp.config.rust = {{\n\
               command = '{}',\n\
               env = {{\n\
                 PMACS_FAKE_LSP_MODE = 'applyeditplan',\n\
                 PMACS_FAKE_LSP_POSITION_ENCODING = 'utf-8',\n\
                 PMACS_FAKE_LSP_EDIT_PLAN = '{}',\n\
                 PMACS_FAKE_LSP_APPLYEDIT_SINK = '{}',\n\
               }},\n\
             }}\n",
            fake_lsp_path(),
            self.plan.display(),
            self.dir.join("sink.json").display()
        )
    }

    /// An editor visiting `a.rs`, the fake initialized.
    fn editor(&self) -> EditorState {
        let s = EditorState::new_with_roots(&iso::roots());
        s.lua_host.lua().remove_app_data::<StateDir>();
        s.lua_host.lua().set_app_data(StateDir(self.dir.clone()));
        exec(&s, &self.server_config());
        exec(
            &s,
            &format!(
                "pmacs.buffer.find_or_open({:?})",
                self.a.display().to_string()
            ),
        );
        let mut s = s;
        assert!(
            pump_until(&mut s, 10, |s| eval(s, &format!("return {INITIALIZED}"))),
            "fake server init"
        );
        s
    }
}

/// `C-c r`, a name, `RET`, and the wait for the applier's word.
fn rename(s: &mut EditorState) {
    ctrl(s, 'c');
    press(s, KeyCode::Char('r'));
    type_str(s, "n");
    press(s, KeyCode::Enter);
    assert!(
        pump_until(s, 10, |s| status(s).starts_with("LSP: renamed")),
        "the rename must be applied; status {:?}",
        status(s)
    );
}

// ---------------------------------------------------------------------------
// One buffer
// ---------------------------------------------------------------------------

/// A rename with edits above the caret, before it on its line, at it
/// and after it. The caret stays on the renamed symbol, moved by what
/// the edits before it took, and the view's first line is as it was. At
/// the base every case ends at line 0, column 0, with the view at 0.
#[test]
fn e10a_3_a_rename_in_one_buffer_keeps_the_caret_on_its_symbol_and_the_view() {
    // (caret column on line 41, where it must be afterwards, what)
    let cases = [
        (
            22,
            17,
            "inside the second `total`: its offset, as far as `n` reaches",
        ),
        (
            20,
            16,
            "at the start of the second `total`: the start of its `n`",
        ),
        (
            25,
            17,
            "at the end of the second `total`: the end of its `n`",
        ),
        (
            10,
            10,
            "before every edit on its line: moved by nothing on it",
        ),
    ];
    for (col, after, what) in cases {
        let fx = fixture(&format!("one-{col}"));
        fx.plan_one();
        let mut s = fx.editor();
        put_caret(&s, &source(), 41, col, 30);
        assert_eq!(
            place(&s),
            ("a.rs".to_owned(), 41, i64::try_from(col).unwrap(), 30),
            "premise: the caret and the view before the rename"
        );
        rename(&mut s);
        assert_eq!(buffer_text(&s), renamed(), "the edit landed");
        assert_eq!(
            place(&s),
            ("a.rs".to_owned(), 41, after, 30),
            "the caret {what}; the view's first line unchanged"
        );
    }
}

/// A caret below every edit keeps its line and column, and one above
/// every edit is not moved at all.
#[test]
fn e10a_3_a_caret_clear_of_the_edits_stays_where_it_is() {
    for (line, col) in [(43, 1), (5, 3)] {
        let fx = fixture(&format!("clear-{line}"));
        fx.plan_one();
        let mut s = fx.editor();
        put_caret(&s, &source(), line, col, 2);
        rename(&mut s);
        assert_eq!(buffer_text(&s), renamed());
        assert_eq!(
            place(&s),
            (
                "a.rs".to_owned(),
                i64::try_from(line).unwrap(),
                i64::try_from(col).unwrap(),
                2
            )
        );
    }
}

// ---------------------------------------------------------------------------
// Two buffers
// ---------------------------------------------------------------------------

/// A rename that edits the shown file and another one, in either order:
/// the window still shows the file the rename was asked from, the caret
/// is on its symbol, the view is as it was, and the other file holds its
/// edit. With the other file last, the applier ends in it and switches
/// back; with it first, the last switch is to the buffer the window
/// already shows. Both zeroed the caret at the base.
#[test]
fn e10a_3_a_rename_across_two_buffers_keeps_the_place_in_the_one_shown() {
    for source_first in [true, false] {
        let fx = fixture(if source_first { "two-ab" } else { "two-ba" });
        fx.plan_two(source_first);
        let mut s = fx.editor();
        put_caret(&s, &source(), 41, 22, 30);
        rename(&mut s);
        assert_eq!(
            place(&s),
            ("a.rs".to_owned(), 41, 17, 30),
            "the window shows the file the rename was asked from, the caret on its symbol \
             (source first: {source_first})"
        );
        assert_eq!(buffer_text(&s), renamed());
        assert_eq!(
            named_text(&s, "b.rs"),
            OTHER_RENAMED,
            "the file no window shows holds its edit"
        );
    }
}

// ---------------------------------------------------------------------------
// The applier's other users
// ---------------------------------------------------------------------------

/// A code action's inline edit goes through the same applier: `C-c a`,
/// the second action, which replaces three bytes of line 0. The caret,
/// forty-one lines below, stays where it is. At the base it went to the
/// top.
#[test]
fn e10a_3_a_code_actions_edit_keeps_the_place() {
    let fx = fixture("action");
    fx.plan_one();
    let mut s = fx.editor();
    put_caret(&s, &source(), 41, 22, 30);
    ctrl(&mut s, 'c');
    press(&mut s, KeyCode::Char('a'));
    assert!(
        pump_until(&mut s, 10, |s| s.core.borrow().minibuffer.is_active()),
        "the action prompt opens; status {:?}",
        status(&s)
    );
    type_str(&mut s, "2");
    press(&mut s, KeyCode::Enter);
    assert!(
        pump_until(&mut s, 10, |s| buffer_text(s).starts_with("// ED1")),
        "the inline edit landed; status {:?}; text starts {:?}",
        status(&s),
        &buffer_text(&s)[..12]
    );
    // The fake's edit is three bytes from column 3 of line 0, which
    // takes that line's end with it: `// 0` and `// 1` become one line,
    // `// ED1/ 1`. The caret is on the same text, a line up, and on the
    // same row of the window.
    assert_eq!(place(&s), ("a.rs".to_owned(), 40, 22, 29));
}

/// A server's own `workspace/applyEdit`, which the first code action's
/// command makes the fake send, carrying the plan: the same applier
/// from the server-request pump. The caret stays on its symbol.
#[test]
fn e10a_3_a_servers_own_apply_edit_keeps_the_place() {
    let fx = fixture("server");
    fx.plan_one();
    let mut s = fx.editor();
    put_caret(&s, &source(), 41, 22, 30);
    ctrl(&mut s, 'c');
    press(&mut s, KeyCode::Char('a'));
    assert!(
        pump_until(&mut s, 10, |s| s.core.borrow().minibuffer.is_active()),
        "the action prompt opens; status {:?}",
        status(&s)
    );
    press(&mut s, KeyCode::Enter);
    assert!(
        pump_until(&mut s, 10, |s| buffer_text(s) == renamed()),
        "the server's edit landed; status {:?}",
        status(&s)
    );
    assert_eq!(place(&s), ("a.rs".to_owned(), 41, 17, 30));
}

/// A format switches no buffer, so its caret never went to the top; it
/// kept its byte while the text moved. The fake's reply takes the four
/// spaces that begin line 0 and adds a `;` on line 3. A caret on line 2
/// stays on its text; one after the `;` moves right with it. At the base
/// the first read line 2, column 9, four bytes along.
#[test]
fn e10a_3_a_format_keeps_the_caret_on_its_text() {
    let text = "    fn a() {}\nfn b() {}\nlet here = 1;\n0123456789\ntail\n";
    let formatted = "fn a() {}\nfn b() {}\nlet here = 1;\n0123456;789\ntail\n";
    for ((line, col), after) in [((2, 5), (2, 5)), ((3, 9), (3, 10)), ((0, 9), (0, 5))] {
        let fx = fixture(&format!("format-{line}"));
        std::fs::write(&fx.a, text).unwrap();
        fx.plan_one();
        let mut s = fx.editor();
        put_caret(&s, text, line, col, 0);
        ctrl(&mut s, 'c');
        press(&mut s, KeyCode::Char('f'));
        assert!(
            pump_until(&mut s, 10, |s| status(s).starts_with("LSP: applied")),
            "the format must be applied; status {:?}",
            status(&s)
        );
        assert_eq!(buffer_text(&s), formatted, "the format landed");
        let (_, got_line, got_col, _) = place(&s);
        assert_eq!(
            (got_line, got_col),
            after,
            "a caret at {line}:{col} follows its text"
        );
    }
}

// ---------------------------------------------------------------------------
// The TUI's screen
// ---------------------------------------------------------------------------

/// The baseline's step on a real TUI in a PTY: the caret walked to the
/// second `total` of line 42, `C-c r`, a name, `RET`. The mode line
/// reads `L42:C18` afterwards and the first row of the screen is the
/// line it was. At the base the mode line read `L1:C1` over `// 0`.
#[test]
fn e10a_3_the_tui_keeps_its_caret_and_its_first_row_across_a_rename() {
    use common::pty::{ScreenTui, spawn_pmacs_in_pty_tapped};

    let fx = fixture("tui");
    fx.plan_one();
    let config = fx.dir.join("config");
    std::fs::create_dir_all(config.join("pmacs")).unwrap();
    std::fs::write(config.join("pmacs/init.lua"), fx.server_config()).unwrap();
    let file = fx.a.display().to_string();
    let (pty, tap) = spawn_pmacs_in_pty_tapped(
        &[&file],
        &[
            ("HOME", fx.dir.as_path()),
            ("XDG_CONFIG_HOME", config.as_path()),
        ],
        24,
        80,
    );
    let mut tui = ScreenTui::new(pty, tap, 24, 80);
    let limit = Duration::from_secs(30);
    tui.wait_for(limit, "the server ready on the mode line", &|t| {
        t.screen_contains("LSP:ready")
    });
    // M-x goto-line RET 42 RET, then twenty-two columns right.
    tui.write(b"\x1bx");
    tui.wait_for(limit, "the M-x prompt", &|t| t.screen_contains("M-x"));
    tui.write(b"goto-line\r");
    tui.wait_for(limit, "the line prompt", &|t| {
        t.bottom_row().to_lowercase().contains("line")
    });
    tui.write(b"42\r");
    tui.wait_for(limit, "the caret on line 42", &|t| {
        t.screen_contains("L42:C1 ")
    });
    for _ in 0..22 {
        tui.write(b"\x06");
    }
    tui.wait_for(limit, "the caret inside the second `total`", &|t| {
        t.screen_contains("L42:C23")
    });
    let first_row = tui.rows()[0].clone();
    assert!(
        !first_row.contains("// 0"),
        "premise: the view is not at the top before the rename; {}",
        tui.describe()
    );

    tui.write(b"\x03r");
    tui.wait_for(limit, "the rename prompt", &|t| {
        t.screen_contains("Rename symbol to:")
    });
    tui.write(b"n\r");
    tui.wait_for(limit, "the applier's word", &|t| {
        t.screen_contains("LSP: renamed")
    });
    assert!(
        tui.screen_contains("L42:C18"),
        "the mode line reads the caret on its renamed symbol; {}",
        tui.describe()
    );
    assert_eq!(
        tui.rows()[0],
        first_row,
        "the first row of the screen is the line it was; {}",
        tui.describe()
    );
    assert!(
        tui.screen_contains("let x = n + n;"),
        "the edit is on screen; {}",
        tui.describe()
    );
}

// ---------------------------------------------------------------------------
// The GPU window
// ---------------------------------------------------------------------------

/// The same step through the GPU's production `App`: the window's own
/// caret and the first line of its view, read before and after. At the
/// base the caret came back at line 1 and the view at 0.
#[cfg(feature = "crdt")]
#[test]
fn e10a_3_the_gpu_window_keeps_its_caret_and_its_view_across_a_rename() {
    use common::daemon::TestDaemon;

    let fx = fixture("gpu");
    fx.plan_one();
    let init = format!(
        "{}pmacs.buffer.find_or_open({:?})\n",
        fx.server_config(),
        fx.a.display().to_string()
    );
    let daemon = TestDaemon::spawn_with_env_and_init(&[], &init);
    let right = "tap:C-f ".repeat(22);
    let steps = format!(
        "shows:a.rs key:M-x type:goto-line key:ret type:42 key:ret quiet:500 {right}quiet:800 \
         place:before key:C-c key:r quiet:500 type:n key:ret band:LSP: quiet:800 place:after"
    );
    let Some(facts) = common::gpu_probe::run_script_probe(daemon.socket_path(), "e10a3", &steps)
    else {
        support::record_skip("the GPU probe did not run");
        return;
    };
    let fact = |key: &str| common::gpu_probe::fact(&facts, key).to_owned();
    assert_eq!(
        (fact("before.name"), fact("before.line"), fact("before.col")),
        ("a.rs".to_owned(), "42".to_owned(), "23".to_owned()),
        "premise: the caret inside the second `total`; facts {facts:?}"
    );
    assert_ne!(
        fact("before.scroll_top"),
        "0",
        "premise: the view is not at the top before the rename; facts {facts:?}"
    );
    assert!(
        fact("after.status").contains("LSP: renamed"),
        "premise: the rename was applied; facts {facts:?}"
    );
    assert_eq!(
        (
            fact("after.name"),
            fact("after.line"),
            fact("after.col"),
            fact("after.scroll_top"),
        ),
        (
            "a.rs".to_owned(),
            "42".to_owned(),
            "18".to_owned(),
            fact("before.scroll_top"),
        ),
        "the caret on its renamed symbol and the view where it was; facts {facts:?}"
    );
}
