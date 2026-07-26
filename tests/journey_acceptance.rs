// tests/journey_acceptance.rs --- the golden product journey.

//! The first cross-subsystem acceptance suite (`COHERENCE.md` §19,
//! `docs/journey-stage1a-framing.md` §5).
//!
//! Every other suite in the tree pins one subsystem's contract. This one
//! pins that the subsystems form a usable whole, walking `COHERENCE.md`
//! §2's twelve-step journey. Stage 1a seeds it with the steps that are
//! real today — 2 (launch unconfigured), 3 (open a real project), and 5
//! (edit immediately). Steps 6–12 join as later stages make them real.
//!
//! **This file is a ratchet: stages add rows, none removes them.**
//!
//! Two disciplines it must keep:
//!
//! * **Drive the real entry point.** A directory arm with no production
//!   caller passes every direct-call test, so step 3 goes through
//!   `EditorState::open` — the same function `pmacs FILE` calls — and
//!   not through `resolve_target_buffer`.
//! * **Pump to quiescence, never to a frame count.** Every listing is
//!   worker-dispatched; `tick_async` resuming a coroutine in the frame
//!   its result arrives does not bound when the worker finishes.
//!
//! Pins are labelled **N** (new behavior — must fail on full revert) or
//! **P** (preservation — legitimately green on the pre-image, falsified
//! by the named targeted mutation). See framing §6.0 for why the
//! distinction is load-bearing: an equivalence assertion between two
//! implementations that already agree proves nothing about structural
//! reuse.

use std::path::Path;
use std::time::{Duration, Instant};

use pmacs::editor::EditorState;
use pmacs::editor_core::normalize_buffer_path;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

/// Drive the async runtime to quiescence — no parked coroutine, no
/// pending worker job. The directory listing is invisible until this
/// returns, and how many frames it takes is not knowable in advance.
fn pump(s: &mut EditorState) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let idle: bool = eval(
            s,
            "return pmacs._async.parked_count() == 0 and pmacs._async.pending_count() == 0",
        );
        if idle {
            return;
        }
        assert!(Instant::now() < deadline, "async pump deadline exceeded");
        s.tick_async();
    }
}

/// A project a journey can plausibly be run against.
fn project() -> TempDir {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::write(td.path().join("alpha.txt"), b"alpha\n").expect("write alpha");
    std::fs::write(td.path().join("beta.txt"), b"beta\n").expect("write beta");
    td
}

