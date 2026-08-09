// tests/worker_identity_acceptance.rs --- worker identity Stage 1.

//! Worker identity Stage 1 (`docs/worker-identity-framing.md` §6,
//! `COHERENCE.md` §9).
//!
//! §9 grades the worker model **mechanism without identity**: a job
//! carries a `JobKind` naming the builtin dispatcher it funnelled
//! through, so every third-party job renders under a builtin's label,
//! and no progress indicator exists anywhere. This suite pins what
//! Stage 1 does about that — a required `purpose` on the job and the
//! process, the dispatch-name ambient that stops
//! `pmacs.workers.dispatch` discarding its handler name, and the first
//! indicator a user sees without running a command.
//!
//! # What is NOT here, deliberately
//!
//! **Presence is enforced by the COMPILER, not by anything below.**
//! `JobSpec::purpose` is non-optional and `JobSpec` has no `Default`, so
//! a dispatcher that supplies none does not build. A funnel test would
//! prove only that the funnel stores what it was handed, and would say
//! nothing about whether fourteen callers handed it anything meaningful.
//! Everything below is about *semantics*.
//!
//! **That a raw `coroutine.yield` inside either dynamic scope is
//! prevented — it is not.** R46 forbids package code from yielding
//! raw, but it is a convention, and the scheduler diagnoses a non-Handle
//! yield only *after* the coroutine has suspended (`async.lua` resumes,
//! then inspects what came back), so no refusal sited in a yield helper
//! is ever consulted. Rule 1 claims **the two supported yield APIs** and
//! nothing more. A test that "proved" coverage this design does not have
//! would be worse than the recorded gap, so the gap is recorded instead
//! (framing §2, §6, §7).
//!
//! **That background work is attributable from one place** (Stage 2's
//! unified view), **that a terminal PTY is visible anywhere** (Q#W-4),
//! or **that any job is attributed to the PACKAGE responsible for it**.
//! `purpose` records what work is being done and, under
//! `pmacs.workers.dispatch`, which registered handler it ran under.
//! Neither is package ownership, which waits for P3 — and there is no
//! `owner` field, in any spelling, for it to squat on (framing §3, §7).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use pmacs::async_runtime::JobKind;
use pmacs::cell::{Cell, CellGrid, CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;
use pmacs::statusline::{
    StatuslineEvaluationOutcome, StatuslineEvaluationTarget, StatuslineProviderId,
    evaluate_statusline,
};

#[path = "common/iso.rs"]
mod iso;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn exec(state: &EditorState, source: &str) {
    state.lua_host.lua().load(source.to_owned()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(state: &EditorState, source: &str) -> T {
    state.lua_host.lua().load(source.to_owned()).eval().unwrap()
}

fn editor() -> EditorState {
    let state = EditorState::new_with_roots(&iso::roots());
    exec(&state, "pmacs.lsp.config = {}");
    state
}

/// Drive the async runtime until nothing is in flight and no coroutine
/// is parked. How many frames that takes is not knowable in advance, so
/// this never counts them.
///
/// Quiescence is measured as **no `Running` job**, not as an empty
/// pending table. Most jobs here are dispatched and never awaited —
/// that is the shape the indicator exists to describe — and a settled
/// entry stays in the pending table until someone takes its result, so
/// `pending_count() == 0` would never come true.
fn pump(state: &mut EditorState) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let idle: bool = eval(
            state,
            "return pmacs._async.parked_count() == 0
                and #pmacs.workers.snapshot().active == 0",
        );
        if idle {
            return;
        }
        assert!(Instant::now() < deadline, "async pump deadline exceeded");
        state.tick_async();
    }
}

/// The purposes of every job the runtime currently has in flight.
///
/// Read through the **Lua** snapshot surface, which is what `*workers*`
/// and any package consume, rather than through the Rust struct.
fn active_purposes(state: &EditorState) -> Vec<String> {
    eval(
        state,
        "local out = {}
         for _, job in ipairs(pmacs.workers.snapshot().active) do
           out[#out + 1] = job.purpose
         end
         return out",
    )
}

fn paint(state: &EditorState, rows: u32, cols: u32) -> Vec<Cell> {
    let mut cells = vec![Cell::default(); (rows * cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: cols,
        size: CellSize::new(rows, cols),
    };
    let _ = pmacs::editor::paint_frame(
        state,
        FrontendId::LOCAL,
        &HashMap::new(),
        &mut grid,
        CellSize::new(rows, cols),
    );
    cells
}

fn row_text(cells: &[Cell], cols: u32, row: u32) -> String {
    (0..cols)
        .map(
            |column| match &cells[(row * cols + column) as usize].glyph {
                Glyph::Char(ch) => *ch,
                Glyph::Cluster(bytes) => std::str::from_utf8(bytes)
                    .ok()
                    .and_then(|text| text.chars().next())
                    .unwrap_or(' '),
                Glyph::Continuation => ' ',
            },
        )
        .collect()
}

/// The registration handle of the builtin activity provider.
fn activity_provider(state: &EditorState) -> StatuslineProviderId {
    state
        .statusline_registry
        .borrow()
        .providers()
        .into_iter()
        .find(|provider| provider.name == "activity")
        .expect("builtin activity provider")
        .id
}

/// The activity provider's segment for `LOCAL`'s only window, or `None`
/// when it produced **no segment at all**.
///
/// `Option<String>`, never `String`, is the whole point of this helper:
/// "absent" and "empty" must be distinguishable, because a zero-width
/// segment still consumes a separator in the composed modeline.
fn activity_segment(state: &EditorState) -> Option<String> {
    let id = activity_provider(state);
    let evaluation = evaluate_statusline(
        state.lua_host.lua(),
        &state.core,
        &state.statusline_registry,
        StatuslineEvaluationTarget::Grid {
            frontend_id: FrontendId::LOCAL,
        },
    );
    let StatuslineEvaluationOutcome::Ready(windows) = evaluation.outcome else {
        panic!("statusline evaluation must be ready in a single-window editor");
    };
    windows
        .iter()
        .flat_map(|window| window.left.iter().chain(window.right.iter()))
        .find(|segment| segment.provider_id == id)
        .map(|segment| segment.text.clone())
}

/// Dispatch one job that will still be **in flight** when the caller
/// looks, without sleeping.
///
/// A pending entry leaves `Running` only inside `AsyncRuntime::tick`, so
/// a dispatch with no intervening tick is in flight by construction —
/// no wall-clock race, and no worker left sleeping past the test.
fn dispatch_one_in_flight(state: &EditorState) {
    exec(state, "IN_FLIGHT = pmacs.workers.sleep(50)");
}

// ---------------------------------------------------------------------------
// 1 — purpose reaches the three structurally distinct entry paths
// ---------------------------------------------------------------------------

/// One per distinct **shape**, not one per dispatcher: a pool
/// dispatcher, an `register_external` job, and a spawned process.
///
/// `register_external` is here because MCP and LSP bypass the worker
/// pool entirely — they are the likeliest paths for a later field to be
/// added to `PendingJob` and quietly missed — and because its `JobKind`
/// is the undifferentiated `LspRequest`/`McpRequest` for every method,
/// so `purpose` is the only thing that tells two of its rows apart.
#[test]
fn every_entry_shape_records_what_its_work_is() {
    let mut state = editor();

    // (a) A pool dispatcher.
    dispatch_one_in_flight(&state);
    let purposes = active_purposes(&state);
    assert_eq!(purposes.len(), 1, "one job in flight: {purposes:?}");
    assert_eq!(
        purposes[0], "sleep 50ms",
        "a pool job records the work, not just its handler's name"
    );

    // (b) An externally-settled job. The purpose is a PARAMETER here
    // because `register_external` has nothing to derive one from: its
    // kind is a category, not a description.
    let (job_id, _token) = state.async_runtime.register_external(
        JobKind::LspRequest,
        None,
        "lsp textDocument/definition file:///tmp/x.rs",
    );
    let purposes = active_purposes(&state);
    assert!(
        purposes
            .iter()
            .any(|p| p == "lsp textDocument/definition file:///tmp/x.rs"),
        "an externally-registered job carries its caller's description: {purposes:?}"
    );
    state.async_runtime.complete_external_cancelled(job_id);

    // (c) A spawned process. `label` keeps its existing meaning and its
    // existing callers; `purpose` is the new, separate answer to "what
    // is this doing".
    exec(
        &state,
        r#"P = pmacs.process.spawn {
             label = "sh-1",
             purpose = "probing the repository for a build system",
             command = "/bin/sh",
             args = { "-c", "sleep 5" },
           }"#,
    );
    let rows: Vec<String> = eval(
        &state,
        "local out = {}
         for _, row in ipairs(pmacs.process.list()) do
           out[#out + 1] = row.label .. ' | ' .. row.purpose
         end
         return out",
    );
    assert!(
        rows.iter()
            .any(|row| row == "sh-1 | probing the repository for a build system"),
        "a spawned process carries a purpose ALONGSIDE its label: {rows:?}"
    );
    exec(&state, "pmacs.process.terminate(P)");

    pump(&mut state);
}

/// A `pmacs.process.spawn` caller that supplies no purpose keeps
/// working, and gets its own label back rather than an empty field.
///
/// The Rust struct's field is required — the compiler enforces that at
/// every construction site. This surface is deliberately lenient,
/// because requiring it here would break every existing caller for no
/// coverage the compiler is not already providing.
#[test]
fn a_process_spawned_without_a_purpose_falls_back_to_its_label() {
    let mut state = editor();
    exec(
        &state,
        r#"P = pmacs.process.spawn {
             label = "legacy-caller",
             command = "/bin/sh",
             args = { "-c", "sleep 5" },
           }"#,
    );
    let purpose: String = eval(
        &state,
        "for _, row in ipairs(pmacs.process.list()) do
           if row.label == 'legacy-caller' then return row.purpose end
         end
         return '<absent>'",
    );
    assert_eq!(purpose, "legacy-caller");
    exec(&state, "pmacs.process.terminate(P)");
    pump(&mut state);
}

/// Q#W-4's preservation half, pinned here as well as by the three
/// leak-detector suites: `purpose` is a new KEY on each existing row and
/// changes nothing about **which** processes `list()` enumerates.
///
/// `m6_8_multi_repl_acceptance`, `compile_mode_acceptance` and
/// `lean4_stage1_acceptance` all assert on `#pmacs.process.list()` as a
/// leak baseline. If any of them needs editing, the design is wrong.
#[test]
fn process_list_still_hides_terminal_ptys() {
    let mut state = editor();
    let before: usize = eval(&state, "return #pmacs.process.list()");
    exec(
        &state,
        "T = pmacs.terminal.open { command = '/bin/sh', args = { '-c', 'sleep 5' } }",
    );
    let after: usize = eval(&state, "return #pmacs.process.list()");
    assert_eq!(
        before, after,
        "a terminal PTY must stay invisible to pmacs.process.list (Q#W-4)"
    );
    assert!(
        eval::<bool>(&state, "return pmacs.terminal.is_terminal(T)"),
        "precondition: the PTY really was opened"
    );
    // No explicit close: terminals have no Lua teardown surface, and
    // `EditorState::drop` shuts the supervisor down with SIGTERM then
    // SIGKILL, so the child cannot outlive the test.
    pump(&mut state);
}

// ---------------------------------------------------------------------------
// 2 — the dispatch-name ambient (Q#W-2)
// ---------------------------------------------------------------------------

/// **Rule 7 + the defect itself.** A job dispatched through
/// `pmacs.workers.dispatch("name", …)` reports `"name"`.
///
/// The witness is a handler **registered from Lua that calls a real
/// dispatcher**, not a synthetic push of the ambient. A test that
/// pushed the name by hand would prove the stack works and leave the
/// actual defect — `name` dying inside an arbitrary handler, three
/// layers above anything that takes a name — completely unwitnessed.
#[test]
fn a_dispatched_job_reports_the_registered_handler_name() {
    let mut state = editor();
    exec(
        &state,
        "pmacs.workers.register('indexer', function()
           return pmacs.workers.sleep(50)
         end)
         H = pmacs.workers.dispatch('indexer')",
    );
    let purposes = active_purposes(&state);
    assert_eq!(purposes.len(), 1, "one job in flight: {purposes:?}");
    assert!(
        purposes[0].starts_with("indexer"),
        "the third party's own name must survive the call chain: {purposes:?}"
    );

    // Rule 7: outside any extent, nothing changes.
    exec(&state, "DIRECT = pmacs.workers.sleep(50)");
    let purposes = active_purposes(&state);
    assert!(
        purposes.iter().any(|p| p == "sleep 50ms"),
        "a builtin invoked directly records its own purpose: {purposes:?}"
    );
    pump(&mut state);
}

/// **Rule 6 — COMPOSE, do not replace.** Both halves asserted, because
/// a test on the prefix alone passes when the description is dropped,
/// and a test on the description alone passes when the third party is
/// lost again.
#[test]
fn a_dispatched_job_composes_the_handler_name_with_the_work() {
    let mut state = editor();
    exec(
        &state,
        "pmacs.workers.register('indexer', function()
           return pmacs.workers.sleep(50)
         end)
         H = pmacs.workers.dispatch('indexer')",
    );
    let purposes = active_purposes(&state);
    assert_eq!(
        purposes,
        vec!["indexer: sleep 50ms".to_owned()],
        "letting the name win discards the work; letting the work win \
         loses the third party"
    );
    pump(&mut state);
}

