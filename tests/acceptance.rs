// acceptance.rs --- M1 acceptance suite.

//! Acceptance tests for Milestone 1.
//!
//! Each test corresponds to one bullet in the M1 acceptance criteria from
//! the tasks doc (T M1.10). Most are gated behind `#[ignore]` because
//! they are slow or perf-sensitive (release-mode only); run them with:
//!
//! ```sh
//! cargo test --release --test acceptance -- --ignored --nocapture
//! ```
//!
//! Targets:
//! * Open 100 MB file < 200 ms.
//! * Edit p99 < 1 ms on 10 MB buffer.
//! * Snapshot < 10 µs.
//! * Memory after 10 000 random edits < 3 × source file size.
//! * 30 s fuzz (10 min via `PMACS_FUZZ_SECS=600`) produces no crashes.

use std::time::{Duration, Instant};

use pmacs::buffer::{Buffer, BufferId, EditOp};
use pmacs::rope::{Range, Rope};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn synthetic_bytes(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
}

/// Approximate live byte count: sum of all leaf chunk sizes.
///
/// Walks the rope and sums chunk lengths. Chunks may be shared (Arc) so
/// this overestimates memory in the presence of structural sharing, which
/// is the conservative direction for the "≤ 3× source size" gate.
fn approx_bytes(rope: &Rope) -> u64 {
    rope.chunks(0, rope.len()).map(|c| c.len() as u64).sum()
}

// ---------------------------------------------------------------------------
// Perf gates (release-only, ignored by default)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "release-only perf gate; run with --ignored --release"]
fn open_100mb_under_200ms() {
    let src = synthetic_bytes(100 * 1024 * 1024);
    let started = Instant::now();
    let rope = Rope::from_bytes(&src);
    let elapsed = started.elapsed();
    eprintln!("open_100mb: {elapsed:?}");
    assert_eq!(rope.len(), src.len() as u64);
    assert!(
        elapsed < Duration::from_millis(200),
        "100MB load took {elapsed:?}, target < 200ms"
    );
}

#[test]
#[ignore = "release-only perf gate; run with --ignored --release"]
fn edit_p99_under_1ms_on_10mb_buffer() {
    let src = synthetic_bytes(10 * 1024 * 1024);
    let mut rope = Rope::from_bytes(&src);

    let mut times = Vec::with_capacity(1000);
    for i in 0..1000 {
        let pos = ((i * 1024) as u64) % rope.len();
        let started = Instant::now();
        rope = rope.insert(pos, b"x").unwrap().new_rope;
        times.push(started.elapsed());
    }
    times.sort_unstable();
    let p50 = times[500];
    let p99 = times[990];
    let max = times[999];
    eprintln!("edit p50={p50:?} p99={p99:?} max={max:?}");
    assert!(
        p99 < Duration::from_millis(1),
        "edit p99 was {p99:?}, target < 1ms"
    );
}

#[test]
#[ignore = "release-only perf gate; run with --ignored --release"]
fn snapshot_under_10us() {
    let src = synthetic_bytes(10 * 1024 * 1024);
    let rope = Rope::from_bytes(&src);

    // Warm + measure 10 000 snapshots; per-op average must be under 10 µs.
    // Snapshot is an Arc::clone, so the floor is sub-µs in practice.
    let started = Instant::now();
    let snaps: Vec<Rope> = (0..10_000).map(|_| rope.snapshot()).collect();
    let elapsed = started.elapsed();
    let per_op = elapsed / 10_000;
    eprintln!("snapshot per-op (avg over 10k): {per_op:?}");
    std::hint::black_box(snaps);
    assert!(
        per_op < Duration::from_micros(10),
        "snapshot avg was {per_op:?}, target < 10µs"
    );
}

