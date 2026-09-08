//! Editing-conveniences acceptance (the archived editing-conveniences framing).
//!
//! Dispatch-driven where a binding exists (`pmacs.command.invoke`
//! bypasses dispatch — a dead binding would pass vacuously), with
//! minibuffer sessions completed by DISPATCHING RET / C-g: the Lua
//! lifecycle `accept()` bypasses `with_after_edit_check`, a path
//! interactive key input never takes. M-x-only commands go through
//! the real M-x minibuffer. The origin/silent-replacement matrices
//! ride the same unregistered-second-`FrontendId` shape as the
//! kill-ring suite.

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

const B: FrontendId = FrontendId(9);

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

fn alt_code(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::ALT));
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn press_as(s: &mut EditorState, fid: FrontendId, code: KeyCode) {
    s.dispatch_key(fid, key(code, KeyModifiers::NONE));
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn lua_str(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
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

fn cursor_line(s: &EditorState) -> i64 {
    eval(s, "return pmacs.editor.cursor_line()")
}

fn region_is_nil(s: &EditorState) -> bool {
    eval(s, "return pmacs.editor.region() == nil")
}

fn ring(s: &EditorState) -> Vec<String> {
    eval(s, "return pmacs.killring.list()")
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

/// Fresh editor whose scratch buffer holds `text` (seeded directly —
/// typing RET would route through edit.newline-and-indent and clone
/// leading whitespace), cursor at 0.
fn editor_with(text: &str) -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    exec(
        &s,
        &format!(
            "local b = pmacs.window.buffer(); b:replace(0, b:len(), {})",
            lua_str(text)
        ),
    );
    exec(&s, "pmacs.editor.goto_byte(0)");
    s
}

/// Complete the open minibuffer session: seed contents directly, then
/// DISPATCH RET (the framing's accept-path requirement).
fn accept_minibuffer(s: &mut EditorState, contents: &str) {
    exec(
        s,
        &format!("pmacs.minibuffer.set_contents({})", lua_str(contents)),
    );
    press(s, KeyCode::Enter);
}

fn m_x(s: &mut EditorState, name: &str) {
    alt(s, 'x');
    type_str(s, name);
    press(s, KeyCode::Enter);
}

fn zap(s: &mut EditorState, ch: &str) {
    alt(s, 'z');
    accept_minibuffer(s, ch);
}

fn undo(s: &mut EditorState) {
    ctrl(s, '/');
}

fn click(s: &mut EditorState) {
    let term = pmacs::cell::CellSize::new(24, 80);
    s.dispatch_mouse(
        FrontendId::LOCAL,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1,
            row: 1,
            modifiers: KeyModifiers::NONE,
        },
        term,
    );
}

// ---------------------------------------------------------------------------
// Boundary-state pin (the Q#EC6 substrate observation, asserted directly)
// ---------------------------------------------------------------------------

#[test]
fn minibuffer_accept_preserves_the_invoking_commands_boundary() {
    let mut s = editor_with("one\ntwo\n");
    exec(
        &s,
        r#"
        pmacs.command.define {
          name = "test.boundary-probe",
          description = "capture boundary state inside on_accept",
          fn = function()
            pmacs.minibuffer.read {
              prompt = "p: ",
              on_accept = function()
                PROBE_THIS = tostring(pmacs.editor.this_command())
                PROBE_LAST = tostring(pmacs.editor.last_command())
              end,
            }
          end,
        }
        pmacs.keymap.bind { scope = "global", sequence = "C-c q", command = "test.boundary-probe" }
        "#,
    );
    ctrl(&mut s, 'k'); // predecessor: edit.kill-line
    ctrl(&mut s, 'c');
    press(&mut s, KeyCode::Char('q'));
    press(&mut s, KeyCode::Enter);
    let this: String = eval(&s, "return PROBE_THIS");
    let last: String = eval(&s, "return PROBE_LAST");
    assert_eq!(
        this, "test.boundary-probe",
        "this_command survives the prompt"
    );
    assert_eq!(
        last, "edit.kill-line",
        "last_command is the pre-prompt predecessor"
    );
}

// ---------------------------------------------------------------------------
// goto-line (Q#EC3)
// ---------------------------------------------------------------------------

#[test]
fn goto_line_moves_and_pushes_a_jump() {
    let mut s = editor_with("a\nb\nc\nd\ne");
    alt(&mut s, 'g');
    press(&mut s, KeyCode::Char('g'));
    accept_minibuffer(&mut s, "4");
    assert_eq!(cursor_line(&s), 3, "1-based input, 0-based line");
    let back: bool = eval(&s, "return pmacs.editor.jump_back()");
    assert!(back, "goto-line pushed a jump");
    assert_eq!(cursor_line(&s), 0);
}

#[test]
fn goto_line_binds_the_double_alt_form_too() {
    let mut s = editor_with("a\nb\nc");
    alt(&mut s, 'g');
    alt(&mut s, 'g');
    accept_minibuffer(&mut s, "2");
    assert_eq!(cursor_line(&s), 1);
}

#[test]
fn goto_line_zero_clamps_to_the_first_line() {
    let mut s = editor_with("a\nb\nc");
    exec(&s, "pmacs.editor.move_to_line(2)");
    alt(&mut s, 'g');
    press(&mut s, KeyCode::Char('g'));
    accept_minibuffer(&mut s, "0");
    assert_eq!(cursor_line(&s), 0, "Emacs clamps line 0 to the first line");
}

#[test]
fn goto_line_huge_decimal_clamps_to_the_last_line() {
    let mut s = editor_with("a\nb\nc\nd\ne");
    alt(&mut s, 'g');
    press(&mut s, KeyCode::Char('g'));
    accept_minibuffer(&mut s, "9999999999999999999999999");
    assert_eq!(cursor_line(&s), 4, "clamps, never errors");
}

#[test]
fn goto_line_rejects_non_numeric_without_touching_the_jump_stack() {
    let mut s = editor_with("a\nb\nc");
    alt(&mut s, 'g');
    press(&mut s, KeyCode::Char('g'));
    accept_minibuffer(&mut s, "abc");
    assert!(status(&s).contains("enter a line number"));
    assert_eq!(cursor_line(&s), 0, "no motion");
    let back: bool = eval(&s, "return pmacs.editor.jump_back()");
    assert!(!back, "nothing pushed before validation");
}

// ---------------------------------------------------------------------------
// Case ops (Q#EC4)
// ---------------------------------------------------------------------------

#[test]
fn upcase_region_clears_the_selection() {
    let mut s = editor_with("foo bar");
    exec(&s, "pmacs.editor.begin_selection(0)");
    exec(&s, "pmacs.editor.goto_byte(3)");
    alt(&mut s, 'u');
    assert_eq!(buffer_text(&s), "FOO bar");
    assert!(region_is_nil(&s), "selection cleared after the edit");
}

#[test]
fn upcase_mid_word_transforms_the_remainder_and_moves_the_cursor() {
    let mut s = editor_with("foo bar");
    exec(&s, "pmacs.editor.goto_byte(1)");
    alt(&mut s, 'u');
    assert_eq!(buffer_text(&s), "fOO bar", "Emacs mid-word remainder");
    assert_eq!(cursor(&s), 3, "cursor at the span end");
}

#[test]
fn downcase_from_a_separator_takes_the_next_word() {
    let mut s = editor_with("foo BAR baz");
    exec(&s, "pmacs.editor.goto_byte(3)");
    alt(&mut s, 'l');
    assert_eq!(buffer_text(&s), "foo bar baz");
    assert_eq!(cursor(&s), 7);
}

#[test]
fn capitalize_word_and_region() {
    let mut s = editor_with("hELLO");
    alt(&mut s, 'c');
    assert_eq!(buffer_text(&s), "Hello");
    let mut s = editor_with("hello WORLD");
    exec(&s, "pmacs.editor.begin_selection(0)");
    exec(&s, "pmacs.editor.goto_byte(11)");
    alt(&mut s, 'c');
    assert_eq!(
        buffer_text(&s),
        "Hello World",
        "per-word capitalize across the region (Emacs capitalize-region)"
    );
}

#[test]
fn capitalize_is_per_word_with_the_packs_word_class() {
    // Emacs 30.2 parity rows: a digit-led word keeps its letters
    // lowercase; a letter after a digit is not a word start.
    let mut s = editor_with("9abc a9bc");
    exec(&s, "pmacs.editor.begin_selection(0)");
    exec(&s, "pmacs.editor.goto_byte(9)");
    alt(&mut s, 'c');
    assert_eq!(buffer_text(&s), "9abc A9bc");
    // The named deviation: `_` is a word constituent in this pack's
    // ASCII class (Emacs's symbol-syntax `_` would give "Foo_Bar").
    let mut s = editor_with("foo_bar baz");
    exec(&s, "pmacs.editor.begin_selection(0)");
    exec(&s, "pmacs.editor.goto_byte(11)");
    alt(&mut s, 'c');
    assert_eq!(buffer_text(&s), "Foo_bar Baz");
}

#[test]
fn case_ops_report_when_no_word_follows() {
    let mut s = editor_with("foo   ");
    exec(&s, "pmacs.editor.goto_byte(4)");
    alt(&mut s, 'u');
    assert_eq!(buffer_text(&s), "foo   ");
    assert!(status(&s).contains("no word after the cursor"));
}

#[test]
fn case_ops_leave_non_ascii_bytes_identical() {
    // é is not in the ASCII conversion ranges: byte-identical in any
    // process locale, while its ASCII neighbors flip.
    let mut s = editor_with("aéb");
    exec(&s, "pmacs.editor.begin_selection(0)");
    exec(&s, &format!("pmacs.editor.goto_byte({})", "aéb".len()));
    alt(&mut s, 'u');
    assert_eq!(buffer_text(&s), "AéB");
}

// ---------------------------------------------------------------------------
// Transpose-chars (Q#EC5)
// ---------------------------------------------------------------------------

#[test]
fn transpose_chars_swaps_and_advances() {
    let mut s = editor_with("abc");
    exec(&s, "pmacs.editor.goto_byte(1)");
    ctrl(&mut s, 't');
    assert_eq!(buffer_text(&s), "bac");
    assert_eq!(cursor(&s), 2, "Emacs drag-forward");
    undo(&mut s);
    assert_eq!(buffer_text(&s), "abc", "one undo step");
}

#[test]
fn transpose_chars_at_eol_swaps_the_two_before() {
    let mut s = editor_with("ab\ncd");
    exec(&s, "pmacs.editor.goto_byte(2)");
    ctrl(&mut s, 't');
    assert_eq!(buffer_text(&s), "ba\ncd");
    assert_eq!(cursor(&s), 2, "cursor stays at EOL");
}

#[test]
fn transpose_chars_at_eof_swaps_the_two_before() {
    let mut s = editor_with("ab");
    exec(&s, "pmacs.editor.goto_byte(2)");
    ctrl(&mut s, 't');
    assert_eq!(buffer_text(&s), "ba");
}

#[test]
fn transpose_chars_no_ops_at_bob_and_on_single_chars() {
    let mut s = editor_with("ab");
    ctrl(&mut s, 't');
    assert_eq!(buffer_text(&s), "ab");
    assert!(status(&s).contains("not enough characters"));
    let mut s = editor_with("a");
    exec(&s, "pmacs.editor.goto_byte(1)");
    ctrl(&mut s, 't');
    assert_eq!(buffer_text(&s), "a");
}

#[test]
fn transpose_chars_swaps_whole_codepoints() {
    let mut s = editor_with("éx");
    exec(&s, "pmacs.editor.goto_byte(2)"); // between é and x
    ctrl(&mut s, 't');
    assert_eq!(buffer_text(&s), "xé", "intact UTF-8, é dragged forward");
    assert_eq!(cursor(&s), 3);
    let mut s = editor_with("xé");
    exec(&s, "pmacs.editor.goto_byte(1)");
    ctrl(&mut s, 't');
    assert_eq!(buffer_text(&s), "éx");
    assert_eq!(cursor(&s), 3);
}

#[test]
fn transpose_chars_swaps_across_a_newline() {
    let mut s = editor_with("a\nb");
    exec(&s, "pmacs.editor.goto_byte(2)");
    ctrl(&mut s, 't');
    assert_eq!(buffer_text(&s), "ab\n");
}

#[test]
fn transpose_chars_fails_closed_on_a_continuation_byte() {
    let mut s = editor_with("éx");
    exec(&s, "pmacs.editor.goto_byte(1)"); // inside é
    ctrl(&mut s, 't');
    assert_eq!(buffer_text(&s), "éx", "no edit");
    assert!(status(&s).contains("multi-byte"));
}

/// Seed raw (possibly malformed) bytes via Lua \x escapes and compare
/// byte-identically Lua-side (`from_utf8_lossy` would mask differences).
fn seed_raw(s: &EditorState, lua_bytes: &str) {
    exec(
        s,
        &format!("local b = pmacs.window.buffer(); b:replace(0, b:len(), \"{lua_bytes}\")"),
    );
    exec(s, "pmacs.editor.goto_byte(0)");
}

fn raw_equals(s: &EditorState, lua_bytes: &str) -> bool {
    eval(
        s,
        &format!("local b = pmacs.window.buffer(); return b:slice(0, b:len()) == \"{lua_bytes}\""),
    )
}

#[test]
fn transpose_chars_fails_closed_on_invalid_trailing_bytes() {
    // A valid lead (C3) followed by a non-continuation byte: the
    // scalar at the cursor must be validated whole, not length-only.
    let mut s = editor_with("");
    seed_raw(&s, "a\\xC3xb");
    exec(&s, "pmacs.editor.goto_byte(1)");
    ctrl(&mut s, 't');
    assert!(raw_equals(&s, "a\\xC3xb"), "no edit on a malformed scalar");
    assert!(status(&s).contains("malformed UTF-8 at the cursor"));
}

#[test]
fn transpose_chars_fails_closed_on_invalid_scalars_behind_the_cursor() {
    // Length-consistent but scalar-invalid spans behind the cursor:
    // an overlong three-byte encoding and a beyond-U+10FFFF four-byte
    // encoding. Both scan back cleanly (lead + continuations) and
    // must still fail closed.
    let cases = [("\\xE0\\x80\\x80b", 3i64), ("\\xF4\\x90\\x80\\x80b", 4)];
    for (bytes, cursor_pos) in cases {
        let mut s = editor_with("");
        seed_raw(&s, bytes);
        exec(&s, &format!("pmacs.editor.goto_byte({cursor_pos})"));
        ctrl(&mut s, 't');
        assert!(raw_equals(&s, bytes), "no edit for {bytes}");
        assert!(
            status(&s).contains("malformed UTF-8 before the cursor"),
            "fail-closed status for {bytes}"
        );
    }
}

// ---------------------------------------------------------------------------
// Transpose-words: the nine-position Emacs 30.2 table (Q#EC5)
// ---------------------------------------------------------------------------

/// (0-based byte cursor, expected text, expected cursor) — mutating
/// rows of the framing's table ("one two three"; Emacs points are
/// these offsets + 1).
#[test]
fn transpose_words_matches_the_emacs_table() {
    let mutating: [(i64, &str, i64); 7] = [
        (0, "two one three", 7),
        (1, "two one three", 7),
        (3, "two one three", 7),
        (4, "two one three", 7),
        (5, "one three two", 13),
        (7, "one three two", 13),
        (8, "one three two", 13),
    ];
    for (pos, want, want_cursor) in mutating {
        let mut s = editor_with("one two three");
        exec(&s, &format!("pmacs.editor.goto_byte({pos})"));
        alt(&mut s, 't');
        assert_eq!(buffer_text(&s), want, "cursor byte {pos}");
        assert_eq!(cursor(&s), want_cursor, "cursor byte {pos}");
    }
    // No-successor rows: no edit, and — the named deviation from
    // Emacs — NO cursor motion.
    for pos in [10i64, 13] {
        let mut s = editor_with("one two three");
        exec(&s, &format!("pmacs.editor.goto_byte({pos})"));
        alt(&mut s, 't');
        assert_eq!(buffer_text(&s), "one two three", "cursor byte {pos}");
        assert_eq!(cursor(&s), pos, "no cursor motion on the no-op");
        assert!(status(&s).contains("no following word"));
    }
}

#[test]
fn transpose_words_preserves_separator_bytes_and_undoes_in_one_step() {
    let mut s = editor_with("aa,, bb");
    exec(&s, "pmacs.editor.goto_byte(3)");
    alt(&mut s, 't');
    assert_eq!(buffer_text(&s), "bb,, aa", "separators verbatim");
    undo(&mut s);
    assert_eq!(buffer_text(&s), "aa,, bb", "one undo step");
}

// ---------------------------------------------------------------------------
// Zap (Q#EC6)
// ---------------------------------------------------------------------------

#[test]
fn zap_to_char_kills_through_the_target_onto_the_ring() {
    let mut s = editor_with("foo(bar");
    zap(&mut s, "(");
    assert_eq!(buffer_text(&s), "bar");
    assert_eq!(cursor(&s), 0);
    assert_eq!(ring(&s), vec!["foo("]);
    let slot: String = eval(&s, "return pmacs.editor.clipboard_get()");
    assert_eq!(slot, "foo(", "clipboard mirrors the kill");
    // The boundary after a completed zap still names the zap.
    let this: String = eval(&s, "return tostring(pmacs.editor.this_command())");
    assert_eq!(this, "edit.zap-to-char");
}

#[test]
fn zap_up_to_char_leaves_the_target() {
    let mut s = editor_with("abc");
    m_x(&mut s, "edit.zap-up-to-char");
    accept_minibuffer(&mut s, "c");
    assert_eq!(buffer_text(&s), "c");
    assert_eq!(ring(&s), vec!["ab"]);
}

#[test]
fn zap_up_to_char_at_the_target_is_a_zero_length_no_op() {
    let mut s = editor_with("abc");
    m_x(&mut s, "edit.zap-up-to-char");
    accept_minibuffer(&mut s, "a");
    assert_eq!(buffer_text(&s), "abc");
    assert!(status(&s).contains("already at"));
}

#[test]
fn zap_rejects_multi_char_input_and_missing_targets() {
    let mut s = editor_with("abc");
    zap(&mut s, "xy");
    assert_eq!(buffer_text(&s), "abc");
    assert!(status(&s).contains("single character"));
    let mut s = editor_with("abc");
    zap(&mut s, "Q");
    assert_eq!(buffer_text(&s), "abc");
    assert!(status(&s).contains("no 'Q' after the cursor"));
}

#[test]
fn zap_fires_after_edit_exactly_once() {
    let mut s = editor_with("foo(bar");
    exec(
        &s,
        "EDITS = 0; pmacs.hook.add('buffer.after-edit', function() EDITS = EDITS + 1 end)",
    );
    zap(&mut s, "(");
    let edits: i64 = eval(&s, "return EDITS");
    assert_eq!(edits, 1, "the RET-dispatch wrapper fires once");
}

// ---- chain matrix ----------------------------------------------------------

#[test]
fn zap_after_a_kill_appends() {
    let mut s = editor_with("abc\nd(e");
    ctrl(&mut s, 'k'); // "abc"
    zap(&mut s, "(");
    assert_eq!(ring(&s), vec!["abc\nd("], "one appended entry");
}

#[test]
fn a_kill_after_a_zap_appends() {
    let mut s = editor_with("a(bc\nd");
    zap(&mut s, "(");
    ctrl(&mut s, 'k'); // "bc"
    assert_eq!(ring(&s), vec!["a(bc"], "one appended entry");
}

#[test]
fn consecutive_zaps_append() {
    let mut s = editor_with("a(b(c");
    zap(&mut s, "(");
    zap(&mut s, "(");
    assert_eq!(ring(&s), vec!["a(b("]);
}

#[test]
fn cancelled_zap_breaks_the_chain() {
    let mut s = editor_with("abc\nx(y");
    ctrl(&mut s, 'k');
    alt(&mut s, 'z');
    ctrl(&mut s, 'g'); // cancel the prompt
    ctrl(&mut s, 'k'); // kills "\n"
    assert_eq!(ring(&s), vec!["\n", "abc"], "two entries, no append");
}

#[test]
fn invalid_zap_input_breaks_the_chain() {
    let mut s = editor_with("abc\nx(y");
    ctrl(&mut s, 'k');
    zap(&mut s, "xy");
    ctrl(&mut s, 'k');
    assert_eq!(ring(&s), vec!["\n", "abc"]);
}

#[test]
fn no_match_zap_breaks_the_chain() {
    let mut s = editor_with("abc\nxyz");
    ctrl(&mut s, 'k');
    zap(&mut s, "Q");
    ctrl(&mut s, 'k');
    assert_eq!(ring(&s), vec!["\n", "abc"]);
}

#[test]
fn zero_length_up_to_zap_breaks_the_chain() {
    let mut s = editor_with("ab\n(cd");
    ctrl(&mut s, 'k'); // "ab"; buffer "\n(cd", cursor 0
    exec(&s, "pmacs.editor.goto_byte(1)"); // programmatic: chain intact, cursor ON '('
    m_x(&mut s, "edit.zap-up-to-char");
    accept_minibuffer(&mut s, "(");
    assert!(status(&s).contains("already at"));
    ctrl(&mut s, 'k'); // kills "(cd"
    assert_eq!(
        ring(&s),
        vec!["(cd", "ab"],
        "no append across the zero-length no-op"
    );
}

// ---- origin-guard matrix (multi-frontend) ----------------------------------

#[test]
fn accept_from_another_frontend_aborts_and_breaks_the_origin_chain() {
    let mut s = editor_with("xx\na(b");
    ctrl(&mut s, 'k'); // A kills "xx"
    alt(&mut s, 'z'); // A opens the zap prompt
    exec(&s, "pmacs.minibuffer.set_contents('(')");
    press_as(&mut s, B, KeyCode::Enter); // B completes it
    assert_eq!(buffer_text(&s), "\na(b", "no edit on either frontend");
    assert!(status(&s).contains("origin changed"));
    ctrl(&mut s, 'k'); // A's next kill: fresh
    assert_eq!(ring(&s), vec!["\n", "xx"], "A's chain was broken");
}

#[test]
fn cancel_from_another_frontend_breaks_the_origin_chain() {
    let mut s = editor_with("xx\na(b");
    ctrl(&mut s, 'k');
    alt(&mut s, 'z');
    s.dispatch_key(B, key(KeyCode::Char('g'), KeyModifiers::CONTROL)); // B cancels
    ctrl(&mut s, 'k');
    assert_eq!(ring(&s), vec!["\n", "xx"], "two entries, no append");
}

#[test]
fn pointer_click_mid_prompt_prevents_a_false_append() {
    let mut s = editor_with("xx\na(b");
    ctrl(&mut s, 'k'); // pre-zap kill: last_kill_id armed
    alt(&mut s, 'z');
    click(&mut s); // breaks the boundary; the prompt stays open
    accept_minibuffer(&mut s, "(");
    assert_eq!(buffer_text(&s), "\na(b", "no kill ran");
    assert!(status(&s).contains("origin changed"));
    // The click legitimately moved the cursor, so the next C-k's text
    // depends on the click position; the pin is that it is a FRESH
    // entry — never an append onto the pre-zap "xx".
    ctrl(&mut s, 'k');
    let r = ring(&s);
    assert_eq!(r.len(), 2, "fresh entry, no append");
    assert_eq!(r[1], "xx", "the pre-zap kill is intact");
}

#[test]
fn goto_line_ignores_an_accept_from_another_frontend() {
    let mut s = editor_with("a\nb\nc\nd");
    alt(&mut s, 'g');
    press(&mut s, KeyCode::Char('g'));
    exec(&s, "pmacs.minibuffer.set_contents('3')");
    press_as(&mut s, B, KeyCode::Enter);
    assert_eq!(cursor_line(&s), 0, "no motion");
    let back: bool = eval(&s, "return pmacs.editor.jump_back()");
    assert!(!back, "nothing on the jump stack");
}

// ---- silent-replacement matrix (the R3 blocker) -----------------------------

#[test]
fn silent_session_replacement_forces_the_next_kill_fresh() {
    let mut s = editor_with("abc\nx(y");
    ctrl(&mut s, 'k'); // "abc"
    alt(&mut s, 'z'); // zap prompt armed
    // A package replaces the session; zap's on_cancel never runs.
    exec(
        &s,
        "pmacs.minibuffer.read { prompt = 'r: ', on_accept = function() end }",
    );
    press(&mut s, KeyCode::Enter); // close the replacement
    ctrl(&mut s, 'k'); // would falsely append without the marker
    assert_eq!(
        ring(&s),
        vec!["\n", "abc"],
        "the uncommitted marker forces a fresh entry"
    );
    let cleared: bool = eval(
        &s,
        "return pmacs.killring._debug_state(pmacs.frontend.id()).pending_kill_prompt == nil",
    );
    assert!(cleared, "the marker was consumed by the fail-safe");
}

#[test]
fn second_zap_after_a_silent_replacement_does_not_append() {
    let mut s = editor_with("abc\nx(y");
    ctrl(&mut s, 'k'); // "abc"
    alt(&mut s, 'z');
    exec(
        &s,
        "pmacs.minibuffer.read { prompt = 'r: ', on_accept = function() end }",
    );
    press(&mut s, KeyCode::Enter);
    zap(&mut s, "("); // completes cleanly — kills "\nx("
    assert_eq!(
        ring(&s),
        vec!["\nx(", "abc"],
        "arm-time abandoned-marker break: fresh, not appended"
    );
}

#[test]
fn consumed_marker_aborts_the_zap_fail_closed() {
    let mut s = editor_with("a(b");
    alt(&mut s, 'z');
    let was: bool = eval(&s, "return pmacs.killring.commit_kill_prompt()");
    assert!(was, "public Lua consumed the armed marker");
    accept_minibuffer(&mut s, "(");
    assert_eq!(buffer_text(&s), "a(b", "no kill");
    assert!(status(&s).contains("prompt state consumed"));
    assert_eq!(ring(&s).len(), 0);
}

// ---- killring API validation ------------------------------------------------

#[test]
fn kill_range_validates_before_mutating() {
    let s = editor_with("abcdef");
    for bad in [
        "pmacs.killring.kill_range(-1, 2)",
        "pmacs.killring.kill_range(0, 0)",
        "pmacs.killring.kill_range(1.5, 3)",
        "pmacs.killring.kill_range(0, 99)",
        "pmacs.killring.kill_range('a', 2)",
    ] {
        let ok: bool = eval(&s, &format!("return (pcall(function() {bad} end))"));
        assert!(!ok, "{bad} must error");
    }
    assert_eq!(buffer_text(&s), "abcdef", "no mutation");
    assert_eq!(ring(&s).len(), 0, "no ring entry");
}

#[test]
fn break_chain_validates_its_frontend_argument() {
    let s = editor_with("abc");
    for bad in [
        "pmacs.killring.break_chain(-1)",
        "pmacs.killring.break_chain(1.5)",
        "pmacs.killring.break_chain('x')",
    ] {
        let ok: bool = eval(&s, &format!("return (pcall(function() {bad} end))"));
        assert!(!ok, "{bad} must error");
    }
    exec(&s, "pmacs.killring.break_chain()"); // acting frontend: fine
    exec(&s, "pmacs.killring.break_chain(0)"); // explicit: fine
}

#[test]
fn kill_range_rejected_and_transformed_leave_the_ring_alone() {
    let s = editor_with("abcdef");
    exec(
        &s,
        r#"
        REJECT = true
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if REJECT then error("nope") end
          return { kind = "delete", start = 0, ["end"] = 4 }
        end)
        "#,
    );
    let (ok, why): (bool, String) = eval(&s, "return pmacs.killring.kill_range(0, 2)");
    assert!(!ok);
    assert_eq!(why, "rejected");
    assert_eq!(buffer_text(&s), "abcdef");
    exec(&s, "REJECT = false");
    let (ok, why): (bool, String) = eval(&s, "return pmacs.killring.kill_range(0, 2)");
    assert!(!ok);
    assert_eq!(why, "transformed");
    assert_eq!(buffer_text(&s), "ef", "the intercept's delete stands");
    assert_eq!(ring(&s).len(), 0, "nothing pushed for a deviating kill");
}

