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
//!
//! Two P pins here — P1 and P2 — *also* fail on full revert, since
//! `commit_to` does not exist on the pre-image. They are labelled P
//! because their discriminating falsifier is the named mutation: a
//! revert-only check cannot distinguish "validates" from "validates in
//! time", which is their entire claim. Each says so at its own site.

use std::path::Path;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::buffer::BufferId;
use pmacs::editor::EditorState;
use pmacs::editor_core::normalize_buffer_path;
use pmacs::protocol::FrontendId;
use pmacs::window::{FrontendView, Layout, Window, WindowId};
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

fn type_char(s: &mut EditorState, c: char) {
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::NONE));
}

/// The 0-based line an entry renders on, found by its trailing name
/// column -- the same shape `dired_acceptance` uses.
fn line_of(s: &EditorState, name: &str) -> usize {
    let text = active_text(s);
    for (index, line) in text.lines().enumerate() {
        if line.trim_end().ends_with(name) {
            return index;
        }
    }
    panic!("no listing line for {name:?} in:\n{text}");
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

/// The buffer a window currently shows, or `None` if it is not live.
fn buffer_in(s: &EditorState, window: WindowId) -> Option<BufferId> {
    s.core.borrow().windows.get(&window).map(|w| w.buffer_id)
}

/// The window `LOCAL` currently has selected.
fn local_window(s: &EditorState) -> WindowId {
    s.core
        .borrow()
        .views
        .get(&FrontendId::LOCAL)
        .expect("LOCAL view")
        .active
}

/// Register a second frontend with its own single-window layout,
/// mirroring `build_fresh_frontend_view` (the same helper shape
/// `bottom_panel_stage1_acceptance` uses).
fn attach_frontend(s: &EditorState, fid: FrontendId) -> WindowId {
    let mut core = s.core.borrow_mut();
    let buffer_id = core.active_buffer_id();
    let text_view = {
        let reg = core.registry.borrow();
        pmacs::text_view::TextView::new(reg.get(buffer_id).expect("buffer"))
    };
    let win = WindowId::next();
    core.windows
        .insert(win, Window::new(win, buffer_id, text_view));
    core.register_frontend_view(
        fid,
        FrontendView {
            layout: Layout::single(win),
            active: win,
            fold_projection: true,
            panel_capable: true,
            frame_geometry: None,
            panel_hidden: false,
        },
    );
    win
}

/// Drive the **real** chain far enough to obtain a genuine destination
/// and leave it in the Lua global `dest`.
///
/// The listener claims (returns `false`), so nothing is committed and no
/// fallback runs: what lands in `dest` is exactly the userdata dired
/// would have received, produced by the production capture rather than
/// fabricated. Nothing in the test suite can construct one — that is
/// N6b's whole subject.
fn capture_dest(s: &mut EditorState, dir: &Path) {
    exec(
        s,
        "dest = nil
         pmacs.hook.add('path.open-directory', function(_, d) dest = d return false end)",
    );
    s.open_directory_target(dir);
    pump(s);
    assert!(
        eval::<bool>(s, "return dest ~= nil"),
        "the chain must hand listeners a destination"
    );
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
// The destination commit (`pmacs.window.commit_to`)
// ---------------------------------------------------------------------------
//
// The substrate half of Stage 1a. A directory listing settles a tick or
// more after the request, by which time the ambient frontend, selected
// window, and active buffer may all name something else — so the whole
// post-await commit runs against a destination captured at request time.
//
// `LOCAL` is the requesting frontend throughout, because
// `open_directory_target` is the local-startup seam; the daemon's
// non-`LOCAL` capture is pinned in `src/daemon.rs`, where the production
// caller lives. What varies here is what the *ambient* frontend is doing
// while the commit runs, which is exactly the misrouting the scope
// exists to prevent.

/// The frontend that competes for ambient authority in these tests.
const COMPETITOR: FrontendId = FrontendId(7);

/// **N4** — the commit lands in the *requesting* frontend's window even
/// though another frontend is the one dispatching.
///
/// The blocker's positive half. Falsified by reverting `commit_to` to an
/// ambient display: the file then appears in the competitor's window.
#[test]
fn commit_to_delivers_to_the_requesting_frontend_not_the_ambient_one() {
    let td = project();
    let mut s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    capture_dest(&mut s, td.path());

    let local_win = local_window(&s);
    let other_win = attach_frontend(&s, COMPETITOR);
    let other_before = buffer_in(&s, other_win);

    // The competitor becomes the dispatching frontend while the work is
    // "in flight" — the state a worker completion actually returns to.
    s.core.borrow_mut().active_frontend = COMPETITOR;

    let alpha = td.path().join("alpha.txt").display().to_string();
    exec(
        &s,
        &format!(
            "assert(pmacs.window.commit_to(dest, function()
               pmacs.window.display_file({alpha:?})
             end))"
        ),
    );

    assert_eq!(
        buffer_in(&s, other_win),
        other_before,
        "the competing frontend's window must be untouched"
    );
    s.core.borrow_mut().active_frontend = FrontendId::LOCAL;
    assert_eq!(
        active_name(&s),
        alpha,
        "the commit must land in the requesting frontend's captured window"
    );
    assert_eq!(
        local_window(&s),
        local_win,
        "and in that window, not a new one"
    );
}

