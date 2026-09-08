//! M5 perf gate (T M5.9c).
//!
//! # Contract
//!
//! Per `pmacs-tasks.tex` T M5.9: "keystroke-to-render round-trip
//! over a loopback `LocalSocket` attach is under 10ms at the p99."
//!
//! # Methodology
//!
//! Definitions, locked here so future "is this regression real or
//! measurement noise?" debates have a single source of truth:
//!
//! - **What "keystroke-to-render" means.** The interval between the
//!   moment the test's frontend writes a `FrontendEvent::Key` frame
//!   to the Unix socket and the moment it receives the *first*
//!   `InstanceMessage` produced in response (`CellDelta`, `Cursor`,
//!   or any other render-bearing variant). This includes the
//!   daemon's full pipeline — protocol decode, key dispatch via the
//!   keymap, command execution, buffer mutation, repaint, diff,
//!   protocol encode — and the wire RTT through the kernel's Unix
//!   socket buffer. It excludes the frontend's escape-sequence
//!   emission and the terminal's own paint, since the test has no
//!   terminal; this matches the spec's "over loopback `LocalSocket`
//!   attach" qualifier.
//!
//! - **Sample count.** 100 warmup keystrokes (discarded) + 1000
//!   measured keystrokes. The warmup absorbs first-call costs:
//!   socket buffer allocation, kernel page-in, and any
//!   one-time-per-process state in the daemon (`LuaJIT` trace
//!   compilation if applicable, hot-path allocator pools warming up).
//!
//! - **Percentile computation.** Across the full 1000-sample run,
//!   not a sliding window. Sort, take index `(len * 99) / 100` for
//!   p99. The ratio is exact for any `len`; for `len = 1000` this
//!   is index 990 (the 991st smallest of 1000 samples).
//!
//! - **Drain between iterations.** Daemon may emit multiple frames
//!   per keystroke (`CellDelta` + `Cursor` + `ModeLine`). After
//!   recording the latency to the first frame, drain remaining
//!   pending frames with a 1ms read timeout before sending the
//!   next key. Without the drain, key N+1's measurement would
//!   absorb key N's tail messages and skew low.
//!
//! - **Loopback configuration.** `tempfile::TempDir` for the socket
//!   directory. On Linux this is typically `/tmp` (often tmpfs, no
//!   disk hop); on macOS CI this is `/var/folders/...` (HFS / APFS,
//!   one disk hop). Both are fine for the gate — a 10ms threshold
//!   has plenty of headroom over either.
//!
//! - **Threshold.** 10ms p99. Actual perf on a quiet developer
//!   machine is sub-millisecond. The threshold is a regression
//!   gate, not a target.
//!
//! # Why `#[ignore]`
//!
//! Perf measurement under debug-mode `cargo test` is meaningless —
//! the daemon's hot path doesn't optimize. CI runs this test
//! release-mode via the `m5-perf-gates` workflow job
//! (`cargo test --release --test m5_perf_acceptance -- --ignored
//! --nocapture`). Local dev runs (`cargo test`) skip it.

use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

use pmacs::cell::CellSize;
use pmacs::protocol::{
    ADVERTISED_PROTOCOL_VERSION, AttachRequest, FrontendCapabilities, FrontendEvent, Hello,
    InstanceMessage, Key, KeyEvent, Modifiers,
};
use pmacs::transport::{TransportError, read_message, write_message};

// ---------------------------------------------------------------------------
// Daemon harness (inline; see m5_5_acceptance.rs for the canonical version)
// ---------------------------------------------------------------------------

struct TestDaemon {
    _tempdir: TempDir,
    socket_path: PathBuf,
    // Held for its Drop, which reaps the daemon's process group.
    _process: reap::Reaped,
}

