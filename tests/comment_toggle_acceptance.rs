//! Comment-toggle acceptance (Arc 2, the archived comment-toggle framing).
//!
//! Dispatch-driven: `M-;` through `dispatch_key`, `M-x` through the
//! real minibuffer. Buffers are file-backed (language detection needs
//! a path); each editor gets a private tempdir `StateDir` and an
//! emptied `pmacs.lsp.config` so opening `.rs`/`.py` fixtures never
//! spawns a real language server.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fresh_state_dir() -> PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pmacs-comment-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn editor(state_dir: &std::path::Path) -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host
        .lua()
        .set_app_data(StateDir(state_dir.to_path_buf()));
    // Language DETECTION must work (filetypes/grammars); server
    // SPAWNING must not (rust/python have default configs).
    exec(&s, "pmacs.lsp.config = {}");
    s
}

fn write_file(dir: &std::path::Path, name: &str, body: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p.display().to_string()
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

fn m_x(s: &mut EditorState, name: &str) {
    alt(s, 'x');
    type_str(s, name);
    press(s, KeyCode::Enter);
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn buffer_text(s: &EditorState) -> String {
    let b: mlua::String = eval(
        s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
}

fn cursor(s: &EditorState) -> i64 {
    eval(s, "return pmacs.editor.cursor()")
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

/// Fresh editor visiting `name` (created in the state tempdir) with
/// `body` on disk, cursor at 0.
fn editor_visiting(name: &str, body: &str) -> EditorState {
    let dir = fresh_state_dir();
    let s = editor(&dir);
    let f = write_file(&dir, name, body);
    exec(&s, &format!("pmacs.buffer.find_or_open({f:?})"));
    exec(&s, "pmacs.editor.goto_byte(0)");
    s
}

// ---------------------------------------------------------------------------
// Single-line toggle (comment-line behavior, Q#CT2)
// ---------------------------------------------------------------------------

#[test]
fn rust_line_toggle_round_trips_and_cursor_walks_to_the_next_line() {
    let mut s = editor_visiting("t.rs", "fn main() {\n    let x = 1;\n}\n");
    exec(&s, "pmacs.editor.goto_byte(16)"); // inside "    let x = 1;"
    alt(&mut s, ';');
    assert_eq!(buffer_text(&s), "fn main() {\n    // let x = 1;\n}\n");
    assert_eq!(cursor(&s), 30, "cursor moved to the next line's start");
    // Toggle back from anywhere in the commented line: exact round
    // trip, including the padding space.
    exec(&s, "pmacs.editor.goto_byte(14)");
    alt(&mut s, ';');
    assert_eq!(buffer_text(&s), "fn main() {\n    let x = 1;\n}\n");
    assert_eq!(cursor(&s), 27);
}

#[test]
fn lua_buffer_gets_the_dash_dash_prefix() {
    let mut s = editor_visiting("t.lua", "local x = 1\nreturn x\n");
    alt(&mut s, ';');
    assert_eq!(buffer_text(&s), "-- local x = 1\nreturn x\n");
    assert_eq!(cursor(&s), 15);
}

#[test]
fn last_line_without_newline_toggles_and_clamps_the_cursor() {
    let mut s = editor_visiting("e.py", "x = 1");
    alt(&mut s, ';');
    assert_eq!(buffer_text(&s), "# x = 1");
    assert_eq!(cursor(&s), 7, "no next line: cursor clamps to buffer end");
    alt(&mut s, ';');
    assert_eq!(buffer_text(&s), "x = 1");
}

#[test]
fn a_blank_line_is_a_noop_with_a_status() {
    let mut s = editor_visiting("b.py", "\n  \n");
    alt(&mut s, ';');
    assert!(
        status(&s).contains("nothing to comment"),
        "got: {:?}",
        status(&s)
    );
    assert_eq!(buffer_text(&s), "\n  \n");
}

// ---------------------------------------------------------------------------
// Region toggles (Q#CT4)
// ---------------------------------------------------------------------------

#[test]
fn region_comments_at_min_indent_skips_blanks_and_clears_the_selection() {
    let mut s = editor_visiting("t.py", "  two\nzero\n\n    four\n");
    exec(
        &s,
        "pmacs.editor.begin_selection(0); pmacs.editor.goto_byte(20)",
    );
    alt(&mut s, ';');
    // Min indent across non-blank lines is 0 (line "zero"), so every
    // prefix lands at column 0; the blank line is untouched.
    assert_eq!(buffer_text(&s), "#   two\n# zero\n\n#     four\n");
    let region_active: bool = eval(&s, "return pmacs.editor.region() ~= nil");
    assert!(!region_active, "selection clears after a region toggle");
    assert_eq!(cursor(&s), 0, "cursor lands at the span start");
}

#[test]
fn mixed_region_comments_preserving_inner_prefixes_and_round_trips() {
    let mut s = editor_visiting("t2.py", "# a\nb\n");
    exec(
        &s,
        "pmacs.editor.begin_selection(0); pmacs.editor.goto_byte(5)",
    );
    alt(&mut s, ';');
    // Mixed span COMMENTS (Q#CT4): the already-commented line gets a
    // second prefix, preserving the inner commented-out code.
    assert_eq!(buffer_text(&s), "# # a\n# b\n");
    exec(
        &s,
        "pmacs.editor.begin_selection(0); pmacs.editor.goto_byte(9)",
    );
    alt(&mut s, ';');
    // Now every line is commented → uncomment strips the outer layer.
    assert_eq!(buffer_text(&s), "# a\nb\n", "double-prefix round-trips");
}

#[test]
fn region_ending_at_column_zero_excludes_that_line() {
    let mut s = editor_visiting("c.py", "one\ntwo\n");
    exec(
        &s,
        "pmacs.editor.begin_selection(0); pmacs.editor.goto_byte(4)",
    );
    alt(&mut s, ';');
    assert_eq!(
        buffer_text(&s),
        "# one\ntwo\n",
        "a region stopping at a line's column 0 does not touch that line"
    );
}

// ---------------------------------------------------------------------------
// Unknown language (Q#CT3)
// ---------------------------------------------------------------------------

#[test]
fn unknown_language_reports_and_edits_nothing() {
    let mut s = editor_visiting("t.txt", "hello\n");
    alt(&mut s, ';');
    assert!(
        status(&s).contains("no comment syntax known"),
        "got: {:?}",
        status(&s)
    );
    assert_eq!(buffer_text(&s), "hello\n");
}

#[test]
fn pathless_scratch_buffer_reports_and_edits_nothing() {
    let dir = fresh_state_dir();
    let mut s = editor(&dir);
    type_str(&mut s, "hello");
    exec(&s, "pmacs.editor.goto_byte(0)");
    alt(&mut s, ';');
    assert!(
        status(&s).contains("no comment syntax known"),
        "got: {:?}",
        status(&s)
    );
    assert_eq!(buffer_text(&s), "hello");
}

// ---------------------------------------------------------------------------
// One edit, one undo step (Q#CT5)
// ---------------------------------------------------------------------------

#[test]
fn a_multi_line_toggle_is_one_undo_step() {
    let mut s = editor_visiting("u.py", "a = 1\nb = 2\n");
    exec(
        &s,
        "pmacs.editor.begin_selection(0); pmacs.editor.goto_byte(11)",
    );
    alt(&mut s, ';');
    assert_eq!(buffer_text(&s), "# a = 1\n# b = 2\n");
    ctrl(&mut s, '/'); // buffer.undo, exactly once
    assert_eq!(
        buffer_text(&s),
        "a = 1\nb = 2\n",
        "one undo restores the whole multi-line toggle"
    );
}

// ---------------------------------------------------------------------------
// Intercept discipline (Q#CT5)
// ---------------------------------------------------------------------------

#[test]
fn rejecting_intercept_reports_without_throwing() {
    let mut s = editor_visiting("i.py", "a\nb\n");
    exec(
        &s,
        r#"
        _G.reject_once = true
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(_op)
          if _G.reject_once then
            _G.reject_once = false
            error("rejected by test intercept")
          end
          return nil
        end)
        "#,
    );
    alt(&mut s, ';'); // rejected: reported, nothing changed, no throw
    assert!(status(&s).contains("rejected"), "got: {:?}", status(&s));
    assert_eq!(buffer_text(&s), "a\nb\n");
    assert_eq!(cursor(&s), 0, "no cursor fix-up on a rejected toggle");
    alt(&mut s, ';'); // allowed again: the command still works
    assert_eq!(buffer_text(&s), "# a\nb\n");
}

#[test]
fn transforming_intercept_is_reported_and_skips_the_cursor_fixup() {
    let mut s = editor_visiting("j.py", "ab\ncd\n");
    // Enlarges every replace's end by one byte — the effective edit
    // deviates from the request, so the toggle must report and leave
    // the cursor alone (the span it would fix up toward moved).
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "replace" then
            return {
              kind = "replace",
              start = op.start,
              ["end"] = op["end"] + 1,
              bytes = op.bytes,
            }
          end
          return nil
        end)
        "#,
    );
    alt(&mut s, ';');
    assert!(status(&s).contains("altered"), "got: {:?}", status(&s));
    // The interceptor's result stands (accepted post-hoc semantics):
    // it swallowed the newline after "ab".
    assert_eq!(buffer_text(&s), "# abcd\n");
    assert_eq!(cursor(&s), 0, "cursor fix-up skipped");
}

// ---------------------------------------------------------------------------
// Substrate plumbing (Q#CT6)
// ---------------------------------------------------------------------------

#[test]
fn after_edit_fires_exactly_once_per_toggle_keybound_and_m_x() {
    let mut s = editor_visiting("h.py", "a\nb\n");
    exec(
        &s,
        "_G.ae = 0; pmacs.hook.add('buffer.after-edit', function() _G.ae = _G.ae + 1 end)",
    );
    alt(&mut s, ';'); // keybound path
    let n: i64 = eval(&s, "return _G.ae");
    assert_eq!(n, 1, "keybound toggle fires after-edit once");
    // Cursor walked to line 2; M-x path must fire it too (via
    // invoke_interactive + with_after_edit_check), and the minibuffer
    // typing itself must not inflate the count.
    m_x(&mut s, "edit.toggle-comment");
    assert_eq!(buffer_text(&s), "# a\n# b\n");
    let n: i64 = eval(&s, "return _G.ae");
    assert_eq!(n, 2, "M-x toggle fires after-edit exactly once more");
}

#[test]
fn toggle_between_kills_breaks_the_kill_chain() {
    let mut s = editor_visiting("k.rs", "one\ntwo\nthree\n");
    ctrl(&mut s, 'k'); // kills "one"; line now blank, cursor 0
    alt(&mut s, ';'); // no-op on the blank line, but the command ROTATES
    ctrl(&mut s, 'k'); // kills "\n" — must push fresh, not append
    let ring: Vec<String> = eval(&s, "return pmacs.killring.list()");
    assert_eq!(
        ring,
        vec!["\n", "one"],
        "C-k, M-;, C-k yields two ring entries (chain broken)"
    );
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
