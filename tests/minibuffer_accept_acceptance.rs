// tests/minibuffer_accept_acceptance.rs --- E6.1 (D18): the accept policy.

//! The minibuffer's accept policy through the keys a user presses:
//! `M-x hel RET` runs `help` under `candidate`; `C-j` takes the typed
//! text under either policy; a `typed` prompt gives `on_accept` the
//! field as written whatever is selected; `switch-buffer` and
//! `write-file` carry the policy the roadmap names for them.
//!
//! Dispatch-driven throughout, for the reason `find_file_acceptance`
//! gives: the Lua lifecycle `minibuffer.accept()` bypasses the path
//! interactive input takes, and a dead binding would pass vacuously.

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

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn fresh() -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    s
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

fn active(s: &EditorState) -> bool {
    eval(s, "return pmacs.minibuffer.is_active()")
}

fn contents(s: &EditorState) -> String {
    eval(s, "return pmacs.minibuffer.contents()")
}

fn candidates(s: &EditorState) -> Vec<String> {
    eval(s, "return pmacs.minibuffer.candidates()")
}

fn named_text(s: &EditorState, name: &str) -> String {
    eval(
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
    )
}

fn active_name(s: &EditorState) -> String {
    eval(
        s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    )
}

/// Open a command-source prompt from Lua under `policy`, recording
/// what `on_accept` receives in `_G.GOT`.
fn open_recording_prompt(s: &EditorState, policy: &str) {
    exec(
        s,
        &format!(
            r#"
            _G.GOT = nil
            pmacs.minibuffer.read {{
              prompt = "P: ",
              source = "commands",
              accept = {policy:?},
              on_accept = function(v) _G.GOT = v end,
            }}
            "#
        ),
    );
    assert!(active(s), "the prompt must be open");
}

fn got(s: &EditorState) -> Option<String> {
    eval(s, "return _G.GOT")
}

/// E6.1's third row: `M-x hel RET` runs `help`, because `M-x` passes
/// `accept = "candidate"` and `help` is the best-scoring candidate.
#[test]
fn m_x_hel_ret_runs_help() {
    let mut s = fresh();
    assert!(named_text(&s, "*help*").is_empty(), "no *help* before");

    alt(&mut s, 'x');
    assert!(active(&s), "M-x must open the palette");
    type_str(&mut s, "hel");
    let cands = candidates(&s);
    assert_eq!(
        cands.first().map(String::as_str),
        Some("help"),
        "fixture premise: `help` must lead the candidates for `hel`; got {cands:?}"
    );

    press(&mut s, KeyCode::Enter);

    assert!(!active(&s), "RET closes the palette");
    let text = named_text(&s, "*help*");
    assert!(
        text.contains("help"),
        "M-x hel RET must run `help` and render *help*; got {text:?}, status {:?}",
        status(&s)
    );
}

/// E6.1's fourth row: `C-j` under `candidate` accepts the typed text.
/// Through `M-x` itself the consequence is an error naming `hel`, and
/// no `*help*`.
#[test]
fn c_j_under_candidate_takes_the_typed_text_through_m_x() {
    let mut s = fresh();
    alt(&mut s, 'x');
    type_str(&mut s, "hel");
    assert_eq!(candidates(&s).first().map(String::as_str), Some("help"));

    ctrl(&mut s, 'j');

    assert!(!active(&s), "C-j closes the palette");
    assert!(
        named_text(&s, "*help*").is_empty(),
        "C-j must not run the selected `help`"
    );
    let line = status(&s);
    assert!(
        line.starts_with("M-x error:") && line.contains("hel"),
        "the typed text `hel` is what M-x tried to run; got {line:?}"
    );
}

/// The same fourth row at the seam every caller shares: one candidate
/// prompt, `C-j` yields the typed text and RET yields the selection.
#[test]
fn c_j_and_ret_resolve_differently_on_one_candidate_prompt() {
    let mut s = fresh();
    open_recording_prompt(&s, "candidate");
    type_str(&mut s, "hel");
    assert_eq!(candidates(&s).first().map(String::as_str), Some("help"));
    ctrl(&mut s, 'j');
    assert_eq!(got(&s).as_deref(), Some("hel"), "C-j: the text as typed");

    open_recording_prompt(&s, "candidate");
    type_str(&mut s, "hel");
    press(&mut s, KeyCode::Enter);
    assert_eq!(got(&s).as_deref(), Some("help"), "RET: the selection");
}