#[test]
#[ignore = "release-only memory gate; run with --ignored --release"]
fn memory_after_10k_edits_within_3x() {
    let original = synthetic_bytes(64 * 1024); // 64 KB source
    let mut rope = Rope::from_bytes(&original);

    let mut state: u64 = 0xACCE_5817;
    let mut rng = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (state >> 33) as u32
    };

    for _ in 0..10_000 {
        let len = rope.len();
        match rng() % 3 {
            0 => {
                let pos = u64::from(rng()) % (len + 1);
                let n = (rng() % 64 + 1) as usize;
                let bytes: Vec<u8> = (0..n)
                    .map(|i| (rng() as u8).wrapping_add(i as u8))
                    .collect();
                rope = rope.insert(pos, &bytes).unwrap().new_rope;
            }
            1 if len > 0 => {
                let s = u64::from(rng()) % len;
                let e = (s + u64::from(rng()) % (len - s).max(1)).min(len);
                if s < e {
                    rope = rope.delete(s, e).unwrap().new_rope;
                }
            }
            _ if len > 0 => {
                let s = u64::from(rng()) % len;
                let e = (s + u64::from(rng()) % (len - s).max(1)).min(len);
                if s < e {
                    let n = (rng() % 16) as usize;
                    let bytes: Vec<u8> = (0..n)
                        .map(|i| (rng() as u8).wrapping_add(i as u8))
                        .collect();
                    rope = rope.replace(s, e, &bytes).unwrap().new_rope;
                }
            }
            _ => {}
        }
    }

    let live = approx_bytes(&rope);
    let limit = (original.len() as u64) * 3;
    eprintln!(
        "after 10k edits: rope.len = {}, leaf-bytes = {} (limit = {})",
        rope.len(),
        live,
        limit
    );
    assert!(
        live <= limit,
        "leaf bytes {live} exceed 3× source ({limit})"
    );
}

#[test]
#[ignore = "long; default 30s, override with PMACS_FUZZ_SECS=N"]
fn fuzz_no_crashes() {
    let secs: u64 = std::env::var("PMACS_FUZZ_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let deadline = Instant::now() + Duration::from_secs(secs);

    let mut state: u64 = 0xF000_0001;
    let mut rng = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (state >> 33) as u32
    };

    let mut buf = Buffer::from_bytes(BufferId::next(), "fuzz", &synthetic_bytes(256));
    let mut iters = 0u64;
    while Instant::now() < deadline {
        iters += 1;
        let len = buf.len();
        let _ = match rng() % 6 {
            0 => {
                let pos = u64::from(rng()) % (len + 1);
                let n = (rng() % 32 + 1) as usize;
                let bytes: Vec<u8> = (0..n)
                    .map(|i| (rng() as u8).wrapping_add(i as u8))
                    .collect();
                buf.apply_edit(EditOp::Insert { pos, bytes: &bytes })
                    .map(|_| ())
            }
            1 if len > 0 => {
                let s = u64::from(rng()) % len;
                let e = (s + u64::from(rng()) % (len - s).max(1)).min(len);
                if s < e {
                    buf.apply_edit(EditOp::Delete {
                        range: Range::new(s, e),
                    })
                    .map(|_| ())
                } else {
                    Ok(())
                }
            }
            2 if len > 0 => {
                let s = u64::from(rng()) % len;
                let e = (s + u64::from(rng()) % (len - s).max(1)).min(len);
                if s < e {
                    let n = (rng() % 16) as usize;
                    let bytes: Vec<u8> = (0..n)
                        .map(|i| (rng() as u8).wrapping_add(i as u8))
                        .collect();
                    buf.apply_edit(EditOp::Replace {
                        range: Range::new(s, e),
                        bytes: &bytes,
                    })
                    .map(|_| ())
                } else {
                    Ok(())
                }
            }
            3 => {
                let _ = buf.undo();
                Ok(())
            }
            4 => {
                let _ = buf.redo();
                Ok(())
            }
            _ => Ok(()),
        };
    }
    eprintln!("fuzz: {iters} iterations in {secs}s, no crash");
}

// ---------------------------------------------------------------------------
// Always-on smoke tests (cheap; run every CI build)
// ---------------------------------------------------------------------------

#[test]
fn snapshot_independence_after_many_edits() {
    // Take a snapshot, then mutate the source many times; the snapshot
    // must remain byte-identical to its original content. This exercises
    // the structural-sharing contract end-to-end at the public API.
    let src = synthetic_bytes(8 * 1024);
    let original = Rope::from_bytes(&src);
    let snap = original.snapshot();

    let mut current = original;
    for i in 0..200 {
        let pos = (i * 17) as u64 % (current.len() + 1);
        current = current.insert(pos, b"X").unwrap().new_rope;
    }

    // Snapshot still round-trips to the source bytes.
    let mut out = vec![0u8; snap.len() as usize];
    snap.slice(0, snap.len(), &mut out);
    assert_eq!(out, src);
}

