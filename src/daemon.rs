// daemon.rs --- Daemon mode for local-attach transport.

//! Daemon mode for the M5.5 local-attach transport (T M5.5e).
//!
//! # Lifecycle
//!
//! [`run_daemon`] is invoked from `main` when the user passes
//! `--daemon`. It:
//!
//! 1. Prepares the runtime subdir (`<runtime>/pmacs/`) under mode 0700
//!    via [`crate::socket_path::ensure_runtime_subdir`].
//! 2. Acquires the sibling lockfile via
//!    [`crate::lockfile::acquire_lock`].
//! 3. Unlinks any stale socket file (a previous crashed daemon may
//!    have left one).
//! 4. Binds the `UnixListener` under `umask(0077)` so the socket file
//!    gets mode 0600.
//! 5. Installs signal handlers: SIGTERM/SIGINT set a shutdown flag;
//!    SIGPIPE/SIGHUP get no-op handlers so writes return EPIPE rather
//!    than killing the process and SIGHUP is reserved for v0.2+ config
//!    reload.
//! 6. Runs an accept loop with non-blocking `accept(2)` and a 50 ms
//!    poll interval. Each accepted connection runs through the
//!    handshake and per-attach scaffolding inline.
//! 7. On shutdown: drops the listener, unlinks the socket, releases
//!    the lock.
//!
//! # Per-attach handler (M5.5f)
//!
//! After [`Hello`] / [`AttachRequest`] / version-check / already-attached
//! checks succeed, the connection enters [`run_per_attach`]:
//!
//! - A reader thread blocks on [`crate::transport::read_message`] and
//!   forwards each [`FrontendEvent`] into an `mpsc` channel.
//! - The main thread renders one frame, writes the resulting
//!   [`InstanceMessage`]s, then waits up to `frame_target_ms` for an
//!   event. Bursts of events are coalesced into a single render pass
//!   (matching the in-process TUI's behavior).
//! - On each iteration the loop ticks the async runtime, process
//!   supervisor, and LSP host so background work progresses.
//!
//! Exit paths:
//! - `FrontendEvent::Detach` → return without sending Goodbye (the
//!   frontend closed the conversation).
//! - Reader thread closes the channel (EOF / I/O / decode) → return.
//! - Shutdown flag set → send `Goodbye(ShuttingDown)`, return.
//! - Editor `quit` flag set → send `Goodbye(ShuttingDown)`, propagate
//!   shutdown to the outer accept loop.
//! - Write fails (broken pipe) → return; ungraceful disconnect.

use std::io::ErrorKind;
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::cell::CellSize;
use crate::editor::EditorState;
use crate::instance_render::RenderState;
use crate::lockfile::{self, LockError, LockHandle};
use crate::protocol::crossterm_translate::{key_to_crossterm, mouse_to_crossterm};
use crate::protocol::{
    AttachRequest, FrontendEvent, FrontendId, GoodbyeReason, Hello, InstanceCapabilities,
    InstanceIdentity, InstanceMessage, PROTOCOL_VERSION,
};
use crate::socket_path::{SocketPathError, ensure_runtime_subdir};
use crate::transport::{read_message, write_message};

/// Errors that abort the daemon's startup or main loop.
#[derive(Debug)]
pub enum DaemonError {
    /// Could not acquire or release the daemon lockfile.
    Lock(LockError),
    /// Could not prepare the runtime directory.
    SocketPath(SocketPathError),
    /// I/O error during bind, accept, or socket-file unlink.
    Io(std::io::Error),
    /// Could not install a signal handler.
    Signal(std::io::Error),
}

impl std::fmt::Display for DaemonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lock(e) => write!(f, "{e}"),
            Self::SocketPath(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "daemon I/O error: {e}"),
            Self::Signal(e) => write!(f, "signal handler installation failed: {e}"),
        }
    }
}

