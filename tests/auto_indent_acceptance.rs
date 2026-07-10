//! Auto-indent acceptance (Arc 2, docs/auto-indent-framing.md).
//!
//! Dispatch-driven: RET through `dispatch_key`, `M-x` through the real
//! minibuffer. Auto-indent is language-agnostic (Q#AI3 copies bytes),
//! so buffers are plain in-memory scratch buffers — no files, no
//! language detection, no `StateDir`.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

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

/// Fresh editor whose active scratch buffer holds `body`, cursor at 0.
fn editor_with(body: &str) -> EditorState {
    let s = EditorState::new();
    if !body.is_empty() {
        exec(&s, &format!("pmacs.window.buffer():insert(0, {body:?})"));
    }
    exec(&s, "pmacs.editor.goto_byte(0)");
    s
}

// ---------------------------------------------------------------------------
// The indent copy (Q#AI3)
// ---------------------------------------------------------------------------

#[test]
fn ret_at_eol_carries_the_space_indent() {
    let mut s = editor_with("    foo\nbar\n");
    exec(&s, "pmacs.editor.goto_byte(7)"); // end of "    foo"
    press(&mut s, KeyCode::Enter);
    assert_eq!(buffer_text(&s), "    foo\n    \nbar\n");
    assert_eq!(cursor(&s), 12, "cursor lands after the carried indent");
}

#[test]
fn tab_and_mixed_indents_round_trip_verbatim() {
    let mut s = editor_with("\tfoo");
    exec(&s, "pmacs.editor.goto_byte(4)");
    press(&mut s, KeyCode::Enter);
    assert_eq!(buffer_text(&s), "\tfoo\n\t", "a tab indent copies as a tab");
    assert_eq!(cursor(&s), 6);

    let mut s = editor_with("\t  foo");
    exec(&s, "pmacs.editor.goto_byte(6)");
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        buffer_text(&s),
        "\t  foo\n\t  ",
        "mixed tab+space indents copy byte-for-byte"
    );
    assert_eq!(cursor(&s), 10);
}

#[test]
fn mid_line_split_carries_the_tail_onto_the_indented_line() {
    let mut s = editor_with("    foobar");
    exec(&s, "pmacs.editor.goto_byte(7)"); // between "foo" and "bar"
    press(&mut s, KeyCode::Enter);
    assert_eq!(buffer_text(&s), "    foo\n    bar");
    assert_eq!(cursor(&s), 12, "cursor sits before the carried tail");
}

#[test]
fn split_inside_the_leading_whitespace_does_not_double_indent() {
    // Q#AI3 clip rule: `··|··foo` → `··` / `····foo` — the carried
    // text keeps its TOTAL indentation (4), instead of gaining the
    // full 4-wide indent on top of its remaining 2 spaces.
    let mut s = editor_with("    foo");
    exec(&s, "pmacs.editor.goto_byte(2)");
    press(&mut s, KeyCode::Enter);
    assert_eq!(buffer_text(&s), "  \n    foo");
    assert_eq!(cursor(&s), 5, "cursor after the clipped indent");
}

#[test]
fn zero_indent_and_empty_buffer_match_plain_newline() {
    let mut s = editor_with("");
    press(&mut s, KeyCode::Enter);
    assert_eq!(buffer_text(&s), "\n");
    assert_eq!(cursor(&s), 1);

    let mut s = editor_with("foo");
    exec(&s, "pmacs.editor.goto_byte(3)");
    press(&mut s, KeyCode::Enter);
    assert_eq!(buffer_text(&s), "foo\n");
    assert_eq!(cursor(&s), 4);
}

#[test]
fn giant_minified_line_splits_correctly() {
    // PR #109 round 1 finding 4: the indent scan is forward-chunked
    // and stops at the first non-whitespace byte, so Enter at the end
    // of a huge unindented line never materializes the line. This
    // pins the behavior; boundedness is by construction.
    let long = "x".repeat(64 * 1024);
    let mut s = editor_with(&long);
    exec(&s, &format!("pmacs.editor.goto_byte({})", long.len()));
    press(&mut s, KeyCode::Enter);
    assert_eq!(buffer_text(&s), format!("{long}\n"));
    assert_eq!(cursor(&s) as usize, long.len() + 1);

    // And an indented giant line still carries exactly its indent.
    let body = format!("  {long}");
    let mut s = editor_with(&body);
    exec(&s, &format!("pmacs.editor.goto_byte({})", body.len()));
    press(&mut s, KeyCode::Enter);
    assert_eq!(buffer_text(&s), format!("{body}\n  "));
}

