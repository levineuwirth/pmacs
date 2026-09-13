// tests/find_file_acceptance.rs --- dired arc Stage 0 (`C-x C-f`) acceptance.

//! Acceptance for `find-file`, the dired arc's Stage 0
//! (the archived dired framing §14, items 0a-0d, Q#DR11).
//!
//! Dispatch-driven throughout: the prompt is opened with a real
//! `C-x C-f`, filled by typing real keys, and completed with a real
//! RET. `pmacs.command.invoke` would bypass the binding (a dead
//! keymap entry would pass vacuously) and the Lua lifecycle
//! `minibuffer.accept()` bypasses the dispatch path interactive input
//! actually takes --- the editops suite's discipline, for the same
//! reasons.
//!
//! Fixtures use `.txt` files so no `buffer.after-load` hook spawns a
//! language server.

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

/// Open the find-file prompt through the real `C-x C-f` binding.
fn open_prompt(s: &mut EditorState) {
    ctrl(s, 'x');
    ctrl(s, 'f');
    assert!(
        eval::<bool>(s, "return pmacs.minibuffer.is_active()"),
        "C-x C-f must open a minibuffer prompt"
    );
}

/// The active buffer's backing path, or `None`.
fn active_path(s: &EditorState) -> Option<String> {
    eval::<Option<String>>(
        s,
        "local b = pmacs.window.buffer()\n\
         if b == nil then return nil end\n\
         local ok, p = pcall(function() return b:path() end)\n\
         if ok then return p end\n\
         return nil",
    )
}

fn candidates(s: &EditorState) -> Vec<String> {
    eval::<Vec<String>>(s, "return pmacs.minibuffer.candidates()")
}

fn contents(s: &EditorState) -> String {
    eval::<String>(s, "return pmacs.minibuffer.contents()")
}

/// Erase the prefilled directory with real Backspaces, so a row that
/// wants to type a path of its own starts from an empty field the way
/// a user would.
fn clear_field(s: &mut EditorState) {
    let n = contents(s).chars().count();
    for _ in 0..n {
        press(s, KeyCode::Backspace);
    }
    assert_eq!(contents(s), "", "the field must be empty after clearing");
}

/// Drive the async runtime until nothing is parked or pending: dired
/// lists a directory on a worker and resumes on a later tick, so a
/// directory opened by RET is observable only after this returns.
fn pump(s: &mut EditorState) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut spins = 0u32;
    loop {
        let idle: bool = eval(
            s,
            "return pmacs._async.parked_count() == 0 and pmacs._async.pending_count() == 0",
        );
        if idle {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "async pump deadline exceeded"
        );
        s.tick_async();
        spins += 1;
        if spins > 64 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

/// The active buffer's name.
fn active_name(s: &EditorState) -> String {
    eval::<String>(
        s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    )
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

/// An editor whose active buffer is a real file inside `dir`, so
/// find-file's root resolves to that directory.
fn editor_in(dir: &std::path::Path) -> EditorState {
    let anchor = dir.join("anchor.txt");
    std::fs::write(&anchor, b"anchor\n").expect("write anchor");
    let state = EditorState::new_with_roots(&crate::iso::roots());
    state.lua_host.reopen_init_phase_for_testing();
    let anchor_str = anchor.display().to_string();
    exec(
        &state,
        &format!("pmacs.buffer.find_or_open({anchor_str:?})"),
    );
    state
}

/// 0a --- completion is flat: it offers the root's own entries and
/// never descends into a subdirectory.
#[test]
fn find_file_completion_lists_the_root_only_and_does_not_descend() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::write(td.path().join("alpha.txt"), b"a").expect("write");
    std::fs::create_dir(td.path().join("sub")).expect("mkdir");
    std::fs::write(td.path().join("sub").join("inner.txt"), b"i").expect("write");

    let mut s = editor_in(td.path());
    open_prompt(&mut s);

    let cands = candidates(&s);
    assert!(
        cands.iter().any(|c| c == "alpha.txt"),
        "root entry must be offered; got {cands:?}"
    );
    assert!(
        cands.iter().any(|c| c == "sub"),
        "the subdirectory itself must be offered; got {cands:?}"
    );
    assert!(
        !cands.iter().any(|c| c == "inner.txt"),
        "completion must NOT descend into subdirectories; got {cands:?}"
    );
}

/// 0b --- free text carries the deeper case. `sub/inner.txt`, typed
/// after the prefilled root, reaches `on_accept` as written (D18) and
/// names the file below the root.
#[test]
fn find_file_free_text_opens_a_path_below_the_root() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(td.path().join("sub")).expect("mkdir");
    let inner = td.path().join("sub").join("inner.txt");
    std::fs::write(&inner, b"deep contents\n").expect("write");

    let mut s = editor_in(td.path());
    open_prompt(&mut s);
    type_str(&mut s, "sub/inner.txt");

    // E6.3: the listing follows the field's directory part, so once
    // `sub/` is typed the candidates are sub's entries, and the typed
    // text still wins under `accept = "typed"` (D18).
    assert_eq!(
        candidates(&s),
        vec!["inner.txt".to_string()],
        "a needle containing '/' lists the directory it names"
    );

    press(&mut s, KeyCode::Enter);

    let path = active_path(&s).expect("a file must be open");
    assert_eq!(
        std::fs::canonicalize(&path).expect("canonicalize opened"),
        std::fs::canonicalize(&inner).expect("canonicalize fixture"),
        "free text must open the deeper path"
    );
    let text: String = eval(&s, "return pmacs.window.buffer():slice(0, 13)");
    assert_eq!(text, "deep contents", "the file's real contents must load");
}