/// **N4b** — the scope beats an *interactive origin*, not merely the
/// ambient frontend.
///
/// Found by bite-testing N4: with the `ScopedFrontend` arm deleted from
/// `acting_frontend`, N4 still passed, because `ScopedFrontend::enter`
/// also swaps `core.active_frontend` and the ambient fallback then
/// answers correctly on its own. The arm is load-bearing in exactly one
/// situation — a commit reached from inside an interactive command,
/// where the origin sits *between* the override and the ambient value
/// and would otherwise win. `acting_frontend`'s comment claims that
/// ordering; nothing pinned it.
///
/// Driven through `dispatch_key`, because the interactive origin is
/// established by dispatch and by nothing else — `invoke_interactive`
/// requires a context rather than creating one.
///
/// Falsified by deleting the `ScopedFrontend` arm from
/// `acting_frontend`, or by reordering it after the interactive origin.
#[test]
fn commit_to_outranks_an_interactive_origin() {
    let td = project();
    let mut s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    capture_dest(&mut s, td.path());

    let local_win = local_window(&s);
    let other_win = attach_frontend(&s, COMPETITOR);
    let other_before = buffer_in(&s, other_win);

    let alpha = td.path().join("alpha.txt").display().to_string();
    exec(
        &s,
        &format!(
            "pmacs.command.define {{
               name = 'test.journey-commit',
               description = 'commit to a captured destination from inside a command',
               fn = function()
                 committed = pmacs.window.commit_to(dest, function()
                   pmacs.window.display_file({alpha:?})
                 end)
               end,
             }}
             pmacs.keymap.bind {{ scope = 'global', sequence = 'C-c j',
                                  command = 'test.journey-commit' }}"
        ),
    );

    // The COMPETITOR runs the command, so ITS id is the interactive
    // origin for the whole invocation.
    s.dispatch_key(COMPETITOR, key(KeyCode::Char('c'), KeyModifiers::CONTROL));
    s.dispatch_key(COMPETITOR, key(KeyCode::Char('j'), KeyModifiers::NONE));

    assert!(
        eval::<bool>(&s, "return committed"),
        "the commit must be accepted"
    );
    assert_eq!(
        buffer_in(&s, other_win),
        other_before,
        "the invoking frontend's own window must be untouched"
    );
    s.core.borrow_mut().active_frontend = FrontendId::LOCAL;
    assert_eq!(
        active_name(&s),
        alpha,
        "the commit must land in the captured destination, not the \
         interactive origin's window"
    );
    assert_eq!(local_window(&s), local_win);
}

