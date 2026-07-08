//! Query-replace acceptance (Arc 2) — the interactive phase end-to-end
//! through `dispatch_key`, exactly as a user (or a round-tripping GPU)
//! drives it: the `M-%` / `C-M-%` bindings; the full key vocabulary
//! (`y`/`SPC` replace, `n`/`DEL` skip, `!` all, `.` last); every quit
//! path (`q`, `RET`, `Esc`, `C-g`) keeping replacements, plus
//! nothing-matched-restores-origin; empty-to deletion; offset-shift
//! correctness (`a`→`aa` doesn't loop); regex; the `buffer.after-edit`
//! hook (once per `y`, and exactly once for an `!` batch); the
//! `dispatch_idle`-false gate; and the wrong-buffer/focus-drift abort.
//!
//! Framing: docs/query-replace-framing.md.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        press(s, KeyCode::Char(ch));
    }
}

/// Move the active cursor to buffer start (no default binding for it;
/// walk up then to line start, like lsp.lua's cursor-move).
fn goto_start(s: &EditorState) {
    s.lua_host
        .lua()
        .load(
            r"
            while pmacs.editor.cursor_line() > 0 do pmacs.editor.move_up() end
            pmacs.editor.move_line_start()
            ",
        )
        .exec()
        .expect("move cursor to buffer start");
}

/// `(buffer text, active?, query-replace active?)` through the Lua
/// surface.
fn probe(s: &EditorState) -> (String, bool) {
    s.lua_host
        .lua()
        .load(
            r"
            local b = pmacs.window.buffer()
            return b:slice(0, b:len()), pmacs.editor.query_replace_active()
            ",
        )
        .eval()
        .expect("probe query-replace state")
}

/// Drive the two minibuffer prompts a `query-replace` command opens:
/// type `from`, RET, type `to`, RET. Leaves the session in its
/// interactive phase (or finished, if `!`/no-match).
fn start_query_replace(s: &mut EditorState, from: &str, to: &str, regex: bool) {
    let cmd = if regex {
        "query-replace-regexp"
    } else {
        "query-replace"
    };
    s.lua_host
        .lua()
        .load(format!("pmacs.command.invoke('{cmd}')"))
        .exec()
        .expect("invoke query-replace command");
    type_str(s, from);
    press(s, KeyCode::Enter);
    type_str(s, to);
    press(s, KeyCode::Enter);
}

#[test]
fn replace_skip_and_quit_is_selective() {
    let mut s = EditorState::new();
    type_str(&mut s, "x x x x");
    // Cursor to buffer start so all four are ahead of point.
    goto_start(&s);
    start_query_replace(&mut s, "x", "y", false);
    let (_, active) = probe(&s);
    assert!(
        active,
        "session is in its interactive phase on the first match"
    );

    press(&mut s, KeyCode::Char('y')); // replace 1st
    press(&mut s, KeyCode::Char('n')); // skip 2nd
    press(&mut s, KeyCode::Char('y')); // replace 3rd
    press(&mut s, KeyCode::Char('q')); // quit before the 4th
    let (text, active) = probe(&s);
    assert_eq!(text, "y x y x", "y/n/y then quit");
    assert!(!active, "q ends the session");
}

#[test]
fn bang_replaces_all_remaining() {
    let mut s = EditorState::new();
    type_str(&mut s, "a a a a");
    goto_start(&s);
    start_query_replace(&mut s, "a", "b", false);
    press(&mut s, KeyCode::Char('!'));
    let (text, active) = probe(&s);
    assert_eq!(text, "b b b b");
    assert!(!active, "! finishes the session");
}

#[test]
fn dot_replaces_current_then_quits() {
    let mut s = EditorState::new();
    type_str(&mut s, "a a a");
    goto_start(&s);
    start_query_replace(&mut s, "a", "z", false);
    press(&mut s, KeyCode::Char('.')); // replace first, then quit
    let (text, active) = probe(&s);
    assert_eq!(text, "z a a", "only the first is replaced");
    assert!(!active);
}

#[test]
fn growing_replacement_does_not_loop() {
    // a → aa must not re-match the inserted text (offset-shift + the
    // search-forward-past-replacement rule).
    let mut s = EditorState::new();
    type_str(&mut s, "a a a");
    goto_start(&s);
    start_query_replace(&mut s, "a", "aa", false);
    press(&mut s, KeyCode::Char('!'));
    let (text, _) = probe(&s);
    assert_eq!(text, "aa aa aa", "each 'a' replaced exactly once");
}

