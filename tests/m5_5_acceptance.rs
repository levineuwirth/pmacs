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
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use tempfile::TempDir;

use pmacs::cell::CellSize;
use pmacs::protocol::{
    AttachRequest, FrontendCapabilities, FrontendEvent, GoodbyeReason, Hello, InstanceMessage, Key,
    KeyEvent, Modifiers, PROTOCOL_VERSION,
};
use pmacs::transport::{read_message, write_message};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A foreground daemon spawned in the test, with cleanup on Drop.
struct TestDaemon {
    /// Tempdir holding the socket and lockfile; auto-cleaned on Drop.
    _tempdir: TempDir,
    socket_path: PathBuf,
    process: Child,
}

impl TestDaemon {
    fn spawn() -> Self {
        let tempdir = TempDir::new().expect("tempdir");
        // tempfile::TempDir creates 0755-mode directories; the daemon
        // requires a 0700-or-stricter parent for the socket. Tighten
        // the tempdir before spawning.
        fs::set_permissions(tempdir.path(), fs::Permissions::from_mode(0o700))
            .expect("chmod tempdir 0700");
        let socket_path = tempdir.path().join("pmacs.sock");
        let process = spawn_daemon_process(&socket_path);
        wait_for_socket_or_exit(&socket_path, &process, Duration::from_secs(10))
            .expect("daemon socket appeared");
        Self {
            _tempdir: tempdir,
            socket_path,
            process,
        }
    }

    fn pid(&self) -> u32 {
        self.process.id()
    }

    fn connect(&self) -> UnixStream {
        UnixStream::connect(&self.socket_path).expect("connect")
    }

    fn is_alive(&mut self) -> bool {
        self.process.try_wait().ok().flatten().is_none()
    }

    fn lockfile_path(&self) -> PathBuf {
        let mut s = self.socket_path.as_os_str().to_os_string();
        s.push(".lock");
        PathBuf::from(s)
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

fn spawn_daemon_process(socket_path: &Path) -> Child {
    // Isolate user config so a test daemon doesn't read $HOME/.pmacs/init.lua.
    // Both HOME and XDG_CONFIG_HOME point at the parent of the socket
    // (which is empty at this point), so config::load_user_config is a
    // no-op.
    let isolated_home = socket_path.parent().expect("socket has parent");
    Command::new(env!("CARGO_BIN_EXE_pmacs"))
        .args(["--daemon", "--socket"])
        .arg(socket_path)
        .env("HOME", isolated_home)
        .env("XDG_CONFIG_HOME", isolated_home)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn pmacs --daemon")
}

fn wait_for_socket_or_exit(
    socket: &Path,
    _process: &Child,
    deadline: Duration,
) -> Result<(), String> {
    // We probe by attempting to *connect*, not by file-exists. A stale
    // socket file from a previous (crashed) daemon can satisfy
    // `exists` without anyone actually listening — only a successful
    // connect proves the new daemon has run through `bind` and
    // `listen`. ECONNREFUSED on a stale socket retries until the new
    // daemon takes over.
    let start = Instant::now();
    while start.elapsed() < deadline {
        if let Ok(mut stream) = UnixStream::connect(socket) {
            // Read the Hello so the daemon's `send_message` succeeds
            // and doesn't log a "send Hello failed" warning to its
            // stderr (which our test runner inherits). After reading
            // we drop without sending AttachRequest; the daemon
            // observes the disconnect and falls back to accept.
            stream
                .set_read_timeout(Some(Duration::from_millis(500)))
                .ok();
            let _ = read_message::<Hello>(&mut stream);
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err(format!(
        "daemon did not start listening on {} within {deadline:?}",
        socket.display()
    ))
}

fn build_default_caps() -> FrontendCapabilities {
    FrontendCapabilities {
        synchronized_output: true,
        unicode_smp: true,
        true_color: true,
        mouse: true,
        bracketed_paste: true,
        terminal_kind: Some("test".into()),
    }
}

/// Read the daemon's `Hello`, send our `AttachRequest`, return the Hello.
fn do_handshake(stream: &mut UnixStream) -> Hello {
    let hello: Hello = read_message(stream).expect("read Hello");
    assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
    let req = AttachRequest {
        protocol_version: PROTOCOL_VERSION,
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
    let socket_meta = fs::metadata(&daemon.socket_path).expect("stat socket");
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
    let parent_meta = fs::metadata(daemon.socket_path.parent().unwrap()).expect("stat parent");
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
        if let Ok(msg) = read_message::<InstanceMessage>(&mut stream) {
            if matches!(
                msg,
                InstanceMessage::CellDelta { .. } | InstanceMessage::Cursor(_)
            ) {
                got_response = true;
                break;
            }
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
    let isolated_home = daemon_a.socket_path.parent().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_pmacs"))
        .args(["--daemon", "--socket"])
        .arg(&daemon_a.socket_path)
        .env("HOME", isolated_home)
        .env("XDG_CONFIG_HOME", isolated_home)
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
    let socket_path = daemon.socket_path.clone();
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
    let status = daemon.process.wait().expect("wait");
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
    wait_for_socket_or_exit(&socket_path, &daemon1, Duration::from_secs(10))
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
    wait_for_socket_or_exit(&socket_path, &daemon2, Duration::from_secs(10))
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
    assert_eq!(hello.protocol_version, PROTOCOL_VERSION);

    // Send AttachRequest with wrong protocol version.
    let req = AttachRequest {
        protocol_version: 999,
        frontend_capabilities: build_default_caps(),
        initial_size: CellSize::new(24, 80),
    };
    write_message(&mut stream, &req).expect("write");

    // Expect Goodbye(VersionMismatch).
    match read_message::<InstanceMessage>(&mut stream) {
        Ok(InstanceMessage::Goodbye(GoodbyeReason::VersionMismatch { server, client })) => {
            assert_eq!(server, PROTOCOL_VERSION);
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