/// **Rules 3 and 4 — nesting is a stack (innermost wins) and fan-out
/// shares the name.**
#[test]
fn nesting_takes_the_innermost_name_and_fan_out_shares_it() {
    let mut state = editor();
    exec(
        &state,
        "pmacs.workers.register('inner', function()
           -- Fan-out: two jobs under one handler.
           A = pmacs.workers.sleep(50)
           B = pmacs.workers.sleep(51)
           return A
         end)
         pmacs.workers.register('outer', function()
           pmacs.workers.dispatch('inner')
           -- Back in `outer`'s extent: the stack restored on return.
           C = pmacs.workers.sleep(52)
           return C
         end)
         pmacs.workers.dispatch('outer')",
    );
    let mut purposes = active_purposes(&state);
    purposes.sort();
    assert_eq!(
        purposes,
        vec![
            "inner: sleep 50ms".to_owned(),
            "inner: sleep 51ms".to_owned(),
            "outer: sleep 52ms".to_owned(),
        ],
        "innermost wins inside, and the outer name is restored after"
    );
    pump(&mut state);
}

/// **Rule 5 — unwind-safe, and this is the one that makes a naive
/// version worse than none.**
///
/// A handler that raises must still pop. Otherwise one failure poisons
/// every subsequent dispatch in the session with a stale name, and the
/// feature stops failing loudly and starts lying silently — a
/// regression that would surface as intermittent misattribution long
/// after the lane landed.
#[test]
fn a_raising_handler_still_pops_its_name() {
    let mut state = editor();
    exec(
        &state,
        "pmacs.workers.register('boom', function() error('handler failed') end)
         OK, ERR = pcall(pmacs.workers.dispatch, 'boom')",
    );
    assert!(
        !eval::<bool>(&state, "return OK"),
        "the handler's error must still reach the caller"
    );
    assert!(
        eval::<String>(&state, "return tostring(ERR)").contains("handler failed"),
        "and must reach it unchanged"
    );

    exec(&state, "LATER = pmacs.workers.sleep(50)");
    let purposes = active_purposes(&state);
    assert_eq!(
        purposes,
        vec!["sleep 50ms".to_owned()],
        "an unrelated later dispatch must not inherit the failed \
         handler's name: {purposes:?}"
    );
    pump(&mut state);
}