// ---------------------------------------------------------------------------
// Line ops (Q#EC7)
// ---------------------------------------------------------------------------

#[test]
fn move_line_down_and_up_round_trip() {
    let mut s = editor_with("aa\nbb\ncc");
    exec(&s, "pmacs.editor.goto_byte(1)");
    alt_code(&mut s, KeyCode::Down);
    assert_eq!(buffer_text(&s), "bb\naa\ncc");
    assert_eq!(cursor(&s), 4, "cursor rides the moved line, same column");
    alt_code(&mut s, KeyCode::Up);
    assert_eq!(buffer_text(&s), "aa\nbb\ncc");
    assert_eq!(cursor(&s), 1);
}

#[test]
fn move_line_no_ops_at_the_edges() {
    let mut s = editor_with("aa\nbb");
    alt_code(&mut s, KeyCode::Up);
    assert_eq!(buffer_text(&s), "aa\nbb");
    assert!(status(&s).contains("first line"));
    exec(&s, "pmacs.editor.goto_byte(4)");
    alt_code(&mut s, KeyCode::Down);
    assert_eq!(buffer_text(&s), "aa\nbb");
    assert!(status(&s).contains("last line"));
}

#[test]
fn move_line_preserves_a_missing_trailing_newline() {
    let mut s = editor_with("aa\nbb");
    alt_code(&mut s, KeyCode::Down);
    assert_eq!(buffer_text(&s), "bb\naa", "no trailing newline appears");
    undo(&mut s);
    assert_eq!(buffer_text(&s), "aa\nbb", "one undo step");
    exec(&s, "pmacs.editor.goto_byte(4)");
    alt_code(&mut s, KeyCode::Up);
    assert_eq!(buffer_text(&s), "bb\naa");
}