impl TestDaemon {
    fn spawn() -> Self {
        let tempdir = TempDir::new().expect("tempdir");
        fs::set_permissions(tempdir.path(), fs::Permissions::from_mode(0o700))
            .expect("chmod tempdir 0700");
        let socket_path = tempdir.path().join("pmacs.sock");
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pmacs"));
        cmd.args(["--daemon", "--socket"])
            .arg(&socket_path)
            .env("HOME", tempdir.path())
            .env("XDG_CONFIG_HOME", tempdir.path())
            .env("XDG_DATA_HOME", tempdir.path())
            .env("XDG_STATE_HOME", tempdir.path())
            .env("PMACS_STATE_HOME", tempdir.path())
            .env("XDG_CACHE_HOME", tempdir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // In its own process group, reaped with everything it spawned
        // when the fixture drops (`tests/common/reap.rs`).
        let process = reap::Reaped::spawn(&mut cmd).expect("spawn pmacs --daemon");
        let mut process = process;
        ready::wait_for_daemon(&socket_path, process.child(), Duration::from_secs(10))
            .expect("daemon socket appears");
        Self {
            _tempdir: tempdir,
            socket_path,
            _process: process,
        }
    }

    fn connect(&self) -> UnixStream {
        UnixStream::connect(&self.socket_path).expect("connect")
    }
}

fn build_default_caps() -> FrontendCapabilities {
    FrontendCapabilities {
        synchronized_output: true,
        unicode_smp: true,
        true_color: true,
        mouse: true,
        bracketed_paste: true,
        terminal_kind: Some("perf-gate".into()),
        multi_frontend: false,
        crdt_replica: false,
        semantic_render: false,
    }
}

fn do_handshake(stream: &mut UnixStream) -> Hello {
    let hello: Hello = read_message(stream).expect("read Hello");
    assert_eq!(hello.protocol_version, ADVERTISED_PROTOCOL_VERSION);
    let req = AttachRequest {
        protocol_version: hello.protocol_version,
        frontend_capabilities: build_default_caps(),
        initial_size: CellSize::new(24, 80),
    };
    write_message(stream, &req).expect("write AttachRequest");
    hello
}

/// Drain any pending instance messages with a short read timeout.
/// Returns the count drained (for diagnostics in case of skew).
fn drain_pending(stream: &mut UnixStream) -> usize {
    stream
        .set_read_timeout(Some(Duration::from_millis(1)))
        .expect("set drain timeout");
    let mut n = 0;
    loop {
        match read_message::<InstanceMessage>(stream) {
            Ok(_) => n += 1,
            // Timeout = no more pending. Either stdlib or transport
            // surfaces this as a kind we don't try to enumerate;
            // treat any error as "drain complete" since the goal is
            // "the socket is quiet, time to send the next key".
            Err(TransportError::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                return n;
            }
            Err(_) => return n,
        }
    }
}

// ---------------------------------------------------------------------------
// Perf gate
// ---------------------------------------------------------------------------

const WARMUP_SAMPLES: usize = 100;
const MEASURED_SAMPLES: usize = 1000;
const P99_THRESHOLD_MS: u128 = 10;
const PER_KEY_TIMEOUT: Duration = Duration::from_secs(5);

#[test]
#[ignore = "perf gate; requires release build"]
fn keystroke_to_render_p99_under_10ms_over_loopback_local_socket() {
    let daemon = TestDaemon::spawn();
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let hello = do_handshake(&mut stream);

    // Drain initial full-grid sync (and any followup Cursor /
    // ModeLine frames). The first keystroke's measurement starts
    // from a quiet socket.
    let _initial: InstanceMessage = read_message(&mut stream).expect("initial frame");
    let _ = drain_pending(&mut stream);

    let total = WARMUP_SAMPLES + MEASURED_SAMPLES;
    let mut samples: Vec<Duration> = Vec::with_capacity(total);
    let mut cursor: u8 = b'a';

    for i in 0..total {
        let key = FrontendEvent::Key(KeyEvent {
            frontend_id: hello.assigned_frontend_id,
            key: Key::Char(cursor as char),
            mods: Modifiers::NONE,
            timestamp_ns: 0,
        });
        // Cycle through a-z so we never overflow a line of the
        // 24×80 grid in a way that triggers wrap-induced full-line
        // repaints (which would skew a small fraction of samples).
        cursor = if cursor >= b'z' { b'a' } else { cursor + 1 };

        stream
            .set_read_timeout(Some(PER_KEY_TIMEOUT))
            .expect("set per-key timeout");
        let t_send = Instant::now();
        write_message(&mut stream, &key).expect("send key");
        let _: InstanceMessage =
            read_message(&mut stream).unwrap_or_else(|e| panic!("read response for key #{i}: {e}"));
        let elapsed = t_send.elapsed();
        samples.push(elapsed);

        // Drain any followup frames so the next iteration starts
        // from a quiet socket.
        let _ = drain_pending(&mut stream);
    }

    let measured = &samples[WARMUP_SAMPLES..];
    let mut sorted: Vec<Duration> = measured.to_vec();
    sorted.sort();

    let percentile = |p: usize| -> Duration {
        // (len * p) / 100 — exact integer arithmetic, no float
        // rounding to argue about. p=99 on len=1000 → idx=990 → the
        // 991st smallest sample (the smallest 99% are at-or-below).
        let idx = (sorted.len() * p) / 100;
        sorted[idx.min(sorted.len() - 1)]
    };
    let max = sorted[sorted.len() - 1];

    let p50 = percentile(50);
    let p90 = percentile(90);
    let p99 = percentile(99);

    println!(
        "keystroke→render latency over {} measured samples (after {} warmup):",
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
        "p99 latency {p99:?} exceeds {P99_THRESHOLD_MS}ms gate; \
         p50={p50:?}, p90={p90:?}, max={max:?}"
    );
}

#[path = "common/reap.rs"]
mod reap;

#[path = "common/ready.rs"]
mod ready;
