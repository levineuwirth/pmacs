//! `pmacs --daemon` subprocess fixture shared across integration tests.
//!
//! `TestDaemon` spawns a real foreground daemon in a tempdir-scoped
//! socket, waits until the socket is reachable, and cleans up on
//! `Drop`. `attach()` / `connect()` produce a `UnixStream` to the
//! daemon's socket; tests handle the protocol handshake themselves.
//!
//! First consumer: M5.5 acceptance suite
//! (`tests/m5_5_acceptance.rs`). Second consumer: M10.11
//! doubled-PTY two-laptop tests (`tests/m10_11_acceptance.rs`).
//!
//! Note: `tests/m5_8_acceptance.rs` builds a different daemon shape
//! (fake-SSH driver, not a real daemon process) and does not consume
//! this module.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;

#[cfg(feature = "crdt")]
use pmacs::cell::CellSize;
#[cfg(feature = "crdt")]
use pmacs::protocol::{AttachRequest, PROTOCOL_VERSION};
use pmacs::protocol::{FrontendCapabilities, Hello};
use pmacs::transport::read_message;
#[cfg(feature = "crdt")]
use pmacs::transport::write_message;

/// A foreground daemon spawned in the test, with cleanup on Drop.
pub struct TestDaemon {
    /// Tempdir holding the socket and lockfile; auto-cleaned on Drop.
    _tempdir: TempDir,
    socket_path: PathBuf,
    process: Child,
}

impl TestDaemon {
    pub fn spawn() -> Self {
        Self::spawn_with_env(&[])
    }

    /// T M10.8 Day 4 — spawn with extra env-var overrides for
    /// instance-capability tests.
    pub fn spawn_with_env(env_vars: &[(&str, &str)]) -> Self {
        Self::spawn_with_env_and_config(env_vars, None)
    }

    /// Spawn with a user `init.lua` pre-written into the daemon's
    /// isolated config home (the tempdir doubles as `HOME` /
    /// `XDG_CONFIG_HOME`, so the chunk lands at
    /// `<tempdir>/pmacs/init.lua` and loads through the real
    /// `load_user_config` path). First consumer: the auto-pairing
    /// CRDT suite, which extends `pmacs.pair.sets` from config to
    /// exercise the optimistic (non-built-in) pair-char route.
    #[allow(dead_code)] // consumed per-suite; not every test crate uses it
    pub fn spawn_with_config(init_lua: &str) -> Self {
        Self::spawn_with_env_and_config(&[], Some(init_lua))
    }

    fn spawn_with_env_and_config(env_vars: &[(&str, &str)], init_lua: Option<&str>) -> Self {
        let tempdir = TempDir::new().expect("tempdir");
        // tempfile::TempDir creates 0755-mode directories; the daemon
        // requires a 0700-or-stricter parent for the socket. Tighten
        // the tempdir before spawning.
        fs::set_permissions(tempdir.path(), fs::Permissions::from_mode(0o700))
            .expect("chmod tempdir 0700");
        if let Some(chunk) = init_lua {
            let config_dir = tempdir.path().join("pmacs");
            fs::create_dir_all(&config_dir).expect("create pmacs config dir");
            fs::write(config_dir.join("init.lua"), chunk).expect("write init.lua");
        }
        let socket_path = tempdir.path().join("pmacs.sock");
        let mut process = spawn_daemon_process_with_env(&socket_path, env_vars);
        wait_for_socket_or_exit(&socket_path, &mut process, Duration::from_secs(10))
            .expect("daemon socket appeared");
        Self {
            _tempdir: tempdir,
            socket_path,
            process,
        }
    }

