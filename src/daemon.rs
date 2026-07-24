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

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::net::Shutdown;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
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
use crate::presence::{PresenceSnapshot, SessionRegistry};
use crate::protocol::crossterm_translate::{key_to_crossterm, mouse_to_crossterm};
use crate::protocol::{
    AttachRequest, FrontendEvent, FrontendId, GoodbyeReason, Hello, InitialTarget,
    InitialTargetResult, InstanceCapabilities, InstanceIdentity, InstanceMessage, InstanceSignal,
    MAX_INITIAL_TARGET_ERROR_BYTES, MAX_INITIAL_TARGET_PATH_BYTES, PROTOCOL_VERSION, PointerKind,
    SelectionSnapshot, SessionBootstrapRequest,
};
use crate::socket_path::{SocketPathError, ensure_runtime_subdir};
use crate::transport::{read_message, write_message};

/// Shared debug switch with the SSH attach path. When set before
/// daemon startup, emits stderr breadcrumbs for accept/handshake
/// progress without changing the wire protocol.
const PMACS_ATTACH_DEBUG: &str = "PMACS_ATTACH_DEBUG";

fn daemon_debug_enabled() -> bool {
    std::env::var_os(PMACS_ATTACH_DEBUG).is_some_and(|v| !v.is_empty() && v != "0")
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_uid(stream: &UnixStream) -> Option<u32> {
    nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials)
        .ok()
        .map(|cred| cred.uid())
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn peer_uid(_stream: &UnixStream) -> Option<u32> {
    None
}

fn daemon_debug(msg: impl AsRef<str>) {
    if daemon_debug_enabled() {
        eprintln!("pmacs daemon debug: {}", msg.as_ref());
    }
}

/// T M10.8 — events the dispatcher thread processes.
///
/// The dispatcher is the single thread that owns the editor; all
/// inputs from attached frontends arrive via this channel. Accept
/// thread + per-attach threads push variants here; dispatcher
/// consumes them in FIFO order.
enum DispatcherEvent {
    /// A new connection finished handshake successfully and is now
    /// ready to participate. The dispatcher registers the session,
    /// allocates a per-frontend `RenderState`, and stores the
    /// write-half of the stream so subsequent frames + broadcasts
    /// can be sent to this frontend.
    SessionEstablished {
        frontend_id: FrontendId,
        session_state: crate::presence::SessionState,
        initial_size: CellSize,
        /// Validated protocol-v20 semantic bootstrap target, if requested.
        initial_target: Option<InitialTarget>,
        /// Write-half of the per-attach stream. The dispatcher owns
        /// this end; the per-attach reader thread keeps the
        /// read-half via `try_clone`.
        write_stream: UnixStream,
    },
    /// An attached frontend dispatched an event (key, mouse, resize,
    /// etc.). The dispatcher applies it to the editor and renders
    /// the resulting frame(s).
    FrontendEvent {
        source: FrontendId,
        event: FrontendEvent,
    },
    /// An attached frontend's connection closed (EOF, decode error,
    /// or explicit `FrontendEvent::Detach`). The dispatcher
    /// unregisters the session and drops the write stream.
    SessionDetached { frontend_id: FrontendId },
}

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
    /// T M10.8 Day 4 — count of currently-attached **non-multi**
    /// sessions per Q5. A non-multi attach is rejected with
    /// `Goodbye(AlreadyAttached)` iff this count is `> 0` at attach
    /// time. Multi sessions don't touch this counter and aren't
    /// gated by it. CAS-incremented during handshake; decremented
    /// via [`NonMultiSlotGuard`] on per-attach exit (normal or
    /// panic).
    ///
    /// Semantically degenerate (0 or 1 in v1.0) but kept as
    /// `AtomicU64` for future-proofing against v0.2+ scenarios that
    /// might allow multiple non-multi sessions.
    non_multi_session_count: AtomicU64,
    /// T M10.9 — per-uid color slot registry. Maps Unix uid to
    /// color palette index (0..[`crate::overlay_color::PALETTE_LEN`]).
    /// First attach from a new uid gets the next available slot;
    /// subsequent attaches from the same uid reuse that slot.
    /// Stable across reconnect within a daemon-process lifetime —
    /// satisfies the spec's "stable across reconnect (within a
    /// session)" criterion for the same-uid case.
    ///
    /// Cross-uid color collisions (two users sharing a uid → same
    /// color) are v0.2+ user-identity refinement. Two distinct uids
    /// with hash collision on the palette also share a color, which
    /// is the same shape as cross-uid collapse.
    color_registry: std::sync::Mutex<HashMap<u32, u8>>,
    /// T M10.10 Day 4 — test-only latency injection for
    /// `CellDelta` emission. Read once at daemon startup from
    /// `PMACS_INSTANCE_LATENCY_MS`. When `> 0`, the dispatcher
    /// sleeps this many milliseconds before each `CellDelta` write
    /// to a stream, simulating slow daemon→frontend transport.
    ///
    /// Used exclusively by the criterion 1 ("less than one frame
    /// regardless of instance latency") acceptance tests and the
    /// V0.2-PREREQUISITES.md baseline measurements. Production
    /// daemons leave this at 0; tests set the env var via
    /// `TestDaemon::spawn_with_env`.
    ///
    /// **Scope: dispatcher-wide, not per-frontend.** The sleep
    /// fires in the dispatcher loop's per-tick render write path.
    /// Multi-frontend tests at injected latency conflate frontends
    /// (all see the same delay). Criterion 1's test uses a single
    /// replica frontend so this conflation doesn't affect the
    /// signal. v0.2+ work on per-frontend latency injection would
    /// move the sleep into a per-frontend writer thread.
    injected_render_latency_ms: u64,
    /// T M10.11 Q6/Q8 — test-only jitter on top of the fixed latency.
    /// Read once at startup from `PMACS_INSTANCE_LATENCY_JITTER_MS`.
    /// When `> 0`, each `CellDelta` write is delayed by
    /// `injected_render_latency_ms + rand(0..jitter)` instead of the
    /// fixed value, simulating variable network latency. No actual
    /// drops — TCP/UDS never drops application bytes and loro has no
    /// dropped-op recovery (Tension B / Q6: "packet loss" is
    /// interpreted as latency variation only). Production leaves
    /// this 0.
    injected_render_latency_jitter_ms: u64,
    /// T M10.11 Q8 — seed for the jitter PRNG. Read from
    /// `PMACS_INSTANCE_LATENCY_JITTER_SEED` (default `0xC0FFEE`,
    /// matching M10.1's microbench-seed convention) so
    /// convergence-under-jitter scenarios are deterministically
    /// reproducible. A flake's seed is the one to re-run.
    jitter_seed: u64,
}

/// T M10.11 Q8 — `SplitMix64` PRNG for deterministic jitter.
///
/// Chosen because it is six lines of pure wrapping arithmetic: no
/// `unsafe`, no new dependency (the project is `forbid(unsafe_code)`
/// and the `rand` crate would be a production dep pulled in for a
/// test-only seam). Statistically adequate for "uniform-ish delay in
/// `[0, jitter)`"; the jitter scenario asserts CRDT convergence
/// regardless of delay ordering, not a distribution property, so PRNG
/// quality is not load-bearing — only reproducibility (seed) is.
struct SplitMix64(u64);

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// T M10.11 F2 — apply jitter delay before a wire write.
///
/// One *mechanism* (this fn), called at two *sites*: the `CrdtOp`
/// broadcast write (criterion 3 — the CRDT-convergence path) and the
/// render-message `CellDelta` write (criterion 1 — the render-latency
/// path). **Q6's original "no new injection seams; one place"
/// commitment was wrong** and Finding 5's first resolution compounded
/// the error: criterion 1 and criterion 3 ride *different message
/// paths* (render output vs `broadcast_crdt_op`), so they
/// structurally require two call sites. Honest framing: one jitter
/// mechanism, two call sites because there are two paths — not "one
/// seam" (there isn't) and not "widen one loop's match" (Finding 5's
/// flawed fix, which matched `CrdtOp` in the render loop that never
/// carries broadcast `CrdtOp`s).
///
/// No-op when `jitter_ms == 0`. Tension-B holds: the write is
/// *delayed*, never dropped.
fn maybe_jitter_sleep(jitter_ms: u64, base_ms: u64, rng: &mut SplitMix64) {
    if jitter_ms == 0 {
        return;
    }
    let delay_ms = base_ms + (rng.next_u64() % jitter_ms);
    if delay_ms > 0 {
        thread::sleep(Duration::from_millis(delay_ms));
    }
}

/// T M10.8 Day 4 — RAII guard for the non-multi-session slot.
///
/// Acquired via [`NonMultiSlotGuard::try_acquire`] at per-attach
/// handshake time; releases the slot on drop (whether the
/// per-attach thread exits normally or panics). Holds an `Arc` so
/// the guard doesn't borrow from a reference whose lifetime might
/// not outlive the slot.
struct NonMultiSlotGuard {
    daemon_state: Arc<DaemonState>,
}

impl NonMultiSlotGuard {
    /// Try to acquire the single non-multi session slot. Returns
    /// `None` if another non-multi session is already attached.
    fn try_acquire(daemon_state: Arc<DaemonState>) -> Option<Self> {
        daemon_state
            .non_multi_session_count
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self { daemon_state })
    }
}

impl Drop for NonMultiSlotGuard {
    fn drop(&mut self) {
        self.daemon_state
            .non_multi_session_count
            .fetch_sub(1, Ordering::SeqCst);
    }
}

impl DaemonState {
    fn new(instance_name: Option<String>) -> Self {
        // T M10.10 Day 4 — read the latency-injection env once at
        // startup. Production deployments don't set this; tests
        // (`TestDaemon::spawn_with_env`) set it for criterion 1
        // verification.
        let injected_render_latency_ms: u64 = std::env::var("PMACS_INSTANCE_LATENCY_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        // T M10.11 Q6/Q8 — jitter magnitude + PRNG seed, same
        // read-once-at-startup discipline. Production leaves both
        // unset (jitter 0; seed defaults but unused when jitter 0).
        let injected_render_latency_jitter_ms: u64 =
            std::env::var("PMACS_INSTANCE_LATENCY_JITTER_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        let jitter_seed: u64 = std::env::var("PMACS_INSTANCE_LATENCY_JITTER_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0x00C0_FFEE);
        Self {
            instance_name,
            started: Instant::now(),
            // FrontendId(1) is reserved for FrontendId::LOCAL (the
            // in-process TUI). Daemon-attached frontends start at 2.
            next_frontend_id: AtomicU64::new(2),
            non_multi_session_count: AtomicU64::new(0),
            color_registry: std::sync::Mutex::new(HashMap::new()),
            injected_render_latency_ms,
            injected_render_latency_jitter_ms,
            jitter_seed,
        }
    }

    /// T M10.10 Day 4 — current injected-latency value for `CellDelta`
    /// emission. Returns 0 in production.
    fn injected_render_latency_ms(&self) -> u64 {
        self.injected_render_latency_ms
    }

    /// T M10.11 Q6/Q8 — jitter magnitude (0 in production).
    fn injected_render_latency_jitter_ms(&self) -> u64 {
        self.injected_render_latency_jitter_ms
    }

    /// T M10.11 Q8 — jitter PRNG seed (default `0xC0FFEE`).
    fn jitter_seed(&self) -> u64 {
        self.jitter_seed
    }

    /// T M10.9 — look up or assign a color slot for the given uid.
    ///
    /// Same uid across reconnect → same slot (the spec's
    /// "stable across reconnect within a session" criterion).
    /// New uid → next free slot, wrapping around the palette length.
    fn color_slot_for_uid(&self, uid: u32) -> u8 {
        use crate::overlay_color::PALETTE_LEN;
        let mut registry = self
            .color_registry
            .lock()
            .expect("color_registry mutex poisoned");
        if let Some(&slot) = registry.get(&uid) {
            slot
        } else {
            // Next slot = number of entries so far, modulo palette length.
            let slot = u8::try_from(registry.len() % PALETTE_LEN).unwrap_or(0);
            registry.insert(uid, slot);
            slot
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
    // The editor outlives any single attachment; constructed once on
    // the dispatcher thread and used until daemon shutdown.
    let mut editor = EditorState::new();
    // Real session: wire up on-disk persistence (history + pmacs.state).
    editor.install_state_dirs();
    // Mark this process a daemon so `pmacs.session.desktop_mode` keeps
    // desktop save/restore local-only in v1 (Q#DS9): the daemon has a
    // layout per attached frontend and none at construction.
    editor
        .lua_host
        .lua()
        .set_app_data(crate::lua_bindings::DaemonMode);
    // Mirror the daemon's `--socket NAME` and start time into the
    // editor's `LocalInstanceInfo` so `pmacs.instance.identity()`
    // (T M5.6f) reports the same identity the daemon hands back over
    // its Hello.
    editor
        .lua_host
        .set_instance_name(daemon_state.instance_name());
    editor.lua_host.set_instance_started(daemon_state.started());

    eprintln!(
        "pmacs: daemon listening on {} (pid {})",
        socket_path.display(),
        std::process::id(),
    );

    // T M10.8 — dispatcher thread topology.
    //
    // The current thread becomes the dispatcher (owns the editor;
    // single-threaded access to all editor state). A spawned accept
    // thread runs `accept_loop`, calling `listener.accept()` and
    // spawning a per-attach thread for each accepted connection.
    //
    // The per-attach thread does the handshake (Hello, AttachRequest,
    // version + capability checks), then sends `SessionEstablished`
    // to the dispatcher and becomes a reader thread for that
    // connection's stream.
    //
    // Dispatcher channel: all attach threads push `DispatcherEvent`
    // variants; dispatcher consumes in FIFO order.
    let (dispatcher_tx, dispatcher_rx) = mpsc::channel::<DispatcherEvent>();
    let accept_handle = {
        let daemon_state = Arc::clone(&daemon_state);
        let shutdown = Arc::clone(&shutdown);
        let tx = dispatcher_tx.clone();
        thread::spawn(move || accept_loop(listener, &daemon_state, tx, &shutdown))
    };

    dispatcher_loop(
        dispatcher_rx,
        &mut editor,
        &shutdown,
        daemon_state.injected_render_latency_ms(),
        daemon_state.injected_render_latency_jitter_ms(),
        daemon_state.jitter_seed(),
    )?;

    // Dispatcher exited (shutdown or quit). Wake the accept thread
    // by closing the channel from our side; the accept thread checks
    // the shutdown flag between accepts and exits accordingly.
    drop(dispatcher_tx);
    let _ = accept_handle.join();
    cleanup(&socket_path, lock);
    eprintln!("pmacs: daemon stopped");
    Ok(())
}

/// T M10.8 — accept thread. Spawns a per-attach thread for each new
/// connection. Runs on its own OS thread parallel to the dispatcher.
///
/// `listener` is taken by value because the accept thread owns it
/// until the daemon shuts down; the file descriptor closes when the
/// thread exits.
#[allow(clippy::needless_pass_by_value)]
fn accept_loop(
    listener: UnixListener,
    daemon_state: &Arc<DaemonState>,
    dispatcher_tx: mpsc::Sender<DispatcherEvent>,
    shutdown: &Arc<AtomicBool>,
) -> Result<(), DaemonError> {
    daemon_debug("accept loop started");
    while !shutdown.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                // macOS can inherit O_NONBLOCK from the listener onto
                // accepted Unix streams. The per-attach reader loop
                // expects blocking reads; a nonblocking stream would
                // turn "no frontend event yet" into WouldBlock, which
                // looks like an immediate detach before the first frame.
                stream.set_nonblocking(false)?;
                daemon_debug("accepted frontend socket; spawning per-attach thread");
                let daemon_state = Arc::clone(daemon_state);
                let tx = dispatcher_tx.clone();
                thread::spawn(move || per_attach_thread(stream, daemon_state, tx));
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_POLL_INTERVAL);
            }
            Err(e) => return Err(DaemonError::Io(e)),
        }
    }
    Ok(())
}

fn cleanup(socket_path: &Path, lock: LockHandle) {
    let _ = std::fs::remove_file(socket_path);
    let _ = lock.release();
}

