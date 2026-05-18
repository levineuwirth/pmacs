// m5_7_acceptance.rs --- Acceptance suite for M5.7 (SSH transport).

//! End-to-end acceptance tests for T M5.7 (`--daemon-attach` byte
//! bridge, daemon auto-start helper, `pmacs --attach <target>` CLI
//! shorthand, and SSH transport activation via [`PMACS_TEST_SSH_BIN`]).
//!
//! Each test spawns the real `pmacs` binary as a subprocess and
//! drives it via the published protocol crate. Tempdir-scoped sockets
//! keep tests isolated under parallel `cargo test`.
//!
//! # Scenarios
//!
//! 1. **CLI argv shape** — `pmacs --attach mac-studio` runs the SSH
//!    binary named by [`PMACS_TEST_SSH_BIN`] with `[<host>, "pmacs",
//!    "--daemon-attach"]` plus the optional `-l USER` and `--socket
//!    NAME` shaping. Tested as
//!    [`cli_attach_invokes_test_ssh_with_expected_argv`].
//! 2. **Bridge byte round-trip** — `pmacs --daemon-attach` correctly
//!    forwards bytes between stdin/stdout and an existing daemon's
//!    socket. Tested as
//!    [`daemon_attach_bridges_hello_from_existing_daemon`].
//! 3. **Auto-start** — `pmacs --daemon-attach` against a missing
//!    socket spawns a daemon (via `current_exe`) and proceeds with
//!    the byte-pump once it binds. Tested as
//!    [`daemon_attach_auto_starts_missing_daemon`].
//! 4. **init.lua → `RunAttachSsh`** — covered exhaustively in
//!    `tests/m5_6_acceptance.rs::init_lua_ssh_target_dispatches_to_run_attach_ssh`
//!    and the kwargs companion `init_lua_kwargs_form_ssh_dispatches_to_run_attach_ssh`.
//!    Not duplicated here.
//! 5. **SSH spawn failure** — `pmacs --attach mac-studio` with
//!    `PMACS_TEST_SSH_BIN` pointing at a non-existent path surfaces a
//!    workaround-pointing diagnostic (PATH hint). Tested as
//!    [`cli_attach_with_missing_ssh_binary_surfaces_spawn_failure`].
//! 6. **Clean detach cascade** — when a frontend closes its end of
//!    the bridge's stdin (the F12-detach reaches the bridge boundary
//!    as stdin EOF on the SSH side), the bridge exits 0 cleanly.
//!    This is the key half of the F12 → SSH child exit 0 contract:
//!    the bridge IS the SSH child in the local-machine test harness.
//!    Tested as [`daemon_attach_exits_clean_on_stdin_eof`].
//! 7. **Mid-session daemon crash** — when the daemon is SIGKILL'd
//!    mid-session, the bridge sees socket EOF, the two `std::io::copy`
//!    loops return, and the bridge exits 0. The frontend side
//!    (real `pmacs --attach`) sees the SSH child exit 0 and returns
//!    `Ok(())` — v0.1 does not distinguish a clean Goodbye from a
//!    daemon crash. Tested as
//!    [`daemon_attach_handles_daemon_crash_as_clean_eof`].
//!
//! # Why the `pmacs --attach` side is exercised only at the spawn /
//! pre-handshake boundary
//!
//! `attach::run_attach_ssh` constructs a [`pmacs::frontend::Frontend`]
//! after the protocol handshake, which takes over the calling
//! process's terminal (raw mode, alternate screen). A `cargo test`
//! environment generally does not have a tty available, and even
//! when it does, taking over the test runner's terminal would corrupt
//! the test output. Tests that drive `pmacs --attach` therefore
//! arrange for the handshake or spawn to fail *before*
//! `Frontend::new()` is reached, which is sufficient for asserting
//! on argv shape (scenario 1) and spawn diagnostics (scenario 5).
//!
//! End-to-end pump behavior (input forwarding, cell rendering, F12
//! detach event production) is exercised at the `run_attach_pair`
//! seam by the lib unit tests in `attach::tests`, which use a
//! transport-agnostic test frontend.

use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use tempfile::TempDir;

