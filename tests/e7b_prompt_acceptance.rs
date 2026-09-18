// tests/e7b_prompt_acceptance.rs --- E7b.2, `y_or_n` takes one key.

//! Emacs's pair, adopted by the owner at E7b.2: `y-or-n-p` is one
//! keypress for a choice the user can take back; `yes-or-no-p` demands
//! a typed word for one that destroys work. `pmacs.minibuffer.y_or_n`
//! now answers on the keypress --- `y`, `n`, `C-g` to abort, anything
//! else re-asks --- through the minibuffer's `key` accept policy, and
//! `pmacs.minibuffer.yes_or_no` is exactly the prompt `y_or_n` was
//! before. Every witness drives keys through `dispatch_key`, the seam
//! both frontends reach the daemon by.
//!
//! The callers, audited (the record says why each is where it is):
//! quit, kill-buffer and revert-buffer on one key; the write-file
//! overwrite and `*git-status*`'s `x` on the typed word (the latter in
//! `tests/e7_git_status_keys_acceptance.rs`, beside its fixtures).

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

/// A `.txt` buffer holding `text`, visited from disk and left active.
fn visit(s: &EditorState, dir: &std::path::Path, name: &str, text: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, text).expect("write fixture");
    let display = path.display().to_string();
    exec(s, &format!("pmacs.buffer.find_or_open({display:?})"));
    display
}

/// `C-x C-c` on a modified buffer: the question is standing, nothing
/// has quit.
fn ask_to_quit(s: &mut EditorState) {
    type_str(s, "X");
    ctrl(s, 'x');
    ctrl(s, 'c');
    assert!(asking(s), "a modified buffer must make the quit ask");
    assert!(!s.core.borrow().quit, "and nothing has quit yet");
}

// ---------------------------------------------------------------------------
// The quit prompt, one key
// ---------------------------------------------------------------------------

/// `n` alone refuses: the question closes on the key, the editor is
/// still running, and the status says so. Bitten by asking through
/// `yes_or_no`, which would leave the question standing with `n` in
/// its field.
#[test]
fn e7b_2_quit_answers_n_on_one_key() {
    let mut s = editor();
    ask_to_quit(&mut s);
    press(&mut s, KeyCode::Char('n'));
    assert!(!asking(&s), "n alone must answer the question");
    assert!(!s.core.borrow().quit, "n keeps the editor running");
    assert_eq!(status(&s), "quit cancelled");
}

/// `y` alone quits, and so does `Y`.
#[test]
fn e7b_2_quit_answers_y_on_one_key() {
    for answer in ['y', 'Y'] {
        let mut s = editor();
        ask_to_quit(&mut s);
        press(&mut s, KeyCode::Char(answer));
        assert!(!asking(&s), "{answer} alone must answer the question");
        assert!(s.core.borrow().quit, "{answer} quits");
    }
}

/// Any other key re-asks at once --- the field is emptied for the next
/// answer, nothing quit, and the status says what is wanted. RET on the
/// empty field re-asks too. Then `C-g` aborts, which is a no.
#[test]
fn e7b_2_quit_re_asks_on_any_other_key_and_c_g_aborts() {
    let mut s = editor();
    ask_to_quit(&mut s);
    for other in ['m', ' ', '7'] {
        press(&mut s, KeyCode::Char(other));
        assert!(asking(&s), "{other:?} must leave the question standing");
        assert_eq!(status(&s), "please answer y or n");
        assert_eq!(field(&s), "", "the re-asked field is empty after {other:?}");
        assert!(!s.core.borrow().quit);
    }
    press(&mut s, KeyCode::Enter);
    assert!(asking(&s), "RET on an empty field re-asks");
    assert_eq!(status(&s), "please answer y or n");
    ctrl(&mut s, 'g');
    assert!(!asking(&s), "C-g closes the question");
    assert!(!s.core.borrow().quit, "and is a refusal");
    assert_eq!(status(&s), "Quit");
}

// ---------------------------------------------------------------------------
// The other one-key callers
// ---------------------------------------------------------------------------

/// `buffer.kill-this` on a modified buffer: `n` alone keeps it.
#[test]
fn e7b_2_kill_buffer_answers_on_one_key() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    let path = visit(&s, td.path(), "alpha.txt", "alpha\n");
    type_str(&mut s, "X");
    exec(&s, "pmacs.command.invoke('buffer.kill-this')");
    assert!(asking(&s));
    press(&mut s, KeyCode::Char('n'));
    assert!(!asking(&s), "n alone answers");
    assert!(buffer_names(&s).contains(&path), "and keeps the buffer");
    assert_eq!(status(&s), "kill-buffer cancelled");
}

