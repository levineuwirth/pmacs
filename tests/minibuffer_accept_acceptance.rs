// tests/minibuffer_accept_acceptance.rs --- E6.1 (D18): the accept policy.

//! The minibuffer's accept policy through the keys a user presses:
//! `M-x hel RET` runs `help` under `candidate`; `C-j` takes the typed
//! text under either policy; a `typed` prompt gives `on_accept` the
//! field as written whatever is selected; `write-file` carries the
//! policy the roadmap names for it, and `switch-buffer` the one D18's
//! amendment names (E7c fix round 2): `candidate`, so a subsequence of
//! a buffer's name switches on RET.
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

/// D18 as amended (E7c fix round 2, the owner's ruling): `switch-buffer`
/// passes `candidate`, so a subsequence of an existing name switches on
/// RET by itself --- `C-x b nts RET` reaches `notes.txt` --- where under
/// `typed` it reported `no buffer: nts` and switched nothing; TAB then
/// RET still switches. The felt case: `C-c l` opens `*lsp*`, and
/// `C-x b lsp RET` reaches it and `C-x b scr RET` reaches `*scratch*`.
/// A name matching nothing is looked up as typed and refused as
/// before: `C-x b zzz RET` says `no buffer: zzz` and switches nothing.
/// Bitten by restoring `accept = "typed"` on the command.
#[test]
fn switch_buffer_ret_takes_the_selected_buffer_so_a_subsequence_switches() {
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
    assert!(!active(&s), "RET closed the prompt");
    assert_eq!(
        active_name(&s),
        notes_name,
        "the selection wins: `nts RET` switched to the notes buffer"
    );
    assert_ne!(status(&s), "no buffer: nts");

    // TAB then RET still switches.
    exec(&s, "pmacs.window.switch_buffer(pmacs.buffer.list()[1])");
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "nts");
    press(&mut s, KeyCode::Tab);
    assert_eq!(contents(&s), notes_name, "TAB completes to the selection");
    press(&mut s, KeyCode::Enter);
    assert_eq!(active_name(&s), notes_name, "TAB then RET switches");

    // The felt case: `*lsp*` opened by `C-c l` (a bottom panel, which
    // needs a frame with rows to take), then reached by its
    // subsequence, and `*scratch*` by its own.
    s.sync_frame_geometry(FrontendId::LOCAL, pmacs::protocol::CellSize::new(40, 100));
    ctrl(&mut s, 'c');
    press(&mut s, KeyCode::Char('l'));
    assert_eq!(active_name(&s), "*lsp*", "C-c l opens *lsp*");
    exec(&s, "pmacs.window.switch_buffer(pmacs.buffer.list()[1])");
    assert_eq!(active_name(&s), scratch);
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "lsp");
    press(&mut s, KeyCode::Enter);
    assert_eq!(active_name(&s), "*lsp*", "C-x b lsp RET reaches *lsp*");
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "scr");
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        active_name(&s),
        "*scratch*",
        "C-x b scr RET reaches *scratch*"
    );

    // No match: refused as typed, nothing switched.
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "zzz");
    assert!(
        candidates(&s).is_empty(),
        "fixture premise: nothing matches `zzz`"
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        status(&s),
        "no buffer: zzz",
        "the no-match behavior is as it was"
    );
    assert_eq!(active_name(&s), "*scratch*", "nothing switched");
}

/// `C-x b`, `text`, RET, as a user types them.
fn switch_by_typing(s: &mut EditorState, text: &str) {
    ctrl(s, 'x');
    press(s, KeyCode::Char('b'));
    assert!(active(s), "C-x b opens the prompt");
    type_str(s, text);
    press(s, KeyCode::Enter);
    assert!(!active(s), "RET closed the prompt");
}