#[test]
fn whitespace_only_line_copies_and_the_abandoned_line_keeps_its_whitespace() {
    // Named non-goal (Q#AI3): no trailing-whitespace cleanup on the
    // line being left behind.
    let mut s = editor_with("    ");
    exec(&s, "pmacs.editor.goto_byte(4)");
    press(&mut s, KeyCode::Enter);
    assert_eq!(buffer_text(&s), "    \n    ");
    assert_eq!(cursor(&s), 9);
}

// ---------------------------------------------------------------------------
// Region type-over + selections (Q#AI4)
// ---------------------------------------------------------------------------

#[test]
fn region_ret_is_one_replace_one_undo_step_and_clears_the_selection() {
    let mut s = editor_with("    hello world");
    exec(
        &s,
        "pmacs.editor.begin_selection(4); pmacs.editor.goto_byte(15)",
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        buffer_text(&s),
        "    \n    ",
        "the region is replaced by newline+indent in one edit"
    );
    assert_eq!(cursor(&s), 9);
    let region_active: bool = eval(&s, "return pmacs.editor.region() ~= nil");
    assert!(!region_active, "selection clears after a region RET");
    ctrl(&mut s, '/'); // buffer.undo, exactly once
    assert_eq!(
        buffer_text(&s),
        "    hello world",
        "one undo restores the whole type-over"
    );
}

#[test]
fn plain_ret_is_one_undo_step() {
    let mut s = editor_with("  ab");
    exec(&s, "pmacs.editor.goto_byte(4)");
    press(&mut s, KeyCode::Enter);
    assert_eq!(buffer_text(&s), "  ab\n  ");
    ctrl(&mut s, '/');
    assert_eq!(buffer_text(&s), "  ab");
}

#[test]
fn zero_length_selection_does_not_type_over_the_fresh_newline() {
    // Q#AI4: an armed anchor at the cursor reports no region; the RET
    // moves the cursor off it, so without the unconditional clear the
    // next self-insert would replace the newline ("S-Left at BOF,
    // RET, x" → "x").
    let mut s = editor_with("");
    exec(&s, "pmacs.editor.begin_selection(0)");
    press(&mut s, KeyCode::Enter);
    type_str(&mut s, "x");
    assert_eq!(buffer_text(&s), "\nx", "the newline survives the 'x'");

    // Q#AI9: the retained plain-newline escape hatch (through the
    // fixed core arm) behaves the same.
    let mut s = editor_with("");
    exec(&s, "pmacs.editor.begin_selection(0)");
    m_x(&mut s, "buffer.newline");
    type_str(&mut s, "x");
    assert_eq!(buffer_text(&s), "\nx");
}

// ---------------------------------------------------------------------------
// Intercept discipline (Q#AI5)
// ---------------------------------------------------------------------------

#[test]
fn rejecting_intercept_reports_without_throwing_or_mutating() {
    let mut s = editor_with("  a");
    exec(&s, "pmacs.editor.goto_byte(3)");
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
    press(&mut s, KeyCode::Enter);
    assert!(status(&s).contains("rejected"), "got: {:?}", status(&s));
    assert_eq!(buffer_text(&s), "  a");
    assert_eq!(cursor(&s), 3, "no cursor motion on a rejected RET");
    press(&mut s, KeyCode::Enter); // allowed again: works
    assert_eq!(buffer_text(&s), "  a\n  ");
}