#[test]
fn empty_to_deletes() {
    let mut s = EditorState::new();
    type_str(&mut s, "a-b-c");
    goto_start(&s);
    start_query_replace(&mut s, "-", "", false);
    press(&mut s, KeyCode::Char('!'));
    let (text, _) = probe(&s);
    assert_eq!(text, "abc", "empty replacement deletes matches");
}

#[test]
fn regex_query_replace_via_binding() {
    let mut s = EditorState::new();
    type_str(&mut s, "a1 b2 c3");
    goto_start(&s);
    start_query_replace(&mut s, "[0-9]", "#", true);
    press(&mut s, KeyCode::Char('!'));
    let (text, _) = probe(&s);
    assert_eq!(text, "a# b# c#");
}

#[test]
fn m_percent_binding_starts_query_replace() {
    // The literal chord: M-% (Alt + Shift+5 → Char('%') with ALT).
    let mut s = EditorState::new();
    type_str(&mut s, "cat cat");
    goto_start(&s);
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('%'), KeyModifiers::ALT),
    );
    // The from-prompt minibuffer should now be active.
    let mb: bool = s
        .lua_host
        .lua()
        .load("return pmacs.minibuffer.is_active and pmacs.minibuffer.is_active() or false")
        .eval()
        .unwrap_or(false);
    assert!(mb, "M-% opened the query-replace from-prompt");
    // Complete the flow and confirm it replaces.
    type_str(&mut s, "cat");
    press(&mut s, KeyCode::Enter);
    type_str(&mut s, "dog");
    press(&mut s, KeyCode::Enter);
    press(&mut s, KeyCode::Char('!'));
    let (text, _) = probe(&s);
    assert_eq!(text, "dog dog", "M-% drove a full query-replace");
}

#[test]
fn c_m_percent_binding_starts_regexp_query_replace() {
    // Control-meta-shifted punctuation — the chord most likely to parse
    // differently across key paths (the C-c H lesson).
    let mut s = EditorState::new();
    type_str(&mut s, "x1 x2");
    goto_start(&s);
    s.dispatch_key(
        FrontendId::LOCAL,
        key(
            KeyCode::Char('%'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ),
    );
    let mb: bool = s
        .lua_host
        .lua()
        .load("return pmacs.minibuffer.is_active and pmacs.minibuffer.is_active() or false")
        .eval()
        .unwrap_or(false);
    assert!(mb, "C-M-% opened the query-replace-regexp from-prompt");
    type_str(&mut s, "x[0-9]");
    press(&mut s, KeyCode::Enter);
    type_str(&mut s, "Q");
    press(&mut s, KeyCode::Enter);
    press(&mut s, KeyCode::Char('!'));
    let (text, _) = probe(&s);
    assert_eq!(text, "Q Q", "C-M-% drove a regexp query-replace");
}

#[test]
fn nothing_matched_leaves_buffer_untouched() {
    let mut s = EditorState::new();
    type_str(&mut s, "hello");
    start_query_replace(&mut s, "zzz", "q", false);
    let (text, active) = probe(&s);
    assert_eq!(text, "hello", "no match → buffer untouched");
    assert!(!active, "no match → session never stays open");
}

#[test]
fn query_replace_flips_dispatch_idle_so_gpu_round_trips() {
    // While the interactive phase runs, dispatch_idle must be false so a
    // semantic frontend round-trips y/n/etc. instead of self-inserting.
    let mut s = EditorState::new();
    type_str(&mut s, "a a");
    goto_start(&s);
    start_query_replace(&mut s, "a", "b", false);
    assert!(
        !s.dispatch_idle(),
        "query-replace interactive phase forces key round-trip"
    );
    press(&mut s, KeyCode::Char('!'));
    assert!(s.dispatch_idle(), "idle again after the session finishes");
}

#[test]
fn replace_fires_after_edit_hook() {
    // The Q#QR1 hook: an LSP/syntax observer must see replaced text.
    let mut s = EditorState::new();
    s.lua_host
        .lua()
        .load(
            r"
            _G.EDITS = 0
            pmacs.hook.add('buffer.after-edit', function() _G.EDITS = _G.EDITS + 1 end)
            ",
        )
        .exec()
        .expect("install after-edit counter");
    type_str(&mut s, "a a a");
    goto_start(&s);
    let before: i64 = s
        .lua_host
        .lua()
        .load("return _G.EDITS")
        .eval()
        .expect("read counter");
    start_query_replace(&mut s, "a", "b", false);
    press(&mut s, KeyCode::Char('y')); // one replacement
    let after: i64 = s
        .lua_host
        .lua()
        .load("return _G.EDITS")
        .eval()
        .expect("read counter");
    assert!(
        after > before,
        "buffer.after-edit fired for the replacement (before {before}, after {after})"
    );
}

#[test]
fn bang_fires_after_edit_hook_once_for_the_batch() {
    // Q#QR1: `!` applies many replacements under one keypress, but the
    // debounced didChange wants a single after-edit — the shadow
    // compares revision once across the whole handler.
    let mut s = EditorState::new();
    s.lua_host
        .lua()
        .load(
            r"
            _G.EDITS = 0
            pmacs.hook.add('buffer.after-edit', function() _G.EDITS = _G.EDITS + 1 end)
            ",
        )
        .exec()
        .expect("install after-edit counter");
    type_str(&mut s, "a a a a");
    goto_start(&s);
    // Zero out the counter after the typing edits.
    s.lua_host.lua().load("_G.EDITS = 0").exec().ok();
    start_query_replace(&mut s, "a", "b", false);
    s.lua_host.lua().load("_G.EDITS = 0").exec().ok();
    press(&mut s, KeyCode::Char('!')); // four replacements in one keypress
    let edits: i64 = s
        .lua_host
        .lua()
        .load("return _G.EDITS")
        .eval()
        .expect("read counter");
    let (text, _) = probe(&s);
    assert_eq!(text, "b b b b", "! replaced all four");
    assert_eq!(edits, 1, "after-edit fires exactly once for the ! batch");
}

#[test]
fn quit_via_ret_and_esc_keeps_replacements() {
    // Q#QR10: RET and Esc both quit (keeping replacements), not just q.
    for quit in [KeyCode::Enter, KeyCode::Esc] {
        let mut s = EditorState::new();
        type_str(&mut s, "a a a");
        goto_start(&s);
        start_query_replace(&mut s, "a", "b", false);
        press(&mut s, KeyCode::Char('y')); // replace the first
        press(&mut s, quit); // quit before the rest
        let (text, active) = probe(&s);
        assert_eq!(text, "b a a", "quit keeps the one replacement ({quit:?})");
        assert!(!active, "{quit:?} ends the session");
    }
}

#[test]
fn ctrl_g_quits_keeping_replacements() {
    // Q#QR10: C-g exits and KEEPS replacements (unlike isearch's C-g).
    let mut s = EditorState::new();
    type_str(&mut s, "a a a");
    goto_start(&s);
    start_query_replace(&mut s, "a", "b", false);
    press(&mut s, KeyCode::Char('y'));
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('g'), KeyModifiers::CONTROL),
    );
    let (text, active) = probe(&s);
    assert_eq!(text, "b a a", "C-g keeps replacements (not an undo)");
    assert!(!active);
}