/// **Preservation.** `pmacs.workers.dispatch` was `return
/// handler(args, opts)` — a tail call that propagates **every** return
/// value. Bracketing it must not quietly truncate that.
///
/// A `local ok, result = pcall(...)` bracketing would pass every other
/// test in this file and lose a two-value handler's second value with no
/// error anywhere, which is the shape of regression that surfaces months
/// later in somebody else's package.
#[test]
fn dispatch_still_propagates_every_value_the_handler_returns() {
    let mut state = editor();
    let values: Vec<String> = eval(
        &state,
        "pmacs.workers.register('multi', function()
           return pmacs.workers.sleep(50), 'second', 'third'
         end)
         local a, b, c = pmacs.workers.dispatch('multi')
         return { type(a), tostring(b), tostring(c) }",
    );
    assert_eq!(
        values,
        vec!["table".to_owned(), "second".to_owned(), "third".to_owned()],
        "a multi-value handler must survive the bracketing"
    );
    pump(&mut state);
}

/// **Rule 2 — work dispatched LATER is not covered, deliberately.**
///
/// A job dispatched from an `on_complete` callback runs ticks later,
/// outside the extent, and carries only its own purpose. Asserted so
/// that the boundary reads as designed rather than as broken; covering
/// it would need the asynchronous lifetime mechanism Stage 3 owns
/// (Q#W-5).
#[test]
fn work_dispatched_from_a_completion_callback_carries_no_handler_name() {
    let mut state = editor();
    exec(
        &state,
        "LATE = nil
         pmacs.workers.register('deferred', function()
           local h = pmacs.workers.sleep(1)
           h:on_complete(function()
             LATE = pmacs.workers.sleep(50)
           end)
           return h
         end)
         pmacs.workers.dispatch('deferred')",
    );
    // One tick settles the first job and fires the callback; the job the
    // callback dispatches is what this test is about, so do not pump to
    // quiescence before reading it.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !eval::<bool>(&state, "return LATE ~= nil") {
        assert!(Instant::now() < deadline, "callback never fired");
        state.tick_async();
    }
    let purposes = active_purposes(&state);
    assert_eq!(
        purposes,
        vec!["sleep 50ms".to_owned()],
        "the extent is the handler CALL, not the job's lifetime: {purposes:?}"
    );
    pump(&mut state);
}

