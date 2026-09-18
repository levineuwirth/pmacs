// tests/e7b_review_prompt_acceptance.rs --- C7b review 1: the one-key
// prompts as a user meets them.

//! E7b.2 put quit, kill-buffer and revert-buffer on one key and moved
//! the write-file overwrite and `*git-status*`'s `x` to the typed
//! word. These rows are the states the phase's witnesses did not
//! choose:
//!
//! * kill-buffer with unsaved changes, answered `n`, `C-g`, a stray
//!   key, RET on the empty field, and `y` --- with the prompt's own
//!   wording read, because on one key the wording is the safety: `y`
//!   is the destructive answer to "kill anyway?".
//! * revert-buffer on one key is right by the adopted rule only if the
//!   revert can be taken back, which the record claims from the code's
//!   comment; here `buffer.undo` after a `y` is the test of it.
//! * the typed question keeps the one-key question's suffix,
//!   "(y or n)", while a bare `y` no longer answers it; the row pins
//!   what a user reads today, for the owner's ruling on the bare `y`.
//!
//! Every key goes through `dispatch_key`, as both frontends' do.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

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

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

fn asking(s: &EditorState) -> bool {
    eval(s, "return pmacs.minibuffer.is_active()")
}

fn prompt(s: &EditorState) -> String {
    eval::<Option<String>>(s, "return pmacs.minibuffer.prompt()").unwrap_or_default()
}

fn field(s: &EditorState) -> String {
    eval::<Option<String>>(s, "return pmacs.minibuffer.contents()").unwrap_or_default()
}

fn buffer_text(s: &EditorState) -> String {
    eval(
        s,
        "local b = pmacs.window.buffer() return b:slice(0, b:len())",
    )
}

fn buffer_names(s: &EditorState) -> Vec<String> {
    eval(
        s,
        "local out = {}
         for _, id in ipairs(pmacs.buffer.list()) do
           out[#out + 1] = pmacs.describe.buffer(id).name
         end
         return out",
    )
}

fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    s
}

fn visit(s: &EditorState, dir: &std::path::Path, name: &str, text: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, text).expect("write fixture");
    let display = path.display().to_string();
    exec(s, &format!("pmacs.buffer.find_or_open({display:?})"));
    display
}

/// `C-x k` RET on a modified buffer (the name prefilled): the question
/// is standing, the buffer is still there.
fn ask_to_kill(s: &mut EditorState, path: &str) {
    ctrl(s, 'x');
    press(s, KeyCode::Char('k'));
    assert!(asking(s), "C-x k asks for a name");
    press(s, KeyCode::Enter);
    assert!(asking(s), "a modified buffer makes the kill ask");
    assert!(
        buffer_names(s).contains(&path.to_owned()),
        "and nothing is killed yet"
    );
}

/// The kill question, every key a user might press at it. `n`, `C-g`
/// and the stray keys keep the buffer; `y` kills it, on one key, with
/// no word to type --- and the question that `y` destroys on reads
/// "kill anyway?", so a `y` meant for the buffer and landing on the
/// prompt is the loss. The wording is pinned as read.
#[test]
fn kill_buffer_with_unsaved_changes_as_a_user_meets_it() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    let path = visit(&s, td.path(), "alpha.txt", "alpha\n");
    type_str(&mut s, "X");

    ask_to_kill(&mut s, &path);
    let asked = prompt(&s);
    eprintln!("PROMPT kill: {asked:?}");
    assert!(
        asked.contains("has unsaved changes; kill anyway?") && asked.ends_with("(y or n) "),
        "the question names the loss and asks for one key: {asked:?}"
    );
    press(&mut s, KeyCode::Char('n'));
    assert!(!asking(&s), "n alone answers");
    assert!(buffer_names(&s).contains(&path));
    assert_eq!(status(&s), "kill-buffer cancelled");

    ask_to_kill(&mut s, &path);
    ctrl(&mut s, 'g');
    assert!(!asking(&s), "C-g answers");
    assert!(buffer_names(&s).contains(&path), "and keeps the buffer");

    ask_to_kill(&mut s, &path);
    for (name, code) in [
        ("k", KeyCode::Char('k')),
        ("SPC", KeyCode::Char(' ')),
        ("RET", KeyCode::Enter),
        ("Y-adjacent t", KeyCode::Char('t')),
    ] {
        press(&mut s, code);
        assert!(asking(&s), "{name} re-asks");
        assert_eq!(field(&s), "", "with the field emptied after {name}");
        assert_eq!(
            status(&s),
            "please answer y or n",
            "and says so after {name}"
        );
        assert!(
            buffer_names(&s).contains(&path),
            "and keeps the buffer after {name}"
        );
    }
    press(&mut s, KeyCode::Char('y'));
    assert!(!asking(&s), "y alone answers");
    assert!(
        !buffer_names(&s).contains(&path),
        "and the buffer is gone with its X: {:?}",
        buffer_names(&s)
    );
}