/// Under `typed`, RET is the field as written whatever is selected,
/// and `C-j` is the same thing.
#[test]
fn typed_policy_ret_takes_the_field_as_written() {
    let mut s = fresh();
    open_recording_prompt(&s, "typed");
    type_str(&mut s, "hel");
    assert_eq!(
        candidates(&s).first().map(String::as_str),
        Some("help"),
        "a candidate is selected, and must not win"
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(got(&s).as_deref(), Some("hel"));

    open_recording_prompt(&s, "typed");
    type_str(&mut s, "hel");
    ctrl(&mut s, 'j');
    assert_eq!(got(&s).as_deref(), Some("hel"));
}

/// Omitting `accept` keeps today's picker semantics, and the field is
/// validated by name.
#[test]
fn accept_defaults_to_candidate_and_rejects_other_values() {
    let mut s = fresh();
    exec(
        &s,
        r#"
        _G.GOT = nil
        pmacs.minibuffer.read {
          prompt = "P: ",
          source = "commands",
          on_accept = function(v) _G.GOT = v end,
        }
        "#,
    );
    type_str(&mut s, "hel");
    press(&mut s, KeyCode::Enter);
    assert_eq!(got(&s).as_deref(), Some("help"), "omitted: candidate");

    let (ok, err): (bool, String) = eval(
        &s,
        r#"
        local ok, err = pcall(pmacs.minibuffer.read, {
          prompt = "P: ", accept = "nope", on_accept = function() end,
        })
        return ok, tostring(err)
        "#,
    );
    assert!(!ok, "an unknown policy must be refused");
    assert!(
        err.contains("accept") && err.contains("nope"),
        "the refusal names the field and the value; got {err:?}"
    );
    let (ok, err): (bool, String) = eval(
        &s,
        r#"
        local ok, err = pcall(pmacs.minibuffer.read, {
          prompt = "P: ", accept = true, on_accept = function() end,
        })
        return ok, tostring(err)
        "#,
    );
    assert!(!ok, "a non-string policy must be refused");
    assert!(err.contains("accept"), "got {err:?}");
    assert!(!active(&s), "a refused read opens no session");
}

/// Probe: `switch-buffer` passes `typed`, so a subsequence of an
/// existing name no longer switches by itself --- RET reports the name
/// as typed, and TAB then RET switches.
#[test]
fn switch_buffer_ret_takes_the_typed_name_and_tab_completes_it() {
    let td = tempfile::tempdir().expect("tempdir");
    let notes = td.path().join("notes.txt");
    std::fs::write(&notes, b"n\n").expect("write");
    let mut s = fresh();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            notes.display().to_string()
        ),
    );
    let scratch = eval::<String>(
        &s,
        "return pmacs.describe.buffer(pmacs.buffer.list()[1]).name",
    );
    exec(&s, "pmacs.window.switch_buffer(pmacs.buffer.list()[1])");
    assert_eq!(active_name(&s), scratch);

    // A file buffer is named by its path as pmacs stores it, so that
    // name is the candidate. Read it back rather than canonicalizing:
    // on macOS the temp root is a symlink (`/var` -> `/private/var`) and
    // pmacs keeps the path as given.
    exec(&s, "pmacs.window.switch_buffer(pmacs.buffer.list()[2])");
    let notes_name = active_name(&s);
    assert!(notes_name.ends_with("notes.txt"), "fixture: {notes_name}");
    exec(&s, "pmacs.window.switch_buffer(pmacs.buffer.list()[1])");
    assert_eq!(active_name(&s), scratch);

    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    assert!(active(&s), "C-x b opens the prompt");
    type_str(&mut s, "nts");
    assert!(
        candidates(&s).contains(&notes_name),
        "fixture premise: `nts` matches the notes buffer; got {:?}",
        candidates(&s)
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        status(&s),
        "no buffer: nts",
        "typed wins: the name as written is what switch-buffer looked up"
    );
    assert_eq!(active_name(&s), scratch, "nothing switched");

    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "nts");
    press(&mut s, KeyCode::Tab);
    assert_eq!(contents(&s), notes_name, "TAB completes to the selection");
    press(&mut s, KeyCode::Enter);
    assert_eq!(active_name(&s), notes_name, "TAB then RET switches");
}

/// Probe: `write-file` roots its field where `find-file` does, prefilled
/// with the directory, and writes the typed name under it.
#[test]
fn write_file_prefills_the_root_and_writes_the_typed_name() {
    let td = tempfile::tempdir().expect("tempdir");
    let anchor = td.path().join("anchor.txt");
    std::fs::write(&anchor, b"anchor\n").expect("write");
    let mut s = fresh();
    exec(
        &s,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            anchor.display().to_string()
        ),
    );

    ctrl(&mut s, 'x');
    ctrl(&mut s, 'w');
    assert!(active(&s), "C-x C-w opens the prompt");
    // The root is the directory of the path pmacs stores for the
    // buffer, not a canonicalized one (macOS's temp root is a symlink).
    let stored: String = eval(&s, "return pmacs.window.buffer():path()");
    let root = std::path::Path::new(&stored)
        .parent()
        .expect("the anchor has a directory")
        .to_path_buf();
    assert_eq!(
        contents(&s),
        format!("{}/", root.display()),
        "the field is prefilled with the buffer's directory"
    );
    type_str(&mut s, "out.txt");
    press(&mut s, KeyCode::Enter);

    let out = root.join("out.txt");
    assert!(
        out.is_file(),
        "the typed name is written under the root; status {:?}",
        status(&s)
    );
    assert_eq!(std::fs::read(&out).expect("read"), b"anchor\n");
}

#[path = "common/iso.rs"]
mod iso;
