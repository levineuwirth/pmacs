// m10_11_perf.rs --- M10.11 perf gate: cross-frontend propagation.

//! T M10.11 perf gate (Q5).
//!
//! # Contract
//!
//! Per `M10.11-AUDIT.md` perf-gate ratchet: "50ms p99 budget for
//! `Key sent on stream_a → corresponding CellDelta read on stream_b`."
//! The 50ms budget is generous-but-honest:
//!
//! - M5.9's keystroke→local-render budget is 10ms p99 over loopback
//!   `LocalSocket`.
//! - M10.11 adds daemon dispatch + cross-frontend broadcast + remote
//!   read scheduling on top of M5.9's measured path. The 50ms
//!   ratchet absorbs that overhead with headroom.
//!
//! # Methodology
//!
//! Pinned here so future "is this regression real?" debates have a
//! single source of truth; mirrors M5.9's methodology where the
//! shape carries over.
//!
//! - **What "cross-frontend propagation" means.** The interval
//!   between A's `write_message(stream_a, FrontendEvent::Key)` and
//!   B's first `read_message(stream_b)` that returns
//!   `InstanceMessage::CellDelta`. Messages of other variants
//!   (`PresenceUpdate`, `CrdtOp`, `BufferSnapshot`, `Cursor`) are
//!   read-through (skipped without ending the wait) because they
//!   represent the daemon's broadcast path but are not the spec's
//!   "edits appear on both screens" observable. The `CellDelta` is.
//!
//! - **Sample count.** 100 warmup + 1000 measured (M5.9's precedent).
//!
//! - **Percentile computation.** `(len * p) / 100` integer
//!   arithmetic; index `(1000 * 99) / 100 = 990` is the 991st
//!   smallest sample for p99.
//!
//! - **Drain between iterations.** After reading B's `CellDelta`,
//!   drain followup frames on both streams (`Cursor`, additional
//!   `CellDelta`s from the same tick, presence broadcasts) with a
//!   1ms read timeout so the next iteration starts from a quiet
//!   socket.
//!
//! - **Character cycling.** A types `'a'..='z'` cycling per
//!   iteration; the test buffer never wraps a line (24×80 grid
//!   absorbs all 1100 keystrokes on row 0 with no soft-wrap).
//!
//! - **Threshold.** 50ms p99. Actual perf on a quiet developer
//!   machine is sub-millisecond (M5.9's machine measures sub-ms;
//!   M10.11 adds one broadcast hop, so a small multiple). The
//!   threshold catches catastrophic regressions, not subtle ones.
//!
//! # Why `#[ignore]`
//!
//! Perf measurement under debug-mode `cargo test` is meaningless —
//! the daemon's hot path doesn't optimize. CI runs this test under
//! a release-mode perf-gate job alongside M5.9's; local dev runs
//! (`cargo test`) skip it.

#![cfg(feature = "crdt")]

use std::time::{Duration, Instant};

use pmacs::protocol::{FrontendEvent, InstanceMessage, Key, KeyEvent, Modifiers};
use pmacs::transport::{TransportError, read_message, write_message};

mod common;
use common::daemon::{TestDaemon, attach_multi};

const WARMUP_SAMPLES: usize = 100;
const MEASURED_SAMPLES: usize = 1000;
const P99_THRESHOLD_MS: u128 = 50;
const PER_KEY_TIMEOUT: Duration = Duration::from_secs(5);
const DRAIN_TIMEOUT: Duration = Duration::from_millis(1);

#[test]
#[ignore = "perf gate; requires release build"]
fn m10_11_cross_frontend_propagation_p99_under_50ms() {
    let daemon = TestDaemon::spawn();
    let (hello_a, mut stream_a) = attach_multi(&daemon);
    let (_hello_b, mut stream_b) = attach_multi(&daemon);

    // Drain attach-time frames from both streams. Each replica
    // receives a BufferSnapshot for *scratch* plus initial
    // CellDelta + presence broadcasts; clear them so the first
    // measured keystroke starts from a quiet socket on both sides.
    drain_pending(&mut stream_a);
    drain_pending(&mut stream_b);

    let total = WARMUP_SAMPLES + MEASURED_SAMPLES;
    let mut samples: Vec<Duration> = Vec::with_capacity(total);
    let mut cursor: u8 = b'a';

    for i in 0..total {
        let key = FrontendEvent::Key(KeyEvent {
            frontend_id: hello_a.assigned_frontend_id,
            key: Key::Char(cursor as char),
            mods: Modifiers::NONE,
            timestamp_ns: 0,
        });
        cursor = if cursor >= b'z' { b'a' } else { cursor + 1 };

        stream_b
            .set_read_timeout(Some(PER_KEY_TIMEOUT))
            .expect("set per-key timeout");
        let t_send = Instant::now();
        write_message(&mut stream_a, &key).expect("send key from A");

        // Read B's stream until the first CellDelta arrives. Skip
        // other variants (PresenceUpdate, CrdtOp, BufferSnapshot,
        // Cursor) — they're part of the daemon's broadcast pipeline
        // but not the spec's "edits appear on screen" observable.
        loop {
            match read_message::<InstanceMessage>(&mut stream_b) {
                Ok(InstanceMessage::CellDelta { .. }) => break,
                Ok(_other) => {}
                Err(e) => panic!("read response on B for key #{i}: {e}"),
            }
        }
        let elapsed = t_send.elapsed();
        samples.push(elapsed);

        // Drain followup frames on both streams so the next
        // iteration starts quiet. A receives its own CellDelta /
        // Cursor; B may receive additional follow-up messages.
        drain_pending(&mut stream_a);
        drain_pending(&mut stream_b);
    }

    let measured = &samples[WARMUP_SAMPLES..];
    let mut sorted: Vec<Duration> = measured.to_vec();
    sorted.sort();

    let percentile = |p: usize| -> Duration {
        let idx = (sorted.len() * p) / 100;
        sorted[idx.min(sorted.len() - 1)]
    };
    let max = sorted[sorted.len() - 1];
    let p50 = percentile(50);
    let p90 = percentile(90);
    let p99 = percentile(99);

    println!(
        "M10.11 cross-frontend propagation over {} measured samples (after {} warmup):",
        measured.len(),
        WARMUP_SAMPLES
    );
    println!("  p50: {p50:?}");
    println!("  p90: {p90:?}");
    println!("  p99: {p99:?}");
    println!("  max: {max:?}");
    println!("  threshold: {P99_THRESHOLD_MS}ms");

    assert!(
        p99.as_millis() < P99_THRESHOLD_MS,
        "p99 cross-frontend latency {p99:?} exceeds {P99_THRESHOLD_MS}ms gate; \
         p50={p50:?}, p90={p90:?}, max={max:?}"
    );
}

/// Read-and-discard any pending frames on `stream` with a short
/// timeout. Returns the number of frames drained.
fn drain_pending(stream: &mut std::os::unix::net::UnixStream) -> usize {
    let mut count = 0;
    stream
        .set_read_timeout(Some(DRAIN_TIMEOUT))
        .expect("set drain timeout");
    loop {
        match read_message::<InstanceMessage>(stream) {
            Ok(_) => count += 1,
            Err(TransportError::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(_) => break,
        }
    }
    count
}