#[test]
fn duplicate_line_copies_below_with_the_cursor_on_the_copy() {
    let mut s = editor_with("aa\nbb");
    exec(&s, "pmacs.editor.goto_byte(1)");
    m_x(&mut s, "edit.duplicate-line");
    assert_eq!(buffer_text(&s), "aa\naa\nbb");
    assert_eq!(cursor(&s), 4, "same column on the copy");
    // Final line without a trailing newline.
    let mut s = editor_with("aa\nbb");
    exec(&s, "pmacs.editor.goto_byte(4)");
    m_x(&mut s, "edit.duplicate-line");
    assert_eq!(buffer_text(&s), "aa\nbb\nbb", "invariant preserved");
    assert_eq!(cursor(&s), 7);
}

#[test]
fn join_line_collapses_the_junction_to_one_space() {
    let mut s = editor_with("foo  \n   bar");
    exec(&s, "pmacs.editor.goto_byte(9)");
    alt(&mut s, '^');
    assert_eq!(buffer_text(&s), "foo bar");
    assert_eq!(cursor(&s), 3, "cursor at the junction");
    undo(&mut s);
    assert_eq!(buffer_text(&s), "foo  \n   bar", "one undo step");
}

#[test]
fn join_line_uses_no_space_when_a_side_is_empty() {
    let mut s = editor_with("\nbar");
    exec(&s, "pmacs.editor.goto_byte(2)");
    alt(&mut s, '^');
    assert_eq!(buffer_text(&s), "bar", "blank previous line: no space");
    let mut s = editor_with("foo\n   ");
    exec(&s, "pmacs.editor.goto_byte(5)");
    alt(&mut s, '^');
    assert_eq!(
        buffer_text(&s),
        "foo",
        "whitespace-only current line: no space"
    );
}