/// T M10.8 Day 4 — read `PMACS_INSTANCE_*` env vars (if set) to
/// override the instance's advertised capabilities. Used by test
/// infrastructure that needs the daemon to advertise non-default
/// capabilities (e.g., M10.7's mismatch test rewrite, which needs
/// the daemon to advertise `multi_frontend: false` so a
/// frontend declaring `true` hits the mismatch path).
///
/// Recognized env vars (each accepts `0`/`false` to disable;
/// anything else / absent → default `true`):
/// - `PMACS_INSTANCE_MULTI_FRONTEND`
/// - `PMACS_INSTANCE_CRDT_REPLICA`
/// - `PMACS_INSTANCE_SEMANTIC_RENDER` (T M11.1; default `false`
///   until the M11.2 projection seam lands, so this env var is the
///   only way to advertise the bit for negotiation tests)
///
/// Production daemons don't set these; tests do.
fn instance_capabilities_with_env_override() -> InstanceCapabilities {
    fn env_bool(key: &str, default: bool) -> bool {
        match std::env::var(key).ok().as_deref() {
            Some("0" | "false" | "FALSE" | "False") => false,
            Some(_) | None => default,
        }
    }
    let defaults = InstanceCapabilities::default();
    InstanceCapabilities {
        multi_frontend: env_bool("PMACS_INSTANCE_MULTI_FRONTEND", defaults.multi_frontend),
        crdt_replica: env_bool("PMACS_INSTANCE_CRDT_REPLICA", defaults.crdt_replica),
        semantic_render: env_bool("PMACS_INSTANCE_SEMANTIC_RENDER", defaults.semantic_render),
    }
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

fn bounded_initial_target_error(mut message: String) -> String {
    if message.len() <= MAX_INITIAL_TARGET_ERROR_BYTES {
        return message;
    }
    let mut end = MAX_INITIAL_TARGET_ERROR_BYTES;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message.truncate(end);
    message
}

fn send_initial_target_failure(stream: &mut UnixStream, message: impl Into<String>) {
    let result = InitialTargetResult::Failed {
        message: bounded_initial_target_error(message.into()),
    };
    let _ = write_message(stream, &InstanceMessage::InitialTargetResult(result));
    let _ = stream.shutdown(Shutdown::Both);
}

fn validate_initial_target(target: &InitialTarget) -> Result<(), String> {
    if target.cwd.is_empty() {
        return Err("initial target cwd is empty".to_owned());
    }
    if target.path.is_empty() {
        return Err("initial target path is empty".to_owned());
    }
    if target.cwd.len() > MAX_INITIAL_TARGET_PATH_BYTES {
        return Err("initial target cwd exceeds 32 KiB".to_owned());
    }
    if target.path.len() > MAX_INITIAL_TARGET_PATH_BYTES {
        return Err("initial target path exceeds 32 KiB".to_owned());
    }
    if target.cwd.contains(&0) || target.path.contains(&0) {
        return Err("initial target path contains an embedded NUL".to_owned());
    }
    if !Path::new(OsStr::from_bytes(&target.cwd)).is_absolute() {
        return Err("initial target cwd is not absolute".to_owned());
    }
    Ok(())
}

/// T M10.8 — per-attach thread. Runs handshake on a fresh thread for
/// each accepted connection; on success, sends `SessionEstablished`
/// to the dispatcher and transitions to reader behavior on the same
/// thread (no other initialization between `SessionEstablished` and
/// the reader loop — any added work would delay first-event
/// processing).
///
/// On handshake failure (version mismatch, capability mismatch, I/O
/// error) the thread writes a `Goodbye` variant and exits without
/// notifying the dispatcher. The dispatcher never learns about
/// failed handshakes.
#[allow(
    clippy::needless_pass_by_value,
    clippy::too_many_lines,
    reason = "the ordered handshake and bootstrap read stay on one per-connection thread"
)]
fn per_attach_thread(
    mut stream: UnixStream,
    daemon_state: Arc<DaemonState>,
    dispatcher_tx: mpsc::Sender<DispatcherEvent>,
) {
    daemon_debug("per-attach thread started");
    let frontend_id = FrontendId(daemon_state.next_frontend_id.fetch_add(1, Ordering::SeqCst));
    daemon_debug(format!("assigned {frontend_id:?}; preparing Hello"));

    // Send Hello immediately on accept. The instance capabilities
    // advertised here (and used for negotiation below) come from the
    // env-var override helper so test infrastructure can drive the
    // mismatch path without changing the default.
    let instance_caps_for_hello = instance_capabilities_with_env_override();
    let hello = Hello {
        protocol_version: PROTOCOL_VERSION,
        assigned_frontend_id: frontend_id,
        instance_identity: daemon_state.build_identity(),
        instance_capabilities: instance_caps_for_hello.clone(),
    };
    if let Err(e) = write_message(&mut stream, &hello) {
        eprintln!("pmacs: send Hello failed: {e}");
        return;
    }
    daemon_debug(format!("sent Hello to {frontend_id:?}"));

    // Read AttachRequest.
    daemon_debug(format!("waiting for AttachRequest from {frontend_id:?}"));
    let req: AttachRequest = match read_message(&mut stream) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("pmacs: read AttachRequest failed: {e}");
            return;
        }
    };
    daemon_debug(format!("received AttachRequest from {frontend_id:?}"));

    // T M10.5 version check.
    if !crate::protocol::is_supported_protocol_version(req.protocol_version) {
        let _ = write_message(
            &mut stream,
            &InstanceMessage::Goodbye(GoodbyeReason::VersionMismatch {
                server: PROTOCOL_VERSION,
                client: req.protocol_version,
            }),
        );
        return;
    }

    // T M10.7 capability negotiation. T M10.8 Day 4: the instance
    // defaults to advertising `multi_frontend: true` and
    // `crdt_replica: true` (M10.10 renamed from `crdt_ops`).
    // Env vars override at daemon startup for test
    // infrastructure that needs to exercise the mismatch path
    // (the M10.7 mismatch test's daemon-end-to-end rewrite).
    // We re-use the caps already computed for Hello so the
    // negotiation sees the same advertised values.
    let instance_caps = instance_caps_for_hello;
    let negotiated_caps =
        match crate::protocol::negotiate_capabilities(&req.frontend_capabilities, &instance_caps) {
            Ok(caps) => caps,
            Err(reason) => {
                let _ = write_message(&mut stream, &InstanceMessage::Goodbye(reason));
                return;
            }
        };

    // Q#GT4 — v20 semantic sessions send one bootstrap envelope after
    // AttachRequest. Legacy and non-semantic sessions retain the exact
    // two-message handshake and therefore must not be read here.
    let initial_target = if req.protocol_version >= 20 && negotiated_caps.semantic_render {
        let bootstrap: SessionBootstrapRequest = match read_message(&mut stream) {
            Ok(bootstrap) => bootstrap,
            Err(e) => {
                eprintln!("pmacs: read SessionBootstrapRequest failed: {e}");
                return;
            }
        };
        if let Some(target) = bootstrap.initial_target.as_ref()
            && let Err(message) = validate_initial_target(target)
        {
            send_initial_target_failure(&mut stream, message);
            return;
        }
        bootstrap.initial_target
    } else {
        None
    };

    // T M10.8 Day 4 — Q5 non-multi-session admission control.
    let _non_multi_guard = if negotiated_caps.multi_frontend {
        None
    } else if let Some(guard) = NonMultiSlotGuard::try_acquire(Arc::clone(&daemon_state)) {
        Some(guard)
    } else {
        let _ = write_message(
            &mut stream,
            &InstanceMessage::Goodbye(GoodbyeReason::AlreadyAttached),
        );
        return;
    };

    // T M10.9 — color slot assignment via SO_PEERCRED. The connecting
    // peer's Unix uid is the stable identifier; same uid across
    // reconnect → same color slot. If SO_PEERCRED fails (e.g.,
    // non-Unix peer, kernel API unavailable), fall back to a
    // per-FrontendId slot (degrades to per-connection stability).
    let color_slot = if let Some(uid) = peer_uid(&stream) {
        daemon_state.color_slot_for_uid(uid)
    } else {
        // Fallback: use frontend_id-based slot; per-connection
        // stability only (no cross-reconnect within session).
        u8::try_from(frontend_id.0 % (crate::overlay_color::PALETTE_LEN as u64)).unwrap_or(0)
    };

    let session_state =
        crate::presence::SessionState::new(req.protocol_version, negotiated_caps, color_slot);

    // Hand the write-half to the dispatcher; keep a read-half for
    // this thread's reader loop. **Reader loop starts immediately
    // after the SessionEstablished send below; any initialization
    // needed must happen before that send. A future contributor
    // adding "let me also do X before reading" would delay
    // first-event processing.**
    let write_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("pmacs: try_clone stream for {frontend_id:?} failed: {e}");
            return;
        }
    };

    if dispatcher_tx
        .send(DispatcherEvent::SessionEstablished {
            frontend_id,
            session_state,
            initial_size: req.initial_size,
            initial_target,
            write_stream,
        })
        .is_err()
    {
        // Dispatcher channel closed (daemon shutting down).
        return;
    }

    // Reader loop: read FrontendEvents from the stream, push to
    // dispatcher tagged with this frontend_id. Any error (EOF,
    // decode error, broken pipe) ends the loop; final
    // `SessionDetached` lets the dispatcher clean up.
    let mut read_stream = stream;
    while let Ok(event) = read_message::<FrontendEvent>(&mut read_stream) {
        if dispatcher_tx
            .send(DispatcherEvent::FrontendEvent {
                source: frontend_id,
                event,
            })
            .is_err()
        {
            break;
        }
    }

    // Notify dispatcher of detach. Best-effort: if the channel is
    // closed (daemon shut down before our reader exited), the send
    // returns Err and we just exit.
    let _ = dispatcher_tx.send(DispatcherEvent::SessionDetached { frontend_id });
}

/// Belt-and-braces write-loop gate for the additive protocol-v18
/// statusline variant. The producer has its own callback/evaluation gate;
/// this filter independently prevents an unknown discriminant reaching an
/// older peer even if a message is injected into the frame vector.
fn peer_accepts_statusline_message(protocol_version: u32, message: &InstanceMessage) -> bool {
    protocol_version >= 18 || !matches!(message, InstanceMessage::StatuslineSegments { .. })
}

/// Whether a session negotiated the v19 wire, and may therefore drive
/// terminal state inbound.
///
/// The outbound `TerminalFrame` is gated twice — in the producer and in
/// the write loop — and the inbound direction is now symmetric. A
/// pre-v19 peer cannot construct these variants at all (its enum lacks
/// them), so this only ever refuses a hand-rolled client; the a32
/// forgery tests already prove such an event can reach nothing but the
/// sender's own authenticated active view. It is defense in depth, and
/// it makes "gated in both directions" true of the code rather than
/// only of the frontends we ship.
fn peer_declared_terminal_support(
    session_registry: &SessionRegistry,
    frontend_id: FrontendId,
) -> bool {
    session_registry
        .session_state(frontend_id)
        .is_some_and(|state| state.negotiated_protocol_version >= 19)
}

/// Whether a session can **render** a side window (bottom-panel arc,
/// Q#BP13).
///
/// Grid sessions paint the whole cell grid the daemon composes, so a side
/// window is just another leaf for them. A semantic session needs the
/// Stage 2 `PanelFrame` band, which does not exist yet — so Stage 1
/// answers `false` for every semantic peer, whatever it declares. No
/// client-asserted standalone boolean is trusted: the answer is derived
/// from the daemon's own negotiated state, and Stage 2 turns the version
/// arm on (`semantic_render && negotiated_protocol_version >=
/// PANEL_MIN_VERSION`).
fn peer_declared_panel_support(session_state: crate::presence::SessionState) -> bool {
    !session_state.negotiated_capabilities.semantic_render
}

/// The same belt-and-braces write-loop gate for the additive
/// protocol-v19 terminal frame. The semantic producer skips construction
/// for an older peer; this filter independently prevents an unknown
/// discriminant reaching one, so neither gate alone is load-bearing.
fn peer_accepts_terminal_message(protocol_version: u32, message: &InstanceMessage) -> bool {
    protocol_version >= 19 || !matches!(message, InstanceMessage::TerminalFrame(_))
}

