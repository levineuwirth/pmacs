// tests/finder_acceptance.rs --- E4.2: the project-wide file finder.

//! Acceptance for `project.find-file` (`C-x p f`): the listing runs from
//! the project root as a job, honors `.gitignore` through `git ls-files`
//! where a repository exists and falls back to a walk where none does,
//! ranks by the minibuffer's fuzzy scorer with the recentf list first,
//! opens the choice through `find_or_open`, and is cancelled by `C-g`
//! and superseded by the next prompt. The 20k-file timing is a
//! measurement behind `#[ignore]` (D12), not an assertion.
//!
//! Dispatch-driven where the user is: the chord opens the prompt, keys
//! type the needle, RET accepts. The listing is asynchronous, so every
//! test pumps the editor's frame until the finder reports that the
//! listing landed, through the readiness helper rather than a sleep.
//!
//! Fixtures use `.txt` files so no `buffer.after-load` hook spawns a
//! language server, and clamp the project marker walk to the fixture so
//! a stray `/tmp/.git` cannot become the root (runbook §1).

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

#[path = "common/iso.rs"]
mod iso;
#[path = "common/ready.rs"]
mod ready;

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

fn candidates(s: &EditorState) -> Vec<String> {
    eval::<Vec<String>>(s, "return pmacs.minibuffer.candidates()")
}

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

/// An editor whose active buffer is `anchor.txt` inside `dir`, with the
/// marker walk clamped to `dir` so the finder's root is `dir` when it
/// holds a `.git` and `dir` (the file's directory) when it does not.
fn editor_in(dir: &Path) -> EditorState {
    let anchor = dir.join("anchor.txt");
    std::fs::write(&anchor, b"anchor\n").expect("write anchor");
    let state = EditorState::new_with_roots(&iso::roots());
    state.lua_host.reopen_init_phase_for_testing();
    exec(
        &state,
        &format!(
            "pmacs.project.set_search_boundary({:?})",
            dir.display().to_string()
        ),
    );
    exec(
        &state,
        &format!(
            "pmacs.buffer.find_or_open({:?})",
            anchor.display().to_string()
        ),
    );
    state
}

/// Open the finder through its real chord.
fn open_finder(s: &mut EditorState) {
    ctrl(s, 'x');
    press(s, KeyCode::Char('p'));
    press(s, KeyCode::Char('f'));
    assert!(
        eval::<bool>(s, "return pmacs.minibuffer.is_active()"),
        "C-x p f must open a minibuffer prompt"
    );
}

/// Pump the editor's frame until the finder reports its listing landed,
/// returning how it was enumerated.
fn wait_listed(s: &mut EditorState) -> String {
    ready::tick_until(s, "the finder's listing", ready::DEADLINE, |s| {
        let (listing, via): (bool, Option<String>) = eval(
            s,
            "local st = pmacs.finder.status() return st.listing, st.via",
        );
        if !listing && via.is_some() {
            ready::Probe::Ready(via.unwrap_or_default())
        } else {
            ready::Probe::Pending(format!("listing={listing} via={via:?}"))
        }
    })
}

fn git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|e| panic!("git {args:?} failed to spawn: {e}"));
    assert!(status.success(), "git {args:?} returned {status}");
}

/// A repository with a tracked file, an untracked file, an ignored
/// directory and a nested tracked file.
fn repo_fixture(dir: &Path) {
    git(dir, &["init", "-q"]);
    std::fs::write(dir.join(".gitignore"), b"ignored/\n").expect("write .gitignore");
    std::fs::write(dir.join("tracked.txt"), b"t\n").expect("write");
    std::fs::create_dir_all(dir.join("sub").join("deep")).expect("mkdir");
    std::fs::write(dir.join("sub").join("deep").join("inner.txt"), b"i\n").expect("write");
    std::fs::create_dir_all(dir.join("ignored")).expect("mkdir");
    std::fs::write(dir.join("ignored").join("hidden.txt"), b"h\n").expect("write");
    git(
        dir,
        &["add", ".gitignore", "tracked.txt", "sub/deep/inner.txt"],
    );
    std::fs::write(dir.join("untracked.txt"), b"u\n").expect("write");
}