/// The claim behind revert-buffer's one key: the reload keeps undo
/// history, so a `y` the user did not mean is one `buffer.undo` from
/// undone. Modified, reverted on `y`, undone: the edit is back.
#[test]
fn revert_on_one_key_is_taken_back_by_undo() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    let alpha = visit(&s, td.path(), "alpha.txt", "alpha\n");
    type_str(&mut s, "X");
    std::fs::write(&alpha, b"changed underneath\n").expect("rewrite");
    exec(&s, "pmacs.command.invoke('revert-buffer')");
    assert!(asking(&s));
    let asked = prompt(&s);
    eprintln!("PROMPT revert: {asked:?}");
    press(&mut s, KeyCode::Char('y'));
    assert!(!asking(&s), "y alone answers");
    assert_eq!(buffer_text(&s), "changed underneath\n", "and reloads");
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('u'));
    assert_eq!(
        buffer_text(&s),
        "Xalpha\n",
        "one undo brings the unsaved work back, which is what makes the question the reversible kind"
    );
}

/// The typed question wears the one-key question's suffix. `C-x C-w`
/// onto an existing file asks "... overwrite? (y or n) "; a bare `y`
/// sits in the field and answers nothing, `y RET` overwrites. The row
/// pins what a user reads today: the suffix that means "press y" on
/// the quit prompt means "type y and RET" here, and Emacs's typed
/// question says "(yes or no)". For the owner's ruling on the bare
/// `y`; the fix, whichever way it goes, changes this row.
#[test]
fn the_typed_question_still_says_y_or_n() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    visit(&s, td.path(), "alpha.txt", "alpha\n");
    let beta_path = td.path().join("beta.txt");
    std::fs::write(&beta_path, b"beta\n").expect("write occupant");
    ctrl(&mut s, 'x');
    ctrl(&mut s, 'w');
    assert!(asking(&s), "C-x C-w asks for a path");
    type_str(&mut s, "beta.txt");
    press(&mut s, KeyCode::Enter);
    assert!(asking(&s), "an existing file makes the overwrite ask");
    let asked = prompt(&s);
    eprintln!("PROMPT overwrite: {asked:?}");
    assert!(
        asked.contains("exists; overwrite?") && asked.ends_with("(y or n) "),
        "the typed question's suffix is the one-key question's: {asked:?}"
    );
    press(&mut s, KeyCode::Char('y'));
    assert!(asking(&s), "a bare y answers nothing");
    assert_eq!(field(&s), "y", "it sits in the field");
    assert_eq!(
        std::fs::read_to_string(&beta_path).unwrap(),
        "beta\n",
        "and nothing was written"
    );
    press(&mut s, KeyCode::Enter);
    assert!(!asking(&s), "y RET answers");
    assert_eq!(
        std::fs::read_to_string(&beta_path).unwrap(),
        "alpha\n",
        "and overwrites"
    );
}