#[test]
fn round_trip_via_buffer() {
    // The integration path: Buffer::from_bytes → many edits → undo all →
    // recover source bytes exactly.
    let src = synthetic_bytes(2 * 1024);
    let mut buf = Buffer::from_bytes(BufferId::next(), "test", &src);
    let edits = 50;
    for i in 0..edits {
        let pos = (i * 41) as u64 % (buf.len() + 1);
        buf.apply_edit(EditOp::Insert { pos, bytes: b"Z" }).unwrap();
    }
    for _ in 0..edits {
        buf.undo().unwrap();
    }
    let mut out = vec![0u8; buf.len() as usize];
    buf.snapshot_rope().slice(0, buf.len(), &mut out);
    assert_eq!(out, src);
}

// ---------------------------------------------------------------------------
// M3.8 — Memory and lifecycle audit (long-running soak)
// ---------------------------------------------------------------------------
//
// The spec asks for a 1-hour continuous-load run with stable memory.
// CI cannot afford that, so the test is `#[ignore]`'d and reads its
// duration from `PMACS_SOAK_SECS` (default: 10 seconds for a quick
// local sanity check). The 1-hour run is:
//
// ```sh
// PMACS_SOAK_SECS=3600 cargo test --release --test acceptance \
//     -- --ignored --nocapture async_runtime_soak_lifecycle_stable
// ```
//
// Acceptance bullets covered:
//   * Memory growth under 5%/hour under representative load.
//   * No file-descriptor leaks.
//   * Internal state-tables (pending, supersede) return to zero on
//     drain; the completion ring stays bounded.
//
// Linux-only for the FD/RSS instrumentation; the test still runs
// elsewhere but skips those gates with a stderr note.
//
// Valgrind / sanitizer gates (run manually; long):
//
// ```sh
// # Valgrind: leak-check the 1000-cycle stress test.
// valgrind --leak-check=full --error-exitcode=1 \
//     ./target/debug/deps/pmacs-* dispatch_cancel_1000_cycles_no_leak \
//     --test-threads=1 --nocapture
//
// # AddressSanitizer (nightly): catches use-after-free / overflows.
// RUSTFLAGS="-Zsanitizer=address" \
//     cargo +nightly test --target x86_64-unknown-linux-gnu --lib \
//     dispatch_cancel_1000_cycles_no_leak supersede_churn_500 stream_dispatch_close
//
// # LeakSanitizer (nightly).
// RUSTFLAGS="-Zsanitizer=leak" \
//     cargo +nightly test --target x86_64-unknown-linux-gnu \
//     dispatch_cancel_1000_cycles_no_leak
// ```
//
// The Rust+luajit FFI surface is the primary suspect for any
// finding; the runtime itself is pure-Rust and ownership-driven.

fn read_proc_fd_count() -> Option<usize> {
    std::fs::read_dir("/proc/self/fd").ok().map(Iterator::count)
}

fn read_proc_rss_kib() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kib: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kib);
        }
    }
    None
}

mod soak_helpers {
    use std::time::{Duration, Instant};

    use pmacs::async_runtime::{AsyncRuntime, JobOutcome};

