// m10_11_acceptance.rs --- M10 acceptance test: two-laptop edit.

//! T M10.11 acceptance suite. End-to-end two-frontend scenarios on
//! top of the M10.1–M10.10 substrate.
//!
//! ## Test taxonomy
//!
//! Two paths. The dual-path interpretation is recorded in
//! `M10.11-AUDIT.md`.
//!
//! - **Synthetic-frontend path (CI default).** UnixStream-driven
//!   frontends speak the wire protocol directly. Combines with the
//!   M10.8/M10.9/M10.10 two-frontend coverage already in
//!   `tests/m5_5_acceptance.rs` to satisfy acceptance criterion 2.
//! - **PTY-doubled path (`#[ignore]`d).** Real pmacs binaries
//!   running inside real PTY pairs against a real daemon
//!   subprocess. Satisfies the spirit of "PTY harness from M5.9
//!   doubled." Operator-invoked before tagging via
//!   `cargo test --features luajit,crdt -- --ignored m10_11`.
//!
//! ## Fixture: [`DoubledPtyFixture`]
//!
//! Two PTY-spawned pmacs frontends attached to one daemon, plus a
//! third synthetic observer that reads daemon-side CRDT state.
//! The observer's assertion target is *daemon convergence*, not
//! frontend pixel equivalence — the two PTY frontends may render
//! with incidental differences (cursor styling, overlay colors)
//! while the underlying CRDT state agrees.

#![cfg(feature = "crdt")]

use std::collections::{HashMap, HashSet};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use pmacs::buffer::BufferId;
use pmacs::cell::CellSize;
use pmacs::crdt::CrdtState;
use pmacs::protocol::{
    AttachRequest, FrontendCapabilities, FrontendEvent, FrontendId, Hello, InstanceMessage,
    PROTOCOL_VERSION,
};
use pmacs::rope::CrdtOp as RopeCrdtOp;
use pmacs::transport::{read_message, write_message};

mod common;
use common::daemon::{TestDaemon, attach_multi};
use common::pty::{PmacsPty, spawn_pmacs_in_pty};

// ---------------------------------------------------------------------------
// Doubled PTY fixture
// ---------------------------------------------------------------------------

/// Which PTY-spawned frontend a fixture operation targets.
#[derive(Copy, Clone, Debug)]
enum Side {
    A,
    B,
}

/// `DoubledPtyFixture` owns two pmacs frontends running in PTYs,
/// one daemon subprocess, and a third synthetic observer that
/// reads daemon-side CRDT state. Cleanup is via Drop on each
/// owned field (Drop order: `pty_a`, `pty_b`, then `daemon`, so
/// the children exit before the daemon they were attached to).
struct DoubledPtyFixture {
    daemon: TestDaemon,
    pty_a: PmacsPty,
    pty_b: PmacsPty,
    observer: Observer,
}

/// The synthetic third frontend. Multi-frontend + CRDT-replica
/// capable; passive (sends no key events). Maintains a local
/// replica per `BufferId` that catches up to the daemon's
/// authoritative state via the `BufferSnapshot` + `CrdtOp` wire
/// stream the daemon broadcasts to every replica.
///
/// Tracking *every* buffer (not just the first one observed) is
/// load-bearing for M10.11's real scenarios: source files,
/// generated buffers like `*workers*` and `*help*`, and mid-session
/// CRDT upgrades all show up as additional `BufferSnapshot`
/// messages. The first-only design would silently miss any edit
/// activity outside the very first buffer the observer learns
/// about.
struct Observer {
    stream: UnixStream,
    frontend_id: FrontendId,
    /// Per-buffer replicas; populated from `BufferSnapshot`
    /// messages and updated by `CrdtOp` broadcasts.
    replicas: HashMap<BufferId, CrdtState>,
    /// First CRDT import error observed per buffer. A bad op should
    /// produce a precise diagnostic instead of being hidden behind a
    /// later generic convergence timeout.
    import_errors: HashMap<BufferId, String>,
    /// `FrontendId`s observed via `PresenceUpdate`, excluding the
    /// observer's own id. Used by [`Observer::wait_for_n_frontends`]
    /// as the PTY-readiness sentinel.
    other_frontends: HashSet<FrontendId>,
}

impl DoubledPtyFixture {
    /// Spawn daemon, attach the observer, then spawn both PTYs.
    /// The observer attaches *before* the PTY frontends so its
    /// `BufferSnapshot` corresponds to the empty initial state.
    ///
    /// Readiness sentinel: after spawning both PTYs, wait until the
    /// observer has received `PresenceUpdate` broadcasts from both
    /// non-observer `FrontendId`s. Each PTY frontend's first-tick
    /// presence sweep broadcasts a `PresenceUpdate` to other
    /// multi-frontend sessions, so two distinct non-observer
    /// frontend ids indicates both PTYs have completed handshake
    /// and the dispatcher has run at least one tick for each.
    /// This replaces a fixed wall-clock sleep with an observable
    /// condition.
    fn new(rows: u16, cols: u16) -> Self {
        let daemon = TestDaemon::spawn();
        let mut observer = Observer::attach(daemon.socket_path());
        let pty_a = spawn_pmacs_attach_in_pty(daemon.socket_path(), rows, cols);
        let pty_b = spawn_pmacs_attach_in_pty(daemon.socket_path(), rows, cols);
        observer
            .wait_for_n_frontends(2, Duration::from_secs(10))
            .expect("both PTY frontends should attach and broadcast presence");
        Self {
            daemon,
            pty_a,
            pty_b,
            observer,
        }
    }

    /// Inject bytes into one PTY frontend's stdin.
    fn type_at(&mut self, side: Side, bytes: &[u8]) {
        let pty = match side {
            Side::A => &mut self.pty_a,
            Side::B => &mut self.pty_b,
        };
        pty.write_input(bytes).expect("write_input");
    }

    /// Poll until any tracked buffer's materialized string contains
    /// `substring`. Returns the matching `BufferId` so subsequent
    /// assertions can target the same buffer.
    ///
    /// Used as the first-edit discovery point in the smoke test:
    /// the daemon picks which buffer is "active" for each PTY's
    /// keystrokes; the test doesn't pre-declare the `BufferId`.
    fn wait_for_any_buffer_contains(
        &mut self,
        substring: &str,
        timeout: Duration,
    ) -> Result<BufferId, String> {
        self.observer
            .wait_for_any_buffer_contains(substring, timeout)
    }

    /// Poll until the given buffer's materialized string contains
    /// *every* `substring` in `substrings`. Order-agnostic — the
    /// CRDT may decide on `"XY"` or `"YX"` for concurrent edits,
    /// and both are valid convergence outcomes.
    fn wait_for_buffer_contains_all(
        &mut self,
        buffer_id: BufferId,
        substrings: &[&str],
        timeout: Duration,
    ) -> Result<(), String> {
        self.observer
            .wait_for_buffer_contains_all(buffer_id, substrings, timeout)
    }

    /// Poll until the given buffer's materialized string equals
    /// `expected` exactly. Used by the per-frontend undo test
    /// where the post-undo state is deterministic regardless of
    /// CRDT ordering ambiguities.
    fn wait_for_buffer_equals(
        &mut self,
        buffer_id: BufferId,
        expected: &str,
        timeout: Duration,
    ) -> Result<(), String> {
        self.observer
            .wait_for_buffer_equals(buffer_id, expected, timeout)
    }

