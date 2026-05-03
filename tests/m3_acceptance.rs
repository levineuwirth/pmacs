// m3_acceptance.rs --- T M3.9 M3 acceptance suite.

//! Acceptance suite for Milestone 3 (workers and async).
//!
//! Per T M3.9, every M3.x acceptance bullet from
//! `spec/pmacs-tasks.tex` is gated by an automated regression test.
//! Most bullets are covered by lib-level unit tests inside the
//! relevant module (`worker.rs`, `message_bus.rs`,
//! `async_runtime.rs`, plus the Lua-coupled `editor.rs::tests`).
//! This file does two things:
//!
//! 1. Tabulates the spec-bullet → test mapping below so a reviewer
//!    can verify "every criterion has a test" by reading one
//!    document.
//! 2. Adds an integration-level regression net for criteria that
//!    aren't already covered, plus the perf gates that are
//!    `#[ignore]`'d for hardware/scale reasons.
//!
//! Run with:
//!
//! ```sh
//! cargo test --test m3_acceptance
//! cargo test --release --test m3_acceptance -- --ignored --nocapture
//! ```
//!
//! # Spec → test map
//!
//! ## M3.1 — Worker pool primitive
//!
//! - Pool size configurable; defaults to cores - 1 →
//!   `m3_1_default_size_is_cores_minus_one` (this file),
//!   `worker::tests::default_size_floors_at_one`.
//! - Jobs run on workers; results returned via callback →
//!   `worker::tests::dispatch_runs_user_closure_on_a_worker`.
//! - Cancellation tokens checked at granular boundaries →
//!   `worker::tests::cancel_during_work_observed_by_user_closure`,
//!   `cancel_before_dispatch_skips_user_work`.
//! - Stress test: 10 000 dispatches with random cancels, no leaks →
//!   `worker::tests::stress_10k_dispatches_with_periodic_cancels_no_hang`.
//!
//! ## M3.2 — Message bus core
//!
//! - Send between main thread and worker pool →
//!   `message_bus::tests::worker_dispatched_job_sends_through_bus`.
//! - Schema mismatch is structured error →
//!   `message_bus::tests::send_with_wrong_type_returns_schema_mismatch`,
//!   `decode_with_wrong_type_returns_schema_mismatch`.
//! - Round-trip latency under 50 µs (small messages) →
//!   `message_bus::tests::round_trip_latency_is_within_budget`.
//! - No allocations in hot path beyond payload buffer →
//!   `message_bus::tests::envelope_topic_is_borrowed_static_str`.
//!
//! ## M3.3 — Lua async API (Lua-coupled; runs under lib unit tests)
//!
//! - Lua coroutine yields cleanly when awaiting a handle, resumes
//!   with result on completion →
//!   `editor::tests::async_coroutine_resumes_with_compute_sum_result`.
//! - Multiple concurrent awaits resolve independently →
//!   `editor::tests::multiple_concurrent_awaits_resolve_independently`.
//! - Cancelled awaits raise structured error (R45) →
//!   `editor::tests::cancelled_await_raises_tagged_error`.
//! - `on_complete` callback fires outside a coroutine →
//!   `editor::tests::on_complete_callback_fires_outside_a_coroutine`.
//! - No raw `coroutine.yield` allowed in package code (R46) →
//!   `editor::tests::non_handle_yield_is_reported_via_pmacs_error`.
//!
//! ## M3.4 — Supersede semantics
//!
//! - Supersession cancels in-flight job within 50 ms →
//!   `async_runtime::tests::supersede_cancels_in_flight_job_within_50ms`.
//! - Queued jobs with same key dropped before running →
//!   `async_runtime::tests::supersede_drops_queued_jobs_before_they_run`.
//! - No race conditions: rapid dispatch yields one running job →
//!   `async_runtime::tests::supersede_table_holds_only_the_most_recent_id`,
//!   `m3_4_rapid_supersede_one_running_at_a_time` (this file).
//!
//! ## M3.5 — Backpressure (output coalescing)
//!
//! - 10 000 msg/sec produces at most one wakeup per frame →
//!   `async_runtime::tests::streaming_handler_emits_all_items_with_few_batches`.
//! - No message loss under coalescing → same.
//! - Tunable batch size and frame target →
//!   `async_runtime::tests::frame_target_is_tunable_and_clamped`,
//!   `default_max_batch_is_tunable_and_clamped`.
//!
//! ## M3.6 — Parallel grep stress test
//!
//! - Grep across Linux kernel source < 2 s on 8 cores →
//!   `m3_6_grep_kernel_under_2s_on_8_cores` (this file, `#[ignore]`).
//! - New query cancels predecessor within 50 ms →
//!   `async_runtime::tests::grep_supersede_cancels_predecessor_within_50ms`.
//! - UI maintains 60 Hz responsiveness during saturating grep →
//!   `async_runtime::tests::grep_coalesces_saturating_match_rate`,
//!   `m3_6_saturating_grep_yields_bounded_main_thread_work` (this file).
//! - No worker holds buffer reference across cancellation (R31) →
//!   `async_runtime::tests::grep_types_satisfy_send_per_r31`.
//!
//! ## M3.7 — Workers observability buffer
//!
//! - Buffer updates within 100 ms of pool state changes →
//!   `editor::tests::workers_show_creates_and_refreshes_the_buffer`.
//! - Shows id, kind, age, supersede key, status →
//!   `workers_buffer::tests::active_row_includes_id_kind_age_status`,
//!   `editor::tests::workers_snapshot_via_lua_lists_active_jobs`.
//! - User can cancel via a binding inside the buffer →
//!   `editor::tests::workers_cancel_at_point_cancels_the_named_job`.
//!
//! ## M3.8 — Memory and lifecycle audit
//!
//! - Valgrind / sanitizer clean → documented in `tests/acceptance.rs`
//!   (module docs); runs the M3.8 stress tests under valgrind / ASAN.
//! - Memory growth < 5 %/hour under representative load →
//!   `acceptance::async_runtime_soak_lifecycle_stable` (`#[ignore]`).
//! - No file descriptor leaks → same soak test (FD count gate).