#[test]
fn join_line_no_ops_on_the_first_line() {
    let mut s = editor_with("foo\nbar");
    alt(&mut s, '^');
    assert_eq!(buffer_text(&s), "foo\nbar");
    assert!(status(&s).contains("first line"));
}

// ---------------------------------------------------------------------------
// Region line ops (Q#EC8)
// ---------------------------------------------------------------------------

fn select_range(s: &EditorState, start: i64, end: i64) {
    exec(s, &format!("pmacs.editor.begin_selection({start})"));
    exec(s, &format!("pmacs.editor.goto_byte({end})"));
}

#[test]
fn sort_lines_uses_byte_order_in_any_locale() {
    let mut s = editor_with("b\nA\na\nB\n");
    select_range(&s, 0, 8);
    m_x(&mut s, "edit.sort-lines");
    assert_eq!(
        buffer_text(&s),
        "A\nB\na\nb\n",
        "explicit byte comparator: uppercase before lowercase"
    );
    assert!(region_is_nil(&s));
    assert_eq!(cursor(&s), 0);
    undo(&mut s);
    assert_eq!(buffer_text(&s), "b\nA\na\nB\n", "one undo step");
}

#[test]
fn sort_lines_excludes_a_line_when_the_region_ends_at_its_bol() {
    let mut s = editor_with("c\nb\na\n");
    select_range(&s, 0, 4); // ends exactly at the BOL of "a"
    m_x(&mut s, "edit.sort-lines");
    assert_eq!(buffer_text(&s), "b\nc\na\n", "the third line is untouched");
}