impl std::error::Error for DaemonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lock(e) => Some(e),
            Self::SocketPath(e) => Some(e),
            Self::Io(e) | Self::Signal(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for DaemonError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<LockError> for DaemonError {
    fn from(e: LockError) -> Self {
        Self::Lock(e)
    }
}

impl From<SocketPathError> for DaemonError {
    fn from(e: SocketPathError) -> Self {
        Self::SocketPath(e)
    }
}

/// Interval between non-blocking `accept` polls in the main loop.
///
/// Short enough that SIGTERM-to-exit latency is bounded by ~50 ms.
/// Long enough that an idle daemon doesn't burn CPU.
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Per-daemon state shared between the accept loop and per-attach
/// handlers.
struct DaemonState {
    instance_name: Option<String>,
    started: Instant,
    next_frontend_id: AtomicU64,
    /// `true` while a frontend is attached. v0.1 single-frontend
    /// invariant: a second attach attempt sees this set and is
    /// rejected with `Goodbye(AlreadyAttached)`.
    attached: AtomicBool,
}

impl DaemonState {
    fn new(instance_name: Option<String>) -> Self {
        Self {
            instance_name,
            started: Instant::now(),
            // FrontendId(1) is reserved for FrontendId::LOCAL (the
            // in-process TUI). Daemon-attached frontends start at 2.
            next_frontend_id: AtomicU64::new(2),
            attached: AtomicBool::new(false),
        }
    }

    fn build_identity(&self) -> InstanceIdentity {
        InstanceIdentity::for_running_process(self.instance_name.clone(), self.started)
    }

    /// `--socket NAME` value the daemon was launched with, or `None`
    /// for the unnamed default daemon. The editor mirrors this into
    /// `pmacs.instance.identity()` (T M5.6f).
    fn instance_name(&self) -> Option<String> {
        self.instance_name.clone()
    }

    /// Start anchor used to compute uptimes. The editor mirrors this
    /// into `pmacs.instance.identity()` (T M5.6f) so the uptime
    /// reported on Lua matches what the daemon sends in its Hello.
    fn started(&self) -> Instant {
        self.started
    }
}

/// Run a daemon on `socket_path`. Returns when SIGTERM / SIGINT have
/// been received and the daemon has cleaned up.
///
/// `instance_name` is the value the user passed via `--socket NAME`
/// (resolved already; this is the human-readable name surfaced in
/// [`InstanceIdentity::instance_name`]). `None` means the default
/// daemon.
// `socket_path: PathBuf` is taken by value so the caller can hand
// the result of `resolve_socket_path` straight in without keeping a
// local; clippy's pedantic pass-by-value is wrong for this call site.
#[allow(clippy::needless_pass_by_value)]
pub fn run_daemon(socket_path: PathBuf, instance_name: Option<String>) -> Result<(), DaemonError> {
    ensure_runtime_subdir(&socket_path)?;
    let lock = lockfile::acquire_lock(&socket_path)?;

    if socket_path.exists() {
        // Stale socket from a previously crashed daemon. The lock we
        // just acquired guarantees no live daemon owns it — safe to
        // unlink and replace.
        std::fs::remove_file(&socket_path)?;
    }

    let listener = bind_with_strict_umask(&socket_path)?;
    listener.set_nonblocking(true)?;

    let shutdown = Arc::new(AtomicBool::new(false));
    install_signal_handlers(&shutdown)?;

    let daemon_state = Arc::new(DaemonState::new(instance_name));
    // The editor and render-state outlive any single attachment; they
    // are constructed once and reused across detach / reattach cycles.
    // The render-state's initial size is a placeholder — the first
    // `AttachRequest` carries the real size and we resize on attach.
    let mut editor = EditorState::new();
    // Mirror the daemon's `--socket NAME` and start time into the
    // editor's `LocalInstanceInfo` so `pmacs.instance.identity()`
    // (T M5.6f) reports the same identity the daemon hands back over
    // its Hello. The DaemonState built above is the source of truth
    // for both, so the editor's identity stays in lock-step with the
    // wire payload.
    editor
        .lua_host
        .set_instance_name(daemon_state.instance_name());
    editor.lua_host.set_instance_started(daemon_state.started());
    let mut render_state = RenderState::new(CellSize::new(24, 80));

    eprintln!(
        "pmacs: daemon listening on {} (pid {})",
        socket_path.display(),
        std::process::id()
    );

    accept_loop(
        &listener,
        &daemon_state,
        &mut editor,
        &mut render_state,
        &shutdown,
    )?;
    cleanup(listener, &socket_path, lock);
    eprintln!("pmacs: daemon stopped");
    Ok(())
}

fn accept_loop(
    listener: &UnixListener,
    daemon_state: &Arc<DaemonState>,
    editor: &mut EditorState,
    render_state: &mut RenderState,
    shutdown: &Arc<AtomicBool>,
) -> Result<(), DaemonError> {
    while !shutdown.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(e) =
                    handle_connection(stream, daemon_state, editor, render_state, shutdown)
                {
                    eprintln!("pmacs: connection handler: {e}");
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_POLL_INTERVAL);
            }
            Err(e) => return Err(DaemonError::Io(e)),
        }
    }
    Ok(())
}