    /// Process ids of the two PTY children. Used by the Drop
    /// discipline test to verify post-drop death.
    #[allow(dead_code)]
    fn pty_pids(&self) -> (Option<u32>, Option<u32>) {
        (self.pty_a.process_id(), self.pty_b.process_id())
    }

    /// PID of the daemon subprocess. Used by the Drop discipline
    /// test to verify post-drop death.
    #[allow(dead_code)]
    fn daemon_pid(&self) -> u32 {
        self.daemon.pid()
    }
}

impl Observer {
    fn attach(socket_path: &Path) -> Self {
        let mut stream = UnixStream::connect(socket_path).expect("observer connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set read timeout");
        let hello: Hello = read_message(&mut stream).expect("observer read Hello");
        let caps = FrontendCapabilities {
            synchronized_output: true,
            unicode_smp: true,
            true_color: true,
            mouse: false,
            bracketed_paste: true,
            terminal_kind: Some("test-observer".into()),
            multi_frontend: true,
            crdt_replica: true,
        };
        let req = AttachRequest {
            protocol_version: PROTOCOL_VERSION,
            frontend_capabilities: caps,
            initial_size: CellSize::new(24, 80),
        };
        write_message(&mut stream, &req).expect("observer write AttachRequest");
        Self {
            stream,
            frontend_id: hello.assigned_frontend_id,
            replicas: HashMap::new(),
            import_errors: HashMap::new(),
            other_frontends: HashSet::new(),
        }
    }

    /// Pump the observer's wire stream non-blocking-ish for up to
    /// `slice`, applying any received `BufferSnapshot` / `CrdtOp` /
    /// `PresenceUpdate` to the local state.
    fn pump(&mut self, slice: Duration) {
        self.stream
            .set_read_timeout(Some(slice))
            .expect("set read timeout");
        let deadline = Instant::now() + slice;
        while Instant::now() < deadline {
            match read_message::<InstanceMessage>(&mut self.stream) {
                Ok(msg) => self.absorb(msg),
                Err(_) => return,
            }
        }
    }

    fn absorb(&mut self, msg: InstanceMessage) {
        match msg {
            InstanceMessage::BufferSnapshot {
                buffer_id,
                crdt_snapshot,
            } if !self.replicas.contains_key(&buffer_id) => {
                // Bootstrap each buffer's replica on first
                // BufferSnapshot for that BufferId. Later snapshots
                // for the same buffer are ignored — the established
                // replica catches up via CrdtOps.
                let r = CrdtState::new(self.frontend_id.0).expect("observer CrdtState::new");
                r.import_snapshot(&crdt_snapshot)
                    .expect("observer import_snapshot");
                self.replicas.insert(buffer_id, r);
            }
            InstanceMessage::CrdtOp { buffer_id, op } => {
                // If we received a CrdtOp for a buffer whose snapshot
                // we haven't seen, skip — the missing snapshot is a
                // dispatcher-state ordering hazard we don't try to
                // reconstruct here. In M10.11's smoke / scenario
                // tests, the observer attaches before any edit
                // activity, so this branch shouldn't fire.
                if let Some(r) = self.replicas.get(&buffer_id) {
                    if let Err(e) = r.import_updates(&op.bytes) {
                        self.import_errors
                            .entry(buffer_id)
                            .or_insert_with(|| format!("{e:?}"));
                    }
                }
            }
            InstanceMessage::PresenceUpdate { frontend_id, .. }
                if frontend_id != self.frontend_id =>
            {
                self.other_frontends.insert(frontend_id);
            }
            _ => {}
        }
    }

    fn materialized(&self, buffer_id: BufferId) -> Option<String> {
        self.replicas
            .get(&buffer_id)
            .map(CrdtState::materialize_string)
    }

    /// Block (via `pump`) until at least `n` distinct non-observer
    /// `FrontendId`s have been seen in `PresenceUpdate` messages.
    /// Used as the PTY-readiness sentinel.
    fn wait_for_n_frontends(&mut self, n: usize, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.pump(Duration::from_millis(100));
            if self.other_frontends.len() >= n {
                return Ok(());
            }
        }
        Err(format!(
            "expected {n} non-observer frontends, saw {} after {timeout:?}",
            self.other_frontends.len()
        ))
    }

    /// Iterate all tracked replicas; return the first `BufferId`
    /// whose materialized string contains `substring`. Pumps the
    /// stream until match or deadline.
    fn wait_for_any_buffer_contains(
        &mut self,
        substring: &str,
        timeout: Duration,
    ) -> Result<BufferId, String> {
        let deadline = Instant::now() + timeout;
        loop {
            self.pump(Duration::from_millis(100));
            for (id, replica) in &self.replicas {
                if replica.materialize_string().contains(substring) {
                    return Ok(*id);
                }
            }
            if Instant::now() >= deadline {
                let snapshot: Vec<(BufferId, String)> = self
                    .replicas
                    .iter()
                    .map(|(id, r)| (*id, r.materialize_string()))
                    .collect();
                return Err(format!(
                    "no buffer contained {substring:?} after {timeout:?}; \
                     observed {snapshot:?}; import_errors={errors:?}",
                    errors = self.import_errors
                ));
            }
        }
    }

    fn wait_for_buffer_equals(
        &mut self,
        buffer_id: BufferId,
        expected: &str,
        timeout: Duration,
    ) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            self.pump(Duration::from_millis(100));
            if let Some(text) = self.materialized(buffer_id) {
                if text == expected {
                    return Ok(());
                }
            }
            if Instant::now() >= deadline {
                let observed = self
                    .materialized(buffer_id)
                    .unwrap_or_else(|| "<no replica>".into());
                return Err(format!(
                    "buffer {buffer_id:?} did not equal {expected:?} \
                     after {timeout:?}; observed {observed:?}"
                ));
            }
        }
    }

    fn wait_for_buffer_contains_all(
        &mut self,
        buffer_id: BufferId,
        substrings: &[&str],
        timeout: Duration,
    ) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            self.pump(Duration::from_millis(100));
            if let Some(text) = self.materialized(buffer_id) {
                if substrings.iter().all(|s| text.contains(s)) {
                    return Ok(());
                }
            }
            if Instant::now() >= deadline {
                let observed = self
                    .materialized(buffer_id)
                    .unwrap_or_else(|| "<no replica>".into());
                return Err(format!(
                    "buffer {buffer_id:?} did not contain all of {substrings:?} \
                     after {timeout:?}; observed {observed:?}; import_error={import_error:?}",
                    import_error = self.import_errors.get(&buffer_id)
                ));
            }
        }
    }
}

fn spawn_pmacs_attach_in_pty(socket_path: &Path, rows: u16, cols: u16) -> PmacsPty {
    let socket_str = socket_path.to_str().expect("socket path is UTF-8");
    let isolated_home = socket_path.parent().expect("socket has parent");
    spawn_pmacs_in_pty(
        &["--attach", "--socket", socket_str],
        &[("HOME", isolated_home), ("XDG_CONFIG_HOME", isolated_home)],
        rows,
        cols,
    )
}