// ---------------------------------------------------------------------------
// 3 — rule 1: the extent is non-yieldable, and that is ENFORCED
// ---------------------------------------------------------------------------

/// **Rule 1, first supported yield API.** Two assertions, and the
/// second is the load-bearing one.
///
/// A guard that raises but leaves the name pushed has converted a silent
/// misattribution into a silent misattribution *plus* an error. So the
/// witness dispatches again after the rejection and asserts the new job
/// carries no stale name.
#[test]
fn awaiting_inside_a_handler_is_refused_and_the_scope_restores() {
    let mut state = editor();
    // The awaited handle is created OUTSIDE the extent on purpose: the
    // second assertion below is about what a job allocated *after* the
    // refusal carries, and a job the handler allocated for itself would
    // legitimately wear the handler's name and blur that.
    exec(
        &state,
        "OUTSIDE = pmacs.workers.sleep(1)
         REFUSAL = nil
         pmacs.workers.register('awaits', function()
           local ok, err = pcall(function() return OUTSIDE:await() end)
           REFUSAL = (not ok) and tostring(err) or '<no raise>'
           return OUTSIDE
         end)
         pmacs.async(function() pmacs.workers.dispatch('awaits') end)",
    );
    let refusal: String = eval(&state, "return REFUSAL");
    assert!(
        refusal.contains("cannot await inside") && refusal.contains("pmacs.workers.dispatch"),
        "the refusal must name the rule it enforces; got {refusal:?}"
    );
    assert!(
        !eval::<bool>(&state, "return pmacs._async._in_dispatch_name_scope()"),
        "a refused await must still leave the scope popped"
    );

    exec(&state, "AFTER = pmacs.workers.sleep(50)");
    let purposes = active_purposes(&state);
    assert!(
        purposes.iter().any(|p| p == "sleep 50ms"),
        "and a later dispatch must carry no stale name: {purposes:?}"
    );
    assert!(
        !purposes.iter().any(|p| p.starts_with("awaits:")),
        "no job allocated after the refusal may inherit the handler's \
         name: {purposes:?}"
    );
    pump(&mut state);
}