fn cleanup(listener: UnixListener, socket_path: &Path, lock: LockHandle) {
    drop(listener);
    let _ = std::fs::remove_file(socket_path);
    let _ = lock.release();
}

/// Bind a Unix-domain listener under a strict umask so the socket
/// file gets mode 0600.
fn bind_with_strict_umask(socket_path: &Path) -> std::io::Result<UnixListener> {
    let strict = nix::sys::stat::Mode::from_bits_truncate(0o077);
    let prev = nix::sys::stat::umask(strict);
    let result = UnixListener::bind(socket_path);
    nix::sys::stat::umask(prev);
    result
}

fn install_signal_handlers(shutdown: &Arc<AtomicBool>) -> Result<(), DaemonError> {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGPIPE, SIGTERM};

    signal_hook::flag::register(SIGTERM, Arc::clone(shutdown)).map_err(DaemonError::Signal)?;
    signal_hook::flag::register(SIGINT, Arc::clone(shutdown)).map_err(DaemonError::Signal)?;

    // SIGPIPE / SIGHUP: install no-op handlers so the kernel doesn't
    // apply the default action (terminate). With a handler installed
    // and `SA_RESTART` set (signal_hook's default), interrupted
    // syscalls are restarted automatically; the practical effect is
    // that `write(2)` returns `EPIPE` on a broken pipe rather than
    // killing the process.
    let dummy = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGPIPE, Arc::clone(&dummy)).map_err(DaemonError::Signal)?;
    signal_hook::flag::register(SIGHUP, Arc::clone(&dummy)).map_err(DaemonError::Signal)?;

    Ok(())
}

fn handle_connection(
    mut stream: UnixStream,
    daemon_state: &Arc<DaemonState>,
    editor: &mut EditorState,
    render_state: &mut RenderState,
    shutdown: &Arc<AtomicBool>,
) -> Result<(), DaemonError> {
    let frontend_id = FrontendId(daemon_state.next_frontend_id.fetch_add(1, Ordering::SeqCst));

    // Send Hello immediately on accept.
    let hello = Hello {
        protocol_version: PROTOCOL_VERSION,
        assigned_frontend_id: frontend_id,
        instance_identity: daemon_state.build_identity(),
        instance_capabilities: InstanceCapabilities::default(),
    };
    if let Err(e) = write_message(&mut stream, &hello) {
        eprintln!("pmacs: send Hello failed: {e}");
        return Ok(());
    }

    // Read AttachRequest.
    let req: AttachRequest = match read_message(&mut stream) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("pmacs: read AttachRequest failed: {e}");
            return Ok(());
        }
    };

    // Version check.
    if req.protocol_version != PROTOCOL_VERSION {
        let _ = write_message(
            &mut stream,
            &InstanceMessage::Goodbye(GoodbyeReason::VersionMismatch {
                server: PROTOCOL_VERSION,
                client: req.protocol_version,
            }),
        );
        return Ok(());
    }

    // Already-attached check (single-frontend invariant).
    if daemon_state
        .attached
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        let _ = write_message(
            &mut stream,
            &InstanceMessage::Goodbye(GoodbyeReason::AlreadyAttached),
        );
        return Ok(());
    }

    // Stamp `active_frontend` so Lua's `pmacs.frontend.id()` returns
    // the assigned id even before any input event arrives. dispatch_key
    // and dispatch_mouse will overwrite this on every event, but the
    // initial value matters for code that runs during the initial
    // render (Lua statusline functions, etc.).
    editor.core.borrow_mut().active_frontend = frontend_id;

    let result = run_per_attach(stream, editor, render_state, shutdown, req.initial_size);
    daemon_state.attached.store(false, Ordering::SeqCst);
    result
}