use pmacs::attach::PMACS_TEST_SSH_BIN;
use pmacs::protocol::{Hello, PROTOCOL_VERSION};
use pmacs::transport::read_message;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A foreground daemon spawned in the test, with cleanup on Drop.
///
/// Mirrors the harness in `tests/m5_5_acceptance.rs`. Duplicated
/// (rather than extracted to a shared module) because integration
/// tests cannot easily share helpers without going through the lib
/// surface, and keeping the suite self-contained makes failures
/// easier to read.
struct TestDaemon {
    _tempdir: TempDir,
    socket_path: PathBuf,
    process: Child,
}

impl TestDaemon {
    fn spawn() -> Self {
        let tempdir = TempDir::new().expect("tempdir");
        // Daemon requires 0700-or-stricter parent for the socket.
        fs::set_permissions(tempdir.path(), fs::Permissions::from_mode(0o700))
            .expect("chmod tempdir 0700");
        let socket_path = tempdir.path().join("pmacs.sock");
        let process = spawn_pmacs_daemon(&socket_path);
        wait_for_socket(&socket_path, Duration::from_secs(10)).expect("daemon socket appeared");
        Self {
            _tempdir: tempdir,
            socket_path,
            process,
        }
    }

    fn pid(&self) -> u32 {
        self.process.id()
    }

