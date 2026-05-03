//! M5.8 acceptance suite — mosh-style reconnect resilience.
//!
//! Two halves:
//!
//! ## Handshake-reconnect (no-PTY) tests
//!
//! Cover the loop's pre-Frontend path. Driven via `Stdio::null()`
//! subprocesses; `Frontend::new` never engages, so retries terminate
//! at `HANDSHAKE_RETRY_CAP`. Fast (single-digit ms with
//! `PMACS_TEST_BACKOFF_SCALE_MS=1`).
//!
//! 1. **Handshake retry cap.** A fake SSH that exits without writing
//!    a `Hello` produces a transient handshake failure on every
//!    attempt. With `PMACS_TEST_BACKOFF_SCALE_MS=1` the retries fire
//!    within milliseconds and the cap is observable via a counter
//!    file the fake SSH updates on each invocation.
//! 2. **Backoff timing scales with env var.** Same fake SSH, but
//!    `PMACS_TEST_BACKOFF_SCALE_MS=50` makes each sleep observable
//!    in wall-clock — verifies the env var actually feeds through
//!    to the loop, not just the unit-tested helper.
//! 3. **SSH stderr from each handshake attempt reaches the user.** A
//!    fake SSH that writes a marker to stderr on every invocation.
//!    The user's stderr ends up with one copy per retry, proving
//!    `spawn_stderr_tee`'s live-tee path still works under the new
//!    loop on the handshake-reconnect (Frontend-not-yet-up) path.
//!
//! ## Session-reconnect (PTY) tests
//!
//! Cover the loop's post-Frontend path. Driven via `portable_pty` so
//! pmacs sees a real TTY and `Frontend::new` succeeds, exercising the
//! session-reconnect (frontend-up) branch with verdict
//! `ReconnectVerdict::Reconnect` and the unbounded-retry semantics
//! that go with it. Slower (hundreds of ms with the same env var
//! scaled).
//!
//! 4. **SSH dies mid-pump → reconnect succeeds.** Counter-aware fake
//!    SSH that completes the handshake on every call but exits
//!    non-zero on calls 1–2 and exits 0 on call 3; pmacs reconnects
//!    and exits clean. The "daemon dies mid-pump" case is
//!    indistinguishable from this at pmacs's observation point (in
//!    both cases the SSH child exits non-zero with the daemon-side
//!    closure cascading through the bridge), so it's covered by the
//!    same test.
//! 5. **Ctrl-C during reconnect sleep → clean exit.** Fake SSH whose
//!    first call completes handshake then exits non-zero; pmacs
//!    enters `sleep_with_countdown`; the test injects `\x03` via the
//!    PTY master and asserts pmacs exits 0 (no error message).
//!
//! All tests use `PMACS_TEST_BACKOFF_SCALE_MS` to keep CI runtime
//! tight; without it, three handshake retries take ~1.5s minimum.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize};
use tempfile::TempDir;