/// Drive the editor for one attached frontend until Detach / EOF /
/// shutdown / write-failure (broken pipe).
///
/// Threading model:
/// - Reader thread blocks on [`read_message`] and pushes events into
///   an `mpsc::channel`.
/// - This (main) thread renders, writes [`InstanceMessage`]s back to
///   the socket, then waits up to one frame for events on the channel
///   before ticking async work and looping.
///
/// Initial frame: `render_state` is resized to the frontend's
/// announced `initial_size` and forced to a full-grid resync, so the
/// first frame paints the entire screen.
///
/// Detach semantics: the daemon does *not* send a Goodbye on graceful
/// detach — the frontend asked first, and a Goodbye for an already-
/// closed connection is noise. Goodbye is reserved for the cases
/// where the instance is doing the closing (shutdown, version
/// mismatch, already-attached, protocol error).
fn run_per_attach(
    stream: UnixStream,
    editor: &mut EditorState,
    render_state: &mut RenderState,
    shutdown: &Arc<AtomicBool>,
    initial_size: CellSize,
) -> Result<(), DaemonError> {
    let reader_stream = stream.try_clone()?;
    let kick_stream = stream.try_clone()?;
    let mut writer_stream = stream;

    let (tx, rx) = mpsc::channel::<FrontendEvent>();
    let reader_handle = thread::spawn(move || run_reader(reader_stream, tx));

    // Initial sync: resize the render-state to the frontend's announced
    // size, force the next frame to be a full-grid resync.
    render_state.resize(initial_size);
    render_state.force_full_grid_resync();
    let mut term_size = initial_size;

    loop {
        // Render and ship.
        let messages = render_state.render_frame(editor);
        let mut write_failed = false;
        for msg in &messages {
            if let Err(e) = write_message(&mut writer_stream, msg) {
                eprintln!("pmacs: write failed in per-attach loop: {e}");
                write_failed = true;
                break;
            }
        }
        if write_failed {
            break;
        }

        if editor.core.borrow().quit {
            // The editor wants to quit (M-x kill-pmacs / `:q` / etc.).
            // In daemon mode, "quit" means shutting down the whole
            // instance — propagate to the outer accept loop.
            let _ = write_message(
                &mut writer_stream,
                &InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown),
            );
            shutdown.store(true, Ordering::SeqCst);
            break;
        }

        if shutdown.load(Ordering::SeqCst) {
            let _ = write_message(
                &mut writer_stream,
                &InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown),
            );
            break;
        }

        // Wait up to one frame for input.
        let frame_target = editor.async_runtime.frame_target_ms();
        let mut had_event = false;
        let mut detached = false;
        match rx.recv_timeout(Duration::from_millis(frame_target)) {
            Ok(FrontendEvent::Detach(_)) => {
                detached = true;
            }
            Ok(ev) => {
                apply_event(editor, ev, &mut term_size, render_state);
                had_event = true;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if detached {
            break;
        }

        if had_event {
            // Drain the burst — coalesce a typing-flurry into one
            // frame, matching the TUI run loop's behavior.
            loop {
                match rx.try_recv() {
                    Ok(FrontendEvent::Detach(_)) => {
                        detached = true;
                        break;
                    }
                    Ok(ev) => apply_event(editor, ev, &mut term_size, render_state),
                    Err(_) => break,
                }
            }
            if detached {
                break;
            }
        }

        editor.tick_async();
        editor.tick_processes();
        editor.tick_lsp();
    }

    let _ = kick_stream.shutdown(Shutdown::Read);
    let _ = reader_handle.join();
    Ok(())
}