/// In a repository the listing is `git ls-files`: tracked and untracked
/// files are offered, an ignored directory is not, and `.git`'s own
/// contents never appear.
#[test]
fn in_a_repository_the_listing_honors_gitignore_through_git_ls_files() {
    let td = tempfile::tempdir().expect("tempdir");
    repo_fixture(td.path());
    let mut s = editor_in(td.path());
    open_finder(&mut s);
    let via = wait_listed(&mut s);
    assert_eq!(via, "git", "a repository is listed by git ls-files");
    let cands = candidates(&s);
    for expected in [
        "tracked.txt",
        "untracked.txt",
        "sub/deep/inner.txt",
        "anchor.txt",
    ] {
        assert!(
            cands.iter().any(|c| c == expected),
            "{expected} must be offered; got {cands:?}"
        );
    }
    assert!(
        !cands.iter().any(|c| c.starts_with("ignored/")),
        "an ignored directory must not be offered; got {cands:?}"
    );
    assert!(
        !cands.iter().any(|c| c.starts_with(".git/") || c == ".git"),
        "git's own directory must not be offered; got {cands:?}"
    );
}

/// Without a repository the walk lists every regular file under the
/// root, minus `.git` itself, and the prompt names the file's directory
/// as its root.
#[test]
fn without_a_repository_the_walk_lists_every_file() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::write(td.path().join("alpha.txt"), b"a").expect("write");
    std::fs::create_dir_all(td.path().join("sub")).expect("mkdir");
    std::fs::write(td.path().join("sub").join("inner.txt"), b"i").expect("write");
    let mut s = editor_in(td.path());
    open_finder(&mut s);
    let via = wait_listed(&mut s);
    assert_eq!(via, "walk", "no repository means the walk");
    let cands = candidates(&s);
    for expected in ["alpha.txt", "sub/inner.txt", "anchor.txt"] {
        assert!(
            cands.iter().any(|c| c == expected),
            "{expected} must be offered; got {cands:?}"
        );
    }
    let prompt: String = eval(&s, "return pmacs.minibuffer.prompt()");
    assert!(
        prompt.contains(&td.path().display().to_string()),
        "the prompt names the root; got {prompt:?}"
    );
}

/// Typing ranks by the fuzzy scorer, and RET opens the top candidate
/// through `find_or_open`.
#[test]
fn typing_ranks_by_fuzzy_score_and_accept_opens_the_file() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::write(td.path().join("alpha.txt"), b"a").expect("write");
    std::fs::create_dir_all(td.path().join("sub")).expect("mkdir");
    std::fs::write(td.path().join("sub").join("inner.txt"), b"inner\n").expect("write");
    std::fs::write(td.path().join("sub").join("winner.txt"), b"w\n").expect("write");
    let mut s = editor_in(td.path());
    open_finder(&mut s);
    wait_listed(&mut s);
    type_str(&mut s, "inner");
    let cands = candidates(&s);
    assert_eq!(
        cands.first().map(String::as_str),
        Some("sub/inner.txt"),
        "the exact-run match ranks first; got {cands:?}"
    );
    assert!(
        cands.iter().any(|c| c == "sub/winner.txt"),
        "a subsequence match is still offered; got {cands:?}"
    );
    assert!(
        !cands.iter().any(|c| c == "alpha.txt"),
        "a non-match is filtered out; got {cands:?}"
    );
    press(&mut s, KeyCode::Enter);
    assert!(
        !eval::<bool>(&s, "return pmacs.minibuffer.is_active()"),
        "RET closes the prompt"
    );
    let opened = active_path(&s).unwrap_or_default();
    assert!(
        opened.ends_with("sub/inner.txt"),
        "accept opens the top candidate; active path {opened:?}"
    );
}

/// The recentf list sits at the top, in most-recent-first order, ahead
/// of better-scoring files from the listing, and is not duplicated.
#[test]
fn recent_files_sit_at_the_top_and_are_not_duplicated() {
    let td = tempfile::tempdir().expect("tempdir");
    for name in ["aa.txt", "ab.txt", "ac.txt", "zz.txt"] {
        std::fs::write(td.path().join(name), b"x").expect("write");
    }
    let mut s = editor_in(td.path());
    // recentf is inert without a state directory (tests are hermetic),
    // so the list the finder merges is stubbed: the merge is the
    // finder's contract, the recording is recentf's own.
    exec(
        &s,
        &format!(
            "pmacs.recentf.list = function() return {{ {:?}, {:?}, {:?} }} end",
            td.path().join("zz.txt").display().to_string(),
            td.path().join("ac.txt").display().to_string(),
            "/elsewhere/not-under-root.txt",
        ),
    );
    open_finder(&mut s);
    wait_listed(&mut s);
    let cands = candidates(&s);
    assert_eq!(
        &cands[..2],
        ["zz.txt".to_owned(), "ac.txt".to_owned()],
        "recent files lead in MRU order; got {cands:?}"
    );
    assert_eq!(
        cands.iter().filter(|c| *c == "zz.txt").count(),
        1,
        "a recent file is offered once; got {cands:?}"
    );
    assert!(
        !cands.iter().any(|c| c.contains("elsewhere")),
        "a recent file outside the root is not offered; got {cands:?}"
    );
    // Recents that match the needle still lead; those that do not are
    // filtered like anything else.
    type_str(&mut s, "a");
    let cands = candidates(&s);
    assert_eq!(
        cands.first().map(String::as_str),
        Some("ac.txt"),
        "the matching recent leads; got {cands:?}"
    );
    assert!(
        !cands.iter().any(|c| c == "zz.txt"),
        "a recent that misses the needle is filtered; got {cands:?}"
    );
}