#[test]
fn del_key_skips_like_n() {
    let mut s = EditorState::new();
    type_str(&mut s, "a a a");
    goto_start(&s);
    start_query_replace(&mut s, "a", "b", false);
    press(&mut s, KeyCode::Backspace); // DEL/Backspace → skip first
    press(&mut s, KeyCode::Char('y')); // replace second
    press(&mut s, KeyCode::Char('q'));
    let (text, _) = probe(&s);
    assert_eq!(
        text, "a b a",
        "DEL skipped the first, y replaced the second"
    );
}

#[test]
fn focus_drift_mid_session_aborts_without_touching_either_buffer() {
    // The merge-blocker, end-to-end: a click into another buffer
    // (simulated by switch_buffer, which the pointer path also uses)
    // while query-replace is active. The next y must abort, not apply
    // the origin-buffer match to the now-active unrelated buffer.
    let mut s = EditorState::new();
    type_str(&mut s, "foo foo");
    goto_start(&s);
    start_query_replace(&mut s, "foo", "bar", false);
    assert!(probe(&s).1, "session active on the first match");

    // Focus drifts to a fresh, unrelated buffer.
    s.lua_host
        .lua()
        .load(
            r"
            _G.OTHER = pmacs.buffer.create('*drift*')
            pmacs.window.switch_buffer(_G.OTHER)
            ",
        )
        .exec()
        .expect("switch to another buffer");

    press(&mut s, KeyCode::Char('y')); // the replace key, now drifted
    assert!(!probe(&s).1, "drift aborts the session");
    let (drift_text, _) = probe(&s); // active buffer is *drift*
    assert_eq!(drift_text, "", "the unrelated buffer was not edited");

    // The origin buffer is also intact — switch back and check.
    s.lua_host
        .lua()
        .load(
            r"
            for _, id in ipairs(pmacs.buffer.list()) do
              if pmacs.describe.buffer(id).name == '*scratch*' then
                pmacs.window.switch_buffer(id)
              end
            end
            ",
        )
        .exec()
        .ok();
    assert_eq!(
        probe(&s).0,
        "foo foo",
        "origin buffer untouched by the aborted replace"
    );
}