/// `revert-buffer` with unsaved edits: `n` alone keeps the edit, `y`
/// alone reloads. The reload keeps undo history, which is why this
/// question is the reversible kind.
#[test]
fn e7b_2_revert_buffer_answers_on_one_key() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    let alpha = visit(&s, td.path(), "alpha.txt", "alpha\n");
    type_str(&mut s, "X");
    std::fs::write(&alpha, b"changed underneath\n").expect("rewrite");

    exec(&s, "pmacs.command.invoke('revert-buffer')");
    assert!(asking(&s));
    press(&mut s, KeyCode::Char('n'));
    assert!(!asking(&s), "n alone answers");
    assert_eq!(buffer_text(&s), "Xalpha\n", "and keeps the edit");
    assert_eq!(status(&s), "revert-buffer cancelled");

    exec(&s, "pmacs.command.invoke('revert-buffer')");
    assert!(asking(&s));
    press(&mut s, KeyCode::Char('y'));
    assert!(!asking(&s), "y alone answers");
    assert_eq!(buffer_text(&s), "changed underneath\n", "and reloads");
}

// ---------------------------------------------------------------------------
// The typed-word caller
// ---------------------------------------------------------------------------

/// `C-x C-w` onto an existing file overwrites another file's bytes on
/// disk, which no undo gets back: the typed question. A bare `y` sits
/// in the field and answers nothing; `y RET` and `yes RET` overwrite;
/// an empty RET and a wrong word re-ask. Bitten by asking through
/// `y_or_n`, which would overwrite on the bare `y`.
#[test]
fn e7b_2_write_file_overwrite_is_the_typed_question() {
    let td = tempfile::tempdir().expect("tempdir");
    let mut s = editor();
    visit(&s, td.path(), "alpha.txt", "alpha\n");
    let occupied = td.path().join("occupied.txt");
    std::fs::write(&occupied, b"old\n").expect("write occupant");

    ctrl(&mut s, 'x');
    ctrl(&mut s, 'w');
    type_str(&mut s, "occupied.txt");
    press(&mut s, KeyCode::Enter);
    let prompt: String = eval(&s, "return pmacs.minibuffer.prompt()");
    assert!(prompt.contains("overwrite"), "the question: {prompt:?}");

    press(&mut s, KeyCode::Char('y'));
    assert!(asking(&s), "a bare y answers nothing here");
    assert_eq!(field(&s), "y");
    assert_eq!(
        std::fs::read_to_string(&occupied).unwrap(),
        "old\n",
        "nothing written on a bare y"
    );
    press(&mut s, KeyCode::Backspace);
    press(&mut s, KeyCode::Enter);
    assert!(asking(&s), "an empty RET re-asks");
    assert_eq!(status(&s), "please answer yes or no");
    type_str(&mut s, "sure");
    press(&mut s, KeyCode::Enter);
    assert!(asking(&s), "a wrong word re-asks");
    assert_eq!(std::fs::read_to_string(&occupied).unwrap(), "old\n");

    type_str(&mut s, "yes");
    press(&mut s, KeyCode::Enter);
    assert!(!asking(&s));
    assert_eq!(
        std::fs::read_to_string(&occupied).unwrap(),
        "alpha\n",
        "yes RET overwrites"
    );
}

// ---------------------------------------------------------------------------
// The primitive
// ---------------------------------------------------------------------------

/// The `key` policy is a `pmacs.minibuffer.read` policy like the other
/// two: named in the refusal for a wrong name, and under it a printable
/// key commits itself while RET commits the typed text.
#[test]
fn e7b_2_the_key_accept_policy_commits_a_printable_key_at_once() {
    let mut s = editor();
    let err: String = eval(
        &s,
        "local ok, err = pcall(pmacs.minibuffer.read, {
           prompt = 'q ', accept = 'bogus', on_accept = function() end })
         return tostring(err)",
    );
    assert!(
        err.contains("\"candidate\", \"typed\" or \"key\""),
        "the refusal names all three: {err}"
    );

    exec(
        &s,
        "_G.ANSWERS = {}
         pmacs.minibuffer.read {
           prompt = 'q ', accept = 'key',
           on_accept = function(v) _G.ANSWERS[#_G.ANSWERS + 1] = v end,
         }",
    );
    assert!(asking(&s));
    press(&mut s, KeyCode::Char('k'));
    assert!(!asking(&s), "the key closed the prompt");
    let answers: Vec<String> = eval(&s, "return _G.ANSWERS");
    assert_eq!(answers, vec!["k".to_owned()]);

    exec(
        &s,
        "pmacs.minibuffer.read {
           prompt = 'q ', accept = 'key',
           on_accept = function(v) _G.ANSWERS[#_G.ANSWERS + 1] = v end,
         }",
    );
    press(&mut s, KeyCode::Enter);
    assert!(!asking(&s), "RET commits the (empty) typed text");
    let answers: Vec<String> = eval(&s, "return _G.ANSWERS");
    assert_eq!(answers, vec!["k".to_owned(), String::new()]);
}