#[test]
fn relocating_intercept_moves_the_payload_but_does_not_teleport_the_cursor() {
    // The only transform an insert admits (M6.4): moving its `pos`.
    // The payload lands where the intercept sent it; the cursor is
    // translated through the edit, NOT jumped to the remote site.
    let mut s = editor_with("  abc");
    exec(&s, "pmacs.editor.goto_byte(5)");
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "insert" then
            return { kind = "insert", pos = 0, bytes = op.bytes }
          end
          return nil
        end)
        "#,
    );
    press(&mut s, KeyCode::Enter);
    assert!(status(&s).contains("altered"), "got: {:?}", status(&s));
    assert_eq!(
        buffer_text(&s),
        "\n    abc",
        "the newline+indent payload landed at the intercept's position"
    );
    assert_eq!(
        cursor(&s),
        8,
        "cursor shifted right by the inserted length (5+3), not teleported to the edit"
    );
}

#[test]
fn shrinking_intercept_leaves_a_valid_cursor_and_no_selection() {
    // Only a replace can shrink the buffer (M6.4): the intercept
    // expands the replaced range past the payload. The cursor must be
    // right-gravity-translated into the shrunken buffer — validity,
    // not immobility — and the selection cleared.
    let mut s = editor_with("  hello world wide");
    exec(
        &s,
        "pmacs.editor.begin_selection(2); pmacs.editor.goto_byte(7)",
    );
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "replace" then
            return {
              kind = "replace",
              start = op.start,
              ["end"] = op["end"] + 8,
              bytes = op.bytes,
            }
          end
          return nil
        end)
        "#,
    );
    press(&mut s, KeyCode::Enter);
    assert!(status(&s).contains("altered"), "got: {:?}", status(&s));
    assert_eq!(buffer_text(&s), "  \n  ide", "the expanded replace stands");
    let len: i64 = eval(&s, "return pmacs.window.buffer():len()");
    assert_eq!(cursor(&s), 5, "cursor translated to the edit's new end");
    assert!(cursor(&s) <= len, "cursor within the shrunken buffer");
    let region_active: bool = eval(&s, "return pmacs.editor.region() ~= nil");
    assert!(!region_active, "selection cleared under the same guard");
}

#[test]
fn context_switching_intercept_skips_fixup_and_leaves_the_new_context_alone() {
    // Q#AI5 guard: an intercept may switch the active window/buffer
    // (the registry borrow is released). The fix-up must not touch
    // whatever is active afterwards. This proves the NEW context is
    // untouched; the original window's state after such an intercept
    // is the substrate-reconciliation deferral's territory.
    let mut s = editor_with("  a");
    exec(&s, "pmacs.editor.goto_byte(3)");
    exec(
        &s,
        r#"
        _G.orig = pmacs.window.buffer()
        _G.other = pmacs.buffer.create("*other*")
        pmacs.buffer.add_intercept(_G.orig, function(_op)
          pmacs.window.switch_buffer(_G.other)
          return nil
        end)
        "#,
    );
    press(&mut s, KeyCode::Enter);
    assert!(
        status(&s).contains("context changed"),
        "got: {:?}",
        status(&s)
    );
    let name: String = eval(&s, "return pmacs.window.buffer():name()");
    assert_eq!(name, "*other*", "the intercept's buffer switch stands");
    assert_eq!(cursor(&s), 0, "the new context's cursor is untouched");
    let orig: mlua::String = eval(&s, "return _G.orig:slice(0, _G.orig:len())");
    assert_eq!(
        String::from_utf8_lossy(&orig.as_bytes()),
        "  a\n  ",
        "the edit itself landed in the original buffer"
    );
}

// ---------------------------------------------------------------------------
// Search staleness through RET (Q#AI8)
// ---------------------------------------------------------------------------

#[test]
fn accepted_search_navigation_fails_closed_after_ret() {
    let mut s = editor_with("ind ind ind");
    ctrl(&mut s, 's');
    type_str(&mut s, "ind");
    press(&mut s, KeyCode::Enter); // accept: matches stay until an edit
    exec(&s, "pmacs.editor.goto_byte(11)");
    press(&mut s, KeyCode::Enter); // auto-indent RET marks them stale
    assert_eq!(buffer_text(&s), "ind ind ind\n");
    let at = cursor(&s);
    exec(&s, "pmacs.editor.search_step(true)");
    assert_eq!(
        cursor(&s),
        at,
        "post-accept navigation is a no-op once RET staled the matches"
    );
}