/// **N6a** — the scope is restored when the callback returns normally.
///
/// Falsified by dropping the guard's restore, or by never swapping
/// `core.active_frontend` in the first place (then `inside` reads the
/// competitor and the assertion fails from the other direction).
#[test]
fn commit_to_scopes_and_restores_on_a_normal_return() {
    let td = project();
    let mut s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    capture_dest(&mut s, td.path());
    attach_frontend(&s, COMPETITOR);
    s.core.borrow_mut().active_frontend = COMPETITOR;

    exec(
        &s,
        "inside, scoped = nil, nil
         assert(pmacs.window.commit_to(dest, function()
           inside = pmacs.frontend.id()
           scoped = pmacs._async._in_commit_scope()
         end))",
    );

    assert_eq!(
        eval::<i64>(&s, "return inside"),
        i64::try_from(FrontendId::LOCAL.0).expect("frontend id"),
        "inside the commit the acting frontend is the requesting one"
    );
    assert!(
        eval::<bool>(&s, "return scoped"),
        "and the commit-scope flag is set while the callback runs"
    );
    assert_eq!(
        s.core.borrow().active_frontend,
        COMPETITOR,
        "the ambient frontend must be restored on return"
    );
    assert!(
        !eval::<bool>(&s, "return pmacs._async._in_commit_scope()"),
        "and the commit-scope flag cleared"
    );
    assert_eq!(
        eval::<i64>(&s, "return pmacs.frontend.id()"),
        i64::try_from(COMPETITOR.0).expect("frontend id"),
        "the Lua-visible frontend must be restored too"
    );
}

/// **N6b (part of N6)** — a raising callback still restores.
///
/// The path that makes the guard RAII rather than a pair of statements:
/// `commit_to` captures the call's result and lets the guard drop before
/// propagating it. Falsified by `?`-propagating the callback's error
/// through the scope, or by restoring on the success path only.
#[test]
fn commit_to_restores_when_the_callback_raises() {
    let td = project();
    let mut s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    capture_dest(&mut s, td.path());
    attach_frontend(&s, COMPETITOR);
    s.core.borrow_mut().active_frontend = COMPETITOR;

    exec(
        &s,
        "local ok, err = pcall(pmacs.window.commit_to, dest, function()
           error('commit exploded')
         end)
         raised = (not ok) and tostring(err) or '<no raise>'",
    );

    assert!(
        eval::<String>(&s, "return raised").contains("commit exploded"),
        "the callback's error must propagate"
    );
    assert_eq!(
        s.core.borrow().active_frontend,
        COMPETITOR,
        "a raising callback must still restore the ambient frontend"
    );
    assert!(
        !eval::<bool>(&s, "return pmacs._async._in_commit_scope()"),
        "and must still clear the commit-scope flag"
    );
}

/// **N6c (part of N6)** — awaiting inside a commit is refused, the
/// refusal names the rule, and the scope is restored anyway.
///
/// A yield would restore the scope while the coroutine is still parked,
/// so the rest of the commit would resume ambient — silently
/// reintroducing exactly the misrouting N4 pins against. Driven inside
/// `pmacs.async`, which is where a real await lives.
///
/// Falsified by dropping the `_in_commit_scope` check from
/// `Handle:await`: the await then succeeds and `refusal` reads
/// `<no raise>`.
#[test]
fn commit_to_refuses_an_await_and_restores() {
    let td = project();
    let mut s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    capture_dest(&mut s, td.path());
    attach_frontend(&s, COMPETITOR);
    s.core.borrow_mut().active_frontend = COMPETITOR;

    exec(
        &s,
        &format!(
            "refusal = nil
             pmacs.async(function()
               local handle = pmacs.fs.read_dir({:?})
               local ok, err = pcall(pmacs.window.commit_to, dest, function()
                 return handle:await()
               end)
               refusal = (not ok) and tostring(err) or '<no raise>'
               -- Drain it OUTSIDE the commit, which is where the refusal
               -- says the await belongs -- and which also settles the job
               -- so the pump can reach quiescence.
               handle:await()
             end)",
            td.path().display().to_string()
        ),
    );
    pump(&mut s);

    let refusal: String = eval(&s, "return refusal");
    assert!(
        refusal.contains("cannot await inside") && refusal.contains("commit_to"),
        "the refusal must name the rule it enforces; got {refusal:?}"
    );
    assert_eq!(
        s.core.borrow().active_frontend,
        COMPETITOR,
        "a refused await must still restore the ambient frontend"
    );
    assert!(
        !eval::<bool>(&s, "return pmacs._async._in_commit_scope()"),
        "and must still clear the commit-scope flag"
    );
}