use std::time::{Duration, Instant};

use pmacs::async_runtime::AsyncRuntime;
use pmacs::worker::WorkerPool;

// ---------------------------------------------------------------------------
// M3.1 — Worker pool primitive
// ---------------------------------------------------------------------------

/// `WorkerPool::with_default_size()` honours the spec's "cores - 1"
/// default whenever the host has at least 2 cores. CI runners and
/// developer machines virtually always do; we skip the assertion
/// rather than fail on the rare 1-core target. The floor case is
/// covered by `worker::tests::default_size_floors_at_one`.
#[test]
fn m3_1_default_size_is_cores_minus_one() {
    let cores = std::thread::available_parallelism().map_or(2, std::num::NonZeroUsize::get);
    let pool = WorkerPool::with_default_size();
    if cores >= 2 {
        assert_eq!(
            pool.size(),
            cores - 1,
            "default pool size should be cores ({cores}) - 1"
        );
    } else {
        assert!(pool.size() >= 1, "pool size must floor at 1");
    }
}

// ---------------------------------------------------------------------------
// M3.4 — Supersede semantics
// ---------------------------------------------------------------------------

/// Rapid dispatch under one supersede key never runs more than one
/// job at a time. We dispatch 50 long sleeps under "search", sample
/// the snapshot during the storm, and assert the active set under
/// that key is at most one running entry per sample. (Earlier
/// dispatches that have been superseded but not yet observed their
/// cancel reply still appear in the snapshot's active list with
/// `cancel_requested = true`; the contract is that at most one is
/// *not* cancel-requested at any moment.)
#[test]
fn m3_4_rapid_supersede_one_running_at_a_time() {
    let rt = AsyncRuntime::with_pool_size(2);
    let mut ids = Vec::with_capacity(50);
    for _ in 0..50 {
        ids.push(rt.dispatch_sleep(500, Some("search")));
        let snap = rt.workers_snapshot();
        let live: Vec<_> = snap
            .active
            .iter()
            .filter(|a| a.supersede_key.as_deref() == Some("search"))
            .filter(|a| !a.cancel_requested)
            .collect();
        assert!(
            live.len() <= 1,
            "supersede invariant: more than one non-cancelled job under 'search' (live={})",
            live.len()
        );
        assert_eq!(
            rt.supersede_len(),
            1,
            "supersede table should hold exactly one slot"
        );
    }
    // Drain so the test exits cleanly. Settled entries in non-Running
    // state aren't visible via `workers_snapshot().active`, so we
    // iterate the ids we dispatched and `take_result` each one as it
    // settles.
    let deadline = Instant::now() + Duration::from_secs(10);
    while rt.pending_len() > 0 {
        assert!(
            Instant::now() < deadline,
            "post-test drain stuck (pending_len={})",
            rt.pending_len()
        );
        let _ = rt.tick();
        for id in &ids {
            if rt.is_complete(*id) {
                let _ = rt.take_result(*id);
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

// ---------------------------------------------------------------------------
// M3.6 — Parallel grep stress test
// ---------------------------------------------------------------------------

/// Spec criterion: grep across the Linux kernel source completes in
/// under 2 s on 8 cores. Requires a kernel checkout; opt in via:
///
/// ```sh
/// PMACS_KERNEL_PATH=/path/to/linux \
///   cargo test --release --test m3_acceptance -- --ignored --nocapture \
///     m3_6_grep_kernel_under_2s_on_8_cores
/// ```
///
/// Without the env var the test is a no-op (still passes) so CI
/// runs that don't have a kernel tree don't fail.
#[test]
#[ignore = "perf gate; requires PMACS_KERNEL_PATH=/path/to/linux"]
fn m3_6_grep_kernel_under_2s_on_8_cores() {
    let Some(root) = std::env::var_os("PMACS_KERNEL_PATH") else {
        eprintln!("PMACS_KERNEL_PATH not set; skipping kernel grep gate");
        return;
    };
    let root = std::path::PathBuf::from(root);
    assert!(
        root.is_dir(),
        "PMACS_KERNEL_PATH={root:?} is not a directory"
    );

    let rt = AsyncRuntime::with_pool_size(1);
    let spec = pmacs::async_runtime::GrepSpec {
        fanout: 8,
        ..pmacs::async_runtime::GrepSpec::new(root, "EXPORT_SYMBOL".to_string())
    };
    let started = Instant::now();
    let id = rt.dispatch_grep(spec, None, None);
    let deadline = started + Duration::from_secs(10);
    let mut closed = false;
    let mut matches: u64 = 0;
    while !closed {
        assert!(Instant::now() < deadline, "kernel grep deadline exceeded");
        let _ = rt.tick();
        for batch in rt.take_stream_batches() {
            if batch.id == id {
                matches += batch.items.len() as u64;
                if batch.closed {
                    closed = true;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let elapsed = started.elapsed();
    eprintln!("m3_6 kernel grep: matches={matches}, elapsed={elapsed:?} (target < 2s on 8 cores)");
    assert!(
        elapsed < Duration::from_secs(2),
        "kernel grep took {elapsed:?}; target < 2s"
    );
}

/// Spec criterion: UI maintains 60 Hz responsiveness during a
/// saturating grep. We can't measure UI cadence from a headless
/// test, but we can prove the runtime's contract that makes 60 Hz
/// possible: one `tick` + `take_stream_batches` pair processes a
/// bounded amount of work regardless of the worker's emit rate.
///
/// The deeper test is `grep_coalesces_saturating_match_rate`. Here
/// we add an integration-scope echo: under a high-output stream,
/// each main-thread coalesce step caps at the configured batch.
#[test]
fn m3_6_saturating_grep_yields_bounded_main_thread_work() {
    let rt = AsyncRuntime::with_pool_size(2);
    rt.set_default_max_batch(64);
    // emit_n is the synthetic high-rate stream the M3.5 tests rely
    // on. 10_000 items at the runtime's default batch cap reproduces
    // the 60 Hz proxy: every main-thread drain handles ≤ batch_cap.
    let id = rt.dispatch_emit_n(10_000, None, None);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut total: u64 = 0;
    let mut max_batch_seen: usize = 0;
    let mut closed = false;
    while !closed {
        assert!(Instant::now() < deadline, "saturating stream deadline");
        let _ = rt.tick();
        for batch in rt.take_stream_batches() {
            if batch.id == id {
                max_batch_seen = max_batch_seen.max(batch.items.len());
                total += batch.items.len() as u64;
                if batch.closed {
                    closed = true;
                }
            }
        }
        // Don't sleep --- we want to observe the smallest possible
        // wakeup cadence. The contract is each tick caps work, so
        // even a tight loop must stay bounded.
    }
    assert_eq!(total, 10_000, "no message loss under coalescing");
    assert!(
        max_batch_seen <= 64,
        "batch exceeded configured cap: {max_batch_seen} > 64"
    );
}
