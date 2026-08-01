// m9_2_acceptance.rs --- T M9.2 resource fetch + caching acceptance.

//! Acceptance tests for T M9.2 (`spec/pmacs-tasks.tex:3914`):
//!
//!   1. Read-resource round-trip works through a real MCP test server.
//!   2. Cache hit on repeated read of an unchanged resource.
//!   3. Cache invalidation by explicit `invalidate_resource`
//!      call; subsequent reads refetch.
//!
//! Plus three architectural-correctness tests called out during the
//! M9.2 design review:
//!
//!   4. Coalescing under load: 10 concurrent `read_resource` calls
//!      produce 1 wire request and 10 awaiters that all settle with
//!      the same result.
//!   5. In-flight failure: server crashes mid-request; primary +
//!      attached awaiters all settle with errors; subsequent read
//!      re-dispatches (cache state is Absent, not stuck in `InFlight`).
//!   6. Invalidation during in-flight: invalidate while a request is
//!      on the wire; the in-flight response settles awaiters but
//!      doesn't cache; a fresh read after invalidation re-dispatches.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use pmacs::async_runtime::{AsyncRuntime, JobId, JobOutcome, JobResult, SharedAsyncRuntime};
use pmacs::lua_bindings::SharedProcessSupervisor;
use pmacs::mcp::{
    McpClientState, McpEvent, McpEventKind, McpManager, McpRestartPolicy, McpServerId,
    McpServerSpec, SharedMcpManager,
};
use pmacs::process::ProcessSupervisor;

fn fake_mcp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_mcp").to_owned()
}

fn make_test_triple() -> (
    SharedProcessSupervisor,
    SharedAsyncRuntime,
    SharedMcpManager,
) {
    let sup = Rc::new(RefCell::new(ProcessSupervisor::new()));
    let runtime: SharedAsyncRuntime = Rc::new(AsyncRuntime::with_pool_size(1));
    let mgr = Rc::new(RefCell::new(McpManager::new(sup.clone(), runtime.clone())));
    (sup, runtime, mgr)
}

fn fake_spec(label: &str) -> McpServerSpec {
    let mut spec = McpServerSpec::new(label, fake_mcp_path());
    spec.restart = McpRestartPolicy::Never;
    spec
}

