// daemon_attach.rs --- `pmacs --daemon-attach` byte bridge (T M5.7b).

//! Far-side byte bridge between stdin/stdout and a local daemon
//! socket.
//!
//! `pmacs --daemon-attach` is the mode that runs on the far side of
//! an SSH connection (or any other transport that can spawn a
//! process and bridge its stdio). It connects to the local daemon's
//! Unix socket and forwards bytes verbatim:
//!
//! ```text
//!     stdin  ──>  socket
//!     socket ──>  stdout
//! ```
//!
//! # Transparent forwarding
//!
//! The bridge is **not** a protocol participant. It does not parse
//! [`crate::protocol::Hello`] frames or speak postcard. It is a pair
//! of `std::io::copy` loops. The Hello/AttachRequest handshake
//! happens between the *real* frontend (on the user's laptop) and
//! the *real* daemon (on the same machine as the bridge); the bridge
//! just shuttles the bytes those two parties exchange.
//!
//! This is the simplest possible thing that works, and it's correct:
//! the bridge has no knowledge of protocol versioning, no opinion on
//! framing, and adds no surface to the wire format. Adding a new
//! protocol message in v0.2 does not require touching this file.
//!
//! # Lifetime
//!
//! The bridge process's lifetime equals the attach session's
//! lifetime. When the user detaches (F12 → Detach → daemon Goodbye →
//! socket close → bridge sees EOF → bridge exits → SSH child exits),
//! both copy loops return naturally and `run_daemon_attach` returns
//! `Ok(())`. When the SSH client closes its end of the pipe (e.g.,
//! the user kills the local frontend), the bridge's stdin sees EOF,
//! the stdin→socket copy returns, the socket→stdout copy returns
//! when the daemon notices and closes its side, and the function
//! returns `Ok(())`.
//!
//! # Auto-start (M5.7c)
//!
//! If the daemon is not already listening when the bridge tries to
//! connect, [`ensure_daemon_running`] spawns `pmacs --daemon
//! --socket PATH` as a child process and polls until the socket
//! becomes connectable (or a timeout expires). The bridge then
//! proceeds with the byte-pump as if the daemon had been there all
//! along.
//!
//! **No setsid.** The crate forbids `unsafe`, so we cannot use
//! `Command::pre_exec` to detach the daemon from our session. We
//! rely on a different property: in the typical SSH spawn pattern
//! (`ssh host pmacs --daemon-attach`), SSH does not allocate a
//! controlling terminal, so there is no terminal-hangup channel to
//! deliver SIGHUP through. The daemon's parent (the bridge) exits;
//! the daemon is re-parented to init/PID 1; no signal is sent. The
//! daemon survives across attach sessions, which is the contract.
//! Edge cases (e.g., explicit `kill -- -PGID` against the bridge's
//! process group) are accepted limitations for v0.1.

use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Errors produced by the daemon-attach bridge.
#[derive(Debug)]
pub enum BridgeError {
    /// Could not connect to the daemon at `socket`, and auto-start
    /// also failed or timed out. v0.1 messages this with a pointer
    /// at the manual recovery path.
    Connect {
        /// The socket path the bridge tried to connect to.
        socket: PathBuf,
        /// The underlying I/O error from `UnixStream::connect`.
        source: std::io::Error,
    },
    /// Auto-start could not spawn `pmacs --daemon` as a subprocess.
    /// Distinct from [`Self::Connect`] because the failure happened
    /// before the daemon had a chance to bind anything.
    SpawnFailed {
        /// Path the spawner tried to invoke (typically the running
        /// pmacs binary, from `current_exe`).
        exe: PathBuf,
        /// The I/O error from `Command::spawn`.
        source: std::io::Error,
    },
    /// Auto-start spawned a daemon process, but the socket did not
    /// become connectable within the configured timeout. The daemon
    /// child is left running — it may bind eventually or it may
    /// have failed at startup; the user can investigate by running
    /// `pmacs --daemon --socket PATH` directly to see its stderr.
    AutoStartTimeout {
        /// The socket path that never became connectable.
        socket: PathBuf,
        /// The timeout that elapsed.
        after: Duration,
    },
    /// Generic I/O error while bridging bytes (clone, shutdown).
    Io(std::io::Error),
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect { socket, source } => write!(
                f,
                "could not connect to daemon at {}: {source}; \
                 start one first with `pmacs --daemon` and try again",
                socket.display()
            ),
            Self::SpawnFailed { exe, source } => write!(
                f,
                "could not spawn daemon ({}): {source}; \
                 verify the pmacs binary is on PATH and executable",
                exe.display(),
            ),
            Self::AutoStartTimeout { socket, after } => write!(
                f,
                "auto-started daemon did not bind {} within {:.1}s; \
                 run `pmacs --daemon --socket {}` directly to see \
                 startup errors",
                socket.display(),
                after.as_secs_f32(),
                socket.display(),
            ),
            Self::Io(e) => write!(f, "bridge I/O error: {e}"),
        }
    }
}