    fn is_alive(&mut self) -> bool {
        self.process.try_wait().ok().flatten().is_none()
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

fn spawn_pmacs_daemon(socket_path: &Path) -> Child {
    // Isolate user config: HOME and XDG_CONFIG_HOME both point at the
    // (currently empty) socket parent directory, so the daemon won't
    // try to read the developer's real `init.lua`.
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

/// Probe by attempting to connect — `exists()` would let a stale
/// socket file fool us into thinking the daemon is up before bind.
fn wait_for_socket(socket: &Path, deadline: Duration) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if let Ok(mut stream) = UnixStream::connect(socket) {
            // Drain the daemon's Hello so it doesn't log a "send
            // Hello failed" warning when our probe drops.
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

/// Wait up to `deadline` for `predicate` to return true. Returns
/// `Ok(())` on success, `Err(elapsed)` on timeout.
fn wait_until<F: FnMut() -> bool>(mut predicate: F, deadline: Duration) -> Result<(), Duration> {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if predicate() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err(start.elapsed())
}

/// Wait up to `deadline` for `child` to exit. Returns the exit
/// status, or `Err` on timeout.
fn wait_for_exit(child: &mut Child, deadline: Duration) -> Result<std::process::ExitStatus, ()> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(_) => return Err(()),
        }
        if start.elapsed() >= deadline {
            return Err(());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Read the advisory pid from a daemon's lockfile (`<socket>.lock`).
/// Returns `None` if the lockfile doesn't exist or doesn't contain a
/// parseable pid.
fn lockfile_pid(socket: &Path) -> Option<u32> {
    let mut s = socket.as_os_str().to_os_string();
    s.push(".lock");
    let lockfile = PathBuf::from(s);
    let content = fs::read_to_string(&lockfile).ok()?;
    content.trim().parse().ok()
}

/// Best-effort kill + reap of a daemon orphaned to init/PID 1 by an
/// auto-start path. Polls until the process is gone or `deadline`
/// elapses; on timeout, falls through with the daemon still running
/// (the OS will eventually reap it once the test exits).
fn sigterm_and_wait(pid: u32, deadline: Duration) {
    #[allow(clippy::cast_possible_wrap)]
    let nix_pid = Pid::from_raw(pid as i32);
    let _ = kill(nix_pid, Signal::SIGTERM);
    let _ = wait_until(
        || kill(nix_pid, None).is_err(), // ESRCH once the process is gone
        deadline,
    );
}

// ---------------------------------------------------------------------------
// Scenario 1: CLI argv shape via PMACS_TEST_SSH_BIN
// ---------------------------------------------------------------------------

/// Stage a fake SSH script that records its argv to a sibling file
/// then exits 0 with no output. The pmacs `--attach` driver will see
/// EOF on `read_message::<Hello>` and surface a transport error; we
/// don't care about the exit shape here, only that the script ran
/// with the argv we expect.
fn write_recording_fake_ssh(dir: &Path) -> PathBuf {
    let script = dir.join("fake-ssh.sh");
    let argv_log = dir.join("argv.log");
    let body = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\nexit 0\n",
        shell_quote(&argv_log),
    );
    fs::write(&script, body).expect("write fake-ssh.sh");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod fake-ssh.sh +x");
    script
}

/// Quote a path for embedding inside a `/bin/sh` script. The path
/// comes from a tempdir we just created, so it's well-formed; this
/// is belt-and-braces against tempdir bases that contain shell
/// metacharacters.
fn shell_quote(p: &Path) -> String {
    let s = p.to_str().expect("tempdir path is UTF-8");
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

#[test]
fn cli_attach_invokes_test_ssh_with_expected_argv() {
    let tmp = TempDir::new().expect("tempdir");
    let fake_ssh = write_recording_fake_ssh(tmp.path());
    let argv_log = tmp.path().join("argv.log");

    // Empty isolated config. The `--attach <host>` path itself
    // doesn't load init.lua (that's editor::run's territory) but we
    // isolate anyway so a developer's stray config can never affect
    // the test.
    let isolated_home = tmp.path();

    let output = Command::new(env!("CARGO_BIN_EXE_pmacs"))
        .args(["--attach", "mac-studio"])
        .env(PMACS_TEST_SSH_BIN, &fake_ssh)
        .env("HOME", isolated_home)
        .env("XDG_CONFIG_HOME", isolated_home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn pmacs --attach");

    // pmacs exits non-zero because the fake ssh produced no Hello
    // (read_message returns EOF, handshake errors out before
    // Frontend::new is reached). The exact stderr shape is checked
    // in scenario 5; here we only care that the fake ssh ran.
    assert!(
        !output.status.success(),
        "expected pmacs to exit non-zero on EOF handshake; stderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let argv = fs::read_to_string(&argv_log).unwrap_or_else(|e| {
        panic!(
            "fake-ssh argv.log missing or unreadable ({e}); pmacs stderr: {}",
            String::from_utf8_lossy(&output.stderr),
        )
    });
    let lines: Vec<&str> = argv.lines().collect();
    // build_ssh_command for `ssh:mac-studio` (no user, no instance)
    // defaults to the stderr protocol channel and advertises the
    // remote fd through env.
    assert_eq!(
        lines,
        vec![
            "-T",
            "mac-studio",
            "env",
            "PMACS_ATTACH_PROTOCOL_FD=2",
            "pmacs",
            "--daemon-attach",
        ],
        "argv shape: {lines:?}"
    );
}

#[test]
fn cli_attach_with_user_and_instance_name_passes_through_dash_l_and_dash_dash_socket() {
    let tmp = TempDir::new().expect("tempdir");
    let fake_ssh = write_recording_fake_ssh(tmp.path());
    let argv_log = tmp.path().join("argv.log");

    let isolated_home = tmp.path();

    let _ = Command::new(env!("CARGO_BIN_EXE_pmacs"))
        .args(["--attach", "ssh:alice@workstation/research"])
        .env(PMACS_TEST_SSH_BIN, &fake_ssh)
        .env("HOME", isolated_home)
        .env("XDG_CONFIG_HOME", isolated_home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn pmacs --attach");

    let argv = fs::read_to_string(&argv_log).expect("argv.log readable");
    let lines: Vec<&str> = argv.lines().collect();
    assert_eq!(
        lines,
        vec![
            "-T",
            "-l",
            "alice",
            "workstation",
            "env",
            "PMACS_ATTACH_PROTOCOL_FD=2",
            "pmacs",
            "--daemon-attach",
            "--socket",
            "research",
        ],
        "argv shape: {lines:?}"
    );
}

// ---------------------------------------------------------------------------
// Scenario 2: bridge byte round-trip against an existing daemon
// ---------------------------------------------------------------------------

/// Spawn `pmacs --daemon-attach --socket PATH` with all three stdio
/// streams piped, returning the child plus its stdin/stdout halves.
fn spawn_bridge(socket: &Path, isolated_home: &Path) -> (Child, ChildStdin, ChildStdout) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pmacs"))
        .args(["--daemon-attach", "--socket"])
        .arg(socket)
        .env("HOME", isolated_home)
        .env("XDG_CONFIG_HOME", isolated_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pmacs --daemon-attach");
    let stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    (child, stdin, stdout)
}

#[test]
fn daemon_attach_bridges_hello_from_existing_daemon() {
    let daemon = TestDaemon::spawn();
    let isolated_home = daemon.socket_path.parent().expect("socket has parent");

    let (mut bridge, bridge_stdin, mut bridge_stdout) =
        spawn_bridge(&daemon.socket_path, isolated_home);

    // The daemon sends Hello unsolicited on accept, so the bridge's
    // socket→stdout copy must have surfaced those bytes by now. Read
    // and decode to prove the bridge is shuttling protocol traffic
    // verbatim. (No AttachRequest sent — the daemon will hold the
    // attach slot until the bridge stdin closes below.)
    let hello: Hello = read_message(&mut bridge_stdout).expect("read Hello via bridge");
    assert_eq!(hello.protocol_version, PROTOCOL_VERSION);

    // Tear down: drop bridge stdin → bridge's stdin→socket copy sees
    // EOF, shuts down the socket write half, the daemon notices and
    // closes its side, the socket→stdout copy returns, the bridge
    // exits.
    drop(bridge_stdin);
    drop(bridge_stdout);
    let status = wait_for_exit(&mut bridge, Duration::from_secs(5))
        .expect("bridge exits within 5s of stdin EOF");
    assert!(
        status.success(),
        "bridge should exit 0 on clean tear-down, got {status:?}",
    );
}

// ---------------------------------------------------------------------------
// Scenario 3: --daemon-attach auto-starts a missing daemon
// ---------------------------------------------------------------------------

#[test]
fn daemon_attach_auto_starts_missing_daemon() {
    // No daemon running. The bridge's `ensure_daemon_running` should
    // spawn one (via `current_exe`, which under cargo test is the
    // pmacs binary), poll until the socket binds, and proceed.
    let tmp = TempDir::new().expect("tempdir");
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).expect("chmod tempdir 0700");
    let socket_path = tmp.path().join("pmacs.sock");

    // Sanity: socket really doesn't exist yet.
    assert!(!socket_path.exists(), "test precondition: socket absent");

    let (mut bridge, bridge_stdin, mut bridge_stdout) = spawn_bridge(&socket_path, tmp.path());

    // Wait for the auto-started daemon to be reachable through the
    // bridge: a successful Hello read proves the spawned daemon
    // bound the socket and the bridge connected.
    let hello: Hello =
        read_message(&mut bridge_stdout).expect("read Hello via auto-started daemon");
    assert_eq!(hello.protocol_version, PROTOCOL_VERSION);

    // The lockfile must exist now: `acquire_lock` writes it on
    // daemon startup. (Existence of the lockfile is what proves
    // *we* didn't start it — we only spawned the bridge.)
    let pid =
        lockfile_pid(&socket_path).expect("auto-started daemon should have written its lockfile");

    // Tear down the bridge.
    drop(bridge_stdin);
    drop(bridge_stdout);
    let bridge_status =
        wait_for_exit(&mut bridge, Duration::from_secs(5)).expect("bridge exits within 5s");
    assert!(bridge_status.success(), "bridge exit: {bridge_status:?}");

    // Clean up the orphan daemon. ensure_daemon_running spawns the
    // daemon detached from our parent — it survives the bridge
    // exiting (re-parented to init), which is the contract. The
    // test must SIGTERM it explicitly so we don't leak processes
    // across test runs.
    sigterm_and_wait(pid, Duration::from_secs(5));
}

// ---------------------------------------------------------------------------
// Scenario 5: SSH spawn failure surfaces a workaround-pointing error
// ---------------------------------------------------------------------------

#[test]
fn cli_attach_with_missing_ssh_binary_surfaces_spawn_failure() {
    let tmp = TempDir::new().expect("tempdir");
    let isolated_home = tmp.path();

    // Path that definitively does not exist. `Command::spawn` will
    // fail with ENOENT, mapping to AttachError::SshSpawnFailed in
    // run_attach_ssh.
    let missing_ssh = tmp.path().join("not-a-real-ssh-binary");

    let output = Command::new(env!("CARGO_BIN_EXE_pmacs"))
        .args(["--attach", "mac-studio"])
        .env(PMACS_TEST_SSH_BIN, &missing_ssh)
        .env("HOME", isolated_home)
        .env("XDG_CONFIG_HOME", isolated_home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn pmacs --attach");

    assert!(!output.status.success(), "pmacs should exit non-zero");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("could not spawn"),
        "stderr should name the spawn failure: {stderr}"
    );
    assert!(
        stderr.contains("PATH"),
        "stderr should hint at the PATH workaround: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Scenario 6: clean detach cascade — bridge exits 0 on stdin EOF
// ---------------------------------------------------------------------------

#[test]
fn daemon_attach_exits_clean_on_stdin_eof() {
    // The user-facing F12 detach reaches the bridge as stdin EOF
    // (the local frontend dropped its writer half of the SSH child's
    // stdin pipe). The bridge's two `std::io::copy` loops must
    // unwind cleanly and the process must exit 0 — that's the
    // SSH-child-exits-0 contract for `pmacs --attach <host>`.
    let mut daemon = TestDaemon::spawn();
    let pid_before = daemon.pid();
    let isolated_home = daemon.socket_path.parent().unwrap();

    let (mut bridge, bridge_stdin, mut bridge_stdout) =
        spawn_bridge(&daemon.socket_path, isolated_home);

    let _hello: Hello = read_message(&mut bridge_stdout).expect("read Hello via bridge");

    drop(bridge_stdin);
    drop(bridge_stdout);

    let status = wait_for_exit(&mut bridge, Duration::from_secs(5))
        .expect("bridge exits within 5s of stdin EOF");
    assert!(
        status.success(),
        "bridge must exit 0 on clean stdin EOF, got {status:?}",
    );

    // Daemon survives the frontend disconnecting — the whole point
    // of the daemon model.
    assert_eq!(daemon.pid(), pid_before, "daemon must outlive the bridge");
    assert!(daemon.is_alive(), "daemon should still be alive");
}

// ---------------------------------------------------------------------------
// Scenario 7: mid-session daemon crash → bridge sees socket EOF, exits 0
// ---------------------------------------------------------------------------

#[test]
fn daemon_attach_handles_daemon_crash_as_clean_eof() {
    // When the daemon dies mid-session (here simulated with SIGKILL),
    // the bridge's socket reader sees EOF, the socket→stdout copy
    // returns, kick_handle.shutdown wakes the stdin thread, and
    // the bridge exits 0.
    //
    // From the `pmacs --attach` side, this is indistinguishable from
    // a clean detach: the SSH child exits 0, classify_ssh_exit
    // returns Ok. v0.1 accepts this — it's a known limitation that
    // a daemon crash and a clean detach produce the same client
    // exit. Future work may add a heartbeat or terminating frame.
    let mut daemon = TestDaemon::spawn();
    let isolated_home = daemon.socket_path.parent().unwrap();

    let (mut bridge, _bridge_stdin, mut bridge_stdout) =
        spawn_bridge(&daemon.socket_path, isolated_home);

    // Confirm the bridge is fully connected before the crash.
    let _hello: Hello = read_message(&mut bridge_stdout).expect("read Hello via bridge");

    // Hard-kill the daemon. SIGKILL bypasses the daemon's graceful
    // shutdown, so no Goodbye frame is sent — the bridge sees a
    // bare socket close.
    #[allow(clippy::cast_possible_wrap)]
    let daemon_pid = Pid::from_raw(daemon.pid() as i32);
    kill(daemon_pid, Signal::SIGKILL).expect("SIGKILL daemon");
    let _ = daemon.process.wait();

    // Bridge must notice the socket EOF and exit promptly. We don't
    // close stdin first — the kick_handle shutdown inside run_bridge
    // is what unsticks the stdin thread.
    let status = wait_for_exit(&mut bridge, Duration::from_secs(5))
        .expect("bridge exits within 5s of daemon crash");
    assert!(
        status.success(),
        "bridge must exit 0 on socket EOF, got {status:?}",
    );

    // Sanity: nothing should have written to bridge stderr in this
    // path. A panic or unwind would surface here.
    let mut bridge_stderr = String::new();
    if let Some(mut s) = bridge.stderr.take() {
        let _ = s.read_to_string(&mut bridge_stderr);
    }
    assert!(
        bridge_stderr.trim().is_empty(),
        "bridge stderr should be empty on crash-EOF, got: {bridge_stderr}",
    );
}
