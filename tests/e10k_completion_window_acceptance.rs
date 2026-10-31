//! E10k.2 --- completion reads a window of the buffer, not the buffer
//! (#413).
//!
//! Before E10k.2 every key that collected completions copied the whole
//! buffer into Lua (`completion.lua`'s `buf:slice(0, buf:len())`), again
//! into the context, and dabbrev tokenized and lowercased all of it. Now
//! the context reads the buffer by range, dabbrev reads at most
//! `completion.dabbrev-window` bytes either side of the cursor, nearest
//! words first, and the whole text is copied only for a registered Lua
//! provider, whose fourth argument it is. Every row types through
//! `dispatch_key`, as a terminal user does, into an in-process editor.

#[path = "common/iso.rs"]
mod iso;

#[path = "common/editor.rs"]
mod editor;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use editor::{eval, exec};
use pmacs::completion_framework::DABBREV_WINDOW_DEFAULT;
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(ch)));
    }
}

/// An editor whose scratch buffer holds `lines` log lines of 70 bytes
/// and then `tail`, the caret at the end.
fn editor_with(lines: usize, tail: &str) -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    exec(
        &s,
        &format!(
            "local b = pmacs.window.buffer()
             local line = '2026-10-27T12:00:00.000Z INFO daemon: served request 0000000 in 000us\\n'
             b:insert(0, string.rep(line, {lines}) .. {tail:?})
             pmacs.editor.goto_byte(b:len())"
        ),
    );
    s
}

fn buffer_len(s: &EditorState) -> usize {
    eval(s, "return pmacs.window.buffer():len()")
}

/// E10k.2's witness: one key in an 8 MiB buffer reads at most twice the
/// window, which is `completion.dabbrev-window`'s default, and
/// `completion.lua` hands `collect` the buffer and the cursor, not a
/// copy of the text. At the base, `CompletionRegistry` counted nothing:
/// the row is run against the base's whole-buffer dabbrev by planting
/// it, as the handoff records.
#[test]
fn e10k_2_one_key_reads_at_most_twice_the_window_of_an_8_mib_buffer() {
    // 119,838 lines of 70 bytes: just past 8 MiB, ending in the word `zq`.
    let mut s = editor_with(119_838, "zq");
    assert!(buffer_len(&s) >= 8 << 20, "the fixture is 8 MiB");
    let window: usize = eval(&s, "return pmacs.config.get('completion.dabbrev-window')");
    assert_eq!(
        window, DABBREV_WINDOW_DEFAULT,
        "completion.lua's default is the provider's"
    );
    // What `collect` is handed, recorded on the way through.
    exec(
        &s,
        "local collect = pmacs.completion.collect
         _G.E10K_CTX = {}
         pmacs.completion.collect = function(ctx)
           _G.E10K_CTX[#_G.E10K_CTX + 1] = {
             text = ctx.buffer_text ~= nil,
             buffer = ctx.buffer ~= nil,
             cursor = ctx.cursor,
           }
           return collect(ctx)
         end",
    );
    // The first key records where the caret was; the second extends the
    // word by one byte, so completion collects for `zqkx`.
    type_str(&mut s, "kx");
    let collects: usize = eval(&s, "return #_G.E10K_CTX");
    assert_eq!(collects, 1, "the second key collected, the first did not");
    let (text, buffer, cursor): (bool, bool, Option<usize>) = eval(
        &s,
        "local c = _G.E10K_CTX[1] return c.text, c.buffer, c.cursor",
    );
    assert!(!text, "completion.lua passes no buffer_text");
    assert!(buffer, "completion.lua passes the buffer");
    assert_eq!(cursor, Some(buffer_len(&s)), "and the cursor, at the end");
    let read = s.completion_registry.borrow().last_read_bytes();
    assert!(read > 0, "dabbrev read the buffer: the control");
    assert!(
        read <= 2 * window,
        "one key read {read} bytes of {}; the window is {window}",
        buffer_len(&s)
    );
}

/// E10k.2: the nearer of two words with the prefix is offered first,
/// so TAB takes it. `zqaa` stands near the top of the buffer and `zqzz`
/// on the line above the caret; by label `zqaa` would come first, as it
/// did at the base.
#[test]
fn e10k_2_the_nearer_of_two_words_is_offered_first() {
    let mut s = EditorState::new_with_roots(&iso::roots());
    exec(
        &s,
        "local b = pmacs.window.buffer()
         b:insert(0, 'zqaa\\n' .. string.rep('filler words here\\n', 100) .. 'zqzz\\n')
         pmacs.editor.goto_byte(b:len())",
    );
    type_str(&mut s, "zq");
    let visible: bool = eval(&s, "return pmacs.completion.popup_visible()");
    assert!(visible, "`zq` opens the popup off dabbrev");
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Tab));
    let text: String = eval(
        &s,
        "local b = pmacs.window.buffer() return b:slice(0, b:len())",
    );
    assert!(
        text.ends_with("zqzz\nzqzz"),
        "TAB took the nearer word: ...{:?}",
        &text[text.len() - 12..]
    );
}

/// E10k.2: a registered Lua provider still receives the whole text as
/// its fourth argument, and that request reads it.
#[test]
fn e10k_2_a_lua_provider_still_receives_the_whole_text() {
    // 2 MiB: past the window either side, so the whole text is more than
    // dabbrev alone would read.
    let mut s = editor_with(30_000, "zq");
    exec(
        &s,
        "_G.E10K_SEEN = nil
         pmacs.completion.register({
           name = 'e10k_whole',
           priority = 5,
           fn = function(prefix, line, col, buffer_text)
             _G.E10K_SEEN = #buffer_text
             return {}
           end,
         })",
    );
    type_str(&mut s, "kx");
    let seen: Option<usize> = eval(&s, "return _G.E10K_SEEN");
    let len = buffer_len(&s);
    assert_eq!(seen, Some(len), "the Lua provider saw the whole buffer");
    let read = s.completion_registry.borrow().last_read_bytes();
    assert!(
        read >= len,
        "that request read the whole text: {read} of {len}"
    );
}