/// E8b.1 (#318): a file buffer is named by its absolute path, so a bare
/// name is also a subsequence of paths it does not name. Under a
/// directory called `bin-tests`, `b.rs` matches `…/bin-tests/a.rs`
/// through the `b` of `bin` and the `.rs`, the scorer gives both paths
/// one score, and `C-x b b.rs RET` stayed on `a.rs`.
///
/// Beside it, in the same session, D18's four witnesses as
/// `switch_buffer_ret_takes_the_selected_buffer_so_a_subsequence_switches`
/// pins them: `nts` reaches `notes.txt`, `scr` `*scratch*`, `lsp`
/// `*lsp*`, and `zzz` is refused as typed; and each file's full path
/// still reaches it. `bin-tests` also spells `nts`, so the abbreviation
/// matches all three files alike and reached `a.rs` until a match inside
/// a name's last component outranked one through its directories. The
/// directory is the fixture's so that both hold on every machine: the
/// temporary root under this laptop's gate spells both, and CI's
/// `/tmp/.tmpXXXXXX` neither.
#[test]
fn switch_buffer_reaches_a_file_by_its_bare_name_and_d18_still_holds() {
    let td = tempfile::tempdir().expect("tempdir");
    let dir = td.path().join("bin-tests");
    std::fs::create_dir_all(&dir).expect("bin-tests dir");
    let mut s = fresh();
    let scratch = eval::<String>(
        &s,
        "return pmacs.describe.buffer(pmacs.buffer.list()[1]).name",
    );
    let mut names = Vec::new();
    for file in ["a.rs", "b.rs", "notes.txt"] {
        let path = dir.join(file);
        std::fs::write(&path, b"x\n").expect("write");
        exec(
            &s,
            &format!(
                "pmacs.buffer.find_or_open({:?})",
                path.display().to_string()
            ),
        );
        // The name as pmacs stores it (macOS's temp root is a symlink).
        names.push(active_name(&s));
    }
    let (a, b, notes) = (names[0].clone(), names[1].clone(), names[2].clone());
    assert!(b.ends_with("/bin-tests/b.rs"), "fixture: {b}");

    // The bare name, from `a.rs`.
    exec(&s, "pmacs.window.switch_buffer(pmacs.buffer.list()[2])");
    assert_eq!(active_name(&s), a);
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "b.rs");
    let offered = candidates(&s);
    assert!(
        offered.contains(&a) && offered.contains(&b),
        "fixture premise: `b.rs` is a subsequence of both paths; got {offered:?}"
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(active_name(&s), b, "C-x b b.rs RET reaches b.rs");

    // The full path reaches each file still.
    switch_by_typing(&mut s, &a);
    assert_eq!(active_name(&s), a, "a.rs's full path reaches it");
    switch_by_typing(&mut s, &b);
    assert_eq!(active_name(&s), b, "b.rs's full path reaches it");

    // D18's four, in this session.
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "nts");
    let offered = candidates(&s);
    assert!(
        [&a, &b, &notes].iter().all(|n| offered.contains(n)),
        "fixture premise: the directory spells `nts` for every file; got {offered:?}"
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(active_name(&s), notes, "C-x b nts RET reaches notes.txt");
    switch_by_typing(&mut s, "scr");
    assert_eq!(active_name(&s), scratch, "C-x b scr RET reaches *scratch*");
    s.sync_frame_geometry(FrontendId::LOCAL, pmacs::protocol::CellSize::new(40, 100));
    ctrl(&mut s, 'c');
    press(&mut s, KeyCode::Char('l'));
    assert_eq!(active_name(&s), "*lsp*", "C-c l opens *lsp*");
    exec(&s, "pmacs.window.switch_buffer(pmacs.buffer.list()[1])");
    assert_eq!(active_name(&s), scratch);
    switch_by_typing(&mut s, "lsp");
    assert_eq!(active_name(&s), "*lsp*", "C-x b lsp RET reaches *lsp*");
    switch_by_typing(&mut s, "scr");
    assert_eq!(active_name(&s), scratch);
    ctrl(&mut s, 'x');
    press(&mut s, KeyCode::Char('b'));
    type_str(&mut s, "zzz");
    assert!(
        candidates(&s).is_empty(),
        "fixture premise: nothing matches `zzz`"
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(status(&s), "no buffer: zzz", "zzz is refused as typed");
    assert_eq!(active_name(&s), scratch, "nothing switched");
}

/// E8b.2: RET names the buffer it reached on the status band, in the
/// band's `<command>: …` idiom, and says when the selection is the
/// buffer already shown, which changes nothing on screen: #318's
/// measured case ended on `a.rs` with an empty band. A subsequence
/// whose selection is the shown buffer says so as a full name does.
/// The next key clears it, as it clears every message, and a name
/// matching nothing is refused as before.
#[test]
fn switch_buffer_says_which_buffer_ret_reached() {
    let td = tempfile::tempdir().expect("tempdir");
    let dir = td.path().join("bin-tests");
    std::fs::create_dir_all(&dir).expect("bin-tests dir");
    let mut s = fresh();
    let scratch = eval::<String>(
        &s,
        "return pmacs.describe.buffer(pmacs.buffer.list()[1]).name",
    );
    let mut names = Vec::new();
    for file in ["a.rs", "b.rs"] {
        let path = dir.join(file);
        std::fs::write(&path, b"x\n").expect("write");
        exec(
            &s,
            &format!(
                "pmacs.buffer.find_or_open({:?})",
                path.display().to_string()
            ),
        );
        names.push(active_name(&s));
    }
    let (a, b) = (names[0].clone(), names[1].clone());
    exec(&s, "pmacs.window.switch_buffer(pmacs.buffer.list()[2])");
    assert_eq!(active_name(&s), a);
    assert_eq!(status(&s), "", "fixture: the band starts empty");

    switch_by_typing(&mut s, "b.rs");
    assert_eq!(active_name(&s), b);
    assert_eq!(status(&s), format!("switch-buffer: showing {b}"));

    switch_by_typing(&mut s, &b);
    assert_eq!(active_name(&s), b, "nothing switched");
    assert_eq!(status(&s), format!("switch-buffer: already showing {b}"));

    switch_by_typing(&mut s, "b.r");
    assert_eq!(active_name(&s), b, "nothing switched");
    assert_eq!(
        status(&s),
        format!("switch-buffer: already showing {b}"),
        "a subsequence selecting the shown buffer says so"
    );

    press(&mut s, KeyCode::Right);
    assert_eq!(status(&s), "", "the next key clears the band");

    switch_by_typing(&mut s, "scr");
    assert_eq!(active_name(&s), scratch);
    assert_eq!(status(&s), format!("switch-buffer: showing {scratch}"));

    switch_by_typing(&mut s, "zzz");
    assert_eq!(status(&s), "no buffer: zzz", "the refusal is as it was");
    assert_eq!(active_name(&s), scratch, "nothing switched");
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