/// **N6b** — a forged destination is rejected, and the callback never
/// runs.
///
/// A plausible `{frontend, window, buffer}` table is what any Lua could
/// fabricate. Falsified by accepting a table, or by borrowing the
/// userdata after invoking the callback.
#[test]
fn commit_to_refuses_a_forged_destination() {
    let td = project();
    let mut s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    capture_dest(&mut s, td.path());

    let win = eval::<i64>(&s, "return dest:window()");
    exec(
        &s,
        &format!(
            "ran = false
             local ok, err = pcall(pmacs.window.commit_to,
               {{ frontend = 0, window = {win}, buffer = 0 }},
               function() ran = true end)
             rejected = (not ok) and tostring(err) or '<accepted>'"
        ),
    );

    let rejected: String = eval(&s, "return rejected");
    assert!(
        rejected.contains("cannot be constructed from Lua"),
        "a forged table must be rejected by type, not merely fail later; got {rejected:?}"
    );
    assert!(
        !eval::<bool>(&s, "return ran"),
        "a rejected destination must not reach the callback"
    );
}

/// **N6c** — a declining listener cannot redirect the destination.
///
/// The same userdata is handed to every listener in turn. As a table, an
/// earlier listener could rewrite the window and then decline, sending
/// the fallback somewhere the user never asked for. Falsified by passing
/// a shared mutable table.
#[test]
fn a_declining_listener_cannot_redirect_the_destination() {
    let td = project();
    let mut s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    let target = local_window(&s);

    exec(
        &s,
        "seen_first, seen_second, mutation = nil, nil, nil
         pmacs.hook.add('path.open-directory', function(_, d)
           seen_first = d:window()
           -- Try to redirect, then decline. Both halves matter: a
           -- successful mutation with a decline is the attack.
           local ok, err = pcall(function() d.window = 999 end)
           mutation = (not ok) and tostring(err) or '<mutated>'
         end)
         pmacs.hook.add('path.open-directory', function(_, d)
           seen_second = d:window()
         end)",
    );

    s.open_directory_target(td.path());
    pump(&mut s);

    let mutation: String = eval(&s, "return mutation");
    assert!(
        !mutation.contains("<mutated>"),
        "the destination must be read-only; got {mutation:?}"
    );
    let first = eval::<i64>(&s, "return seen_first");
    let second = eval::<i64>(&s, "return seen_second");
    assert_eq!(
        first, second,
        "every listener must see the same, unaltered destination"
    );
    assert_eq!(
        u64::try_from(second).expect("window id"),
        target.raw(),
        "and it must still name the window the editor captured"
    );
    // And the fallback commits THERE, not to whatever the first listener
    // wanted -- the observable the attack was aiming at.
    assert!(
        active_name(&s).starts_with("*dired:"),
        "the declined chain must still fall back to dired"
    );
    assert_eq!(
        buffer_in(&s, target),
        Some(eval::<pmacs::lua_bindings::BufferIdLua>(&s, "return pmacs.window.buffer()").0),
        "in the captured window"
    );
}

// --- the commit's preservation pins ---------------------------------------