// Takes `ev` by value because it semantically consumes the event;
// the caller pulls events out of the channel one at a time and never
// needs to look at them again.
#[allow(clippy::needless_pass_by_value)]
fn apply_event(
    editor: &mut EditorState,
    ev: FrontendEvent,
    term_size: &mut CellSize,
    render_state: &mut RenderState,
) {
    match ev {
        FrontendEvent::Key(pmacs_key) => {
            if let Some(ct_key) = key_to_crossterm(&pmacs_key) {
                editor.dispatch_key(pmacs_key.frontend_id, ct_key);
            }
            // `Key::Unknown` keys (media buttons etc.) have no
            // crossterm equivalent and do not actuate commands; drop.
        }
        FrontendEvent::Mouse(pmacs_mouse) => {
            let ct_mouse = mouse_to_crossterm(&pmacs_mouse);
            editor.dispatch_mouse(pmacs_mouse.frontend_id, ct_mouse, *term_size);
        }
        FrontendEvent::Resize { size, .. } => {
            render_state.resize(size);
            *term_size = size;
        }
        FrontendEvent::Paste { .. }
        | FrontendEvent::FocusGained(_)
        | FrontendEvent::FocusLost(_) => {
            // v0.1: silently ignored. Future work surfaces these
            // through Lua hooks (paste-text-fn, focus-changed-hook).
        }
        FrontendEvent::Detach(_) => {
            // Caller handles Detach as a control event before reaching
            // here.
            unreachable!("Detach is handled by run_per_attach directly");
        }
    }
}

// `stream: UnixStream` is owned so the fd closes when the reader
// thread returns; this is the lifecycle we want, not what
// `needless_pass_by_value` suggests.
#[allow(clippy::needless_pass_by_value)]
fn run_reader(mut stream: UnixStream, tx: mpsc::Sender<FrontendEvent>) {
    loop {
        match read_message::<FrontendEvent>(&mut stream) {
            Ok(ev) => {
                if tx.send(ev).is_err() {
                    return;
                }
            }
            // Any transport error ends the reader. The main thread
            // sees the channel close and exits the per-attach loop.
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_state_starts_frontend_id_at_two() {
        let s = DaemonState::new(None);
        let a = s.next_frontend_id.fetch_add(1, Ordering::SeqCst);
        let b = s.next_frontend_id.fetch_add(1, Ordering::SeqCst);
        assert_eq!(a, 2);
        assert_eq!(b, 3);
        assert!(!s.attached.load(Ordering::SeqCst));
    }

    #[test]
    fn build_identity_includes_version_and_uptime() {
        let s = DaemonState::new(Some("research".into()));
        thread::sleep(Duration::from_millis(20));
        let id = s.build_identity();
        assert_eq!(id.pmacs_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(id.instance_name.as_deref(), Some("research"));
        // uptime_secs is whole seconds; 20 ms might round to 0,
        // which is the expected lower bound (uptime never negative).
        // No upper bound assertion — sleep precision is variable.
    }

    #[test]
    fn build_identity_default_instance_name_is_none() {
        let s = DaemonState::new(None);
        let id = s.build_identity();
        assert!(id.instance_name.is_none());
    }

    #[test]
    fn build_identity_working_directory_is_utf8() {
        let s = DaemonState::new(None);
        let id = s.build_identity();
        // Just verify it's set (or empty if cwd was non-UTF-8).
        // Test environments are UTF-8, so this should be non-empty.
        assert!(!id.working_directory.is_empty());
    }
}