#[test]
fn sort_lines_preserves_a_missing_final_newline() {
    let mut s = editor_with("b\na");
    select_range(&s, 0, 3);
    m_x(&mut s, "edit.sort-lines");
    assert_eq!(buffer_text(&s), "a\nb");
}

#[test]
fn reverse_and_dedupe_lines() {
    let mut s = editor_with("a\nb\nc\n");
    select_range(&s, 0, 6);
    m_x(&mut s, "edit.reverse-lines");
    assert_eq!(buffer_text(&s), "c\nb\na\n");
    let mut s = editor_with("x\ny\nx\nz\n");
    select_range(&s, 0, 8);
    m_x(&mut s, "edit.delete-duplicate-lines");
    assert_eq!(buffer_text(&s), "x\ny\nz\n", "first occurrence kept");
    assert!(status(&s).contains("1 removed"));
}

#[test]
fn region_ops_require_a_region() {
    let mut s = editor_with("b\na\n");
    m_x(&mut s, "edit.sort-lines");
    assert_eq!(buffer_text(&s), "b\na\n");
    assert!(status(&s).contains("no active region"));
}

// ---------------------------------------------------------------------------
// Intercept discipline (Q#EC2)
// ---------------------------------------------------------------------------

#[test]
fn rejecting_intercept_leaves_everything_alone() {
    let mut s = editor_with("abc");
    exec(
        &s,
        "pmacs.buffer.add_intercept(pmacs.window.buffer(), function() error('ro') end)",
    );
    exec(&s, "pmacs.editor.goto_byte(1)");
    ctrl(&mut s, 't');
    assert_eq!(buffer_text(&s), "abc");
    assert_eq!(cursor(&s), 1, "no cursor motion");
    assert!(status(&s).contains("rejected by buffer intercept"));
}