use pmacs::attach::PMACS_TEST_SSH_BIN;
use pmacs::attach_reconnect::{HANDSHAKE_RETRY_CAP, PMACS_TEST_BACKOFF_SCALE_MS};
use pmacs::protocol::{
    FrontendId, Hello, InstanceCapabilities, InstanceIdentity, PROTOCOL_VERSION,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Single-quote a path for embedding in a `/bin/sh` script. The path
/// comes from a tempdir; this is belt-and-braces against tempdir
/// bases that contain shell metacharacters.
fn shell_quote(p: &Path) -> String {
    let s = p.to_str().expect("tempdir path is UTF-8");
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// Write a fake SSH that increments a counter file on each
/// invocation and exits 0 without writing a `Hello`. Returns the
/// `(script_path, counter_path)` pair.
fn write_counting_fake_ssh(dir: &Path) -> (PathBuf, PathBuf) {
    let script = dir.join("fake-ssh.sh");
    let counter = dir.join("invocation-count");
    fs::write(&counter, "0").expect("seed counter");
    let body = format!(
        "#!/bin/sh\n\
         n=$(cat {counter})\n\
         n=$((n+1))\n\
         echo $n > {counter}\n\
         exit 0\n",
        counter = shell_quote(&counter),
    );
    fs::write(&script, body).expect("write fake-ssh.sh");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod fake-ssh.sh +x");
    (script, counter)
}

/// Write a fake SSH that emits a fixed line to its stderr, then
/// exits with code 1 (no `Hello` on stdout). Used to verify the
/// stderr tail propagates into the give-up error.
fn write_noisy_fake_ssh(dir: &Path, stderr_line: &str) -> PathBuf {
    let script = dir.join("fake-ssh-noisy.sh");
    let body = format!(
        "#!/bin/sh\n\
         printf '%s\\n' {line} >&2\n\
         exit 1\n",
        line = shell_quote(Path::new(stderr_line)),
    );
    fs::write(&script, body).expect("write fake-ssh-noisy.sh");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
        .expect("chmod fake-ssh-noisy.sh +x");
    script
}

fn read_invocation_count(counter: &Path) -> u32 {
    fs::read_to_string(counter)
        .expect("counter readable")
        .trim()
        .parse()
        .expect("counter parses as u32")
}

// ---------------------------------------------------------------------------
// Test 1: Handshake retry cap fires after exactly HANDSHAKE_RETRY_CAP attempts
// ---------------------------------------------------------------------------

/// Each failed handshake should produce one SSH invocation. With
/// the retry cap pinned at [`HANDSHAKE_RETRY_CAP`] (3 in v0.1), the
/// counter file should record exactly that many invocations. Off-
/// by-one errors in the cap-check (e.g. `>` vs `>=`) would change
/// this number; off-by-one errors in the counter increment (banner
/// `attempt N of M` formatting) would not.
#[test]
fn handshake_retry_cap_fires_after_three_failed_handshakes() {
    let tmp = TempDir::new().expect("tempdir");
    let (fake_ssh, counter) = write_counting_fake_ssh(tmp.path());
    let isolated_home = tmp.path();

    let output = Command::new(env!("CARGO_BIN_EXE_pmacs"))
        .args(["--attach", "host"])
        .env(PMACS_TEST_SSH_BIN, &fake_ssh)
        .env(PMACS_TEST_BACKOFF_SCALE_MS, "1")
        .env("HOME", isolated_home)
        .env("XDG_CONFIG_HOME", isolated_home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn pmacs --attach");

    assert!(
        !output.status.success(),
        "pmacs should exit non-zero after exhausting handshake retries; \
         stderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let count = read_invocation_count(&counter);
    assert_eq!(
        count, HANDSHAKE_RETRY_CAP,
        "expected exactly {HANDSHAKE_RETRY_CAP} fake-SSH invocations \
         (one per handshake attempt before the cap fires); got {count}"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Backoff timing scales with PMACS_TEST_BACKOFF_SCALE_MS
// ---------------------------------------------------------------------------

/// `PMACS_TEST_BACKOFF_SCALE_MS` should change the loop's wait
/// behavior in wall-clock — proving the env var feeds through the
/// production code path, not just the unit-tested helper. With
/// scale=50ms the schedule yields 50 + 100 = 150ms of sleep across
/// 3 attempts; with scale=1ms it's ~3ms. A 10x ratio between the
/// two runs is well outside CI noise.
#[test]
fn backoff_scaling_observable_in_wall_clock_runtime() {
    let tmp = TempDir::new().expect("tempdir");
    let (fake_ssh, _) = write_counting_fake_ssh(tmp.path());
    let isolated_home = tmp.path();

    let run = |scale_ms: &str| -> Duration {
        let start = Instant::now();
        let output = Command::new(env!("CARGO_BIN_EXE_pmacs"))
            .args(["--attach", "host"])
            .env(PMACS_TEST_SSH_BIN, &fake_ssh)
            .env(PMACS_TEST_BACKOFF_SCALE_MS, scale_ms)
            .env("HOME", isolated_home)
            .env("XDG_CONFIG_HOME", isolated_home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .expect("spawn pmacs --attach");
        assert!(!output.status.success());
        start.elapsed()
    };

    let fast = run("1");
    let slow = run("50");

    // Fast must complete within a generous CI ceiling — three SSH
    // spawns + handshake EOFs + ~3ms of total sleep should fit
    // easily in 2s on any reasonable runner.
    assert!(
        fast < Duration::from_secs(2),
        "scale=1 should be near-instant, got {fast:?}"
    );

    // Slow should have at least 100ms of measurable sleep
    // (50 + 100 = 150ms minimum, allowing some slop for missed
    // wakeups). A 100ms floor catches the "env var ignored"
    // regression without being flaky on slow runners.
    assert!(
        slow >= Duration::from_millis(100),
        "scale=50 should sleep at least 100ms, got {slow:?}"
    );

    // Slow must also be meaningfully longer than fast — a 3x ratio
    // is the conservative floor (theoretical 50x but CI overhead
    // dominates short runs). This is the "env var actually feeds
    // through" guard.
    assert!(
        slow.as_millis() >= fast.as_millis().saturating_mul(3),
        "scale=50 ({slow:?}) should be ≥ 3x scale=1 ({fast:?})"
    );
}

// ---------------------------------------------------------------------------
// Test 3: SSH stderr from each failed handshake reaches the user's terminal
// ---------------------------------------------------------------------------

/// During handshake-reconnect attempts the Frontend has not yet
/// engaged raw mode, so `spawn_stderr_tee` should forward the SSH
/// child's stderr live to our stderr — once per attempt. Three
/// retries → three copies of the SSH stderr line in our stderr.
///
/// This is the user-visible diagnostic path for "handshake fails
/// repeatedly": the user sees SSH's actual error message in their
/// scrollback, not just pmacs's terminal Transport-EOF wrapper.
///
/// (The give-up wrapper itself is the [`pmacs::attach::AttachError`]
/// from the LAST attempt — typically a Transport error because
/// `read_message` returns EOF before `child.wait` would observe a
/// non-zero exit. Including the captured tail in that message is a
/// possible UX improvement but distinct from the contract this test
/// guards.)
#[test]
fn ssh_stderr_from_handshake_attempts_reaches_user() {
    let tmp = TempDir::new().expect("tempdir");
    let stderr_line = "FATAL_TAIL_MARKER_2026";
    let fake_ssh = write_noisy_fake_ssh(tmp.path(), stderr_line);
    let isolated_home = tmp.path();

    let output = Command::new(env!("CARGO_BIN_EXE_pmacs"))
        .args(["--attach", "host"])
        .env(PMACS_TEST_SSH_BIN, &fake_ssh)
        .env(PMACS_TEST_BACKOFF_SCALE_MS, "1")
        .env("HOME", isolated_home)
        .env("XDG_CONFIG_HOME", isolated_home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn pmacs --attach");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);

    // The marker should appear once per failed handshake attempt
    // (live tee, not buffered behind the give-up message).
    let marker_count = stderr.matches(stderr_line).count();
    let expected_count = HANDSHAKE_RETRY_CAP as usize;
    assert_eq!(
        marker_count, expected_count,
        "expected {expected_count} occurrences of {stderr_line:?} in \
         stderr (one per handshake attempt); got {marker_count} in: {stderr}"
    );
    // Pmacs's final give-up message follows the marker copies.
    assert!(
        stderr.contains("pmacs:"),
        "expected pmacs's give-up message after the SSH stderr; \
         got: {stderr}"
    );
}

// ===========================================================================
// PTY-based session-reconnect tests
// ===========================================================================
//
// These tests spawn pmacs in a real PTY so `Frontend::new` succeeds
// and the session-reconnect (frontend-up) branch of the loop can be
// exercised end-to-end. The fake SSH is a shell script that writes a
// pre-encoded `Hello` frame from a binary file, sleeps long enough
// for pmacs to write its `AttachRequest` and call `Frontend::new`,
// then exits with a counter-controlled exit code.

/// Build the wire-format bytes for a `Hello` frame: the 4-byte
/// big-endian length prefix followed by the postcard-encoded
/// payload. Used by session-reconnect tests so the fake SSH can
/// `cat hello.bin` to deliver a valid Hello to pmacs without the
/// shell having to do binary protocol work.
fn encode_hello_frame() -> Vec<u8> {
    let hello = Hello {
        protocol_version: PROTOCOL_VERSION,
        assigned_frontend_id: FrontendId::LOCAL,
        instance_identity: InstanceIdentity {
            pmacs_version: "0.0.0-test".to_string(),
            build_hash: None,
            instance_name: Some("pty-test".to_string()),
            uptime_secs: 0,
            working_directory: "/tmp".to_string(),
        },
        instance_capabilities: InstanceCapabilities::default(),
    };
    let mut buf = Vec::new();
    pmacs::transport::write_message(&mut buf, &hello).expect("encode Hello");
    buf
}

/// Write a counter-aware fake SSH that delivers a real `Hello` frame
/// and exits with a per-call code. The script:
///
/// 1. Reads/increments the counter file.
/// 2. Writes the precomputed Hello frame to its stdout.
/// 3. Sleeps `keepalive_ms` so pmacs has time to write `AttachRequest`
///    and run `Frontend::new` before the pipe closes.
/// 4. Exits with the code at index `count - 1` in `exit_codes`. After
///    the list is exhausted, exits with the last code (so e.g.
///    `[1, 1, 0]` → fail twice, then succeed forever).
fn write_session_reconnect_fake_ssh(
    dir: &Path,
    counter: &Path,
    hello_path: &Path,
    keepalive_ms: u32,
    exit_codes: &[i32],
) -> PathBuf {
    let script = dir.join("fake-ssh-session.sh");
    // Build a shell `case` body dispatching on counter value.
    let mut case_body = String::new();
    for (i, code) in exit_codes.iter().enumerate() {
        let n = i + 1;
        let _ = writeln!(case_body, "{n}) exit {code} ;;");
    }
    let last = exit_codes.last().copied().unwrap_or(0);
    let _ = writeln!(case_body, "*) exit {last} ;;");

    let seconds = f64::from(keepalive_ms) / 1000.0;
    let body = format!(
        "#!/bin/sh\n\
         n=$(cat {counter})\n\
         n=$((n+1))\n\
         echo $n > {counter}\n\
         cat {hello}\n\
         # Keep stdin/stdout open long enough for pmacs to finish\n\
         # writing AttachRequest and bring up the Frontend.\n\
         sleep {seconds:.3}\n\
         case $n in\n\
         {case}\
         esac\n",
        counter = shell_quote(counter),
        hello = shell_quote(hello_path),
        case = case_body,
    );
    fs::write(&script, body).expect("write fake-ssh-session.sh");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
        .expect("chmod fake-ssh-session.sh +x");
    script
}

/// Pmacs spawned inside a real PTY. Holds the master so the slave
/// stays alive; `child` is reapable via `try_wait`. The writer
/// allows the test to inject keystrokes (e.g. `\x03` for Ctrl-C);
/// the reader is kept around so the slave's output buffer doesn't
/// fill and block pmacs's writes.
struct PmacsPty {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    _reader_thread: thread::JoinHandle<()>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
}

impl PmacsPty {
    /// Inject bytes into pmacs's stdin via the PTY master.
    fn write_input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    /// Poll-wait for pmacs to exit, up to `timeout`. Returns the
    /// exit status on success, `None` on timeout (and leaves the
    /// child running for the caller to clean up).
    fn wait_for_exit(&mut self, timeout: Duration) -> Option<portable_pty::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return Some(status),
                Ok(None) => {}
                Err(_) => return None,
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for PmacsPty {
    fn drop(&mut self) {
        // Best-effort cleanup if the test panicked / timed out.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Spawn pmacs inside a fresh PTY pair, returning a handle that
/// owns the master + child + reader-thread. The reader thread
/// drains the master so pmacs's writes never block on a full
/// terminal buffer.
fn spawn_pmacs_in_pty(args: &[&str], envs: &[(&str, &Path)], rows: u16, cols: u16) -> PmacsPty {
    let pty_system = portable_pty::native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_pmacs"));
    for arg in args {
        cmd.arg(arg);
    }
    for (k, v) in envs {
        let mut value = OsString::new();
        value.push(v);
        cmd.env(k, value);
    }

    let child = pair.slave.spawn_command(cmd).expect("spawn pmacs");
    // Drop the slave on our side so EOF on the master is detectable
    // when the child exits (pmacs's stdio is the only thing keeping
    // the slave alive after this).
    drop(pair.slave);

    let writer = pair.master.take_writer().expect("take_writer");
    let mut reader = pair.master.try_clone_reader().expect("try_clone_reader");
    // Drain reader to /dev/null so pmacs's writes never block on a
    // backed-up terminal output buffer. We don't need to inspect the
    // bytes; the tests assert on exit status and side effects, not
    // on screen content.
    let reader_thread = thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    });

    PmacsPty {
        child,
        writer,
        _reader_thread: reader_thread,
        _master: pair.master,
    }
}

/// Common test setup for the PTY tests: a tempdir with the
/// pre-encoded Hello frame, a zeroed counter file, and the host
/// args that point at the fake SSH and the test scale env var.
struct PtyFixture {
    _tmp: TempDir,
    counter: PathBuf,
    fake_ssh: PathBuf,
    isolated_home: PathBuf,
}

fn setup_pty_fixture(keepalive_ms: u32, exit_codes: &[i32]) -> PtyFixture {
    let tmp = TempDir::new().expect("tempdir");
    let counter = tmp.path().join("counter");
    fs::write(&counter, "0").expect("seed counter");

    let hello_path = tmp.path().join("hello.bin");
    fs::write(&hello_path, encode_hello_frame()).expect("write hello.bin");

    let fake_ssh = write_session_reconnect_fake_ssh(
        tmp.path(),
        &counter,
        &hello_path,
        keepalive_ms,
        exit_codes,
    );

    let isolated_home = tmp.path().to_path_buf();
    PtyFixture {
        _tmp: tmp,
        counter,
        fake_ssh,
        isolated_home,
    }
}

// ---------------------------------------------------------------------------
// Test 4: SSH dies mid-pump → reconnect succeeds → clean exit
// ---------------------------------------------------------------------------

/// The session-reconnect path of the loop, exercised end-to-end:
/// the fake SSH delivers a valid `Hello` on every call, lets pmacs
/// reach the pump, then exits non-zero on calls 1–2 and exits 0 on
/// call 3. With `Frontend::new` succeeding (real PTY), each pump
/// returns `Ok(())` (reader EOF on SSH stdout), the child's exit
/// code controls classification: non-zero → `SshChildExited` →
/// `Reconnect` (transient, frontend up); zero → `ExitClean` → loop
/// breaks `Ok(())`.
///
/// Asserts pmacs exits 0 and the counter file shows exactly 3
/// invocations. The "daemon dies mid-pump" case is covered by this
/// test as well: from pmacs's view, both manifest as the SSH child
/// exiting non-zero (a daemon death cascades through the bridge to
/// SSH stdout EOF + bridge exit + SSH exit).
#[test]
fn ssh_dies_mid_pump_then_reconnect_succeeds() {
    // 100ms keepalive lets pmacs finish handshake + Frontend::new
    // before the SSH pipe closes. PMACS_TEST_BACKOFF_SCALE_MS=1
    // makes the inter-attempt sleep effectively instant.
    let fx = setup_pty_fixture(100, &[1, 1, 0]);

    let mut pty = spawn_pmacs_in_pty(
        &["--attach", "host"],
        &[
            (PMACS_TEST_SSH_BIN, fx.fake_ssh.as_path()),
            (PMACS_TEST_BACKOFF_SCALE_MS, Path::new("1")),
            ("HOME", fx.isolated_home.as_path()),
            ("XDG_CONFIG_HOME", fx.isolated_home.as_path()),
        ],
        24,
        80,
    );

    let status = pty
        .wait_for_exit(Duration::from_secs(10))
        .expect("pmacs should exit within 10s");
    assert!(
        status.success(),
        "pmacs should exit clean after reconnect succeeds; status: {status:?}"
    );

    let count: u32 = fs::read_to_string(&fx.counter)
        .expect("counter readable")
        .trim()
        .parse()
        .expect("counter parses");
    assert_eq!(
        count, 3,
        "expected 3 fake-SSH invocations (fail, fail, succeed); got {count}"
    );
}

// ---------------------------------------------------------------------------
// Test 5: Ctrl-C during reconnect sleep → clean exit
// ---------------------------------------------------------------------------

/// The cancellation path of `sleep_with_countdown`: after the first
/// session attempt fails, pmacs enters the countdown sleep with
/// `Frontend::new` already up, and a Ctrl-C arriving via the PTY
/// (delivered as `KeyEvent { Char('c'), CONTROL }` because raw mode
/// disables `ISIG`) flips the loop to `Ok(())` exit.
///
/// `PMACS_TEST_BACKOFF_SCALE_MS=1000` makes the first sleep 1s wide
/// so the test's Ctrl-C injection comfortably lands inside the
/// sleep window: pmacs reaches sleep around t≈120ms, sleeps until
/// t≈1120ms, and the test injects at t≈400ms.
#[test]
fn ctrl_c_during_reconnect_sleep_yields_clean_exit() {
    // Fake SSH: every call completes handshake, sleeps 100ms, exits 1.
    // Pmacs would keep retrying forever without the Ctrl-C cancel.
    let fx = setup_pty_fixture(100, &[1]);

    let mut pty = spawn_pmacs_in_pty(
        &["--attach", "host"],
        &[
            (PMACS_TEST_SSH_BIN, fx.fake_ssh.as_path()),
            (PMACS_TEST_BACKOFF_SCALE_MS, Path::new("1000")),
            ("HOME", fx.isolated_home.as_path()),
            ("XDG_CONFIG_HOME", fx.isolated_home.as_path()),
        ],
        24,
        80,
    );

    // Timeline:
    //   t≈0:    pmacs spawns fake SSH
    //   t≈100:  fake SSH exits (keepalive elapsed)
    //   t≈120:  pmacs classifies SshChildExited as Reconnect (slot
    //           is Some), enters sleep_with_countdown(1000ms)
    //   t≈400:  test injects \x03; sleep_with_countdown's
    //           250ms-tick poll picks it up within ≤250ms
    //   t≈650:  loop breaks Ok(()), Frontend drops, pmacs exits 0
    //
    // 400ms places the injection well inside the [120, 1120]ms
    // sleep window without sitting at the edge.
    thread::sleep(Duration::from_millis(400));

    pty.write_input(b"\x03").expect("inject Ctrl-C");

    let status = pty
        .wait_for_exit(Duration::from_secs(5))
        .expect("pmacs should exit within 5s of Ctrl-C");
    assert!(
        status.success(),
        "Ctrl-C during reconnect sleep should produce a clean exit; status: {status:?}"
    );
}