// ---------------------------------------------------------------------------
// Synthetic-frontend helpers (used by the synthesis test below)
// ---------------------------------------------------------------------------

/// Read the daemon's initial `BufferSnapshot` for a freshly-attached
/// stream. Returns the buffer id and the snapshot bytes. Panics if
/// the first message isn't a `BufferSnapshot` (the daemon always
/// emits the snapshot before any other render message for a new
/// replica session).
fn read_initial_snapshot(stream: &mut std::os::unix::net::UnixStream) -> (BufferId, Vec<u8>) {
    match read_message::<InstanceMessage>(stream).expect("read initial BufferSnapshot") {
        InstanceMessage::BufferSnapshot {
            buffer_id,
            crdt_snapshot,
        } => (buffer_id, crdt_snapshot),
        other => panic!("expected initial BufferSnapshot, got {other:?}"),
    }
}

/// Send a `FrontendEvent::CrdtOp` derived from a local replica's
/// last-emitted delta. Used by the synthesis test's optimistic
/// edit path.
fn send_crdt_op(
    stream: &mut std::os::unix::net::UnixStream,
    frontend_id: FrontendId,
    buffer_id: BufferId,
    op_bytes: Vec<u8>,
) {
    let ev = FrontendEvent::CrdtOp {
        frontend_id,
        buffer_id,
        op: RopeCrdtOp {
            peer_id: frontend_id.0,
            bytes: op_bytes,
        },
    };
    write_message(stream, &ev).expect("write CrdtOp");
}

/// Pump messages off `stream` into `replica` (importing every
/// `CrdtOp` for `buffer_id`) until the replica's materialized
/// string equals `expected` or the deadline elapses.
///
/// Used by the synthesis test to wait for cross-frontend
/// propagation: A sends an op, B's `pump_until` reads the
/// daemon's broadcast off `stream_b` and integrates it into
/// `replica_b`.
fn pump_until(
    stream: &mut std::os::unix::net::UnixStream,
    replica: &CrdtState,
    buffer_id: BufferId,
    expected: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if replica.materialize_string() == expected {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let slice = remaining.min(Duration::from_millis(100));
        stream.set_read_timeout(Some(slice)).ok();
        match read_message::<InstanceMessage>(stream) {
            Ok(InstanceMessage::CrdtOp { buffer_id: b, op }) if b == buffer_id => {
                let _ = replica.import_updates(&op.bytes);
            }
            // Ignore the daemon's render-side messages (CellDelta,
            // Cursor, PresenceUpdate, additional BufferSnapshots for
            // buffers we don't care about) AND read timeouts — both
            // just loop back to recheck materialize and try again.
            Ok(_) | Err(_) => {}
        }
    }
    Err(format!(
        "expected materialize {expected:?}, got {observed:?} after {timeout:?}",
        observed = replica.materialize_string()
    ))
}

// ---------------------------------------------------------------------------
// Day 1 sanity test — fixture wires together correctly
// ---------------------------------------------------------------------------

/// Day 1 acceptance: the doubled-PTY fixture spawns two real pmacs
/// frontends against one daemon, the synthetic observer attaches and
/// receives the initial `BufferSnapshot`, and keystrokes injected into
/// *both* frontends propagate to the observer's CRDT state.
///
/// The two PTY paths are exercised independently: A types "AB" first,
/// the observer auto-discovers the edited `BufferId`, then B types
/// "XY" into the same buffer. The final convergence assertion is
/// substring-based ("contains both tokens") because CRDT
/// ordering between A's and B's edits depends on their respective
/// cursor positions at the time of typing and on peer-id-tiebreaking
/// rules — both token orderings are correct outcomes.
///
/// `#[ignore]`d by default per M5.8's PTY-test precedent and the
/// M10.11 audit doc's CI-cost discipline: PTY-doubled tests are
/// operator-invoked before tagging, not CI-default.
#[test]
#[ignore = "PTY-doubled tests are operator-invoked before tagging, not CI-default"]
fn m10_11_doubled_pty_fixture_propagates_keystrokes_from_both_sides() {
    let mut fixture = DoubledPtyFixture::new(24, 80);

    // Frontend A types "AB". The keystrokes flow: PTY stdin → pmacs
    // frontend → KeyEvent → daemon → dispatch_key → buffer edit →
    // CrdtOp broadcast → observer's replica.
    fixture.type_at(Side::A, b"AB");
    let buffer_id = fixture
        .wait_for_any_buffer_contains("AB", Duration::from_secs(5))
        .expect("observer should see A's \"AB\" on some tracked buffer");

    // Frontend B types "XY" into the same buffer (B's cursor is at
    // B's own local position; the daemon dispatches its keystroke
    // independently of A's). The observer should see both tokens
    // converge on `buffer_id`.
    fixture.type_at(Side::B, b"XY");
    fixture
        .wait_for_buffer_contains_all(buffer_id, &["AB", "XY"], Duration::from_secs(5))
        .expect("observer should see both A's \"AB\" and B's \"XY\" converge");
}

/// End-to-end PTY-doubled proof that the frontend's optimistic-undo
/// path (M10.11 P1) is wired through real pmacs binaries.
///
/// **What this test verifies:** the keystroke crossterm parses as
/// `Char('4') + CTRL` (the only undo-bound keystroke that arrives
/// from a raw PTY without Kitty Keyboard Protocol — byte 0x1C; see
/// `src/optimistic.rs` `classify_key` for the parsing rationale)
/// reaches pmacs's frontend orchestrator, triggers
/// `BufferMirror::apply_local_undo` on the local `CrdtState`'s
/// peer-bound `UndoManager`, produces an inverse `CrdtOp`, and
/// propagates through the daemon to the observer.
///
/// **What this test does NOT verify:** symmetric interleaved-edit
/// per-frontend isolation (A edits, B edits, each undoes only their
/// own). That property is verified at the protocol level by
/// `m10_11_synthesis_two_frontends_converge_through_edits_and_undo`
/// above. The PTY-doubled version can't reliably exercise it because
/// cursor-freshness timing across remote-op broadcasts isn't
/// observable from the test harness: after one side's first
/// optimistic edit broadcasts, the other side's mirror may be
/// transiently stale (post-broadcast, pre-CursorByte) and its next
/// keystroke would round-trip via Key dispatch instead of going
/// optimistic. `MANUAL-TEST-CHECKLIST.md`'s two-laptop procedure
/// exercises both sides with real human-pace timing where the
/// daemon's per-tick `CursorByte` messages always arrive before
/// the next keystroke.
///
/// Single-side coverage is sufficient for the wiring proof: the
/// frontend orchestrator path is symmetric across all sessions; if
/// it works for one PTY frontend it works for any.
#[test]
#[ignore = "PTY-doubled tests are operator-invoked before tagging, not CI-default"]
fn m10_11_doubled_pty_optimistic_undo_propagates_end_to_end() {
    let mut fixture = DoubledPtyFixture::new(24, 80);

    // A types 'X' via optimistic CrdtOp. A's mirror is fresh at
    // attach time — no remote ops have arrived yet to staleify the
    // cursor, so the optimistic-apply predicate fires and A's
    // local CrdtState records the insert in its peer-bound
    // UndoManager.
    fixture.type_at(Side::A, b"X");
    let buffer_id = fixture
        .wait_for_any_buffer_contains("X", Duration::from_secs(5))
        .expect("observer should see A's 'X'");

    // A undoes via Ctrl-4 (byte 0x1C → crossterm parses as
    // `Char('4')` + CTRL). The optimistic layer recognizes this as
    // `OptimisticAction::Undo` and calls
    // `BufferMirror::apply_local_undo`, which uses loro's
    // peer-bound UndoManager on A's local CrdtState. The inverse
    // op broadcasts through the daemon to the observer.
    fixture.type_at(Side::A, b"\x1c");
    fixture
        .wait_for_buffer_equals(buffer_id, "", Duration::from_secs(5))
        .expect("observer should see buffer empty after A's optimistic undo");
}