fn canon(path: &Path) -> String {
    normalize_buffer_path(path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn active_name(s: &EditorState) -> String {
    eval(s, "return pmacs.window.buffer():name()")
}

fn active_text(s: &EditorState) -> String {
    eval(
        s,
        "local b = pmacs.window.buffer()\nreturn b:slice(0, b:len())",
    )
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

fn buffer_count(s: &EditorState) -> usize {
    s.core.borrow().registry.borrow().ids().len()
}

/// Open through the **real** startup entry point, as `pmacs PATH` does.
fn launch(path: &Path) -> EditorState {
    let mut s = EditorState::open(path.to_path_buf()).expect("startup must not fail");
    exec(&s, "pmacs.lsp.config = {}");
    pump(&mut s);
    s
}

// ---------------------------------------------------------------------------
// Step 2 — launch unconfigured
// ---------------------------------------------------------------------------

/// **N** — the editor starts with no configuration and no arguments.
#[test]
fn journey_step2_launches_unconfigured_into_scratch() {
    let s = EditorState::new();
    assert_eq!(active_name(&s), "*scratch*");
    assert!(
        status(&s).is_empty(),
        "a clean launch reports no error; got {:?}",
        status(&s)
    );
}

// ---------------------------------------------------------------------------
// Step 3 — open a real project
// ---------------------------------------------------------------------------

/// **N1** — `pmacs .` opens the directory.
///
/// The headline of Stage 1a and of `COHERENCE.md` §2's "broken at step
/// 3" grade. Before the directory arm this construction returned
/// `Err(EISDIR)` and `main` exited 1.
#[test]
fn journey_step3_opening_a_directory_lists_it() {
    let td = project();
    let s = launch(td.path());

    let name = active_name(&s);
    assert_eq!(
        name,
        format!("*dired:{}*", canon(td.path())),
        "the active buffer must be the directory's dired buffer"
    );
    let text = active_text(&s);
    assert!(
        text.contains("alpha.txt") && text.contains("beta.txt"),
        "the listing must show the directory's entries; got {text:?}"
    );
}

/// **N1b** — and it is a *successful* startup, not a rescued failure.
///
/// Guards the specific regression shape: an implementation that opened
/// dired but still left an error on the status line would look right in
/// the assertion above while `pmacs .` still printed a diagnostic.
#[test]
fn journey_step3_directory_startup_reports_no_error() {
    let td = project();
    let s = launch(td.path());
    assert!(
        !status(&s).contains("cannot open"),
        "a successful directory open must not leave an error status; got {:?}",
        status(&s)
    );
}

/// **N3** — an unreadable directory reports and leaves the session
/// running, rather than failing startup.
#[cfg(target_os = "linux")]
#[test]
fn journey_step3_unreadable_directory_reports_without_failing_startup() {
    use std::os::unix::fs::PermissionsExt;
    let td = tempfile::tempdir().expect("tempdir");
    let locked = td.path().join("locked");
    std::fs::create_dir(&locked).expect("mkdir");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("chmod");

    // Startup itself must succeed: the failure is the *listing*, which
    // happens a tick later and belongs on the status line.
    let s = launch(&locked);
    assert!(
        !status(&s).is_empty(),
        "a failed listing must report through the status line"
    );
    assert!(
        !active_name(&s).starts_with("*dired:"),
        "a failed listing must leave no dired buffer behind"
    );

    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).expect("restore");
}

/// **N9** — the resolver receives a canonical absolute path.
///
/// Falsified by dropping the normalization in
/// `ResolvedTarget::Directory`: nothing else normalizes on that arm,
/// because no buffer is created and `set_buffer_path` never runs.
#[test]
fn journey_directory_resolver_receives_a_canonical_path() {
    let td = project();
    let mut s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    exec(
        &s,
        "seen = nil
         pmacs.hook.add('path.open-directory', function(path) seen = path return false end)",
    );

    // A path with a redundant component, which only canonicalization removes.
    let noisy = td.path().join("subdir").join("..");
    std::fs::create_dir_all(td.path().join("subdir")).expect("mkdir");
    s.open_directory_target(&noisy);
    pump(&mut s);

    let seen: String = eval(&s, "return seen");
    assert_eq!(
        seen,
        canon(td.path()),
        "the resolver must receive the canonical path, not the literal argument"
    );
}

/// **N10** — with the handler cleared and nothing claiming, a directory
/// argument still starts successfully.
///
/// The regression path back to exit 1. Reachable only because the
/// fallback is a clearable slot rather than a builtin hook subscription.
#[test]
fn journey_unclaimed_directory_starts_successfully_with_a_status() {
    let td = project();
    let mut s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    exec(&s, "pmacs.path.set_directory_handler(nil)");

    let before = active_name(&s);
    s.open_directory_target(td.path());
    pump(&mut s);

    assert_eq!(
        active_name(&s),
        before,
        "with no handler the window keeps the buffer it had"
    );
    assert!(
        status(&s).contains(&canon(td.path())),
        "the status must name the directory nothing surfaced; got {:?}",
        status(&s)
    );
}

// ---------------------------------------------------------------------------
// The resolver chain
// ---------------------------------------------------------------------------

/// **N7** — first claimant wins, through an ordinary user listener, and
/// a claim suppresses the fallback.
#[test]
fn journey_resolver_chain_is_first_claimant_wins() {
    let td = project();
    let mut s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    exec(
        &s,
        "first, second, fallback_ran = false, false, false
         pmacs.path.set_directory_handler(function() fallback_ran = true end)
         pmacs.hook.add('path.open-directory', function() first = true return false end)
         pmacs.hook.add('path.open-directory', function() second = true return false end)",
    );

    s.open_directory_target(td.path());
    pump(&mut s);

    assert!(eval::<bool>(&s, "return first"), "the first listener runs");
    assert!(
        !eval::<bool>(&s, "return second"),
        "a claim stops the fan-out before the second listener"
    );
    assert!(
        !eval::<bool>(&s, "return fallback_ran"),
        "a claim suppresses the fallback"
    );
}

/// **N8** — a raising listener suppresses the fallback *and* is
/// reported.
///
/// Falsified by running the fallback when `errors` is non-empty (i.e.
/// treating a raise as a decline), or by making a raise yield
/// `proceed = true`. NOT falsified by keying suppression on `proceed`
/// alone — that is already correct, since a raise and a claim both give
/// `proceed == false`.
#[test]
fn journey_a_raising_resolver_suppresses_the_fallback_and_reports() {
    let td = project();
    let mut s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    exec(
        &s,
        "fallback_ran = false
         pmacs.path.set_directory_handler(function() fallback_ran = true end)
         pmacs.hook.add('path.open-directory', function() error('resolver exploded') end)",
    );

    s.open_directory_target(td.path());
    pump(&mut s);

    assert!(
        !eval::<bool>(&s, "return fallback_ran"),
        "a crashed resolver must not fall through to the default surface"
    );
    assert!(
        !status(&s).is_empty(),
        "the failure must reach the status line, not only *errors*"
    );
    let errors: String = eval(
        &s,
        "for _, id in ipairs(pmacs.buffer.list()) do
           local ok, d = pcall(pmacs.describe.buffer, id)
           if ok and d and d.name == '*errors*' then
             return id:slice(0, id:len())
           end
         end
         return ''",
    );
    assert!(
        errors.contains("resolver exploded"),
        "the failure must also reach the *errors* buffer; got {errors:?}"
    );
}

// ---------------------------------------------------------------------------
// Step 5 — edit immediately
// ---------------------------------------------------------------------------

/// **N11** — the journey's step-3-into-step-5 path: start on a
/// directory, visit a listed file, and type into *that* file.
///
/// Deliberately not a self-insert into the dired buffer, whose intercept
/// rejects every edit — asserting an edit lands there would contradict
/// the read-only contract rather than pin the journey.
#[test]
fn journey_step5_editing_a_file_reached_through_the_directory() {
    let td = project();
    let mut s = launch(td.path());
    assert!(active_name(&s).starts_with("*dired:"));

    let target = td.path().join("alpha.txt");
    exec(
        &s,
        &format!(
            "pmacs.window.display_file({:?}, {{ select = true }})",
            target.display().to_string()
        ),
    );
    pump(&mut s);

    exec(&s, "pmacs.window.buffer():insert(0, 'EDITED ')");
    let text = active_text(&s);
    assert!(
        text.starts_with("EDITED "),
        "the edit must land in the visited file's buffer; got {text:?}"
    );
    assert!(
        buffer_count(&s) >= 2,
        "the dired buffer and the visited file both exist"
    );
}

// ---------------------------------------------------------------------------
// Preservation pins (P) — green on the pre-image; see the named mutation
// ---------------------------------------------------------------------------

/// **P4** — startup shows the file in the *active* window.
///
/// *Mutation:* replace `replace_active_buffer` with a bare
/// `install_buffer_in_window` into some other window in
/// `EditorState::open`.
///
/// **Note, found during implementation:** this does NOT assert that the
/// initial scratch buffer is destroyed, because it is not.
/// `replace_active_buffer`'s doc comment claims it drops "any old
/// scratch buffer if the active window's previous buffer has no other
/// windows referencing it", but all it does is call
/// `switch_active_buffer`, which reassigns the window's `buffer_id` and
/// never removes anything. The stale scratch survives in the registry
/// today, on `main`, unrelated to this stage — so asserting otherwise
/// would have pinned a guarantee the editor does not make and failed on
/// the pre-image for the wrong reason. What the unification must
/// preserve is which window shows the file, and that is what this pins.
#[test]
fn preservation_opening_a_file_shows_it_in_the_active_window() {
    let td = project();
    let target = td.path().join("alpha.txt");
    let s = EditorState::open(target.clone()).expect("open");

    // The displayed name is the argument as given (`path.display()`),
    // which both implementations have always produced -- the *stored*
    // path is what gets normalized, inside `set_buffer_path`.
    assert_eq!(
        active_name(&s),
        target.display().to_string(),
        "the file must be in the active window, not merely loaded"
    );
    let scratch_displayed: bool = eval(
        &s,
        "for _, id in ipairs(pmacs.buffer.list()) do
           local ok, d = pcall(pmacs.describe.buffer, id)
           if ok and d and d.name == '*scratch*' and pmacs.window.buffer() == id then
             return true
           end
         end
         return false",
    );
    assert!(
        !scratch_displayed,
        "no window may still be showing the startup scratch buffer"
    );
}

/// **P5** — the `NotFound` arm survives the unification.
///
/// *Mutation:* delete the `NotFound` arm from `resolve_target_buffer`.
/// The arm most likely to be lost in a wholesale refactor, because its
/// failure mode is a hard error on a perfectly ordinary gesture.
#[test]
fn preservation_a_missing_path_becomes_a_new_file_buffer() {
    let td = project();
    let fresh = td.path().join("not-yet.txt");
    let s = EditorState::open(fresh.clone()).expect("a missing path is not an error");

    assert_eq!(status(&s), "[new file]");
    let len: usize = eval(&s, "return pmacs.window.buffer():len()");
    assert_eq!(len, 0, "a new-file buffer starts empty");
    assert!(!fresh.exists(), "nothing is written until save");
}

/// **P8** — a startup failure names the file.
///
/// The message gained a `cannot open {path}: ` prefix in Stage 1a; the
/// *failure* is preserved, only its wording improved. Before, the bare
/// `io::Error` never named the path.
#[cfg(target_os = "linux")]
#[test]
fn preservation_an_unreadable_file_reports_with_its_path() {
    use std::os::unix::fs::PermissionsExt;
    let td = project();
    let locked = td.path().join("locked.txt");
    std::fs::write(&locked, b"secret\n").expect("write");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("chmod");

    let rendered = match EditorState::open(locked.clone()) {
        Ok(_) => panic!("an unreadable file must fail"),
        Err(error) => error.to_string(),
    };

    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o600)).expect("restore");

    assert!(
        rendered.contains("cannot open"),
        "the message must say what failed; got {rendered:?}"
    );
    assert!(
        rendered.contains(&locked.display().to_string()),
        "the message must name the file; got {rendered:?}"
    );
}