    /// One soak cycle: dispatch a workload, optionally cancel it,
    /// drain it through to eviction. Returns once the dispatched id
    /// is no longer in `pending`.
    pub(super) fn one_cycle(rt: &AsyncRuntime, rng: &mut dyn FnMut() -> u32, iter: u64) {
        let id = match rng() % 3 {
            0 => rt.dispatch_sleep(i64::from(rng()) % 4 + 1, None),
            1 => rt.dispatch_compute_sum((u64::from(rng()) % 1_000) + 1, None),
            _ => rt.dispatch_emit_n((u64::from(rng()) % 32) + 1, None, Some(8)),
        };
        if rng() % 4 == 0 {
            rt.cancel(id);
        }
        let inner_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let _ = rt.tick();
            for batch in rt.take_stream_batches() {
                if batch.id == id && batch.closed {
                    break;
                }
            }
            if rt.is_complete(id) {
                let outcome = rt.take_result(id);
                debug_assert!(matches!(
                    outcome,
                    Some(JobOutcome::Complete(_) | JobOutcome::Cancelled | JobOutcome::Failed(_))
                        | None
                ));
                return;
            }
            if rt.workers_snapshot().active.iter().all(|a| a.id != id) {
                return;
            }
            assert!(
                Instant::now() < inner_deadline,
                "soak: per-cycle settle deadline at iter {iter}"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Drain the runtime to `pending_len() == 0` after the soak loop
    /// exits. Anything still pending must reach a terminal state and
    /// be evicted within a bounded window.
    pub(super) fn final_drain(rt: &AsyncRuntime) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while rt.pending_len() > 0 {
            assert!(
                Instant::now() < deadline,
                "soak: post-loop drain stuck at pending_len={}",
                rt.pending_len()
            );
            let _ = rt.tick();
            let _ = rt.take_stream_batches();
            let snap = rt.workers_snapshot();
            for a in snap.active {
                if rt.is_complete(a.id) {
                    let _ = rt.take_result(a.id);
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

#[test]
#[ignore = "long; default 10s, override with PMACS_SOAK_SECS=N (3600 for spec gate)"]
fn async_runtime_soak_lifecycle_stable() {
    use pmacs::async_runtime::AsyncRuntime;

    let secs: u64 = std::env::var("PMACS_SOAK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let rss_growth_pct: f64 = std::env::var("PMACS_SOAK_RSS_PCT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5.0);

    // Warm-up: a full dispatch / settle / take_result so the
    // baseline excludes first-touch costs and leaves zero pending.
    let rt = AsyncRuntime::with_pool_size(2);
    let warm = rt.dispatch_sleep(1, None);
    let warm_deadline = Instant::now() + Duration::from_secs(1);
    while !rt.is_complete(warm) {
        assert!(Instant::now() < warm_deadline, "warm-up dispatch stuck");
        let _ = rt.tick();
        std::thread::sleep(Duration::from_millis(1));
    }
    let _ = rt.take_result(warm);

    let baseline_fd = read_proc_fd_count();
    let baseline_rss = read_proc_rss_kib();
    let mut peak_pending: usize = 0;
    let mut peak_supersede: usize = 0;
    let mut iters: u64 = 0;

    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut state: u64 = 0xD17C_0DE5;
    let mut rng = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (state >> 33) as u32
    };

    while Instant::now() < deadline {
        iters += 1;
        soak_helpers::one_cycle(&rt, &mut rng, iters);
        peak_pending = peak_pending.max(rt.pending_len());
        peak_supersede = peak_supersede.max(rt.supersede_len());
    }

    soak_helpers::final_drain(&rt);

    eprintln!(
        "soak: {iters} iters in {secs}s, peak pending={peak_pending}, peak supersede={peak_supersede}"
    );
    assert_eq!(rt.pending_len(), 0, "post-soak pending leaked");
    assert_eq!(rt.supersede_len(), 0, "post-soak supersede leaked");

    // FD gate (Linux-only): tolerate up to +4 fds for caches the
    // stdlib opens lazily (timezone files, locale data, etc).
    if let (Some(b), Some(a)) = (baseline_fd, read_proc_fd_count()) {
        eprintln!("soak: fd baseline={b}, after={a}");
        assert!(
            a <= b + 4,
            "soak: file-descriptor leak --- baseline {b}, after {a}"
        );
    } else {
        eprintln!("soak: /proc/self/fd unavailable; FD gate skipped");
    }

    // RSS gate (Linux-only): scale the allowed growth to the soak
    // duration. The spec calls for 5%/hour; over a 10-second smoke
    // run the absolute slack is tiny, so we floor the allowance at
    // 8 MiB to keep the gate noise-tolerant for short runs.
    if let (Some(b), Some(a)) = (baseline_rss, read_proc_rss_kib()) {
        let scaled_pct = rss_growth_pct * (secs as f64) / 3600.0;
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let allowed_kib = (b as f64 * scaled_pct / 100.0).max(8.0 * 1024.0) as u64;
        let grew = a.saturating_sub(b);
        eprintln!(
            "soak: rss baseline={b} KiB, after={a} KiB, grew={grew} KiB, allowed={allowed_kib} KiB"
        );
        assert!(
            grew <= allowed_kib,
            "soak: RSS grew {grew} KiB over {secs}s; allowed {allowed_kib} KiB"
        );
    } else {
        eprintln!("soak: /proc/self/status unavailable; RSS gate skipped");
    }
}
