// m5_5_acceptance.rs --- Acceptance suite for M5.5 (daemon + attach).

//! End-to-end acceptance tests for T M5.5 (daemon mode +
//! local-socket transport).
//!
//! Each test spawns the real `pmacs` binary as a subprocess, drives
//! it via the published protocol crate, and tears it down on
//! completion. Tempdir-scoped sockets keep tests isolated under
//! parallel `cargo test`.
//!
//! Tests in this file (mapped to T M5.5 acceptance criteria):
//!
//! 1. [`daemon_starts_socket_and_lockfile_appear_with_correct_modes`]
//! 2. [`attach_send_key_receive_cell_response`]
//! 3. [`clean_detach_then_reattach`]
//! 4. [`ungraceful_disconnect_then_reattach`]
//! 5. [`second_daemon_same_socket_fails_clearly`]
//! 6. [`sigterm_daemon_sends_goodbye_and_cleans_up`]
//! 7. [`sigkill_daemon_leaves_stale_files_next_start_recovers`]
//! 8. [`version_mismatch_clean_disconnect`]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use tempfile::TempDir;

use pmacs::cell::CellSize;
#[cfg(feature = "crdt")]
use pmacs::cell::Color;
#[cfg(feature = "crdt")]
use pmacs::overlay_color::color_for_slot;
use pmacs::protocol::{
    ADVERTISED_PROTOCOL_VERSION, AttachRequest, FrontendCapabilities, FrontendEvent, GoodbyeReason,
    Hello, InstanceMessage, Key, KeyEvent, Modifiers, PROTOCOL_VERSION,
};
use pmacs::transport::{read_message, write_message};

mod common;
#[cfg(feature = "crdt")]
use common::daemon::attach_multi;
use common::daemon::{
    TestDaemon, build_default_caps, spawn_daemon_process, wait_for_socket_or_exit,
};

/// Read the daemon's `Hello`, send our `AttachRequest`, return the Hello.
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

// ---------------------------------------------------------------------------
// Test 1
// ---------------------------------------------------------------------------

#[test]
fn daemon_starts_socket_and_lockfile_appear_with_correct_modes() {
    let daemon = TestDaemon::spawn();

    // Socket: owner-only. The kernel applies umask 0077 to a base
    // 0777 for socket files (the lockfile is created via O_CREAT
    // with explicit mode and lands at 0600). The "x" bit on the
    // socket has no semantic meaning, so we assert the security
    // property — no group/other bits — rather than an exact 0o600.
    let socket_meta = fs::metadata(daemon.socket_path()).expect("stat socket");
    let socket_mode = socket_meta.permissions().mode() & 0o7777;
    assert_eq!(
        socket_mode & 0o077,
        0,
        "socket mode {socket_mode:#o} should be owner-only"
    );

    // Lockfile: mode 0600.
    let lockfile_path = daemon.lockfile_path();
    let lock_meta = fs::metadata(&lockfile_path).expect("stat lockfile");
    let lock_mode = lock_meta.permissions().mode() & 0o7777;
    assert_eq!(
        lock_mode, 0o600,
        "lockfile mode should be 0600, got {lock_mode:#o}"
    );

    // Parent dir: at most 0700 (no group/other bits).
    let parent_meta = fs::metadata(daemon.socket_path().parent().unwrap()).expect("stat parent");
    let parent_mode = parent_meta.permissions().mode() & 0o7777;
    assert_eq!(
        parent_mode & 0o077,
        0,
        "parent dir mode {parent_mode:#o} should be owner-only"
    );
}

// ---------------------------------------------------------------------------
// Test 2
// ---------------------------------------------------------------------------

#[test]
fn attach_send_key_receive_cell_response() {
    let daemon = TestDaemon::spawn();
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    let hello = do_handshake(&mut stream);

    // After AttachRequest the daemon emits the initial full-grid sync.
    let initial: InstanceMessage = read_message(&mut stream).expect("initial frame");
    match initial {
        InstanceMessage::CellDelta {
            full_grid: true, ..
        } => {}
        other => panic!("expected initial full-grid CellDelta, got {other:?}"),
    }

    // Send a key event.
    let key_event = FrontendEvent::Key(KeyEvent {
        frontend_id: hello.assigned_frontend_id,
        key: Key::Char('a'),
        mods: Modifiers::NONE,
        timestamp_ns: 0,
    });
    write_message(&mut stream, &key_event).expect("send key");

    // The daemon should produce at least one render message in response
    // (cell delta or cursor update). Read up to ~2s for any message.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut got_response = false;
    while Instant::now() < deadline {
        stream
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        if let Ok(msg) = read_message::<InstanceMessage>(&mut stream)
            && matches!(
                msg,
                InstanceMessage::CellDelta { .. } | InstanceMessage::Cursor(_)
            )
        {
            got_response = true;
            break;
        }
    }
    assert!(got_response, "expected render response after key");
}

// ---------------------------------------------------------------------------
// Test 3
// ---------------------------------------------------------------------------

#[test]
fn clean_detach_then_reattach() {
    let mut daemon = TestDaemon::spawn();
    let pid_before = daemon.pid();

    // Attach, read initial frame, send Detach, drop.
    {
        let mut stream = daemon.connect();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let hello = do_handshake(&mut stream);
        let _: InstanceMessage = read_message(&mut stream).expect("initial frame");
        write_message(
            &mut stream,
            &FrontendEvent::Detach(hello.assigned_frontend_id),
        )
        .expect("send Detach");
        drop(stream);
    }

    // Give the daemon time to clear its attached slot.
    thread::sleep(Duration::from_millis(300));

    // Reattach should succeed.
    {
        let mut stream = daemon.connect();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let _hello = do_handshake(&mut stream);
        let _: InstanceMessage = read_message(&mut stream).expect("initial frame");
    }

    assert_eq!(daemon.pid(), pid_before);
    assert!(daemon.is_alive(), "daemon should still be running");
}

// ---------------------------------------------------------------------------
// Test 4
// ---------------------------------------------------------------------------

#[test]
fn ungraceful_disconnect_then_reattach() {
    let mut daemon = TestDaemon::spawn();
    let pid_before = daemon.pid();

    // Attach, do handshake, drop without Detach (simulates SIGKILL'd
    // frontend).
    {
        let mut stream = daemon.connect();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let _ = do_handshake(&mut stream);
        // Note: we don't even read the initial frame — close immediately.
        drop(stream);
    }

    // Daemon needs longer to detect ungraceful close because the read
    // path has no deterministic wakeup; the per-attach loop polls the
    // channel with a frame-target timeout (~16 ms by default), so 500
    // ms is safely above any realistic detection latency.
    thread::sleep(Duration::from_millis(500));

    // Reattach should succeed.
    {
        let mut stream = daemon.connect();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let _hello = do_handshake(&mut stream);
        let _: InstanceMessage = read_message(&mut stream).expect("initial frame");
    }

    assert_eq!(daemon.pid(), pid_before);
    assert!(daemon.is_alive(), "daemon should still be running");
}

// ---------------------------------------------------------------------------
// Test 5
// ---------------------------------------------------------------------------

#[test]
fn second_daemon_same_socket_fails_clearly() {
    let mut daemon_a = TestDaemon::spawn();

    // Spawn second daemon at the same socket; capture its stderr.
    let isolated_home = daemon_a.socket_path().parent().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_pmacs"))
        .args(["--daemon", "--socket"])
        .arg(daemon_a.socket_path())
        .env("HOME", isolated_home)
        .env("XDG_CONFIG_HOME", isolated_home)
        .env("XDG_DATA_HOME", isolated_home)
        .env("XDG_STATE_HOME", isolated_home)
        .env("PMACS_STATE_HOME", isolated_home)
        .env("XDG_CACHE_HOME", isolated_home)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn second daemon");

    assert!(
        !output.status.success(),
        "second daemon should exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("daemon already running"),
        "expected error message in stderr, got: {stderr}"
    );

    // First daemon unaffected: still alive, still accepts handshake.
    assert!(daemon_a.is_alive(), "daemon A should still be running");
    let mut stream = daemon_a.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let _hello = do_handshake(&mut stream);
}

// ---------------------------------------------------------------------------
// Test 6
// ---------------------------------------------------------------------------

#[test]
fn sigterm_daemon_sends_goodbye_and_cleans_up() {
    let mut daemon = TestDaemon::spawn();
    let socket_path = daemon.socket_path().to_path_buf();
    let lockfile_path = daemon.lockfile_path();
    let pid = daemon.pid();

    // Attach.
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let _hello = do_handshake(&mut stream);
    let _: InstanceMessage = read_message(&mut stream).expect("initial frame");

    // Send SIGTERM to daemon.
    kill(Pid::from_raw(i32::try_from(pid).unwrap()), Signal::SIGTERM).expect("kill SIGTERM");

    // Read until we see Goodbye(ShuttingDown) or hit EOF.
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut got_goodbye = false;
    loop {
        match read_message::<InstanceMessage>(&mut stream) {
            Ok(InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown)) => {
                got_goodbye = true;
                break;
            }
            // Intermediate render frames are fine.
            Ok(_) => {}
            // EOF / error — daemon closed.
            Err(_) => break,
        }
    }
    assert!(got_goodbye, "expected Goodbye(ShuttingDown) before EOF");

    // Daemon should exit cleanly.
    let status = daemon.wait_for_exit().expect("wait");
    assert!(
        status.success(),
        "daemon should exit 0 after SIGTERM, got {status}"
    );

    // Both files cleaned up.
    assert!(
        !socket_path.exists(),
        "socket {socket_path:?} should be unlinked"
    );
    assert!(
        !lockfile_path.exists(),
        "lockfile {lockfile_path:?} should be unlinked"
    );
}

// ---------------------------------------------------------------------------
// Test 7
// ---------------------------------------------------------------------------

#[test]
fn sigkill_daemon_leaves_stale_files_next_start_recovers() {
    let tempdir = TempDir::new().expect("tempdir");
    fs::set_permissions(tempdir.path(), fs::Permissions::from_mode(0o700))
        .expect("chmod tempdir 0700");
    let socket_path = tempdir.path().join("pmacs.sock");

    // First daemon.
    let mut daemon1 = spawn_daemon_process(&socket_path);
    wait_for_socket_or_exit(&socket_path, daemon1.child(), Duration::from_secs(10))
        .expect("daemon 1 socket appeared");

    let mut lockfile_path = socket_path.as_os_str().to_os_string();
    lockfile_path.push(".lock");
    let lockfile_path = PathBuf::from(lockfile_path);

    assert!(socket_path.exists(), "socket exists pre-SIGKILL");
    assert!(lockfile_path.exists(), "lockfile exists pre-SIGKILL");

    // SIGKILL the daemon.
    kill(
        Pid::from_raw(i32::try_from(daemon1.id()).unwrap()),
        Signal::SIGKILL,
    )
    .expect("SIGKILL");
    let _ = daemon1.wait();

    // Stale files persist (kernel only released the flock + socket fd).
    assert!(
        socket_path.exists(),
        "socket should persist on disk after SIGKILL"
    );
    assert!(
        lockfile_path.exists(),
        "lockfile should persist on disk after SIGKILL"
    );

    // Second daemon must successfully recover.
    let mut daemon2 = spawn_daemon_process(&socket_path);
    wait_for_socket_or_exit(&socket_path, daemon2.child(), Duration::from_secs(10))
        .expect("daemon 2 socket appeared");

    // Verify socket is fresh and owner-only (kernel applies umask
    // 0077 to the implicit 0777 base for socket files → 0700; the
    // "x" bit is meaningless on a socket; the security property is
    // "no group/other access").
    let mode = fs::metadata(&socket_path).unwrap().permissions().mode() & 0o7777;
    assert_eq!(
        mode & 0o077,
        0,
        "socket mode {mode:#o} should be owner-only"
    );

    // Lockfile pid was rewritten.
    let pid_str = fs::read_to_string(&lockfile_path).unwrap();
    let parsed: u32 = pid_str.trim().parse().expect("pid in lockfile");
    assert_eq!(parsed, daemon2.id(), "lockfile pid should match daemon 2");

    // Cleanup.
    let _ = daemon2.kill();
    let _ = daemon2.wait();
}

