// tests/e7b_tab_acceptance.rs --- E7b.4, TAB completes when there is
// nothing to indent.

//! Emacs's `tab-always-indent` set to `complete`: TAB indents the line,
//! and when the line is already at the indentation TAB would produce,
//! TAB completes at point instead. The indent function is `indent.lua`'s
//! own --- the nearest non-blank line above, copied byte for byte ---
//! and knows nothing of languages; E1.6's language-aware clause is not
//! this row's and stays the owner's.
//!
//! What this settles: the TAB key's behavior on both frontends (the key
//! round-trips to the daemon from the GPU, so one keymap entry decides
//! it). What it does not: the minibuffer's TAB (E6.3), which is the
//! prompt's hardcoded `Complete` and never reaches the keymap --- one
//! witness below says so; and TAB with an active region, which keeps
//! the CUA type-over it had, since indenting a region needs the engine
//! E1.6 owns.
//!
//! Every witness drives `dispatch_key` with a plain TAB, the seam both
//! frontends reach the daemon by.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use std::path::Path;
use std::time::{Duration, Instant};

#[path = "common/iso.rs"]
mod iso;

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn tab(s: &mut EditorState) {
    press(s, KeyCode::Tab);
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn text(s: &EditorState) -> String {
    eval(
        s,
        "local b = pmacs.window.buffer() return b:slice(0, b:len())",
    )
}

fn cursor(s: &EditorState) -> usize {
    eval(s, "return pmacs.editor.cursor()")
}

fn goto(s: &EditorState, byte: usize) {
    exec(s, &format!("pmacs.editor.goto_byte({byte})"));
}

fn popup_visible(s: &EditorState) -> bool {
    eval(s, "return pmacs.completion.popup_visible()")
}

fn tick(s: &mut EditorState) {
    s.tick_processes();
    s.tick_lsp();
    s.tick_async();
}

fn pump_until(s: &mut EditorState, ms: u64, mut done: impl FnMut(&EditorState) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(ms);
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

/// A scratch editor with no server anywhere, holding `body`.
fn editor_with(body: &str) -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    exec(&s, "pmacs.lsp.config = {}");
    exec(
        &s,
        &format!("local b = pmacs.window.buffer() b:replace(0, b:len(), {body:?})"),
    );
    s
}

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

const INITIALIZED: &str = "(function() \
   for _,r in ipairs(pmacs.lsp.list()) do \
     if r.state and r.state.kind=='initialized' then return true end \
   end \
   return false \
 end)()";

/// An editor visiting `dir/a.rs` holding `body`, the rust server
/// pointed at the fake, which answers every completion with items.
fn rust_editor(dir: &Path, body: &str) -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.to_path_buf()));
    exec(&s, "pmacs.lsp.config = {}");
    let fake = fake_lsp_path();
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust = {{ command = '{fake}', env = {{ PMACS_FAKE_LSP_MODE = 'e7b' }} }}"
        ),
    );
    std::fs::write(dir.join("Cargo.toml"), b"[package]\nname=\"x\"\n").unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, body).unwrap();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    let mut s = s;
    assert!(
        pump_until(&mut s, 10_000, |s| eval::<bool>(
            s,
            &format!("return {INITIALIZED}")
        )),
        "fake server init"
    );
    // The Lua drain attaches the buffer on the `initialized` event a
    // tick or more after the layer reports it; a completion asked
    // before that is refused and its session dies, so let the drain
    // catch up as a user's hands would.
    let settle = Instant::now() + Duration::from_millis(250);
    while Instant::now() < settle {
        tick(&mut s);
        std::thread::sleep(Duration::from_millis(2));
    }
    s
}

// ---------------------------------------------------------------------------
// Indenting
// ---------------------------------------------------------------------------

/// A line under an indented line, itself unindented: TAB gives it the
/// line above's indentation, byte for byte (tabs stay tabs), and puts
/// point after it. Bitten by rebinding TAB to `buffer.tab`: a tab
/// character is inserted at point instead.
#[test]
fn e7b_4_tab_indents_the_line_to_the_line_above() {
    let mut s = editor_with("    first\nsecond\n");
    goto(&s, 10); // start of `second`
    tab(&mut s);
    assert_eq!(text(&s), "    first\n    second\n");
    assert_eq!(cursor(&s), 14, "point sits after the new indentation");

    let mut s = editor_with("\tfirst\nsecond\n");
    goto(&s, 7);
    tab(&mut s);
    assert_eq!(text(&s), "\tfirst\n\tsecond\n", "a tab is copied as a tab");
    assert_eq!(cursor(&s), 8);
}

/// Too much indentation is taken away, and point past the indentation
/// keeps its distance from the text.
#[test]
fn e7b_4_tab_removes_extra_indentation_and_keeps_point_in_the_text() {
    let mut s = editor_with("  first\n        sec|ond\n");
    // Point on the `|` (byte 19): 8 + 8 spaces + "sec".
    goto(&s, 19);
    tab(&mut s);
    assert_eq!(text(&s), "  first\n  sec|ond\n");
    assert_eq!(cursor(&s), 13, "point still before the |");
}