/// T M10.8 — dispatcher loop. The single thread that owns the editor.
///
/// All attached frontends' inputs arrive via the `dispatcher_rx`
/// channel as `DispatcherEvent` variants. Per-attach reader threads
/// (spawned by the accept thread on each new connection) push events
/// here. The dispatcher consumes them in FIFO order, mutates the
/// editor, and per-tick:
///
/// 1. Renders a frame for each attached frontend (per-frontend
///    `RenderState`, each rendered against its own view).
/// 2. Sweeps the `SessionRegistry` for presence broadcasts;
///    routes them to per-recipient streams.
/// 3. Writes outgoing messages to each frontend's write stream
///    (synchronous — M10.8 Day 3 doesn't have per-frontend writer
///    threads; v0.3 may add them if N attachments grow).
/// 4. Ticks async / processes / LSP.
///
/// Exits when the editor's `quit` flag is set, the `shutdown` flag
/// is set, or all per-attach senders have disconnected.
///
/// Return value is `Result` for symmetry with other daemon entry
/// points; the function does not propagate errors today, but a
/// future failure mode (e.g., catastrophic editor state corruption)
/// would surface here.
// M10.10 grew this function with per-tick CursorByte emit + lazy
// CRDT upgrade + latency injection on top of M10.8/M10.9's
// dispatcher loop. The 121-line size is cohesive — the loop body
// coordinates render + presence sweep + CRDT broadcast + shutdown
// against one stack frame's borrow scope. Splitting would require
// either passing many `&mut` parameters between helpers or moving
// state behind RefCells. Defer to v0.2+ refactor if growth continues.
#[allow(
    clippy::unnecessary_wraps,
    clippy::needless_pass_by_value,
    clippy::too_many_lines
)]
fn dispatcher_loop(
    dispatcher_rx: mpsc::Receiver<DispatcherEvent>,
    editor: &mut EditorState,
    shutdown: &Arc<AtomicBool>,
    injected_render_latency_ms: u64,
    injected_render_latency_jitter_ms: u64,
    jitter_seed: u64,
) -> Result<(), DaemonError> {
    // Per-frontend dispatcher state.
    let mut render_states: HashMap<FrontendId, RenderState> = HashMap::new();
    // T M11.2 — parallel to `render_states`, but for `semantic_render`
    // sessions: the dispatcher selects the projection *per session*,
    // so a frontend has exactly one of a `RenderState` (grid) or a
    // `SemanticRenderState` (layout-local), never both. A grid and a
    // semantic frontend can attach to the same buffer simultaneously.
    let mut semantic_states: HashMap<FrontendId, crate::semantic_render::SemanticRenderState> =
        HashMap::new();
    let mut streams: HashMap<FrontendId, UnixStream> = HashMap::new();
    let mut term_sizes: HashMap<FrontendId, CellSize> = HashMap::new();
    // T M11.6 — last `DispatchIdle` value broadcast per `crdt_replica`
    // frontend. Absence means "never sent" — the first tick after
    // attach emits an initial `DispatchIdle` so the frontend starts
    // from a known idle state (its default is pessimistic-`false`).
    let mut last_dispatch_idle_sent: HashMap<FrontendId, bool> = HashMap::new();
    // Arc 1b — the buffer each replica frontend last received a
    // `BufferSnapshot` for via the active-buffer-follow path. Absence
    // means "never sent": the first tick after attach ships the
    // frontend its own active buffer, which also repairs the
    // attach-time last-snapshot-wins ambiguity (the initial
    // `send_buffer_snapshots` sweep sends every buffer; the display
    // follows whichever arrived last, not necessarily the active one).
    // Declared for both flavors (the follow path is crdt-gated; the
    // detach cleanup isn't).
    let mut last_active_buffer_sent: HashMap<FrontendId, crate::buffer::BufferId> = HashMap::new();
    // Active-terminal BEL delivery baseline. Switching away forgets the
    // terminal so historical bells are never replayed on later activation.
    let mut terminal_bell_baselines: HashMap<FrontendId, (crate::buffer::BufferId, u64)> =
        HashMap::new();
    let mut session_registry = SessionRegistry::new();
    // T M10.11 Q8 — jitter PRNG, seeded once so the
    // convergence-under-jitter scenario is deterministically
    // reproducible. Mutated across the loop; one stream of delays
    // for the whole dispatcher (jitter is dispatcher-wide, matching
    // the fixed-latency seam's scope per the field docs).
    let mut jitter_rng = SplitMix64::new(jitter_seed);

    loop {
        // Per-tick render + presence sweep for each attached
        // frontend. T M10.8 — temporarily flip `active_frontend` to
        // the frontend being rendered so its FrontendView is the one
        // `active_window()` returns. Restored after the render-pass
        // loop to the last-dispatched value (Q11: tick-driven render
        // doesn't update active_frontend in the user-driving sense).
        let last_dispatched = editor.core.borrow().active_frontend;
        // Union of grid + semantic sessions — each fid is in exactly
        // one of the two maps (projection selected per session).
        let attached_fids: Vec<FrontendId> = render_states
            .keys()
            .chain(semantic_states.keys())
            .copied()
            .collect();

        // T M10.10 post-audit-round-3 F18 — drain + broadcast pending
        // CRDT ops **before** the render pass. Otherwise frontends
        // receive `CellDelta` + `CursorByte` (showing the edit
        // visually + the new cursor position) before the `CrdtOp`
        // that updates their `BufferMirror`'s rope state — a fast
        // next keystroke would run optimistic logic against stale
        // mirror content with the new cursor position.
        //
        // F16 — `CrdtOpOrigin` controls sender exclusion:
        // `OptimisticReplica(fid)` excludes `fid` (already
        // locally-applied); `DaemonKey` excludes nobody (no
        // frontend has applied locally; the active frontend's
        // mirror must receive too).
        #[cfg(feature = "crdt")]
        {
            let pending_ops = std::mem::take(&mut editor.core.borrow_mut().pending_crdt_ops);
            for (origin, buffer_id, op) in pending_ops {
                let exclude = match origin {
                    crate::editor_core::CrdtOpOrigin::OptimisticReplica(fid) => Some(fid),
                    crate::editor_core::CrdtOpOrigin::DaemonKey => None,
                };
                let entries = session_registry.broadcast_crdt_op(exclude, buffer_id, op);
                for entry in entries {
                    if let Some(stream) = streams.get_mut(&entry.recipient) {
                        // T M10.11 F2 — THE criterion-3 jitter site.
                        // CRDT convergence is driven by these
                        // `broadcast_crdt_op` writes, NOT by render
                        // CellDeltas. Finding 5's first fix jittered
                        // the render loop (which never carries
                        // broadcast CrdtOps) and falsely claimed
                        // criterion 3 was exercised. This is the
                        // write that actually delivers ops to
                        // replicas; jittering here is what makes
                        // `m10_11_q8_convergence_under_jitter`
                        // genuinely test CRDT-under-jitter.
                        maybe_jitter_sleep(
                            injected_render_latency_jitter_ms,
                            injected_render_latency_ms,
                            &mut jitter_rng,
                        );
                        let _ = write_message(stream, &entry.message);
                    }
                }
            }
        }
        // Non-CRDT build: `pending_crdt_ops` is empty (only the
        // CRDT-feature code paths push to it). Drop the take/iter
        // to keep the non-CRDT build free of unused imports.
        #[cfg(not(feature = "crdt"))]
        {
            // Defensive: empty the queue in case shared state was
            // populated through some path we haven't traced.
            let _ = std::mem::take(&mut editor.core.borrow_mut().pending_crdt_ops);
        }

        // Q#CM6 — outbound clipboard publish. A copy/cut queued the
        // region bytes for the originating frontend; deliver them as an
        // `InstanceSignal::Clipboard` (a v6-floor variant every peer
        // understands, so no version gate) and let the frontend write
        // the OS clipboard (OSC 52 / arboard). One-shot, like the CRDT
        // drain above.
        if let Some((fid, bytes)) = editor.core.borrow_mut().take_pending_clipboard()
            && let Some(stream) = streams.get_mut(&fid)
        {
            let _ = write_message(
                stream,
                &InstanceMessage::Signal(InstanceSignal::Clipboard(bytes)),
            );
        }

        for fid in &attached_fids {
            editor.core.borrow_mut().active_frontend = *fid;

            // T M10.10 Day 3 — lazy CRDT upgrade on active-buffer
            // access for replica frontends. Keeps the daemon
            // invariant "active buffer for a replica is CRDT-backed"
            // even when buffers are created mid-session
            // (post-SessionEstablished). The upgrade fires at most
            // once per buffer (idempotent via `is_crdt_backed()`
            // check). Documented in M10.10-FRAMING.md's composition-
            // consistency-check application section.
            #[cfg(feature = "crdt")]
            if session_registry
                .session_state(*fid)
                .is_some_and(|s| s.negotiated_capabilities.crdt_replica)
            {
                // F29 — when a mid-session upgrade occurs, push a
                // `BufferSnapshot` to every grid replica so its
                // `BufferMirror` gains an entry for the buffer. A
                // semantic replica receives it only when that replica
                // is displaying this buffer: applying a foreign-buffer
                // snapshot would switch the GPU window away from its
                // own active view.
                if let Some(upgraded) = ensure_active_buffer_crdt_backed(editor, *fid) {
                    broadcast_buffer_snapshot_to_replicas(
                        editor,
                        upgraded,
                        &session_registry,
                        &mut streams,
                        &mut semantic_states,
                    );
                    // The broadcast just delivered this buffer to this
                    // frontend too; record it so the follow check below
                    // doesn't send a duplicate on the same tick.
                    last_active_buffer_sent.insert(*fid, upgraded);
                }

                // Arc 1b — follow this frontend's active buffer. The
                // F29 push above only fires on the *upgrade* tick;
                // switching to an already-CRDT-backed buffer (a
                // panel's `q`, `find_or_open` of an open file, plain
                // `C-x b`) previously sent nothing, so a semantic
                // frontend kept rendering the old buffer while
                // daemon-side input targeted the new one — a
                // typing-into-a-buffer-you-can't-see hazard. Ship the
                // now-active buffer's snapshot to THIS frontend only
                // (its own view changed; nobody else's did).
                //
                // SEMANTIC sessions only: display-follows-snapshot is
                // a grid-less-frontend concept, and the GPU rebuilds
                // its replica wholesale on every snapshot. The grid
                // TUI renders via CellDelta and its `BufferMirror` is
                // init-once — a follow send there is a guaranteed
                // duplicate that errors ("already has a CRDT snapshot
                // applied") on every attach and every buffer switch
                // (the PR #94 round-2 startup regression).
                if session_registry
                    .session_state(*fid)
                    .is_some_and(|s| s.negotiated_capabilities.semantic_render)
                {
                    let active_now = {
                        let core = editor.core.borrow();
                        core.active_window_for(*fid).map(|w| w.buffer_id)
                    };
                    if let Some(active_now) = active_now
                        && last_active_buffer_sent.get(fid) != Some(&active_now)
                    {
                        send_buffer_snapshot_to_frontend(editor, active_now, *fid, &mut streams);
                        // PR #120 round 2 — the snapshot just wiped
                        // this frontend's buffer-scoped render state;
                        // the emission baselines must die with it or
                        // an unchanged-generation revisit (A → B → A)
                        // suppresses every re-send.
                        if let Some(sem) = semantic_states.get_mut(fid) {
                            sem.on_buffer_snapshot_sent(active_now);
                        }
                        last_active_buffer_sent.insert(*fid, active_now);
                    }
                }
            }
            #[cfg(not(feature = "crdt"))]
            {
                let _ = session_registry.session_state(*fid);
                let _ = ensure_active_buffer_crdt_backed(editor, *fid);
            }

            // Projection selected per session (T M11.2). A semantic
            // session produces `StyleSpans` scoped to its declared
            // viewport and NEVER `CellDelta` / grid `Cursor` (it lays
            // out locally); it still receives `CursorByte` below
            // (semantic implies `crdt_replica`) and participates in
            // presence. A grid session takes the M5.2 cell path.
            let messages = if let Some(sem) = semantic_states.get_mut(fid) {
                sem.render_frame(editor)
            } else {
                // T M10.9 — gather other-frontend presences for the
                // overlay paint. Reads `last_broadcast` (updated by
                // the sweep below); other-frontend snapshots lag by
                // at most one tick. Imperceptible at frame cadence.
                let other_presences = session_registry.other_presences_for(*fid);
                let render_size = render_states
                    .get(fid)
                    .expect("render_state present for attached grid fid")
                    .size();
                let terminal_snapshots = editor.prepare_terminal_views(*fid, render_size);
                let render_state = render_states
                    .get_mut(fid)
                    .expect("render_state present for attached grid fid");
                render_state.render_frame(editor, *fid, &terminal_snapshots, &other_presences)
            };

            // Vterm Stage 3 — a semantic frontend showing a terminal has
            // no document cursor: the identity buffer is empty, so a
            // `CursorByte` would describe byte 0 of a buffer with no
            // text and scroll the frontend's document view against it.
            let terminal_mode = semantic_states
                .get(fid)
                .is_some_and(crate::semantic_render::SemanticRenderState::in_terminal_mode);

            // T M10.6 per-frontend presence sweep. The snapshot is
            // computed from this frontend's view; the sweep then
            // produces broadcasts to OTHER multi-frontend recipients.
            //
            // Terminal mode does NOT skip this (PR #135 review finding 1).
            // `sweep` is diff-keyed on `last_broadcast`, so a skip can
            // only ever FREEZE a frontend's presence — it can never
            // retract it. Sweeping truthfully moves the presence into
            // the terminal identity buffer, which is what takes this
            // frontend's caret off every peer showing the document.
            //
            // Honest note on why the skip was not a live bug: the
            // buffer-follow above clears the terminal declaration when
            // it ships the snapshot, so `render_frame` reports
            // `terminal_active == false` on the tick a window first
            // shows a terminal, and the declaration cannot arrive until
            // a later tick (the frontend learns the buffer id FROM that
            // snapshot). Every entry into terminal mode is therefore
            // already preceded by one truthful sweep. The skip was
            // load-bearing on that ordering and bought nothing; removing
            // it makes "presence follows the frontend" structural
            // instead of a property of tick sequencing.
            //
            // The framing's "suppress presence for the terminal identity
            // buffer" is a RENDER rule — no peer overlay is painted
            // inside a terminal — which the GPU honors by not preparing
            // the decoration batch in terminal mode. It was never a
            // reason to stop telling peers where this frontend went.
            let snapshot = build_presence_snapshot(editor, *fid);
            let broadcasts = session_registry.sweep(&[(*fid, snapshot)]);

            // T M11.6 — DispatchIdle signal. `crdt_replica` frontends
            // gate their optimistic-apply path on this; we ship it
            // before the frame's other messages so a frontend that
            // wakes mid-tick sees the gate flip first. Diff-suppressed
            // — initial-after-attach (`last_dispatch_idle_sent` absent)
            // and value-change emissions only.
            let mut write_failed = false;
            if take_pending_terminal_bell(editor, *fid, &mut terminal_bell_baselines)
                && let Some(stream) = streams.get_mut(fid)
                && let Err(error) =
                    write_message(stream, &InstanceMessage::Signal(InstanceSignal::Bell))
            {
                eprintln!("pmacs: write terminal Bell for {fid:?} failed: {error}");
                write_failed = true;
            }
            if session_registry.session_state(*fid).is_some_and(|s| {
                // Filter on both the `crdt_replica` capability (only
                // optimistic-apply frontends care) and the negotiated
                // wire version (>= 4 means peer knows the variant).
                s.negotiated_capabilities.crdt_replica && s.negotiated_protocol_version >= 4
            }) && let Some(stream) = streams.get_mut(fid)
            {
                let idle_now = editor.dispatch_idle_for(*fid);
                if last_dispatch_idle_sent.get(fid) != Some(&idle_now) {
                    if let Err(e) =
                        write_message(stream, &InstanceMessage::DispatchIdle { idle: idle_now })
                    {
                        eprintln!("pmacs: write DispatchIdle for {fid:?} failed: {e}");
                        write_failed = true;
                    } else {
                        last_dispatch_idle_sent.insert(*fid, idle_now);
                    }
                }
            }

            // Write frame messages to this frontend's stream.
            if let Some(stream) = streams.get_mut(fid)
                && !write_failed
            {
                // Q#S1 — `StatusFacts` is a v8 variant; an older peer
                // would hard-error decoding it. Same per-session gate
                // shape as `DispatchIdle` (v4).
                // `StatusFacts` gained the transient status `message`
                // in v15 (encoding change to the variant), so the gate
                // moved 8 → 15: an older peer's band goes dark rather
                // than mis-decoding the wider shape (the v10
                // SearchPrompt / v14 LineNumbers precedent).
                let peer_knows_status_facts = session_registry
                    .session_state(*fid)
                    .is_some_and(|s| s.negotiated_protocol_version >= 15);
                // Q#SR5 / Q#RX6 — `SearchPrompt` gained regex/invalid
                // fields in v10 (encoding change); gate at >= 10 so a v9
                // peer is sent no SearchPrompt rather than the wider
                // shape it would mis-decode.
                let peer_knows_search_prompt = session_registry
                    .session_state(*fid)
                    .is_some_and(|s| s.negotiated_protocol_version >= 10);
                let peer_knows_menu_prompt = session_registry
                    .session_state(*fid)
                    .is_some_and(|s| s.negotiated_protocol_version >= 11);
                let peer_knows_minibuffer_prompt = session_registry
                    .session_state(*fid)
                    .is_some_and(|s| s.negotiated_protocol_version >= 12);
                // UX gutter — `LineNumbers` carries a `LineNumberMode` since
                // v14 (was `enabled: bool` in v13); a peer below 14 keeps
                // its gutter off rather than mis-decoding the wider shape.
                let peer_knows_line_numbers = session_registry
                    .session_state(*fid)
                    .is_some_and(|s| s.negotiated_protocol_version >= 14);
                // Arc 1a Q#C5 — CompletionPopup gated at v15; a v14 peer
                // still completes via the daemon-side session + key
                // round-trip, it just gets no GPU dropdown.
                let peer_knows_completion_popup = session_registry
                    .session_state(*fid)
                    .is_some_and(|s| s.negotiated_protocol_version >= 15);
                // Themes Q#TH7 — ThemeFacts gated at v16; a v15 peer's
                // chrome simply stays on its frontend defaults.
                let peer_knows_theme_facts = session_registry
                    .session_state(*fid)
                    .is_some_and(|s| s.negotiated_protocol_version >= 16);
                // Themes stage 2 Q#F4 — FontFacts gated at v17; a v16
                // peer simply keeps its built-in font.
                let peer_knows_font_facts = session_registry
                    .session_state(*fid)
                    .is_some_and(|s| s.negotiated_protocol_version >= 17);
                // Q#SL7 — independently gate the v18 statusline variant
                // even though the semantic producer also skips callbacks
                // and message construction for older peers.
                let negotiated_protocol_version = session_registry
                    .session_state(*fid)
                    .map_or(0, |s| s.negotiated_protocol_version);
                for msg in &messages {
                    if !peer_knows_status_facts
                        && matches!(msg, InstanceMessage::StatusFacts { .. })
                    {
                        continue;
                    }
                    if !peer_knows_search_prompt
                        && matches!(msg, InstanceMessage::SearchPrompt { .. })
                    {
                        continue;
                    }
                    // Q#CM1 — MenuPrompt gated at v11; a v10 peer keeps
                    // its decoration-only highlights and never opens a
                    // GPU menu, rather than mis-decoding the new variant.
                    if !peer_knows_menu_prompt && matches!(msg, InstanceMessage::MenuPrompt { .. })
                    {
                        continue;
                    }
                    // Q#MB1 — MinibufferPrompt gated at v12; a v11 peer
                    // simply can't render the GUI minibuffer.
                    if !peer_knows_minibuffer_prompt
                        && matches!(msg, InstanceMessage::MinibufferPrompt { .. })
                    {
                        continue;
                    }
                    if !peer_knows_line_numbers
                        && matches!(msg, InstanceMessage::LineNumbers { .. })
                    {
                        continue;
                    }
                    if !peer_knows_completion_popup
                        && matches!(msg, InstanceMessage::CompletionPopup { .. })
                    {
                        continue;
                    }
                    if !peer_knows_theme_facts && matches!(msg, InstanceMessage::ThemeFacts { .. })
                    {
                        continue;
                    }
                    if !peer_knows_font_facts && matches!(msg, InstanceMessage::FontFacts { .. }) {
                        continue;
                    }
                    // Vterm Stage 3 — TerminalFrame gated at v19. A v18
                    // semantic peer keeps the empty identity snapshot and
                    // no terminal surface; a v18 grid peer is unaffected
                    // because it composes terminal windows into its own
                    // CellDelta.
                    if !peer_accepts_terminal_message(negotiated_protocol_version, msg) {
                        continue;
                    }
                    if !peer_accepts_statusline_message(negotiated_protocol_version, msg) {
                        continue;
                    }
                    // T M10.10 Day 4 / M10.11 F2 — the criterion-1
                    // jitter site: render-write latency.
                    //
                    // `messages` is render output only (CellDelta /
                    // Cursor / CursorByte) — it NEVER carries broadcast
                    // CrdtOps (those go out via `broadcast_crdt_op` at
                    // the top of the loop, the criterion-3 site). So
                    // jitter here is CellDelta-only *by the nature of
                    // this loop*, not by a match choice. Finding 5's
                    // first fix added `| CrdtOp` to the match below
                    // believing it widened jitter to the CRDT path;
                    // that arm was dead — no broadcast CrdtOp ever
                    // reaches this loop. Reverted to honest
                    // CellDelta-only; criterion-3 jitter lives at the
                    // broadcast site via the same `maybe_jitter_sleep`
                    // mechanism. Criterion 1 ("local edit visible in
                    // <1 frame regardless of instance latency") is a
                    // render-write-latency property; CellDelta is its
                    // correct and only target. Fixed-latency mode
                    // (no jitter) is unchanged from M10.10 Day 4.
                    if matches!(msg, InstanceMessage::CellDelta { .. }) {
                        if injected_render_latency_jitter_ms > 0 {
                            maybe_jitter_sleep(
                                injected_render_latency_jitter_ms,
                                injected_render_latency_ms,
                                &mut jitter_rng,
                            );
                        } else if injected_render_latency_ms > 0 {
                            thread::sleep(Duration::from_millis(injected_render_latency_ms));
                        }
                    }
                    if let Err(e) = write_message(stream, msg) {
                        eprintln!("pmacs: write failed for {fid:?} in dispatcher: {e}");
                        write_failed = true;
                        break;
                    }
                }
                // T M10.10 Finding 2: emit authoritative byte-position
                // cursor for replica frontends, paired with the grid
                // Cursor above. Both are derived from the same
                // render-frame iteration (no editor mutation between
                // the two derivations), so they describe the cursor
                // in the same instant in two reference frames. The
                // optimistic-apply path consumes byte_pos; the legacy
                // paint path consumes the grid coord.
                if !write_failed
                    && !terminal_mode
                    && session_registry
                        .session_state(*fid)
                        .is_some_and(|s| s.negotiated_capabilities.crdt_replica)
                {
                    let core = editor.core.borrow();
                    if let Some(window) = core.active_window_for(*fid) {
                        let cursor_byte_msg = InstanceMessage::CursorByte {
                            buffer_id: window.buffer_id,
                            byte_pos: window.cursor,
                        };
                        if let Err(e) = write_message(stream, &cursor_byte_msg) {
                            eprintln!("pmacs: write CursorByte for {fid:?} failed: {e}");
                            write_failed = true;
                        }
                    }
                }
            }

            // Route presence broadcasts to their recipient streams
            // (recipients != fid per sender-exclusion).
            for entry in &broadcasts {
                if let Some(stream) = streams.get_mut(&entry.recipient) {
                    let _ = write_message(stream, &entry.message);
                }
            }

            if write_failed {
                // Drop the broken connection.
                streams.remove(fid);
                render_states.remove(fid);
                semantic_states.remove(fid);
                term_sizes.remove(fid);
                last_dispatch_idle_sent.remove(fid);
                last_active_buffer_sent.remove(fid);
                terminal_bell_baselines.remove(fid);
                editor.detach_frontend_input(*fid);
                session_registry.unregister_session(*fid);
                editor
                    .statusline_registry
                    .borrow_mut()
                    .detach_frontend(*fid);
                editor.core.borrow_mut().unregister_frontend_view(*fid);
            }
        }

        editor.core.borrow_mut().active_frontend = last_dispatched;
        // T M10.8 Day 4 drain + broadcast block lives at the **top**
        // of the loop now (post-audit-round-3 F18 reorder); CrdtOp
        // broadcasts arrive at replicas before the CellDelta /
        // CursorByte for the same edit.

        // Shutdown / quit checks. Send Goodbye to all attached
        // frontends before exiting.
        let core_wants_quit = editor.core.borrow().quit;
        let shutting_down = shutdown.load(Ordering::SeqCst) || core_wants_quit;
        if shutting_down {
            for stream in streams.values_mut() {
                let _ = write_message(
                    stream,
                    &InstanceMessage::Goodbye(GoodbyeReason::ShuttingDown),
                );
            }
            if core_wants_quit {
                shutdown.store(true, Ordering::SeqCst);
            }
            break;
        }

        // Wait up to one frame for the next dispatcher event.
        let frame_target = editor.async_runtime.frame_target_ms();
        match dispatcher_rx.recv_timeout(Duration::from_millis(frame_target)) {
            Ok(event) => {
                handle_dispatcher_event(
                    event,
                    editor,
                    &mut render_states,
                    &mut semantic_states,
                    &mut streams,
                    &mut term_sizes,
                    &mut last_dispatch_idle_sent,
                    &mut last_active_buffer_sent,
                    &mut terminal_bell_baselines,
                    &mut session_registry,
                );
                // Drain a burst of immediately-available events to
                // coalesce typing-flurries / multi-frontend traffic
                // into a single render pass (matches the v0.1
                // run_per_attach drain behavior).
                while let Ok(event) = dispatcher_rx.try_recv() {
                    handle_dispatcher_event(
                        event,
                        editor,
                        &mut render_states,
                        &mut semantic_states,
                        &mut streams,
                        &mut term_sizes,
                        &mut last_dispatch_idle_sent,
                        &mut last_active_buffer_sent,
                        &mut terminal_bell_baselines,
                        &mut session_registry,
                    );
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        // Accepted terminal context controls PTY size. Apply any focus,
        // window, or resize changes before consuming another child-output
        // batch so screen reflow and subsequent bytes share one geometry.
        for frontend_id in &attached_fids {
            if let Some(size) = term_sizes.get(frontend_id).copied() {
                editor.sync_terminal_layout(*frontend_id, size);
            }
            // Vterm Stage 3 — the semantic twin, right beside the grid
            // sync so both frontend kinds resize the screen before the
            // next child-output drain. The frontend declared a CONTENT
            // rectangle, so this consumes the size directly instead of
            // running the TUI placement helper, which would subtract a
            // modeline the GPU never drew.
            if let Some((buffer_id, size)) = semantic_states
                .get(frontend_id)
                .and_then(crate::semantic_render::SemanticRenderState::terminal_viewport)
            {
                editor.sync_semantic_terminal_layout(*frontend_id, buffer_id, size);
            }
        }

        // `tick_async` last: the M4.5 async bridge settles awaiters
        // inside `tick_lsp` (via the message bus); draining + resuming
        // in the same frame keeps LSP `:await()` latency at one frame
        // instead of two. Mirrors the in-process loop in `editor::run`.
        editor.tick_processes();
        editor.tick_lsp();
        editor.tick_async();
    }

    Ok(())
}

/// Handle one `DispatcherEvent`. Extracted so the dispatcher loop
/// can both timeout-recv and burst-drain via the same code path.
/// T M11.2 — extracted from `handle_dispatcher_event`'s
/// `SessionEstablished` arm (kept the parent under the 100-line
/// clippy ceiling). Registers the frontend's view, bootstraps the
/// `BufferMirror` via `BufferSnapshot` when `crdt_replica`, and
/// selects the per-session projection: a `semantic_render` session
/// gets a `SemanticRenderState` (no grid `RenderState`, no
/// initial-full-grid analogue — it emits nothing until the frontend
/// declares a viewport); every other session keeps the M5.3
/// force-full-grid grid path.
fn take_pending_terminal_bell(
    editor: &EditorState,
    frontend_id: FrontendId,
    baselines: &mut HashMap<FrontendId, (crate::buffer::BufferId, u64)>,
) -> bool {
    let buffer_id = editor
        .core
        .borrow()
        .active_window_for(frontend_id)
        .map(|window| window.buffer_id);
    let Some((buffer_id, count)) = buffer_id.and_then(|buffer_id| {
        editor
            .terminal_manager
            .borrow()
            .bell_count(buffer_id)
            .map(|count| (buffer_id, count))
    }) else {
        baselines.remove(&frontend_id);
        return false;
    };

    match baselines.get_mut(&frontend_id) {
        Some((baseline_buffer, delivered))
            if *baseline_buffer == buffer_id && count > *delivered =>
        {
            *delivered += 1;
            true
        }
        Some((baseline_buffer, delivered)) if *baseline_buffer == buffer_id => {
            *delivered = count;
            false
        }
        _ => {
            baselines.insert(frontend_id, (buffer_id, count));
            false
        }
    }
}

struct OpenedInitialTarget {
    buffer_id: crate::buffer::BufferId,
    publish_to_replicas: bool,
}

struct InitialTargetSnapshot {
    crdt_snapshot: Vec<u8>,
    upgraded_to_crdt: bool,
}

fn resolve_initial_target(target: InitialTarget) -> PathBuf {
    let cwd = PathBuf::from(OsString::from_vec(target.cwd));
    let path = PathBuf::from(OsString::from_vec(target.path));
    let absolute = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    crate::editor_core::lexical_normalize(&absolute)
}

fn open_initial_target(
    editor: &mut EditorState,
    frontend_id: FrontendId,
    target: InitialTarget,
) -> Result<OpenedInitialTarget, String> {
    let path = resolve_initial_target(target);
    // Bottom-panel arc (Q#BP11b, R4-B4): capture the fresh view's
    // ORIGINAL document window before any I/O. A startup hook may now
    // create and select a side window, and bootstrap must reassert the
    // requested buffer in a document window rather than overwriting a
    // panel merely because it became `view.active`.
    let (origin_window, buffer_id, fire) = {
        let mut core = editor.core.borrow_mut();
        core.active_frontend = frontend_id;
        let origin_window = core
            .primary_document_window(frontend_id)
            .ok_or_else(|| "attaching frontend has no document window".to_string())?;
        let (buffer_id, fire) = core.resolve_target_buffer(&path)?;
        core.install_buffer_in_window(origin_window, buffer_id)
            .map_err(|error| format!("cannot select {}: {error}", path.display()))?;
        core.focus_window(frontend_id, origin_window);
        (origin_window, buffer_id, fire)
    };

    match fire {
        crate::editor_core::HookKind::AfterLoad => {
            editor
                .lua_host
                .run_hook("buffer.after-load", mlua::MultiValue::new());
        }
        // Dedup is a logical switch even when the fresh view already shares
        // this BufferId; configuration must observe it exactly once.
        crate::editor_core::HookKind::AfterSwitch => {
            editor
                .lua_host
                .run_hook("buffer.after-switch", mlua::MultiValue::new());
        }
        crate::editor_core::HookKind::None => {}
    }
    editor.reconcile_panel_layout(frontend_id);

    let mut core = editor.core.borrow_mut();
    core.active_frontend = frontend_id;
    if !core.registry.borrow().contains(buffer_id) {
        return Err(format!(
            "initial target {} was removed by a startup hook",
            path.display()
        ));
    }
    // Reassert into the original document window when it is still live;
    // if a hook closed it, rehome to an eligible non-side window in the
    // same frontend WITHOUT firing a second hook.
    let destination = if core
        .views
        .get(&frontend_id)
        .is_some_and(|view| view.layout.iter_ids().contains(&origin_window))
    {
        origin_window
    } else {
        core.non_side_target(frontend_id)
            .map_err(|error| format!("cannot reselect {}: {error}", path.display()))?
    };
    core.install_buffer_in_window(destination, buffer_id)
        .map_err(|error| format!("cannot reselect {}: {error}", path.display()))?;
    core.focus_window(frontend_id, destination);
    Ok(OpenedInitialTarget {
        buffer_id,
        publish_to_replicas: matches!(
            fire,
            crate::editor_core::HookKind::AfterLoad | crate::editor_core::HookKind::None
        ),
    })
}

#[cfg(feature = "crdt")]
fn initial_target_snapshot(
    editor: &EditorState,
    buffer_id: crate::buffer::BufferId,
) -> Result<InitialTargetSnapshot, String> {
    let core = editor.core.borrow();
    let mut registry = core.registry.borrow_mut();
    let buffer = registry
        .get_mut(buffer_id)
        .map_err(|error| format!("initial target buffer disappeared: {error}"))?;
    let upgraded_to_crdt = !buffer.is_crdt_backed();
    if upgraded_to_crdt {
        let peer_id = crate::crdt::peer_id_from_frontend(FrontendId::LOCAL);
        buffer
            .upgrade_to_crdt(peer_id)
            .map_err(|error| format!("initial target CRDT upgrade failed: {error:?}"))?;
    }
    let crdt_snapshot = buffer
        .crdt_state()
        .ok_or_else(|| "initial target CRDT state is unavailable".to_owned())?
        .export_snapshot()
        .map_err(|error| format!("initial target snapshot export failed: {error:?}"))?;
    Ok(InitialTargetSnapshot {
        crdt_snapshot,
        upgraded_to_crdt,
    })
}

#[cfg(not(feature = "crdt"))]
fn initial_target_snapshot(
    _editor: &EditorState,
    _buffer_id: crate::buffer::BufferId,
) -> Result<InitialTargetSnapshot, String> {
    Err("initial target requires a CRDT-enabled daemon".to_owned())
}

#[allow(clippy::too_many_arguments)]
fn cleanup_provisional_session(
    editor: &mut EditorState,
    render_states: &mut HashMap<FrontendId, RenderState>,
    semantic_states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
    streams: &mut HashMap<FrontendId, UnixStream>,
    term_sizes: &mut HashMap<FrontendId, CellSize>,
    last_active_buffer_sent: &mut HashMap<FrontendId, crate::buffer::BufferId>,
    session_registry: &mut SessionRegistry,
    frontend_id: FrontendId,
) {
    render_states.remove(&frontend_id);
    semantic_states.remove(&frontend_id);
    if let Some(stream) = streams.remove(&frontend_id) {
        let _ = stream.shutdown(Shutdown::Both);
    }
    term_sizes.remove(&frontend_id);
    last_active_buffer_sent.remove(&frontend_id);
    session_registry.unregister_session(frontend_id);
    editor
        .core
        .borrow_mut()
        .unregister_frontend_view(frontend_id);
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "one session bootstrap transaction"
)]
fn handle_session_established(
    editor: &mut EditorState,
    render_states: &mut HashMap<FrontendId, RenderState>,
    semantic_states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
    streams: &mut HashMap<FrontendId, UnixStream>,
    term_sizes: &mut HashMap<FrontendId, CellSize>,
    last_active_buffer_sent: &mut HashMap<FrontendId, crate::buffer::BufferId>,
    session_registry: &mut SessionRegistry,
    frontend_id: FrontendId,
    session_state: crate::presence::SessionState,
    initial_size: CellSize,
    initial_target: Option<InitialTarget>,
    mut write_stream: UnixStream,
) {
    // Register the frontend's view (M10.8 Day 3: fresh scratch
    // buffer view; future milestones may clone LOCAL's view or
    // take an explicit initial-buffer argument).
    //
    // Arc 6 Stage 2 (Q#FD21): the projection is decided in this same
    // attach transaction and from the same bit that selects a grid
    // `RenderState` vs a `SemanticRenderState` below — a grid session
    // collapses folds, a semantic one keeps raw-line reckoning until
    // Stage 3.
    // Bottom-panel arc (Q#BP13): panel capability comes from the SAME
    // negotiated bit in this same transaction. Stage 1 ships the TUI
    // side windows only, so a semantic session is not panel-capable and
    // a `side` request falls back to its document target with every
    // side-specific parameter discarded.
    let fresh_view = build_fresh_frontend_view(
        editor,
        !session_state.negotiated_capabilities.semantic_render,
        peer_declared_panel_support(session_state),
    );
    {
        let mut core = editor.core.borrow_mut();
        core.register_frontend_view(frontend_id, fresh_view);
        core.active_frontend = frontend_id;
    }

    let opened_target = match initial_target {
        Some(target) => match open_initial_target(editor, frontend_id, target) {
            Ok(opened) => Some(opened),
            Err(message) => {
                send_initial_target_failure(&mut write_stream, message);
                editor
                    .core
                    .borrow_mut()
                    .unregister_frontend_view(frontend_id);
                return;
            }
        },
        None => None,
    };

    let crdt_replica = session_state.negotiated_capabilities.crdt_replica;
    let semantic_render = session_state.negotiated_capabilities.semantic_render;
    let negotiated_protocol_version = session_state.negotiated_protocol_version;

    if let Some(opened) = opened_target.as_ref() {
        let target_snapshot = match initial_target_snapshot(editor, opened.buffer_id) {
            Ok(snapshot) => snapshot,
            Err(message) => {
                send_initial_target_failure(&mut write_stream, message);
                editor
                    .core
                    .borrow_mut()
                    .unregister_frontend_view(frontend_id);
                return;
            }
        };
        let snapshot_message = InstanceMessage::BufferSnapshot {
            buffer_id: opened.buffer_id,
            crdt_snapshot: target_snapshot.crdt_snapshot,
        };
        if opened.publish_to_replicas || target_snapshot.upgraded_to_crdt {
            publish_buffer_snapshot_to_replicas(
                editor,
                opened.buffer_id,
                &snapshot_message,
                session_registry,
                streams,
                semantic_states,
            );
        }
        if write_message(&mut write_stream, &snapshot_message).is_err() {
            let _ = write_stream.shutdown(Shutdown::Both);
            editor
                .core
                .borrow_mut()
                .unregister_frontend_view(frontend_id);
            return;
        }
    } else if crdt_replica {
        // Legacy no-target attach remains an all-buffer replica bootstrap.
        send_buffer_snapshots(editor, &mut write_stream);
    }

    session_registry.register_session(frontend_id, session_state);
    if semantic_render {
        semantic_states.insert(
            frontend_id,
            crate::semantic_render::SemanticRenderState::for_peer(
                frontend_id,
                negotiated_protocol_version,
            ),
        );
    } else {
        let mut render_state = RenderState::new(initial_size);
        render_state.force_full_grid_resync();
        render_states.insert(frontend_id, render_state);
    }
    streams.insert(frontend_id, write_stream);
    term_sizes.insert(frontend_id, initial_size);
    // Bottom-panel arc (Q#BP2b): a grid session's real attach size IS its
    // authoritative geometry declaration, cached BEFORE any input can
    // reach it. A semantic session deliberately stays UNKNOWN — Stage 2's
    // authenticated `FrontendCellGeometry` fills it, and the permanent
    // 24x80 attach placeholder is never consulted for panel layout.
    if editor.core.borrow().panel_capable_for(frontend_id) {
        editor.sync_frame_geometry(frontend_id, initial_size);
    }

    if let Some(opened) = opened_target {
        last_active_buffer_sent.insert(frontend_id, opened.buffer_id);
        let result = InstanceMessage::InitialTargetResult(InitialTargetResult::Opened {
            buffer_id: opened.buffer_id,
        });
        let write_result = {
            let stream = streams
                .get_mut(&frontend_id)
                .expect("new session stream installed");
            write_message(stream, &result)
        };
        if write_result.is_err() {
            cleanup_provisional_session(
                editor,
                render_states,
                semantic_states,
                streams,
                term_sizes,
                last_active_buffer_sent,
                session_registry,
                frontend_id,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)] // per-variant dispatcher match.
fn handle_dispatcher_event(
    event: DispatcherEvent,
    editor: &mut EditorState,
    render_states: &mut HashMap<FrontendId, RenderState>,
    semantic_states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
    streams: &mut HashMap<FrontendId, UnixStream>,
    term_sizes: &mut HashMap<FrontendId, CellSize>,
    last_dispatch_idle_sent: &mut HashMap<FrontendId, bool>,
    last_active_buffer_sent: &mut HashMap<FrontendId, crate::buffer::BufferId>,
    terminal_bell_baselines: &mut HashMap<FrontendId, (crate::buffer::BufferId, u64)>,
    session_registry: &mut SessionRegistry,
) {
    match event {
        DispatcherEvent::SessionEstablished {
            frontend_id,
            session_state,
            initial_size,
            initial_target,
            write_stream,
        } => {
            handle_session_established(
                editor,
                render_states,
                semantic_states,
                streams,
                term_sizes,
                last_active_buffer_sent,
                session_registry,
                frontend_id,
                session_state,
                initial_size,
                initial_target,
                write_stream,
            );
        }
        DispatcherEvent::FrontendEvent { source, event } => {
            if session_registry.session_state(source).is_none() {
                eprintln!("pmacs: dropping frontend event from uninstalled session {source:?}");
                return;
            }
            match event {
                FrontendEvent::Detach(_) => {
                    // The per-attach thread will follow up with a
                    // `SessionDetached` event after its reader
                    // loop exits; cleanup happens there. Just stop
                    // processing here.
                }
                FrontendEvent::Resize { size, .. } => {
                    if let Some(rs) = render_states.get_mut(&source) {
                        rs.resize(size);
                    }
                    if let Some(ts) = term_sizes.get_mut(&source) {
                        *ts = size;
                    }
                    // Bottom-panel arc (Q#BP2b): a frame that can no
                    // longer satisfy the panel hides it, moves focus out,
                    // and releases its terminal controller here — before
                    // the next drained event dispatches.
                    if editor.core.borrow().panel_capable_for(source) {
                        editor.sync_frame_geometry(source, size);
                    }
                }
                #[cfg(feature = "crdt")]
                FrontendEvent::CrdtOp {
                    frontend_id: claimed_fid,
                    buffer_id,
                    op,
                } => {
                    // T M10.10 — handled here (not in apply_event) so
                    // the authenticated `source` is in scope. The
                    // event's `claimed_fid` is client-supplied and not
                    // trusted; we use `source` for sender-exclusion
                    // routing. (Original Finding 4 fix.)
                    //
                    // Second-round audit added three pre-apply checks
                    // (F11, F12, F13). All identity-and-scope fields
                    // (negotiated cap, claimed_fid, op.peer_id,
                    // buffer_id) must agree with the authenticated
                    // `source` and the source's active-window buffer
                    // before the op is applied.
                    if let Err(reason) = validate_remote_crdt_op(
                        editor,
                        session_registry,
                        source,
                        claimed_fid,
                        buffer_id,
                        &op,
                    ) {
                        eprintln!(
                            "pmacs daemon: dropping CrdtOp from {source:?} \
                             (claimed_fid={claimed_fid:?}, buffer_id={buffer_id:?}, \
                             op.peer_id={pid}): {reason}",
                            pid = op.peer_id
                        );
                    } else {
                        handle_remote_crdt_op(editor, source, buffer_id, op);
                    }
                }
                FrontendEvent::Viewport {
                    buffer_id,
                    visible,
                    generation,
                    ..
                } => {
                    // T M11.2 — feed the semantic projection the byte
                    // range the frontend has on screen. Routed by the
                    // authenticated `source` (the client-supplied
                    // `frontend_id` field is not trusted, consistent
                    // with the CrdtOp source-trust rule). A grid
                    // session never sends this; if one does, there is
                    // no `SemanticRenderState` to update and it is a
                    // benign no-op.
                    // Vterm Stage 3 — a v19 frontend declares BOTH a
                    // byte viewport and a terminal cell size after every
                    // snapshot, because an empty terminal identity
                    // snapshot does not announce itself as a terminal.
                    // The daemon keeps only the declaration appropriate
                    // to the authenticated source's ACTIVE buffer.
                    //
                    // Keying on the active buffer rather than the
                    // declared one is load-bearing: `Viewport` also
                    // ALIGNS the window to the buffer it names, so a
                    // stale document viewport still in flight when a
                    // command opens a terminal would drag the frontend
                    // straight back off it. The declared buffer is
                    // checked too — a terminal has no byte viewport to
                    // honor from any direction.
                    let terminal_context = {
                        let manager = editor.terminal_manager.borrow();
                        let core = editor.core.borrow();
                        let active = core
                            .active_window_for(source)
                            .is_some_and(|window| manager.is_terminal(window.buffer_id));
                        active || manager.is_terminal(buffer_id)
                    };
                    if semantic_states.contains_key(&source) && !terminal_context {
                        // Phase B (B1) — the Viewport declares *which
                        // buffer this frontend is displaying*. Align its
                        // editor window to that buffer so keyboard input
                        // (`dispatch_key`) and the `CursorByte` it emits
                        // target the displayed buffer. Without this, a
                        // semantic frontend's window stays bound to
                        // LOCAL's attach-time buffer (often a scratch the
                        // user isn't viewing), so arrow keys moved an
                        // off-screen cursor and the caret never tracked.
                        align_semantic_window_to_buffer(editor, source, buffer_id);
                        if let Some(sem) = semantic_states.get_mut(&source) {
                            sem.set_viewport(buffer_id, visible, generation);
                        }
                    }
                }
                FrontendEvent::TerminalResize {
                    buffer_id, size, ..
                } => {
                    // Vterm Stage 3 — the terminal half of the dual
                    // declaration. Routed by the authenticated `source`;
                    // the payload's `frontend_id` is never read.
                    //
                    // Recording the geometry is what lets a PASSIVE view
                    // receive its own clipped/padded projection, so the
                    // record happens for any accepted declaration. Only
                    // the durable controller's declaration reaches the
                    // shared PTY — a declaration never claims control.
                    if semantic_states.contains_key(&source)
                        && peer_declared_terminal_support(session_registry, source)
                        && editor.semantic_terminal_declaration_is_active(source, buffer_id)
                    {
                        if let Some(sem) = semantic_states.get_mut(&source) {
                            sem.set_terminal_viewport(buffer_id, size);
                        }
                        editor.sync_semantic_terminal_layout(source, buffer_id, size);
                    }
                }
                FrontendEvent::TerminalPointer {
                    buffer_id,
                    coord,
                    kind,
                    mods,
                    ..
                } => {
                    // Vterm Stage 3 — a terminal-cell gesture. The
                    // adapter re-derives the window from the
                    // authenticated source and checks the coordinate
                    // against the geometry that source declared, so a
                    // forged id, a stale buffer, a missing declaration,
                    // or an out-of-bounds cell all drop before any view,
                    // controller, selection, menu, or PTY mutation.
                    if semantic_states.contains_key(&source)
                        && peer_declared_terminal_support(session_registry, source)
                    {
                        editor.dispatch_semantic_terminal_pointer(
                            source, buffer_id, coord, kind, mods,
                        );
                    }
                }
                FrontendEvent::Pointer {
                    buffer_id,
                    byte,
                    kind,
                    mods,
                    ..
                } => {
                    // Mouse framing Q#M1 — a semantic frontend's
                    // locally hit-tested gesture, in source bytes.
                    // Routed by the authenticated `source` (the
                    // client-supplied frontend_id is untrusted — the
                    // CrdtOp / Viewport source-trust rule). The window
                    // aligns to the buffer the frontend says it was
                    // displaying: a click can race a buffer switch.
                    if semantic_states.contains_key(&source) {
                        align_semantic_window_to_buffer(editor, source, buffer_id);
                        if kind == PointerKind::Context {
                            // Q#CM1 — right-click opens the context menu
                            // at the hit byte (needs the Lua builder, so
                            // it routes here rather than dispatch_pointer).
                            editor.open_menu_at_byte(source, buffer_id, byte);
                        } else {
                            editor.dispatch_pointer(source, buffer_id, byte, kind, mods);
                        }
                    }
                }
                FrontendEvent::MenuPointer { index, invoke, .. } => {
                    // Q#CM1 — semantic frontend menu navigation (hover /
                    // click), hit-tested against the popup it drew locally.
                    if semantic_states.contains_key(&source) {
                        editor.dispatch_menu_pointer(source, index, invoke);
                    }
                }
                FrontendEvent::FocusGained(_) => {
                    editor.dispatch_focus(source, true);
                }
                FrontendEvent::FocusLost(_) => {
                    editor.dispatch_focus(source, false);
                }
                FrontendEvent::Paste {
                    frontend_id: claimed_fid,
                    data,
                } => {
                    // Kill ring Q#KR10a — the unified paste route, for
                    // BOTH attachment kinds. Handled here (not in
                    // `apply_event`) for two reasons:
                    //
                    //  1. The semantic input dispatcher used to drop
                    //     `Paste` entirely, so GPU Ctrl-V was a no-op
                    //     (pmacs-gpu always negotiates semantic render).
                    //  2. The authenticated `source` is in scope. The
                    //     event's `claimed_fid` is client-supplied and
                    //     not trusted (the CrdtOp / Viewport / Pointer
                    //     source-trust rule); the old grid arm set
                    //     `active_frontend` from it, letting a forged id
                    //     paste into another frontend's active window.
                    //
                    // The paste is a non-command edit, so it breaks the
                    // source's command chain (Q#KR2), and it fires
                    // `buffer.after-edit` like any other edit (Q#KR10b)
                    // — previously it never did, so LSP missed pastes.
                    if !editor.dispatch_paste(source, &data) {
                        handle_inbound_paste(editor, source, claimed_fid, &data);
                    }
                }
                _ => {
                    let Some(&term_size) = term_sizes.get(&source) else {
                        eprintln!(
                            "pmacs: dropping frontend event without size state for {source:?}"
                        );
                        return;
                    };
                    let mut term_size = term_size;
                    if let Some(render_state) = render_states.get_mut(&source) {
                        apply_event(editor, source, event, &mut term_size, render_state);
                        term_sizes.insert(source, term_size);
                    } else if semantic_states.contains_key(&source) {
                        // Phase B (session B1) — a semantic (grid-less)
                        // session has no `RenderState`, but its keyboard
                        // input still drives the shared editor core. The
                        // input events that don't need grid state
                        // (`Key`, `Mouse`) dispatch through the same
                        // `dispatch_key` / `dispatch_mouse` path the TUI
                        // uses; the resulting cursor move / edit flows
                        // back as `CursorByte` / `CrdtOp`. (Earlier this
                        // arm dropped these events — the "M11.5 scope"
                        // posture — which is why typing in pmacs-gpu did
                        // nothing before B1.)
                        apply_semantic_input_event(editor, source, event, term_size);
                    } else {
                        eprintln!(
                            "pmacs: dropping frontend event without render state for {source:?}"
                        );
                    }
                }
            }
        }
        DispatcherEvent::SessionDetached { frontend_id } => {
            render_states.remove(&frontend_id);
            semantic_states.remove(&frontend_id);
            streams.remove(&frontend_id);
            term_sizes.remove(&frontend_id);
            last_dispatch_idle_sent.remove(&frontend_id);
            last_active_buffer_sent.remove(&frontend_id);
            terminal_bell_baselines.remove(&frontend_id);
            editor.detach_frontend_input(frontend_id);
            session_registry.unregister_session(frontend_id);
            editor
                .statusline_registry
                .borrow_mut()
                .detach_frontend(frontend_id);
            {
                let mut core = editor.core.borrow_mut();
                core.unregister_frontend_view(frontend_id);
                // Kill ring Q#KR11: frontend ids are monotonic, so
                // per-frontend state must not outlive the session.
                core.command_history.remove(&frontend_id);
            }
            // Q#KR11: let Lua modules holding per-frontend tables
            // (killring sessions / kill flags) drop this id's entries.
            // The first frontend-lifecycle hook; carries the raw id.
            let mut args = mlua::MultiValue::new();
            args.push_back(mlua::Value::Integer(
                i64::try_from(frontend_id.0).unwrap_or(i64::MAX),
            ));
            editor.lua_host.run_hook("frontend.detached", args);
        }
    }
}

/// T M10.10: send one `InstanceMessage::BufferSnapshot` per buffer
/// in the editor's registry to the newly-attaching frontend's write
/// stream.
///
/// Called only when the session negotiated `crdt_replica: true`. The
/// receiving frontend's `BufferMirror` consumes these to bootstrap
/// its CRDT replicas before any local-edit path can reference them.
///
/// # M10.10 finding: M10.8's deferred upgrade-on-attach wiring
///
/// M10.2 shipped `Buffer::upgrade_to_crdt`; the doc comment notes
/// "Used by M10.8 (multi-frontend instance state) when a v0.1
/// frontend's buffer is promoted to CRDT-backed at attach time" — but
/// M10.8 shipped without wiring the upgrade call. M10.10 surfaces
/// the gap (no CRDT state to snapshot → no `BufferSnapshot` fires).
///
/// Resolution here: upgrade each non-CRDT buffer to CRDT-backed
/// in-place before exporting its snapshot. Uses
/// `peer_id_from_frontend(FrontendId::LOCAL)` (peer id 1) as the
/// instance's CRDT identity — the daemon-owned edit-source ID. Once
/// upgraded, subsequent attaches see the buffer as already CRDT-
/// backed and skip the upgrade.
///
/// Errors on individual buffers (upgrade failure, snapshot export
/// failure, write failure) are logged and skipped; one failed buffer
/// doesn't abort the others.
#[cfg(feature = "crdt")]
fn send_buffer_snapshots(editor: &EditorState, write_stream: &mut UnixStream) {
    let core = editor.core.borrow();
    let mut registry = core.registry.borrow_mut();
    let buffer_ids: Vec<_> = registry.ids().to_vec();
    let instance_peer_id = crate::crdt::peer_id_from_frontend(FrontendId::LOCAL);

    for buffer_id in buffer_ids {
        let Ok(buf) = registry.get_mut(buffer_id) else {
            continue;
        };
        // Upgrade non-CRDT buffers to CRDT-backed in place. The
        // upgrade preserves the buffer's id, name, and content; only
        // the CRDT machinery is added.
        if !buf.is_crdt_backed()
            && let Err(e) = buf.upgrade_to_crdt(instance_peer_id)
        {
            eprintln!("pmacs: upgrade_to_crdt for {buffer_id:?} failed: {e:?}");
            continue;
        }
        let Some(crdt) = buf.crdt_state() else {
            // Upgrade succeeded but somehow crdt is still None —
            // shouldn't happen; defensive skip.
            continue;
        };
        let snapshot = match crdt.export_snapshot() {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("pmacs: export_snapshot for {buffer_id:?} failed: {e:?}");
                continue;
            }
        };
        let msg = InstanceMessage::BufferSnapshot {
            buffer_id,
            crdt_snapshot: snapshot,
        };
        if let Err(e) = write_message(write_stream, &msg) {
            eprintln!("pmacs: send BufferSnapshot for {buffer_id:?} failed: {e:?}");
            // Continue trying other buffers — the stream may
            // recover, or the next per-tick error handling will
            // detach the session.
        }
    }
}

/// No-op stub for non-CRDT builds. v0.1 frontends never advertise
/// `crdt_replica`, so the caller is gated on a capability that's
/// always false in non-CRDT builds; this stub keeps the call site
/// compiling without conditional logic.
#[cfg(not(feature = "crdt"))]
fn send_buffer_snapshots(_editor: &EditorState, _write_stream: &mut UnixStream) {}

/// T M10.10 Day 3 — ensure the active buffer for `fid`'s window is
/// CRDT-backed, upgrading in place if needed.
///
/// Called per-tick for replica frontends from `dispatcher_loop`.
/// Idempotent: after the first upgrade, `is_crdt_backed()` returns
/// true and subsequent calls are no-ops.
///
/// Documented in M10.10-FRAMING.md's composition-consistency-check
/// application — keeps the invariant "active buffer for replica is
/// CRDT-backed" holding even for mid-session-created buffers
/// (post-`send_buffer_snapshots`). Sets up v0.2's mid-session
/// `BufferSnapshot` broadcast work without retrofit.
///
/// Errors on upgrade are logged but don't abort the session — the
/// per-tick loop will retry on the next iteration; persistent failure
/// signals a deeper buffer-state issue worth surfacing to the user
/// elsewhere.
///
/// # F29 (post-audit-round-5) — mid-session `BufferSnapshot` push
///
/// When this function performs an upgrade (returns `Some(buffer_id)`),
/// the caller broadcasts a `BufferSnapshot` to every currently-
/// attached replica frontend so their `BufferMirror`s gain an
/// entry for the newly-CRDT-backed buffer. Without this push, the
/// replicas' `init_from_snapshot` is never called for the buffer
/// and the optimistic-apply path falls through to v0.1 round-trip
/// (`is_ready` returns false) until the replica detaches and
/// reattaches.
///
/// Idempotency: the function returns `None` if the buffer was
/// already CRDT-backed, so the broadcast only fires on the actual
/// upgrade tick. A receiving frontend whose mirror already has the
/// buffer (e.g. it attached after the upgrade and received the
/// snapshot in `send_buffer_snapshots`) sees `AlreadyInitialized`
/// from `init_from_snapshot` and logs but keeps existing state —
/// non-fatal.
///
/// Returns `Some(buffer_id)` when an upgrade just happened (caller
/// must broadcast); `None` when the buffer was already CRDT-backed
/// or the upgrade failed (failure already logged inside).
#[cfg(feature = "crdt")]
fn ensure_active_buffer_crdt_backed(
    editor: &EditorState,
    fid: FrontendId,
) -> Option<crate::buffer::BufferId> {
    let buffer_id_opt = {
        let core = editor.core.borrow();
        core.active_window_for(fid).map(|w| w.buffer_id)
    };
    let buffer_id = buffer_id_opt?;
    let core = editor.core.borrow();
    let mut registry = core.registry.borrow_mut();
    let Ok(buf) = registry.get_mut(buffer_id) else {
        return None;
    };
    if buf.is_crdt_backed() {
        return None;
    }
    let instance_peer_id = crate::crdt::peer_id_from_frontend(FrontendId::LOCAL);
    match buf.upgrade_to_crdt(instance_peer_id) {
        Ok(()) => Some(buffer_id),
        Err(e) => {
            eprintln!("pmacs: lazy upgrade_to_crdt for {buffer_id:?} (fid {fid:?}) failed: {e:?}");
            None
        }
    }
}

#[cfg(not(feature = "crdt"))]
fn ensure_active_buffer_crdt_backed(
    _editor: &EditorState,
    _fid: FrontendId,
) -> Option<crate::buffer::BufferId> {
    None
}

/// T M10.10 post-audit-round-5 F29 — broadcast a single buffer's
/// `BufferSnapshot` to every currently-attached replica frontend
/// (with `crdt_replica` negotiated).
///
/// Called when [`ensure_active_buffer_crdt_backed`] performs a
/// mid-session upgrade (or when any future code path creates /
/// upgrades a buffer that existing replicas haven't seen yet).
/// Per-replica state tracking isn't kept: replicas whose mirror
/// already has the buffer surface `AlreadyInitialized` from
/// `init_from_snapshot` and log but don't fail. The duplicate
/// send is small (snapshot bytes for the upgrade-instant state,
/// which is the empty / freshly-loaded buffer content the replica
/// already has) and only fires on the actual upgrade tick.
/// Export `buffer_id`'s CRDT snapshot bytes, or `None` (logged) when
/// the buffer is missing, not CRDT-backed, or the export fails.
#[cfg(feature = "crdt")]
fn export_buffer_snapshot(
    editor: &EditorState,
    buffer_id: crate::buffer::BufferId,
) -> Option<Vec<u8>> {
    let core = editor.core.borrow();
    let registry = core.registry.borrow();
    let buf = registry.get(buffer_id).ok()?;
    let crdt = buf.crdt_state()?;
    match crdt.export_snapshot() {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            eprintln!("pmacs: export_snapshot for {buffer_id:?} failed: {e:?}");
            None
        }
    }
}

/// Arc 1b — send `buffer_id`'s snapshot to ONE frontend. The
/// active-buffer-follow path (see the per-tick loop) uses this when a
/// semantic frontend's own active buffer changes to an
/// already-CRDT-backed buffer: the F29 broadcast only fires on the
/// upgrade tick, so without this a frontend that switched *back* to a
/// known buffer (a panel's `q`, `find_or_open` of an open file) kept
/// displaying the old buffer while daemon-side input targeted the new
/// one.
#[cfg(feature = "crdt")]
fn send_buffer_snapshot_to_frontend(
    editor: &EditorState,
    buffer_id: crate::buffer::BufferId,
    fid: FrontendId,
    streams: &mut HashMap<FrontendId, UnixStream>,
) {
    let Some(snapshot_bytes) = export_buffer_snapshot(editor, buffer_id) else {
        return;
    };
    let msg = InstanceMessage::BufferSnapshot {
        buffer_id,
        crdt_snapshot: snapshot_bytes,
    };
    if let Some(stream) = streams.get_mut(&fid)
        && let Err(e) = write_message(stream, &msg)
    {
        eprintln!("pmacs: send BufferSnapshot for {buffer_id:?} to {fid:?} failed: {e}");
    }
}

#[cfg(feature = "crdt")]
fn broadcast_buffer_snapshot_to_replicas(
    editor: &EditorState,
    buffer_id: crate::buffer::BufferId,
    session_registry: &SessionRegistry,
    streams: &mut HashMap<FrontendId, UnixStream>,
    semantic_states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
) {
    let Some(snapshot) = export_buffer_snapshot(editor, buffer_id) else {
        return;
    };
    let message = InstanceMessage::BufferSnapshot {
        buffer_id,
        crdt_snapshot: snapshot,
    };
    publish_buffer_snapshot_to_replicas(
        editor,
        buffer_id,
        &message,
        session_registry,
        streams,
        semantic_states,
    );
}

fn publish_buffer_snapshot_to_replicas(
    editor: &EditorState,
    buffer_id: crate::buffer::BufferId,
    message: &InstanceMessage,
    session_registry: &SessionRegistry,
    streams: &mut HashMap<FrontendId, UnixStream>,
    semantic_states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
) {
    for (peer_id, stream) in streams {
        let Some(session) = session_registry.session_state(*peer_id) else {
            continue;
        };
        if !session.negotiated_capabilities.crdt_replica {
            continue;
        }
        if session.negotiated_capabilities.semantic_render {
            let displays_buffer = editor
                .core
                .borrow()
                .active_window_for(*peer_id)
                .is_some_and(|window| window.buffer_id == buffer_id);
            if !displays_buffer {
                continue;
            }
        }
        if let Err(error) = write_message(stream, message) {
            eprintln!(
                "pmacs: BufferSnapshot publish for {buffer_id:?} to {peer_id:?} failed: {error}"
            );
            continue;
        }
        if let Some(semantic) = semantic_states.get_mut(peer_id) {
            semantic.on_buffer_snapshot_sent(buffer_id);
        }
    }
}

/// T M10.10 (post-audit round 2) — validate an incoming
/// `FrontendEvent::CrdtOp` against four invariants. Returns the
/// rejection reason on failure; `Ok(())` means the op may be applied.
///
/// The four invariants:
///
/// - **F11 — negotiated cap.** The source session must have
///   negotiated `crdt_replica: true`. A legacy / non-replica session
///   has no contract to send `CrdtOp` events; accepting one would let
///   such a session mutate daemon state under a capability it never
///   advertised.
/// - **Original Finding 4 — `claimed_fid` matches `source`.** The
///   event's `frontend_id` field is client-supplied. A buggy or
///   malicious frontend can put another frontend's id there. The
///   original Finding 4 fix used `source` for routing and merely
///   logged a warning on mismatch; F12 tightens this to a hard
///   reject — there is no legitimate reason for the values to
///   differ.
/// - **F12 — `op.peer_id` matches `source`.** The receiving
///   frontend's attach loop derives the broadcast's source via
///   `FrontendId(op.peer_id)` (see `src/attach.rs` in the `CrdtOp`
///   message branch). A frontend A that puts B's peer id in the op
///   payload can cause B's mirror to dedup-skip its way into
///   divergence: B sees the broadcast, thinks it's its own echo,
///   skips it. The daemon must reject before the op is applied or
///   re-broadcast.
/// - **F13 — `buffer_id` matches source's active window buffer.**
///   M10.10's local-edit path (`optimistic::frontend_event_for_keystroke`)
///   only emits `CrdtOp`s for the active mirror buffer. A frontend has
///   no v1.0-scope reason to target a different buffer; rejecting
///   non-active-buffer ops keeps the surface to what the test matrix
///   actually exercises.
#[cfg(feature = "crdt")]
fn validate_remote_crdt_op(
    editor: &EditorState,
    session_registry: &SessionRegistry,
    source: FrontendId,
    claimed_fid: FrontendId,
    buffer_id: crate::buffer::BufferId,
    op: &crate::rope::CrdtOp,
) -> Result<(), &'static str> {
    let crdt_replica = session_registry
        .session_state(source)
        .is_some_and(|s| s.negotiated_capabilities.crdt_replica);
    if !crdt_replica {
        return Err("session did not negotiate crdt_replica");
    }
    if claimed_fid != source {
        return Err("event frontend_id does not match authenticated source");
    }
    if op.peer_id != crate::crdt::peer_id_from_frontend(source) {
        return Err("op.peer_id does not match authenticated source");
    }
    let active_buffer_id = editor
        .core
        .borrow()
        .active_window_for(source)
        .map(|w| w.buffer_id);
    if active_buffer_id != Some(buffer_id) {
        return Err("buffer_id does not match source's active window buffer");
    }
    // F26 (post-audit-round-4) — the wire wrapper's `op.peer_id`
    // matches `source`, but the loro update bytes carry their own
    // internal peer attribution. A hostile or buggy client can
    // wrap update bytes generated under a different peer with the
    // wrapper peer_id set correctly. Fork the buffer's CRDT state
    // and inspect which peers' counters advance on import. Any
    // peer that isn't the authenticated source's peer_id is a
    // protocol violation.
    let expected_peer_id = crate::crdt::peer_id_from_frontend(source);
    let registry_handle = editor.core.borrow().registry.clone();
    let registry = registry_handle.borrow();
    if let Ok(buf) = registry.get(buffer_id)
        && buf
            .validate_remote_op_peer_ids(expected_peer_id, &op.bytes)
            .is_err()
    {
        return Err(
            "op.bytes carry CRDT ops attributed to a peer other than the authenticated source",
        );
    }
    Ok(())
}

/// The unified inbound-paste route (kill ring Q#KR10a) — one handler
/// for grid *and* semantic sessions, keyed by the dispatcher's
/// authenticated `source`. `claimed` is the event payload's
/// client-supplied id: never trusted (a forged id must not paste into
/// another frontend's active window), only logged on mismatch. The
/// paste breaks the source's command chain (a non-command edit, Q#KR2)
/// and fires `buffer.after-edit` when the buffer changed (Q#KR10b).
fn handle_inbound_paste(
    editor: &mut EditorState,
    source: FrontendId,
    claimed: FrontendId,
    data: &[u8],
) {
    if claimed != source {
        eprintln!(
            "pmacs daemon: Paste claimed {claimed:?} but came \
             from {source:?}; using the authenticated source"
        );
    }
    editor.core.borrow_mut().active_frontend = source;
    editor.with_after_edit_check(|state| {
        let mut core = state.core.borrow_mut();
        core.break_command_chain(source);
        if let Err(e) = core.paste_inbound(data) {
            eprintln!("pmacs: inbound paste failed: {e}");
        }
    });
}

/// True when `edit` inserted exactly one UTF-8 codepoint: the leading
/// byte's sequence length equals `inserted_len` (kill ring review
/// round 4 — the typed-character classification for optimistic edits).
#[cfg(feature = "crdt")]
fn is_single_codepoint_insert(edit: &crate::rope::Edit) -> bool {
    let len = edit.inserted_len;
    if !(1..=4).contains(&len) {
        return false;
    }
    let mut first = [0u8; 1];
    edit.new_rope
        .slice(edit.range.start, edit.range.start + 1, &mut first);
    let expected = match first[0] {
        b if b < 0x80 => 1,
        b if b < 0xC0 => return false, // bare continuation byte
        b if b < 0xE0 => 2,
        b if b < 0xF0 => 3,
        _ => 4,
    };
    expected == len
}

/// The exact codepoint a single-codepoint insert landed (auto-pairing
/// Q#AP9). Preconditions are [`is_single_codepoint_insert`]'s; the
/// inserted bytes live in the post-edit rope at `range.start`. `None`
/// on malformed UTF-8 (a classification the byte-length check above
/// already rejects, kept fail-closed rather than panicking).
#[cfg(feature = "crdt")]
fn decoded_single_codepoint(edit: &crate::rope::Edit) -> Option<char> {
    let len = usize::try_from(edit.inserted_len)
        .ok()
        .filter(|l| *l <= 4)?;
    let mut buf = [0u8; 4];
    edit.new_rope.slice(
        edit.range.start,
        edit.range.start + edit.inserted_len,
        &mut buf[..len],
    );
    std::str::from_utf8(&buf[..len]).ok()?.chars().next()
}

/// T M10.10 (post-audit) — apply a *pre-validated*
/// `FrontendEvent::CrdtOp`. Identity, capability, and scope checks
/// happen upstream in `validate_remote_crdt_op`; this function trusts
/// `source` and `buffer_id` and performs four effects:
///
/// 1. **Apply op to the buffer's CRDT state + rope projection**
///    via `Buffer::apply_remote_crdt_op`. Returns an `Edit` so
///    downstream effects can use the `range/inserted_len`.
/// 2. **Update source window's cursor** to the optimistic post-edit
///    position (`edit.range.start + edit.inserted_len` — matches
///    the source frontend's mirror cursor after `advance_cursor` or
///    `retreat_cursor`). Without this, the next per-tick `CursorByte`
///    carries the daemon's stale window cursor and snaps the source
///    frontend's mirror cursor back to the wrong byte.
/// 3. **Notify other windows displaying this buffer** of the edit
///    via `notify_buffer_edit`. Updates `TextView` line caches and
///    overlays so future cursor motions / paints derive from
///    current rope state. Adjusts other-window cursors using
///    right-gravity semantics (mirrors `Buffer::adjust_marks_for_edit`
///    behavior).
/// 4. **Queue for broadcast** to other replica frontends via
///    `pending_crdt_ops`. Sender-exclusion uses the authenticated
///    `source`.
#[cfg(feature = "crdt")]
fn handle_remote_crdt_op(
    editor: &mut EditorState,
    source: FrontendId,
    buffer_id: crate::buffer::BufferId,
    op: crate::rope::CrdtOp,
) {
    // Kill ring Q#KR2: an optimistic edit arrives here without ever
    // touching dispatch_key, so the source's command boundary must be
    // updated — or `C-k x C-k` on the GPU would append across the typed
    // character. Break first (covers every early-return path); a
    // successful apply refines this below: a single-codepoint insert is
    // re-classified as `buffer.self-insert`, giving typed characters the
    // same boundary on both frontends. That keeps kill-chain semantics
    // identical (self-insert is not a kill) while making `this_command`
    // a usable input-origin signal for typed-char consumers (signature
    // help; the completion popup can migrate later).
    editor.core.borrow_mut().break_command_chain(source);
    // Effect 1: apply to buffer's CRDT + rope. Capture the Edit
    // (or `None` for an op that imported cleanly but produced no
    // text delta — F17).
    let edit_opt = {
        let core = editor.core.borrow();
        let registry_handle = core.registry.clone();
        drop(core);
        let mut registry = registry_handle.borrow_mut();
        if let Ok(buf) = registry.get_mut(buffer_id) {
            match buf.apply_remote_crdt_op(&op.bytes) {
                Ok(opt) => opt,
                Err(e) => {
                    eprintln!(
                        "pmacs daemon: apply_remote_crdt_op for \
                     {buffer_id:?} failed: {e:?}; dropping op"
                    );
                    return;
                }
            }
        } else {
            eprintln!("pmacs daemon: CrdtOp for unknown {buffer_id:?}; dropping op");
            return;
        }
    };

    // Effects 2 + 3: update window cursors + notify views. ONLY
    // when an Edit was produced — a CRDT import with no text delta
    // (e.g. concurrent same-character delete) has nothing to
    // notify but the op still needs broadcasting (F17).
    if let Some(edit) = edit_opt.as_ref() {
        let mut core = editor.core.borrow_mut();
        // The input-origin refinement promised above. The optimistic
        // layer emits exactly one op per keystroke, so an empty-range
        // insert of EXACTLY ONE codepoint is a typed character —
        // Backspace/Delete/Undo produce deletes or larger shapes and
        // stay chain-breaks. Decoding the inserted bytes (they are in
        // the post-edit rope) rather than trusting `inserted_len`
        // alone: a 2-byte insert of "a(" is two ASCII codepoints and
        // must NOT classify as typing (review round 4 — it would
        // spuriously auto-trigger signature help). Exact provenance on
        // the wire op is the named deferred general fix.
        let typed_codepoint =
            if edit.range.start == edit.range.end && is_single_codepoint_insert(edit) {
                core.rotate_command(source, "buffer.self-insert");
                // Auto-pairing Q#AP9: the optimistic arm is the second
                // typed self-insert producer. The decoded codepoint plus
                // this Edit build the same exact provenance record the
                // dispatch fallback arms — remote CRDT imports run no
                // intercepts, so requested == effective and clean == true.
                decoded_single_codepoint(edit)
            } else {
                None
            };
        // Transient status messages clear on user input. The Key path
        // gets this from `dispatch_key`'s entry clear; the optimistic
        // path routes plain typing here instead, and since v15 ships
        // `core.status` over `StatusFacts`, a stale "12 references"
        // would otherwise stay wedged in a semantic frontend's band
        // through ordinary typing.
        core.status.clear();
        let post_edit_cursor = edit.range.start + edit.inserted_len;

        // Identify source's active window id (so we can skip it
        // when adjusting other windows' cursors below; the source
        // window's cursor is set to the optimistic post-edit
        // position directly).
        let source_active_window_id = core.views.get(&source).map(|v| v.active);

        // Right-gravity cursor adjustment shape (same as
        // Buffer::adjust_marks_for_edit for MarkGravity::Right).
        let old_len = edit.range.end - edit.range.start;
        let new_end = edit.range.start + edit.inserted_len;
        let inserted_len = edit.inserted_len;

        for (wid, win) in &mut core.windows {
            if win.buffer_id != buffer_id {
                continue;
            }
            if Some(*wid) == source_active_window_id {
                // Q#AI9 (PR #109 round 1): an empty anchor armed at
                // the pre-edit cursor must not survive the cursor
                // moving off it — otherwise the optimistic paths
                // (GPU always; TUI mirror, which tracks no selection
                // state) re-arm the type-over that
                // `insert_char_over_region`'s no-region clear fixed
                // on the dispatch path. Nonempty selections stand:
                // the TUI gate's missing type-over check is a named
                // deferral, and guessing here would destroy a real
                // selection.
                if win.selection.map(|sel| sel.anchor) == Some(win.cursor) {
                    win.selection = None;
                }
                // Source window: set directly to optimistic post-edit
                // position (matches the source frontend's mirror
                // cursor after advance/retreat).
                win.cursor = post_edit_cursor;
                continue;
            }
            // Other window displaying this buffer: shift cursor with
            // right-gravity semantics.
            let pos = win.cursor;
            win.cursor = if pos < edit.range.start {
                pos
            } else if pos > edit.range.end {
                pos - old_len + inserted_len
            } else {
                // Within edit range — clamp to new_end (right-gravity).
                new_end
            };
        }

        core.notify_buffer_edit(buffer_id, edit);
        // Auto-pairing Q#AP9: arm the typed-edit record for the one
        // after-edit fan-out below — but only when the source's
        // active window actually displays the edited buffer, so
        // `post_cursor` (set to the optimistic post-edit position in
        // the window loop above) is that window's real cursor. A
        // synthetic replica editing a background buffer gets no
        // record: absence fails closed, silently.
        if let Some(ch) = typed_codepoint
            && let Some(wid) = source_active_window_id
            && core
                .windows
                .get(&wid)
                .is_some_and(|w| w.buffer_id == buffer_id)
        {
            // Revision postcondition anchor: this arm consumes the
            // record in the same fan-out (no command body runs after
            // the import), so the current revision is trivially the
            // post-edit one.
            let revision = core
                .registry
                .borrow()
                .get(buffer_id)
                .ok()
                .map_or(0, crate::buffer::Buffer::revision);
            core.typed_edit_set_armed(
                source,
                crate::editor_core::TypedEditRecord {
                    buffer: buffer_id,
                    window: wid,
                    codepoint: ch,
                    requested_start: edit.range.start,
                    requested_end: edit.range.end,
                    effective_start: edit.range.start,
                    effective_end: edit.range.end,
                    inserted_len: edit.inserted_len,
                    post_cursor: post_edit_cursor,
                    clean: true,
                    revision,
                },
            );
        }
        // T M11.9 — temporarily switch active_frontend to source so
        // the `buffer.after-edit` hook's Lua observers (notably the
        // LSP `did_change` glue in `builtin/runtime/lsp.lua`) read
        // the right buffer via `pmacs.window.buffer()`. Matches the
        // pattern `dispatch_key` uses (it assigns `active_frontend`
        // before running its hook).
        core.active_frontend = source;
        drop(core);

        // T M11.9 — fire `buffer.after-edit` for replicated edits.
        // Without this, LSP `textDocument/didChange` is never sent
        // for keystrokes the M10.10 optimistic-apply layer routed as
        // `FrontendEvent::CrdtOp` (the bulk of plain-char typing),
        // so clangd's view of the document drifts behind reality.
        // Diagnostics, semantic tokens, and inlay hints all silently
        // freeze at the byte positions they last had when an edit
        // happened to fall back to the Key path. Closes the actual
        // root cause of the session-5 wrong-position-color
        // artifact; the diag-store stale-flag from T M11.8 finally
        // gets reached.
        editor
            .lua_host
            .run_hook("buffer.after-edit", mlua::MultiValue::new());
        // Q#AP9: drop any untaken record the moment the fan-out
        // returns — the slot must never leak into a later hook run.
        editor.core.borrow_mut().typed_edit_clear_armed();
    }

    // Effect 4: queue for broadcast. The source frontend's mirror
    // already applied the op (this is the optimistic-replica
    // path); use `OptimisticReplica(source)` so the broadcast
    // sweep excludes it. F17: this push happens even when
    // `edit_opt` is None — concurrent same-char deletes still
    // need their CRDT causal metadata propagated to peers.
    editor.core.borrow_mut().pending_crdt_ops.push((
        crate::editor_core::CrdtOpOrigin::OptimisticReplica(source),
        buffer_id,
        op,
    ));
}

// Non-CRDT build: the call site in `handle_dispatcher_event` is
// itself feature-gated, so no stub is needed. A non-CRDT daemon
// never receives `FrontendEvent::CrdtOp` from a properly-negotiated
// frontend because `InstanceCapabilities::default()` advertises
// `crdt_replica: false` in non-CRDT builds (Finding 3 fix).

/// Build a `FrontendView` for an attaching frontend.
///
/// T M10.8 Day 3 → T M10.9 update: attaching frontends now share
/// `FrontendId::LOCAL`'s active buffer (typically the daemon's
/// scratch buffer). Each gets its OWN `Window` instance — same
/// buffer, fresh cursor at position 0. This makes M10.9's
/// "two frontends in the same buffer see each other's cursors"
/// acceptance criterion observable: A and B start in the same
/// buffer, their `PresenceUpdate` broadcasts carry matching
/// `buffer_id`, the overlay paint fires.
///
/// Frontends that want their own buffer can still do
/// `pmacs.editor.open(path)` to switch their window to a different
/// buffer; the per-frontend window-tree refactor (M10.8 Q1) makes
/// this independent.
/// Re-point a semantic frontend's active window at `buffer_id` — the
/// buffer it just declared (via `FrontendEvent::Viewport`) that it is
/// displaying. No-op when the window is already on that buffer or the
/// buffer is gone.
///
/// A semantic frontend renders from the wire (`StyleSpans` + its local
/// CRDT replica), so its daemon-side window holds only the cursor and
/// the buffer identity — no grid overlays to migrate. Rebuilding the
/// `TextView` (a cheap line index) and resetting the cursor is the
/// whole switch. This is the input/display alignment fix for B1: the
/// frontend's *declared* buffer becomes the buffer its keys edit and
/// its `CursorByte` reports.
fn align_semantic_window_to_buffer(
    editor: &mut EditorState,
    fid: FrontendId,
    buffer_id: crate::buffer::BufferId,
) {
    use crate::text_view::TextView;

    let text_view = {
        let core = editor.core.borrow();
        let Some(win_id) = core.views.get(&fid).map(|v| v.active) else {
            return;
        };
        if core.windows.get(&win_id).map(|w| w.buffer_id) == Some(buffer_id) {
            return; // Already displaying this buffer.
        }
        let reg = core.registry.borrow();
        let Ok(buf) = reg.get(buffer_id) else {
            return; // Unknown buffer — leave the window as-is.
        };
        TextView::new(buf)
    };

    let mut core = editor.core.borrow_mut();
    let Some(win_id) = core.views.get(&fid).map(|v| v.active) else {
        return;
    };
    if let Some(win) = core.windows.get_mut(&win_id) {
        win.buffer_id = buffer_id;
        win.text_view = text_view;
        win.cursor = 0;
        win.selection = None;
        win.overlays.clear();
    }
}

fn build_fresh_frontend_view(
    editor: &mut EditorState,
    // Arc 6 Stage 2 (Q#FD21, Bet B8): whether this session's display
    // collapses folds. Passed explicitly from the negotiated
    // selected-render bit at the call site — never inferred here.
    fold_projection: bool,
    // Bottom-panel arc (Q#BP13): whether this session can RENDER a side
    // window. Same explicit-at-the-call-site discipline as
    // `fold_projection`; never inferred from a `FrontendId` here.
    panel_capable: bool,
) -> crate::window::FrontendView {
    use crate::text_view::TextView;
    use crate::window::{FrontendView, Layout, Window, WindowId};
    let mut core = editor.core.borrow_mut();
    // T M10.9 — share LOCAL's buffer (don't create a fresh
    // scratch). M10.8's fresh-scratch behavior made overlays
    // never fire because attaching frontends were in distinct
    // buffers.
    //
    // Bottom-panel arc (§1.3 #22): clone LOCAL's PRIMARY DOCUMENT
    // buffer, not `local_view.active`. A TUI panel may own focus at
    // attach time, and panel content must never become a newly attached
    // frontend's full-window document.
    let buffer_id = core
        .primary_document_buffer(FrontendId::LOCAL)
        .expect("LOCAL always retains a document window");
    let text_view = {
        let reg = core.registry.borrow();
        let buf = reg.get(buffer_id).expect("shared buffer present");
        TextView::new(buf)
    };
    let id = WindowId::next();
    let window = Window::new(id, buffer_id, text_view);
    core.windows.insert(id, window);
    FrontendView {
        layout: Layout::single(id),
        active: id,
        fold_projection,
        panel_capable,
        // Grid sessions cache their real attach/resize size; a semantic
        // session stays UNKNOWN until Stage 2's authenticated
        // declaration, and must never be sized against the attach
        // request's permanent 24×80 placeholder (Q#BP15a).
        frame_geometry: None,
        panel_hidden: false,
    }
}

/// Snapshot one frontend's presence (cursor + selection +
/// containing buffer) for T M10.6/8's per-tick broadcast sweep.
///
/// **T M10.8 — explicit `frontend_id` parameter.** M10.6 used the
/// active-frontend default via `core.active_window()`; M10.8 takes
/// the explicit `frontend_id` so the dispatcher can sweep multiple
/// frontends in one tick by calling this for each attached session.
/// If `frontend_id` has no registered view yet (Day 2 transitional
/// state before the dispatcher registers per-attach views), falls
/// back to the active window — preserves M10.6 behavior unchanged.
///
/// The snapshot is taken at the tick boundary — the daemon's render
/// flush point — so multiple cursor moves between sweeps appear as
/// one snapshot transition.
fn build_presence_snapshot(editor: &EditorState, frontend_id: FrontendId) -> PresenceSnapshot {
    let core = editor.core.borrow();
    let win = core
        .active_window_for(frontend_id)
        .unwrap_or_else(|| core.active_window());
    PresenceSnapshot {
        buffer_id: win.buffer_id,
        cursor: win.cursor,
        selection: win.selection.map(|sel| SelectionSnapshot {
            anchor: sel.anchor,
            active: win.cursor,
        }),
    }
}

/// Dispatch a semantic (grid-less) frontend's input event into the
/// shared editor core (Phase B, session B1). Mirrors the `Key` / `Mouse`
/// arms of [`apply_event`] but takes no `RenderState` — a semantic
/// frontend lays out locally, so the only state these events touch is
/// the editor core (cursor, buffer, commands), which `dispatch_key` /
/// `dispatch_mouse` operate on directly. `Resize` / `Focus` have no
/// grid-less effect yet and are dropped; `Viewport` / `CrdtOp` /
/// `Paste` (Q#KR10a) are handled in their own dispatcher arms and
/// never reach here.
#[allow(clippy::needless_pass_by_value)] // consumes the event, mirroring `apply_event`.
fn apply_semantic_input_event(
    editor: &mut EditorState,
    source: FrontendId,
    ev: FrontendEvent,
    term_size: CellSize,
) {
    match ev {
        FrontendEvent::Key(pmacs_key) => {
            if let Some(ct_key) = key_to_crossterm(&pmacs_key) {
                editor.dispatch_key(source, ct_key);
            }
        }
        FrontendEvent::Mouse(pmacs_mouse) => {
            let ct_mouse = mouse_to_crossterm(&pmacs_mouse);
            editor.dispatch_mouse(source, ct_mouse, term_size);
        }
        _ => {}
    }
}

// Takes `ev` by value because it semantically consumes the event;
// the caller pulls events out of the channel one at a time and never
// needs to look at them again.
#[allow(clippy::needless_pass_by_value)]
fn apply_event(
    editor: &mut EditorState,
    source: FrontendId,
    ev: FrontendEvent,
    term_size: &mut CellSize,
    render_state: &mut RenderState,
) {
    match ev {
        FrontendEvent::Key(pmacs_key) => {
            if let Some(ct_key) = key_to_crossterm(&pmacs_key) {
                editor.dispatch_key(source, ct_key);
            }
            // `Key::Unknown` keys (media buttons etc.) have no
            // crossterm equivalent and do not actuate commands; drop.
        }
        FrontendEvent::Mouse(pmacs_mouse) => {
            let ct_mouse = mouse_to_crossterm(&pmacs_mouse);
            editor.dispatch_mouse(source, ct_mouse, *term_size);
        }
        FrontendEvent::Resize { size, .. } => {
            render_state.resize(size);
            *term_size = size;
        }
        // Q#KR10a — Paste is handled in the dispatcher's own
        // `FrontendEvent::Paste` arm (unified for grid and semantic
        // sessions, keyed by the authenticated source), and never
        // reaches here. Listed explicitly so a future reshuffle can't
        // silently re-route it through this payload-trusting path.
        FrontendEvent::Paste { .. }
        | FrontendEvent::FocusGained(_)
        | FrontendEvent::FocusLost(_)
        // T M11.1: the semantic-frontend viewport declaration. Its
        // consumer is the instance-side projection seam
        // (`SemanticRenderState`, M11.2), which scopes the
        // SemanticFrame family to this byte range. M11.1 only
        // declares the wire shape; no projection seam exists yet and
        // the instance advertises `semantic_render: false`, so
        // negotiation rejects any session that would emit this — it
        // is unreachable in practice. Dropped silently until M11.2
        // wires the consumer (same "declared, not yet wired" posture
        // CrdtOp had between M10.5 and M10.8).
        | FrontendEvent::Viewport { .. } => {
            // v0.1: silently ignored. Future work surfaces these
            // through Lua hooks (paste-text-fn, focus-changed-hook).
        }
        FrontendEvent::Detach(_) => {
            // Caller handles Detach as a control event before reaching
            // here.
            unreachable!("Detach is handled by run_per_attach directly");
        }
        FrontendEvent::CrdtOp { .. } => {
            // T M10.10 — handled by `handle_remote_crdt_op` directly
            // from `handle_dispatcher_event` so the authenticated
            // source FrontendId is in scope (the dispatcher's
            // `DispatcherEvent::FrontendEvent { source, event }` tags
            // the message with the per-attach-authenticated id, not
            // the client-supplied `frontend_id` field on the variant).
            // This arm is unreachable in practice; left as a defensive
            // log in case future routing changes deliver a CrdtOp
            // through `apply_event` instead.
            eprintln!(
                "pmacs daemon: FrontendEvent::CrdtOp reached apply_event; \
                 this path is supposed to be intercepted in \
                 handle_dispatcher_event. Dropping op."
            );
        }
        FrontendEvent::Pointer { .. } => {
            // Mouse framing Q#M1 — only semantic sessions emit
            // Pointer, and `handle_dispatcher_event` routes those via
            // `apply_semantic_input_event` (with the authenticated
            // source). A grid session sending one is a protocol
            // violation; drop it like the CrdtOp arm above.
            eprintln!(
                "pmacs daemon: FrontendEvent::Pointer from a grid session; dropping \
                 (semantic sessions route via apply_semantic_input_event)"
            );
        }
        FrontendEvent::MenuPointer { .. } => {
            // Q#CM1 — like Pointer, only semantic sessions emit
            // MenuPointer, routed by the authenticated source in
            // `handle_dispatcher_event`. Drop a grid session's.
            eprintln!(
                "pmacs daemon: FrontendEvent::MenuPointer from a grid session; dropping"
            );
        }
        FrontendEvent::TerminalResize { .. } | FrontendEvent::TerminalPointer { .. } => {
            // Vterm Stage 3 — the terminal declarations belong to
            // semantic sessions and are routed by the authenticated
            // source in `handle_dispatcher_event`. A grid session
            // resizes its terminal through the Stage 2 layout path, so
            // one arriving here is a protocol violation; drop it
            // rather than letting a payload-trusted id reach a view.
            eprintln!(
                "pmacs daemon: terminal declaration from a grid session; dropping \
                 (grid terminals resize through the Stage 2 layout path)"
            );
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
    }

    #[test]
    fn statusline_segments_write_gate_rejects_v17_independently() {
        let segments = InstanceMessage::StatuslineSegments {
            buffer_id: crate::buffer::BufferId::from_raw(1),
            left: Vec::new(),
            right: Vec::new(),
        };
        assert!(!peer_accepts_statusline_message(17, &segments));
        assert!(peer_accepts_statusline_message(18, &segments));
        assert!(peer_accepts_statusline_message(
            17,
            &InstanceMessage::FontFacts {
                family: None,
                size_centi_px: None,
            }
        ));
    }

    #[test]
    fn terminal_frame_write_gate_rejects_v18_independently() {
        let frame = InstanceMessage::TerminalFrame(crate::terminal::TerminalFrame {
            buffer_id: crate::buffer::BufferId::from_raw(2),
            size: crate::cell::CellSize::new(1, 1),
            cells: vec![crate::cell::Cell::default()],
            cursor: None,
            title: None,
            screen_generation: 0,
            selection: Vec::new(),
            scroll_offset: 0,
            at_bottom: true,
            pid: 1,
            process: crate::terminal::TerminalProcessState::Running,
        });
        assert!(!peer_accepts_terminal_message(18, &frame));
        assert!(peer_accepts_terminal_message(19, &frame));
        // The gate is variant-scoped: it must not silence anything else
        // on an older wire.
        assert!(peer_accepts_terminal_message(
            18,
            &InstanceMessage::StatuslineSegments {
                buffer_id: crate::buffer::BufferId::from_raw(2),
                left: Vec::new(),
                right: Vec::new(),
            }
        ));
    }

    #[test]
    fn inbound_terminal_events_require_a_negotiated_v19_session() {
        // Review round 2, finding 5: the outbound `TerminalFrame` was
        // gated twice (producer + write loop) while the inbound
        // declarations relied on the frontend's send gate alone. A
        // pre-v19 peer cannot construct these variants, so this only
        // refuses a hand-rolled client — but it makes "gated in both
        // directions" true of the code, not just of the frontends we
        // ship.
        let mut registry = SessionRegistry::new();
        let semantic = crate::protocol::NegotiatedCapabilities {
            multi_frontend: true,
            crdt_replica: true,
            semantic_render: true,
        };
        let old_peer = FrontendId(2);
        let new_peer = FrontendId(3);
        registry.register_session(
            old_peer,
            crate::presence::SessionState::new(18, semantic, 0),
        );
        registry.register_session(
            new_peer,
            crate::presence::SessionState::new(19, semantic, 1),
        );

        assert!(!peer_declared_terminal_support(&registry, old_peer));
        assert!(peer_declared_terminal_support(&registry, new_peer));
        // An unknown session is refused rather than defaulted open.
        assert!(!peer_declared_terminal_support(&registry, FrontendId(99)));
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn snapshot_publication_skips_foreign_semantic_views_and_ignores_dead_peers() {
        let mut editor = EditorState::new();
        let semantic_peer = FrontendId(20);
        let live_grid_peer = FrontendId(21);
        let dead_grid_peer = FrontendId(22);
        let semantic_view = build_fresh_frontend_view(&mut editor, false, false);
        editor
            .core
            .borrow_mut()
            .register_frontend_view(semantic_peer, semantic_view);

        let semantic_caps = crate::protocol::NegotiatedCapabilities {
            multi_frontend: true,
            crdt_replica: true,
            semantic_render: true,
        };
        let grid_caps = crate::protocol::NegotiatedCapabilities {
            semantic_render: false,
            ..semantic_caps
        };
        let mut registry = SessionRegistry::new();
        registry.register_session(
            semantic_peer,
            crate::presence::SessionState::new(PROTOCOL_VERSION, semantic_caps, 0),
        );
        registry.register_session(
            live_grid_peer,
            crate::presence::SessionState::new(PROTOCOL_VERSION, grid_caps, 1),
        );
        registry.register_session(
            dead_grid_peer,
            crate::presence::SessionState::new(PROTOCOL_VERSION, grid_caps, 2),
        );

        let (semantic_server, mut semantic_client) =
            UnixStream::pair().expect("semantic socketpair");
        let (live_grid_server, mut live_grid_client) =
            UnixStream::pair().expect("live grid socketpair");
        let (dead_grid_server, dead_grid_client) =
            UnixStream::pair().expect("dead grid socketpair");
        drop(dead_grid_client);
        let mut streams = HashMap::from([
            (semantic_peer, semantic_server),
            (live_grid_peer, live_grid_server),
            (dead_grid_peer, dead_grid_server),
        ]);
        let published_buffer = crate::buffer::BufferId::from_raw(900);
        let message = InstanceMessage::BufferSnapshot {
            buffer_id: published_buffer,
            crdt_snapshot: vec![1, 2, 3],
        };

        publish_buffer_snapshot_to_replicas(
            &editor,
            published_buffer,
            &message,
            &registry,
            &mut streams,
            &mut HashMap::new(),
        );

        let delivered: InstanceMessage =
            read_message(&mut live_grid_client).expect("live grid snapshot");
        assert_eq!(delivered, message);
        semantic_client
            .set_read_timeout(Some(Duration::from_millis(50)))
            .expect("semantic timeout");
        assert!(
            read_message::<InstanceMessage>(&mut semantic_client).is_err(),
            "a semantic peer displaying another buffer must receive no snapshot"
        );
    }

    #[test]
    fn frontend_events_from_uninstalled_sessions_are_dropped_without_state_access() {
        let source = FrontendId(77);
        let mut editor = EditorState::new();
        let mut render_states = HashMap::new();
        let mut semantic_states = HashMap::new();
        let mut streams = HashMap::new();
        let mut term_sizes = HashMap::new();
        let mut last_dispatch_idle_sent = HashMap::new();
        let mut last_active_buffer_sent = HashMap::new();
        let mut terminal_bell_baselines = HashMap::new();
        let mut session_registry = SessionRegistry::new();

        handle_dispatcher_event(
            DispatcherEvent::FrontendEvent {
                source,
                event: FrontendEvent::Key(crate::protocol::KeyEvent {
                    frontend_id: source,
                    key: crate::protocol::Key::Char('x'),
                    mods: crate::protocol::Modifiers::NONE,
                    timestamp_ns: 0,
                }),
            },
            &mut editor,
            &mut render_states,
            &mut semantic_states,
            &mut streams,
            &mut term_sizes,
            &mut last_dispatch_idle_sent,
            &mut last_active_buffer_sent,
            &mut terminal_bell_baselines,
            &mut session_registry,
        );

        assert_eq!(editor.core.borrow().active_frontend, FrontendId::LOCAL);
        assert!(term_sizes.is_empty());
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

    /// T M11.9 regression: `handle_remote_crdt_op` fires the
    /// `buffer.after-edit` Lua hook. Without this, the M10.10
    /// optimistic-apply path's `FrontendEvent::CrdtOp` route
    /// bypasses every Lua observer of buffer mutations — most
    /// importantly the LSP `did_change` notification, which means
    /// clangd never re-analyzes documents edited via the optimistic
    /// path. The session-5 wrong-position-color artifact was the
    /// downstream symptom: diagnostics frozen at pre-edit byte
    /// positions because clangd had never been told about the edit.
    #[cfg(feature = "crdt")]
    #[test]
    fn handle_remote_crdt_op_fires_after_edit_hook() {
        use crate::editor::EditorState;
        use crate::protocol::FrontendId;

        let mut editor = EditorState::new();

        // Install an after-edit hook that bumps a global counter
        // we can read back from Lua.
        editor
            .lua_host
            .eval(
                Some("test"),
                r#"
                _G.PMACS_TEST_AFTER_EDIT_FIRED = 0
                pmacs.hook.add("buffer.after-edit", function()
                    _G.PMACS_TEST_AFTER_EDIT_FIRED = (_G.PMACS_TEST_AFTER_EDIT_FIRED or 0) + 1
                end)
                "#,
            )
            .expect("install after-edit hook");

        // Upgrade the active buffer to CRDT-backed so
        // `handle_remote_crdt_op` finds a `CrdtState` to apply
        // against (the non-CRDT path is not exercised here).
        let buffer_id = editor.core.borrow().active_window().buffer_id;
        {
            let core = editor.core.borrow();
            let mut reg = core.registry.borrow_mut();
            reg.get_mut(buffer_id)
                .expect("active buffer")
                .upgrade_to_crdt(2)
                .expect("upgrade to crdt");
        }

        // Build a peer CRDT doc from the buffer's snapshot, perform
        // an edit on the peer, export the op bytes. This is the
        // shape `optimistic::apply_local_insert` produces in the
        // attach loop's optimistic-apply branch.
        let snapshot_bytes = {
            let core = editor.core.borrow();
            let reg = core.registry.borrow();
            let buf = reg.get(buffer_id).expect("buffer");
            buf.crdt_state()
                .expect("crdt-backed")
                .export_snapshot()
                .expect("export snapshot")
        };
        let peer = loro::LoroDoc::new();
        peer.set_peer_id(99).expect("set peer id");
        peer.import(&snapshot_bytes).expect("import snapshot");
        let v_before = peer.oplog_vv();
        peer.get_text("body").insert(0, "x").expect("peer insert");
        let op_bytes = peer
            .export(loro::ExportMode::updates(&v_before))
            .expect("export op");

        // Apply the op via `handle_remote_crdt_op`.
        super::handle_remote_crdt_op(
            &mut editor,
            FrontendId(99),
            buffer_id,
            crate::rope::CrdtOp {
                peer_id: 99,
                bytes: op_bytes,
            },
        );

        // The hook should have fired exactly once.
        let count_val = editor
            .lua_host
            .eval(
                Some("test-readback"),
                "return _G.PMACS_TEST_AFTER_EDIT_FIRED",
            )
            .expect("read counter");
        let count = match count_val {
            mlua::Value::Integer(n) => n,
            other => panic!("expected counter integer, got {other:?}"),
        };
        assert_eq!(
            count, 1,
            "buffer.after-edit must fire when handle_remote_crdt_op produces a text Edit"
        );
    }

    /// Q#AI9 (PR #109 round 1): the optimistic-apply arm clears an
    /// EMPTY anchor on the source window — the GPU always takes this
    /// path, and the TUI attach mirror tracks no selection state, so
    /// neither frontend's gate stops an armed-empty-anchor sequence
    /// from re-creating the type-over that
    /// `insert_char_over_region`'s no-region clear fixed on the
    /// dispatch path. A NONEMPTY selection must survive untouched
    /// (the TUI gate's missing type-over check is a named deferral).
    #[cfg(feature = "crdt")]
    #[test]
    fn handle_remote_crdt_op_clears_only_an_empty_source_anchor() {
        use crate::editor::EditorState;
        use crate::protocol::FrontendId;
        use crate::window::Selection;

        // Shared fixture: CRDT-backed active buffer + a peer doc that
        // produces the optimistic op, sourced from LOCAL (which has a
        // registered view, so the source-window arm runs).
        fn apply_peer_insert(editor: &mut EditorState, buffer_id: crate::buffer::BufferId) {
            let snapshot_bytes = {
                let core = editor.core.borrow();
                let reg = core.registry.borrow();
                let buf = reg.get(buffer_id).expect("buffer");
                buf.crdt_state()
                    .expect("crdt-backed")
                    .export_snapshot()
                    .expect("export snapshot")
            };
            let peer = loro::LoroDoc::new();
            peer.set_peer_id(u64::from(FrontendId::LOCAL.0))
                .expect("set peer id");
            peer.import(&snapshot_bytes).expect("import snapshot");
            let v_before = peer.oplog_vv();
            peer.get_text("body").insert(0, "x").expect("peer insert");
            let op_bytes = peer
                .export(loro::ExportMode::updates(&v_before))
                .expect("export op");
            super::handle_remote_crdt_op(
                editor,
                FrontendId::LOCAL,
                buffer_id,
                crate::rope::CrdtOp {
                    peer_id: FrontendId::LOCAL.0,
                    bytes: op_bytes,
                },
            );
        }

        // Case 1: empty anchor at the cursor (S-Left-at-BOF shape) —
        // cleared by the optimistic apply.
        let mut editor = EditorState::new();
        let buffer_id = editor.core.borrow().active_window().buffer_id;
        {
            let core = editor.core.borrow();
            let mut reg = core.registry.borrow_mut();
            reg.get_mut(buffer_id)
                .expect("active buffer")
                .upgrade_to_crdt(2)
                .expect("upgrade to crdt");
        }
        {
            let mut core = editor.core.borrow_mut();
            let at = core.active_window().cursor;
            core.active_window_mut().selection = Some(Selection { anchor: at });
        }
        apply_peer_insert(&mut editor, buffer_id);
        {
            let core = editor.core.borrow();
            assert!(
                core.active_window().selection.is_none(),
                "an empty anchor must not survive an optimistic source edit"
            );
            assert_eq!(
                core.active_window().cursor,
                1,
                "cursor at post-edit position"
            );
        }

        // Case 2: nonempty selection — the arm must not touch it.
        let mut editor = EditorState::new();
        let buffer_id = editor.core.borrow().active_window().buffer_id;
        editor.core.borrow_mut().insert_char('a');
        editor.core.borrow_mut().insert_char('b');
        {
            let core = editor.core.borrow();
            let mut reg = core.registry.borrow_mut();
            reg.get_mut(buffer_id)
                .expect("active buffer")
                .upgrade_to_crdt(2)
                .expect("upgrade to crdt");
        }
        {
            let mut core = editor.core.borrow_mut();
            core.active_window_mut().selection = Some(Selection { anchor: 0 });
            // cursor is at 2 after the two inserts: nonempty region.
        }
        apply_peer_insert(&mut editor, buffer_id);
        {
            let core = editor.core.borrow();
            assert_eq!(
                core.active_window().selection,
                Some(Selection { anchor: 0 }),
                "a nonempty selection survives the optimistic source edit"
            );
        }
    }

    /// Kill ring Q#KR2 — GPU typing arrives here without touching
    /// dispatch_key, so it must update the source frontend's command
    /// boundary or `C-k x C-k` on the GPU would append across the typed
    /// character. A single-codepoint insert classifies as
    /// `buffer.self-insert` (the input-origin signal for signature
    /// help); anything else breaks the chain outright.
    #[cfg(feature = "crdt")]
    #[test]
    fn handle_remote_crdt_op_classifies_typed_input_and_ends_kill_chains() {
        use crate::editor::EditorState;
        use crate::protocol::FrontendId;

        let mut editor = EditorState::new();
        let source = FrontendId(7);
        // A live kill chain for the source frontend...
        editor
            .core
            .borrow_mut()
            .rotate_command(source, "edit.kill-line");
        // ...and one for a bystander that must survive.
        editor
            .core
            .borrow_mut()
            .rotate_command(FrontendId::LOCAL, "edit.kill-line");

        let buffer_id = editor.core.borrow().active_window().buffer_id;
        {
            let core = editor.core.borrow();
            let mut reg = core.registry.borrow_mut();
            reg.get_mut(buffer_id)
                .expect("active buffer")
                .upgrade_to_crdt(2)
                .expect("upgrade to crdt");
        }
        let snapshot_bytes = {
            let core = editor.core.borrow();
            let reg = core.registry.borrow();
            reg.get(buffer_id)
                .expect("buffer")
                .crdt_state()
                .expect("crdt-backed")
                .export_snapshot()
                .expect("export snapshot")
        };
        let peer = loro::LoroDoc::new();
        peer.set_peer_id(7).expect("set peer id");
        peer.import(&snapshot_bytes).expect("import snapshot");
        let v_before = peer.oplog_vv();
        peer.get_text("body").insert(0, "x").expect("peer insert");
        let op_bytes = peer
            .export(loro::ExportMode::updates(&v_before))
            .expect("export op");

        handle_remote_crdt_op(
            &mut editor,
            source,
            buffer_id,
            crate::rope::CrdtOp {
                peer_id: 7,
                bytes: op_bytes,
            },
        );

        let core = editor.core.borrow();
        // A single-codepoint optimistic insert classifies as a typed
        // character: the boundary rotates to buffer.self-insert (the
        // input-origin signal), which — not being a kill command —
        // still breaks the kill chain exactly like the TUI typed-char
        // path.
        assert_eq!(
            core.command_history
                .get(&source)
                .and_then(|b| b.this.as_deref()),
            Some("buffer.self-insert"),
            "a typed optimistic insert classifies as self-insert"
        );
        assert_eq!(
            core.command_history
                .get(&source)
                .and_then(|b| b.last.as_deref()),
            None,
            "the pre-existing kill chain is gone (break-then-classify): a \
             following kill reads last = self-insert after its own rotation \
             and never appends"
        );
        drop(core);

        // A TWO-codepoint insert ("a(") must NOT classify as typing
        // (review round 4): its 2-byte length satisfies a naive 1-4
        // predicate, but decoding shows two ASCII codepoints — a typed
        // key never produces that, and classifying it would let a
        // multi-char op spuriously auto-trigger signature help.
        editor
            .core
            .borrow_mut()
            .rotate_command(source, "edit.kill-line");
        let snapshot_bytes = {
            let core = editor.core.borrow();
            let reg = core.registry.borrow();
            reg.get(buffer_id)
                .expect("buffer")
                .crdt_state()
                .expect("crdt-backed")
                .export_snapshot()
                .expect("export snapshot")
        };
        let peer2 = loro::LoroDoc::new();
        peer2.set_peer_id(7).expect("set peer id");
        peer2.import(&snapshot_bytes).expect("import snapshot");
        let v_before = peer2.oplog_vv();
        peer2.get_text("body").insert(0, "a(").expect("peer insert");
        let op_bytes = peer2
            .export(loro::ExportMode::updates(&v_before))
            .expect("export op");
        handle_remote_crdt_op(
            &mut editor,
            source,
            buffer_id,
            crate::rope::CrdtOp {
                peer_id: 7,
                bytes: op_bytes,
            },
        );
        let core = editor.core.borrow();
        assert_eq!(
            core.command_history
                .get(&source)
                .and_then(|b| b.this.as_deref()),
            None,
            "a multi-codepoint insert breaks the chain instead of classifying as typing"
        );
        assert_eq!(
            core.command_history
                .get(&FrontendId::LOCAL)
                .and_then(|b| b.this.as_deref()),
            Some("edit.kill-line"),
            "a bystander frontend's chain is untouched"
        );
    }

    #[test]
    fn forged_resize_mutates_only_the_authenticated_frontend() {
        let source = FrontendId(41);
        let forged = FrontendId(42);
        let old_size = CellSize::new(24, 80);
        let new_size = CellSize::new(31, 97);
        let mut editor = EditorState::new();
        let mut render_states = HashMap::from([
            (source, RenderState::new(old_size)),
            (forged, RenderState::new(old_size)),
        ]);
        let mut semantic_states: HashMap<FrontendId, crate::semantic_render::SemanticRenderState> =
            HashMap::new();
        let mut streams: HashMap<FrontendId, UnixStream> = HashMap::new();
        let mut term_sizes = HashMap::from([(source, old_size), (forged, old_size)]);
        let mut last_dispatch_idle_sent = HashMap::new();
        let mut last_active_buffer_sent = HashMap::new();
        let mut terminal_bell_baselines = HashMap::new();
        let mut session_registry = SessionRegistry::new();
        session_registry.register_session(
            source,
            crate::presence::SessionState::new(
                PROTOCOL_VERSION,
                crate::protocol::NegotiatedCapabilities {
                    multi_frontend: true,
                    crdt_replica: false,
                    semantic_render: false,
                },
                0,
            ),
        );

        handle_dispatcher_event(
            DispatcherEvent::FrontendEvent {
                source,
                event: FrontendEvent::Resize {
                    frontend_id: forged,
                    size: new_size,
                },
            },
            &mut editor,
            &mut render_states,
            &mut semantic_states,
            &mut streams,
            &mut term_sizes,
            &mut last_dispatch_idle_sent,
            &mut last_active_buffer_sent,
            &mut terminal_bell_baselines,
            &mut session_registry,
        );

        assert_eq!(render_states[&source].size(), new_size);
        assert_eq!(term_sizes[&source], new_size);
        assert_eq!(render_states[&forged].size(), old_size);
        assert_eq!(term_sizes[&forged], old_size);
    }
    #[test]
    fn terminal_bell_baseline_suppresses_history_and_delivers_each_new_bell_once() {
        let mut editor = EditorState::new();
        let mut spec = crate::terminal::TerminalSpec::new("/bin/sh");
        spec.args = vec![
            "-c".into(),
            "printf '\\a'; IFS= read -r _; printf '\\a'; sleep 30".into(),
        ];
        let buffer_id = editor
            .terminal_manager
            .borrow_mut()
            .open(
                spec,
                &mut editor.core.borrow_mut(),
                &mut editor.process_supervisor.borrow_mut(),
            )
            .expect("open bell probe");
        editor
            .core
            .borrow_mut()
            .switch_active_buffer_for(FrontendId::LOCAL, buffer_id)
            .expect("display bell probe");

        let deadline = Instant::now() + Duration::from_secs(5);
        while editor.terminal_manager.borrow().bell_count(buffer_id) != Some(1) {
            editor.tick_processes();
            assert!(Instant::now() < deadline, "initial terminal bell timed out");
            thread::sleep(Duration::from_millis(10));
        }
        let mut baselines = HashMap::new();
        assert!(!take_pending_terminal_bell(
            &editor,
            FrontendId::LOCAL,
            &mut baselines
        ));

        editor
            .terminal_manager
            .borrow()
            .send(
                buffer_id,
                b"\n",
                &mut editor.process_supervisor.borrow_mut(),
            )
            .expect("advance bell probe");
        let deadline = Instant::now() + Duration::from_secs(5);
        while editor.terminal_manager.borrow().bell_count(buffer_id) != Some(2) {
            editor.tick_processes();
            assert!(Instant::now() < deadline, "second terminal bell timed out");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(take_pending_terminal_bell(
            &editor,
            FrontendId::LOCAL,
            &mut baselines
        ));
        assert!(!take_pending_terminal_bell(
            &editor,
            FrontendId::LOCAL,
            &mut baselines
        ));

        editor
            .terminal_manager
            .borrow_mut()
            .terminate(buffer_id, &mut editor.process_supervisor.borrow_mut())
            .expect("terminate bell probe");
    }

    /// Kill ring Q#KR10a — the unified paste route trusts only the
    /// dispatcher's authenticated source. A forged payload id must not
    /// paste into another frontend's active window, and the paste
    /// breaks the SOURCE's chain (not the claimed frontend's) and
    /// fires `buffer.after-edit` exactly once.
    #[test]
    fn inbound_paste_uses_authenticated_source_not_the_claimed_id() {
        use crate::editor::EditorState;
        use crate::protocol::FrontendId;
        use crate::text_view::TextView;
        use crate::window::{FrontendView, Layout, Window, WindowId};

        let mut editor = EditorState::new();
        editor
            .lua_host
            .eval(
                Some("test"),
                r#"
                _G.PASTE_AFTER_EDIT = 0
                pmacs.hook.add("buffer.after-edit", function()
                    _G.PASTE_AFTER_EDIT = _G.PASTE_AFTER_EDIT + 1
                end)
                "#,
            )
            .expect("install after-edit hook");

        // Give the attacker frontend its OWN view onto its own buffer,
        // so "which window did the text land in" is observable.
        let source = FrontendId(7);
        let victim = FrontendId::LOCAL;
        let attacker_buf = {
            let core = editor.core.borrow();
            let mut reg = core.registry.borrow_mut();
            reg.create("attacker-buffer")
        };
        {
            let mut core = editor.core.borrow_mut();
            let tv = {
                let reg = core.registry.borrow();
                TextView::new(reg.get(attacker_buf).expect("attacker buffer"))
            };
            let wid = WindowId::next();
            core.windows.insert(wid, Window::new(wid, attacker_buf, tv));
            core.register_frontend_view(
                source,
                FrontendView {
                    layout: Layout::single(wid),
                    active: wid,
                    fold_projection: true,
                    panel_capable: true,
                    frame_geometry: None,
                    panel_hidden: false,
                },
            );
        }
        let victim_buf = editor.core.borrow().active_window().buffer_id;
        // Seed a live chain on the victim: the forged paste must not
        // break it (only the authenticated source's chain breaks).
        editor
            .core
            .borrow_mut()
            .rotate_command(victim, "edit.kill-line");

        // The payload CLAIMS to be the victim.
        handle_inbound_paste(&mut editor, source, victim, b"FORGED");

        let core = editor.core.borrow();
        let text_of = |id| {
            let reg = core.registry.borrow();
            let buf = reg.get(id).expect("buffer");
            let len = buf.len();
            let mut out = vec![0u8; usize::try_from(len).unwrap_or(0)];
            if len > 0 {
                buf.snapshot_rope().slice(0, len, &mut out);
            }
            String::from_utf8_lossy(&out).into_owned()
        };
        assert!(
            text_of(attacker_buf).contains("FORGED"),
            "the paste lands in the AUTHENTICATED source's active window"
        );
        assert!(
            !text_of(victim_buf).contains("FORGED"),
            "a forged payload id must not paste into the claimed frontend's window"
        );
        assert!(
            core.command_history
                .get(&source)
                .is_none_or(|b| b.this.is_none()),
            "the paste breaks the source's chain"
        );
        assert_eq!(
            core.command_history
                .get(&victim)
                .and_then(|b| b.this.as_deref()),
            Some("edit.kill-line"),
            "the claimed frontend's chain is untouched"
        );
        drop(core);
        let count = editor
            .lua_host
            .eval(Some("test-readback"), "return _G.PASTE_AFTER_EDIT")
            .expect("read counter");
        assert!(
            matches!(count, mlua::Value::Integer(1)),
            "paste fires buffer.after-edit exactly once, got {count:?}"
        );
    }

    /// v15 regression: an optimistic-path edit (the bulk of plain-char
    /// typing from a semantic frontend) must clear the transient
    /// status message, exactly as `dispatch_key`'s entry clear does
    /// for round-tripped keys — otherwise "12 references" stays
    /// wedged in the GPU band (which renders `StatusFacts.message`)
    /// through ordinary typing.
    #[cfg(feature = "crdt")]
    #[test]
    fn handle_remote_crdt_op_clears_the_transient_status() {
        use crate::editor::EditorState;
        use crate::protocol::FrontendId;

        let mut editor = EditorState::new();
        let buffer_id = editor.core.borrow().active_window().buffer_id;
        {
            let core = editor.core.borrow();
            let mut reg = core.registry.borrow_mut();
            reg.get_mut(buffer_id)
                .expect("active buffer")
                .upgrade_to_crdt(2)
                .expect("upgrade to crdt");
        }
        let snapshot_bytes = {
            let core = editor.core.borrow();
            let reg = core.registry.borrow();
            reg.get(buffer_id)
                .expect("active buffer")
                .crdt_state()
                .expect("crdt-backed")
                .export_snapshot()
                .expect("export snapshot")
        };
        let peer = loro::LoroDoc::new();
        peer.set_peer_id(99).expect("set peer id");
        peer.import(&snapshot_bytes).expect("import snapshot");
        let v_before = peer.oplog_vv();
        peer.get_text("body").insert(0, "x").expect("peer insert");
        let op_bytes = peer
            .export(loro::ExportMode::updates(&v_before))
            .expect("export op");

        editor.core.borrow_mut().status = "12 references".to_owned();
        super::handle_remote_crdt_op(
            &mut editor,
            FrontendId(99),
            buffer_id,
            crate::rope::CrdtOp {
                peer_id: 99,
                bytes: op_bytes,
            },
        );
        assert!(
            editor.core.borrow().status.is_empty(),
            "an optimistic-path edit must clear the transient status"
        );
    }

    /// Session B1 regression: a `Key` event from a *semantic*
    /// (grid-less) frontend must reach the editor core. Before B1 the
    /// dispatcher's catch-all only called `apply_event` when the
    /// frontend had a `RenderState`, so a semantic frontend's keys were
    /// silently dropped — typing in pmacs-gpu did nothing. The routing
    /// now goes through `apply_semantic_input_event`; a printable char
    /// must self-insert at the frontend's window cursor.
    #[cfg(feature = "crdt")]
    #[test]
    fn semantic_frontend_key_event_reaches_the_core() {
        use crate::editor::EditorState;
        use crate::protocol::FrontendId;
        use pmacs_protocol::{Key, KeyEvent, Modifiers};

        let mut editor = EditorState::new();
        let fid = FrontendId(99);
        // Both these fixtures model a SEMANTIC session (Q#FD21: no fold
        // projection until Stage 3).
        let view = build_fresh_frontend_view(&mut editor, false, false);
        editor.core.borrow_mut().register_frontend_view(fid, view);

        let before = editor
            .core
            .borrow()
            .active_window_for(fid)
            .expect("fid window")
            .cursor;

        apply_semantic_input_event(
            &mut editor,
            fid,
            FrontendEvent::Key(KeyEvent {
                frontend_id: fid,
                key: Key::Char('X'),
                mods: Modifiers::NONE,
                timestamp_ns: 0,
            }),
            CellSize::new(24, 80),
        );

        let after = editor
            .core
            .borrow()
            .active_window_for(fid)
            .expect("fid window")
            .cursor;
        assert_eq!(
            after,
            before + 1,
            "a semantic frontend's printable Key must self-insert and advance its window cursor \
             (pre-B1 the dispatcher dropped it)"
        );
    }

    /// B1 input/display alignment: a semantic frontend's window is bound
    /// to LOCAL's attach-time buffer, but the buffer it *displays* is
    /// the one it declares via `Viewport`. `align_semantic_window_to_buffer`
    /// re-points the window so keys edit the displayed buffer — without
    /// it, arrow keys moved an off-screen cursor in the wrong buffer and
    /// the caret never tracked.
    #[cfg(feature = "crdt")]
    #[test]
    fn viewport_aligns_semantic_window_to_displayed_buffer() {
        use crate::editor::EditorState;
        use crate::protocol::FrontendId;
        use pmacs_protocol::{Key, KeyEvent, Modifiers};

        let mut editor = EditorState::new();
        let scratch = editor.core.borrow().active_window().buffer_id;
        let file = {
            let core = editor.core.borrow();
            core.registry
                .borrow_mut()
                .create_from_bytes("file".to_owned(), b"hello\nworld\n")
        };
        assert_ne!(scratch, file);

        // Attach: window shares LOCAL's active (scratch).
        let fid = FrontendId(99);
        // Both these fixtures model a SEMANTIC session (Q#FD21: no fold
        // projection until Stage 3).
        let view = build_fresh_frontend_view(&mut editor, false, false);
        editor.core.borrow_mut().register_frontend_view(fid, view);
        assert_eq!(
            editor
                .core
                .borrow()
                .active_window_for(fid)
                .unwrap()
                .buffer_id,
            scratch
        );

        // The frontend declares it is displaying the file buffer.
        align_semantic_window_to_buffer(&mut editor, fid, file);
        assert_eq!(
            editor
                .core
                .borrow()
                .active_window_for(fid)
                .unwrap()
                .buffer_id,
            file,
            "Viewport must re-point the window at the displayed buffer"
        );

        // A key now edits the *displayed* buffer, advancing its cursor.
        apply_semantic_input_event(
            &mut editor,
            fid,
            FrontendEvent::Key(KeyEvent {
                frontend_id: fid,
                key: Key::Char('Z'),
                mods: Modifiers::NONE,
                timestamp_ns: 0,
            }),
            CellSize::new(24, 80),
        );
        assert_eq!(
            editor.core.borrow().active_window_for(fid).unwrap().cursor,
            1,
            "key must self-insert into the displayed buffer, not the attach-time scratch"
        );
    }

    /// Bottom-panel arc, §1.3 #22 (framing acceptance 51's Stage-1 half).
    ///
    /// A fresh no-target attach clones `LOCAL`'s **primary document**
    /// buffer, not `local_view.active`. Stage 1 makes a TUI panel a real
    /// focus target, so `LOCAL` can legitimately own focus in a panel at
    /// attach time — and panel content must never become a newly attached
    /// frontend's full-window document.
    #[test]
    fn fresh_attach_inherits_locals_document_buffer_not_its_focused_panel() {
        let mut editor = EditorState::new();
        let document_buffer = editor.core.borrow().active_buffer_id();
        let panel_buffer = editor.core.borrow().registry.borrow_mut().create("*panel*");
        // Open a bottom panel on LOCAL and focus it.
        let panel = {
            let mut core = editor.core.borrow_mut();
            let mut request = crate::editor_core::DisplayRequest::new(panel_buffer);
            request.side = Some(crate::window::Side::Bottom);
            request.height = Some(5);
            request.select = Some(true);
            let outcome = core
                .display_buffer(FrontendId::LOCAL, &request)
                .expect("panel placement");
            core.focus_window(FrontendId::LOCAL, outcome.target);
            outcome.target
        };
        assert_eq!(
            editor.core.borrow().views[&FrontendId::LOCAL].active,
            panel,
            "LOCAL really is focused in the panel"
        );

        let fid = FrontendId(123);
        let view = build_fresh_frontend_view(&mut editor, false, false);
        editor.core.borrow_mut().register_frontend_view(fid, view);

        assert_eq!(
            editor
                .core
                .borrow()
                .active_window_for(fid)
                .expect("fresh view window")
                .buffer_id,
            document_buffer,
            "the new frontend inherited LOCAL's DOCUMENT buffer; inheriting \
             `local_view.active` would have made the panel its document"
        );
        assert_ne!(document_buffer, panel_buffer);
    }

    /// Bottom-panel arc, Q#BP11b / R4-B4 (framing acceptance 55's
    /// Stage-1 half).
    ///
    /// Stage 1 lets a startup hook create and select a side window. The
    /// initial-target bootstrap must still reassert the requested buffer
    /// in — and activate — a **non-side** document window, rather than
    /// overwriting the panel merely because it became `view.active`.
    #[test]
    fn initial_target_reasserts_a_document_window_when_a_hook_selects_a_panel() {
        use std::os::unix::ffi::OsStrExt as _;

        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target.txt");
        std::fs::write(&target, b"target contents\n").expect("write target");

        let mut editor = EditorState::new();
        editor
            .lua_host
            .lua()
            .load(
                r#"
                pmacs.lsp.config = {}
                pmacs.hook.add("buffer.after-load", function()
                  if HOOK_RAN then return end
                  HOOK_RAN = true
                  HOOK_PANEL = pmacs.window.display(
                    pmacs.buffer.create("*hook-panel*"),
                    { side = "bottom", height = 4, select = true })
                end)
                "#,
            )
            .exec()
            .expect("install hook");

        // A GRID session (panel-capable), which is the realistic shape
        // for a hook-created panel in Stage 1 — and real geometry, so
        // the panel is genuinely VISIBLE and focused when the reassert
        // runs. Without the declaration, reconciliation would hide the
        // panel and move focus out on its own, and the assertions below
        // would pass without exercising the reassert at all.
        let fid = FrontendId(124);
        let view = build_fresh_frontend_view(&mut editor, true, true);
        editor.core.borrow_mut().register_frontend_view(fid, view);
        editor.sync_frame_geometry(fid, CellSize::new(24, 80));

        let opened = open_initial_target(
            &mut editor,
            fid,
            InitialTarget {
                path: target.as_os_str().as_bytes().to_vec(),
                cwd: dir.path().as_os_str().as_bytes().to_vec(),
            },
        )
        .expect("bootstrap succeeds despite the panel-creating hook");

        let core = editor.core.borrow();
        assert!(
            !core.views[&fid].panel_hidden,
            "the hook's panel is visible, so focus really was on it when \
             the reassert ran"
        );
        let active = core.views[&fid].active;
        let active_window = core.windows.get(&active).expect("active window live");
        assert!(
            !active_window.is_side(),
            "bootstrap activated a DOCUMENT window, not the hook's panel"
        );
        assert_eq!(
            active_window.buffer_id, opened.buffer_id,
            "…showing the requested target"
        );
        let panel = core
            .side_window_for(fid)
            .expect("the hook's panel survived");
        assert_ne!(
            core.windows[&panel].buffer_id, opened.buffer_id,
            "the panel was not overwritten with the target"
        );
    }
}