#[test]
fn transforming_intercept_translates_and_clamps_the_cursor() {
    // Expanding replace that shrinks the buffer below the old cursor:
    // dedupe of three identical lines, with the intercept widening the
    // replace to the whole buffer. Unrepaired, the cursor (9) would
    // strand past the new length (3).
    let mut s = editor_with("aa\naa\naa\nbb");
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "replace" then
            local b = pmacs.window.buffer()
            return { kind = "replace", start = 0, ["end"] = b:len() }
          end
        end)
        "#,
    );
    select_range(&s, 0, 9);
    m_x(&mut s, "edit.delete-duplicate-lines");
    assert!(status(&s).contains("altered by buffer intercept"));
    assert_eq!(buffer_text(&s), "aa\n", "the intercept's replace stands");
    assert_eq!(cursor(&s), 3, "translated and clamped, never past len");
    assert!(region_is_nil(&s), "stale selection cleared");
}

#[test]
fn context_switching_intercept_skips_all_fixup() {
    let mut s = editor_with("abc");
    exec(
        &s,
        r#"
        OTHER = pmacs.buffer.create("*editops-other*")
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          pmacs.window.switch_buffer(OTHER)
        end)
        "#,
    );
    exec(&s, "pmacs.editor.goto_byte(1)");
    ctrl(&mut s, 't');
    assert!(status(&s).contains("context changed"));
    let name: String = eval(
        &s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    );
    assert_eq!(name, "*editops-other*", "the switch stands");
    assert_eq!(cursor(&s), 0, "the switched-to context is untouched");
    let other_len: i64 = eval(&s, "return OTHER:len()");
    assert_eq!(other_len, 0);
}