impl std::error::Error for BridgeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Connect { source, .. } | Self::SpawnFailed { source, .. } | Self::Io(source) => {
                Some(source)
            }
            Self::AutoStartTimeout { .. } => None,
        }
    }
}

/// Default timeout for [`ensure_daemon_running`] to wait for an
/// auto-started daemon to bind its socket.
///
/// The daemon's startup work (lockfile, socket bind, init.lua) is
/// fast — sub-second on a healthy machine — but a busy or cold
/// system can be slower. Five seconds is generous without forcing
/// users to wait long when something is genuinely wrong.
const AUTO_START_TIMEOUT: Duration = Duration::from_secs(5);

/// Polling interval while waiting for the auto-started daemon's
/// socket to become connectable.
const AUTO_START_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Shared with `attach.rs`: when set to a non-empty, non-"0" value,
/// emit stderr breadcrumbs. The remote bridge's stderr is the SSH
/// child's stderr, so this stays off the protocol stdout stream.
const PMACS_ATTACH_DEBUG: &str = "PMACS_ATTACH_DEBUG";

fn bridge_debug_enabled() -> bool {
    std::env::var_os(PMACS_ATTACH_DEBUG).is_some_and(|v| !v.is_empty() && v != "0")
}

fn bridge_debug(msg: impl AsRef<str>) {
    if bridge_debug_enabled() {
        eprintln!("pmacs daemon-attach debug: {}", msg.as_ref());
    }
}

/// Run `pmacs --daemon-attach`: ensure a daemon is listening at
/// `socket_path` (auto-starting one if not), then bridge our
/// stdin/stdout to it.
///
/// This is the entry point the binary calls. The auto-start step
/// runs first so [`run_bridge`]'s `Connect` error path is reached
/// only when *something* genuinely cannot reach the daemon (spawn
/// failed, timeout fired, etc.).
// `PathBuf` by value matches the other CLI entry points
// (`run_attach`, `run_daemon`); pedantic clippy is wrong here.
#[allow(clippy::needless_pass_by_value)]
pub fn run_daemon_attach(socket_path: PathBuf) -> Result<(), BridgeError> {
    bridge_debug(format!(
        "starting bridge for socket {}",
        socket_path.display()
    ));
    let socket = ensure_daemon_running(&socket_path)?;
    bridge_debug("daemon socket ready; entering byte bridge");
    // Use the owned `Stdin` / `Stdout` (not `.lock()`) so the halves
    // can move into threads with `'static` lifetimes. Both are
    // `Send` and behave as expected for line-oriented or byte
    // streaming use.
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    run_bridge_connected(socket, stdin, stdout)
}

/// Make sure a daemon is listening at `socket_path`, auto-starting
/// one if necessary.
///
/// 1. Try to connect. If that succeeds, the daemon is already
///    running — return that stream for the bridge to use.
/// 2. Otherwise spawn `pmacs --daemon --socket PATH` as a child,
///    then poll the socket every [`AUTO_START_POLL_INTERVAL`] until
///    a connect succeeds or [`AUTO_START_TIMEOUT`] elapses. The
///    successful poll stream is returned to the bridge.
///
/// On timeout the spawned daemon child is **not** killed — it may
/// be slow to start rather than broken. The user can investigate by
/// running the daemon directly to see its stderr.
pub(crate) fn ensure_daemon_running(socket_path: &Path) -> Result<UnixStream, BridgeError> {
    ensure_daemon_running_with(socket_path, AUTO_START_TIMEOUT, spawn_daemon_subprocess)
}