/// `C-g` cancels the listing in flight, and a second prompt supersedes
/// the first: the late result of the first generation never fills the
/// second.
#[test]
fn cancel_stops_the_listing_and_a_new_prompt_supersedes_it() {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::write(td.path().join("one.txt"), b"1").expect("write");
    let mut s = editor_in(td.path());
    open_finder(&mut s);
    let generation1: i64 = eval(&s, "return pmacs.finder.status().generation");
    ctrl(&mut s, 'g');
    assert!(
        !eval::<bool>(&s, "return pmacs.minibuffer.is_active()"),
        "C-g closes the prompt"
    );
    assert!(
        !eval::<bool>(&s, "return pmacs.finder.status().listing"),
        "cancel stops the listing synchronously"
    );
    // Whatever the cancelled job still delivers is dropped: the status
    // never reports files for the cancelled generation.
    let stop = Instant::now() + Duration::from_millis(300);
    while Instant::now() < stop {
        s.tick_processes();
        s.tick_async();
        std::thread::sleep(Duration::from_millis(5));
    }
    let (generation, files): (i64, Option<i64>) = eval(
        &s,
        "local st = pmacs.finder.status() return st.generation, st.files",
    );
    assert!(
        generation > generation1,
        "cancel retires the generation so a queued result is dropped: {generation} vs {generation1}"
    );
    assert_eq!(files, None, "a cancelled listing never lands");

    // Two prompts in a row: the second's generation wins and its
    // listing fills the live prompt. The chord cannot reach a second
    // open while the first prompt holds the keys, so the second is the
    // command invoked by name, which is how a script or a package
    // supersedes a live prompt.
    open_finder(&mut s);
    let generation2: i64 = eval(&s, "return pmacs.finder.status().generation");
    exec(&s, "pmacs.command.invoke('project.find-file')");
    let generation3: i64 = eval(&s, "return pmacs.finder.status().generation");
    assert!(
        generation1 < generation2 && generation2 < generation3,
        "each prompt is its own generation: {generation1} {generation2} {generation3}"
    );
    wait_listed(&mut s);
    assert!(
        candidates(&s).iter().any(|c| c == "one.txt"),
        "the live prompt is filled by the live generation"
    );
}

/// The 20k-file measurement (D12): a dated number for the handoff, not
/// a pass/fail. Prints the time from the chord to the first populated
/// candidate list, once by the walk and once by git.
#[test]
#[ignore = "measurement, run by scripts/perf-budgets"]
fn finder_lists_a_20k_file_tree_measured() {
    fn build_tree(dir: &Path) {
        for d in 0..200 {
            let sub = dir.join(format!("dir{d:03}"));
            std::fs::create_dir_all(&sub).expect("mkdir");
            for f in 0..100 {
                std::fs::write(sub.join(format!("file{f:03}.txt")), b"x").expect("write");
            }
        }
    }
    fn measure(s: &mut EditorState) -> (Duration, String, i64) {
        let start = Instant::now();
        open_finder(s);
        let via = wait_listed(s);
        let elapsed = start.elapsed();
        let n: i64 = eval(s, "return pmacs.finder.status().files");
        (elapsed, via, n)
    }
    let walk = tempfile::tempdir().expect("tempdir");
    build_tree(walk.path());
    let mut s = editor_in(walk.path());
    let (t_walk, via_walk, n_walk) = measure(&mut s);
    ctrl(&mut s, 'g');
    let repo = tempfile::tempdir().expect("tempdir");
    build_tree(repo.path());
    git(repo.path(), &["init", "-q"]);
    let mut s = editor_in(repo.path());
    let (t_git, via_git, n_git) = measure(&mut s);
    eprintln!(
        "FINDER-20K: {via_walk} listed {n_walk} files in {:.3} s; {via_git} listed {n_git} files in {:.3} s",
        t_walk.as_secs_f64(),
        t_git.as_secs_f64()
    );
    assert_eq!(via_walk, "walk");
    assert_eq!(via_git, "git");
    assert!(n_walk >= 20_000, "the walk saw the whole tree: {n_walk}");
    assert!(n_git >= 20_000, "git saw the whole tree: {n_git}");
}