// ---------------------------------------------------------------------------
// Day 1 Drop discipline — fixture cleanup contract
// ---------------------------------------------------------------------------

/// Day 1 Drop guard: when the fixture is dropped (whether by normal
/// scope exit, panic, or test failure), the two PTY children and
/// the daemon subprocess must all exit. The framing pass committed
/// to verifying this explicitly so subsequent tests can rely on the
/// guarantee.
///
/// Mechanism: capture PIDs before drop; drop the fixture; verify
/// the PIDs are no longer live via `kill(pid, 0)` (POSIX-portable
/// "does this process exist" probe — SIGNAL 0 doesn't deliver, but
/// returns ESRCH if the target is gone). Polls briefly because the
/// OS may not reap immediately after `Drop`'s `kill + wait` calls.
#[test]
#[ignore = "PTY-doubled tests are operator-invoked before tagging, not CI-default"]
fn m10_11_doubled_pty_fixture_drop_kills_all_children() {
    let (pty_a_pid, pty_b_pid, daemon_pid) = {
        let fixture = DoubledPtyFixture::new(24, 80);
        let (a, b) = fixture.pty_pids();
        (
            a.expect("pty A has a pid"),
            b.expect("pty B has a pid"),
            fixture.daemon_pid(),
        )
        // fixture drops here; daemon + both PTYs receive kill + wait
    };

    // Poll up to 2s for all three pids to become unreachable.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut still_live = Vec::new();
    while Instant::now() < deadline {
        still_live.clear();
        for (label, pid) in [
            ("pty_a", pty_a_pid),
            ("pty_b", pty_b_pid),
            ("daemon", daemon_pid),
        ] {
            if pid_alive(pid) {
                still_live.push((label, pid));
            }
        }
        if still_live.is_empty() {
            return; // all reaped; Drop discipline holds.
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("post-drop: still-live processes after 2s: {still_live:?}");
}

/// `kill(pid, 0)` returns 0 if the target is reachable (could be
/// signaled), ESRCH if the target doesn't exist. We don't care
/// about EPERM here — every process this test spawns is the test's
/// own child, so EPERM isn't a failure mode we expect.
fn pid_alive(pid: u32) -> bool {
    use std::ffi::OsStr;
    use std::process::Command;
    // Avoid pulling in a libc dependency for one POSIX probe.
    // `kill -0 <pid>` is the shell-portable equivalent.
    let pid_str = pid.to_string();
    let status = Command::new("kill")
        .args([OsStr::new("-0"), OsStr::new(pid_str.as_str())])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    matches!(status, Ok(s) if s.success())
}

// ---------------------------------------------------------------------------
// Day 2 synthesis — synthetic-frontend end-to-end (CI default)
// ---------------------------------------------------------------------------

/// **M10.11's flagship acceptance test.** Two synthetic replica
/// frontends attach to one daemon, both edit one buffer, both undo
/// their own edits, and both converge to the same materialized
/// state at every step. Satisfies acceptance criterion 2
/// ("automated equivalent (two synthetic frontends) passes in CI").
///
/// Test shape — strictly serial-with-convergence-waits per the
/// framing pass's Q7 decision. Each step waits for both replicas
/// to converge before the next step proceeds, so per-step
/// assertions are deterministic strings rather than substring
/// containment.
///
/// 1. A inserts `"AAA"` at position 0 via optimistic `CrdtOp`.
///    Wait until B's replica materializes `"AAA"`.
/// 2. B inserts `"BBB"` at position 3 (end) via optimistic `CrdtOp`.
///    Wait until A's replica materializes `"AAABBB"`.
/// 3. A undoes its own insert: A's local `UndoManager` produces the
///    inverse op, A sends it as a `CrdtOp`. The daemon integrates
///    and broadcasts to B. Wait until both materialize `"BBB"`.
/// 4. B undoes its own insert symmetrically. Both materialize `""`.
///
/// Each undo step follows the production optimistic-undo path: the
/// **frontend's** local `CrdtState::undo` produces the inverse op
/// (loro's `UndoManager` is local-only per `src/crdt.rs:60-65`,
/// scoped to the bound peer-id, so a frontend undoes its own ops
/// regardless of remote concurrent activity). The daemon's
/// `dispatch_key` Ctrl-/ path is the same logical path but runs
/// against the **daemon's** local CRDT — that path is exercised
/// by single-frontend tests in `tests/m5_5_acceptance.rs` and by
/// the PTY-doubled tests below, where real frontend processes drive
/// it. In this synthetic flagship, the test plays the role of the
/// frontend's optimistic-undo orchestrator.
///
/// This test runs in the default `cargo test --features luajit,crdt`
/// invocation — no `#[ignore]`. It is the load-bearing CI-default
/// gate for M10.11.
#[test]
fn m10_11_synthesis_two_frontends_converge_through_edits_and_undo() {
    let daemon = TestDaemon::spawn();

    let (hello_a, mut stream_a) = attach_multi(&daemon);
    let (hello_b, mut stream_b) = attach_multi(&daemon);

    let (buffer_id, snap_a) = read_initial_snapshot(&mut stream_a);
    let (_, snap_b) = read_initial_snapshot(&mut stream_b);

    // Bootstrap local replicas. Per M10.10, each frontend
    // maintains its own CRDT replica; the synthetic test simulates
    // that bookkeeping in-process.
    let replica_a = CrdtState::new(hello_a.assigned_frontend_id.0).expect("A CrdtState::new");
    replica_a
        .import_snapshot(&snap_a)
        .expect("A import_snapshot");
    let replica_b = CrdtState::new(hello_b.assigned_frontend_id.0).expect("B CrdtState::new");
    replica_b
        .import_snapshot(&snap_b)
        .expect("B import_snapshot");

    // ----- Step 1: A inserts "AAA" via optimistic CrdtOp -----
    send_optimistic_op_from(
        &mut stream_a,
        &replica_a,
        hello_a.assigned_frontend_id,
        buffer_id,
        |r| {
            r.insert(0, "AAA").expect("A insert AAA");
        },
    );

    pump_until(
        &mut stream_b,
        &replica_b,
        buffer_id,
        "AAA",
        Duration::from_secs(5),
    )
    .expect("B converges to AAA after A's optimistic insert");
    assert_eq!(replica_a.materialize_string(), "AAA");

    // ----- Step 2: B appends "BBB" via optimistic CrdtOp -----
    send_optimistic_op_from(
        &mut stream_b,
        &replica_b,
        hello_b.assigned_frontend_id,
        buffer_id,
        |r| {
            r.insert(3, "BBB").expect("B insert BBB at end");
        },
    );

    pump_until(
        &mut stream_a,
        &replica_a,
        buffer_id,
        "AAABBB",
        Duration::from_secs(5),
    )
    .expect("A converges to AAABBB after B's optimistic append");
    assert_eq!(replica_b.materialize_string(), "AAABBB");

    // ----- Step 3: A undoes its own "AAA" via optimistic CrdtOp -----
    // Loro's UndoManager is peer-scoped: replica_a.undo() reverses
    // A's last op regardless of B's concurrent inserts. The inverse
    // op is broadcast to B; A applied locally already.
    send_optimistic_op_from(
        &mut stream_a,
        &replica_a,
        hello_a.assigned_frontend_id,
        buffer_id,
        |r| {
            let did = r.undo().expect("A undo");
            assert!(did, "A's local UndoManager should have AAA on its stack");
        },
    );

    assert_eq!(replica_a.materialize_string(), "BBB");
    pump_until(
        &mut stream_b,
        &replica_b,
        buffer_id,
        "BBB",
        Duration::from_secs(5),
    )
    .expect("B converges to BBB after A's undo (per-frontend undo isolates A's ops)");

    // ----- Step 4: B undoes its own "BBB" via optimistic CrdtOp -----
    send_optimistic_op_from(
        &mut stream_b,
        &replica_b,
        hello_b.assigned_frontend_id,
        buffer_id,
        |r| {
            let did = r.undo().expect("B undo");
            assert!(did, "B's local UndoManager should have BBB on its stack");
        },
    );

    assert_eq!(replica_b.materialize_string(), "");
    pump_until(
        &mut stream_a,
        &replica_a,
        buffer_id,
        "",
        Duration::from_secs(5),
    )
    .expect("A converges to empty after B's undo");
}

/// Run `mutate` on a local replica, capture the resulting op via
/// version-diff, and send it as a `FrontendEvent::CrdtOp`. This is
/// the optimistic-apply shape: the frontend mutates locally, then
/// hands the wire bytes to the daemon for broadcast to other
/// replicas.
fn send_optimistic_op_from<F>(
    stream: &mut std::os::unix::net::UnixStream,
    replica: &CrdtState,
    frontend_id: FrontendId,
    buffer_id: BufferId,
    mutate: F,
) where
    F: FnOnce(&CrdtState),
{
    let v = replica.version();
    mutate(replica);
    let op_bytes = replica
        .export_updates_since(&v)
        .expect("export updates after local mutation");
    send_crdt_op(stream, frontend_id, buffer_id, op_bytes);
}

// ---------------------------------------------------------------------------
// Q13 adversarial scenarios — actively try to break the arc
// (verification-milestone premise check; M10.11 framing reframe).
// ---------------------------------------------------------------------------

/// Drain both streams, importing every matching `CrdtOp` into the
/// respective replica, until both replicas materialize the *same*
/// non-empty string (convergence + cross-replica agreement) or the
/// deadline elapses. Unlike [`pump_until`], the converged value is
/// not known a priori — concurrent same-position inserts converge to
/// a CRDT-deterministic interleaving whose exact shape is loro's
/// peer-id tiebreak, not the test's to predict. The load-bearing
/// assertion is *agreement*, not a specific string.
fn pump_both_until_equal(
    stream_a: &mut std::os::unix::net::UnixStream,
    replica_a: &CrdtState,
    stream_b: &mut std::os::unix::net::UnixStream,
    replica_b: &CrdtState,
    buffer_id: BufferId,
    timeout: Duration,
) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let (sa, sb) = (
            replica_a.materialize_string(),
            replica_b.materialize_string(),
        );
        if sa == sb && !sa.is_empty() {
            return Ok(sa);
        }
        for (stream, replica) in [(&mut *stream_a, replica_a), (&mut *stream_b, replica_b)] {
            stream
                .set_read_timeout(Some(Duration::from_millis(50)))
                .ok();
            if let Ok(InstanceMessage::CrdtOp { buffer_id: b, op }) =
                read_message::<InstanceMessage>(stream)
                && b == buffer_id
            {
                let _ = replica.import_updates(&op.bytes);
            }
        }
    }
    Err(format!(
        "no convergence after {timeout:?}: A={a:?} B={b:?}",
        a = replica_a.materialize_string(),
        b = replica_b.materialize_string()
    ))
}