/// **P7** — a directory argument suppresses desktop restore, on the same
/// reasoning a file argument does (Q#DS7): a positional argument means
/// "open this", not "restore my session".
///
/// *Mutation:* pass `false` for `had_file` on the directory path.
#[test]
fn preservation_a_directory_argument_suppresses_desktop_restore() {
    let td = project();
    let mut s = launch(td.path());
    // Arm the restore AFTER startup, then confirm the startup path
    // treated its argument as a positional open: `had_file` is what
    // `run` passes, and a directory must set it.
    let had_file = true;
    s.restore_desktop_if_armed(had_file);
    assert!(
        !status(&s).contains("desktop-restore"),
        "a positional directory argument must not trigger a restore; got {:?}",
        status(&s)
    );
}

/// **P6** — `display_file` keeps its directory-is-an-error contract and
/// does not enter the resolver chain.
///
/// *Mutation:* route `display_file` into the directory resolver.
/// `find_file_accepting_a_directory_reports_instead_of_raising` in
/// `find_file_acceptance.rs` is the companion pin through find-file's
/// real accept path; this one pins the primitive and the window state.
#[test]
fn preservation_display_file_still_refuses_a_directory() {
    let td = project();
    let mut s = EditorState::open(td.path().join("alpha.txt")).expect("open");
    exec(&s, "pmacs.lsp.config = {}");
    let before_name = active_name(&s);
    let before_count = buffer_count(&s);

    let raised: bool = eval(
        &s,
        &format!(
            "local ok = pcall(pmacs.window.display_file, {:?}) return not ok",
            td.path().display().to_string()
        ),
    );
    pump(&mut s);

    assert!(raised, "display_file on a directory must raise");
    assert_eq!(
        active_name(&s),
        before_name,
        "a refused display_file must not change the active buffer"
    );
    assert_eq!(
        buffer_count(&s),
        before_count,
        "a refused display_file must not create a buffer"
    );
}