/// 0c --- a path that does not exist creates a `[new file]` buffer
/// bound to it, rather than erroring. The typed text is what arrives
/// (D18; see `find_file_typed_text_wins_over_the_selected_candidate`
/// for the case where a candidate is selected).
#[test]
fn find_file_nonexistent_path_creates_a_new_file_buffer() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(td.path().join("sub")).expect("mkdir");
    let fresh = td.path().join("sub").join("brand-new.txt");
    assert!(!fresh.exists(), "fixture must not exist yet");

    let mut s = editor_in(td.path());
    open_prompt(&mut s);
    type_str(&mut s, "sub/brand-new.txt");
    press(&mut s, KeyCode::Enter);

    let path = active_path(&s).expect("a buffer must be bound to the new path");
    assert!(
        path.ends_with("sub/brand-new.txt"),
        "the buffer must be bound to the typed path; got {path}"
    );
    let len: usize = eval(&s, "return pmacs.window.buffer():len()");
    assert_eq!(len, 0, "a new-file buffer starts empty");
    assert!(
        !fresh.exists(),
        "find-file must not create the file on disk --- only the buffer"
    );
    let line = status(&s);
    assert!(
        line.contains("[new file]"),
        "the new-file status must surface; got {line:?}"
    );
}

/// The everyday new-file flow: a BARE name, no separator, matching no
/// existing entry, typed after the prefilled root. The candidate list
/// empties on its own and the typed path arrives.
#[test]
fn find_file_bare_new_name_creates_in_the_root() {
    let td = tempfile::tempdir().expect("tempdir");
    let fresh = td.path().join("zzz.txt");

    let mut s = editor_in(td.path());
    open_prompt(&mut s);
    // "zzz.txt" is not a subsequence of "anchor.txt" (no 'z' in it), so
    // nothing survives the filter and the typed name is what accepts.
    type_str(&mut s, "zzz.txt");
    assert!(
        candidates(&s).is_empty(),
        "fixture premise: a bare non-matching name must empty the list; got {:?}",
        candidates(&s)
    );

    press(&mut s, KeyCode::Enter);

    let path = active_path(&s).expect("a buffer must be bound to the new path");
    assert_eq!(
        std::path::Path::new(&path).parent(),
        Some(td.path()),
        "a bare name must join onto the prompt's root; got {path}"
    );
    assert!(
        path.ends_with("zzz.txt"),
        "the buffer must carry the typed name; got {path}"
    );
    let len: usize = eval(&s, "return pmacs.window.buffer():len()");
    assert_eq!(len, 0, "a new-file buffer starts empty");
    assert!(!fresh.exists(), "nothing is written to disk until save");
}

/// A DIRECTORY named by hand opens in dired (D18), the same route the
/// prefilled root takes: `sub` completed by TAB and accepted by RET is
/// a directory, and a directory is not a file load that fails.
#[test]
fn find_file_accepting_a_directory_opens_it_in_dired() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(td.path().join("sub")).expect("mkdir");

    let mut s = editor_in(td.path());

    open_prompt(&mut s);
    // Only the directory matches: "anchor.txt" contains no 's'.
    type_str(&mut s, "sub");
    assert_eq!(
        candidates(&s),
        vec!["sub".to_string()],
        "fixture premise: the directory must be the sole candidate"
    );
    // The typed name is already the whole candidate, so TAB's third
    // step (E6.3) descends into the directory rather than completing.
    press(&mut s, KeyCode::Tab);
    assert!(
        contents(&s).ends_with("/sub/"),
        "TAB on a complete directory name descends; got {:?}",
        contents(&s)
    );

    press(&mut s, KeyCode::Enter);
    pump(&mut s);

    assert!(
        !eval::<bool>(&s, "return pmacs.minibuffer.is_active()"),
        "the prompt must have closed"
    );
    let name = active_name(&s);
    assert!(
        name.starts_with("*dired:") && name.contains("sub"),
        "a directory must open in dired; the active buffer is {name:?}, status {:?}",
        status(&s)
    );
}