/// **Q13 Category 1 — edits during the convergence window.**
///
/// Two replicas insert at the *same byte position* before either has
/// seen the other's op (the op is in flight to the daemon when the
/// competing op is generated). Asserts the three load-bearing
/// properties the verification-milestone premise check demands an
/// adversarial test prove, not just confirm:
///
/// 1. **No op lost** — both inserted tokens are present in the
///    converged state.
/// 2. **Convergence + cross-replica agreement** — both replicas
///    materialize the *identical* string (the load-bearing CRDT
///    property; a divergence here is a real bug, not a cosmetic one).
/// 3. **Determinism** — the converged interleaving is stable. The
///    exact value is loro's peer-id tiebreak (lower peer wins the
///    earlier position); pinned here so a loro-determinism
///    regression fails loudly rather than silently changing the
///    user-visible merge. (Record-and-assert-stable, mirroring the
///    framing's Q8 fixed-seed discipline.)
///
/// CI-default (no `#[ignore]`): synthetic frontends, deterministic,
/// fast. This is adversarial, not confirmatory — it manufactures the
/// same-position race the happy-path synthesis test deliberately
/// serializes away (Q7).
#[test]
fn m10_11_q13_cat1_concurrent_same_position_inserts_converge() {
    let daemon = TestDaemon::spawn();
    let (hello_a, mut stream_a) = attach_multi(&daemon);
    let (hello_b, mut stream_b) = attach_multi(&daemon);
    // A attaches first → FrontendId(2) → peer 2; B → FrontendId(3) →
    // peer 3. Tiebreak determinism is asserted against this ordering.
    assert!(
        hello_a.assigned_frontend_id.0 < hello_b.assigned_frontend_id.0,
        "A must be the lower peer id for the determinism pin to mean anything"
    );

    let (buffer_id, snap_a) = read_initial_snapshot(&mut stream_a);
    let (_, snap_b) = read_initial_snapshot(&mut stream_b);
    let replica_a = CrdtState::new(hello_a.assigned_frontend_id.0).expect("A new");
    replica_a.import_snapshot(&snap_a).expect("A import");
    let replica_b = CrdtState::new(hello_b.assigned_frontend_id.0).expect("B new");
    replica_b.import_snapshot(&snap_b).expect("B import");

    // The convergence window: A generates+sends its op, then B
    // generates+sends ITS op at the same position *before pumping* —
    // so neither replica has integrated the other when both ops were
    // produced. This is the race Q7's synthesis test serializes away;
    // Q13 manufactures it on purpose.
    send_optimistic_op_from(
        &mut stream_a,
        &replica_a,
        hello_a.assigned_frontend_id,
        buffer_id,
        |r| {
            r.insert(0, "A1").expect("A insert at 0");
        },
    );
    send_optimistic_op_from(
        &mut stream_b,
        &replica_b,
        hello_b.assigned_frontend_id,
        buffer_id,
        |r| {
            r.insert(0, "B1")
                .expect("B insert at 0 — same position, concurrent");
        },
    );

    let converged = pump_both_until_equal(
        &mut stream_a,
        &replica_a,
        &mut stream_b,
        &replica_b,
        buffer_id,
        Duration::from_secs(5),
    )
    .expect("Q13 cat1: concurrent same-position inserts must converge");

    // (1) no op lost.
    assert!(
        converged.contains("A1") && converged.contains("B1"),
        "both tokens must survive the merge; got {converged:?}"
    );
    // (2) cross-replica agreement (pump_both_until_equal already
    // required sa == sb to return; re-assert explicitly for the
    // record).
    assert_eq!(
        replica_a.materialize_string(),
        replica_b.materialize_string(),
        "replicas must agree (convergence is the load-bearing CRDT property)"
    );
    // (3) determinism pin. loro orders concurrent same-position
    // inserts by peer id; A (peer 2) < B (peer 3). The exact merge is
    // pinned so a loro-version determinism change fails this test
    // loudly rather than silently altering the user-visible result.
    assert_eq!(
        converged, "A1B1",
        "deterministic peer-id tiebreak regressed (lower peer wins \
         earlier position); converged={converged:?}"
    );
}