/// **P1** — every destination precondition is checked *before* the
/// callback runs, so a failure mutates nothing.
///
/// Four refusals, each asserted the same way: `commit_to` returns
/// `(false, reason)`, the callback never ran, and no buffer was created.
/// Table-driven deliberately — the failure message names which
/// precondition regressed, which four separate near-identical tests
/// would give up in exchange for nothing.
///
/// *Mutation:* move the preflight from before the callback to after it
/// (rev 2's design, which validated at display time). All four fail.
/// *Second mutation, for the dedicated case:* pass `Some(dest.buffer)`
/// instead of `None` to `window_accepts_buffer`. Only that case fails —
/// which is why it is listed separately from the stale-buffer case it
/// otherwise resembles.
///
/// **Also fails on full revert**, since `commit_to` does not exist on the
/// pre-image. It is listed as a P because the discriminating falsifier is
/// the named mutation, not the revert: a revert-only check would not
/// distinguish "validates" from "validates in time".
#[test]
fn preservation_a_failed_precondition_never_reaches_the_callback() {
    // (label, Lua that breaks the precondition, expected reason fragment)
    let cases: [(&str, &str, &str); 4] = [
        (
            "frontend gone",
            // Handled in Rust below: unregistering a view has no Lua surface.
            "",
            "requesting frontend is gone",
        ),
        (
            "window gone",
            "local doomed = dest:window()
             pmacs.window.split_horizontal()
             while pmacs.window.current() == doomed do pmacs.window.focus_next() end
             pmacs.window.close_others()",
            "is gone",
        ),
        (
            "stale buffer",
            "pmacs.window.switch_buffer(pmacs.buffer.create('*usurper*'))",
            "now shows another buffer",
        ),
        (
            "dedicated",
            "pmacs.window.set_params(dest:window(), { dedicated = true })",
            "is dedicated",
        ),
    ];

    for (label, break_it, expected) in cases {
        let td = project();
        let mut s = EditorState::new();
        exec(&s, "pmacs.lsp.config = {}");
        capture_dest(&mut s, td.path());

        if label == "frontend gone" {
            s.core
                .borrow_mut()
                .unregister_frontend_view(FrontendId::LOCAL);
        } else {
            exec(&s, break_it);
        }
        let before = buffer_count(&s);

        exec(
            &s,
            "ran = false
             ok, reason = pmacs.window.commit_to(dest, function() ran = true end)",
        );

        assert!(
            !eval::<bool>(&s, "return ok"),
            "{label}: commit_to must refuse"
        );
        let reason: String = eval(&s, "return tostring(reason)");
        assert!(
            reason.contains(expected),
            "{label}: reason must say why; wanted {expected:?}, got {reason:?}"
        );
        assert!(
            !eval::<bool>(&s, "return ran"),
            "{label}: the callback must not run at all -- validating after it \
             is four mutations too late"
        );
        assert_eq!(
            buffer_count(&s),
            before,
            "{label}: a refused commit must create no buffer"
        );
    }
}

/// **P2 — stale intent loses**, through dired's real commit path.
///
/// The user replaced the destination window's buffer while the listing
/// was in flight. Their action is newer information than the request, so
/// the request loses: dired refuses, their buffer survives, and no dired
/// buffer or handle is left behind for that path.
///
/// P1 pins the preflight in isolation; this drives `pmacs.dired.open`
/// with a captured destination — the same call the handler makes — so
/// the atomicity claim is asserted where the four mutations actually
/// live.
///
/// *Mutation:* drop the `dest.buffer` comparison from the preflight
/// (window-only validation). The dired buffer then replaces the user's.
#[test]
fn preservation_a_stale_destination_loses_to_the_users_newer_buffer() {
    let td = project();
    let mut s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    capture_dest(&mut s, td.path());
    let target = local_window(&s);

    // The user switches the destination window while the work is in flight.
    exec(
        &s,
        "usurper = pmacs.buffer.create('*usurper*')
         pmacs.window.switch_buffer(usurper)",
    );
    let usurper = buffer_in(&s, target);
    let before = buffer_count(&s);

    exec(
        &s,
        &format!(
            "failure = nil
             pmacs.async(function()
               local ok, err = pcall(pmacs.dired.open, {:?}, {{ dest = dest }})
               failure = (not ok) and tostring(err) or '<committed>'
             end)",
            canon(td.path())
        ),
    );
    pump(&mut s);

    let failure: String = eval(&s, "return failure");
    assert!(
        failure.contains("destination is gone"),
        "dired must report the refusal rather than commit; got {failure:?}"
    );
    assert_eq!(
        buffer_in(&s, target),
        usurper,
        "the user's newer buffer must survive"
    );
    assert_eq!(
        buffer_count(&s),
        before,
        "and no dired buffer may be left behind"
    );
    assert_eq!(
        active_name(&s),
        "*usurper*",
        "nor may the refusal change what is displayed"
    );
}