/// Spawner-injected variant of [`ensure_daemon_running`] used by
/// tests.
///
/// `spawner` is invoked with the resolved socket path when the
/// initial connect fails; it is responsible for arranging that the
/// path becomes connectable. In production, `spawner` is
/// [`spawn_daemon_subprocess`]. Tests substitute closures that
/// either bind a `UnixListener` from a worker thread (success) or
/// do nothing (timeout).
fn ensure_daemon_running_with<F>(
    socket_path: &Path,
    timeout: Duration,
    spawner: F,
) -> Result<UnixStream, BridgeError>
where
    F: FnOnce(&Path) -> Result<(), BridgeError>,
{
    // Fast path: daemon already listening. Keep this connection and
    // use it as the bridge socket. The daemon speaks first, so a
    // disposable connect probe is not transparent: dropping it before
    // reading the daemon's Hello can make the daemon log BrokenPipe.
    match UnixStream::connect(socket_path) {
        Ok(stream) => {
            bridge_debug(format!(
                "connected to existing daemon at {}",
                socket_path.display()
            ));
            return Ok(stream);
        }
        Err(e) => {
            bridge_debug(format!(
                "initial connect to {} failed: {e}; attempting auto-start",
                socket_path.display()
            ));
        }
    }

    // Slow path: ask the spawner to arrange a daemon, then poll.
    spawner(socket_path)?;
    bridge_debug("auto-start command spawned; polling socket");

    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(stream) = UnixStream::connect(socket_path) {
            bridge_debug(format!(
                "connected to auto-started daemon at {}",
                socket_path.display()
            ));
            return Ok(stream);
        }
        if Instant::now() >= deadline {
            bridge_debug(format!(
                "auto-start timed out after {:.1}s",
                timeout.as_secs_f32()
            ));
            return Err(BridgeError::AutoStartTimeout {
                socket: socket_path.to_path_buf(),
                after: timeout,
            });
        }
        thread::sleep(AUTO_START_POLL_INTERVAL);
    }
}

/// Production spawner: invoke the running pmacs binary with
/// `--daemon --socket PATH`.
///
/// All three standard streams are redirected to `/dev/null` —
/// stdin because the daemon doesn't read it, stdout/stderr because
/// our own stdout is reserved for the bridge's protocol traffic and
/// any daemon log noise would corrupt the wire. Users diagnosing
/// startup errors run `pmacs --daemon` directly, where stderr is
/// inherited normally.
fn spawn_daemon_subprocess(socket_path: &Path) -> Result<(), BridgeError> {
    let exe = std::env::current_exe().map_err(|source| BridgeError::SpawnFailed {
        exe: PathBuf::from("<current_exe unavailable>"),
        source,
    })?;
    bridge_debug(format!(
        "spawning daemon subprocess: {} --daemon --socket {}",
        exe.display(),
        socket_path.display()
    ));
    Command::new(&exe)
        .arg("--daemon")
        .arg("--socket")
        .arg(socket_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| BridgeError::SpawnFailed {
            exe: exe.clone(),
            source,
        })
        .map(|child| {
            bridge_debug(format!("spawned daemon subprocess pid {}", child.id()));
        })?;
    Ok(())
}

/// Generic byte-bridge implementation, parameterized over the local
/// reader and writer for testability.
///
/// Spawns a thread for the local-input → socket direction and runs
/// the socket → local-output direction on the calling thread. When
/// the caller-side copy returns (clean EOF or error), the socket is
/// shutdown to wake the stdin thread's next write, and we attempt
/// to join.
///
/// # Tear-down
///
/// Two natural exits drive the bridge's lifetime:
/// * **Local input EOF** — caller closed stdin (in production: SSH
///   client disconnected). The stdin thread's `copy` returns; the
///   thread exits. The main `socket → output` copy continues until
///   the daemon notices and closes the socket (typically after the
///   detach round-trip).
/// * **Socket EOF** — daemon closed the connection. The main
///   `socket → output` copy returns immediately. We shutdown the
///   socket to wake any pending write on the stdin thread, then
///   join.
///
/// In both paths, `run_bridge` returns `Ok(())`. The caller is the
/// `pmacs --daemon-attach` CLI process whose lifetime equals the
/// bridge's; on return, the process exits and the OS reaps any
/// stdin thread still blocked in a read of stdin (production-only;
/// tests close their input writers explicitly).
/// `std::io::copy` plus a `flush` after every chunk.
///
/// Postcard frames carry no newlines, but the production
/// `local_output` for the bridge is `std::io::Stdout`, whose
/// `LineWriter` only flushes on `\n` or buffer-fill. A 200-byte
/// `Hello` parked in that 1 KiB buffer never reaches the frontend.
/// Flushing per-chunk turns the bridge into a true pass-through —
/// each batch the kernel hands us is forwarded immediately.
///
/// `Interrupted` is retried (matches `std::io::copy`'s behavior).
/// Returns the same `(reader, writer)` error that `copy` would.
fn copy_with_flush<R: Read, W: Write>(reader: &mut R, writer: &mut W) -> std::io::Result<u64> {
    let mut buf = [0u8; 8192];
    let mut total: u64 = 0;
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Ok(total),
            Ok(n) => {
                writer.write_all(&buf[..n])?;
                writer.flush()?;
                total += n as u64;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

/// F8 diagnostic byte-capture. Active only when the marker directory
/// `/tmp/pmacs-bridge-capture` exists on the bridge's host (or
/// `PMACS_BRIDGE_CAPTURE_DIR` points at a directory). When active,
/// each direction's bytes are mirrored verbatim to a file there so
/// the exact wire image the daemon sees can be decoded offline. The
/// mirror is best-effort and never alters the forwarded stream;
/// when inactive this is a single `is_dir` syscall and nothing else.
fn capture_sink(name: &str) -> Option<std::fs::File> {
    let dir = std::env::var_os("PMACS_BRIDGE_CAPTURE_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| {
            let marker = PathBuf::from("/tmp/pmacs-bridge-capture");
            marker.is_dir().then_some(marker)
        })?;
    std::fs::File::create(dir.join(name)).ok()
}

/// Reader that mirrors every byte it yields to a side file
/// (best-effort) before returning it to the caller. Diagnostic only;
/// the byte stream the bridge forwards is unchanged.
struct TeeReader<R> {
    inner: R,
    sink: std::fs::File,
}

impl<R: Read> Read for TeeReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        let _ = self.sink.write_all(&buf[..n]);
        let _ = self.sink.flush();
        Ok(n)
    }
}