/// Read exactly one `CrdtOp` for `buffer_id` off `stream`, skipping
/// the daemon's render-side messages (CellDelta/Cursor/Presence) and
/// snapshots for other buffers. Returns the op's wire bytes. Used by
/// the cat-2 scenario to model *selective, test-controlled* delivery
/// order into a replica (the synthetic frontend chooses when to
/// integrate each op — deterministic, unlike wall-clock jitter).
fn read_one_crdt_op(
    stream: &mut std::os::unix::net::UnixStream,
    buffer_id: BufferId,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        stream
            .set_read_timeout(Some(remaining.min(Duration::from_millis(100))))
            .ok();
        match read_message::<InstanceMessage>(stream) {
            Ok(InstanceMessage::CrdtOp { buffer_id: b, op }) if b == buffer_id => {
                return Ok(op.bytes);
            }
            Ok(_) | Err(_) => {}
        }
    }
    Err(format!("no CrdtOp for {buffer_id:?} within {timeout:?}"))
}

/// **Q13 Category 2 — undo across delayed ops.**
///
/// Models the framing's "B sees ops 1 and 3 before 2" via
/// deterministic test-controlled import ordering (the synthetic
/// frontend *is* B; it chooses integration order) rather than
/// wall-clock daemon jitter — deterministic, no flake, and it
/// exercises the genuinely adversarial interaction: **per-frontend
/// undo (M10.4) under causally-pending delayed delivery
/// (M10.10 wire) — an arc-level interaction no single milestone
/// tested.**
///
/// Scenario (A = peer 2, B = peer 3):
/// 1. A inserts "1"@0; B integrates → B="1".
/// 2. A inserts "2"@1 (A="12"); B *withholds* this op (delayed).
/// 3. A inserts "3"@2 (A="123"); B integrates op3 — causally pending
///    op2, loro buffers it, B still "1".
/// 4. B issues "undo my last edit": B has no ops → must be a no-op
///    (per-peer undo isolation; B must NOT reverse any of A's ops).
/// 5. A undoes: A's `UndoManager` reverses A's op3 → A="12". B
///    integrates A's undo (still pending op2).
/// 6. The withheld op2 is finally delivered to B.
/// 7. **Assert:** both converge to "12" (op1+op2 survive, op3
///    undone), replicas agree, B's step-4 undo damaged nothing.
///
/// CI-default (synthetic, deterministic). Adversarial: it
/// manufactures causally-pending-delivery + concurrent-undo, the
/// exact interaction the happy-path tests serialize away.
#[test]
fn m10_11_q13_cat2_undo_across_delayed_ops() {
    let daemon = TestDaemon::spawn();
    let (hello_a, mut stream_a) = attach_multi(&daemon);
    let (hello_b, mut stream_b) = attach_multi(&daemon);

    let (buffer_id, snap_a) = read_initial_snapshot(&mut stream_a);
    let (_, snap_b) = read_initial_snapshot(&mut stream_b);
    let replica_a = CrdtState::new(hello_a.assigned_frontend_id.0).expect("A new");
    replica_a.import_snapshot(&snap_a).expect("A import");
    let replica_b = CrdtState::new(hello_b.assigned_frontend_id.0).expect("B new");
    replica_b.import_snapshot(&snap_b).expect("B import");

    // Step 1: A op1 "1"@0; B integrates.
    send_optimistic_op_from(
        &mut stream_a,
        &replica_a,
        hello_a.assigned_frontend_id,
        buffer_id,
        |r| {
            r.insert(0, "1").expect("A op1");
        },
    );
    let op1 =
        read_one_crdt_op(&mut stream_b, buffer_id, Duration::from_secs(5)).expect("B receives op1");
    replica_b.import_updates(&op1).expect("B import op1");
    assert_eq!(replica_b.materialize_string(), "1", "B has op1");

    // Step 2: A op2 "2"@1 — B withholds (delayed delivery).
    send_optimistic_op_from(
        &mut stream_a,
        &replica_a,
        hello_a.assigned_frontend_id,
        buffer_id,
        |r| {
            r.insert(1, "2").expect("A op2");
        },
    );
    let op2_withheld = read_one_crdt_op(&mut stream_b, buffer_id, Duration::from_secs(5))
        .expect("B receives op2 (withheld, not yet imported)");

    // Step 3: A op3 "3"@2; B integrates op3 (causally pending op2).
    send_optimistic_op_from(
        &mut stream_a,
        &replica_a,
        hello_a.assigned_frontend_id,
        buffer_id,
        |r| {
            r.insert(2, "3").expect("A op3");
        },
    );
    let op3 =
        read_one_crdt_op(&mut stream_b, buffer_id, Duration::from_secs(5)).expect("B receives op3");
    replica_b
        .import_updates(&op3)
        .expect("B import op3 (op2 pending)");
    assert_eq!(replica_a.materialize_string(), "123", "A has 123");
    // B's view with op2 causally pending: loro buffers op3's effect.
    let b_before_undo = replica_b.materialize_string();

    // Step 4: B "undo my last edit" — B has no local ops. Must be a
    // no-op; must NOT reverse any of A's ops (per-peer undo
    // isolation, the load-bearing M10.4 property under delay).
    let b_undid = replica_b.undo().expect("B undo call");
    assert!(
        !b_undid,
        "B has no own ops; undo must be a no-op, not reach across to A's"
    );
    assert_eq!(
        replica_b.materialize_string(),
        b_before_undo,
        "B's no-op undo must not change B's state"
    );

    // Step 5: A undoes → reverses A's op3 → A="12". B integrates A's
    // undo op (still pending op2).
    let v_before_a_undo = replica_a.version();
    let a_undid = replica_a.undo().expect("A undo call");
    assert!(a_undid, "A has own ops; undo reverses op3");
    assert_eq!(replica_a.materialize_string(), "12", "A undid op3 → 12");
    let a_undo_bytes = replica_a
        .export_updates_since(&v_before_a_undo)
        .expect("export A's undo op");
    send_crdt_op(
        &mut stream_a,
        hello_a.assigned_frontend_id,
        buffer_id,
        a_undo_bytes.clone(),
    );
    replica_b
        .import_updates(&a_undo_bytes)
        .expect("B import A's undo (op2 still pending)");

    // Step 6: the withheld op2 is finally delivered to B.
    replica_b
        .import_updates(&op2_withheld)
        .expect("B import the delayed op2");

    // Step 7: convergence. A may still need A's own broadcast echo /
    // nothing further; drain both until equal.
    let converged = pump_both_until_equal(
        &mut stream_a,
        &replica_a,
        &mut stream_b,
        &replica_b,
        buffer_id,
        Duration::from_secs(5),
    )
    .expect("cat2: converge after delayed op2 + concurrent undos");

    assert_eq!(
        converged, "12",
        "op1+op2 survive, op3 undone by A, B's no-op undo damaged \
         nothing; converged={converged:?}"
    );
    assert_eq!(
        replica_a.materialize_string(),
        replica_b.materialize_string(),
        "replicas agree after delayed-delivery + concurrent-undo"
    );
}