    pub fn pid(&self) -> u32 {
        self.process.id()
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn connect(&self) -> UnixStream {
        UnixStream::connect(&self.socket_path).expect("connect")
    }

    pub fn is_alive(&mut self) -> bool {
        self.process.try_wait().ok().flatten().is_none()
    }

    pub fn lockfile_path(&self) -> PathBuf {
        let mut s = self.socket_path.as_os_str().to_os_string();
        s.push(".lock");
        PathBuf::from(s)
    }

    /// Block until the daemon's child process exits, returning the
    /// exit status. Used by the SIGTERM test in `m5_5_acceptance.rs`
    /// to confirm a clean shutdown after the signal was sent.
    pub fn wait_for_exit(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.process.wait()
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

pub fn spawn_daemon_process(socket_path: &Path) -> Child {
    spawn_daemon_process_with_env(socket_path, &[])
}

/// Spawn a daemon with extra environment-variable overrides.
///
/// T M10.8 Day 4 — used by tests that need to exercise non-default
/// instance capabilities (e.g., the M10.7 mismatch test, which
/// needs `PMACS_INSTANCE_MULTI_FRONTEND=0` so a frontend declaring
/// `multi_frontend: true` hits the capability-mismatch path).
/// Production daemons don't set these vars.
///
/// Stderr is redirected to a socket-adjacent log file so that
/// [`wait_for_socket_or_exit`] can surface daemon panics / startup
/// failures in test error messages instead of leaving operators
/// with an opaque 10s timeout. File-backed stderr avoids the
/// deadlock risk of an undrained pipe for long-running daemon tests.
pub fn spawn_daemon_process_with_env(socket_path: &Path, env_vars: &[(&str, &str)]) -> Child {
    let isolated_home = socket_path.parent().expect("socket has parent");
    let stderr_path = daemon_stderr_path(socket_path);
    let stderr = fs::File::create(&stderr_path).expect("create daemon stderr log");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_pmacs"));
    cmd.args(["--daemon", "--socket"])
        .arg(socket_path)
        .env("HOME", isolated_home)
        .env("XDG_CONFIG_HOME", isolated_home)
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr));
    for (key, value) in env_vars {
        cmd.env(key, value);
    }
    cmd.spawn().expect("spawn pmacs --daemon")
}

/// Poll until the daemon's socket is reachable, the daemon exits,
/// or `deadline` elapses.
///
/// **Diagnostic contract:** if the daemon exits before the socket
/// becomes reachable, or if the deadline expires, the returned
/// error includes the daemon's exit status (when known) and any
/// captured stderr. M10.11's PTY-doubled tests are operator-invoked
/// and a 10s "daemon did not start listening" with no further
/// information was found to be expensive to debug in practice — this
/// helper takes ownership of that diagnostic cost.
pub fn wait_for_socket_or_exit(
    socket: &Path,
    process: &mut Child,
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
        // Daemon may have exited early (panic on bind, missing
        // env, etc.). Surface its exit status + stderr immediately
        // instead of letting the connect-probe burn the full
        // deadline.
        if let Ok(Some(status)) = process.try_wait() {
            let stderr = read_daemon_stderr(socket);
            return Err(format!(
                "daemon exited with {status} before socket appeared; \
                 socket={}\n--- daemon stderr ---\n{stderr}",
                socket.display()
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
    // Timeout: kill, then capture whatever stderr is available so the
    // operator sees the daemon's last words rather than "no signal."
    let _ = process.kill();
    let _ = process.wait();
    let stderr = read_daemon_stderr(socket);
    Err(format!(
        "daemon did not start listening on {} within {deadline:?}\n\
         --- daemon stderr ---\n{stderr}",
        socket.display()
    ))
}

fn daemon_stderr_path(socket: &Path) -> PathBuf {
    let mut path = socket.as_os_str().to_os_string();
    path.push(".stderr.log");
    PathBuf::from(path)
}

/// Read the daemon's file-backed stderr, best-effort. Used only on
/// failure paths in [`wait_for_socket_or_exit`].
fn read_daemon_stderr(socket: &Path) -> String {
    let path = daemon_stderr_path(socket);
    match fs::read_to_string(&path) {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => String::from("<stderr empty>"),
        Err(e) => format!("<stderr unavailable at {}: {e}>", path.display()),
    }
}

// ---------------------------------------------------------------------------
// Frontend-attach helpers (shared across daemon-driven tests)
// ---------------------------------------------------------------------------

/// Default v0.1 capabilities — no multi-frontend, no CRDT replica.
/// Used by tests that exercise the legacy single-frontend path.
pub fn build_default_caps() -> FrontendCapabilities {
    FrontendCapabilities {
        synchronized_output: true,
        unicode_smp: true,
        true_color: true,
        mouse: true,
        bracketed_paste: true,
        terminal_kind: Some("test".into()),
        multi_frontend: false,
        crdt_replica: false,
        semantic_render: false,
    }
}

/// Caps for a v1.0 multi-frontend + CRDT-replica frontend. CRDT-only
/// because [`build_default_caps`]'s non-multi defaults are kept for
/// the v0.1 path; multi-frontend tests opt in explicitly.
#[cfg(feature = "crdt")]
pub fn multi_frontend_caps() -> FrontendCapabilities {
    FrontendCapabilities {
        multi_frontend: true,
        crdt_replica: true,
        ..build_default_caps()
    }
}

/// Connect, read the daemon's `Hello`, send an `AttachRequest` with
/// multi-frontend + CRDT-replica caps, and return the Hello plus the
/// connected stream. The caller is responsible for any subsequent
/// reads (initial `BufferSnapshot`, `CellDelta`, etc.).
///
/// First consumer: M5.5 acceptance suite's multi-frontend tests
/// (M10.8/M10.9/M10.10 sections). Second consumer: M10.11 synthesis
/// tests and the doubled-PTY observer.
#[cfg(feature = "crdt")]
pub fn attach_multi(daemon: &TestDaemon) -> (Hello, UnixStream) {
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let hello: Hello = read_message(&mut stream).expect("read Hello");
    let req = AttachRequest {
        protocol_version: PROTOCOL_VERSION,
        frontend_capabilities: multi_frontend_caps(),
        initial_size: CellSize::new(24, 80),
    };
    write_message(&mut stream, &req).expect("write AttachRequest");
    (hello, stream)
}