#[test]
fn zero_length_anchor_does_not_reactivate_as_a_selection() {
    // The command must MOVE the cursor for this pin to be non-vacuous:
    // mid-word upcase travels to the word end.
    let mut s = editor_with("foo bar");
    exec(&s, "pmacs.editor.begin_selection(0)");
    alt(&mut s, 'u');
    assert_eq!(buffer_text(&s), "FOO bar");
    assert_eq!(cursor(&s), 3);
    assert!(
        region_is_nil(&s),
        "the dormant anchor must not span the cursor's travel"
    );
}

// ---------------------------------------------------------------------------
// delete-trailing-whitespace (Q#EC9)
// ---------------------------------------------------------------------------

#[test]
fn trim_command_trims_and_translates_the_cursor() {
    let mut s = editor_with("ab   \ncd\t\t\nef");
    exec(&s, "pmacs.editor.goto_byte(3)"); // inside the first run
    m_x(&mut s, "edit.delete-trailing-whitespace");
    assert_eq!(buffer_text(&s), "ab\ncd\nef");
    assert_eq!(cursor(&s), 2, "inside a trimmed run: its start");
    assert!(status(&s).contains("trimmed 2 lines"));
}

#[test]
fn trim_shifts_a_cursor_after_the_runs() {
    let mut s = editor_with("ab   \ncd");
    exec(&s, "pmacs.editor.goto_byte(6)"); // 'c'
    m_x(&mut s, "edit.delete-trailing-whitespace");
    assert_eq!(buffer_text(&s), "ab\ncd");
    assert_eq!(cursor(&s), 3, "shifted left through the deleted run");
}

#[test]
fn trim_undo_grain_is_one_step_per_line() {
    let mut s = editor_with("a \nb \n");
    m_x(&mut s, "edit.delete-trailing-whitespace");
    assert_eq!(buffer_text(&s), "a\nb\n");
    // Applied bottom-up, so line 1's run was deleted LAST and is
    // restored by the FIRST undo: one step per trimmed line.
    undo(&mut s);
    assert_eq!(buffer_text(&s), "a \nb\n");
    undo(&mut s);
    assert_eq!(buffer_text(&s), "a \nb \n");
}

#[test]
fn trim_partial_sweep_stops_and_still_translates() {
    let mut s = editor_with("a \nb \nc");
    exec(&s, "pmacs.editor.goto_byte(6)"); // 'c'
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "delete" and op.start < 3 then error("no") end
        end)
        "#,
    );
    m_x(&mut s, "edit.delete-trailing-whitespace");
    assert_eq!(
        buffer_text(&s),
        "a \nb\nc",
        "line 2 trimmed, line 1 rejected"
    );
    assert!(status(&s).contains("line 1 rejected"));
    assert_eq!(cursor(&s), 5, "translated through the one landed delete");
}