#[test]
fn direct_lua_edit_stales_accepted_search_navigation() {
    let mut s = editor_with("foo foo");
    ctrl(&mut s, 's');
    type_str(&mut s, "foo");
    press(&mut s, KeyCode::Enter); // accept
    exec(&s, "pmacs.window.buffer():insert(0, \"zz\")"); // notify path
    let at = cursor(&s);
    exec(&s, "pmacs.editor.search_step(true)");
    assert_eq!(
        cursor(&s),
        at,
        "a direct buf:insert must stale the matches like any edit"
    );
}

// ---------------------------------------------------------------------------
// Substrate plumbing (Q#AI7)
// ---------------------------------------------------------------------------

#[test]
fn after_edit_fires_exactly_once_per_ret_keybound_and_m_x() {
    let mut s = editor_with("  a");
    exec(&s, "pmacs.editor.goto_byte(3)");
    exec(
        &s,
        "_G.ae = 0; pmacs.hook.add('buffer.after-edit', function() _G.ae = _G.ae + 1 end)",
    );
    press(&mut s, KeyCode::Enter);
    let n: i64 = eval(&s, "return _G.ae");
    assert_eq!(n, 1, "keybound RET fires after-edit once");
    m_x(&mut s, "edit.newline-and-indent");
    assert_eq!(buffer_text(&s), "  a\n  \n  ");
    let n: i64 = eval(&s, "return _G.ae");
    assert_eq!(n, 2, "M-x RET fires after-edit exactly once more");
}

#[test]
fn ret_between_kills_breaks_the_kill_chain() {
    let mut s = editor_with("one\ntwo\nthree\n");
    ctrl(&mut s, 'k'); // kills "one"; line now blank, cursor 0
    press(&mut s, KeyCode::Enter); // rotates the command boundary
    ctrl(&mut s, 'k'); // kills "\n" — must push fresh, not append
    let ring: Vec<String> = eval(&s, "return pmacs.killring.list()");
    assert_eq!(
        ring,
        vec!["\n", "one"],
        "C-k, RET, C-k yields two ring entries (chain broken)"
    );
}

#[test]
fn this_command_during_ret_is_the_new_command() {
    let mut s = editor_with("");
    exec(
        &s,
        "pmacs.hook.add('buffer.after-edit', function() _G.tc = pmacs.editor.this_command() end)",
    );
    press(&mut s, KeyCode::Enter);
    let tc: String = eval(&s, "return _G.tc");
    assert_eq!(tc, "edit.newline-and-indent");
}

// ---------------------------------------------------------------------------
// Contexts RET must not disturb (ground truth: consumed before the keymap)
// ---------------------------------------------------------------------------

#[test]
fn minibuffer_and_buffer_list_ret_are_unaffected() {
    // The m_x helper itself proves minibuffer accept (used throughout
    // this suite). The classic buffer list's RET is a buffer-local
    // binding through normal dispatch (ground truth) — it must visit,
    // not newline-and-indent into the list.
    let mut s = editor_with("hello");
    ctrl(&mut s, 'x');
    ctrl(&mut s, 'b');
    let name: String = eval(&s, "return pmacs.window.buffer():name()");
    assert_eq!(name, "*buffer-list*");
    let listed = buffer_text(&s);
    exec(&s, "_G.list = pmacs.window.buffer()");
    press(&mut s, KeyCode::Enter); // buffer-local RET: visit
    let name: String = eval(&s, "return pmacs.window.buffer():name()");
    assert_ne!(name, "*buffer-list*", "RET visits instead of inserting");
    let list_after: String = eval(
        &s,
        "if not _G.list:is_valid() then return \"<gone>\" end \
         return _G.list:slice(0, _G.list:len())",
    );
    assert!(
        list_after == "<gone>" || list_after == listed,
        "no newline landed in the buffer list"
    );
}

#[test]
fn isearch_ret_accepts_instead_of_inserting() {
    let mut s = editor_with("abc abc");
    ctrl(&mut s, 's');
    type_str(&mut s, "abc");
    press(&mut s, KeyCode::Enter); // isearch accept, consumed pre-keymap
    assert_eq!(
        buffer_text(&s),
        "abc abc",
        "RET during isearch accepts; no newline is inserted"
    );
}