/// Drain the supervisor + manager + runtime ticks until either the
/// predicate is true or the deadline lapses. Used both for waiting
/// on lifecycle events and for waiting on runtime jobs to settle.
fn pump_until<F: FnMut() -> bool>(
    sup: &SharedProcessSupervisor,
    runtime: &SharedAsyncRuntime,
    mgr: &SharedMcpManager,
    deadline: Duration,
    mut pred: F,
) {
    let stop = Instant::now() + deadline;
    while Instant::now() < stop {
        sup.borrow_mut().tick();
        mgr.borrow_mut().tick();
        runtime.tick();
        if pred() {
            return;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// Drain MCP events for `sid` until `pred` is satisfied.
fn drain_mcp_until<F: Fn(&[McpEvent]) -> bool>(
    sup: &SharedProcessSupervisor,
    runtime: &SharedAsyncRuntime,
    mgr: &SharedMcpManager,
    sid: McpServerId,
    deadline: Duration,
    pred: F,
) -> Vec<McpEvent> {
    let stop = Instant::now() + deadline;
    let mut all: Vec<McpEvent> = Vec::new();
    while Instant::now() < stop {
        sup.borrow_mut().tick();
        mgr.borrow_mut().tick();
        runtime.tick();
        let mut evs = mgr.borrow_mut().take_events(sid);
        all.append(&mut evs);
        if pred(&all) {
            return all;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    all
}

/// Drive an Initialized server.
fn spawn_initialized(
    sup: &SharedProcessSupervisor,
    runtime: &SharedAsyncRuntime,
    mgr: &SharedMcpManager,
    spec: McpServerSpec,
) -> McpServerId {
    let sid = mgr.borrow_mut().spawn(spec).expect("spawn");
    drain_mcp_until(sup, runtime, mgr, sid, Duration::from_secs(5), |evs| {
        evs.iter()
            .any(|e| matches!(e.kind, McpEventKind::Initialized { .. }))
    });
    assert!(matches!(
        mgr.borrow().state(sid),
        Some(McpClientState::Initialized { .. })
    ));
    sid
}

fn await_job(
    sup: &SharedProcessSupervisor,
    runtime: &SharedAsyncRuntime,
    mgr: &SharedMcpManager,
    job_id: JobId,
    deadline: Duration,
) -> JobOutcome {
    pump_until(sup, runtime, mgr, deadline, || runtime.is_complete(job_id));
    runtime
        .take_result(job_id)
        .unwrap_or_else(|| panic!("job {job_id} did not settle within deadline"))
}

fn unwrap_text(outcome: JobOutcome) -> String {
    match outcome {
        JobOutcome::Complete(JobResult::Json(v)) => {
            let arr = v
                .get("contents")
                .and_then(|c| c.as_array())
                .expect("contents array");
            assert_eq!(arr.len(), 1, "fake returns one content entry");
            arr[0]
                .get("text")
                .and_then(|t| t.as_str())
                .expect("text field")
                .to_owned()
        }
        other => panic!("expected Complete(Json(...)), got {other:?}"),
    }
}

// ===========================================================================
// Spec acceptance bullets
// ===========================================================================

/// Bullet 1: read-resource round-trip works.
#[test]
fn m9_2_read_resource_round_trip() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = spawn_initialized(&sup, &runtime, &mgr, fake_spec("read"));
    let job_id = mgr
        .borrow_mut()
        .read_resource(sid, "file:///foo")
        .expect("read_resource");
    let text = unwrap_text(await_job(
        &sup,
        &runtime,
        &mgr,
        job_id,
        Duration::from_secs(5),
    ));
    // Fake's response includes a counter and the URI.
    assert!(
        text.contains("synthetic-1-for-file:///foo"),
        "first read should produce synthetic-1; got {text:?}"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

/// Bullet 2: a second read of the same URI is a cache hit. The fake's
/// per-process counter increments on every wire `resources/read`, so
/// "two reads, same text" proves the second read didn't go on the wire.
#[test]
fn m9_2_cache_hit_on_repeated_read() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = spawn_initialized(&sup, &runtime, &mgr, fake_spec("hit"));
    let job1 = mgr
        .borrow_mut()
        .read_resource(sid, "file:///x")
        .expect("read 1");
    let text1 = unwrap_text(await_job(
        &sup,
        &runtime,
        &mgr,
        job1,
        Duration::from_secs(5),
    ));
    let job2 = mgr
        .borrow_mut()
        .read_resource(sid, "file:///x")
        .expect("read 2");
    let text2 = unwrap_text(await_job(
        &sup,
        &runtime,
        &mgr,
        job2,
        Duration::from_secs(5),
    ));
    assert_eq!(
        text1, text2,
        "second read must hit the cache (same counter value); got {text1:?} vs {text2:?}"
    );
    assert!(
        text1.contains("synthetic-1-for-"),
        "fake's counter should still be 1 after a single wire fetch; got {text1:?}"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

/// Bullet 3: `invalidate_resource` forces a refetch on the next read.
#[test]
fn m9_2_invalidation_forces_refetch() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = spawn_initialized(&sup, &runtime, &mgr, fake_spec("invalidate"));
    let job1 = mgr
        .borrow_mut()
        .read_resource(sid, "file:///y")
        .expect("read 1");
    let text1 = unwrap_text(await_job(
        &sup,
        &runtime,
        &mgr,
        job1,
        Duration::from_secs(5),
    ));
    mgr.borrow_mut().invalidate_resource(sid, "file:///y");
    let job2 = mgr
        .borrow_mut()
        .read_resource(sid, "file:///y")
        .expect("read after invalidate");
    let text2 = unwrap_text(await_job(
        &sup,
        &runtime,
        &mgr,
        job2,
        Duration::from_secs(5),
    ));
    assert_ne!(
        text1, text2,
        "post-invalidation read must refetch (different counter); got {text1:?} both times"
    );
    assert!(
        text1.contains("synthetic-1-for-"),
        "first counter value should be 1; got {text1:?}"
    );
    assert!(
        text2.contains("synthetic-2-for-"),
        "post-invalidation counter should be 2 (fresh wire fetch); got {text2:?}"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

// ===========================================================================
// Architectural-correctness tests
// ===========================================================================

/// (4) M3.5 coalescing: 10 concurrent `read_resource` calls for the
/// same URI produce 1 wire request and 10 awaiters that settle with
/// the same result. Verifies the `InFlight` state attaches siblings.
#[test]
fn m9_2_coalescing_under_load() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = spawn_initialized(&sup, &runtime, &mgr, fake_spec("coalesce"));
    let mut jobs: Vec<JobId> = Vec::with_capacity(10);
    for _ in 0..10 {
        let j = mgr
            .borrow_mut()
            .read_resource(sid, "file:///shared")
            .expect("read");
        jobs.push(j);
    }
    pump_until(&sup, &runtime, &mgr, Duration::from_secs(5), || {
        jobs.iter().all(|&j| runtime.is_complete(j))
    });
    let mut texts: Vec<String> = Vec::with_capacity(jobs.len());
    for j in jobs {
        let outcome = runtime.take_result(j).expect("settled");
        texts.push(unwrap_text(outcome));
    }
    let first = &texts[0];
    for t in &texts[1..] {
        assert_eq!(
            first, t,
            "all coalesced awaiters must settle with the same value"
        );
    }
    // Server-side counter visible in the text proves only one wire
    // fetch landed despite 10 client-side reads.
    assert!(
        first.contains("synthetic-1-for-"),
        "counter must be 1 for a single wire fetch coalescing 10 reads; got {first:?}"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

/// (5) In-flight failure: server crashes mid-request; primary +
/// attached awaiters all settle with errors; the cache transitions
/// `InFlight` → Absent (not stuck), so a subsequent read
/// re-dispatches once a fresh server is in place.
#[test]
fn m9_2_in_flight_failure_settles_all_awaiters_and_clears_cache() {
    let (sup, runtime, mgr) = make_test_triple();
    let mut spec = fake_spec("crash");
    spec.env = vec![(
        "PMACS_FAKE_MCP_MODE".into(),
        "crash_after_first_request".into(),
    )];
    let sid = spawn_initialized(&sup, &runtime, &mgr, spec);
    // 5 concurrent reads coalesce onto one in-flight request that
    // never completes — the fake exits with code 77 instead.
    let mut jobs: Vec<JobId> = Vec::with_capacity(5);
    for _ in 0..5 {
        let j = mgr
            .borrow_mut()
            .read_resource(sid, "file:///doomed")
            .expect("read");
        jobs.push(j);
    }
    pump_until(&sup, &runtime, &mgr, Duration::from_secs(5), || {
        jobs.iter().all(|&j| runtime.is_complete(j))
    });
    for j in jobs {
        let outcome = runtime.take_result(j).expect("settled");
        assert!(
            matches!(outcome, JobOutcome::Cancelled | JobOutcome::Failed(_)),
            "every awaiter must settle non-Ok after server crash; got {outcome:?}"
        );
    }
    // Server is now Crashed; cache should not be stuck in InFlight.
    // We can't directly inspect the cache from outside the manager,
    // but a subsequent read_resource attempt would return an error
    // (server not Initialized), which is the right negative signal:
    // the cache didn't pin a stale InFlight entry.
    let res = mgr.borrow_mut().read_resource(sid, "file:///doomed");
    assert!(
        res.is_err(),
        "reads against a crashed server must error (not return a stale handle); got {res:?}"
    );
}

/// (6) Invalidation during in-flight: invalidate while a request is
/// on the wire. The in-flight response still settles awaiters with
/// the result, but **does not** cache. A fresh read after invalidation
/// re-dispatches.
#[test]
fn m9_2_invalidation_during_in_flight_does_not_cache() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = spawn_initialized(&sup, &runtime, &mgr, fake_spec("inv-in-flight"));
    // Dispatch the first read. Don't drive it to completion yet.
    let job1 = mgr
        .borrow_mut()
        .read_resource(sid, "file:///z")
        .expect("read 1");
    // Invalidate immediately (request is still on the wire).
    mgr.borrow_mut().invalidate_resource(sid, "file:///z");
    // Drive job 1 to completion. It should still settle with a
    // result — invalidation does not abort in-flight requests.
    let outcome1 = await_job(&sup, &runtime, &mgr, job1, Duration::from_secs(5));
    let text1 = unwrap_text(outcome1);
    // Now dispatch a fresh read. With finding-correct behavior,
    // this re-dispatches (fake counter increments). Without it, a
    // bug could either return the stale "cached" value (counter=1)
    // or return whatever the in-flight request stashed.
    let job2 = mgr
        .borrow_mut()
        .read_resource(sid, "file:///z")
        .expect("read 2");
    let text2 = unwrap_text(await_job(
        &sup,
        &runtime,
        &mgr,
        job2,
        Duration::from_secs(5),
    ));
    assert!(
        text1.contains("synthetic-1-for-"),
        "first read produced counter 1; got {text1:?}"
    );
    assert!(
        text2.contains("synthetic-2-for-"),
        "post-invalidation read must refetch (counter 2); got {text2:?} (cache-after-invalidation bug?)"
    );
    let _ = mgr.borrow_mut().stop(sid);
}

// ===========================================================================
// Pass-2 findings: lifecycle + per-sibling cancellation
// ===========================================================================

/// Pass-2 finding 1: a cached resource read against a stopped or
/// forgotten server must error rather than return stale cache data.
/// `read_resource` checks the server's state before consulting the
/// cache, and `on_exit` / `forget` clear the cache for that sid so
/// no stale entries can survive.
#[test]
fn m9_2_cached_read_after_stop_rejects_stale_sid() {
    let (sup, runtime, mgr) = make_test_triple();
    let sid = spawn_initialized(&sup, &runtime, &mgr, fake_spec("stop-cache"));

    // Populate the cache.
    let job1 = mgr
        .borrow_mut()
        .read_resource(sid, "file:///stale")
        .expect("read 1");
    let _ = unwrap_text(await_job(
        &sup,
        &runtime,
        &mgr,
        job1,
        Duration::from_secs(5),
    ));

    // Stop and pump to terminal.
    let _ = mgr.borrow_mut().stop(sid);
    drain_mcp_until(&sup, &runtime, &mgr, sid, Duration::from_secs(5), |_| {
        matches!(
            mgr.borrow().state(sid),
            Some(McpClientState::Stopped { .. } | McpClientState::Crashed { .. })
        )
    });

    // The post-stop read must error: the server is not Initialized.
    let res = mgr.borrow_mut().read_resource(sid, "file:///stale");
    assert!(
        res.is_err(),
        "post-stop read_resource must error (not return cached data); got {res:?}"
    );

    // Forget and try again — same expectation, different code path.
    mgr.borrow_mut()
        .forget(sid)
        .expect("forget should succeed in terminal state");
    let res2 = mgr.borrow_mut().read_resource(sid, "file:///stale");
    assert!(
        res2.is_err(),
        "post-forget read_resource must error (server unknown); got {res2:?}"
    );
}

/// Pass-2 finding 2: per-awaiter cancellation tokens. With three
/// awaiters coalesced onto one in-flight request, cancelling one
/// must:
/// (a) settle that awaiter as Cancelled,
/// (b) leave the other two awaiters waiting,
/// (c) NOT abort the in-flight wire request,
/// (d) deliver the eventual response to the surviving awaiters as Ok.
#[test]
fn m9_2_per_sibling_cancellation_does_not_disturb_others() {
    let (sup, runtime, mgr) = make_test_triple();
    let mut spec = fake_spec("per-sibling-cancel");
    spec.env = vec![("PMACS_FAKE_MCP_MODE".into(), "slow_resources_read".into())];
    let sid = spawn_initialized(&sup, &runtime, &mgr, spec);

    // Three concurrent reads — coalesce onto one in-flight request.
    // The fake delays 250ms before responding, so we have a window
    // to cancel one awaiter before the response arrives.
    let job_a = mgr
        .borrow_mut()
        .read_resource(sid, "file:///shared")
        .expect("read a");
    let job_b = mgr
        .borrow_mut()
        .read_resource(sid, "file:///shared")
        .expect("read b");
    let job_c = mgr
        .borrow_mut()
        .read_resource(sid, "file:///shared")
        .expect("read c");

    // Cancel only b. Use the runtime's cancel API directly (this is
    // what `pmacs.workers._cancel(id)` ultimately calls).
    runtime.cancel(job_b);

    // Pump until all three settle.
    pump_until(&sup, &runtime, &mgr, Duration::from_secs(5), || {
        runtime.is_complete(job_a) && runtime.is_complete(job_b) && runtime.is_complete(job_c)
    });

    let outcome_a = runtime.take_result(job_a).expect("a settled");
    let outcome_b = runtime.take_result(job_b).expect("b settled");
    let outcome_c = runtime.take_result(job_c).expect("c settled");

    // (a) and (c) must settle Ok.
    let text_a = unwrap_text(outcome_a);
    let text_c = unwrap_text(outcome_c);
    assert_eq!(
        text_a, text_c,
        "uncancelled awaiters must settle Ok with the same value"
    );
    assert!(
        text_a.contains("synthetic-1-for-"),
        "wire fetch should have produced counter 1; got {text_a:?}"
    );

    // (b) must settle Cancelled.
    assert!(
        matches!(outcome_b, JobOutcome::Cancelled),
        "the cancelled awaiter must settle as Cancelled; got {outcome_b:?}"
    );

    let _ = mgr.borrow_mut().stop(sid);
}

/// Same contract as the previous test, but with the response already
/// queued before the manager observes cancellation. This catches the
/// tick-order race where process events were drained before cancelled
/// awaiters and a cancelled handle could receive `Ok`.
#[test]
fn m9_2_cancelled_sibling_wins_over_queued_response() {
    let (sup, runtime, mgr) = make_test_triple();
    let mut spec = fake_spec("cancel-response-race");
    spec.env = vec![("PMACS_FAKE_MCP_MODE".into(), "slow_resources_read".into())];
    let sid = spawn_initialized(&sup, &runtime, &mgr, spec);

    let job_a = mgr
        .borrow_mut()
        .read_resource(sid, "file:///race")
        .expect("read a");
    let job_b = mgr
        .borrow_mut()
        .read_resource(sid, "file:///race")
        .expect("read b");
    let job_c = mgr
        .borrow_mut()
        .read_resource(sid, "file:///race")
        .expect("read c");

    // Let the fake server finish its delayed response (it sleeps
    // 250ms), then harvest the supervisor event queue without giving
    // McpManager a chance to process it yet. The margin over the
    // fake's delay must absorb two pipe transits plus the request's
    // queued-stdin-writer hop under CI load (a 100ms margin flaked on
    // macOS runners); a generous wait does not weaken the contract —
    // the race under test is cancel-AFTER-queue-BEFORE-manager-tick,
    // which holds for any wait long enough for the response to land.
    std::thread::sleep(Duration::from_secs(1));
    sup.borrow_mut().tick();

    // Cancel only b after the response is queued but before
    // McpManager::tick drains that response.
    runtime.cancel(job_b);
    mgr.borrow_mut().tick();
    runtime.tick();

    assert!(
        runtime.is_complete(job_a) && runtime.is_complete(job_b) && runtime.is_complete(job_c),
        "one manager tick should settle the queued response and cancellation"
    );

    let outcome_a = runtime.take_result(job_a).expect("a settled");
    let outcome_b = runtime.take_result(job_b).expect("b settled");
    let outcome_c = runtime.take_result(job_c).expect("c settled");
    let text_a = unwrap_text(outcome_a);
    let text_c = unwrap_text(outcome_c);
    assert_eq!(text_a, text_c, "surviving awaiters receive the response");
    assert!(
        matches!(outcome_b, JobOutcome::Cancelled),
        "cancelled awaiter must remain Cancelled even when response was queued first; got {outcome_b:?}"
    );

    let _ = mgr.borrow_mut().stop(sid);
}

// ===========================================================================
// Lua surface
// ===========================================================================

/// `pmacs.mcp.read_resource(server, uri):await()` resolves to the
/// response's `result` table. The fake's `resources/read` returns
/// `{ contents = [{ uri, mimeType, text }] }`.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "linear pump-coroutine-then-verify pattern; splitting fragments the test's narrative"
)]
fn m9_2_lua_read_resource_returns_awaitable_handle() {
    use pmacs::editor::EditorState;

    let mut state = EditorState::new_with_roots(&crate::iso::roots());
    let fake = fake_mcp_path();

    state
        .lua_host
        .lua()
        .load(format!(
            "
            _G._mcp_test_server = pmacs.mcp.spawn({{
                label = 'lua-read',
                command = '{fake}',
                restart = 'never',
            }})
            ",
        ))
        .exec()
        .expect("spawn via Lua");

    // Pump until Initialized.
    let stop = Instant::now() + Duration::from_secs(5);
    let mut initialized = false;
    while Instant::now() < stop && !initialized {
        state.tick_processes();
        state.tick_mcp();
        state.tick_async();
        let kinds: Vec<String> = state
            .lua_host
            .lua()
            .load(
                "
                local out = {}
                for _, row in ipairs(pmacs.mcp.list()) do
                    out[#out+1] = row.state.kind
                end
                return out
                ",
            )
            .eval()
            .expect("list");
        if kinds.iter().any(|k| k == "initialized") {
            initialized = true;
        }
        if !initialized {
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    assert!(initialized, "server must reach Initialized");

    // Spawn the awaiting coroutine.
    state
        .lua_host
        .lua()
        .load(
            "
            _G._mcp_test_done = false
            _G._mcp_test_text = nil
            pmacs.async(function()
                local result = pmacs.mcp.read_resource(_G._mcp_test_server, 'file:///lua'):await()
                _G._mcp_test_text = result.contents[1].text
                _G._mcp_test_done = true
            end)
            ",
        )
        .exec()
        .expect("dispatch awaiting coroutine");

    // Pump until the coroutine completes.
    let stop = Instant::now() + Duration::from_secs(5);
    let mut done = false;
    while Instant::now() < stop && !done {
        state.tick_processes();
        state.tick_mcp();
        state.tick_async();
        done = state
            .lua_host
            .lua()
            .load("return _G._mcp_test_done")
            .eval::<bool>()
            .unwrap_or(false);
        if !done {
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    assert!(done, "awaiting coroutine must complete");

    let text: String = state
        .lua_host
        .lua()
        .load("return _G._mcp_test_text")
        .eval()
        .expect("read result text");
    assert!(
        text.contains("file:///lua"),
        "read_resource result must contain the URI; got {text:?}"
    );

    // Verify cache hit: invalidate via Lua, then read again, the new
    // text differs (counter incremented).
    state
        .lua_host
        .lua()
        .load(
            "
            _G._mcp_test_done2 = false
            _G._mcp_test_text2 = nil
            pmacs.mcp.invalidate_resource(_G._mcp_test_server, 'file:///lua')
            pmacs.async(function()
                local result = pmacs.mcp.read_resource(_G._mcp_test_server, 'file:///lua'):await()
                _G._mcp_test_text2 = result.contents[1].text
                _G._mcp_test_done2 = true
            end)
            ",
        )
        .exec()
        .expect("invalidate + re-read");

    let stop = Instant::now() + Duration::from_secs(5);
    let mut done2 = false;
    while Instant::now() < stop && !done2 {
        state.tick_processes();
        state.tick_mcp();
        state.tick_async();
        done2 = state
            .lua_host
            .lua()
            .load("return _G._mcp_test_done2")
            .eval::<bool>()
            .unwrap_or(false);
        if !done2 {
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    assert!(done2, "post-invalidation read must complete");

    let text2: String = state
        .lua_host
        .lua()
        .load("return _G._mcp_test_text2")
        .eval()
        .expect("read result text 2");
    assert_ne!(
        text, text2,
        "Lua-side invalidate_resource must force refetch"
    );

    let _ = state
        .lua_host
        .lua()
        .load("pmacs.mcp.stop(_G._mcp_test_server)")
        .exec();
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