/// **Rule 1, unconditionally.** The refusal fires even when the awaited
/// handle has already settled.
///
/// This is the case that separates an unconditional guard from one whose
/// behaviour depends on a race: a guard placed after the `_is_complete`
/// check would fire only when a yield would really occur, passing under
/// test and failing intermittently in production depending on whether
/// the job happened to finish first.
#[test]
fn the_await_refusal_fires_even_for_an_already_complete_handle() {
    let mut state = editor();
    exec(&state, "SETTLED = pmacs.workers.sleep(0)");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !eval::<bool>(&state, "return SETTLED:is_complete()") {
        assert!(Instant::now() < deadline, "the canary never settled");
        state.tick_async();
    }

    exec(
        &state,
        "REFUSAL = nil
         pmacs.workers.register('awaits-settled', function()
           local ok, err = pcall(function() return SETTLED:await() end)
           REFUSAL = (not ok) and tostring(err) or '<no raise>'
           return pmacs.workers.sleep(50)
         end)
         pmacs.workers.dispatch('awaits-settled')",
    );
    let refusal: String = eval(&state, "return REFUSAL");
    assert!(
        refusal.contains("cannot await inside") && refusal.contains("pmacs.workers.dispatch"),
        "a settled handle must be refused too, or the guard's behaviour \
         depends on a race; got {refusal:?}"
    );
    assert!(
        !eval::<bool>(&state, "return pmacs._async._in_dispatch_name_scope()"),
        "and the scope must still be popped"
    );
    pump(&mut state);
}