/// **P3** — dired reads its `prev` inside the scope, so `q` returns to
/// the *destination* window's buffer, not the ambient frontend's.
///
/// `handle.prev` is captured with `pmacs.window.buffer()`, whose no-arg
/// arm reads the core's ambient `active_buffer_id()`. That is precisely
/// why the scope swaps `core.active_frontend` and not only the override:
/// a scope that swapped the override alone would leave this one line
/// reading the competitor's buffer, and `q` would drop the user into a
/// buffer from another frontend's window.
///
/// Asserted through `q` rather than by reaching into dired's handle
/// table — `prev`'s entire meaning is where `q` lands.
///
/// *Mutation:* stop swapping `core.active_frontend` in
/// `ScopedFrontend::enter` (keep the override). `q` then lands in
/// `*competitor*`.
#[test]
fn preservation_dired_captures_prev_from_the_destination_not_the_ambient_frontend() {
    let td = project();
    let mut s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");

    let target = local_window(&s);
    let origin = buffer_in(&s, target).expect("the startup buffer");

    // A competitor whose window shows a buffer of its own, ambient while
    // the listing settles.
    let other_win = attach_frontend(&s, COMPETITOR);
    let competitor_buffer =
        eval::<pmacs::lua_bindings::BufferIdLua>(&s, "return pmacs.buffer.create('*competitor*')")
            .0;
    s.core
        .borrow_mut()
        .install_buffer_in_window(other_win, competitor_buffer)
        .expect("install");
    s.core.borrow_mut().active_frontend = COMPETITOR;

    s.open_directory_target(td.path());
    pump(&mut s);
    s.core.borrow_mut().active_frontend = FrontendId::LOCAL;
    assert!(
        active_name(&s).starts_with("*dired:"),
        "the listing must have committed"
    );

    type_char(&mut s, 'q');
    assert_eq!(
        buffer_in(&s, target),
        Some(origin),
        "`q` must return to the buffer the DESTINATION window showed, not \
         the ambient frontend's"
    );
}

// ---------------------------------------------------------------------------
// Step 5 — edit immediately
// ---------------------------------------------------------------------------

/// **N11** — the journey's step-3-into-step-5 path, through the real
/// input path at every step: start on a directory, press `RET` on a
/// listed file, then type a character into it.
///
/// Rev 6 correction: this previously called `display_file` and
/// `buf:insert` directly, so it stayed green with dired's `RET` binding,
/// its entry dispatch, or the editor's self-insert path all broken —
/// which is most of what "the journey works" is supposed to mean. Both
/// gestures are now dispatched as keys.
///
/// Deliberately not a self-insert into the dired buffer, whose intercept
/// rejects every edit: asserting an edit lands there would contradict
/// the read-only contract rather than pin the journey.
#[test]
fn journey_step5_editing_a_file_reached_through_the_directory() {
    let td = project();
    let mut s = launch(td.path());
    assert!(active_name(&s).starts_with("*dired:"));

    // Seat on the entry, then VISIT it with the real key.
    let line = line_of(&s, "alpha.txt");
    exec(&s, &format!("pmacs.editor.move_to_line({line})"));
    press(&mut s, KeyCode::Enter);
    pump(&mut s);

    assert_eq!(
        active_name(&s),
        td.path().join("alpha.txt").display().to_string(),
        "RET on a listed file must visit it"
    );

    // And type into it with the real key.
    type_char(&mut s, 'X');
    let text = active_text(&s);
    assert!(
        text.starts_with('X'),
        "a self-insert must land in the visited file's buffer; got {text:?}"
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

// **P7 — removed in rev 6, not weakened.**
//
// Q#JR12 said a directory argument must suppress desktop restore, and
// rev 5 carried a pin for it. There is nothing to pin. `run` computes
// `had_file = file.is_some()` (`editor.rs:3152`) and a directory path is
// `Some` like any other, so the suppression is structural: no
// directory-specific branch exists that could get it wrong, and the
// named mutation ("pass false for `had_file` on the directory path")
// would require inventing the branch first.
//
// The rev 5 test also never armed desktop restore and hard-coded
// `had_file = true` after startup, so it asserted nothing about `run`'s
// decision and would have passed against any implementation. Keeping a
// green test that cannot fail is worse than having none: it reads as
// coverage. Q#JR12 is downgraded to an observation in the framing.

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
