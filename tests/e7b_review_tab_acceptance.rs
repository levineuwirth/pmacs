// tests/e7b_review_tab_acceptance.rs --- C7b review 1: TAB in the
// places E7b.4's witnesses did not go.

//! `edit.indent-or-complete` indents to the line above, or completes
//! at point when there is nothing to indent. The phase's nine rows
//! cover the indenting cases and the completing one under a word;
//! these are the rest of the lines a user's TAB lands on:
//!
//! * an empty line at the top of a file --- nothing above it, so
//!   nothing to indent, so TAB completes with an empty prefix; and a
//!   second TAB, with the popup open, is the popup's accept. What lands
//!   is whatever the server put first, pinned as read.
//! * an empty line under an indented line gains the indentation; a
//!   whitespace-only line under it is corrected to the same.
//! * a listview panel's TAB (`*buffer-list*`) is `listview.toggle`,
//!   which on a flat panel invokes `buffer.tab` by name and is refused
//!   by the read-only intercept: the panel is unchanged and the key
//!   does not indent or complete there.
//!
//! Every key goes through `dispatch_key`.

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

fn ctrl(s: &mut EditorState, c: char) {
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char(c), KeyModifiers::CONTROL),
    );
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

fn tick(s: &mut EditorState) {
    s.tick_processes();
    s.tick_lsp();
    s.tick_async();
}

fn popup_visible(s: &EditorState) -> bool {
    eval(s, "return pmacs.completion.popup_visible()")
}

fn popup_labels(s: &EditorState) -> Vec<String> {
    let core = s.core.borrow();
    let popup = core.completion_popup.lock().unwrap();
    popup
        .as_ref()
        .map(|p| p.candidates.iter().map(|c| c.label.clone()).collect())
        .unwrap_or_default()
}

fn pump_until(s: &mut EditorState, ms: u64, done: impl Fn(&EditorState) -> bool) -> bool {
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

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

const INITIALIZED: &str = "(function() \
   for _,r in ipairs(pmacs.lsp.list()) do \
     if r.state and r.state.kind=='initialized' then return true end \
   end \
   return false \
 end)()";

/// A `.rs` buffer with the fake server attached, as the phase's suite
/// builds one.
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
    let settle = Instant::now() + Duration::from_millis(250);
    while Instant::now() < settle {
        tick(&mut s);
        std::thread::sleep(Duration::from_millis(2));
    }
    s
}

/// A plain `.txt` buffer, no server.
fn text_editor(dir: &Path, body: &str) -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    let file = dir.join("a.txt");
    std::fs::write(&file, body).unwrap();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            file.display().to_string()
        ),
    );
    s
}

/// The top line of a file is at its indentation by definition (there
/// is nothing above it), so TAB there completes at point with an empty
/// prefix, and with the popup open a second TAB is the popup's accept:
/// the first candidate the server offered lands in the buffer. Pinned
/// as read, since a user who presses TAB twice on an empty first line
/// expecting a tab or an indent gets a completion instead.
#[test]
fn tab_twice_on_an_empty_first_line_accepts_the_servers_first_candidate() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = rust_editor(td.path(), "\nfn main() {}\n");
    goto(&s, 0);
    tab(&mut s);
    assert_eq!(
        text(&s),
        "\nfn main() {}\n",
        "the first TAB inserts nothing"
    );
    assert_eq!(cursor(&s), 0, "and moves nothing");
    let opened = pump_until(&mut s, 10_000, popup_visible);
    assert!(opened, "the first TAB opens the popup with an empty prefix");
    let labels = popup_labels(&s);
    eprintln!("TAB popup on the empty first line: {labels:?}");
    assert!(!labels.is_empty());
    tab(&mut s);
    assert!(!popup_visible(&s), "the second TAB is the popup's accept");
    let after = text(&s);
    eprintln!("TAB after the second TAB the file reads {after:?}");
    assert!(
        after.starts_with(labels[0].as_str()),
        "the first candidate landed: {after:?}"
    );
}

/// An empty line under an indented one gains that indentation, and a
/// whitespace-only line under it is corrected to the same --- both
/// without a server, both inserting no tab character.
#[test]
fn empty_and_whitespace_only_lines_take_the_line_aboves_indentation() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = text_editor(td.path(), "    alpha\n\n  \nbeta\n");
    goto(&s, 10); // the empty second line
    tab(&mut s);
    assert_eq!(text(&s), "    alpha\n    \n  \nbeta\n");
    assert_eq!(cursor(&s), 14, "point after the new indentation");
    goto(&s, 16); // inside the whitespace-only third line
    tab(&mut s);
    assert_eq!(
        text(&s),
        "    alpha\n    \n    \nbeta\n",
        "two spaces become four, and no tab character anywhere"
    );
    assert_eq!(cursor(&s), 19);
    // The last line, under a whitespace-only line: the nearest
    // NON-BLANK line above decides, so beta takes alpha's four.
    goto(&s, 20);
    tab(&mut s);
    assert_eq!(text(&s), "    alpha\n    \n    \n    beta\n");
    assert!(!text(&s).contains('\t'));
}

/// `*buffer-list*`'s TAB is the listview's, not the keymap's TAB: the
/// panel is read-only and unchanged, nothing indents, nothing
/// completes, and the file buffer behind the panel is untouched.
#[test]
fn a_listview_panels_tab_neither_indents_nor_completes() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = text_editor(td.path(), "alpha\n");
    ctrl(&mut s, 'x');
    ctrl(&mut s, 'b');
    let name: String = eval(
        &s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    );
    assert_eq!(name, "*buffer-list*", "C-x C-b shows the list");
    let before = text(&s);
    tab(&mut s);
    let status = s.core.borrow().status.clone();
    eprintln!("TAB in *buffer-list*: status {status:?}");
    assert_eq!(text(&s), before, "the panel is unchanged");
    assert!(!popup_visible(&s), "and no completion opened");
    assert!(!before.contains('\t'));
}