/// **Rule 1, second supported yield API.** Guarding `:await()` and not
/// `yield_to_next_tick` would leave the extent open through a second
/// door — and Q#W-7 below is the proof that exactly that happens when
/// only one door is guarded.
#[test]
fn yield_to_next_tick_inside_a_handler_is_refused_and_the_scope_restores() {
    let mut state = editor();
    exec(
        &state,
        "REFUSAL = nil
         pmacs.workers.register('yields', function()
           local ok, err = pcall(pmacs.async.yield_to_next_tick)
           REFUSAL = (not ok) and tostring(err) or '<no raise>'
           return pmacs.workers.sleep(50)
         end)
         pmacs.async(function() pmacs.workers.dispatch('yields') end)",
    );
    let refusal: String = eval(&state, "return REFUSAL");
    assert!(
        refusal.contains("cannot yield inside") && refusal.contains("pmacs.workers.dispatch"),
        "the second yield API must refuse too; got {refusal:?}"
    );
    assert!(
        !eval::<bool>(&state, "return pmacs._async._in_dispatch_name_scope()"),
        "and must leave the scope popped"
    );

    exec(&state, "AFTER = pmacs.workers.sleep(51)");
    let purposes = active_purposes(&state);
    assert!(
        purposes.iter().any(|p| p == "sleep 51ms"),
        "a later dispatch must carry no stale name: {purposes:?}"
    );
    pump(&mut state);
}

// ---------------------------------------------------------------------------
// 4 — Q#W-7: the same hole in `commit_to`, closed here
// ---------------------------------------------------------------------------

/// **Q#W-7 — a pre-existing defect, found by reading and repaired in
/// this lane.**
///
/// `Handle:await()` refuses inside `pmacs.window.commit_to` precisely so
/// a coroutine cannot park with the frontend scope pushed (Journey Stage
/// 1a, Q#JR14b). But `pmacs.async.yield_to_next_tick()` also yields, is
/// public, and carried **no** such refusal — so that invariant had a
/// second entrance.
///
/// **Reachability by a real caller is UNPROVEN.** No production caller
/// is known to yield through this door inside a commit; this pins the
/// guard rather than reproducing a user-visible bug.
///
/// Both halves asserted, for the same reason as rule 1's: a refusal that
/// leaves the scope pushed swaps a silent misrouting for a loud one and
/// fixes neither.
#[test]
fn yield_to_next_tick_inside_commit_to_is_refused_and_the_commit_scope_restores() {
    let mut state = editor();
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("alpha.txt"), b"alpha\n").expect("write");

    // A GENUINE destination, produced by the production capture: the
    // listener claims (returns false), so nothing commits and what lands
    // in `dest` is exactly the userdata dired would have received.
    // Nothing in a test can construct one.
    exec(
        &state,
        "dest = nil
         pmacs.hook.add('path.open-directory', function(_, d) dest = d return false end)",
    );
    state.open_directory_target(dir.path());
    pump(&mut state);
    assert!(
        eval::<bool>(&state, "return dest ~= nil"),
        "the chain must hand listeners a destination"
    );

    exec(
        &state,
        "REFUSAL = nil
         pmacs.async(function()
           local ok, err = pcall(pmacs.window.commit_to, dest, function()
             pmacs.async.yield_to_next_tick()
           end)
           REFUSAL = (not ok) and tostring(err) or '<no raise>'
         end)",
    );
    let refusal: String = eval(&state, "return REFUSAL");
    assert!(
        refusal.contains("cannot yield inside") && refusal.contains("commit_to"),
        "the second door into the commit scope must be shut; got {refusal:?}"
    );
    assert!(
        !eval::<bool>(&state, "return pmacs._async._in_commit_scope()"),
        "and the commit scope must still be restored afterwards"
    );
    pump(&mut state);
}