/// **Q8 — convergence under jitter (acceptance criterion 3).**
///
/// Daemon spawned with `PMACS_INSTANCE_LATENCY_JITTER_MS=50` and a
/// pinned seed (`0xC0FFEE` = 12648430) so the delay pattern is
/// deterministically reproducible — a flake's seed is the one to
/// re-run (framing Q8).
///
/// **F2 correction.** Finding 5's first resolution ("(B): jitter-mode
/// delays both `CellDelta` and `CrdtOp`") was wrong — it widened the
/// match in the render-message loop, which never carries broadcast
/// `CrdtOp`s; the CRDT-convergence path was *not* exercised and this
/// test silently asserted nothing about CRDT-under-jitter. F2 moved
/// the jitter to the actual `broadcast_crdt_op` write site
/// (`daemon.rs`), so delivered ops are now genuinely delayed.
///
/// **Falsification guard.** Because a no-op jitter (or jitter at the
/// wrong site, the original bug) would let convergence happen in
/// sub-millisecond time, this test asserts a **wall-clock floor**:
/// with `JITTER_MS=50` applied to every broadcast `CrdtOp` write,
/// first-send→convergence must take materially longer than
/// un-jittered delivery. A regression that detaches the jitter from
/// the CRDT path again fails the floor, not just the (weaker)
/// convergence assertion. This is the reviewer's "no-op jitter
/// cannot pass" requirement made executable.
///
/// The load-bearing CRDT property: **convergence is
/// delivery-order-independent.** Jitter reorders/delays op delivery;
/// the converged result must be *identical to the no-jitter result*.
/// Pinned (record-and-assert-stable, mirroring cat-1).
///
/// One scenario, not a sweep (Q8 scope guard; a fuzz sweep is v0.2).
/// Generous timeout: 50ms jitter × every broadcast-CrdtOp write
/// accumulates; convergence-within-timeout, not per-event budget
/// (Q5: PTY/jitter paths have no perf gate).
///
/// CI-default (synthetic, deterministic via pinned seed).
#[test]
fn m10_11_q8_convergence_under_jitter() {
    let daemon = TestDaemon::spawn_with_env(&[
        ("PMACS_INSTANCE_LATENCY_JITTER_MS", "50"),
        // 0xC0FFEE — explicit so the reproducibility claim is not
        // implicit in the daemon's default.
        ("PMACS_INSTANCE_LATENCY_JITTER_SEED", "12648430"),
    ]);
    let (hello_a, mut stream_a) = attach_multi(&daemon);
    let (hello_b, mut stream_b) = attach_multi(&daemon);

    let (buffer_id, snap_a) = read_initial_snapshot(&mut stream_a);
    let (_, snap_b) = read_initial_snapshot(&mut stream_b);
    let replica_a = CrdtState::new(hello_a.assigned_frontend_id.0).expect("A new");
    replica_a.import_snapshot(&snap_a).expect("A import");
    let replica_b = CrdtState::new(hello_b.assigned_frontend_id.0).expect("B new");
    replica_b.import_snapshot(&snap_b).expect("B import");

    // Falsification-guard timer: started before the first send,
    // measured after convergence. With JITTER_MS=50 applied to every
    // broadcast-CrdtOp write (the F2-corrected site), first-send →
    // convergence accumulates tens-to-hundreds of ms. Un-jittered
    // synthetic in-process convergence is sub-10ms. A 30ms floor sits
    // far above un-jittered and far below the jittered expectation —
    // non-flaky in both directions, and a regression that detaches
    // jitter from the CRDT path (Finding 5's original bug) drops
    // convergence back under 10ms and fails the floor loudly.
    let t0 = Instant::now();

    // Deterministic op sequence, interleaved between peers, several
    // positions. The daemon's jittered delivery reorders these on the
    // wire; the CRDT must converge to one delivery-order-independent
    // result. (No convergence wait between sends — they race through
    // the jittered dispatcher; that race is the point.)
    send_optimistic_op_from(
        &mut stream_a,
        &replica_a,
        hello_a.assigned_frontend_id,
        buffer_id,
        |r| {
            r.insert(0, "a").expect("A a@0");
        },
    );
    send_optimistic_op_from(
        &mut stream_b,
        &replica_b,
        hello_b.assigned_frontend_id,
        buffer_id,
        |r| {
            r.insert(0, "b").expect("B b@0 (concurrent same pos)");
        },
    );
    send_optimistic_op_from(
        &mut stream_a,
        &replica_a,
        hello_a.assigned_frontend_id,
        buffer_id,
        |r| {
            let end = r.materialize_string().len();
            r.insert(end, "A").expect("A A@end");
        },
    );
    send_optimistic_op_from(
        &mut stream_b,
        &replica_b,
        hello_b.assigned_frontend_id,
        buffer_id,
        |r| {
            let end = r.materialize_string().len();
            r.insert(end, "B").expect("B B@end");
        },
    );

    // Generous: cumulative 50ms jitter across all CellDelta+CrdtOp
    // writes for both frontends. Convergence-within-timeout is the
    // assertion (Q5: no per-event budget on jitter paths).
    let converged = pump_both_until_equal(
        &mut stream_a,
        &replica_a,
        &mut stream_b,
        &replica_b,
        buffer_id,
        Duration::from_secs(20),
    )
    .expect("Q8/criterion-3: CRDT layer must converge under jitter");
    let elapsed = t0.elapsed();

    // Falsification guard: jitter must have actually delayed the
    // broadcast-CrdtOp path. If this fails with a small `elapsed`,
    // the jitter is detached from the CRDT path again (Finding 5's
    // original bug / an F2 regression) — the convergence assertions
    // below would still pass while testing nothing about
    // CRDT-under-jitter.
    assert!(
        elapsed >= Duration::from_millis(30),
        "criterion-3 jitter not reaching the broadcast-CrdtOp path: \
         converged in {elapsed:?} (un-jittered speed). With \
         JITTER_MS=50 on every broadcast write this must be \
         materially slower. This is the F2 regression guard."
    );

    // No op lost: all four tokens survive the jittered merge.
    for tok in ["a", "b", "A", "B"] {
        assert!(
            converged.contains(tok),
            "jitter must not lose ops; {tok:?} missing from {converged:?}"
        );
    }
    // Cross-replica agreement (the load-bearing CRDT property).
    assert_eq!(
        replica_a.materialize_string(),
        replica_b.materialize_string(),
        "replicas must agree under jitter (convergence is the criterion-3 property)"
    );
    // Delivery-order independence: the converged value is exactly
    // what loro produces for this op sequence + peer ids (A=2, B=3),
    // regardless of the jittered delivery order. Pinned: a change is
    // either loro-determinism regression or jitter leaking into the
    // CRDT outcome.
    // `"aAbB"`: A (peer 2) runs "a"@0 then "A"@end → "aA"; B (peer 3)
    // runs "b"@0 then "B"@end → "bB"; loro merges the concurrent runs
    // by peer-id tiebreak (lower peer's run orders first) → "aAbB".
    // Independent of the seeded jitter delivery order (the CRDT
    // property; convergence+agreement+no-loss above already proved it
    // under jitter — this pin additionally guards determinism).
    assert_eq!(
        converged, "aAbB",
        "jitter changed the CRDT outcome (must be timing-only, \
         delivery-order-independent); converged={converged:?}"
    );
}

