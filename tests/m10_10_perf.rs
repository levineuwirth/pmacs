//! T M10.10 Day 3 step 7 — perf measurement baseline.
//!
//! Measures `Buffer::apply_remote_crdt_op` cost across buffer sizes
//! (1KB / 100KB / 1MB). Under Path β's narrow optimistic-paint
//! scope, M10.10 doesn't have a tight perf requirement — the
//! measurements are baseline numbers for v0.2+ optimization work:
//!
//! - **v0.2+ Path γ** (layout-aware optimistic paint) needs
//!   before-state to compare against.
//! - **v0.x A1 mitigation** (unicode-method path from M10.2's 391×
//!   finding) needs before-state to verify improvement.
//!
//! The numbers are recorded to stdout via eprintln (visible under
//! `cargo test -- --nocapture`) and asserted against generous bounds
//! that exist to catch catastrophic regressions, not to verify a
//! tight perf claim.

#![cfg(feature = "crdt")]

use std::time::Instant;

use pmacs::buffer::{Buffer, BufferId};
use pmacs::crdt::CrdtState;

/// Build a buffer of approximately `size` bytes seeded with ASCII
/// content, CRDT-upgraded under `peer_id` 1 (the LOCAL daemon peer).
/// Returns the buffer plus a remote-peer state synced with it for
/// generating ops.
fn build_buffer_with_size(size: usize) -> (Buffer, CrdtState) {
    // Seed text: 'a' repeated. Same content via both paths so the
    // buffer and donor CrdtState are byte-equivalent.
    let content: Vec<u8> = std::iter::repeat_n(b'a', size).collect();
    let buf = Buffer::from_bytes_with_crdt(BufferId::next(), "*perf*", &content, 1)
        .expect("buf from bytes with crdt");

    // Synchronize a donor (simulating a remote peer) to the buffer's
    // CRDT state via snapshot. Donor uses a distinct peer_id so the
    // ops it produces are attributable to a different peer.
    let donor_snap = buf
        .crdt_state()
        .expect("buf crdt")
        .export_snapshot()
        .expect("snap");
    let donor = CrdtState::new(2).expect("donor");
    donor.import_snapshot(&donor_snap).expect("donor import");

    (buf, donor)
}

/// Measure `apply_remote_crdt_op` for a buffer of `size` bytes
/// receiving a single 1-byte insertion at position 0 from a remote
/// peer. Returns elapsed time.
fn measure_apply_remote(size: usize) -> std::time::Duration {
    let (mut buf, donor) = build_buffer_with_size(size);

    // Donor produces a small op (insert one char at position 0).
    let v_before = donor.version();
    donor.insert(0, "X").expect("donor edit");
    let op_bytes = donor.export_updates_since(&v_before).expect("export");

    let start = Instant::now();
    let _edit = buf.apply_remote_crdt_op(&op_bytes).expect("apply remote");
    start.elapsed()
}

#[test]
fn m10_10_apply_remote_crdt_op_at_1kb() {
    let elapsed = measure_apply_remote(1024);
    let us = elapsed.as_micros();
    eprintln!("[M10.10 perf] apply_remote_crdt_op at 1 KB: {us}µs");
    // Generous bound: 1KB should never exceed 10ms even on slow CI.
    // Tight bound is recorded in audit, not asserted.
    assert!(
        elapsed < std::time::Duration::from_millis(10),
        "apply_remote_crdt_op at 1KB took {us}µs; expected sub-10ms"
    );
}

#[test]
fn m10_10_apply_remote_crdt_op_at_100kb() {
    let elapsed = measure_apply_remote(100 * 1024);
    let us = elapsed.as_micros();
    eprintln!("[M10.10 perf] apply_remote_crdt_op at 100 KB: {us}µs");
    // Generous bound: 100KB on CI should complete in under 200ms.
    // The audit records the actual measurement; this assertion is a
    // catastrophic-regression guard.
    assert!(
        elapsed < std::time::Duration::from_millis(200),
        "apply_remote_crdt_op at 100KB took {us}µs; expected sub-200ms"
    );
}

#[test]
fn m10_10_apply_remote_crdt_op_at_1mb() {
    // The 1MB case stresses the path. Under M10.2's 391× finding,
    // byte-native loro operations on multi-MB buffers can take tens
    // of ms per op (the unicode-method mitigation closes this gap).
    // M10.10's perf measurement records the before-state.
    let elapsed = measure_apply_remote(1024 * 1024);
    let ms = elapsed.as_millis();
    eprintln!("[M10.10 perf] apply_remote_crdt_op at 1 MB: {ms}ms");
    // Very generous bound: 1MB on CI should complete in under 5s.
    // If we exceed this, something is structurally wrong (not just
    // the M10.2 391× pattern).
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "apply_remote_crdt_op at 1MB took {ms}ms; expected sub-5s"
    );
}

/// Records buffer-size scaling in one test for the audit's perf
/// table. Runs three measurements and prints them together so
/// `cargo test -- --nocapture` shows the scaling pattern at a
/// glance.
#[test]
fn m10_10_apply_remote_crdt_op_scaling_report() {
    let sizes = [1024usize, 100 * 1024, 1024 * 1024];
    eprintln!("\n[M10.10 perf scaling]");
    eprintln!("  size    | apply_remote_crdt_op");
    eprintln!("  --------|---------------------");
    for size in sizes {
        // Run three iterations and report the median; smooths out
        // outliers from cold-start and noisy CI runners.
        let mut samples: Vec<_> = (0..3).map(|_| measure_apply_remote(size)).collect();
        samples.sort();
        let median = samples[1];
        let label = match size {
            n if n < 10 * 1024 => format!("{n} B"),
            n if n < 10 * 1024 * 1024 => format!("{} KB", n / 1024),
            n => format!("{} MB", n / (1024 * 1024)),
        };
        eprintln!(
            "  {label:7} | {} µs ({} ms)",
            median.as_micros(),
            median.as_millis()
        );
    }
    eprintln!();
}