// ---------------------------------------------------------------------------
// 5 — the statusline activity indicator (Q#W-3, Q#W-6)
// ---------------------------------------------------------------------------

/// **Absent at zero, asserted as an absent SEGMENT rather than as an
/// empty string.** A zero-width segment still consumes a separator in
/// the composed modeline, so "returns nothing" and "returns nothing
/// visible" are different claims and only one of them is the design.
#[test]
fn the_indicator_produces_no_segment_at_all_when_nothing_is_running() {
    let state = editor();
    assert_eq!(
        activity_segment(&state),
        None,
        "an idle editor must produce NO activity segment"
    );
}

/// **A count plus the oldest in-flight job's purpose, witnessed through
/// the real per-frame evaluation path.**
///
/// Driven through `paint_frame`, not by calling the provider function
/// directly: a provider that works in isolation and never gets evaluated
/// is exactly the failure this must exclude.
#[test]
fn the_indicator_shows_a_count_and_the_oldest_purpose_in_a_painted_frame() {
    let mut state = editor();
    exec(
        &state,
        "FIRST = pmacs.workers.sleep(50)
         SECOND = pmacs.workers.grep({ root = '/tmp', pattern = 'zzz-no-match' })",
    );

    let cells = paint(&state, 24, 160);
    let modeline = row_text(&cells, 160, 22);
    assert!(
        modeline.contains("⋯2 sleep 50ms"),
        "the painted modeline must carry the count and the OLDEST job's \
         purpose (not the newest); got {modeline:?}"
    );

    // And the same value reaches the evaluator's segment vector, which is
    // what the semantic frontend ships.
    assert_eq!(
        activity_segment(&state).as_deref(),
        Some("⋯2 sleep 50ms"),
        "the segment and the painted row must agree"
    );

    exec(&state, "SECOND:cancel()");
    pump(&mut state);
}

/// **Q#W-6 — the setting, witnessed with work genuinely in flight.**
///
/// The discriminating case: an assertion taken on an idle editor cannot
/// tell "disabled" from "nothing is happening", which is the only thing
/// this setting changes.
#[test]
fn the_indicator_honours_its_setting_while_work_is_in_flight() {
    let mut state = editor();
    dispatch_one_in_flight(&state);
    assert!(
        activity_segment(&state).is_some(),
        "precondition: work is in flight and the indicator is on"
    );

    exec(&state, "pmacs.config.set('ui.activity-indicator', false)");
    assert_eq!(
        activity_segment(&state),
        None,
        "disabled means NO segment, with work still running"
    );

    exec(&state, "pmacs.config.set('ui.activity-indicator', true)");
    assert!(
        activity_segment(&state).is_some(),
        "and re-enabling brings it back without a restart"
    );
    pump(&mut state);
}

/// The setting is a real registry entry, not an ad-hoc global: it is
/// discoverable through `pmacs.config.describe` like every other
/// setting, which is what `COHERENCE.md` §11 grades.
#[test]
fn the_setting_is_registered_with_a_true_default() {
    let state = editor();
    let (kind, default): (String, bool) = eval(
        &state,
        "local d = pmacs.config.describe('ui.activity-indicator')
         return d.type, d.default",
    );
    assert_eq!(kind, "boolean");
    assert!(default, "visible by default — no configuration, no command");
}

// ---------------------------------------------------------------------------
// 6 — `*workers*` renders the purpose
// ---------------------------------------------------------------------------

/// The view §9 already has, now answering §9's question.
///
/// `Kind` names the builtin dispatcher a job funnelled through, which
/// for a third-party job is a builtin's label rather than the caller's;
/// the `Purpose` column is what carries the caller's own account.
#[test]
fn the_workers_buffer_renders_the_purpose_column() {
    let mut state = editor();
    exec(
        &state,
        "pmacs.workers.register('indexer', function()
           return pmacs.workers.sleep(50)
         end)
         pmacs.workers.dispatch('indexer')
         BUF = pmacs.workers.show()",
    );
    let text: String = eval(&state, "return BUF:slice(0, BUF:len())");
    assert!(
        text.contains("Purpose"),
        "the active table must have a Purpose column:\n{text}"
    );
    assert!(
        text.contains("indexer: sleep 50ms"),
        "and the row must render it:\n{text}"
    );
    exec(&state, "pmacs.workers.hide()");
    pump(&mut state);
}
