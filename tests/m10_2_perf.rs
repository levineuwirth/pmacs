//! T M10.2 Day 7 — performance regression check.
//!
//! Measures CRDT-mode buffer performance against v0.1-mode buffer
//! performance across four axes, with methodology matching M10.1's
//! library-survey benchmarks so numbers are directly comparable.
//!
//! All tests are `#[ignore]` — they don't run in CI. Invocation:
//!
//! ```sh
//! cargo test --release --features "luajit crdt" \
//!     --test m10_2_perf -- --ignored --nocapture
//! ```
//!
//! Methodology (pinned for reproducibility against M10.1 + future
//! re-runs):
//! - Document size points: 1KB / 100KB / 1MB / 10MB
//! - Mixed-workload mix: 50% inserts / 30% deletes / 15% replaces / 5% large
//! - Op-size distribution: log-normal mu=1.1 sigma=1.5 for small,
//!   mu=6 sigma=1.5 for large (matches M10.1)
//! - Deterministic seed: 0xc0ffee (matches M10.1)
//! - Window: 30s for the audit numbers; 5s for development iteration
//! - Initial document for mixed workload: 100KB
//! - Release profile, single-thread
//!
//! Reports printed to stderr via `eprintln!` (visible with --nocapture).

#![cfg(feature = "crdt")]
#![allow(
    clippy::uninlined_format_args,
    clippy::unreadable_literal,
    reason = "perf bench: table-formatted numeric output reads better column-aligned than inline"
)]

use pmacs::buffer::{Buffer, BufferId, EditOp};
use pmacs::rope::Range;
use rand::{Rng, SeedableRng};
use rand_distr::{Distribution, LogNormal};
use std::time::{Duration, Instant};

const SEED: u64 = 0xc0ffee;
const WINDOW_SECS: u64 = 30;
const ASCII_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz ABCDEFGHIJKLMNOPQRSTUVWXYZ\n";

fn fmt_size(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{} MB", n / 1_000_000)
    } else if n >= 1_000 {
        format!("{} KB", n / 1_000)
    } else {
        format!("{} B", n)
    }
}