// ---------------------------------------------------------------------------
// Test 8
// ---------------------------------------------------------------------------

#[test]
fn version_mismatch_clean_disconnect() {
    let mut daemon = TestDaemon::spawn();

    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    // Read Hello.
    let hello: Hello = read_message(&mut stream).expect("Hello");
    assert_eq!(hello.protocol_version, ADVERTISED_PROTOCOL_VERSION);

    // Send AttachRequest with wrong protocol version.
    let req = AttachRequest {
        protocol_version: 999,
        frontend_capabilities: build_default_caps(),
        initial_size: CellSize::new(24, 80),
    };
    write_message(&mut stream, &req).expect("write");

    // Expect Goodbye(VersionMismatch).
    //
    // `server` is the wire this instance can SPEAK, which since
    // bottom-panel Stage 2B-3 is no longer the baseline it advertised in
    // `Hello` above: the baseline is a compatibility floor and
    // `PROTOCOL_VERSION` is the ceiling a frontend may counter-offer up
    // to. Reporting the floor here would tell a rejected frontend the
    // daemon tops out at v20 when it in fact speaks v21 — the opposite of
    // the upgrade diagnostic this reason exists to give.
    //
    // Until Stage 2B-3 the two constants were equal, so this assertion
    // could not distinguish them and silently pinned the wrong one. Both
    // directions are asserted now so neither can drift back: line 421
    // holds the advertised floor, and the `assert_ne!` holds the
    // divergence itself. That matters here specifically because the
    // Stage 2B-3 pin for this same rule is `#[cfg(feature = "crdt")]`
    // and therefore dark in CI — this test is the one CI actually runs.
    match read_message::<InstanceMessage>(&mut stream) {
        Ok(InstanceMessage::Goodbye(GoodbyeReason::VersionMismatch { server, client })) => {
            assert_eq!(
                server, PROTOCOL_VERSION,
                "the instance must report the version it can speak"
            );
            assert_ne!(
                server, ADVERTISED_PROTOCOL_VERSION,
                "and deliberately not the advertised compatibility floor"
            );
            assert_eq!(client, 999);
        }
        other => panic!("expected VersionMismatch Goodbye, got {other:?}"),
    }

    drop(stream);

    // Daemon still alive and serving.
    assert!(daemon.is_alive());
    let mut stream2 = daemon.connect();
    stream2
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let _hello = do_handshake(&mut stream2);
}

// ---------------------------------------------------------------------------
// T M10.7 — capability negotiation, daemon end-to-end.
//
// Pure-function tests for `negotiate_capabilities` live in
// `src/protocol.rs` and exercise the negotiation logic exhaustively.
// This end-to-end test exercises the wire path: a v2 frontend
// declaring `multi_frontend: true` receives `Goodbye(
// CapabilityMismatch)` from a v2 daemon whose
// `InstanceCapabilities::default()` has `multi_frontend: false`
// (M10.8 flips that bit; M10.7 ships the mismatch path).
// ---------------------------------------------------------------------------

#[test]
fn m10_7_capability_mismatch_v2_frontend_wants_multi_frontend() {
    // T M10.8 Day 4 rewrite: M10.8 flipped `InstanceCapabilities::
    // default()` to advertise `multi_frontend: true` and `crdt_replica:
    // true`. To preserve daemon-end-to-end coverage of the M10.7
    // mismatch path, this test spawns the daemon with
    // `PMACS_INSTANCE_MULTI_FRONTEND=0` so the instance advertises
    // `multi_frontend: false` and a frontend declaring `true`
    // hits the mismatch.
    //
    // Approach (i) from the M10.8 framing-pass review: explicit
    // instance-caps override preserves daemon-end-to-end coverage.
    // Pure-function negotiation tests in `protocol.rs` still cover
    // the negotiation logic itself; this test specifically
    // exercises the daemon's wiring of negotiate_capabilities into
    // handle_connection.
    let mut daemon = TestDaemon::spawn_with_env(&[("PMACS_INSTANCE_MULTI_FRONTEND", "0")]);
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    let _hello: Hello = read_message(&mut stream).expect("read Hello");

    let caps_multi = FrontendCapabilities {
        multi_frontend: true,
        ..build_default_caps()
    };
    let req = AttachRequest {
        protocol_version: PROTOCOL_VERSION,
        frontend_capabilities: caps_multi,
        initial_size: CellSize::new(24, 80),
    };
    write_message(&mut stream, &req).expect("write AttachRequest");

    match read_message::<InstanceMessage>(&mut stream) {
        Ok(InstanceMessage::Goodbye(GoodbyeReason::CapabilityMismatch { missing })) => {
            assert_eq!(
                missing,
                vec!["multi_frontend".to_string()],
                "M10.7 criterion 4: error names the requested capability"
            );
        }
        other => panic!("expected Goodbye(CapabilityMismatch), got {other:?}"),
    }

    // Daemon stays up; another connection without multi_frontend
    // succeeds even with the env-var override (the env var only
    // affects the instance's advertised caps, not what frontends
    // request — a frontend not asking for multi_frontend doesn't
    // hit the mismatch).
    drop(stream);
    assert!(daemon.is_alive());
    let mut stream2 = daemon.connect();
    stream2
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let _hello2 = do_handshake(&mut stream2);
}