/// The line above being blank, the nearest non-blank line above decides;
/// at the top of the buffer the indentation is nothing.
#[test]
fn e7b_4_the_nearest_non_blank_line_above_decides() {
    let mut s = editor_with("    first\n\n\nsecond\n");
    goto(&s, 12);
    tab(&mut s);
    assert_eq!(text(&s), "    first\n\n\n    second\n");

    let mut s = editor_with("    top\n");
    goto(&s, 4);
    tab(&mut s);
    assert_eq!(text(&s), "top\n", "the first line has nothing to copy");
    assert_eq!(cursor(&s), 0);
}

/// A line already at its indentation with point inside it: TAB moves
/// point to the indentation and changes nothing else --- Emacs's rule,
/// which completes only when neither the buffer nor point moved.
#[test]
fn e7b_4_tab_inside_the_right_indentation_moves_point_to_it() {
    let mut s = editor_with("    first\n    second\n");
    goto(&s, 11); // inside the second line's indentation
    tab(&mut s);
    assert_eq!(text(&s), "    first\n    second\n", "nothing changed");
    assert_eq!(cursor(&s), 14, "point moved to the indentation");
    assert!(!popup_visible(&s), "and no completion opened");
}

// ---------------------------------------------------------------------------
// Completing
// ---------------------------------------------------------------------------

/// A line already at its indentation with point past it: TAB is
/// `completion.at-point`. With a server that answers, the popup opens
/// with its candidates and the text is untouched. Bitten by removing
/// the `completion.at-point` invocation: nothing opens.
#[test]
fn e7b_4_tab_on_an_indented_line_completes_at_point() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = rust_editor(td.path(), "fn main() {\n    let a = 1;\n    prin\n}\n");
    goto(&s, 35); // after `prin`, on a line indented like the one above
    let before = text(&s);
    tab(&mut s);
    let opened = pump_until(&mut s, 10_000, popup_visible);
    if !opened {
        let errors = s.lua_host.errors_buffer_text();
        let recent: Vec<String> = eval(
            &s,
            "local out = {}
             local rec = pmacs.lsp.active_attachment()
             if not rec then return { 'no attachment' } end
             for _, m in ipairs(pmacs.lsp.recent_messages(rec.server)) do
               out[#out + 1] = m.summary
             end
             return out",
        );
        panic!(
            "TAB with nothing to indent opens the completion popup; status {:?}; *errors*: {errors:?}; *lsp*: {recent:?}",
            s.core.borrow().status
        );
    }
    assert_eq!(text(&s), before, "and inserts nothing");
}

/// The same line, but under-indented: the FIRST TAB indents and opens
/// nothing; the SECOND completes.
#[test]
fn e7b_4_first_tab_indents_second_tab_completes() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = rust_editor(td.path(), "fn main() {\n    let a = 1;\nprin\n}\n");
    goto(&s, 31); // after `prin`
    tab(&mut s);
    assert_eq!(text(&s), "fn main() {\n    let a = 1;\n    prin\n}\n");
    assert_eq!(cursor(&s), 35);
    for _ in 0..20 {
        tick(&mut s);
    }
    assert!(!popup_visible(&s), "the indenting TAB opens nothing");
    tab(&mut s);
    assert!(
        pump_until(&mut s, 10_000, popup_visible),
        "the second TAB completes"
    );
}

/// No server: TAB with nothing to indent asks for completion and, with
/// no candidates anywhere, nothing opens and nothing is inserted ---
/// the key is inert rather than a tab character.
#[test]
fn e7b_4_tab_with_nothing_to_indent_and_no_server_inserts_nothing() {
    let mut s = editor_with("plain\n");
    goto(&s, 5);
    tab(&mut s);
    for _ in 0..20 {
        tick(&mut s);
    }
    assert_eq!(text(&s), "plain\n");
    assert!(!popup_visible(&s));
}

// ---------------------------------------------------------------------------
// What it does not settle
// ---------------------------------------------------------------------------

/// An active region keeps the CUA type-over: the selection is replaced
/// by one tab character in one edit, as before E7b.4.
#[test]
fn e7b_4_tab_with_a_region_keeps_the_type_over() {
    let mut s = editor_with("    first\nsecond\n");
    goto(&s, 10);
    exec(&s, "pmacs.command.invoke('region.set-mark')");
    goto(&s, 13);
    tab(&mut s);
    assert_eq!(text(&s), "    first\n\tond\n");
}

/// The minibuffer's TAB is E6.3's common-prefix completion, decided by
/// the prompt's own decoder before any keymap is consulted; E7b.4 does
/// not reach it.
#[test]
fn e7b_4_the_minibuffer_s_tab_is_untouched() {
    let mut s = editor_with("x\n");
    exec(
        &s,
        "pmacs.minibuffer.read { prompt = 'P: ',
           source = function() return { 'alpha-one', 'alpha-two' } end,
           on_accept = function() end }",
    );
    assert!(eval::<bool>(&s, "return pmacs.minibuffer.is_active()"));
    for ch in "alp".chars() {
        press(&mut s, KeyCode::Char(ch));
    }
    tab(&mut s);
    let field: String = eval(&s, "return pmacs.minibuffer.contents()");
    assert_eq!(field, "alpha-", "TAB completed the prompt's common prefix");
    assert_eq!(text(&s), "x\n", "and touched no buffer");
}