fn random_ascii(n: usize, rng: &mut impl Rng) -> Vec<u8> {
    (0..n)
        .map(|_| ASCII_ALPHABET[rng.r#gen_range(0..ASCII_ALPHABET.len())])
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Microbenchmark — document-size sweep.
//
// Bulk insert + snapshot (Arc-clone of rope) at 4 size points,
// measured for both modes. The bulk insert exercises the rope's
// build path; the snapshot exercises the worker-facing handoff.
//
// Expected: v0.1 mode unchanged (no CRDT field touched). CRDT mode
// pays loro's bulk-insert + version-capture/export overhead per call.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "perf bench; release-mode-only via --ignored --nocapture"]
fn perf_document_size_sweep() {
    let sizes = [1_000usize, 100_000, 1_000_000, 10_000_000];
    eprintln!("\n=== M10.2 Day 7 — document size sweep ===\n");
    eprintln!(
        "{:>8} | {:>14} | {:>14} | {:>10}",
        "size", "v0.1 bulk", "CRDT bulk", "ratio"
    );
    eprintln!("{}", "-".repeat(60));
    let mut rng = rand::rngs::StdRng::seed_from_u64(SEED);
    for &size in &sizes {
        let payload = random_ascii(size, &mut rng);

        // v0.1 mode: build via Buffer::from_bytes.
        let t = Instant::now();
        let _b_v01 = Buffer::from_bytes(BufferId::next(), "v01", &payload);
        let v01_us = t.elapsed().as_micros();

        // CRDT mode: build via Buffer::from_bytes_with_crdt.
        let t = Instant::now();
        let _b_crdt = Buffer::from_bytes_with_crdt(BufferId::next(), "crdt", &payload, 1)
            .expect("crdt construct");
        let crdt_us = t.elapsed().as_micros();

        let ratio = if v01_us > 0 {
            crdt_us as f64 / v01_us as f64
        } else {
            f64::NAN
        };
        eprintln!(
            "{:>8} | {:>11} us | {:>11} us | {:>9.2}x",
            fmt_size(size),
            v01_us,
            crdt_us,
            ratio
        );
    }
    eprintln!();
}

// ---------------------------------------------------------------------------
// 2. Per-op throughput — mixed workload (M10.1 methodology).
//
// 50/30/15/5 mix, log-normal op sizes, 30s window. Comparable to the
// M10.1 library benchmarks (loro: 279k ops/sec at 30s; yrs: 1.5k).
// pmacs's Buffer adds intercept dispatch, mark adjustment, undo
// bookkeeping, and on_edit broadcast on top of the underlying CRDT
// or rope operations — those overheads are part of the measurement.
//
// Expected: v0.1 mode in the hundreds of thousands of ops/sec range
// (rope edits are cheap). CRDT mode pays additional cost per op
// (CRDT apply + Day 3 crdt_op extraction).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum OpKind {
    Insert,
    Delete,
    Replace,
    LargeOp,
}

fn pick_op<R: Rng>(rng: &mut R) -> OpKind {
    let r: f64 = rng.r#gen();
    if r < 0.50 {
        OpKind::Insert
    } else if r < 0.80 {
        OpKind::Delete
    } else if r < 0.95 {
        OpKind::Replace
    } else {
        OpKind::LargeOp
    }
}

fn op_size_lognormal<R: Rng>(rng: &mut R, large: bool) -> usize {
    let mu = if large { 6.0 } else { 1.1 };
    let dist = LogNormal::new(mu, 1.5).unwrap();
    let v: f64 = dist.sample(rng);
    let n = v.round() as usize;
    n.clamp(1, if large { 5_000 } else { 50 })
}

fn run_workload(buf: &mut Buffer, window: Duration) -> u64 {
    let mut rng = rand::rngs::StdRng::seed_from_u64(SEED);
    let deadline = Instant::now() + window;
    let mut ops = 0u64;
    while Instant::now() < deadline {
        let kind = pick_op(&mut rng);
        let len = buf.len() as usize;
        if len == 0 {
            let _ = buf.apply_edit(EditOp::Insert {
                pos: 0,
                bytes: b"x",
            });
            ops += 1;
            continue;
        }
        match kind {
            OpKind::Insert => {
                let size = op_size_lognormal(&mut rng, false);
                let pos = rng.r#gen_range(0..=len) as u64;
                let bytes = random_ascii(size, &mut rng);
                let _ = buf.apply_edit(EditOp::Insert { pos, bytes: &bytes });
            }
            OpKind::Delete => {
                let l = op_size_lognormal(&mut rng, false).min(len);
                let pos = rng.r#gen_range(0..=len.saturating_sub(l)) as u64;
                let _ = buf.apply_edit(EditOp::Delete {
                    range: Range::new(pos, pos + l as u64),
                });
            }
            OpKind::Replace => {
                let l = op_size_lognormal(&mut rng, false).min(len);
                let new_size = op_size_lognormal(&mut rng, false);
                let pos = rng.r#gen_range(0..=len.saturating_sub(l)) as u64;
                let bytes = random_ascii(new_size, &mut rng);
                let _ = buf.apply_edit(EditOp::Replace {
                    range: Range::new(pos, pos + l as u64),
                    bytes: &bytes,
                });
            }
            OpKind::LargeOp => {
                let size = op_size_lognormal(&mut rng, true);
                let pos = rng.r#gen_range(0..=len) as u64;
                let bytes = random_ascii(size, &mut rng);
                let _ = buf.apply_edit(EditOp::Insert { pos, bytes: &bytes });
            }
        }
        ops += 1;
    }
    ops
}

/// Run M10.1-style mixed workload against a bare `CrdtState` (no
/// Buffer wrapper). Returns the op count over the window. Mirrors
/// `run_workload` but for `CrdtState`'s byte-native methods.
fn run_workload_bare_crdt(state: &pmacs::crdt::CrdtState, window: Duration) -> u64 {
    let mut rng = rand::rngs::StdRng::seed_from_u64(SEED);
    let deadline = Instant::now() + window;
    let mut ops = 0u64;
    while Instant::now() < deadline {
        let kind = pick_op(&mut rng);
        let len = state.len_utf8();
        if len == 0 {
            let _ = state.insert(0, "x");
            ops += 1;
            continue;
        }
        match kind {
            OpKind::Insert => {
                let size = op_size_lognormal(&mut rng, false);
                let pos = rng.r#gen_range(0..=len);
                let bytes = random_ascii(size, &mut rng);
                let s = std::str::from_utf8(&bytes).expect("ASCII");
                let _ = state.insert(pos, s);
            }
            OpKind::Delete => {
                let l = op_size_lognormal(&mut rng, false).min(len);
                let pos = rng.r#gen_range(0..=len.saturating_sub(l));
                let _ = state.delete(pos, l);
            }
            OpKind::Replace => {
                let l = op_size_lognormal(&mut rng, false).min(len);
                let new_size = op_size_lognormal(&mut rng, false);
                let pos = rng.r#gen_range(0..=len.saturating_sub(l));
                let bytes = random_ascii(new_size, &mut rng);
                let s = std::str::from_utf8(&bytes).expect("ASCII");
                let _ = state.delete(pos, l);
                let _ = state.insert(pos, s);
            }
            OpKind::LargeOp => {
                let size = op_size_lognormal(&mut rng, true);
                let pos = rng.r#gen_range(0..=len);
                let bytes = random_ascii(size, &mut rng);
                let s = std::str::from_utf8(&bytes).expect("ASCII");
                let _ = state.insert(pos, s);
            }
        }
        ops += 1;
    }
    ops
}

/// Methodology-reconciliation bench: M10.1 measured 314,691 ops/sec
/// for bare loro at the same mixed-workload methodology. Day 7's
/// initial export-overhead-isolation test measured 41 µs/op for
/// bare `CrdtState` — but on a *different* workload (sequential
/// append from empty, no deletes/replaces). This test re-runs
/// M10.1's exact methodology against bare `CrdtState` to determine
/// whether the gap is workload-shape (expected) or regression
/// (alarming).
/// Bare-loro mixed workload using `insert` (unicode positions) —
/// matches M10.1's methodology exactly. If this produces ~3 µs/op
/// it confirms that the gap between M10.1 (unicode path) and Day 7
/// (byte-native path) is the byte-vs-unicode method choice, not a
/// regression.
#[test]
#[ignore = "perf bench; release-mode-only via --ignored --nocapture"]
fn perf_bare_loro_unicode_path_matches_m10_1() {
    use loro::LoroDoc;
    eprintln!("\n=== M10.2 Day 7 — direct loro unicode-path bench (M10.1 replica) ===\n");
    eprintln!("Uses text.insert/delete (unicode positions), matching M10.1's methodology.\n");

    let mut seed_rng = rand::rngs::StdRng::seed_from_u64(SEED);
    let seed_bytes = random_ascii(100_000, &mut seed_rng);
    let doc = LoroDoc::new();
    doc.set_peer_id(1).expect("peer");
    let text = doc.get_text("body");
    text.insert(0, std::str::from_utf8(&seed_bytes).expect("ASCII seed"))
        .expect("seed");

    let mut rng = rand::rngs::StdRng::seed_from_u64(SEED);
    let deadline = Instant::now() + Duration::from_secs(WINDOW_SECS);
    let mut ops = 0u64;
    while Instant::now() < deadline {
        let kind = pick_op(&mut rng);
        let len = text.len_unicode();
        if len == 0 {
            let _ = text.insert(0, "x");
            ops += 1;
            continue;
        }
        match kind {
            OpKind::Insert => {
                let size = op_size_lognormal(&mut rng, false);
                let pos = rng.r#gen_range(0..=len);
                let bytes = random_ascii(size, &mut rng);
                let _ = text.insert(pos, std::str::from_utf8(&bytes).expect("ASCII"));
            }
            OpKind::Delete => {
                let l = op_size_lognormal(&mut rng, false).min(len);
                let pos = rng.r#gen_range(0..=len.saturating_sub(l));
                let _ = text.delete(pos, l);
            }
            OpKind::Replace => {
                let l = op_size_lognormal(&mut rng, false).min(len);
                let new_size = op_size_lognormal(&mut rng, false);
                let pos = rng.r#gen_range(0..=len.saturating_sub(l));
                let bytes = random_ascii(new_size, &mut rng);
                let _ = text.delete(pos, l);
                let _ = text.insert(pos, std::str::from_utf8(&bytes).expect("ASCII"));
            }
            OpKind::LargeOp => {
                let size = op_size_lognormal(&mut rng, true);
                let pos = rng.r#gen_range(0..=len);
                let bytes = random_ascii(size, &mut rng);
                let _ = text.insert(pos, std::str::from_utf8(&bytes).expect("ASCII"));
            }
        }
        ops += 1;
    }
    let per_sec = ops as f64 / WINDOW_SECS as f64;
    let us = 1_000_000.0 / per_sec;
    eprintln!(
        "bare loro (unicode path, M10.1 replica): {:>9} ops | {:>9.0} ops/sec | {:>7.2} us/op",
        ops, per_sec, us
    );
    eprintln!();
    eprintln!("If close to 314,691 ops/sec (M10.1's number): confirms the byte-native");
    eprintln!("methods (insert_utf8/delete_utf8) are dramatically more expensive than");
    eprintln!("the unicode methods (insert/delete) at non-trivial doc sizes.");
}

#[test]
#[ignore = "perf bench; release-mode-only via --ignored --nocapture"]
fn perf_bare_crdt_mixed_workload_reconcile_with_m10_1() {
    use pmacs::crdt::CrdtState;
    eprintln!("\n=== M10.2 Day 7 reconciliation — bare CrdtState mixed workload ===\n");
    eprintln!("Methodology: M10.1's exact pattern (50/30/15/5, log-normal sizes, 30s, 100KB seed)");
    eprintln!("Comparison target: M10.1's bare-loro number was 314,691 ops/sec (3.18 µs/op)\n");

    let mut seed_rng = rand::rngs::StdRng::seed_from_u64(SEED);
    let seed_bytes = random_ascii(100_000, &mut seed_rng);
    let state = CrdtState::from_bytes(1, &seed_bytes).expect("seed");

    let bare_ops = run_workload_bare_crdt(&state, Duration::from_secs(WINDOW_SECS));
    let bare_per_sec = bare_ops as f64 / WINDOW_SECS as f64;
    let bare_us = 1_000_000.0 / bare_per_sec;
    let final_len = state.len_utf8();
    eprintln!(
        "bare CrdtState (M10.1 methodology): {:>9} ops | {:>9.0} ops/sec | {:>7.2} us/op | final doc {} B",
        bare_ops, bare_per_sec, bare_us, final_len
    );
    eprintln!();
    eprintln!("Reconciliation:");
    eprintln!("  M10.1 bare loro (mixed workload):    314,691 ops/sec");
    eprintln!(
        "  Day 7 bare CrdtState (mixed):     {:>9.0} ops/sec",
        bare_per_sec
    );
    let m101_ratio = 314_691.0 / bare_per_sec;
    eprintln!(
        "  ratio:                            {:>8.2}x slower than M10.1",
        m101_ratio
    );
    eprintln!();
    eprintln!("If close to 1x: workload-shape was the gap (Day 7's export-overhead-");
    eprintln!("  isolation used sequential append-from-empty, not M10.1's mixed).");
    eprintln!("If much greater than 1x: real regression vs M10.1 worth investigating.");
    eprintln!();
}

#[test]
#[ignore = "perf bench; release-mode-only via --ignored --nocapture"]
fn perf_mixed_workload_throughput() {
    eprintln!("\n=== M10.2 Day 7 — per-op throughput (mixed workload, 30s) ===\n");
    eprintln!(
        "Initial doc: 100KB; mix: 50/30/15/5; window: {}s\n",
        WINDOW_SECS
    );

    let mut seed_rng = rand::rngs::StdRng::seed_from_u64(SEED);
    let seed_bytes = random_ascii(100_000, &mut seed_rng);

    // v0.1 mode
    let mut b_v01 = Buffer::from_bytes(BufferId::next(), "v01", &seed_bytes);
    let v01_ops = run_workload(&mut b_v01, Duration::from_secs(WINDOW_SECS));
    let v01_per_sec = v01_ops as f64 / WINDOW_SECS as f64;
    let v01_us = 1_000_000.0 / v01_per_sec;
    eprintln!(
        "v0.1 mode: {:>9} ops total | {:>9.0} ops/sec | {:>7.2} us/op",
        v01_ops, v01_per_sec, v01_us
    );

    // CRDT mode
    let mut b_crdt =
        Buffer::from_bytes_with_crdt(BufferId::next(), "crdt", &seed_bytes, 1).expect("crdt");
    let crdt_ops = run_workload(&mut b_crdt, Duration::from_secs(WINDOW_SECS));
    let crdt_per_sec = crdt_ops as f64 / WINDOW_SECS as f64;
    let crdt_us = 1_000_000.0 / crdt_per_sec;
    eprintln!(
        "CRDT mode: {:>9} ops total | {:>9.0} ops/sec | {:>7.2} us/op",
        crdt_ops, crdt_per_sec, crdt_us
    );

    let ratio = v01_per_sec / crdt_per_sec;
    eprintln!("\nCRDT mode is {:.2}x slower per op than v0.1 mode", ratio);
    eprintln!("(M10.2 target: within 2x of v0.1 for typical edit patterns)\n");
}

// ---------------------------------------------------------------------------
// 3. Export overhead — CRDT mode without crdt_op extraction vs with.
//
// Day 3's framing called for this specific measurement: separate the
// cost of "apply CRDT op" from the cost of "export the delta bytes."
// The wrapper always extracts; to measure without, we time the loro
// underlying ops directly (via the CrdtState wrapper) against the full
// Buffer::apply_edit path.
//
// Specifically:
//   - bare_crdt: time N inserts into CrdtState directly (no wrapper)
//   - with_extraction: time N inserts via Buffer::apply_edit (full path,
//     includes version_capture / op application / export)
//   - difference = wrapper overhead (extraction + rope-sync + bookkeeping)
//
// This isn't a perfectly-isolated "extraction only" measurement
// because the wrapper also does rope-sync and undo bookkeeping. But
// it scopes the wrapper cost so the audit can record both numbers.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "perf bench; release-mode-only via --ignored --nocapture"]
fn perf_export_overhead_isolation() {
    use pmacs::crdt::CrdtState;
    const N_OPS: usize = 10_000;
    eprintln!("\n=== M10.2 Day 7 — export overhead isolation ===\n");
    eprintln!(
        "Workload: {} sequential inserts, each ~5 bytes of ASCII\n",
        N_OPS
    );
    let mut rng = rand::rngs::StdRng::seed_from_u64(SEED);
    let payload: Vec<Vec<u8>> = (0..N_OPS).map(|_| random_ascii(5, &mut rng)).collect();

    // 3a. Bare CRDT (no wrapper, no extraction).
    let state = CrdtState::new(1).expect("crdt");
    let t = Instant::now();
    for bytes in &payload {
        let s = std::str::from_utf8(bytes).expect("ASCII");
        state.insert(state.len_utf8(), s).expect("insert");
    }
    let bare_us = t.elapsed().as_micros() as f64;
    eprintln!(
        "bare CrdtState (no wrapper):       {:>10.0} us total | {:>6.2} us/op",
        bare_us,
        bare_us / N_OPS as f64
    );

    // 3b. Bare CRDT + per-op export (the extraction step in isolation).
    let state = CrdtState::new(2).expect("crdt");
    let t = Instant::now();
    for bytes in &payload {
        let pre = state.version();
        let s = std::str::from_utf8(bytes).expect("ASCII");
        state.insert(state.len_utf8(), s).expect("insert");
        let _ = state.export_updates_since(&pre).expect("export");
    }
    let with_export_us = t.elapsed().as_micros() as f64;
    eprintln!(
        "bare CrdtState + per-op export:    {:>10.0} us total | {:>6.2} us/op",
        with_export_us,
        with_export_us / N_OPS as f64
    );
    let export_only_us_per_op = (with_export_us - bare_us) / N_OPS as f64;
    eprintln!(
        "    → export-only overhead:        {:>6.2} us/op",
        export_only_us_per_op
    );

    // 3c. Full Buffer::apply_edit (CRDT mode). Includes:
    //   - intercept dispatch (no intercepts attached, so cheap)
    //   - lossy UTF-8 normalization (no-op for ASCII)
    //   - version capture + CRDT apply + export
    //   - rope mutation
    //   - mark adjustment (no marks attached, so cheap)
    //   - undo stack push
    //   - on_edit broadcast (no views, so cheap)
    let mut buf = Buffer::new_with_crdt(BufferId::next(), "full", 3).expect("crdt buf");
    let t = Instant::now();
    for bytes in &payload {
        let pos = buf.len();
        let _ = buf.apply_edit(EditOp::Insert { pos, bytes }).expect("ins");
    }
    let full_us = t.elapsed().as_micros() as f64;
    eprintln!(
        "Buffer::apply_edit (CRDT mode):    {:>10.0} us total | {:>6.2} us/op",
        full_us,
        full_us / N_OPS as f64
    );
    let wrapper_us_per_op = (full_us - with_export_us) / N_OPS as f64;
    eprintln!(
        "    → wrapper overhead (rope + bookkeeping): {:>6.2} us/op",
        wrapper_us_per_op
    );

    // 3d. Full Buffer::apply_edit (v0.1 mode), for comparison.
    let mut buf_v01 = Buffer::new(BufferId::next(), "v01");
    let t = Instant::now();
    for bytes in &payload {
        let pos = buf_v01.len();
        let _ = buf_v01
            .apply_edit(EditOp::Insert { pos, bytes })
            .expect("ins");
    }
    let v01_us = t.elapsed().as_micros() as f64;
    eprintln!(
        "Buffer::apply_edit (v0.1 mode):    {:>10.0} us total | {:>6.2} us/op",
        v01_us,
        v01_us / N_OPS as f64
    );

    eprintln!();
    eprintln!("Decomposition (per-op):");
    eprintln!("  raw CRDT op:          {:>6.2} us", bare_us / N_OPS as f64);
    eprintln!(
        "  + export extraction:  {:>6.2} us  (Day 3 cost)",
        export_only_us_per_op
    );
    eprintln!(
        "  + wrapper overhead:   {:>6.2} us  (rope sync + bookkeeping)",
        wrapper_us_per_op
    );
    eprintln!("  = full CRDT mode:     {:>6.2} us", full_us / N_OPS as f64);
    eprintln!("  v0.1 mode baseline:   {:>6.2} us", v01_us / N_OPS as f64);
    eprintln!();
}

// ---------------------------------------------------------------------------
// 4. Undo cost scaling.
//
// Day 2 framing noted undo cost scales with the size of the undone
// edit in CRDT mode (the synthetic-Replace op carries the full pre-
// edit content). Confirm linear scaling; surface superlinear if
// present.
//
// Methodology: build a buffer, apply an N-byte edit, time the undo.
// N in {10, 100, 1000, 10000}. Run both modes. v0.1 should be ~flat
// (pre-edit rope is held via Arc); CRDT should scale with N (synthetic
// Replace reads pre-edit bytes from the saved rope).
// ---------------------------------------------------------------------------

#[test]
#[ignore = "perf bench; release-mode-only via --ignored --nocapture"]
fn perf_undo_cost_scaling() {
    eprintln!("\n=== M10.2 Day 7 — undo cost scaling ===\n");
    eprintln!(
        "{:>9} | {:>14} | {:>14} | {:>10}",
        "edit size", "v0.1 undo", "CRDT undo", "ratio"
    );
    eprintln!("{}", "-".repeat(60));
    let sizes = [10usize, 100, 1_000, 10_000];
    let mut rng = rand::rngs::StdRng::seed_from_u64(SEED);
    for &n in &sizes {
        let payload = random_ascii(n, &mut rng);

        // v0.1 mode
        let mut b_v01 = Buffer::new(BufferId::next(), "v01");
        b_v01
            .apply_edit(EditOp::Insert {
                pos: 0,
                bytes: &payload,
            })
            .unwrap();
        let t = Instant::now();
        b_v01.undo().expect("v01 undo");
        let v01_us = t.elapsed().as_micros();

        // CRDT mode
        let mut b_crdt = Buffer::new_with_crdt(BufferId::next(), "crdt", 1).expect("crdt buf");
        b_crdt
            .apply_edit(EditOp::Insert {
                pos: 0,
                bytes: &payload,
            })
            .unwrap();
        let t = Instant::now();
        b_crdt.undo().expect("crdt undo");
        let crdt_us = t.elapsed().as_micros();

        let ratio = if v01_us > 0 {
            crdt_us as f64 / v01_us as f64
        } else {
            f64::NAN
        };
        eprintln!(
            "{:>9} B | {:>11} us | {:>11} us | {:>9.2}x",
            n, v01_us, crdt_us, ratio
        );
    }
    eprintln!();
}