// Owned `PathBuf` matches the public entry-point signature; clippy's
// pedantic pass-by-value is wrong for this seam.
#[allow(clippy::needless_pass_by_value)]
#[cfg(test)]
pub(crate) fn run_bridge<R, W>(
    socket_path: PathBuf,
    local_input: R,
    local_output: W,
) -> Result<(), BridgeError>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let socket = UnixStream::connect(&socket_path).map_err(|source| BridgeError::Connect {
        socket: socket_path.clone(),
        source,
    })?;
    run_bridge_connected(socket, local_input, local_output)
}

fn run_bridge_connected<R, W>(
    socket: UnixStream,
    local_input: R,
    mut local_output: W,
) -> Result<(), BridgeError>
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let socket_for_writer = socket.try_clone().map_err(BridgeError::Io)?;
    let kick_handle = socket.try_clone().map_err(BridgeError::Io)?;
    let mut socket_reader = socket;
    bridge_debug("bridge connected; starting copy loops");

    let (done_tx, done_rx) = mpsc::channel::<()>();
    let stdin_thread = thread::spawn(move || {
        let mut local_input = local_input;
        let mut socket_writer = socket_for_writer;
        bridge_debug("stdin->daemon copy loop started");
        // Diagnostic: mirror exactly the bytes we forward toward the
        // daemon (the client's AttachRequest et seq).
        let result = match capture_sink("to_daemon.bin") {
            Some(sink) => {
                let mut tee = TeeReader {
                    inner: local_input,
                    sink,
                };
                copy_with_flush(&mut tee, &mut socket_writer)
            }
            None => copy_with_flush(&mut local_input, &mut socket_writer),
        };
        match &result {
            Ok(n) => bridge_debug(format!("stdin->daemon copy loop ended after {n} bytes")),
            Err(e) => bridge_debug(format!("stdin->daemon copy loop failed: {e}")),
        }
        // After stdin EOF, signal write-side close so the daemon's
        // blocking read returns 0. Without this, the daemon never
        // sees EOF (we hold other socket FDs on the main thread)
        // and the socket→stdout copy on the main thread blocks
        // forever.
        let _ = socket_writer.shutdown(Shutdown::Write);
        let _ = done_tx.send(());
        result
    });

    // Main thread: forward instance bytes to local stdout. Returns
    // when the socket closes (EOF) or stdout breaks. Errors are
    // swallowed — once we've decided to tear down, the caller cares
    // only about whether the connect succeeded, not about the exact
    // shape of the disconnect.
    // Diagnostic: mirror exactly the bytes the daemon sent us (its
    // Hello et seq), as the client receives them.
    bridge_debug("daemon->stdout copy loop started");
    let daemon_to_stdout = match capture_sink("from_daemon.bin") {
        Some(sink) => {
            let mut tee = TeeReader {
                inner: socket_reader,
                sink,
            };
            copy_with_flush(&mut tee, &mut local_output)
        }
        None => copy_with_flush(&mut socket_reader, &mut local_output),
    };
    match &daemon_to_stdout {
        Ok(n) => bridge_debug(format!("daemon->stdout copy loop ended after {n} bytes")),
        Err(e) => bridge_debug(format!("daemon->stdout copy loop failed: {e}")),
    }
    let _ = daemon_to_stdout;

    // Wake the stdin thread's pending socket write. If the thread is
    // currently blocked in a *read* of local_input, this doesn't
    // help; we rely on the caller (or production stdin EOF) to
    // unblock that path. Best-effort.
    let _ = kick_handle.shutdown(Shutdown::Both);

    // Best-effort *bounded* join. Two real cases:
    //
    // * Clean detach / SSH-side EOF: the stdin thread exits within
    //   microseconds (`copy_with_flush` returned, `shutdown(Write)`
    //   ran, the channel send fired). The recv_timeout returns
    //   immediately and we proceed.
    // * Daemon crash mid-session: the daemon is gone but the SSH
    //   client still holds our stdin open, so the stdin thread is
    //   wedged forever in `read(local_input)`. A pure `join()`
    //   would block the bridge process forever, leaving the user's
    //   `pmacs --attach` stuck because our stdout never closes.
    //   Bound the wait so the bridge can exit; the OS reaps the
    //   wedged thread on process termination.
    //
    // 500ms is a generous upper bound on natural exit latency
    // without making the crash-recovery path feel sluggish.
    let _ = done_rx.recv_timeout(Duration::from_millis(500));
    drop(stdin_thread);

    bridge_debug("bridge exiting cleanly");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Try to bind a `UnixListener` at `path`. On `PermissionDenied`
    /// (e.g., a sandboxed CI environment that disallows `AF_UNIX`
    /// socket creation), prints a skip notice to stderr and returns
    /// `None`; the calling test should then early-return so the
    /// suite reports `0 failed` rather than a misleading panic.
    ///
    /// Any other `io::Error` is still a hard failure: the test
    /// should not silently skip on (e.g.) `EADDRINUSE` --- that's a
    /// real bug in the test setup.
    fn bind_or_skip(path: &Path) -> Option<UnixListener> {
        match UnixListener::bind(path) {
            Ok(listener) => Some(listener),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                eprintln!(
                    "test skipped: UnixListener::bind {} → PermissionDenied \
                     (sandboxed environment). To run this test, give the \
                     test process permission to create AF_UNIX sockets.",
                    path.display()
                );
                None
            }
            Err(e) => panic!("UnixListener::bind {} failed: {e}", path.display()),
        }
    }

    /// End-to-end byte echo through the bridge.
    ///
    /// Setup:
    /// * A `UnixListener` plays the daemon at a tempdir socket.
    /// * The daemon thread accepts one connection and echoes every
    ///   byte it receives back to itself.
    /// * Two pipe pairs simulate the local stdin/stdout.
    /// * `run_bridge` runs in its own thread.
    ///
    /// Test asserts: a known sequence of bytes written to the input
    /// writer reappears on the output reader after the round trip
    /// through the daemon.
    #[test]
    fn bridge_round_trips_bytes_through_daemon() {
        let tmp = tempfile::tempdir().unwrap();
        let socket_path = tmp.path().join("test.sock");
        let Some(listener) = bind_or_skip(&socket_path) else {
            return;
        };

        // Echo daemon: read everything, write it back, exit on EOF.
        let daemon = thread::spawn(move || {
            let (mut stream, _addr) = listener.accept().unwrap();
            let mut stream_clone = stream.try_clone().unwrap();
            let mut buf = [0u8; 1024];
            loop {
                let n = match stream.read(&mut buf) {
                    Ok(n) if n > 0 => n,
                    _ => break,
                };
                if stream_clone.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        });

        // Pipe pairs standing in for stdin/stdout.
        let (input_reader, mut input_writer) = pipe();
        let (mut output_reader, output_writer) = pipe();

        let bridge_socket = socket_path.clone();
        let bridge = thread::spawn(move || run_bridge(bridge_socket, input_reader, output_writer));

        // Round-trip a known sequence.
        let payload = b"hello, bridge\n";
        input_writer.write_all(payload).unwrap();
        input_writer.flush().unwrap();

        let mut received = vec![0u8; payload.len()];
        // Read exactly payload.len() bytes back. read_exact retries
        // on Interrupted but will return UnexpectedEof if the bridge
        // closes early — that would fail the test loudly.
        output_reader.read_exact(&mut received).unwrap();
        assert_eq!(&received, payload);

        // Tear down: drop the input writer to EOF the stdin thread,
        // which closes the socket-side write. The daemon notices,
        // closes its end, the socket-side EOF reaches the bridge's
        // socket_reader, the main `copy` returns, run_bridge returns.
        drop(input_writer);

        // Bound the wait for run_bridge to return; if anything
        // deadlocks we want the test to fail with a clear message
        // rather than hang the whole suite.
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let result = bridge.join();
            let _ = done_tx.send(result);
        });
        let bridge_result = done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("run_bridge should return within 2s after EOF");
        let bridge_result = bridge_result.expect("bridge thread did not panic");
        assert!(
            bridge_result.is_ok(),
            "run_bridge should return Ok on clean EOF, got {bridge_result:?}",
        );

        let _ = daemon.join();
    }

    #[test]
    fn connect_failure_yields_workaround_pointing_message() {
        // No daemon running here. The Connect error message tells the
        // user how to recover.
        let tmp = tempfile::tempdir().unwrap();
        let socket_path = tmp.path().join("nonexistent.sock");

        let (input_reader, _input_writer) = pipe();
        let (_output_reader, output_writer) = pipe();

        let result = run_bridge(socket_path.clone(), input_reader, output_writer);
        match result {
            Err(BridgeError::Connect { socket, source: _ }) => {
                assert_eq!(socket, socket_path);
                let msg = format!(
                    "{}",
                    BridgeError::Connect {
                        socket: socket.clone(),
                        source: std::io::Error::other("test"),
                    }
                );
                assert!(
                    msg.contains("pmacs --daemon"),
                    "error message must point at the workaround: {msg}",
                );
            }
            other => panic!("expected Connect error, got {other:?}"),
        }
    }

    /// In-process pipe pair using a `UnixStream::pair`. Each half is
    /// `Read + Write + Send + 'static`; tests only use one direction
    /// of each.
    fn pipe() -> (UnixStream, UnixStream) {
        UnixStream::pair().expect("UnixStream::pair")
    }

    /// Writer that buffers internally and only "delivers" on explicit
    /// flush — models `std::io::Stdout`'s `LineWriter` against binary
    /// payloads (no newlines means `write` alone never reaches the
    /// reader). Surfaces buffered-but-unflushed bytes as a discrepancy
    /// between `delivered.len()` and the post-write byte count.
    struct FlushOnlyWriter {
        buffer: Vec<u8>,
        delivered: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for FlushOnlyWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.buffer.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.delivered
                .lock()
                .unwrap()
                .extend_from_slice(&self.buffer);
            self.buffer.clear();
            Ok(())
        }
    }

    /// Regression test for the `LineWriter` / buffered-stdout bug:
    /// `copy_with_flush` must call `flush` after every chunk, not
    /// just rely on `write_all`. Without the per-chunk flush, the
    /// production `pmacs --daemon-attach` parked a 200-byte `Hello`
    /// frame in stdout's `LineWriter` buffer and the frontend never
    /// saw it until the buffer filled.
    #[test]
    fn copy_with_flush_delivers_each_chunk_without_waiting_for_buffer_fill() {
        let payload: &[u8] = &[0xAA; 200]; // postcard-shaped binary, no '\n'
        let mut reader: &[u8] = payload;
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let mut writer = FlushOnlyWriter {
            buffer: Vec::new(),
            delivered: delivered.clone(),
        };

        let total = copy_with_flush(&mut reader, &mut writer).expect("copy ok");
        assert_eq!(total, payload.len() as u64);
        assert_eq!(
            delivered.lock().unwrap().as_slice(),
            payload,
            "every chunk must be flushed (else the bridge stalls on small binary frames)",
        );
        assert!(
            writer.buffer.is_empty(),
            "no bytes left unflushed in the writer's internal buffer",
        );
    }

    // -----------------------------------------------------------------
    // M5.7c — auto-start helper tests
    // -----------------------------------------------------------------

    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn ensure_running_returns_immediately_when_daemon_already_listening() {
        let tmp = tempfile::tempdir().unwrap();
        let socket_path = tmp.path().join("test.sock");
        let Some(_listener) = bind_or_skip(&socket_path) else {
            return;
        };

        let spawner_called = Arc::new(AtomicBool::new(false));
        let sc = spawner_called.clone();

        let result = ensure_daemon_running_with(
            &socket_path,
            Duration::from_secs(1),
            move |_path: &Path| -> Result<(), BridgeError> {
                sc.store(true, Ordering::SeqCst);
                Ok(())
            },
        );

        assert!(result.is_ok(), "fast path should return Ok, got {result:?}");
        assert!(
            !spawner_called.load(Ordering::SeqCst),
            "spawner must not be invoked when daemon is already listening",
        );
    }

    #[test]
    fn ensure_running_reuses_existing_connection_instead_of_probe_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let socket_path = tmp.path().join("test.sock");
        let Some(listener) = bind_or_skip(&socket_path) else {
            return;
        };

        let daemon = thread::spawn(move || {
            let (mut stream, _addr) = listener.accept().unwrap();
            stream.write_all(b"HELLO").unwrap();
            stream.flush().unwrap();
        });

        let mut stream = ensure_daemon_running_with(
            &socket_path,
            Duration::from_secs(1),
            |_path: &Path| -> Result<(), BridgeError> {
                panic!("spawner must not run when daemon is already listening")
            },
        )
        .expect("existing daemon connection");

        let mut received = [0u8; 5];
        stream.read_exact(&mut received).unwrap();
        assert_eq!(&received, b"HELLO");

        daemon.join().unwrap();
    }

    #[test]
    fn ensure_running_invokes_spawner_then_waits_for_socket_to_appear() {
        let tmp = tempfile::tempdir().unwrap();
        let socket_path = tmp.path().join("test.sock");

        // Probe-bind once up front. The actual bind happens inside a
        // child thread (and we cannot return from the test from
        // there), so checking PermissionDenied here lets us skip
        // before scheduling the worker.
        match bind_or_skip(&socket_path) {
            Some(listener) => drop(listener),
            None => return,
        }
        // Some kernels keep the inode visible after drop. Make sure
        // the path is gone so the spawner's bind doesn't EADDRINUSE.
        let _ = std::fs::remove_file(&socket_path);

        // Spawner: defer binding by 100ms in a worker thread, then
        // hold the listener long enough for `ensure_running` to see
        // it. The spawner returns Ok as soon as the worker is
        // launched; the polling in `ensure_running` does the rest.
        let spawner = move |path: &Path| -> Result<(), BridgeError> {
            let path = path.to_path_buf();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(100));
                let listener = UnixListener::bind(&path).unwrap();
                // Hold the listener until the test should be done.
                thread::sleep(Duration::from_millis(500));
                drop(listener);
            });
            Ok(())
        };

        let start = Instant::now();
        let result = ensure_daemon_running_with(&socket_path, Duration::from_secs(2), spawner);
        let elapsed = start.elapsed();

        assert!(result.is_ok(), "expected Ok, got {result:?}");
        assert!(
            elapsed >= Duration::from_millis(100),
            "should have waited ~100ms for the worker to bind, took {elapsed:?}",
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "should not have hit the timeout; took {elapsed:?}",
        );
    }

    #[test]
    fn ensure_running_times_out_when_socket_never_binds() {
        let tmp = tempfile::tempdir().unwrap();
        let socket_path = tmp.path().join("test.sock");

        // No-op spawner: returns Ok but never binds the socket.
        // ensure_running should poll until the timeout fires.
        let spawner = |_path: &Path| -> Result<(), BridgeError> { Ok(()) };

        let timeout = Duration::from_millis(200);
        let start = Instant::now();
        let result = ensure_daemon_running_with(&socket_path, timeout, spawner);
        let elapsed = start.elapsed();

        match result {
            Err(BridgeError::AutoStartTimeout { socket, after }) => {
                assert_eq!(socket, socket_path);
                assert_eq!(after, timeout);
            }
            other => panic!("expected AutoStartTimeout, got {other:?}"),
        }
        assert!(
            elapsed >= timeout,
            "should have waited at least {timeout:?}, took {elapsed:?}",
        );
        // Loose upper bound: timeout + one poll interval + scheduling
        // slack.
        assert!(
            elapsed < timeout + Duration::from_millis(500),
            "should have returned shortly after the timeout, took {elapsed:?}",
        );
    }

    #[test]
    fn ensure_running_propagates_spawner_error() {
        let tmp = tempfile::tempdir().unwrap();
        let socket_path = tmp.path().join("test.sock");

        // Spawner that fails outright. The error must bubble up
        // unchanged so users see *why* spawn failed (not just "your
        // daemon never showed up").
        let spawner = |_path: &Path| -> Result<(), BridgeError> {
            Err(BridgeError::SpawnFailed {
                exe: PathBuf::from("/path/to/test"),
                source: std::io::Error::other("test forced spawn failure"),
            })
        };

        let result = ensure_daemon_running_with(&socket_path, Duration::from_secs(1), spawner);
        match result {
            Err(BridgeError::SpawnFailed { exe, source }) => {
                assert_eq!(exe, PathBuf::from("/path/to/test"));
                assert!(format!("{source}").contains("test forced spawn failure"));
            }
            other => panic!("expected SpawnFailed, got {other:?}"),
        }
    }

    #[test]
    fn auto_start_timeout_message_points_at_manual_recovery() {
        let err = BridgeError::AutoStartTimeout {
            socket: PathBuf::from("/tmp/x.sock"),
            after: Duration::from_secs(5),
        };
        let msg = format!("{err}");
        assert!(
            msg.contains("pmacs --daemon"),
            "timeout message must point at manual recovery: {msg}",
        );
        assert!(msg.contains("/tmp/x.sock"));
        assert!(msg.contains("5.0s"));
    }

    #[test]
    fn spawn_failed_message_includes_exe_path_and_workaround_pointer() {
        let err = BridgeError::SpawnFailed {
            exe: PathBuf::from("/usr/local/bin/pmacs"),
            source: std::io::Error::other("permission denied"),
        };
        let msg = format!("{err}");
        assert!(msg.contains("/usr/local/bin/pmacs"));
        assert!(msg.contains("permission denied"));
        assert!(msg.contains("PATH"), "should hint at PATH check: {msg}",);
    }

    /// F8 reproduction (M10.11 acceptance criterion 1, SSH transport).
    ///
    /// Models the production failure: the daemon speaks first
    /// (`per_attach_thread` writes `Hello` *before* reading
    /// `AttachRequest`, see `src/daemon.rs:617`), and the bridge's
    /// local input reaches EOF immediately — exactly what
    /// `std::io::empty()` provides, and what was observed when
    /// `pmacs --attach ssh:...` drove the remote `pmacs
    /// --daemon-attach`.
    ///
    /// The daemon thread deliberately sleeps before writing `Hello`
    /// so the bridge's immediate-stdin-EOF teardown path
    /// (`copy_with_flush` returns `Ok(0)` → `shutdown(Write)` →
    /// `done_tx`) wins the race, reproducing the production ordering
    /// where the daemon's first write lands *after* the bridge has
    /// reacted to EOF.
    ///
    /// Acceptance: the daemon's `Hello` write must NOT fail with
    /// `BrokenPipe`, and the `Hello` bytes must reach the bridge's
    /// local output. If this test fails, F8 is reproduced in-process
    /// and the defect is in `run_bridge`'s local teardown logic — not
    /// ssh-specific. If it passes against unfixed code, the in-process
    /// model does NOT capture F8 and the cause is ssh-transport
    /// specific (report honestly; do not claim a `run_bridge` fix).
    #[test]
    fn f8_daemon_speaks_first_survives_immediate_stdin_eof() {
        // Length-prefixed Hello-shaped frame. Content is irrelevant —
        // the bridge is byte-transparent; only survival matters.
        const HELLO_FRAME: &[u8] = &[0x00, 0x00, 0x00, 0x05, b'H', b'E', b'L', b'L', b'O'];

        let tmp = tempfile::tempdir().unwrap();
        let socket_path = tmp.path().join("f8.sock");
        let Some(listener) = bind_or_skip(&socket_path) else {
            return;
        };

        let (write_result_tx, write_result_rx) = mpsc::channel::<std::io::Result<()>>();
        let daemon = thread::spawn(move || {
            let (mut stream, _addr) = listener.accept().unwrap();
            // Production ordering: the bridge has process/network
            // latency before our first write reaches it. Sleep so the
            // bridge's immediate-EOF teardown definitely happens first.
            thread::sleep(Duration::from_millis(150));
            let result = stream.write_all(HELLO_FRAME).and_then(|()| stream.flush());
            let _ = write_result_tx.send(result);
            // Mirror per_attach_thread: after Hello, read AttachRequest
            // (here just drain to EOF so the socket lifecycle matches).
            let mut sink = Vec::new();
            let _ = stream.read_to_end(&mut sink);
        });

        let (mut output_reader, output_writer) = pipe();
        let bridge_socket = socket_path.clone();
        // std::io::empty() == immediate stdin EOF.
        let bridge =
            thread::spawn(move || run_bridge(bridge_socket, std::io::empty(), output_writer));

        // 1. The daemon's Hello write must not EPIPE.
        let write_result = write_result_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("daemon thread should attempt the Hello write within 3s");
        assert!(
            write_result.is_ok(),
            "F8 reproduced: daemon's Hello write failed ({:?}) — \
             run_bridge tore the socket down on immediate stdin-EOF \
             before the daemon's first write could land",
            write_result.as_ref().err().map(std::io::Error::kind),
        );

        // 2. The Hello bytes must reach the bridge's local output.
        let mut received = vec![0u8; HELLO_FRAME.len()];
        output_reader
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        match output_reader.read_exact(&mut received) {
            Ok(()) => assert_eq!(received, HELLO_FRAME, "Hello bytes corrupted in transit",),
            Err(e) => panic!(
                "F8 reproduced: Hello never reached local output ({e}) — \
                 the bridge dropped the daemon→client direction on \
                 immediate stdin-EOF",
            ),
        }

        // Bounded teardown so a deadlock fails loudly, not silently.
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = done_tx.send(bridge.join());
        });
        let _ = done_rx.recv_timeout(Duration::from_secs(3));
        let _ = daemon.join();
    }
}