/// 0d, rewritten under D18 (E6.1) --- with no backing path, the prompt
/// roots at the process cwd, the field is PREFILLED with that
/// directory (Emacs's directory-in-the-field, which E6.3's listing
/// makes compatible with completion), and RET on the prefilled
/// directory opens it in dired. The test crate's cwd is the crate
/// root, so `Cargo.toml` is a stable, real candidate there.
#[test]
fn find_file_ret_on_the_prefilled_directory_opens_dired() {
    let mut s = EditorState::new_with_roots(&crate::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    assert!(
        active_path(&s).is_none(),
        "the scratch buffer must have no backing path"
    );

    open_prompt(&mut s);

    let cands = candidates(&s);
    assert!(
        cands.iter().any(|c| c == "Cargo.toml"),
        "a pathless buffer must root the prompt at the process cwd; got {cands:?}"
    );
    let cwd = std::env::current_dir().expect("cwd");
    let field = contents(&s);
    assert_eq!(
        field,
        format!("{}/", cwd.display()),
        "the field is prefilled with the root directory and a trailing slash"
    );

    press(&mut s, KeyCode::Enter);
    pump(&mut s);

    assert!(
        !eval::<bool>(&s, "return pmacs.minibuffer.is_active()"),
        "the prompt must have closed"
    );
    let name = active_name(&s);
    assert!(
        name.starts_with("*dired:"),
        "RET on the prefilled directory must open it in dired; the active buffer is {name:?}, status {:?}",
        status(&s)
    );
    // One buffer per directory, named `*dired:<canonical path>*`.
    let canonical = std::fs::canonicalize(&cwd).expect("canonicalize cwd");
    assert_eq!(
        name,
        format!("*dired:{}*", canonical.display()),
        "the dired buffer must be the cwd's"
    );
}

/// The hole Q#DR11 documented, closed by D18 (E6.1): under
/// `accept = "typed"` the typed text wins over the selected candidate,
/// so a NEW bare name that is a subsequence of an existing entry
/// creates the new file rather than opening the existing one. Before
/// E6.1 this row pinned the opposite as a decision.
#[test]
fn find_file_typed_text_wins_over_the_selected_candidate() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::write(td.path().join("notes.md"), b"existing\n").expect("write");

    let mut s = editor_in(td.path());
    open_prompt(&mut s);
    // "nots" is a subsequence of "notes.md", so the candidate survives
    // the filter and is selected while the user types.
    type_str(&mut s, "nots");
    assert_eq!(
        candidates(&s),
        vec!["notes.md".to_string()],
        "the fixture depends on 'nots' matching 'notes.md'"
    );

    press(&mut s, KeyCode::Enter);

    let path = active_path(&s).expect("a buffer must be bound");
    assert!(
        path.ends_with("/nots"),
        "the typed text wins: a new file named as typed; got {path}"
    );
    assert_eq!(
        std::path::Path::new(&path).parent(),
        Some(td.path()),
        "the new file lives in the prefilled root"
    );
    let len: usize = eval(&s, "return pmacs.window.buffer():len()");
    assert_eq!(len, 0, "a new-file buffer starts empty");
    assert!(
        status(&s).contains("[new file]"),
        "the new-file status must surface; got {:?}",
        status(&s)
    );
}

/// A leading `~` is expanded before the path reaches the core. This
/// matters because `get_or_load_buffer` normalizes the path it STORES
/// but loads from the RAW one, so an unexpanded `~/...` would dedup
/// against an open buffer yet fail to load a file that is not open.
#[test]
fn find_file_expands_a_leading_tilde() {
    let Some(home) = std::env::var_os("HOME") else {
        eprintln!("HOME unset; skipping tilde expansion pin");
        return;
    };
    let home = home.to_string_lossy().into_owned();
    if home.is_empty() || !std::path::Path::new(&home).is_dir() {
        eprintln!("HOME is not a usable directory; skipping");
        return;
    }

    let mut s = EditorState::new_with_roots(&crate::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    open_prompt(&mut s);
    // The field starts as the prefilled cwd; a user who wants `~/...`
    // erases it first. The leaf does not exist, so this lands on the
    // new-file path and touches no disk state.
    clear_field(&mut s);
    type_str(&mut s, "~/pmacs-find-file-tilde-probe.txt");
    press(&mut s, KeyCode::Enter);

    let path = active_path(&s).expect("a buffer must be bound");
    assert!(
        !path.contains('~'),
        "the tilde must be expanded, not passed through; got {path}"
    );
    assert!(
        path.starts_with(&home),
        "the expansion must use $HOME; got {path} with HOME={home}"
    );
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