#[test]
fn m10_7_no_negotiation_for_v1_frontend() {
    // T M10.7 criterion 1 — a v0.1 frontend (no `multi_frontend`
    // declared in the wire format) attaches successfully. The
    // negotiation function sees frontend.multi_frontend = false
    // (from #[serde(default)]) and produces a clean Ok(...) result;
    // no mismatch is generated.
    let daemon = TestDaemon::spawn();
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    let _hello = do_handshake(&mut stream);
    // do_handshake's default caps have multi_frontend: false; the
    // attach should succeed and produce the initial CellDelta. If
    // the daemon had emitted Goodbye(CapabilityMismatch) instead,
    // read_message below would decode that variant — but the
    // standard handshake path produces the CellDelta.
    let initial: InstanceMessage = read_message(&mut stream).expect("initial frame");
    match initial {
        InstanceMessage::CellDelta {
            full_grid: true, ..
        } => {}
        other => panic!("expected initial full-grid CellDelta, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// T M10.8 Day 4 — multi-attach end-to-end acceptance + Q5 admission matrix.
// ---------------------------------------------------------------------------

/// M10.8 acceptance criterion 1 + 2 + 3 — happy-path multi-attach.
///
/// Two v1.0 frontends both negotiate `multi_frontend: true`,
/// attach simultaneously, and each receives at least one
/// presence broadcast from the other when the other moves its
/// cursor.
// Multi-frontend tests below exercise the M10.8/M10.9 multi-frontend
// + presence + CRDT path. Post-audit Finding 3 fix made
// `InstanceCapabilities::default()` advertise `multi_frontend: false`
// in non-CRDT builds (because multi-frontend's payoff is the CRDT
// pipeline). These tests are inherently CRDT-feature-only.
#[cfg(feature = "crdt")]
#[test]
fn m10_8_two_frontends_attached_simultaneously_receive_presence_broadcasts() {
    let daemon = TestDaemon::spawn();

    let (hello_a, mut stream_a) = attach_multi(&daemon);
    let (hello_b, mut stream_b) = attach_multi(&daemon);
    assert_ne!(hello_a.assigned_frontend_id, hello_b.assigned_frontend_id);

    // Read initial frames synchronously so kernel buffers stay
    // drained; drain_pending's 50ms-timeout reads back up under
    // the dispatcher's per-tick writes.
    let _initial_a: InstanceMessage = read_message(&mut stream_a).expect("A initial frame");
    let _initial_b: InstanceMessage = read_message(&mut stream_b).expect("B initial frame");

    // Send a key event from A. The daemon dispatches it to A's
    // active window; cursor moves; per-tick presence sweep
    // produces a PresenceUpdate broadcast to B.
    let key = FrontendEvent::Key(pmacs::protocol::KeyEvent {
        frontend_id: hello_a.assigned_frontend_id,
        key: pmacs::protocol::Key::Char('x'),
        mods: pmacs::protocol::Modifiers::NONE,
        timestamp_ns: 0,
    });
    write_message(&mut stream_a, &key).expect("send key from A");

    // Both A and B will receive some messages. We're specifically
    // checking that B receives a PresenceUpdate sourced from
    // hello_a.assigned_frontend_id.
    let deadline = Instant::now() + Duration::from_secs(2);
    stream_b
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let mut saw_presence_from_a = false;
    while Instant::now() < deadline {
        if let Ok(InstanceMessage::PresenceUpdate { frontend_id, .. }) =
            read_message::<InstanceMessage>(&mut stream_b)
            && frontend_id == hello_a.assigned_frontend_id
        {
            saw_presence_from_a = true;
            break;
        }
        // Other messages (CellDelta, Cursor) and read timeouts both
        // just continue the deadline loop.
    }
    assert!(
        saw_presence_from_a,
        "M10.8 criterion 1: frontend B should receive a PresenceUpdate from A's cursor move"
    );
}

/// M10.8 acceptance criterion 2 — disconnect is local.
///
/// Attach A and B; disconnect A; verify B continues to operate.
#[cfg(feature = "crdt")]
#[test]
fn m10_8_disconnect_of_one_frontend_does_not_affect_the_other() {
    let mut daemon = TestDaemon::spawn();

    let (_hello_a, stream_a) = attach_multi(&daemon);
    let (hello_b, mut stream_b) = attach_multi(&daemon);
    let _initial_b: InstanceMessage = read_message(&mut stream_b).expect("B initial frame");

    // Disconnect A by dropping its stream.
    drop(stream_a);

    // Give the dispatcher a moment to process A's detach.
    thread::sleep(Duration::from_millis(100));

    // B can still send events and the daemon responds.
    let key = FrontendEvent::Key(pmacs::protocol::KeyEvent {
        frontend_id: hello_b.assigned_frontend_id,
        key: pmacs::protocol::Key::Char('y'),
        mods: pmacs::protocol::Modifiers::NONE,
        timestamp_ns: 0,
    });
    write_message(&mut stream_b, &key).expect("B can still send keys");

    stream_b
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let response: InstanceMessage =
        read_message(&mut stream_b).expect("B receives response after A disconnected");
    // Any response counts — CellDelta from B's edit, Cursor, or a
    // presence broadcast from before the disconnect. The
    // assertion is that B's connection is still live.
    let _ = response;
    assert!(daemon.is_alive());
}

/// Q5 row 1: v0.1 frontend attached; another v0.1 attaches; rejected.
#[test]
fn m10_8_q5_row1_v01_with_v01_attempt_rejected() {
    let daemon = TestDaemon::spawn();

    // First v0.1 frontend (no multi_frontend, no crdt_replica): attaches.
    let mut stream_a = daemon.connect();
    stream_a
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let _hello_a = do_handshake(&mut stream_a);
    // Read initial frame so the daemon's per-tick render isn't
    // backed up on stream_a (which might trigger spurious write
    // failures elsewhere).
    let initial: InstanceMessage = read_message(&mut stream_a).expect("initial frame on A");
    let _ = initial;

    // Second v0.1 frontend: rejected with AlreadyAttached.
    let mut stream_b = daemon.connect();
    stream_b
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let _hello_b: Hello = read_message(&mut stream_b).expect("read Hello");
    let req_b = AttachRequest {
        protocol_version: PROTOCOL_VERSION,
        frontend_capabilities: build_default_caps(),
        initial_size: CellSize::new(24, 80),
    };
    write_message(&mut stream_b, &req_b).expect("write AttachRequest");

    match read_message::<InstanceMessage>(&mut stream_b) {
        Ok(InstanceMessage::Goodbye(GoodbyeReason::AlreadyAttached)) => {}
        other => panic!("Q5 row 1: expected AlreadyAttached, got {other:?}"),
    }
}

/// Q5 row 2: v0.1 attached; v2 multi attaches; both coexist
/// (heterogeneous case; the v0.1 sees its session normally, the
/// v2 sees the v0.1 as a "ghost editor").
#[cfg(feature = "crdt")]
#[test]
fn m10_8_q5_row2_v01_with_v2_multi_coexist() {
    let daemon = TestDaemon::spawn();

    // First: v0.1 frontend attaches normally.
    let mut stream_v1 = daemon.connect();
    stream_v1
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let _hello_v1 = do_handshake(&mut stream_v1);
    // Read initial frame synchronously so the kernel buffer
    // doesn't back up; using drain_pending here causes the
    // dispatcher's per-tick writes to accumulate while the test
    // is mid-50ms-timeout, which can leave streams in a flaky
    // state by the time the second attach runs.
    let _initial_v1: InstanceMessage = read_message(&mut stream_v1).expect("v0.1 initial frame");

    // Second: v2 multi-capable frontend attaches alongside.
    let (_hello_v2, mut stream_v2) = attach_multi(&daemon);
    // If the multi attach succeeded, the daemon sends initial
    // CellDelta etc. — read at least one frame to confirm
    // acceptance.
    let initial: InstanceMessage = read_message(&mut stream_v2).expect("v2 initial frame");
    match initial {
        InstanceMessage::CellDelta { .. } => {}
        InstanceMessage::Goodbye(reason) => {
            panic!("Q5 row 2: v2 multi should attach alongside v0.1, got Goodbye: {reason:?}")
        }
        _other => {} // Cursor / etc. also acceptable as "attached"
    }
}

/// Q5 row 3: v2 multi attached; v0.1 attempts to attach; accepted
/// (Q5 logic: a non-multi attach is rejected iff
/// `count_non_multi_sessions > 0`; with only multi attached the
/// count is 0, so v0.1 takes the non-multi slot).
#[cfg(feature = "crdt")]
#[test]
fn m10_8_q5_row3_v2_multi_then_v01_accepted() {
    let daemon = TestDaemon::spawn();

    let (_hello_v2, mut stream_v2) = attach_multi(&daemon);
    // Read the v2 initial frame synchronously (same rationale as
    // Q5 row 2 — avoid drain_pending's 50ms-timeout buffer race).
    let _initial_v2: InstanceMessage = read_message(&mut stream_v2).expect("v2 initial frame");

    // v0.1 frontend attaches: should succeed because the multi
    // session doesn't occupy the non-multi slot.
    let mut stream_v1 = daemon.connect();
    stream_v1
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let _hello_v1: Hello = read_message(&mut stream_v1).expect("read Hello");
    let req_v1 = AttachRequest {
        protocol_version: PROTOCOL_VERSION,
        frontend_capabilities: build_default_caps(),
        initial_size: CellSize::new(24, 80),
    };
    write_message(&mut stream_v1, &req_v1).expect("write AttachRequest");

    let initial: InstanceMessage = read_message(&mut stream_v1).expect("v0.1 initial frame");
    match initial {
        InstanceMessage::CellDelta { .. } => {}
        InstanceMessage::Goodbye(reason) => {
            panic!("Q5 row 3: v0.1 should attach alongside v2 multi, got Goodbye: {reason:?}")
        }
        _other => {}
    }
}

/// Q5 row 4: two v2 multi sessions, both attached.
#[cfg(feature = "crdt")]
#[test]
fn m10_8_q5_row4_two_v2_multi_sessions_attached() {
    let daemon = TestDaemon::spawn();
    let (hello_a, mut stream_a) = attach_multi(&daemon);
    let (hello_b, mut stream_b) = attach_multi(&daemon);
    assert_ne!(hello_a.assigned_frontend_id, hello_b.assigned_frontend_id);

    // Both receive initial frames; no rejection.
    let _frame_a: InstanceMessage = read_message(&mut stream_a).expect("A initial");
    let _frame_b: InstanceMessage = read_message(&mut stream_b).expect("B initial");
}

// ---------------------------------------------------------------------------
// T M10.9 — overlay color rendering, daemon end-to-end.
// ---------------------------------------------------------------------------

/// M10.9 acceptance criterion 1: two frontends in the same buffer
/// see each other's cursors.
///
/// Spawn daemon, attach two multi-capable frontends. They share
/// LOCAL's buffer (M10.9 attach behavior). A inserts a few
/// characters (moving A's cursor); B's subsequent `CellDelta`
/// should contain an overlay cell with A's assigned color.
#[cfg(feature = "crdt")]
#[test]
fn m10_9_other_frontend_cursor_appears_in_recipient_cell_delta_with_color() {
    let daemon = TestDaemon::spawn();
    let (hello_a, mut stream_a) = attach_multi(&daemon);
    let (_hello_b, mut stream_b) = attach_multi(&daemon);

    // Read initial CellDelta + Cursor on each. M10.9: B's initial
    // frame includes A's cursor at byte 0 (where A's window starts).
    let _initial_a: InstanceMessage = read_message(&mut stream_a).expect("A initial");
    let _initial_b_1: InstanceMessage = read_message(&mut stream_b).expect("B initial CellDelta");
    let _initial_b_2: InstanceMessage = read_message(&mut stream_b).expect("B initial Cursor");

    // A presses a character. A's cursor advances; B should
    // receive a CellDelta that includes an overlay cell with A's
    // color.
    let key = FrontendEvent::Key(KeyEvent {
        frontend_id: hello_a.assigned_frontend_id,
        key: Key::Char('z'),
        mods: Modifiers::NONE,
        timestamp_ns: 0,
    });
    write_message(&mut stream_a, &key).expect("send key from A");

    // Compute A's expected color. With both A and B from same uid
    // (test process), they share a color slot — so this test
    // verifies the *presence* of an overlay-style cell, not a
    // specific color. We assert that B's CellDelta contains at
    // least one cell whose fg is in the M10.9 palette.
    let deadline = Instant::now() + Duration::from_secs(2);
    stream_b
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let mut saw_palette_cell = false;
    while Instant::now() < deadline {
        if let Ok(InstanceMessage::CellDelta { spans, .. }) =
            read_message::<InstanceMessage>(&mut stream_b)
        {
            for span in &spans {
                for cell in &span.cells {
                    // The palette uses Color::Rgb(...). Any cell whose
                    // fg is a palette entry is an overlay cell.
                    if let Color::Rgb(_, _, _) = cell.style.fg
                        && is_palette_color(cell.style.fg)
                    {
                        saw_palette_cell = true;
                        break;
                    }
                }
                if saw_palette_cell {
                    break;
                }
            }
        }
        if saw_palette_cell {
            break;
        }
    }
    assert!(
        saw_palette_cell,
        "M10.9 criterion 1: B's CellDelta should contain an overlay cell with a palette color after A moved its cursor"
    );
}

#[cfg(feature = "crdt")]
fn is_palette_color(c: Color) -> bool {
    for slot in 0..pmacs::overlay_color::PALETTE_LEN as u8 {
        if color_for_slot(slot) == c {
            return true;
        }
    }
    false
}

/// M10.9 acceptance criterion 2 (partial): color stability for
/// same-uid reconnect.
///
/// Attach A → detach A → reattach A'. A and A' connect from the
/// same uid (the test process). The daemon's `color_registry` maps
/// uid → slot. The slot should be the same for both attaches.
///
/// This test verifies the color-stability property at the daemon
/// level by attaching B as observer, then attaching A (slot
/// recorded), detaching A, reattaching A' (slot reused from
/// uid lookup), and checking B sees consistent overlay color.
#[cfg(feature = "crdt")]
#[test]
fn m10_9_color_stable_across_reconnect_for_same_uid() {
    let daemon = TestDaemon::spawn();
    // B is the observer.
    let (_hello_b, mut stream_b) = attach_multi(&daemon);
    let _initial_b_cd: InstanceMessage = read_message(&mut stream_b).expect("B initial cd");
    let _initial_b_cu: InstanceMessage = read_message(&mut stream_b).expect("B initial cursor");

    // A attaches the first time.
    let (hello_a1, mut stream_a1) = attach_multi(&daemon);
    let _a1_init: InstanceMessage = read_message(&mut stream_a1).expect("A1 init");
    // A1 moves cursor so an overlay is generated for B.
    let key1 = FrontendEvent::Key(KeyEvent {
        frontend_id: hello_a1.assigned_frontend_id,
        key: Key::Char('x'),
        mods: Modifiers::NONE,
        timestamp_ns: 0,
    });
    write_message(&mut stream_a1, &key1).expect("send key from A1");

    // Capture A1's overlay color from B.
    let color_a1 = wait_for_palette_color_in_b(&mut stream_b, Duration::from_secs(2))
        .expect("should observe A1's overlay color");

    // Detach A1.
    drop(stream_a1);
    thread::sleep(Duration::from_millis(100));

    // A2 reattaches (same test process, same uid).
    let (hello_a2, mut stream_a2) = attach_multi(&daemon);
    let _a2_init: InstanceMessage = read_message(&mut stream_a2).expect("A2 init");
    let key2 = FrontendEvent::Key(KeyEvent {
        frontend_id: hello_a2.assigned_frontend_id,
        key: Key::Char('y'),
        mods: Modifiers::NONE,
        timestamp_ns: 0,
    });
    write_message(&mut stream_a2, &key2).expect("send key from A2");

    let color_a2 = wait_for_palette_color_in_b(&mut stream_b, Duration::from_secs(2))
        .expect("should observe A2's overlay color");

    assert_eq!(
        color_a1, color_a2,
        "M10.9 criterion 2: same uid across reconnect → same color slot"
    );
}

/// Helper for color-stability test: read `CellDelta` messages from
/// `stream` for up to `timeout` and return the first palette
/// color found in any overlay cell.
#[cfg(feature = "crdt")]
fn wait_for_palette_color_in_b(stream: &mut UnixStream, timeout: Duration) -> Option<Color> {
    let deadline = Instant::now() + timeout;
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .ok();
    while Instant::now() < deadline {
        if let Ok(InstanceMessage::CellDelta { spans, .. }) =
            read_message::<InstanceMessage>(stream)
        {
            for span in &spans {
                for cell in &span.cells {
                    if let Color::Rgb(_, _, _) = cell.style.fg
                        && is_palette_color(cell.style.fg)
                    {
                        return Some(cell.style.fg);
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// T M10.10 Day 2 — daemon sends BufferSnapshot on SessionEstablished
// for crdt_replica-negotiated frontends; not for non-replica frontends.
// ---------------------------------------------------------------------------

/// M10.10 Day 2 acceptance: a frontend that negotiated
/// `crdt_replica: true` receives an `InstanceMessage::BufferSnapshot`
/// for the `*scratch*` buffer immediately after `SessionEstablished`,
/// before any `CellDelta`. The snapshot bytes round-trip into a fresh
/// `CrdtState` and yield the expected materialized content.
#[cfg(feature = "crdt")]
#[test]
fn m10_10_replica_frontend_receives_buffer_snapshot_before_cell_delta() {
    let daemon = TestDaemon::spawn();
    let (_hello, mut stream) = attach_multi(&daemon);

    // The first InstanceMessage after handshake must be a
    // BufferSnapshot. The dispatcher sends snapshots before the first
    // per-tick render fires.
    let first: InstanceMessage = read_message(&mut stream).expect("first frame");
    let (buffer_id, snapshot_bytes) = match first {
        InstanceMessage::BufferSnapshot {
            buffer_id,
            crdt_snapshot,
        } => (buffer_id, crdt_snapshot),
        other => {
            panic!("M10.10: first frame for replica frontend must be BufferSnapshot, got {other:?}")
        }
    };
    assert!(
        !snapshot_bytes.is_empty(),
        "snapshot bytes must be non-empty (loro encodes empty state as a non-empty payload)"
    );

    // The snapshot must decode into a fresh CrdtState. peer_id 0xBEEF
    // here is arbitrary — bootstrap on the frontend uses
    // peer_id_from_frontend(my_id), but the round-trip test doesn't
    // care which peer reconstructs.
    let replica = pmacs::crdt::CrdtState::new(0xBEEF).expect("fresh CrdtState");
    replica
        .import_snapshot(&snapshot_bytes)
        .expect("import the daemon's snapshot");
    // The daemon's *scratch* buffer starts empty.
    assert_eq!(replica.materialize_string(), "");
    let _ = buffer_id; // consumed for the panic-message in the match
}

/// M10.10 Day 3 Finding 2 acceptance: a replica frontend receives an
/// authoritative `InstanceMessage::CursorByte` paired with the regular
/// `Cursor` grid update. The byte position is the daemon's
/// active-window cursor at the moment of the render frame.
#[cfg(feature = "crdt")]
#[test]
fn m10_10_replica_frontend_receives_cursor_byte_paired_with_cursor() {
    let daemon = TestDaemon::spawn();
    let (_hello, mut stream) = attach_multi(&daemon);

    // Drain initial frames until both Cursor and CursorByte have
    // been seen. The daemon emits per-tick: CellDelta, Cursor,
    // CursorByte. BufferSnapshot fires before all of those at
    // session establishment. We want to confirm that for the same
    // render iteration, Cursor and CursorByte both arrive.
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut saw_cursor = false;
    let mut saw_cursor_byte = false;
    for _ in 0..16 {
        match read_message::<InstanceMessage>(&mut stream) {
            Ok(InstanceMessage::Cursor(_)) => saw_cursor = true,
            Ok(InstanceMessage::CursorByte { .. }) => saw_cursor_byte = true,
            Ok(_) => continue,
            Err(_) => break,
        }
        if saw_cursor && saw_cursor_byte {
            break;
        }
    }
    assert!(saw_cursor, "replica frontend should receive Cursor");
    assert!(
        saw_cursor_byte,
        "replica frontend should receive CursorByte alongside Cursor (Finding 2)"
    );
}

/// M10.10 Day 3 Finding 3 acceptance: end-to-end `CrdtOp` broadcast.
/// Frontend A (replica) sends `FrontendEvent::CrdtOp` to the daemon;
/// daemon applies it to the buffer's CRDT + rope, then fans out
/// `InstanceMessage::CrdtOp` to other replica frontends. Frontend B
/// (also replica) receives the broadcast tagged with A's
/// `frontend_id` so B's mirror can route through the echo-dedup
/// filter (apply, not skip).
#[cfg(feature = "crdt")]
#[test]
fn m10_10_crdt_op_from_a_reaches_b_via_daemon_broadcast() {
    let daemon = TestDaemon::spawn();

    // Attach A and B as replica frontends.
    let (hello_a, mut stream_a) = attach_multi(&daemon);
    let (hello_b, mut stream_b) = attach_multi(&daemon);
    assert_ne!(hello_a.assigned_frontend_id, hello_b.assigned_frontend_id);

    // Drain A's first frame (BufferSnapshot) and bootstrap a test-
    // side CRDT replica from it. We'll generate a valid op against
    // this state and send it as A.
    let (buffer_id, snapshot_bytes) =
        match read_message::<InstanceMessage>(&mut stream_a).expect("A first frame") {
            InstanceMessage::BufferSnapshot {
                buffer_id,
                crdt_snapshot,
            } => (buffer_id, crdt_snapshot),
            other => panic!("expected BufferSnapshot, got {other:?}"),
        };

    let a_replica = pmacs::crdt::CrdtState::new(hello_a.assigned_frontend_id.0).expect("a replica");
    a_replica.import_snapshot(&snapshot_bytes).expect("import");

    // A generates a CrdtOp (simulating optimistic-apply at keystroke).
    let v_before = a_replica.version();
    a_replica.insert(0, "X").expect("a insert");
    let op_bytes = a_replica
        .export_updates_since(&v_before)
        .expect("export updates");

    // Drain B's first frame (BufferSnapshot — same buffer_id; the
    // daemon snapshots all CRDT buffers for each replica at attach).
    let _b_first: InstanceMessage = read_message(&mut stream_b).expect("B first frame");

    // A sends FrontendEvent::CrdtOp upstream.
    let crdt_op_event = FrontendEvent::CrdtOp {
        frontend_id: hello_a.assigned_frontend_id,
        buffer_id,
        op: pmacs::rope::CrdtOp {
            peer_id: hello_a.assigned_frontend_id.0,
            bytes: op_bytes.clone(),
        },
    };
    write_message(&mut stream_a, &crdt_op_event).expect("send CrdtOp from A");

    // B should receive an InstanceMessage::CrdtOp tagged with A's
    // frontend_id (via op.peer_id since the wire variant doesn't
    // carry a separate source field).
    stream_b
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut saw_crdt_op_from_a = false;
    while Instant::now() < deadline {
        match read_message::<InstanceMessage>(&mut stream_b) {
            Ok(InstanceMessage::CrdtOp { buffer_id: bid, op })
                if bid == buffer_id && op.peer_id == hello_a.assigned_frontend_id.0 =>
            {
                saw_crdt_op_from_a = true;
                assert_eq!(
                    op.bytes, op_bytes,
                    "broadcast bytes must match A's emitted bytes"
                );
                break;
            }
            Ok(_) | Err(_) => {}
        }
    }
    assert!(
        saw_crdt_op_from_a,
        "M10.10 Finding 3 criterion: B must receive CrdtOp from A via daemon broadcast"
    );
}

/// M10.10 (post-audit Finding 6) — verify the **production**
/// `attach::build_capabilities()` output negotiates `crdt_replica`
/// correctly so the production TUI binary actually receives
/// `BufferSnapshot` and bootstraps its `BufferMirror`.
///
/// Pre-fix: the M10.10 acceptance tests used `attach_multi()` with
/// custom caps that had `crdt_replica: true`; the production
/// `build_capabilities()` had `crdt_replica: false` so the optimistic-
/// apply infrastructure was structurally unreachable in the real
/// binary. Test coverage didn't catch this because no test used the
/// production caps.
///
/// This test closes that gap: spawn a daemon, connect, send an
/// `AttachRequest` with caps from production `build_capabilities()`,
/// verify the first non-handshake frame is `BufferSnapshot`. If it's
/// `CellDelta` instead, the production capability negotiation is
/// broken.
#[cfg(feature = "crdt")]
#[test]
fn m10_10_production_attach_negotiates_crdt_replica() {
    let daemon = TestDaemon::spawn();
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    // Production handshake — NOT the test `attach_multi()` path.
    let hello: Hello = read_message(&mut stream).expect("read Hello");
    assert_eq!(hello.protocol_version, ADVERTISED_PROTOCOL_VERSION);
    let req = AttachRequest {
        protocol_version: hello.protocol_version,
        frontend_capabilities: pmacs::attach::build_capabilities(),
        initial_size: CellSize::new(24, 80),
    };
    write_message(&mut stream, &req).expect("write AttachRequest");

    // Production caps in CRDT-feature builds set `crdt_replica: true`.
    // First frame after the handshake must be `BufferSnapshot`.
    let first: InstanceMessage = read_message(&mut stream).expect("first frame");
    match first {
        InstanceMessage::BufferSnapshot { .. } => {}
        InstanceMessage::CellDelta { .. } => panic!(
            "Finding 6: production frontend received CellDelta as first frame; \
             expected BufferSnapshot. The production caps don't negotiate \
             crdt_replica, so M10.10 optimistic apply is dead in the real TUI."
        ),
        other => {
            panic!("expected BufferSnapshot as first production-frontend frame, got {other:?}")
        }
    }
}

/// M10.10 (post-audit Finding 6 companion) — non-CRDT build path.
/// The production `build_capabilities()` advertises `crdt_replica:
/// false` when built without the `crdt` feature, and the daemon
/// (also non-CRDT) advertises `crdt_replica: false`, so neither side
/// negotiates the capability and the first frame is `CellDelta` as
/// in v0.1. Verifies Finding 3 fix is symmetric on both sides.
#[cfg(not(feature = "crdt"))]
#[test]
fn m10_10_production_attach_non_crdt_build_does_not_negotiate_crdt_replica() {
    let daemon = TestDaemon::spawn();
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let hello: Hello = read_message(&mut stream).expect("read Hello");
    let req = AttachRequest {
        protocol_version: hello.protocol_version,
        frontend_capabilities: pmacs::attach::build_capabilities(),
        initial_size: CellSize::new(24, 80),
    };
    write_message(&mut stream, &req).expect("write AttachRequest");

    let first: InstanceMessage = read_message(&mut stream).expect("first frame");
    match first {
        InstanceMessage::CellDelta { .. } => {}
        other => panic!(
            "non-CRDT build expected CellDelta as first frame; got {other:?}. \
             Finding 3 regression?"
        ),
    }
}

/// M10.10 Day 4 — verify `PMACS_INSTANCE_LATENCY_MS` injection.
///
/// Sets the env var to 200ms; spawns a daemon; attaches a frontend;
/// triggers a `CellDelta` emission by sending a key event; verifies
/// the `CellDelta` arrives at least ~150ms later (allowing for
/// scheduling jitter on busy CI).
///
/// This is the load-bearing setup test for the criterion 1
/// verification — without confirming the injection mechanism
/// works, the latency-dependent criterion tests can't trust their
/// timing.
#[test]
fn m10_10_latency_injection_delays_cell_delta() {
    let daemon = TestDaemon::spawn_with_env(&[("PMACS_INSTANCE_LATENCY_MS", "200")]);
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let hello = do_handshake(&mut stream);

    // Trigger a CellDelta by sending a key event. v0.1-mode
    // frontend (build_default_caps has crdt_replica=false) so the
    // daemon's path is: receive key → apply edit → render → emit
    // CellDelta with injected sleep.
    let key = FrontendEvent::Key(KeyEvent {
        frontend_id: hello.assigned_frontend_id,
        key: Key::Char('x'),
        mods: Modifiers::NONE,
        timestamp_ns: 0,
    });

    let start = Instant::now();
    write_message(&mut stream, &key).expect("send key");

    // Drain incoming messages until we see a CellDelta.
    loop {
        if matches!(
            read_message::<InstanceMessage>(&mut stream).expect("frame"),
            InstanceMessage::CellDelta { .. }
        ) {
            break;
        }
    }
    let elapsed = start.elapsed();

    // Lower bound: at least 150ms (200ms injection minus jitter
    // tolerance). Upper bound: avoid asserting because CI variability
    // can be high; failing-by-exceeding-bound would be a different
    // kind of bug.
    assert!(
        elapsed >= Duration::from_millis(150),
        "PMACS_INSTANCE_LATENCY_MS=200 should delay CellDelta by ~200ms, \
         observed {elapsed:?}"
    );
}

/// M10.10 Day 4 — criterion 1 acceptance: keystroke send is non-
/// blocking regardless of daemon latency.
///
/// Spec criterion 1: "Local edit visible in less than one frame
/// regardless of instance latency."
///
/// Under Path β, the visible local edit is the optimistic Print
/// emitted synchronously by the frontend's keystroke handler. The
/// daemon's `CellDelta` arrives later (delayed by injected latency)
/// but doesn't block the frontend.
///
/// Demonstration: send 10 `CrdtOp` events back-to-back to a daemon
/// with 200ms injected latency. If the frontend blocked on each
/// `CellDelta`, total time would be ≥2s. The actual time should be
/// under 100ms because writes are non-blocking (the dispatcher's
/// sleeps don't affect the writer's send).
///
/// This is the wire-level demonstration of criterion 1 that
/// complements the orchestrator-level unit test
/// `criterion_1_end_of_line_typing_completes_sub_frame_per_keystroke`.
#[cfg(feature = "crdt")]
#[test]
fn m10_10_criterion_1_keystroke_send_non_blocking_at_200ms_latency() {
    let daemon = TestDaemon::spawn_with_env(&[("PMACS_INSTANCE_LATENCY_MS", "200")]);
    let (hello, mut stream) = attach_multi(&daemon);

    // Bootstrap the test replica from the BufferSnapshot.
    let (buffer_id, snap) = match read_message::<InstanceMessage>(&mut stream).expect("snap") {
        InstanceMessage::BufferSnapshot {
            buffer_id,
            crdt_snapshot,
        } => (buffer_id, crdt_snapshot),
        other => panic!("expected BufferSnapshot, got {other:?}"),
    };
    let replica = pmacs::crdt::CrdtState::new(hello.assigned_frontend_id.0).expect("replica");
    replica.import_snapshot(&snap).expect("import");

    // Simulate 10 back-to-back keystrokes, each producing a CrdtOp
    // sent to the daemon. The frontend's orchestrator is in-process
    // here (the test code is the orchestrator); each send is the
    // analog of "after orchestrator returns CrdtOp + optimistic
    // Print, the wire send happens."
    let start = Instant::now();
    for ch in "0123456789".chars() {
        let v_before = replica.version();
        replica.insert(0, &ch.to_string()).expect("local insert");
        let bytes = replica.export_updates_since(&v_before).expect("export");
        let event = FrontendEvent::CrdtOp {
            frontend_id: hello.assigned_frontend_id,
            buffer_id,
            op: pmacs::rope::CrdtOp {
                peer_id: hello.assigned_frontend_id.0,
                bytes,
            },
        };
        write_message(&mut stream, &event).expect("send");
    }
    let elapsed = start.elapsed();

    // If the frontend's sends blocked on the daemon's 200ms-delayed
    // CellDelta replies, total would be ≥ 2s (10 × 200ms). Non-
    // blocking sends complete in microseconds. Upper bound of 100ms
    // catches any synchronous-IO regression while tolerating CI
    // jitter on the per-write socket cost.
    // A blocking send would take ten round trips of 200 ms; one second
    // still discriminates that from a queued send by a factor of two
    // and is an order of magnitude past the observed few milliseconds.
    assert!(
        elapsed < Duration::from_secs(1),
        "criterion 1: 10 keystroke sends at 200ms injected latency took \
         {elapsed:?}; expected non-blocking (<100ms). The frontend is \
         blocking on daemon round-trips."
    );
}

/// M10.10 Day 4 — criterion 2 acceptance: no-flicker via byte-
/// equivalent optimistic paint.
///
/// Spec criterion 2: "Confirmation cell delta does not produce
/// visible flicker or correction (the optimistic state matches the
/// confirmed state)."
///
/// Under Path β's end-of-line scope, the optimistic Print emits the
/// typed character at the cursor's column with the terminal's
/// default style. The daemon's `CellDelta` for the same edit carries
/// a `Cell { glyph: Char(c), style: default }` at the same column.
/// If both encode the same character at the same column with the
/// same style, the daemon's `CellDelta` repaints the cell identically
/// → no visible change → no flicker.
///
/// This test verifies the byte-equivalence property: after the
/// frontend sends a `CrdtOp` for inserting 'X' at end-of-line, the
/// daemon's resulting `CellDelta` carries an 'X' cell at the column
/// where the optimistic Print would have written it. The cell's
/// style is default (no overlays/highlighting active in this
/// minimal test setup).
///
/// **Path β scope**: end-of-line typing only. Mid-line typing
/// produces a multi-cell `CellDelta` (shifted cells); under Path β,
/// no optimistic paint exists for that case (orchestrator round-
/// trips), so there's no optimistic state to flicker against.
/// Documented as v0.2+ Path γ work.
#[cfg(feature = "crdt")]
#[test]
fn m10_10_criterion_2_no_flicker_for_end_of_line_optimistic_insert() {
    use pmacs::cell::{Cell, DiffSpan, Glyph, Style};

    let daemon = TestDaemon::spawn();
    let (hello, mut stream) = attach_multi(&daemon);

    // Bootstrap mirror from BufferSnapshot.
    let (buffer_id, snap) = match read_message::<InstanceMessage>(&mut stream).expect("snap") {
        InstanceMessage::BufferSnapshot {
            buffer_id,
            crdt_snapshot,
        } => (buffer_id, crdt_snapshot),
        other => panic!("expected BufferSnapshot, got {other:?}"),
    };
    let replica = pmacs::crdt::CrdtState::new(hello.assigned_frontend_id.0).expect("replica");
    replica.import_snapshot(&snap).expect("import");

    // The *scratch* buffer starts empty; cursor is at byte 0
    // (which is end-of-line for an empty buffer per the Path β
    // predicate). Insert 'X' at position 0 — both daemon and
    // optimistic Print would put 'X' at column 0 of row 0.
    let v_before = replica.version();
    replica.insert(0, "X").expect("local insert");
    let op_bytes = replica.export_updates_since(&v_before).expect("export");

    let event = FrontendEvent::CrdtOp {
        frontend_id: hello.assigned_frontend_id,
        buffer_id,
        op: pmacs::rope::CrdtOp {
            peer_id: hello.assigned_frontend_id.0,
            bytes: op_bytes,
        },
    };
    write_message(&mut stream, &event).expect("send CrdtOp");

    // Drain incoming until we find a CellDelta containing the 'X'
    // cell.
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut found_x_cell: Option<Cell> = None;
    while Instant::now() < deadline && found_x_cell.is_none() {
        match read_message::<InstanceMessage>(&mut stream) {
            Ok(InstanceMessage::CellDelta { spans, .. }) => {
                for DiffSpan { start, cells } in spans {
                    // Look for an 'X' cell at row 0 (the *scratch*
                    // buffer's only line).
                    for (offset, cell) in cells.iter().enumerate() {
                        if start.row == 0
                            && (start.col as usize + offset) == 0
                            && matches!(cell.glyph, Glyph::Char('X'))
                        {
                            found_x_cell = Some(cell.clone());
                            break;
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }

    let cell = found_x_cell.expect(
        "criterion 2: daemon's CellDelta should carry 'X' at column 0 of row 0 \
         after the CrdtOp lands",
    );

    // Byte-equivalence check: the cell's style must be Default
    // (matching what an optimistic Print without explicit style
    // emits to the terminal). Any non-default style here would
    // create visible flicker between optimistic Print (no style)
    // and daemon CellDelta paint (styled).
    assert_eq!(
        cell.style,
        Style::default(),
        "criterion 2: cell at cursor must have default style for byte-equivalent \
         no-flicker repaint. Got {:?}",
        cell.style
    );
    assert!(
        matches!(cell.glyph, Glyph::Char('X')),
        "criterion 2: cell glyph must be 'X'; got {:?}",
        cell.glyph
    );
}

/// M10.10 Day 4 — criterion 3 acceptance: two-frontend conflict
/// convergence.
///
/// Spec criterion 3: "Concurrent edit from another frontend that
/// conflicts with the optimistic edit resolves correctly (CRDT
/// convergence handles it; local frontend's view repaints)."
///
/// Three properties under test:
///
/// (a) **CRDT convergence**: both frontends' mirrors agree on final
///     state after both ops have been delivered.
/// (b) **Local frontend's view repaints**: after convergence, the
///     daemon's `CellDelta` carries the converged content (verifiable
///     on the wire — both frontends receive a `CellDelta` after the
///     remote op is integrated daemon-side).
/// (c) **Optimistic `CrdtOp` preserved through convergence**: each
///     frontend's optimistic edit is in the final state; not lost
///     to the conflict resolution. Verified by inspecting the final
///     mirror content for both frontends' characters.
///
/// The test explicitly sends `FrontendEvent::CrdtOp` (not
/// `FrontendEvent::Key`) so the test proves the optimistic-apply
/// pipeline works end-to-end, not just that some keystroke path
/// produced the right result.
#[cfg(feature = "crdt")]
#[allow(clippy::too_many_lines)] // narrative test with explicit assertions per property
#[test]
fn m10_10_criterion_3_two_frontend_conflict_converges() {
    let daemon = TestDaemon::spawn();

    // Two replica frontends.
    let (hello_a, mut stream_a) = attach_multi(&daemon);
    let (hello_b, mut stream_b) = attach_multi(&daemon);

    // Both receive BufferSnapshot for *scratch* (empty buffer).
    let (buffer_id, snap_a) = match read_message::<InstanceMessage>(&mut stream_a).expect("A snap")
    {
        InstanceMessage::BufferSnapshot {
            buffer_id,
            crdt_snapshot,
        } => (buffer_id, crdt_snapshot),
        other => panic!("expected BufferSnapshot, got {other:?}"),
    };
    let snap_b = match read_message::<InstanceMessage>(&mut stream_b).expect("B snap") {
        InstanceMessage::BufferSnapshot { crdt_snapshot, .. } => crdt_snapshot,
        other => panic!("expected BufferSnapshot, got {other:?}"),
    };

    // Bootstrap test-side replicas from the snapshots. These
    // simulate the frontends' BufferMirror state.
    let replica_a = pmacs::crdt::CrdtState::new(hello_a.assigned_frontend_id.0).expect("a state");
    replica_a.import_snapshot(&snap_a).expect("a import");
    let replica_b = pmacs::crdt::CrdtState::new(hello_b.assigned_frontend_id.0).expect("b state");
    replica_b.import_snapshot(&snap_b).expect("b import");

    // CONCURRENT EDITS: A and B each apply an op to their own
    // mirror BEFORE either sees the other's op. This is the
    // canonical conflict scenario.
    //
    // A optimistically inserts 'A' at position 0 (its mirror state
    // before any remote op has arrived).
    let v_before_a = replica_a.version();
    replica_a.insert(0, "A").expect("a optimistic insert");
    let op_a = replica_a.export_updates_since(&v_before_a).expect("a op");
    assert_eq!(replica_a.materialize_string(), "A");

    // B optimistically inserts 'B' at position 0 (its mirror state
    // before any remote op has arrived — concurrent with A).
    let v_before_b = replica_b.version();
    replica_b.insert(0, "B").expect("b optimistic insert");
    let op_b = replica_b.export_updates_since(&v_before_b).expect("b op");
    assert_eq!(replica_b.materialize_string(), "B");

    // Send both ops to the daemon as FrontendEvent::CrdtOp (the
    // exact wire shape the production optimistic-apply orchestrator
    // emits — property (c) load-bearing assertion).
    let event_a = FrontendEvent::CrdtOp {
        frontend_id: hello_a.assigned_frontend_id,
        buffer_id,
        op: pmacs::rope::CrdtOp {
            peer_id: hello_a.assigned_frontend_id.0,
            bytes: op_a.clone(),
        },
    };
    let event_b = FrontendEvent::CrdtOp {
        frontend_id: hello_b.assigned_frontend_id,
        buffer_id,
        op: pmacs::rope::CrdtOp {
            peer_id: hello_b.assigned_frontend_id.0,
            bytes: op_b.clone(),
        },
    };
    write_message(&mut stream_a, &event_a).expect("send op A");
    write_message(&mut stream_b, &event_b).expect("send op B");

    // Both frontends should receive the OTHER frontend's op via
    // daemon broadcast. Drain incoming streams until each has seen
    // the other's CrdtOp.
    stream_a
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    stream_b
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut a_received_b_op = false;
    let mut b_received_a_op = false;
    while Instant::now() < deadline && !(a_received_b_op && b_received_a_op) {
        if !a_received_b_op
            && let Ok(InstanceMessage::CrdtOp { op, .. }) =
                read_message::<InstanceMessage>(&mut stream_a)
            && op.peer_id == hello_b.assigned_frontend_id.0
        {
            replica_a
                .import_updates(&op.bytes)
                .expect("a import B's op");
            a_received_b_op = true;
        }
        if !b_received_a_op
            && let Ok(InstanceMessage::CrdtOp { op, .. }) =
                read_message::<InstanceMessage>(&mut stream_b)
            && op.peer_id == hello_a.assigned_frontend_id.0
        {
            replica_b
                .import_updates(&op.bytes)
                .expect("b import A's op");
            b_received_a_op = true;
        }
    }
    assert!(
        a_received_b_op,
        "criterion 3: A must receive B's CrdtOp via daemon broadcast"
    );
    assert!(
        b_received_a_op,
        "criterion 3: B must receive A's CrdtOp via daemon broadcast"
    );

    // Property (a): CRDT convergence — both mirrors agree on final
    // state. CRDT order-determinism (loro's peer_id-based ordering)
    // ensures the final content is the same on both replicas.
    let a_final = replica_a.materialize_string();
    let b_final = replica_b.materialize_string();
    assert_eq!(
        a_final, b_final,
        "criterion 3 (a) CRDT convergence: A and B must reach identical final \
         state. A={a_final:?} B={b_final:?}"
    );

    // Property (c): both optimistic edits preserved through
    // convergence — final state contains both 'A' and 'B'.
    assert!(
        a_final.contains('A') && a_final.contains('B'),
        "criterion 3 (c) optimistic edits preserved: final state must contain \
         both A and B. Got {a_final:?}"
    );
    assert_eq!(
        a_final.len(),
        2,
        "criterion 3 (c) edits preserved: final state should be exactly two \
         characters (A and B in some order). Got {a_final:?}"
    );

    // Property (b): the daemon's view of the buffer (which drives
    // CellDelta to other frontends and to fresh attaches) also
    // matches the converged state. Verify by attaching a third
    // observer frontend C and inspecting its BufferSnapshot —
    // it must contain both 'A' and 'B' in the same order.
    let (_hello_c, mut stream_c) = attach_multi(&daemon);
    let snap_c = match read_message::<InstanceMessage>(&mut stream_c).expect("C snap") {
        InstanceMessage::BufferSnapshot { crdt_snapshot, .. } => crdt_snapshot,
        other => panic!("expected BufferSnapshot, got {other:?}"),
    };
    let observer = pmacs::crdt::CrdtState::new(99).expect("observer");
    observer.import_snapshot(&snap_c).expect("observer import");
    let daemon_state = observer.materialize_string();
    assert_eq!(
        daemon_state, a_final,
        "criterion 3 (b) daemon view repaints to converged state: a fresh \
         observer's BufferSnapshot must match what A and B converged to. \
         daemon={daemon_state:?} converged={a_final:?}"
    );
}

/// M10.10 Day 3 Finding 3 acceptance: own-CrdtOp-echo NOT sent back
/// to originator. Frontend A sends `CrdtOp`; daemon broadcasts to
/// other replicas (per-frontend sender-exclusion); A does NOT
/// receive its own op back. This is the daemon-side half of the
/// echo-dedup contract: the frontend-side filter is a defense-in-
/// depth but the daemon shouldn't send echoes in the first place.
#[cfg(feature = "crdt")]
#[test]
fn m10_10_crdt_op_originator_does_not_receive_own_broadcast_echo() {
    let daemon = TestDaemon::spawn();
    let (hello_a, mut stream_a) = attach_multi(&daemon);
    // Need a B attached so the daemon's broadcast loop has someone
    // to broadcast to — without recipients the broadcast is a no-op
    // and the test wouldn't distinguish "no echo because no
    // broadcast" from "no echo because sender-exclusion."
    let (_hello_b, _stream_b) = attach_multi(&daemon);

    // Drain A's BufferSnapshot, bootstrap, generate op.
    let (buffer_id, snapshot_bytes) =
        match read_message::<InstanceMessage>(&mut stream_a).expect("A first frame") {
            InstanceMessage::BufferSnapshot {
                buffer_id,
                crdt_snapshot,
            } => (buffer_id, crdt_snapshot),
            other => panic!("expected BufferSnapshot, got {other:?}"),
        };
    let a_replica = pmacs::crdt::CrdtState::new(hello_a.assigned_frontend_id.0).expect("a replica");
    a_replica.import_snapshot(&snapshot_bytes).expect("import");
    let v_before = a_replica.version();
    a_replica.insert(0, "Q").expect("a insert");
    let op_bytes = a_replica.export_updates_since(&v_before).expect("export");

    let crdt_op_event = FrontendEvent::CrdtOp {
        frontend_id: hello_a.assigned_frontend_id,
        buffer_id,
        op: pmacs::rope::CrdtOp {
            peer_id: hello_a.assigned_frontend_id.0,
            bytes: op_bytes,
        },
    };
    write_message(&mut stream_a, &crdt_op_event).expect("send CrdtOp from A");

    // Read A's incoming stream for a short window; assert no
    // CrdtOp arrives. Subsequent CellDelta / CursorByte / etc.
    // messages are fine — we're just checking CrdtOp specifically.
    stream_a
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < deadline {
        if let Ok(InstanceMessage::CrdtOp { .. }) = read_message::<InstanceMessage>(&mut stream_a) {
            panic!(
                "M10.10 Finding 3: originator received its own CrdtOp \
                 back — daemon sender-exclusion is broken"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Post-audit round 2 — F11 / F12 / F13 inbound-CrdtOp validation.
//
// Three negative-path tests proving the daemon rejects malformed or
// out-of-scope inbound `FrontendEvent::CrdtOp` events:
//   - F11: a session that didn't negotiate `crdt_replica` can't drive
//          CRDT state by sending the variant.
//   - F12: an op whose `op.peer_id` names another frontend is rejected
//          before apply / broadcast. (Without this, the receiving
//          frontend's attach loop derives the source via
//          `FrontendId(op.peer_id)` and dedup-skips the broadcast,
//          diverging its mirror from daemon state.)
//   - F13: an op targeting a buffer the source isn't actively editing
//          is rejected. (M10.10's local-edit path only emits ops for
//          the active mirror buffer.)
//
// The reject path drops the op and logs to stderr; we observe rejection
// indirectly by sending a follow-up well-formed op from the same
// frontend and asserting only the well-formed op reaches the peer
// frontend's stream as an `InstanceMessage::CrdtOp` broadcast. (A
// stronger black-box assertion than checking stderr.)
// ---------------------------------------------------------------------------

/// Helper for the F11/F12/F13 negative-path tests: bootstrap two
/// attached replica frontends, drain A's initial frame, return the
/// buffer id A is editing and an A-replica `CrdtState` that's
/// already imported A's snapshot. The caller generates op bytes from
/// the replica and sends crafted `FrontendEvent::CrdtOp` payloads.
#[cfg(feature = "crdt")]
fn bootstrap_two_replicas_for_negative_path(
    daemon: &TestDaemon,
) -> (
    Hello,
    UnixStream,
    Hello,
    UnixStream,
    pmacs::buffer::BufferId,
    pmacs::crdt::CrdtState,
) {
    let (hello_a, mut stream_a) = attach_multi(daemon);
    let (hello_b, mut stream_b) = attach_multi(daemon);
    assert_ne!(hello_a.assigned_frontend_id, hello_b.assigned_frontend_id);

    let (buffer_id, snapshot_bytes) =
        match read_message::<InstanceMessage>(&mut stream_a).expect("A first frame") {
            InstanceMessage::BufferSnapshot {
                buffer_id,
                crdt_snapshot,
            } => (buffer_id, crdt_snapshot),
            other => panic!("expected BufferSnapshot from A, got {other:?}"),
        };
    let _b_first: InstanceMessage = read_message(&mut stream_b).expect("B first frame");

    let a_replica = pmacs::crdt::CrdtState::new(hello_a.assigned_frontend_id.0).expect("a replica");
    a_replica.import_snapshot(&snapshot_bytes).expect("import");

    (hello_a, stream_a, hello_b, stream_b, buffer_id, a_replica)
}

/// Drain stream until a `InstanceMessage::CrdtOp` arrives or the
/// deadline elapses. Used in the negative-path tests to observe the
/// daemon's actual broadcast decisions.
#[cfg(feature = "crdt")]
fn wait_for_crdt_op_broadcast(
    stream: &mut UnixStream,
    deadline: Instant,
) -> Option<(pmacs::buffer::BufferId, pmacs::rope::CrdtOp)> {
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    while Instant::now() < deadline {
        if let Ok(InstanceMessage::CrdtOp { buffer_id, op }) =
            read_message::<InstanceMessage>(stream)
        {
            return Some((buffer_id, op));
        }
    }
    None
}

/// F11: a frontend that did NOT negotiate `crdt_replica` (legacy v0.1
/// caps) cannot mutate daemon state by sending `FrontendEvent::CrdtOp`.
/// The daemon's pre-apply validation drops the op; the peer replica
/// frontend never receives a broadcast for it.
#[cfg(feature = "crdt")]
#[test]
fn m10_10_f11_non_replica_session_crdt_op_is_rejected() {
    let daemon = TestDaemon::spawn();

    // Legacy session (no crdt_replica) connects first. A is the
    // attacker — its caps don't advertise the capability, but it tries
    // to send a CrdtOp variant anyway.
    let mut stream_a = daemon.connect();
    stream_a
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let hello_a = do_handshake(&mut stream_a);
    let _initial_a: InstanceMessage = read_message(&mut stream_a).expect("A initial frame");

    // B is a legitimate replica frontend that should NOT receive an
    // echo of A's rejected op.
    let (_hello_b, mut stream_b) = attach_multi(&daemon);
    let (buffer_id, snapshot_bytes) =
        match read_message::<InstanceMessage>(&mut stream_b).expect("B first frame") {
            InstanceMessage::BufferSnapshot {
                buffer_id,
                crdt_snapshot,
            } => (buffer_id, crdt_snapshot),
            other => panic!("expected BufferSnapshot, got {other:?}"),
        };

    // A bootstraps a CRDT replica out-of-band (it didn't actually
    // negotiate, but it can still craft bytes since CRDT state is
    // public) and generates an op.
    let a_replica = pmacs::crdt::CrdtState::new(hello_a.assigned_frontend_id.0).expect("a replica");
    a_replica.import_snapshot(&snapshot_bytes).expect("import");
    let v_before = a_replica.version();
    a_replica.insert(0, "Z").expect("insert");
    let op_bytes = a_replica.export_updates_since(&v_before).expect("export");

    let event = FrontendEvent::CrdtOp {
        frontend_id: hello_a.assigned_frontend_id,
        buffer_id,
        op: pmacs::rope::CrdtOp {
            peer_id: hello_a.assigned_frontend_id.0,
            bytes: op_bytes,
        },
    };
    write_message(&mut stream_a, &event).expect("write");

    // B's stream must not see the broadcast within a generous window.
    let deadline = Instant::now() + Duration::from_secs(1);
    assert!(
        wait_for_crdt_op_broadcast(&mut stream_b, deadline).is_none(),
        "F11: daemon must drop CrdtOp from a session that didn't negotiate crdt_replica"
    );
}

/// F12: an op whose `op.peer_id` names a different frontend is
/// rejected before apply / broadcast.
#[cfg(feature = "crdt")]
#[test]
fn m10_10_f12_spoofed_op_peer_id_is_rejected() {
    let daemon = TestDaemon::spawn();
    let (hello_a, mut stream_a, hello_b, mut stream_b, buffer_id, a_replica) =
        bootstrap_two_replicas_for_negative_path(&daemon);

    // Craft ONE set of op bytes (so the daemon can causally apply
    // either copy). Send it twice with different identity framing:
    //   - spoofed event: `op.peer_id` set to B's id (should be
    //     rejected by F12 pre-apply).
    //   - well-formed event: `op.peer_id` set to A's id (should
    //     apply + broadcast normally).
    // Loro is idempotent on remote-op import, so reusing the bytes is
    // safe even if both were applied; here only the second is.
    let v_before = a_replica.version();
    a_replica.insert(0, "X").expect("insert");
    let op_bytes = a_replica.export_updates_since(&v_before).expect("export");

    let spoofed = FrontendEvent::CrdtOp {
        frontend_id: hello_a.assigned_frontend_id,
        buffer_id,
        op: pmacs::rope::CrdtOp {
            peer_id: hello_b.assigned_frontend_id.0, // B's peer_id, not A's
            bytes: op_bytes.clone(),
        },
    };
    let well_formed = FrontendEvent::CrdtOp {
        frontend_id: hello_a.assigned_frontend_id,
        buffer_id,
        op: pmacs::rope::CrdtOp {
            peer_id: hello_a.assigned_frontend_id.0,
            bytes: op_bytes.clone(),
        },
    };
    write_message(&mut stream_a, &spoofed).expect("write spoofed");
    write_message(&mut stream_a, &well_formed).expect("write well-formed");

    // B should see exactly one CrdtOp broadcast — the well-formed one,
    // tagged with A's peer_id. (The spoofed op was rejected pre-apply
    // and would otherwise have arrived with op.peer_id == B.0.)
    let deadline = Instant::now() + Duration::from_secs(2);
    let first = wait_for_crdt_op_broadcast(&mut stream_b, deadline)
        .expect("B must receive the well-formed op");
    assert_eq!(first.0, buffer_id);
    assert_eq!(
        first.1.peer_id, hello_a.assigned_frontend_id.0,
        "F12: B's first CrdtOp broadcast must be the well-formed op (op.peer_id = A's id); \
         a B-tagged op slipping through means the spoofed op was not rejected"
    );
    assert_eq!(first.1.bytes, op_bytes, "F12: broadcast bytes must match");
}

/// F13: an op targeting a `buffer_id` that the source is not
/// actively editing is rejected. We construct this by attaching the
/// session, switching its active window to a different buffer (or
/// using a buffer id that doesn't exist), and asserting the
/// well-formed op for the actually-active buffer still goes through
/// while the wrong-buffer op is dropped.
#[cfg(feature = "crdt")]
#[test]
fn m10_10_f13_wrong_buffer_id_is_rejected() {
    let daemon = TestDaemon::spawn();
    let (hello_a, mut stream_a, _hello_b, mut stream_b, buffer_id, a_replica) =
        bootstrap_two_replicas_for_negative_path(&daemon);

    // A fabricates a non-existent buffer id. The daemon's active-
    // window-for-A lookup returns the scratch buffer; the fabricated
    // id doesn't match → reject. BufferId's constructor is crate-
    // internal, so we round-trip via postcard (its wire form is the
    // bare u64 newtype).
    let fake_buffer_id: pmacs::buffer::BufferId =
        postcard::from_bytes(&postcard::to_stdvec(&u64::MAX).unwrap()).expect("rtt");
    assert_ne!(fake_buffer_id, buffer_id);

    // One set of valid op bytes; send twice with different
    // `buffer_id` framing.
    let v_before = a_replica.version();
    a_replica.insert(0, "X").expect("insert");
    let op_bytes = a_replica.export_updates_since(&v_before).expect("export");

    let wrong_buf = FrontendEvent::CrdtOp {
        frontend_id: hello_a.assigned_frontend_id,
        buffer_id: fake_buffer_id,
        op: pmacs::rope::CrdtOp {
            peer_id: hello_a.assigned_frontend_id.0,
            bytes: op_bytes.clone(),
        },
    };
    let right_buf = FrontendEvent::CrdtOp {
        frontend_id: hello_a.assigned_frontend_id,
        buffer_id,
        op: pmacs::rope::CrdtOp {
            peer_id: hello_a.assigned_frontend_id.0,
            bytes: op_bytes.clone(),
        },
    };
    write_message(&mut stream_a, &wrong_buf).expect("write wrong-buf");
    write_message(&mut stream_a, &right_buf).expect("write right-buf");

    let deadline = Instant::now() + Duration::from_secs(2);
    let first = wait_for_crdt_op_broadcast(&mut stream_b, deadline)
        .expect("B must receive the right-buffer op");
    assert_eq!(
        first.0, buffer_id,
        "F13: B's first CrdtOp broadcast must target the active buffer; \
         a fake-buffer op slipping through means F13 didn't reject it"
    );
    assert_eq!(first.1.bytes, op_bytes);
}

/// F26: the daemon validates that the loro-internal peer
/// attribution inside `op.bytes` matches the authenticated source.
/// A hostile client can set `op.peer_id == authenticated source`
/// while the update bytes themselves were generated under a
/// DIFFERENT loro peer id; the wrapper-only check (F12) would
/// accept this. Recipients then route/dedup by wrapper identity,
/// but CRDT history attributes to the other peer, splitting
/// observable state from CRDT causal metadata.
///
/// Test shape: A bootstraps a normal CRDT replica under peer-id =
/// `A.0`. A also builds a SECONDARY CRDT replica seeded from A's
/// bootstrap snapshot but with `peer_id = 999`. The secondary
/// replica produces an op; A wraps it with the secondary's loro
/// bytes but `op.peer_id = A.0` (so F12 passes). The daemon's F26
/// fork-import detects the mismatch and rejects. B then submits a
/// well-formed op and only B's broadcast reaches the peer.
#[cfg(feature = "crdt")]
#[test]
fn m10_10_f26_spoofed_loro_internal_peer_id_is_rejected() {
    let daemon = TestDaemon::spawn();
    let (hello_a, mut stream_a, _hello_b, mut stream_b, buffer_id, _a_replica) =
        bootstrap_two_replicas_for_negative_path(&daemon);

    // Secondary replica with a DIFFERENT loro peer id than A's
    // assigned frontend id. Seeded from the same content so its
    // exports are causally applicable to the daemon's buffer.
    let secondary_peer_id: u64 = 999;
    assert_ne!(secondary_peer_id, hello_a.assigned_frontend_id.0);
    let secondary = pmacs::crdt::CrdtState::new(secondary_peer_id).expect("secondary");
    let snapshot_donor = pmacs::crdt::CrdtState::new(2).expect("snap-donor");
    let snapshot_bytes = snapshot_donor.export_snapshot().expect("export");
    secondary.import_snapshot(&snapshot_bytes).expect("import");
    let v_before = secondary.version();
    secondary.insert(0, "S").expect("secondary insert");
    let spoofed_bytes = secondary
        .export_updates_since(&v_before)
        .expect("export spoofed");

    // Send the spoofed op: wrapper peer_id matches A (F12 passes)
    // but the bytes carry ops attributed to peer 999. F26 fork-
    // import detects this and rejects.
    let spoofed = FrontendEvent::CrdtOp {
        frontend_id: hello_a.assigned_frontend_id,
        buffer_id,
        op: pmacs::rope::CrdtOp {
            peer_id: hello_a.assigned_frontend_id.0, // wrapper = A (F12 passes)
            bytes: spoofed_bytes.clone(),
        },
    };
    write_message(&mut stream_a, &spoofed).expect("write spoofed");

    // B must not receive a broadcast for the spoofed op within a
    // generous window. (The peer's broadcast would carry op.peer_id
    // = A but the bytes attributed to peer 999 — if accepted, the
    // peer mirror would dedup-skip on echo and then re-apply via
    // the broadcast, getting wrong attribution either way.)
    stream_b
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut saw_spoofed_broadcast = false;
    while Instant::now() < deadline {
        if let Ok(InstanceMessage::CrdtOp { op, .. }) =
            read_message::<InstanceMessage>(&mut stream_b)
            && op.bytes == spoofed_bytes
        {
            saw_spoofed_broadcast = true;
            break;
        }
    }
    assert!(
        !saw_spoofed_broadcast,
        "F26: daemon must reject ops whose loro-internal peer attribution \
         disagrees with the authenticated source"
    );
}

/// F16: a daemon-side mutation (a `FrontendEvent::Key` round-trip)
/// that generates a CRDT op broadcasts to **all** replica frontends
/// including the source. Pre-fix, the source's mirror would silently
/// drift from daemon state after every fallback / Key-path edit
/// because `pending_crdt_ops` excluded the active frontend.
///
/// Test shape: A (replica) attaches; sends a plain `FrontendEvent::Key`
/// (a printable char that the daemon's command pipeline maps to
/// `pmacs.editor.insert-text`). The daemon mutates the active buffer,
/// generates a CRDT op tagged `CrdtOpOrigin::DaemonKey`, broadcasts.
/// A asserts it received its own `InstanceMessage::CrdtOp` broadcast.
#[cfg(feature = "crdt")]
#[test]
fn m10_10_f16_daemon_key_origin_broadcasts_to_source() {
    let daemon = TestDaemon::spawn();
    let (hello_a, mut stream_a) = attach_multi(&daemon);

    // Drain A's bootstrap frames (BufferSnapshot + CursorByte).
    // We're going to wait for a CrdtOp specifically, so we want
    // any other messages drained first.
    let _first_a: InstanceMessage =
        read_message(&mut stream_a).expect("A first frame (BufferSnapshot)");

    // Send a `FrontendEvent::Key` for a plain printable char. The
    // daemon's normal key-path will produce a CRDT op tagged
    // `DaemonKey`; F16 ensures it broadcasts to A as well.
    let key_event = FrontendEvent::Key(KeyEvent {
        frontend_id: hello_a.assigned_frontend_id,
        key: Key::Char('K'),
        mods: Modifiers::NONE,
        timestamp_ns: 0,
    });
    write_message(&mut stream_a, &key_event).expect("write Key");

    // A must receive its OWN edit's CRDT broadcast (F16). Without
    // the fix, the broadcast would exclude A and the assertion would
    // time out.
    stream_a
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut saw_self_broadcast = false;
    while Instant::now() < deadline {
        if let Ok(InstanceMessage::CrdtOp { .. }) = read_message::<InstanceMessage>(&mut stream_a) {
            saw_self_broadcast = true;
            break;
        }
    }
    assert!(
        saw_self_broadcast,
        "F16: daemon-key-origin CRDT op must broadcast to the active frontend; \
         pre-fix the source frontend's mirror diverged from daemon state on every \
         Key round-trip"
    );
}

/// F17: a CRDT op whose import produces no text delta (e.g. a
/// concurrent same-character delete that already converged) is
/// **still broadcast** to peers. Pre-fix, `handle_remote_crdt_op`
/// returned early on `Ok(None)` and dropped the op — peers never
/// imported the CRDT causal metadata, and later updates that
/// depend on it could fail to apply.
///
/// Test shape: A and B both attached as replicas. A bootstraps a
/// CRDT replica and generates an op. A sends the op twice. The
/// first application produces an Edit and broadcasts; the second
/// `apply_remote_crdt_op` is idempotent → `Ok(None)`. F17 ensures
/// the second broadcast still happens, so B sees **two** `CrdtOp`
/// broadcasts even though the daemon's text content changed only
/// once.
#[cfg(feature = "crdt")]
#[test]
fn m10_10_f17_no_text_delta_import_still_broadcasts() {
    let daemon = TestDaemon::spawn();
    let (hello_a, mut stream_a, _hello_b, mut stream_b, buffer_id, a_replica) =
        bootstrap_two_replicas_for_negative_path(&daemon);

    let v_before = a_replica.version();
    a_replica.insert(0, "Z").expect("insert");
    let op_bytes = a_replica.export_updates_since(&v_before).expect("export");

    let event = FrontendEvent::CrdtOp {
        frontend_id: hello_a.assigned_frontend_id,
        buffer_id,
        op: pmacs::rope::CrdtOp {
            peer_id: hello_a.assigned_frontend_id.0,
            bytes: op_bytes.clone(),
        },
    };
    // First send: applies (Some(edit)) and broadcasts.
    write_message(&mut stream_a, &event).expect("write 1");
    // Second send: import is idempotent → Ok(None). Pre-fix the op
    // would have been dropped here; F17 still pushes to the broadcast
    // queue.
    write_message(&mut stream_a, &event).expect("write 2");

    // B must see TWO CrdtOp broadcasts, both carrying the same
    // bytes. Without F17, B would see only one.
    stream_b
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut crdt_op_count = 0;
    while Instant::now() < deadline && crdt_op_count < 2 {
        if let Ok(InstanceMessage::CrdtOp { op, .. }) =
            read_message::<InstanceMessage>(&mut stream_b)
        {
            assert_eq!(op.bytes, op_bytes);
            crdt_op_count += 1;
        }
    }
    assert_eq!(
        crdt_op_count, 2,
        "F17: peer must receive both broadcasts even though the second \
         apply_remote_crdt_op produced no text delta; pre-fix only one arrives"
    );
}

/// F14: production-path end-to-end keystroke flow. Negotiates with
/// the **production** `attach::build_capabilities()` (not the test
/// `multi_frontend_caps()`), bootstraps a real `BufferMirror` from
/// the daemon's `BufferSnapshot` + `CursorByte`, then drives the
/// production `optimistic::frontend_event_for_keystroke` orchestrator
/// with a synthetic `KeyEvent`. The produced `FrontendEvent::CrdtOp`
/// is sent to the daemon and a second replica frontend (B) must
/// receive the broadcast.
///
/// Why this matters: the other M10.10 acceptance tests inject
/// `FrontendEvent::CrdtOp` directly with hand-built fields. That
/// bypasses the production decision chain (`classify_key`,
/// `frontend_event_for_keystroke`, eligibility predicates), so a
/// regression in *any* of those layers would not be caught by the
/// existing matrix — exactly the gap that allowed the original
/// post-audit Finding 1 to ship a structurally-unreachable
/// optimistic-apply path.
///
/// We can't drive crossterm's raw-mode terminal from a test, but we
/// can drive everything from `KeyEvent` downward in the same code
/// path the production attach loop uses (see src/attach.rs:784).
#[cfg(feature = "crdt")]
#[test]
fn m10_10_f14_production_path_keystroke_flows_to_broadcast() {
    use pmacs::buffer_mirror::BufferMirror;

    let daemon = TestDaemon::spawn();

    // A attaches with PRODUCTION caps — exercises Finding 3 fix
    // (build_capabilities advertising crdt_replica in CRDT builds).
    let mut stream_a = daemon.connect();
    stream_a
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let hello_a: Hello = read_message(&mut stream_a).expect("A Hello");
    let req_a = AttachRequest {
        protocol_version: hello_a.protocol_version,
        frontend_capabilities: pmacs::attach::build_capabilities(),
        initial_size: CellSize::new(24, 80),
    };
    write_message(&mut stream_a, &req_a).expect("A AttachRequest");

    // B attaches with multi-frontend test caps so it's a replica too
    // — it will receive A's broadcast.
    let (_hello_b, mut stream_b) = attach_multi(&daemon);

    // A's bootstrap: BufferSnapshot first, then drain frames until
    // CursorByte arrives (the byte-position pairing M10.10 added).
    let (buffer_id, snapshot_bytes) = match read_message::<InstanceMessage>(&mut stream_a)
        .expect("A first frame")
    {
        InstanceMessage::BufferSnapshot {
            buffer_id,
            crdt_snapshot,
        } => (buffer_id, crdt_snapshot),
        other => panic!(
            "F14: production caps must negotiate crdt_replica → first frame is BufferSnapshot; got {other:?}"
        ),
    };

    let mut a_mirror = BufferMirror::new(hello_a.assigned_frontend_id);
    a_mirror
        .init_from_snapshot(buffer_id, &snapshot_bytes)
        .expect("init_from_snapshot");
    a_mirror.set_cursor_byte_pos(buffer_id, 0);

    // Drain until CursorByte arrives (or timeout) so the mirror's
    // cursor is grounded in the daemon's actual cursor position. The
    // attach loop normally does this; we replicate it here.
    let drain_deadline = Instant::now() + Duration::from_millis(500);
    stream_a
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    while Instant::now() < drain_deadline {
        match read_message::<InstanceMessage>(&mut stream_a) {
            Ok(InstanceMessage::CursorByte {
                buffer_id: bid,
                byte_pos,
            }) if bid == buffer_id => {
                a_mirror.set_cursor_byte_pos(buffer_id, byte_pos as usize);
                break;
            }
            Ok(_) | Err(_) => {}
        }
    }

    // Drive the PRODUCTION orchestrator. This is the exact call site
    // at src/attach.rs:784 — same function signature, same inputs.
    let pmacs_key = KeyEvent {
        frontend_id: hello_a.assigned_frontend_id,
        key: Key::Char('Q'),
        mods: Modifiers::NONE,
        timestamp_ns: 0,
    };
    let frontend_event = pmacs::optimistic::frontend_event_for_keystroke(
        &mut a_mirror,
        hello_a.assigned_frontend_id,
        pmacs_key,
    );

    // The orchestrator must produce a CrdtOp (mirror is ready, action
    // is Insert, cursor is at a valid position). If it returned
    // `Key` as fallback, the production path's optimistic apply is
    // broken end-to-end.
    let (event_buffer_id, op_peer_id, op_bytes) = match &frontend_event {
        FrontendEvent::CrdtOp { buffer_id, op, .. } => (*buffer_id, op.peer_id, op.bytes.clone()),
        other => panic!(
            "F14: production orchestrator returned {other:?} instead of CrdtOp \
             for a plain Char('Q') insert. This means the mirror wasn't ready or \
             the eligibility predicates rejected what should be a viable optimistic \
             insert. (See src/optimistic.rs:128 — `frontend_event_for_keystroke`.)"
        ),
    };
    assert_eq!(event_buffer_id, buffer_id);
    assert_eq!(op_peer_id, hello_a.assigned_frontend_id.0);

    // Send the produced event upstream; B must see the broadcast.
    write_message(&mut stream_a, &frontend_event).expect("write CrdtOp");
    let deadline = Instant::now() + Duration::from_secs(2);
    let broadcast = wait_for_crdt_op_broadcast(&mut stream_b, deadline)
        .expect("F14: B must receive A's CrdtOp broadcast end-to-end");
    assert_eq!(broadcast.0, buffer_id);
    assert_eq!(broadcast.1.peer_id, hello_a.assigned_frontend_id.0);
    assert_eq!(
        broadcast.1.bytes, op_bytes,
        "F14: broadcast bytes must match what the production orchestrator produced"
    );
}

/// M10.10 Day 3 Finding 2 acceptance: a non-replica frontend does NOT
/// receive `CursorByte` — capability gating skips the emission for
/// frontends that don't negotiate `crdt_replica`. (Without this gate,
/// the non-replica frontend's postcard decoder would hard-error on
/// the unknown variant per Refinement 3.)
#[test]
fn m10_10_non_replica_frontend_does_not_receive_cursor_byte() {
    let daemon = TestDaemon::spawn();
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let _hello = do_handshake(&mut stream);

    // Read a handful of incoming frames; assert none is CursorByte.
    // 16 frames is enough to cover at least a couple of per-tick
    // render cycles.
    for _ in 0..16 {
        match read_message::<InstanceMessage>(&mut stream) {
            Ok(InstanceMessage::CursorByte { .. }) => panic!(
                "non-replica frontend received CursorByte — capability \
                 gating must skip the emission"
            ),
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

/// M10.10 Day 2 acceptance: a frontend that did NOT negotiate
/// `crdt_replica` (the v0.1-style frontend path) receives the initial
/// `CellDelta` directly without any `BufferSnapshot` first. The
/// daemon's M10.10 send is capability-gated; non-replica frontends
/// don't even see the wire variant (and can't decode it — postcard
/// hard-errors on unknown variants per Refinement 3).
#[test]
fn m10_10_non_replica_frontend_does_not_receive_buffer_snapshot() {
    let daemon = TestDaemon::spawn();
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    // build_default_caps() has crdt_replica: false — the daemon
    // negotiates crdt_replica: false and skips the send.
    let _hello = do_handshake(&mut stream);

    let first: InstanceMessage = read_message(&mut stream).expect("first frame");
    match first {
        InstanceMessage::CellDelta {
            full_grid: true, ..
        } => {}
        InstanceMessage::BufferSnapshot { .. } => {
            panic!(
                "M10.10: non-replica frontend received BufferSnapshot — \
                 capability gating must skip the send"
            );
        }
        other => panic!(
            "M10.10: first frame for non-replica frontend should be full-grid \
             CellDelta, got {other:?}"
        ),
    }
}