/// **Q13 Category 3 — reattach mid-divergence (narrowed per
/// Finding 4).**
///
/// **Asserts:** CRDT state converges across reattach. A edits, A's
/// connection drops, B edits while A is gone, A reattaches and
/// bootstraps from `BufferSnapshot`; reattached-A's replica
/// converges to the same state as B and the daemon — including B's
/// post-disconnect edits *and* A's pre-disconnect edits. This is the
/// spec's Scenario-4 load-bearing sub-claim ("A's reattach restores
/// the converged state").
///
/// **Does NOT assert** per-frontend undo reaches pre-disconnect ops.
/// Per **Finding 4** (M5.8-inherited reconnect-identity gap):
/// `daemon.rs` issues a *fresh* `FrontendId`/`peer_id` on every
/// accepted connection (no `handle_reattach`), so reattached-A is a
/// different CRDT peer than pre-disconnect-A; loro's per-peer
/// `UndoManager` on reattached-A cannot reach pre-disconnect ops.
/// This is a **documented v1.0 limitation**, not a bug this test
/// should fail on — the broken sub-claim is recorded in the manual
/// checklist Scenario 4 wording and in V0.2-PREREQUISITES.md
/// (SO_PEERCRED-based reattach identity, the v0.2 follow-up). The
/// test asserts the sub-claim that *holds* and explicitly documents
/// the one that doesn't, rather than asserting the full Scenario-4
/// claim and failing.
///
/// The fresh-FrontendId behavior is asserted *in-test* (not hidden):
/// recording Finding 4's mechanism at the assertion site so a future
/// reader sees the gap, not just the narrowed pass.
///
/// CI-default (synthetic, deterministic). Adversarial: it
/// manufactures the disconnect-mid-divergence the happy-path tests
/// never exercise.
#[test]
fn m10_11_q13_cat3_reattach_converges_crdt_state() {
    let daemon = TestDaemon::spawn();
    let (hello_a1, mut stream_a1) = attach_multi(&daemon);
    let (hello_b, mut stream_b) = attach_multi(&daemon);

    let (buffer_id, snap_a1) = read_initial_snapshot(&mut stream_a1);
    let (_, snap_b) = read_initial_snapshot(&mut stream_b);
    let replica_a1 = CrdtState::new(hello_a1.assigned_frontend_id.0).expect("A1 new");
    replica_a1.import_snapshot(&snap_a1).expect("A1 import");
    let replica_b = CrdtState::new(hello_b.assigned_frontend_id.0).expect("B new");
    replica_b.import_snapshot(&snap_b).expect("B import");

    // A edits before the drop.
    send_optimistic_op_from(
        &mut stream_a1,
        &replica_a1,
        hello_a1.assigned_frontend_id,
        buffer_id,
        |r| {
            r.insert(0, "a1").expect("A pre-disconnect edit");
        },
    );
    pump_until(
        &mut stream_b,
        &replica_b,
        buffer_id,
        "a1",
        Duration::from_secs(5),
    )
    .expect("B converges with A's pre-disconnect edit");

    // A's network drops: closing the stream is the disconnect.
    drop(stream_a1);

    // B keeps editing alone while A is gone.
    send_optimistic_op_from(
        &mut stream_b,
        &replica_b,
        hello_b.assigned_frontend_id,
        buffer_id,
        |r| {
            let end = r.materialize_string().len();
            r.insert(end, "b1").expect("B post-disconnect edit");
        },
    );

    // A reattaches. Finding 4: a *fresh* FrontendId is issued — no
    // reconnect-identity preservation. Asserted in-test so the gap
    // is recorded at the site, not hidden behind the narrowed pass.
    let (hello_a2, mut stream_a2) = attach_multi(&daemon);
    assert_ne!(
        hello_a2.assigned_frontend_id, hello_a1.assigned_frontend_id,
        "Finding 4: reattach issues a fresh FrontendId (no \
         handle_reattach); reattached-A is a different CRDT peer. \
         This documents the v1.0 limitation in-test."
    );

    // Reattached-A bootstraps from the daemon's current snapshot —
    // which must reflect the converged state (A's pre-disconnect a1
    // + B's post-disconnect b1).
    let (_, snap_a2) = read_initial_snapshot(&mut stream_a2);
    let replica_a2 = CrdtState::new(hello_a2.assigned_frontend_id.0).expect("A2 new");
    replica_a2
        .import_snapshot(&snap_a2)
        .expect("A2 import snapshot");

    // Drain both until they agree (handles the
    // reattach-vs-B's-op-integration race the same way cat-1/2 do).
    let converged = pump_both_until_equal(
        &mut stream_a2,
        &replica_a2,
        &mut stream_b,
        &replica_b,
        buffer_id,
        Duration::from_secs(5),
    )
    .expect("cat3: reattached-A must converge to daemon/B state via BufferSnapshot");

    // CRDT state restored across reattach: both pre- and
    // post-disconnect edits present, replicas agree.
    assert!(
        converged.contains("a1") && converged.contains("b1"),
        "reattach must restore the converged state (pre-disconnect a1 \
         + post-disconnect b1); got {converged:?}"
    );
    assert_eq!(
        replica_a2.materialize_string(),
        replica_b.materialize_string(),
        "reattached-A and B must agree (Scenario-4 load-bearing sub-claim)"
    );
    // Determinism pin (record-and-assert-stable): a1@0 then b1@end.
    assert_eq!(
        converged, "a1b1",
        "reattach-converged value regressed; converged={converged:?}"
    );

    // NOTE (Finding 4, deliberate non-assertion): no
    // undo-across-reattach check. reattached-A (peer
    // {hello_a2.assigned_frontend_id}) cannot undo pre-disconnect-A
    // (peer {hello_a1.assigned_frontend_id}) ops — documented v1.0
    // limitation; V0.2-PREREQUISITES.md carries the follow-up.
}