#[test]
fn trim_mid_sweep_context_switch_stops_the_sweep() {
    let mut s = editor_with("a \nb \nc");
    exec(
        &s,
        r#"
        OTHER = pmacs.buffer.create("*trim-other*")
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "delete" then pmacs.window.switch_buffer(OTHER) end
        end)
        "#,
    );
    m_x(&mut s, "edit.delete-trailing-whitespace");
    assert!(status(&s).contains("context changed"));
    let name: String = eval(
        &s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    );
    assert_eq!(name, "*trim-other*");
    assert_eq!(cursor(&s), 0, "no fix-up in the switched-to context");
    // The bottom-most run was deleted (cleanly) before the guard
    // tripped; the earlier line's run must be untouched.
    let orig: String = eval(
        &s,
        "return (function() local b for _, id in ipairs(pmacs.buffer.list()) do local d = pmacs.describe.buffer(id) if d.name ~= '*trim-other*' then b = id end end return b:slice(0, b:len()) end)()",
    );
    assert_eq!(orig, "a \nb\nc", "sweep stopped at the switch");
}

// ---- trim-on-save -----------------------------------------------------------

fn temp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("pmacs-editops-{}-{}", std::process::id(), name));
    p
}

#[test]
fn trim_on_save_defaults_off_and_writes_untouched_bytes() {
    let path = temp_path("off.txt");
    std::fs::write(&path, "x  \ny\t\n").unwrap();
    let mut s = EditorState::new_with_roots(&crate::iso::roots());
    let on: bool = eval(&s, "return pmacs.editops.trim_on_save()");
    assert!(!on, "default off");
    exec(
        &s,
        &format!(
            "pmacs.buffer.from_file({})",
            lua_str(path.to_str().unwrap())
        ),
    );
    ctrl(&mut s, 'x');
    ctrl(&mut s, 's');
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x  \ny\t\n");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn trim_on_save_trims_the_written_bytes_before_later_callbacks() {
    let path = temp_path("on.txt");
    std::fs::write(&path, "x  \ny\t\n").unwrap();
    let mut s = EditorState::new_with_roots(&crate::iso::roots());
    exec(&s, "pmacs.editops.trim_on_save(true)");
    // A callback registered AFTER editops' (load-time) hook observes
    // the fan-out order saveplace sees: post-trim text.
    exec(
        &s,
        r#"
        SEEN = nil
        pmacs.hook.add("buffer.before-save", function()
          local b = pmacs.window.buffer()
          SEEN = b:slice(0, b:len())
        end)
        "#,
    );
    exec(
        &s,
        &format!(
            "pmacs.buffer.from_file({})",
            lua_str(path.to_str().unwrap())
        ),
    );
    ctrl(&mut s, 'x');
    ctrl(&mut s, 's');
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "x\ny\n",
        "disk trimmed"
    );
    let seen: String = eval(&s, "return SEEN");
    assert_eq!(
        seen, "x\ny\n",
        "later before-save callbacks see post-trim text"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn trim_on_save_failure_never_vetoes_the_save() {
    let path = temp_path("veto-immune.txt");
    std::fs::write(&path, "x  \n").unwrap();
    let mut s = EditorState::new_with_roots(&crate::iso::roots());
    exec(&s, "pmacs.editops.trim_on_save(true)");
    exec(
        &s,
        &format!(
            "pmacs.buffer.from_file({})",
            lua_str(path.to_str().unwrap())
        ),
    );
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "delete" then error("ro") end
        end)
        "#,
    );
    ctrl(&mut s, 'x');
    ctrl(&mut s, 's');
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "x  \n",
        "the save proceeded (with the untrimmed bytes)"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn trim_on_save_unexpected_error_reports_and_still_saves() {
    let path = temp_path("unexpected.txt");
    std::fs::write(&path, "x  \n").unwrap();
    let mut s = EditorState::new_with_roots(&crate::iso::roots());
    exec(&s, "pmacs.editops.trim_on_save(true)");
    // Capture the pmacs.error log (the m9_6 stub pattern — the
    // `if pmacs.error` branch is a no-op without it).
    exec(
        &s,
        r"
        PMACS_ERROR_LOG = {}
        pmacs.error = function(msg)
            PMACS_ERROR_LOG[#PMACS_ERROR_LOG + 1] = msg
        end
        ",
    );
    exec(
        &s,
        &format!(
            "pmacs.buffer.from_file({})",
            lua_str(path.to_str().unwrap())
        ),
    );
    // Force an error INSIDE trim, past the per-edit pcalls: trim's
    // context snapshot reads pmacs.window.current(), which nothing
    // else on the save path touches.
    exec(
        &s,
        "TRIM_ORIG_WC = pmacs.window.current; pmacs.window.current = function() error('boom') end",
    );
    ctrl(&mut s, 'x');
    ctrl(&mut s, 's');
    exec(&s, "pmacs.window.current = TRIM_ORIG_WC");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "x  \n",
        "the save proceeded (trim aborted before any edit)"
    );
    let logged: String = eval(&s, "return PMACS_ERROR_LOG[1] or ''");
    assert!(
        logged.contains("delete-trailing-whitespace (on save) failed:"),
        "the unexpected error reached the pmacs.error log, got: {logged}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn another_callbacks_veto_is_not_masked_by_trim() {
    let path = temp_path("veto.txt");
    std::fs::write(&path, "x  \n").unwrap();
    let mut s = EditorState::new_with_roots(&crate::iso::roots());
    exec(&s, "pmacs.editops.trim_on_save(true)");
    exec(
        &s,
        &format!(
            "pmacs.buffer.from_file({})",
            lua_str(path.to_str().unwrap())
        ),
    );
    exec(
        &s,
        "pmacs.hook.add('buffer.before-save', function() return false end)",
    );
    ctrl(&mut s, 'x');
    ctrl(&mut s, 's');
    assert!(status(&s).contains("vetoed"));
    // A wrongly-unvetoed save would have written the TRIMMED bytes
    // (trim ran before the veto and edited the buffer).
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "x  \n",
        "the veto still blocked the write"
    );
    let _ = std::fs::remove_file(&path);
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
