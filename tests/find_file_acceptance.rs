// tests/find_file_acceptance.rs --- dired arc Stage 0 (`C-x C-f`) acceptance.

//! Acceptance for `find-file`, the dired arc's Stage 0
//! (`docs/dired-framing.md` §14, items 0a-0d, Q#DR11).
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

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

/// An editor whose active buffer is a real file inside `dir`, so
/// find-file's root resolves to that directory.
fn editor_in(dir: &std::path::Path) -> EditorState {
    let anchor = dir.join("anchor.txt");
    std::fs::write(&anchor, b"anchor\n").expect("write anchor");
    let state = EditorState::new();
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

/// 0b --- free text carries the deeper case. `sub/inner.txt` matches no
/// bare-basename candidate, so it reaches `on_accept` verbatim and is
/// joined onto the prompt's root.
#[test]
fn find_file_free_text_opens_a_path_below_the_root() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(td.path().join("sub")).expect("mkdir");
    let inner = td.path().join("sub").join("inner.txt");
    std::fs::write(&inner, b"deep contents\n").expect("write");

    let mut s = editor_in(td.path());
    open_prompt(&mut s);
    type_str(&mut s, "sub/inner.txt");

    assert!(
        candidates(&s).is_empty(),
        "a needle containing '/' must filter every basename candidate away, \
         or the selection would shadow the typed text"
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
/// bound to it, rather than erroring. The name contains a `/` so the
/// candidate list is empty and the typed text is what arrives (see
/// `find_file_selected_candidate_shadows_typed_text` for the other
/// half of that rule).
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
/// existing entry. The candidate list empties on its own, so the typed
/// text arrives and joins onto the root. This is the path users hit
/// first, and it is the only route through `find_file_resolve` that
/// combines free text with a relative join.
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

/// The failure arm. Accepting a DIRECTORY candidate reaches
/// `display_file`, whose load fails (opening a directory succeeds, the
/// read does not), and the command's `pcall` must turn that into a
/// status message rather than letting the error escape mid-dispatch.
/// Without the `pcall` this test fails, which is the point --- the
/// guard is pinned through the real accept path, not asserted directly.
#[test]
fn find_file_accepting_a_directory_reports_instead_of_raising() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(td.path().join("sub")).expect("mkdir");

    let mut s = editor_in(td.path());
    let before = active_path(&s).expect("the anchor must be open");

    open_prompt(&mut s);
    // Only the directory matches: "anchor.txt" contains no 's'.
    type_str(&mut s, "sub");
    assert_eq!(
        candidates(&s),
        vec!["sub".to_string()],
        "fixture premise: the directory must be the sole candidate"
    );

    press(&mut s, KeyCode::Enter);

    let line = status(&s);
    assert!(
        line.starts_with("find-file: "),
        "the failure must surface as this command's status message; got {line:?}"
    );
    assert_eq!(
        active_path(&s).as_deref(),
        Some(before.as_str()),
        "a failed open must leave the active buffer alone"
    );
    assert!(
        !eval::<bool>(&s, "return pmacs.minibuffer.is_active()"),
        "the prompt must have closed even though the open failed"
    );
}

/// 0d --- with no backing path, the prompt roots at the process cwd
/// (`source_root` is omitted, and the Rust side defaults to ".").
/// The test crate's cwd is the crate root, so `Cargo.toml` is a
/// stable, real candidate there.
#[test]
fn find_file_without_a_backing_path_roots_at_the_process_cwd() {
    let mut s = EditorState::new();
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
    // The field must start EMPTY. Any prefill (e.g. Emacs's
    // directory-in-the-field) would contain a `/`, which filters every
    // basename candidate away and silently disables completion --- the
    // reason the root is named in the prompt string instead.
    let typed: String = eval(&s, "return pmacs.minibuffer.contents()");
    assert_eq!(
        typed, "",
        "the prompt field must start empty or completion is dead on arrival"
    );
}

/// The documented hole in Q#DR11, pinned so it is a decision rather
/// than an accident: `recompute_candidates` selects index 0 whenever
/// the list is non-empty and `resolve_accepted_value` returns the
/// SELECTED CANDIDATE over the typed text, so typing a new bare name
/// that is a subsequence of an existing entry opens the existing file.
/// Fixing this needs a Rust change to accept semantics, which Stage 0
/// deliberately does not make.
#[test]
fn find_file_selected_candidate_shadows_typed_text() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::write(td.path().join("notes.md"), b"existing\n").expect("write");

    let mut s = editor_in(td.path());
    open_prompt(&mut s);
    // "nots" is a subsequence of "notes.md", so the candidate survives
    // the filter and shadows the typed name.
    type_str(&mut s, "nots");
    assert_eq!(
        candidates(&s),
        vec!["notes.md".to_string()],
        "the fixture depends on 'nots' matching 'notes.md'"
    );

    press(&mut s, KeyCode::Enter);

    let path = active_path(&s).expect("a file must be open");
    assert!(
        path.ends_with("notes.md"),
        "documented behavior: the selected candidate wins over typed text; got {path}"
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

    let mut s = EditorState::new();
    s.lua_host.reopen_init_phase_for_testing();
    open_prompt(&mut s);
    // Contains a '/', so the typed text reaches on_accept verbatim.
    // The leaf does not exist, so this lands on the new-file path and
    // touches no disk state.
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
