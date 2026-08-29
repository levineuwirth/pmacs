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
    ADVERTISED_PROTOCOL_VERSION, AttachRequest, FrontendEvent, FrontendId, GoodbyeReason, Hello,
    InitialTarget, InitialTargetResult, InstanceCapabilities, InstanceIdentity, InstanceMessage,
    InstanceSignal, MAX_INITIAL_TARGET_ERROR_BYTES, MAX_INITIAL_TARGET_PATH_BYTES,
    PANEL_MAPPING_MIN_VERSION, PANEL_MIN_VERSION, PointerKind, SelectionSnapshot,
    SessionBootstrapRequest, TEXT_INPUT_MAX_BYTES, TEXT_INPUT_MIN_VERSION,
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
        InstanceIdentity::for_running_process(
            env!("CARGO_PKG_VERSION"),
            self.instance_name.clone(),
            self.started,
        )
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
        protocol_version: ADVERTISED_PROTOCOL_VERSION,
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
    //
    // Bottom-panel Stage 2B-3: this is where a session's version is
    // actually settled. `Hello` above carried only the compatibility
    // BASELINE (`ADVERTISED_PROTOCOL_VERSION`) — it has to, because a
    // server-first handshake reaches a shipped frontend before that
    // frontend can say anything, and a version it does not recognize is
    // rejected outright. The frontend's counter-offer is therefore the
    // upper half of the negotiation, and this membership test is what
    // bounds it.
    if !crate::protocol::is_supported_protocol_version(req.protocol_version) {
        let _ = write_message(
            &mut stream,
            &InstanceMessage::Goodbye(GoodbyeReason::VersionMismatch {
                // The wire field is "the instance's `PROTOCOL_VERSION`", not
                // the version it advertised. Since Stage 2B-3 those differ:
                // the `Hello` baseline is a compatibility floor, and reporting
                // it here would tell a frontend the daemon tops out at 20 when
                // it in fact speaks 21 — the exact opposite of the upgrade
                // diagnostic this reason exists to give.
                server: pmacs_protocol::PROTOCOL_VERSION,
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

    // The session speaks the lower of the two ceilings. The membership
    // test above already bounds the offer, so this clamp cannot bind
    // today; it is applied through the shared rule anyway so a future
    // ladder widening cannot silently record a version this binary is
    // unable to produce.
    let session_state = crate::presence::SessionState::new(
        crate::protocol::negotiated_session_version(req.protocol_version),
        negotiated_caps,
        color_slot,
    );

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
/// window is just another leaf for them. A semantic session needs the GPU
/// band, which Stage 2B-3 lands — so it is panel-capable exactly when it
/// negotiated a wire that can carry the band. No client-asserted
/// standalone boolean is trusted: the answer is derived from the daemon's
/// own negotiated state.
///
/// **Stage 2B-3 turns the version arm on** (framing §3.5): `panel_capable`
/// is true for an authenticated semantic session that negotiated
/// [`PANEL_MIN_VERSION`] or later, and false for every earlier one. The
/// gate is on *placement*, not only on transport — denying the events
/// while still putting a pre-panel peer's window in a side panel it cannot
/// render would leave that window invisible, so a v6–v20 semantic session
/// keeps the Stage 1 fallback with every side-specific parameter
/// discarded (Q#BP2c).
///
/// The version reaching this predicate is the *negotiated* one, which is
/// the frontend's `AttachRequest` counter-offer rather than the
/// [`ADVERTISED_PROTOCOL_VERSION`](pmacs_protocol::ADVERTISED_PROTOCOL_VERSION)
/// baseline the daemon put in `Hello`. That distinction is the whole
/// activation mechanism: the baseline stays where every shipped frontend
/// can accept it, and only a frontend that named the newer wire itself
/// becomes panel-capable.
fn peer_declared_panel_support(session_state: crate::presence::SessionState) -> bool {
    !session_state.negotiated_capabilities.semantic_render
        || session_state.negotiated_protocol_version >= PANEL_MIN_VERSION
}

/// The same belt-and-braces write-loop gate for the additive
/// protocol-v19 terminal frame. The semantic producer skips construction
/// for an older peer; this filter independently prevents an unknown
/// discriminant reaching one, so neither gate alone is load-bearing.
fn peer_accepts_terminal_message(protocol_version: u32, message: &InstanceMessage) -> bool {
    protocol_version >= 19 || !matches!(message, InstanceMessage::TerminalFrame(_))
}

/// The same belt-and-braces write-loop gate for the additive
/// protocol-v21 panel frame (Q#BP9).
///
/// The producer already skips construction for a peer below
/// [`PANEL_MIN_VERSION`]; this filter independently prevents an unknown
/// discriminant reaching one, so neither gate alone is load-bearing.
fn peer_accepts_panel_message(protocol_version: u32, message: &InstanceMessage) -> bool {
    protocol_version >= PANEL_MIN_VERSION || !matches!(message, InstanceMessage::PanelFrame(_))
}

/// Whether an authenticated source may send the v21 panel event family
/// (Q#BP9's "every gate keys on the daemon's own state").
///
/// All three inbound events require the same four facts, and they are
/// checked together so no arm can satisfy three and forget the fourth:
/// an installed **semantic** projection, a negotiated version that
/// carries the variants, and a `FrontendView` this daemon itself marked
/// panel-capable. A grid session, a pre-panel semantic peer, or a
/// non-panel-capable view is rejected before any payload state is
/// trusted.
///
/// Q#BP16 steps 2–4: the event addresses the panel declaration this
/// session most recently shipped, under the geometry it most recently
/// accepted, and that declaration still describes the panel on screen.
///
/// Four facts, one predicate, because they close four different holes
/// and no three of them subsume the fourth:
///
/// * the latest declaration is a `Present` (an `Absent` cleared input
///   authority, so nothing is addressable),
/// * its echoed `geometry_epoch` equals both the payload's **and** the
///   daemon's latest accepted declaration — the font/scale/resize race,
/// * its `panel_epoch` equals the payload's — close/hide/reopen of the
///   same persistent buffer, which a `buffer_id` alone cannot see,
/// * and the presentation behind it is the side window that is live
///   **now** — because a close/reopen inside one dispatcher burst does
///   not invalidate the shipped declaration, only the window it named
///   (review round 1, R1-2).
fn panel_event_epochs_are_current(
    editor: &EditorState,
    semantic_states: &HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
    source: FrontendId,
    geometry_epoch: u64,
    panel_epoch: u64,
) -> bool {
    let core = editor.core.borrow();
    let live_presentation = core.side_window_for(source).and_then(|window_id| {
        core.windows
            .get(&window_id)
            .map(|window| (window_id, window.buffer_id))
    });
    semantic_states.get(&source).is_some_and(|sem| {
        sem.panel_declaration_matches(geometry_epoch, panel_epoch, live_presentation)
    }) && core
        .frame_geometry_for(source)
        .is_some_and(|geometry| geometry.geometry_epoch == geometry_epoch)
}

/// Project one SEMANTIC frontend's frame, paying any release that
/// projection itself raised before the caller can write the result
/// (parent 48, drain point three).
///
/// **The seam exists because the cancellation happens too deep to pay
/// itself.** A mapping-generation advance cancels the live gesture
/// INSIDE `render_frame`, while the successor `PresentMapped` is still
/// being built, so "before that frame is produced" is not a place that
/// exists. This is the first place that is: projection has returned,
/// and none of what it returned has been written.
///
/// Returning the messages UNWRITTEN is what makes the ordering
/// testable — a caller holding them has, by construction, not yet sent
/// the successor frame, so a release already delivered at that moment
/// provably precedes it.
///
/// Grid sessions do not come here: they hold no panel and no gesture.
fn project_semantic_frame(
    editor: &mut EditorState,
    semantic_states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
    fid: FrontendId,
) -> Vec<InstanceMessage> {
    let messages = semantic_states
        .get_mut(&fid)
        .map(|sem| sem.render_frame(editor))
        .unwrap_or_default();
    drain_pending_release(editor, semantic_states, fid);
    messages
}

/// Deliver the release this frontend is owed, if any (parent 48).
///
/// **Order is the ruling, not just the existence of a slot.** A
/// cancellation raised inside frame production cannot deliver its own
/// release, so the record is parked; this is where it is paid. It runs
/// at three points, and each is chosen against a specific way the
/// release would otherwise arrive too late or not at all:
///
/// * **before any subsequent panel-pointer effect**, so the old
///   gesture's release reaches the child ahead of the new gesture's
///   press rather than after it;
/// * **before detach teardown**, because detach removes the state that
///   holds the record and there is no later opportunity;
/// * **after semantic projection returns and before its messages are
///   written**, so the successor frame cannot overtake the release its
///   own new mapping required.
///
/// The synthetic release carries NO modifiers: nothing is physically
/// held, and inventing a modifier state would report a chord the user
/// never made.
fn drain_pending_release(
    editor: &mut EditorState,
    semantic_states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
    source: FrontendId,
) {
    let Some(record) = semantic_states
        .get_mut(&source)
        .and_then(crate::semantic_render::SemanticRenderState::take_pending_release)
    else {
        return;
    };
    editor.complete_panel_gesture(source, &record, pmacs_protocol::Modifiers::default());
}

/// Parent 48 Q#BP-R4 — the authoritative lifecycle table.
///
/// The disposition is decided BEFORE any target effect, and the live
/// record is consulted before a left tail reaches a child or a
/// selection. That ordering is the point: the old shape validated,
/// classified and mutated in one pass, so an `Up` or `Drag` with no
/// accepted `Down` had already landed by the time the latch was read.
///
/// Exactly one completion per gesture. An `Accepted` release performs
/// the ordinary in-content completion and takes the record; a
/// `Consumed` release did not reach content, so it delivers the
/// RECORDED completion and takes the record. Running both is P5.
fn replay_panel_pointer(
    editor: &mut EditorState,
    semantic_states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
    source: FrontendId,
    buffer_id: crate::buffer::BufferId,
    coord: pmacs_protocol::CellCoord,
    kind: pmacs_protocol::MouseKind,
    mods: pmacs_protocol::Modifiers,
) {
    use crate::editor::PanelPointerOutcome as Outcome;
    use pmacs_protocol::{MouseButton, MouseKind};

    // DRAIN FIRST. An owed release has to reach the child before this
    // gesture's press does; draining afterwards would put them on the
    // wire in the wrong order, which reads to the child as a press
    // followed by a release of the gesture BEFORE it.
    drain_pending_release(editor, semantic_states, source);

    let disposition = editor.classify_panel_pointer(source, buffer_id, coord, kind);
    let outcome = disposition.outcome();
    if outcome == Outcome::Refused {
        // No effect, and the latch is left exactly as it was: a refused
        // event cannot be known to concern the live gesture at all.
        return;
    }
    let live = semantic_states
        .get(&source)
        .is_some_and(crate::semantic_render::SemanticRenderState::has_accepted_gesture);

    match kind {
        MouseKind::Down(MouseButton::Left) => {
            if outcome != Outcome::Accepted {
                // A chrome press begins nothing.
                return;
            }
            // A SECOND PRESS WITH THE FIRST STILL LIVE. The entry drain
            // above saw nothing, because the old gesture had not been
            // cancelled yet --- it was live, not owed. Arming used to do
            // the cancelling, which happens AFTER this press has already
            // reached the target, so the child received
            // `old press, new press, old release`.
            //
            // End the old gesture and PAY it here, before the
            // replacement lands, so the wire carries
            // `old press, old release, new press`.
            if live {
                if let Some(state) = semantic_states.get_mut(&source) {
                    let _ = state.cancel_accepted_gesture();
                }
                drain_pending_release(editor, semantic_states, source);
            }

            // ARMED FROM THE EFFECT RESULT, and only if there was one.
            // `None` means the target refused the press --- a terminal
            // view that is gone, for instance --- and arming over that
            // would record a gesture no tail can deliver.
            let Some(domain) = editor.apply_panel_pointer(source, &disposition, coord, kind, mods)
            else {
                return;
            };
            if let Some(state) = semantic_states.get_mut(&source) {
                state.arm_accepted_gesture(crate::semantic_render::AcceptedPanelGesture {
                    button: MouseButton::Left,
                    coord,
                    buffer_id,
                    domain,
                });
            }
        }
        MouseKind::Drag(MouseButton::Left) => {
            if outcome != Outcome::Accepted || !live {
                // Stale tail, or a drag over chrome: inert. Any live
                // record is retained rather than advanced, because the
                // pointer is not over content.
                return;
            }
            // THE TAIL FOLLOWS THE RECORD, not a fresh classification.
            // The disposition above decided only whether this cell is
            // ours and in content; WHERE the drag goes is the domain the
            // press resolved (G5k).
            let Some(domain) = semantic_states
                .get(&source)
                .and_then(crate::semantic_render::SemanticRenderState::accepted_gesture)
                .map(|record| record.domain)
            else {
                return;
            };
            editor.replay_panel_gesture_in_domain(source, domain, coord, kind, mods);
            if let Some(state) = semantic_states.get_mut(&source) {
                state.note_gesture_content_cell(coord);
            }
        }
        MouseKind::Up(MouseButton::Left) => {
            if !live {
                // A release with no accepted press is inert. Letting it
                // through would send a child tail or mutate a selection
                // for a gesture that never began.
                return;
            }
            match outcome {
                Outcome::Accepted => {
                    // In content, so the ordinary completion runs --- but
                    // still in the RECORDED domain, or a mid-gesture mode
                    // flip would route this release away from the target
                    // that received the press (G5k).
                    let record = semantic_states.get_mut(&source).and_then(
                        crate::semantic_render::SemanticRenderState::consume_accepted_gesture,
                    );
                    if let Some(record) = record {
                        // Taken WITHOUT counting a cancellation, and
                        // exactly one completion: this is the ordinary
                        // one, so `complete_panel_gesture` must not also
                        // run (P5).
                        editor.replay_panel_gesture_in_domain(
                            source,
                            record.domain,
                            coord,
                            kind,
                            mods,
                        );
                    }
                }
                Outcome::Consumed => {
                    // The release landed on chrome, so the content path
                    // never ran. Terminate from the record, at its last
                    // valid content cell.
                    let record = semantic_states.get_mut(&source).and_then(
                        crate::semantic_render::SemanticRenderState::consume_accepted_gesture,
                    );
                    if let Some(record) = record {
                        editor.complete_panel_gesture(source, &record, mods);
                    }
                }
                Outcome::Refused => unreachable!("refused returned above"),
            }
        }
        _ => {
            // Every other kind: a one-shot content effect, and it never
            // touches the left-gesture latch.
            if outcome == Outcome::Accepted {
                let _ = editor.apply_panel_pointer(source, &disposition, coord, kind, mods);
            }
        }
    }
}

/// §5b — which panel-pointer family this session speaks.
///
/// **Read from the AUTHENTICATED source, never from the payload's
/// `frontend_id`.** That field is untrusted on every inbound variant,
/// and looking the negotiation up by it would let a peer claim another
/// session's family — the forged cross-session claim G8e mutates for.
///
/// Consulted BEFORE the payload is trusted, before any generation is
/// validated and before any mutation: the family decides which variant
/// is even admissible, so it cannot depend on the variant's contents.
///
/// **Re-homed here.** This paragraph documented THIS function but sat
/// above `update_accepted_gesture`, whose own doc followed it in the
/// same block. Deleting that function's doc with it made the
/// misplacement visible.
fn peer_uses_mapped_panel_family(session_registry: &SessionRegistry, source: FrontendId) -> bool {
    session_registry
        .session_state(source)
        .is_some_and(|state| state.negotiated_protocol_version >= PANEL_MAPPING_MIN_VERSION)
}

/// §5b — whether an echoed mapping generation still names the mapping
/// the daemon holds.
///
/// The last rung of the ladder and the finest: `buffer_id` catches an
/// A→B replacement, `panel_epoch` a close/reopen, `geometry_epoch` a
/// declaration race, and this catches **the text under that cell
/// changing** — a foreign edit, a fold, a reload, none of which moves
/// an epoch.
///
/// **Zero is refused outright, and BEFORE the wheel exemption.** Zero is
/// what a default-constructed or half-initialised sender produces, so
/// accepting it would let a peer opt out of the check by sending
/// nothing. Ordering the exemption first would reopen that opt-out
/// through the exempt path: a sender emitting zeroed wheels would face
/// no check at all (G10b).
///
/// **Coordinate-free wheels are then EXEMPT from the freshness
/// comparison.** A wheel tick changes `view_top`, which advances the
/// key; the next tick already queued behind it echoes the previous
/// generation and would be refused, so the panel would scroll exactly
/// once per frame and appear dead. The exemption is safe because the
/// coordinate is not what a wheel means — the tick is. It returns
/// before the read, so a wheel does not advance the key either;
/// advancing would make the wheel invalidate the press after it.
///
/// The carve-out the framing states for **child-reported** terminal
/// wheels, where SGR does carry row and column, is
/// `panel-pointer-replay`'s. Whether a wheel is forwarded to a child is
/// decided by the reporting mode, and no panel pointer coordinate is
/// consumed on this base at all — `dispatch_semantic_panel_pointer`
/// bounds-checks and routes focus. Re-imposing the check belongs in the
/// branch that introduces the forwarding it protects.
///
/// Read through the SAME accessor projection stamps with, so "what the
/// frontend was shown" and "what the daemon checks" cannot drift.
fn panel_mapping_is_current(
    editor: &EditorState,
    semantic_states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
    source: FrontendId,
    kind: pmacs_protocol::MouseKind,
    echoed: u64,
) -> bool {
    // ORDER IS LOAD-BEARING: nonzero first, exemption second.
    if echoed == 0 {
        return false;
    }
    if matches!(
        kind,
        pmacs_protocol::MouseKind::ScrollUp
            | pmacs_protocol::MouseKind::ScrollDown
            | pmacs_protocol::MouseKind::ScrollLeft
            | pmacs_protocol::MouseKind::ScrollRight
    ) {
        return true;
    }
    let snapshot = editor.panel_mapping_snapshot(source);
    semantic_states
        .get_mut(&source)
        .and_then(|state| state.panel_mapping_generation(snapshot))
        .is_some_and(|current| current == echoed)
}

/// Whether an authenticated source may send the v21 panel event family
/// (Q#BP9's "every gate keys on the daemon's own state").
///
/// All three inbound events require the same three facts, and they are
/// checked together so no arm can satisfy two and forget the third: an
/// installed **semantic** projection, a negotiated version that carries
/// the variants, and a `FrontendView` this daemon itself marked
/// panel-capable. A grid session, a pre-panel semantic peer, or a
/// non-panel-capable view is rejected before any payload state is
/// trusted.
///
/// The claimed `frontend_id` in the payload is never consulted anywhere:
/// routing is by the authenticated transport `source`, so a forged id
/// addresses nothing.
fn peer_may_send_panel_events(
    editor: &EditorState,
    session_registry: &SessionRegistry,
    semantic_states: &HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
    source: FrontendId,
) -> bool {
    semantic_states.contains_key(&source)
        && session_registry
            .session_state(source)
            .is_some_and(|state| state.negotiated_protocol_version >= PANEL_MIN_VERSION)
        && editor.core.borrow().panel_capable_for(source)
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
                    let active_now = document_buffer_to_follow(editor, *fid);
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
            let messages = if semantic_states.contains_key(fid) {
                project_semantic_frame(editor, &mut semantic_states, *fid)
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
                // Q#MB1 / Discovery Stage 2 — the minibuffer is the one
                // surface with TWO live variants, and the gate is a
                // RANGE on both sides rather than a floor. The legacy
                // `MinibufferPrompt` is frozen and belongs to `12..=22`;
                // `MinibufferPromptRows` belongs to `>= 23`. Writing the
                // legacy gate as a bare `>= 12` would let a v23 peer
                // receive both and double-render its dropdown.
                let peer_knows_minibuffer_prompt =
                    session_registry.session_state(*fid).is_some_and(|s| {
                        (12..crate::semantic_render::MINIBUFFER_ROWS_MIN_VERSION)
                            .contains(&s.negotiated_protocol_version)
                    });
                let peer_knows_minibuffer_rows =
                    session_registry.session_state(*fid).is_some_and(|s| {
                        s.negotiated_protocol_version
                            >= crate::semantic_render::MINIBUFFER_ROWS_MIN_VERSION
                    });
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
                    // simply can't render the GUI minibuffer. Discovery
                    // Stage 2 closed the range at the top: a v23 peer
                    // gets the rows form instead, never both.
                    if !peer_knows_minibuffer_prompt
                        && matches!(msg, InstanceMessage::MinibufferPrompt { .. })
                    {
                        continue;
                    }
                    // Discovery Stage 2 — MinibufferPromptRows gated at
                    // v23. A `12..=22` peer keeps the frozen legacy
                    // variant above, which is why gating alone was never
                    // enough: with one variant it would have lost the
                    // minibuffer entirely.
                    if !peer_knows_minibuffer_rows
                        && matches!(msg, InstanceMessage::MinibufferPromptRows { .. })
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
                    // Long lines — LineWrapFacts gated at v22. A v21 peer
                    // keeps whatever it does today; the semantic producer
                    // also skips it, so this is the belt-and-braces half.
                    if negotiated_protocol_version < 22
                        && matches!(msg, InstanceMessage::LineWrapFacts { .. })
                    {
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
                    // Bottom panel Q#BP9 — PanelFrame gated at v21. A v20
                    // peer receives no band and, per Q#BP13, is never
                    // placed in a side window either.
                    if !peer_accepts_panel_message(negotiated_protocol_version, msg) {
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
                    && let Some((buffer_id, byte_pos)) = document_cursor_byte(editor, *fid)
                {
                    let cursor_byte_msg = InstanceMessage::CursorByte {
                        buffer_id,
                        byte_pos,
                    };
                    if let Err(e) = write_message(stream, &cursor_byte_msg) {
                        eprintln!("pmacs: write CursorByte for {fid:?} failed: {e}");
                        write_failed = true;
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
        sync_terminal_layouts_for_tick(editor, &attached_fids, &term_sizes, &semantic_states);

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
    let (origin_window, resolved) = {
        let mut core = editor.core.borrow_mut();
        core.active_frontend = frontend_id;
        let origin_window = core
            .primary_document_window(frontend_id)
            .ok_or_else(|| "attaching frontend has no document window".to_string())?;
        let resolved = core.resolve_target_buffer(&path)?;
        (origin_window, resolved)
    };

    // Journey Stage 1a (Q#JR6/Q#JR9): a DIRECTORY installs nothing.
    //
    // Nothing can be installed, because the listing that satisfies a
    // directory open is asynchronous and this block is synchronous — the
    // frontend is blocked on `InitialTargetResult` and will not create
    // its window until it arrives, so there is no tick in which a
    // listing could settle. The reply therefore names the buffer the
    // fresh view's document window ALREADY holds, which is a valid,
    // ready session; the listing replaces it a tick or more later.
    //
    // That buffer is NOT necessarily `*scratch*`: `build_fresh_frontend_view`
    // clones LOCAL's primary document buffer. If LOCAL holds a real
    // document, this session briefly displays and snapshots it. Accepted
    // and documented rather than papered over with a placeholder buffer,
    // which would need reaping and would be fought by the reassert below.
    //
    // `publish_to_replicas` is false for the same reason an `AfterSwitch`
    // dedup sets it false: this buffer is pre-existing and already
    // published, not freshly loaded here.
    let (buffer_id, fire) = match resolved {
        crate::editor_core::ResolvedTarget::Directory { path } => {
            let dest = editor
                .capture_view_destination(frontend_id, origin_window)
                .ok_or_else(|| format!("cannot open {}: no document window", path.display()))?;
            editor.dispatch_directory_open(&path, dest);
            editor.reconcile_panel_layout(frontend_id);

            // The reply must name what the window ACTUALLY holds now, not
            // what it held before the dispatch.
            //
            // The chain runs synchronously. dired's handler defers (it
            // spawns a coroutine for the listing), but a user's resolver
            // is under no such obligation: a handler that opens something
            // synchronously -- through `commit_to`, which is exactly the
            // supported way to do it -- has already replaced this
            // window's buffer by the time we get here. Reporting the
            // captured id would then send the snapshot of one buffer and
            // the identity of another, and the frontend would render a
            // document nobody asked for.
            //
            // Re-reading also covers the case a hook closed the window,
            // which is why this rehomes through `non_side_target` exactly
            // as the file arm's reassert does rather than returning early
            // and skipping that check.
            let mut core = editor.core.borrow_mut();
            core.active_frontend = frontend_id;
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
            core.focus_window(frontend_id, destination);
            let buffer_id = core
                .windows
                .get(&destination)
                .map(|window| window.buffer_id)
                .ok_or_else(|| format!("cannot reselect {}: window died", path.display()))?;
            return Ok(OpenedInitialTarget {
                buffer_id,
                // False whether or not the chain replaced the buffer: an
                // untouched destination is pre-existing and already
                // published, and a buffer a synchronous handler installed
                // went through the ordinary display path, which publishes
                // on its own terms.
                publish_to_replicas: false,
            });
        }
        crate::editor_core::ResolvedTarget::Buffer { id, fire } => (id, fire),
    };

    {
        let mut core = editor.core.borrow_mut();
        core.install_buffer_in_window(origin_window, buffer_id)
            .map_err(|error| format!("cannot select {}: {error}", path.display()))?;
        core.focus_window(frontend_id, origin_window);
    }

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
    // negotiated state in this same transaction. Stage 2B-3 made the
    // semantic arm live: a semantic session that negotiated
    // `PANEL_MIN_VERSION` or later can render the GPU band and is
    // panel-capable, while a v6-v20 semantic session still falls back to
    // its document target with every side-specific parameter discarded.
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
    // reach it. A semantic session deliberately stays UNKNOWN — its
    // authenticated `FrontendCellGeometry` fills it, and the permanent
    // 24x80 attach placeholder is never consulted for panel layout.
    //
    // Stage 2B-2: the gate is `!semantic_render`, NOT `panel_capable`.
    // Stage 1 could conflate them because panel capability implied grid;
    // once a semantic session can be panel-capable, a capability-keyed
    // gate would feed it exactly the placeholder Q#BP15a forbids, and
    // parent acceptance 40 would fail through this line rather than
    // through the projection.
    if !semantic_render && editor.core.borrow().panel_capable_for(frontend_id) {
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
                    //
                    // Stage 2B-2: gated on the absence of a semantic
                    // projection as well as on capability. A semantic
                    // frontend's `Resize` describes its own surface in
                    // whatever units it chose; only `FrontendCellGeometry`
                    // is its authoritative cell-equivalent declaration
                    // (Q#BP15a), and letting `Resize` mint an epoch here
                    // would let the two allocators interleave.
                    if !semantic_states.contains_key(&source)
                        && editor.core.borrow().panel_capable_for(source)
                    {
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
                    // Bottom-panel §1.3 #9 — Projection. The gate asks
                    // "is this frontend's DOCUMENT surface a terminal",
                    // so it tests the primary document window. A focused
                    // TERMINAL PANEL must not suppress the still-visible
                    // document's viewport.
                    let terminal_context = {
                        let manager = editor.terminal_manager.borrow();
                        let core = editor.core.borrow();
                        let active = core
                            .primary_document_buffer(source)
                            .is_some_and(|document| manager.is_terminal(document));
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
                        // Bottom-panel §1.3 #7 — Projection, and it must
                        // NOT move focus. Routing this through the
                        // focused window would let an ordinary document
                        // viewport overwrite a focused panel's buffer.
                        align_primary_document_window(editor, source, buffer_id);
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
                FrontendEvent::FrontendCellGeometry {
                    geometry_epoch,
                    total,
                    ..
                } => {
                    // Bottom panel Q#BP15a — the frontend's authoritative
                    // cell-equivalent layout capacity. Routed by the
                    // authenticated `source`; the payload's `frontend_id`
                    // is never read.
                    //
                    // Deliberately does NOT require a side window: the
                    // daemon needs columns before it can paint a first
                    // panel frame, so gating this on panel presence would
                    // deadlock the first open. "Without a side window"
                    // refers to side-window presence only — the protocol,
                    // session, and capability gates all still apply.
                    if peer_may_send_panel_events(editor, session_registry, semantic_states, source)
                    {
                        editor.accept_semantic_frame_geometry(source, geometry_epoch, total);
                    }
                }
                FrontendEvent::PanelResizeRows {
                    geometry_epoch,
                    panel_epoch,
                    rows,
                    ..
                } => {
                    // Bottom panel Q#BP15a / Q#BP16 — a divider drag's
                    // requested rows. Accepted only against the currently
                    // visible `Present` declaration, matching BOTH the
                    // latest accepted frontend geometry and that
                    // declaration's presentation epoch, so a drag racing a
                    // font change or a panel reopen cannot resize its
                    // successor.
                    if peer_may_send_panel_events(editor, session_registry, semantic_states, source)
                        && panel_event_epochs_are_current(
                            editor,
                            semantic_states,
                            source,
                            geometry_epoch,
                            panel_epoch,
                        )
                    {
                        editor.apply_panel_resize_rows(source, rows);
                    }
                }
                FrontendEvent::PanelPointer {
                    geometry_epoch,
                    panel_epoch,
                    buffer_id,
                    coord,
                    kind,
                    // Parent 48 R-a: modifiers are NOT decoration here.
                    // `apply_terminal_gesture` gates child reporting on
                    // `!shift`, so Shift is the user's "select locally
                    // instead of talking to the child" override, and the
                    // document path reads Shift to extend the selection.
                    // Dropping them into `..` inverted both.
                    mods,
                    ..
                } => {
                    // Bottom panel Q#BP16 — a gesture the frontend
                    // hit-tested to a panel CELL. Steps 1, 3, and 4 of the
                    // ladder are checked here (authenticated source, both
                    // epochs against the declaration the frontend was
                    // looking at); steps 2, 5, and 6 are re-derived from
                    // the daemon's own state inside the dispatcher. Any
                    // failure drops the event before any view, controller,
                    // selection, menu, or PTY mutation.
                    // §5b G8a — the family is decided FIRST, from the
                    // AUTHENTICATED session, and a `>= v25` session
                    // sending the bare variant is REFUSED rather than
                    // handled under legacy semantics. Handling it would
                    // leave the mapping hole reachable by choosing a
                    // discriminant, which is the whole of the bypass.
                    if !peer_uses_mapped_panel_family(session_registry, source)
                        && peer_may_send_panel_events(
                            editor,
                            session_registry,
                            semantic_states,
                            source,
                        )
                        && panel_event_epochs_are_current(
                            editor,
                            semantic_states,
                            source,
                            geometry_epoch,
                            panel_epoch,
                        )
                    {
                        // THE LATCH FOLLOWS THE DISPATCH. The ladder
                        // above authenticates the SENDER; the dispatcher
                        // re-derives the TARGET and refuses an
                        // out-of-grid coordinate, an absent side window,
                        // or a buffer that is no longer the one shown
                        // there. Those refusals are not hypothetical --
                        // a stale coordinate outlives the frame it was
                        // hit-tested against.
                        //
                        // Arming on a refusal lets a rejected press
                        // manufacture a cancellation, and -- once replay
                        // attaches effects -- a release for a child that
                        // was never pressed. Consuming on a refusal is
                        // worse: a rejected release swallows a REAL
                        // armed gesture, so the authority loss that
                        // should have ended it finds nothing, and the
                        // child holds the button down forever.
                        replay_panel_pointer(
                            editor,
                            semantic_states,
                            source,
                            buffer_id,
                            coord,
                            kind,
                            mods,
                        );
                    }
                }
                FrontendEvent::PanelPointerMapped {
                    geometry_epoch,
                    panel_epoch,
                    buffer_id,
                    coord,
                    kind,
                    mapping_generation,
                    // Bound for the same reason as the legacy arm above,
                    // and NOT optional here: the mapped family carries
                    // the same modifiers, so leaving them in `..` would
                    // give a v25 session the inverted Shift behaviour
                    // that parent 48 R-a fixed for v24.
                    mods,
                    ..
                } => {
                    // §5b — the mapped family, in this order:
                    //
                    //   1. family, from the AUTHENTICATED session
                    //   2. the existing epoch ladder
                    //   3. the mapping generation
                    //   4. only then, dispatch
                    //
                    // G8c is the first line: a `<= v24` session sending
                    // this variant is refused even though a peer built
                    // from this crate can encode the discriminant.
                    // Negotiation is a gate, not a sender convention.
                    if peer_uses_mapped_panel_family(session_registry, source)
                        && peer_may_send_panel_events(
                            editor,
                            session_registry,
                            semantic_states,
                            source,
                        )
                        && panel_event_epochs_are_current(
                            editor,
                            semantic_states,
                            source,
                            geometry_epoch,
                            panel_epoch,
                        )
                        && panel_mapping_is_current(
                            editor,
                            semantic_states,
                            source,
                            kind,
                            mapping_generation,
                        )
                    {
                        // The latch follows the dispatch, for the
                        // reason spelled out on the legacy arm above.
                        // The mapping rung narrows WHICH coordinates
                        // survive the ladder; it does not make a
                        // surviving one land, so this arm needs the same
                        // gate.
                        replay_panel_pointer(
                            editor,
                            semantic_states,
                            source,
                            buffer_id,
                            coord,
                            kind,
                            mods,
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
                        // Bottom-panel §1.3 #8 — Projection + focus. A
                        // click in the DOCUMENT area means "work here",
                        // so unlike `Viewport` (#7) this one also takes
                        // focus out of a panel.
                        align_and_activate_primary_document_window(editor, source, buffer_id);
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
                FrontendEvent::TextInput { text, .. } => {
                    // GUI arc 1a / A5 — committed text from a keypress
                    // or an IME composition. Handled here, beside
                    // `Paste`, for the same two reasons: the
                    // authenticated `source` is in scope (the event's
                    // own `frontend_id` is client-supplied and not
                    // trusted), and the semantic input dispatcher would
                    // otherwise drop it, which is exactly how GPU
                    // Ctrl-V was a no-op before Q#KR10a.
                    //
                    // A9 — the cap is enforced at the BOUNDARY, before
                    // anything is inserted. Rejected, never truncated:
                    // cutting UTF-8 at a byte offset corrupts the last
                    // character, and a silently-shortened insert is
                    // worse than a refused one. An empty payload is
                    // dropped too — it would be an edit that edits
                    // nothing, and would still cost an undo unit.
                    // **The producer gate is only half the contract.**
                    // A frontend that negotiated v6–v23 can still encode
                    // this variant — it is compiled from the same crate,
                    // and postcard will happily write the discriminant —
                    // so a peer that never declared v24 could otherwise
                    // mutate the buffer through a variant its own
                    // session does not include. Gate on the
                    // AUTHENTICATED session's negotiated version, not on
                    // the payload and not on what the daemon supports.
                    let peer_declared_text_input = session_registry
                        .session_state(source)
                        .is_some_and(|s| s.negotiated_protocol_version >= TEXT_INPUT_MIN_VERSION);
                    if !peer_declared_text_input {
                        eprintln!(
                            "pmacs: dropping TextInput from {source:?}, which negotiated \
                             below v{TEXT_INPUT_MIN_VERSION}"
                        );
                        return;
                    }
                    if text.is_empty() {
                        return;
                    }
                    if text.len() > TEXT_INPUT_MAX_BYTES {
                        eprintln!(
                            "pmacs: rejecting oversize TextInput from {source:?} \
                             ({} bytes > {TEXT_INPUT_MAX_BYTES})",
                            text.len()
                        );
                        return;
                    }
                    editor.dispatch_text_input(source, &text);
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
            // Parent 48 G5b/G5i — DETACH is the fifth authority loss,
            // and the only one with no later chance at all. End the live
            // gesture, then pay it, and do BOTH before any teardown: the
            // next line drops the state holding the record, and
            // `detach_frontend_input` below releases the terminal
            // controller the release still needs.
            if let Some(state) = semantic_states.get_mut(&frontend_id) {
                let _ = state.cancel_accepted_gesture();
            }
            drain_pending_release(editor, semantic_states, frontend_id);
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
    // Bottom-panel §1.3 #2 — Projection, and the sharpest case in the
    // census. The upgrade BROADCASTS a `BufferSnapshot` to every
    // replica, so keying it on focus would mean focusing a fresh
    // generated panel buffer swaps every peer's document mirror to it.
    // A panel buffer that genuinely needs CRDT backing gets it when it
    // is displayed as a document, not as a side effect of focus.
    let buffer_id_opt = {
        let core = editor.core.borrow();
        core.primary_document_buffer(fid)
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
            let displays_buffer = peer_displays_buffer_as_document(editor, *peer_id, buffer_id);
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
/// The buffer a semantic frontend DISPLAYS AS ITS DOCUMENT — the
/// buffer-follow / `BufferSnapshot` re-send target (bottom-panel §1.3
/// #1, Projection).
///
/// Not the focused buffer: focusing a panel must re-send no snapshot and
/// must never swap the replica's document mirror. Named as its own
/// function so the rule is pinnable — its only caller is
/// `dispatcher_loop`, which no test can drive.
#[cfg(feature = "crdt")]
fn document_buffer_to_follow(
    editor: &EditorState,
    fid: FrontendId,
) -> Option<crate::buffer::BufferId> {
    editor.core.borrow().primary_document_buffer(fid)
}

/// The `(buffer, byte)` a semantic replica's authoritative `CursorByte`
/// describes (bottom-panel §1.3 #3, Projection).
///
/// Q#BP14's vocabulary split: "active buffer" in the replica is a
/// DOCUMENT-SURFACE term, not an input-focus term, so a focused panel
/// must not retarget the document caret at the panel's buffer.
fn document_cursor_byte(
    editor: &EditorState,
    fid: FrontendId,
) -> Option<(crate::buffer::BufferId, u64)> {
    let core = editor.core.borrow();
    let win_id = core.primary_document_window(fid)?;
    let window = core.windows.get(&win_id)?;
    Some((window.buffer_id, window.cursor))
}

/// Whether `peer_id` displays `buffer_id` on its DOCUMENT surface — the
/// `BufferSnapshot` publication recipient filter (bottom-panel §1.3 #21,
/// Projection).
///
/// Testing the focused window instead would both miss a buffer visible
/// in the document while a panel holds focus, and replace the peer's
/// document mirror for a buffer visible only in a panel.
fn peer_displays_buffer_as_document(
    editor: &EditorState,
    peer_id: FrontendId,
    buffer_id: crate::buffer::BufferId,
) -> bool {
    editor.core.borrow().primary_document_buffer(peer_id) == Some(buffer_id)
}

/// Align a semantic frontend's **primary document window** to the
/// buffer it declared (bottom-panel §1.3 #7, Q#BP14).
///
/// **Never touches `view.active`.** This is why rejecting panel-named
/// events does not fix the *document* event: with a panel focused, an
/// ordinary document `Viewport` routed through the focused window would
/// overwrite the panel's buffer with the document buffer. Returns the
/// window it aligned so the `Pointer` path (#8) can activate it.
fn align_primary_document_window(
    editor: &mut EditorState,
    fid: FrontendId,
    buffer_id: crate::buffer::BufferId,
) -> Option<crate::window::WindowId> {
    use crate::text_view::TextView;

    let (win_id, text_view) = {
        let core = editor.core.borrow();
        let win_id = core.primary_document_window(fid)?;
        if core.windows.get(&win_id).map(|w| w.buffer_id) == Some(buffer_id) {
            return Some(win_id); // Already displaying this buffer.
        }
        let reg = core.registry.borrow();
        let Ok(buf) = reg.get(buffer_id) else {
            // Unknown buffer — leave the window as-is, and report
            // FAILURE. Returning the window here would let a stale or
            // forged `Pointer` naming a dead buffer take focus out of a
            // panel via #8's activation, *before* `dispatch_pointer`
            // rejects the mismatched buffer. Alignment did not happen,
            // so no caller may treat this as a document gesture.
            return None;
        };
        (win_id, TextView::new(buf))
    };

    let mut core = editor.core.borrow_mut();
    if let Some(win) = core.windows.get_mut(&win_id) {
        win.buffer_id = buffer_id;
        win.text_view = text_view;
        win.cursor = 0;
        win.selection = None;
        win.overlays.clear();
    }
    Some(win_id)
}

/// Align the primary document window **and take focus to it**
/// (bottom-panel §1.3 #8, Q#BP14).
///
/// A click in the document area means "work here", so it moves focus
/// out of a panel. This is the one place projection and focus
/// legitimately move together — every other Projection consumer must
/// use [`align_primary_document_window`] alone.
fn align_and_activate_primary_document_window(
    editor: &mut EditorState,
    fid: FrontendId,
    buffer_id: crate::buffer::BufferId,
) {
    if let Some(win_id) = align_primary_document_window(editor, fid, buffer_id) {
        editor.core.borrow_mut().focus_window(fid, win_id);
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

/// One dispatcher tick's terminal-layout step, for every attached frontend.
///
/// Extracted from the dispatcher loop so the grid/semantic exclusivity is
/// **structural** rather than two adjacent `if`s, and so acceptance tests can
/// drive the real loop body instead of re-implementing it (Q#GT1).
///
/// The shape that matters: liveness is frontend-kind NEUTRAL and runs for
/// everyone, exactly once; the geometry arms are EXCLUSIVE alternatives keyed
/// on the same `semantic_states` membership that session establishment uses,
/// so a session can never be caught by both.
///
/// Before this existed, both arms ran for every frontend. A semantic session
/// has a `term_sizes` entry (from `AttachRequest`) *and* a terminal
/// declaration, so its PTY was resized twice per tick, forever: the grid arm
/// installed the TUI placement size, the semantic arm installed the declared
/// content rectangle, and each arm's own idempotence guard saw only the size
/// the other had just written. The child got a `SIGWINCH` storm at tick
/// cadence, which is what made typing into a GPU terminal impossible while
/// output kept flowing.
fn sync_terminal_layouts_for_tick(
    editor: &mut EditorState,
    attached_fids: &[FrontendId],
    term_sizes: &HashMap<FrontendId, CellSize>,
    semantic_states: &HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
) {
    for frontend_id in attached_fids {
        // Neutral half: panel reconciliation (Q#BP2b's only per-tick
        // enforcement point) and the release of a controller whose window
        // moved away. A semantic frontend gets this from nowhere else —
        // its own arm stops running the moment the buffer-follow snapshot
        // clears the declaration (Q#GT4/Q#GT7).
        editor.sync_terminal_controller_liveness(*frontend_id);

        // Geometry: exactly one arm per frontend kind.
        if let Some(state) = semantic_states.get(frontend_id) {
            // Vterm Stage 3 — the frontend declared a CONTENT rectangle,
            // so this consumes the size directly instead of running the
            // TUI placement helper, which would subtract a modeline the
            // GPU never drew. A semantic frontend with no declaration yet
            // gets NO resize at all, which is correct: the terminal keeps
            // the geometry it was opened with until one arrives.
            if let Some((buffer_id, size)) = state.terminal_viewport() {
                editor.sync_semantic_terminal_layout(*frontend_id, buffer_id, size);
            }
            // Bottom-panel R1-3: the band is the semantic frontend's
            // OTHER terminal surface, and it has no declaration to
            // consult — the daemon derives its geometry (Q#BP15a). The
            // grid arm below gets this for free because it resolves
            // through `controller_view_for_frontend`, which is
            // window-agnostic; the semantic arm above is keyed to the
            // document declaration and structurally cannot see a side
            // window. Both calls target disjoint windows, so this is a
            // second CASE, not a second resize of the same child.
            editor.sync_semantic_panel_terminal_layout(*frontend_id);
        } else if let Some(size) = term_sizes.get(frontend_id).copied() {
            editor.sync_terminal_grid_geometry(*frontend_id, size);
        }
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
        // Everything else, including BOTH panel-pointer families. Panel
        // gestures are dispatched from their own arm in
        // `handle_dispatcher_event`, behind the epoch ladder; reaching
        // them from here would route around it. §5b's mapped variant is
        // dropped for the same reason, and specifically NOT unwrapped to
        // its legacy meaning — that is the bypass the family gate exists
        // to close.
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
        // Q#KR10a (Paste) and GUI arc 1a (TextInput) — both are handled
        // in the dispatcher's own arms, unified for grid and semantic
        // sessions and keyed by the AUTHENTICATED source, and neither
        // reaches here. Listed explicitly rather than left to a
        // wildcard so a future reshuffle cannot silently re-route
        // either through this payload-trusting path: both carry a
        // client-supplied `frontend_id` this function would believe.
        FrontendEvent::Paste { .. }
        | FrontendEvent::TextInput { .. }
        // §5b: listed here for the same reason as the two above — a
        // mapped panel gesture must not reach a payload-trusting path.
        // Its own dispatcher arm authenticates the source and checks
        // the family gate; arriving here it is dropped, never unwrapped
        // to the legacy family.
        | FrontendEvent::PanelPointerMapped { .. }
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
        FrontendEvent::FrontendCellGeometry { .. }
        | FrontendEvent::PanelResizeRows { .. }
        | FrontendEvent::PanelPointer { .. } => {
            // Bottom panel Stage 2 — panel declarations belong to
            // negotiated panel-capable semantic sessions and are routed
            // by the authenticated source in `handle_dispatcher_event`.
            // A grid session has no panel band at all, so one arriving
            // here is a protocol violation; drop it rather than letting
            // a payload-trusted id reach a view.
            //
            // Stage 2B-2 added the real routing arms, which `peer_may_
            // send_panel_events` already refuses for a grid session, so
            // this is now the belt-and-braces half of the same gate —
            // exactly like the `Pointer` and `TerminalResize` arms above.
            eprintln!(
                "pmacs daemon: panel declaration from a grid session; dropping \
                 (grid sessions negotiate no panel band)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PROTOCOL_VERSION;

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

    /// Bottom-panel §1.3 #21 through the REAL producer (round 3).
    ///
    /// A semantic peer with a FOCUSED PANEL must still receive the
    /// snapshot for the buffer on its DOCUMENT surface, and must NOT
    /// receive one for a buffer visible only in its panel. Asserting the
    /// helper alone was insufficient: reverting the producer's call site
    /// to focused-window routing left every helper-level test green.
    #[cfg(feature = "crdt")]
    #[test]
    fn snapshot_publication_follows_the_document_under_a_focused_panel() {
        let (editor, fid, document, panel) = panel_focused_semantic_fixture();
        let (doc_buf, panel_buf) = {
            let core = editor.core.borrow();
            (
                core.windows[&document].buffer_id,
                core.windows[&panel].buffer_id,
            )
        };
        assert_ne!(doc_buf, panel_buf, "fixture: distinct buffers");

        let caps = crate::protocol::NegotiatedCapabilities {
            multi_frontend: true,
            crdt_replica: true,
            semantic_render: true,
        };
        let mut registry = SessionRegistry::new();
        registry.register_session(
            fid,
            crate::presence::SessionState::new(PROTOCOL_VERSION, caps, 0),
        );

        // The DOCUMENT buffer's snapshot must be delivered.
        {
            let (server, mut client) = UnixStream::pair().expect("socketpair");
            // A read timeout on the DELIVERY read too. Without it a
            // regression that suppresses the snapshot makes this test
            // HANG rather than fail, which is strictly worse than a red
            // assertion — found by biting this very test.
            client
                .set_read_timeout(Some(Duration::from_millis(500)))
                .expect("delivery timeout");
            let mut streams = HashMap::from([(fid, server)]);
            let message = InstanceMessage::BufferSnapshot {
                buffer_id: doc_buf,
                crdt_snapshot: vec![1, 2, 3],
            };
            publish_buffer_snapshot_to_replicas(
                &editor,
                doc_buf,
                &message,
                &registry,
                &mut streams,
                &mut HashMap::new(),
            );
            let delivered: InstanceMessage =
                read_message(&mut client).expect("the document snapshot must arrive");
            assert_eq!(
                delivered, message,
                "#21: a buffer on the DOCUMENT surface must still be published while a \
                 panel holds focus"
            );
        }

        // The PANEL-only buffer's snapshot must NOT be delivered.
        {
            let (server, mut client) = UnixStream::pair().expect("socketpair");
            let mut streams = HashMap::from([(fid, server)]);
            let message = InstanceMessage::BufferSnapshot {
                buffer_id: panel_buf,
                crdt_snapshot: vec![4, 5, 6],
            };
            publish_buffer_snapshot_to_replicas(
                &editor,
                panel_buf,
                &message,
                &registry,
                &mut streams,
                &mut HashMap::new(),
            );
            client
                .set_read_timeout(Some(Duration::from_millis(50)))
                .expect("timeout");
            assert!(
                read_message::<InstanceMessage>(&mut client).is_err(),
                "#21: a buffer visible only in a PANEL must not replace the peer's \
                 document mirror"
            );
        }
    }

    // ---- GPU terminal input: the double terminal-layout sync -------------
    //
    // These drive `sync_terminal_layouts_for_tick` — the REAL dispatcher loop
    // body, not a re-implementation of it. That distinction is the whole
    // point: the Stage 3 acceptance sent input through `client.send_key`
    // directly and therefore pinned transport rather than routing, which is
    // how the defect these pin shipped.
    //
    // The observable is `TerminalScreen::generation`. It advances once per
    // screen mutation, so with a child that produces no output and no
    // `tick_processes` call, "generation stopped advancing" is exactly "the
    // geometry settled" — a state predicate, not a readout.

    /// Open a quiet terminal and give `frontend_id` a view that shows it,
    /// holding its controller — the state the dispatcher loop runs against.
    fn quiet_terminal_for(
        editor: &EditorState,
        frontend_id: FrontendId,
    ) -> (crate::buffer::BufferId, crate::window::WindowId) {
        let mut spec = crate::terminal::TerminalSpec::new("/bin/sh");
        spec.args = vec!["-c".into(), "sleep 30".into()];
        spec.rows = 24;
        spec.cols = 80;
        let buffer_id = editor
            .terminal_manager
            .borrow_mut()
            .open(
                spec,
                &mut editor.core.borrow_mut(),
                &mut editor.process_supervisor.borrow_mut(),
            )
            .expect("open terminal");

        let window_id = crate::window::WindowId::next();
        {
            let mut core = editor.core.borrow_mut();
            let text_view = {
                let registry = core.registry.clone();
                let registry = registry.borrow();
                let buffer = registry.get(buffer_id).expect("terminal buffer");
                crate::text_view::TextView::new(buffer)
            };
            core.windows.insert(
                window_id,
                crate::window::Window::new(window_id, buffer_id, text_view),
            );
            core.register_frontend_view(
                frontend_id,
                crate::window::FrontendView {
                    layout: crate::window::Layout::single(window_id),
                    active: window_id,
                    fold_projection: true,
                    panel_capable: false,
                    frame_geometry: None,
                    panel_hidden: false,
                },
            );
        }
        let key = crate::terminal::TerminalViewKey::new(frontend_id, window_id, buffer_id);
        let mut manager = editor.terminal_manager.borrow_mut();
        manager.register_view(key);
        manager.claim_controller(key);
        (buffer_id, window_id)
    }

    fn screen_generation(editor: &EditorState, buffer_id: crate::buffer::BufferId) -> u64 {
        editor
            .terminal_manager
            .borrow()
            .snapshot(buffer_id)
            .expect("terminal snapshot")
            .screen_generation
    }

    /// Acceptance 2 and 3: one declaration produces exactly one resize, and
    /// the screen then STAYS at the declared content rectangle.
    ///
    /// Against the pre-split tree both arms ran for the semantic frontend and
    /// generation advanced by two per iteration forever, because each arm's
    /// idempotence guard only ever saw the size the other had just written.
    #[test]
    fn semantic_terminal_geometry_settles_after_one_declaration() {
        let fid = FrontendId(41);
        let mut editor = EditorState::new();
        let (buffer_id, _window) = quiet_terminal_for(&editor, fid);

        // The GPU declares a CONTENT rectangle; the grid size it also
        // reported at attach is deliberately DIFFERENT, which is the
        // collision the defect fed on.
        let declared = CellSize::new(25, 92);
        let mut semantic = crate::semantic_render::SemanticRenderState::for_peer(fid, 20);
        semantic.set_terminal_viewport(buffer_id, declared);
        let semantic_states = HashMap::from([(fid, semantic)]);
        let term_sizes = HashMap::from([(fid, CellSize::new(24, 80))]);
        let attached = vec![fid];

        sync_terminal_layouts_for_tick(&mut editor, &attached, &term_sizes, &semantic_states);
        let after_first = screen_generation(&editor, buffer_id);
        assert_eq!(
            editor.terminal_manager.borrow().screen_size(buffer_id),
            Some(declared),
            "the declared content rectangle must win"
        );

        // Acceptance 2: every further tick is a no-op.
        for _ in 0..8 {
            sync_terminal_layouts_for_tick(&mut editor, &attached, &term_sizes, &semantic_states);
        }
        assert_eq!(
            screen_generation(&editor, buffer_id),
            after_first,
            "an unchanged declaration must not mutate the screen again \
             (pre-split: +2 per tick, forever)"
        );
        // Acceptance 3: the state predicate, not "a frame at this width
        // arrived at some point".
        assert_eq!(
            editor.terminal_manager.borrow().screen_size(buffer_id),
            Some(declared),
            "the geometry must SETTLE at the declared rectangle"
        );

        editor.process_supervisor.borrow_mut().shutdown();
    }

    /// Acceptance 6: a semantic frontend whose window switches away releases
    /// its terminal controller.
    ///
    /// This bites against BOTH the pre-split tree's sibling arms and against
    /// the naive "skip the grid arm for semantic frontends" guard, which is
    /// why B1 is recorded as half-false. The release cannot live in
    /// `sync_semantic_terminal_layout`: the buffer-follow snapshot clears the
    /// viewport declaration, so that arm stops running in exactly this case —
    /// modelled here by dropping the declaration alongside the switch.
    #[test]
    fn semantic_frontend_releases_its_terminal_controller_when_its_window_switches_away() {
        let fid = FrontendId(42);
        let mut editor = EditorState::new();
        let (buffer_id, window_id) = quiet_terminal_for(&editor, fid);

        let declared = CellSize::new(25, 92);
        let mut semantic = crate::semantic_render::SemanticRenderState::for_peer(fid, 20);
        semantic.set_terminal_viewport(buffer_id, declared);
        let mut semantic_states = HashMap::from([(fid, semantic)]);
        let term_sizes = HashMap::from([(fid, CellSize::new(24, 80))]);
        let attached = vec![fid];

        sync_terminal_layouts_for_tick(&mut editor, &attached, &term_sizes, &semantic_states);
        assert_eq!(
            editor
                .terminal_manager
                .borrow()
                .controller_view_for_frontend(fid),
            Some(crate::terminal::TerminalViewKey::new(
                fid, window_id, buffer_id
            )),
            "precondition: the frontend holds the controller"
        );

        // The window switches to a document, and the snapshot that announces
        // it clears the semantic declaration — `on_buffer_snapshot_sent`.
        let document = editor.core.borrow().registry.borrow_mut().create("doc");
        {
            let mut core = editor.core.borrow_mut();
            let text_view = {
                let registry = core.registry.clone();
                let registry = registry.borrow();
                let buffer = registry.get(document).expect("document buffer");
                crate::text_view::TextView::new(buffer)
            };
            let window = core.windows.get_mut(&window_id).expect("window");
            *window = crate::window::Window::new(window_id, document, text_view);
        }
        semantic_states
            .get_mut(&fid)
            .expect("semantic state")
            .on_buffer_snapshot_sent(document);

        sync_terminal_layouts_for_tick(&mut editor, &attached, &term_sizes, &semantic_states);
        assert_eq!(
            editor
                .terminal_manager
                .borrow()
                .controller_view_for_frontend(fid),
            None,
            "a semantic frontend that left its terminal must release the \
             controller, or no peer can resize that PTY again"
        );

        editor.process_supervisor.borrow_mut().shutdown();
    }

    /// Acceptance 5 at the unit seam: a GRID frontend still gets its
    /// placement-derived resize. The split must not turn the storm fix into
    /// "semantic frontends win everywhere".
    #[test]
    fn grid_terminal_geometry_still_syncs_for_a_grid_frontend() {
        let fid = FrontendId(43);
        let mut editor = EditorState::new();
        let (buffer_id, _window) = quiet_terminal_for(&editor, fid);

        let semantic_states = HashMap::new();
        let term_sizes = HashMap::from([(fid, CellSize::new(40, 100))]);
        let attached = vec![fid];

        let before = editor.terminal_manager.borrow().screen_size(buffer_id);
        sync_terminal_layouts_for_tick(&mut editor, &attached, &term_sizes, &semantic_states);
        let after = editor.terminal_manager.borrow().screen_size(buffer_id);

        assert_ne!(before, after, "a grid frontend must still resize its PTY");
        assert_eq!(
            after.map(|size| size.cols),
            Some(100),
            "the grid arm supplies the full declared width"
        );
        // And it too settles.
        let settled = screen_generation(&editor, buffer_id);
        for _ in 0..4 {
            sync_terminal_layouts_for_tick(&mut editor, &attached, &term_sizes, &semantic_states);
        }
        assert_eq!(
            screen_generation(&editor, buffer_id),
            settled,
            "an unchanged grid size must not mutate the screen again"
        );

        editor.process_supervisor.borrow_mut().shutdown();
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

    /// Arc 8 Stage 4b acceptance 45f: the Lean abbreviation expander
    /// works on the OPTIMISTIC producer, not only on `dispatch_key`.
    ///
    /// This is the path most users take and the one no other Stage 4b
    /// test covers. `classify_key` (`src/optimistic.rs`) returns
    /// `Insert(c)` for `\` and for every ASCII letter — only the nine
    /// built-in pair chars are excluded (Q#AP1) — so on a CRDT frontend
    /// `\alpha` arrives here as six source-peer optimistic inserts,
    /// while the expansion is a single daemon-peer replace spanning all
    /// six. That asymmetry is the accepted undo degradation of Q#LN21;
    /// what this pins is that the expansion happens at all.
    ///
    /// It lives in `--lib` deliberately: the gate list runs
    /// `--features crdt` only for `cargo test --lib`, so a crdt-gated
    /// INTEGRATION test would be dark in CI and dark in the gates both.
    ///
    /// The source frontend needs a REGISTERED WINDOW on the edited
    /// buffer or nothing is armed at all — `handle_remote_crdt_op`
    /// arms the record only when the source's active window displays
    /// the buffer, so a source with no view fails closed and silently.
    /// A version of this test without the view below passed six
    /// fan-outs with a nil record and proved nothing.
    #[cfg(feature = "crdt")]
    #[test]
    fn the_optimistic_producer_also_expands_a_lean_abbreviation() {
        use crate::editor::EditorState;
        use crate::protocol::FrontendId;
        use crate::window::{FrontendView, Layout, Window, WindowId};

        let dir = std::env::temp_dir().join(format!("pmacs-lean-opt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("a.lean");
        std::fs::write(&path, "").expect("write fixture");

        let source = FrontendId(77);
        let mut editor = EditorState::new();
        editor
            .lua_host
            .eval(Some("test"), "pmacs.lsp.config = {}")
            .expect("clear lsp config");
        editor
            .lua_host
            .eval(
                Some("test-open"),
                &format!(
                    "pmacs.buffer.find_or_open({:?}); pmacs.editor.goto_byte(0)",
                    path.display().to_string()
                ),
            )
            .expect("open the lean fixture");

        let buffer_id = editor.core.borrow().active_window().buffer_id;
        {
            let mut core = editor.core.borrow_mut();
            let mut reg = core.registry.borrow_mut();
            reg.get_mut(buffer_id)
                .expect("active buffer")
                .upgrade_to_crdt(2)
                .expect("upgrade to crdt");
            drop(reg);

            // The replica's own window on the shared buffer.
            let text_view = {
                let registry = core.registry.clone();
                let reg = registry.borrow();
                crate::text_view::TextView::new(reg.get(buffer_id).expect("buffer"))
            };
            let win_id = WindowId::next();
            core.windows
                .insert(win_id, Window::new(win_id, buffer_id, text_view));
            core.register_frontend_view(
                source,
                FrontendView {
                    layout: Layout::single(win_id),
                    active: win_id,
                    fold_projection: true,
                    panel_capable: true,
                    frame_geometry: None,
                    panel_hidden: false,
                },
            );
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
        peer.set_peer_id(77).expect("set peer id");
        peer.import(&snapshot_bytes).expect("import snapshot");

        // One op per keystroke, exactly as the attach loop's
        // optimistic-apply branch produces them.
        for (i, ch) in "\\alpha".chars().enumerate() {
            let v_before = peer.oplog_vv();
            peer.get_text("body")
                .insert(i, &ch.to_string())
                .expect("peer insert");
            let op_bytes = peer
                .export(loro::ExportMode::updates(&v_before))
                .expect("export op");
            super::handle_remote_crdt_op(
                &mut editor,
                source,
                buffer_id,
                crate::rope::CrdtOp {
                    peer_id: 77,
                    bytes: op_bytes,
                },
            );
        }

        let text = match editor
            .lua_host
            .eval(
                Some("test-readback"),
                "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
            )
            .expect("read buffer text")
        {
            mlua::Value::String(s) => String::from_utf8_lossy(&s.as_bytes()).into_owned(),
            other => panic!("expected buffer text, got {other:?}"),
        };
        assert_eq!(
            text, "α",
            "the abbreviation expanded on the optimistic path — the \
             record the expander reads is armed by handle_remote_crdt_op, \
             not only by dispatch_key"
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
            peer.set_peer_id(FrontendId::LOCAL.0).expect("set peer id");
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
    /// `dispatch_key`, so it must update the source frontend's command
    /// boundary or `C-k x C-k` on the GPU would append across the typed
    /// character. A single-codepoint insert classifies as
    /// `buffer.self-insert` (the input-origin signal for signature
    /// help); anything else breaks the chain outright.
    #[cfg(feature = "crdt")]
    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one end-to-end classification scenario per kill-chain case"
    )]
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
    /// the one it declares via `Viewport`. `align_primary_document_window`
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
        // Bottom-panel §1.3 #7: `Viewport` takes the projection-only
        // aligner, which never touches `view.active`.
        align_primary_document_window(&mut editor, fid, file);
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

    /// Bottom-panel §1.3 #8, review round 1 finding 1: a STALE document
    /// `Pointer` must not steal focus out of a panel.
    ///
    /// Driven through `handle_dispatcher_event` — the real dispatcher
    /// seam — because the defect lived in the *pair* of alignment and
    /// activation, not in either alone. `align_primary_document_window`
    /// once returned the window even when the named buffer was gone, so
    /// #8's activation focused the document before `dispatch_pointer`
    /// ever rejected the mismatched buffer.
    #[cfg(feature = "crdt")]
    #[test]
    fn a_stale_document_pointer_does_not_steal_focus_from_a_panel() {
        use crate::editor::EditorState;
        use crate::protocol::FrontendId;
        use crate::window::{FrontendView, Layout, LayoutNode, Orientation, Window, WindowParams};
        use pmacs_protocol::{Modifiers, PointerKind};

        let mut editor = EditorState::new();
        let fid = FrontendId(88);

        // One document window + one focused bottom panel.
        let (document, panel, dead_buffer) = {
            let mut core = editor.core.borrow_mut();
            let doc_buf = core.active_window().buffer_id;
            let panel_buf = core.registry.borrow_mut().create("*panel*");
            // A buffer id that names nothing: the stale-pointer payload.
            let dead_buffer = crate::buffer::BufferId::from_raw(999_999);

            let document = crate::window::WindowId::next();
            let panel_id = crate::window::WindowId::next();
            let (doc_view, panel_view) = {
                let reg = core.registry.borrow();
                (
                    crate::text_view::TextView::new(reg.get(doc_buf).expect("doc")),
                    crate::text_view::TextView::new(reg.get(panel_buf).expect("panel")),
                )
            };
            core.windows
                .insert(document, Window::new(document, doc_buf, doc_view));
            let mut panel = Window::new(panel_id, panel_buf, panel_view);
            let mut params = WindowParams::default();
            params.side = Some(crate::window::Side::Bottom);
            params.fixed_rows = Some(4);
            panel.params = params;
            core.windows.insert(panel_id, panel);
            core.register_frontend_view(
                fid,
                FrontendView {
                    layout: Layout {
                        root: LayoutNode::Split {
                            orientation: Orientation::Horizontal,
                            children: vec![LayoutNode::Leaf(document), LayoutNode::Leaf(panel_id)],
                            weights: vec![1, 1],
                        },
                    },
                    active: panel_id,
                    fold_projection: false,
                    panel_capable: true,
                    frame_geometry: None,
                    panel_hidden: false,
                },
            );
            (document, panel_id, dead_buffer)
        };

        let mut render_states = HashMap::new();
        let mut semantic_states = HashMap::new();
        semantic_states.insert(fid, crate::semantic_render::SemanticRenderState::new(fid));
        let mut streams = HashMap::new();
        let mut term_sizes = HashMap::new();
        term_sizes.insert(fid, CellSize::new(24, 80));
        let mut last_idle = HashMap::new();
        let mut last_active = HashMap::new();
        let mut bells = HashMap::new();
        // The dispatcher drops any event from an UNINSTALLED session
        // (#148's defense-in-depth membership check), so the session must
        // be registered or this test passes for the wrong reason — it did,
        // on the first attempt.
        let mut registry = SessionRegistry::new();
        registry.register_session(
            fid,
            crate::presence::SessionState {
                negotiated_protocol_version: pmacs_protocol::PROTOCOL_VERSION,
                negotiated_capabilities: crate::protocol::NegotiatedCapabilities {
                    semantic_render: true,
                    crdt_replica: true,
                    ..Default::default()
                },
                color_slot: 0,
            },
        );

        handle_dispatcher_event(
            DispatcherEvent::FrontendEvent {
                source: fid,
                event: FrontendEvent::Pointer {
                    frontend_id: fid,
                    buffer_id: dead_buffer,
                    byte: 0,
                    kind: PointerKind::Down,
                    mods: Modifiers::default(),
                },
            },
            &mut editor,
            &mut render_states,
            &mut semantic_states,
            &mut streams,
            &mut term_sizes,
            &mut last_idle,
            &mut last_active,
            &mut bells,
            &mut registry,
        );

        assert_eq!(
            editor.core.borrow().views[&fid].active,
            panel,
            "a stale Pointer naming a dead buffer must NOT move focus out of the panel"
        );
        assert_ne!(
            editor.core.borrow().views[&fid].active,
            document,
            "non-vacuity: the document window is a real, distinct focus target"
        );
    }

    /// **N2** (Journey Stage 1a) — a DIRECTORY initial target reaches
    /// readiness instead of failing.
    ///
    /// This deliberately supersedes the directory half of the GPU
    /// initial-target framing's Q#GT6 and its acceptance 10, which
    /// required `IsADirectory` to fail before window creation.
    /// Permission-denied and every other pre-readiness failure keep that
    /// contract.
    #[test]
    fn initial_target_directory_reaches_ready() {
        use crate::editor::EditorState;
        use crate::protocol::FrontendId;

        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("alpha.txt"), b"alpha\n").expect("write");

        let mut editor = EditorState::new();
        editor
            .lua_host
            .lua()
            .load("pmacs.lsp.config = {}")
            .exec()
            .expect("wipe lsp config");
        let fid = FrontendId(131);
        let view = build_fresh_frontend_view(&mut editor, false, false);
        editor.core.borrow_mut().register_frontend_view(fid, view);

        let opened = open_initial_target(
            &mut editor,
            fid,
            InitialTarget {
                path: dir.path().as_os_str().as_bytes().to_vec(),
                cwd: dir.path().as_os_str().as_bytes().to_vec(),
            },
        )
        .expect("a directory target must reach readiness, not fail");

        // The reply names a live buffer in a live document window: a
        // valid, ready session. The listing arrives later, asynchronously.
        let core = editor.core.borrow();
        assert!(
            core.registry.borrow().contains(opened.buffer_id),
            "the reported buffer must exist so its snapshot can be sent"
        );
        let active = core.views[&fid].active;
        assert_eq!(
            core.windows[&active].buffer_id, opened.buffer_id,
            "the reported buffer is the one the document window shows"
        );
    }

    /// **N5** — the bootstrap buffer is not necessarily `*scratch*`.
    ///
    /// `build_fresh_frontend_view` clones LOCAL's PRIMARY DOCUMENT
    /// buffer, so when LOCAL holds a real document the fresh session
    /// briefly displays and snapshots it. Q#JR9 accepts that rather than
    /// introducing a placeholder; this observes it instead of assuming.
    #[test]
    fn initial_target_directory_reports_a_non_scratch_primary() {
        use crate::editor::EditorState;
        use crate::protocol::FrontendId;

        let dir = tempfile::tempdir().expect("tempdir");
        let doc = dir.path().join("already-open.txt");
        std::fs::write(&doc, b"local document\n").expect("write");

        // LOCAL holds a real document, not scratch.
        let mut editor = EditorState::open(doc.clone()).expect("open");
        editor
            .lua_host
            .lua()
            .load("pmacs.lsp.config = {}")
            .exec()
            .expect("wipe lsp config");
        let local_primary = editor
            .core
            .borrow()
            .primary_document_buffer(FrontendId::LOCAL)
            .expect("LOCAL always has a document window");

        let fid = FrontendId(132);
        let view = build_fresh_frontend_view(&mut editor, false, false);
        editor.core.borrow_mut().register_frontend_view(fid, view);

        let opened = open_initial_target(
            &mut editor,
            fid,
            InitialTarget {
                path: dir.path().as_os_str().as_bytes().to_vec(),
                cwd: dir.path().as_os_str().as_bytes().to_vec(),
            },
        )
        .expect("a directory target must reach readiness");

        assert_eq!(
            opened.buffer_id, local_primary,
            "the bootstrap reply names LOCAL's primary document buffer, \
             which is a real document here rather than *scratch*"
        );
    }

    /// **N2b (rev 6)** — a resolver that claims SYNCHRONOUSLY is reported
    /// correctly.
    ///
    /// The bug this pins: the arm captured the destination buffer id
    /// *before* dispatching the chain and reported that. The chain runs
    /// synchronously, so a handler that opens something immediately —
    /// through `commit_to`, the supported way — had already replaced the
    /// window's buffer, and the reply paired one buffer's snapshot with
    /// another's identity.
    ///
    /// Falsified by reporting the captured id instead of re-reading.
    #[test]
    fn initial_target_directory_reports_what_a_synchronous_handler_installed() {
        use crate::editor::EditorState;
        use crate::protocol::FrontendId;

        let dir = tempfile::tempdir().expect("tempdir");

        let mut editor = EditorState::new();
        editor
            .lua_host
            .lua()
            .load(
                "pmacs.lsp.config = {}
                 claimed = pmacs.buffer.create('*claimed*')
                 pmacs.path.set_directory_handler(function(path, dest)
                   pmacs.window.commit_to(dest, function()
                     pmacs.window.display(claimed, { select = true })
                   end)
                 end)",
            )
            .exec()
            .expect("install a synchronous handler");

        let fid = FrontendId(133);
        let view = build_fresh_frontend_view(&mut editor, false, false);
        editor.core.borrow_mut().register_frontend_view(fid, view);

        let opened = open_initial_target(
            &mut editor,
            fid,
            InitialTarget {
                path: dir.path().as_os_str().as_bytes().to_vec(),
                cwd: dir.path().as_os_str().as_bytes().to_vec(),
            },
        )
        .expect("a claimed directory target must reach readiness");

        // Compare by NAME: the reported id must be the handler's buffer,
        // and naming it is what makes the failure legible when it is not.
        let core = editor.core.borrow();
        let reported_name = core
            .registry
            .borrow()
            .get(opened.buffer_id)
            .expect("the reported buffer exists")
            .name()
            .to_string();
        assert_eq!(
            reported_name, "*claimed*",
            "the reply must name what the handler installed, not the \
             buffer captured before the dispatch"
        );
        let active = core.views[&fid].active;
        assert_eq!(
            core.windows[&active].buffer_id, opened.buffer_id,
            "…and that buffer is what the window shows"
        );
    }

    /// Bottom-panel §1.3 #1/#3/#21 — the three Projection producers whose
    /// only production caller is `dispatcher_loop`, pinned at the named
    /// seams that loop calls. Round 2 finding: reverting any of them to
    /// `active_window_for` previously left every test green.
    #[cfg(feature = "crdt")]
    #[test]
    fn tick_producers_describe_the_document_while_a_panel_is_focused() {
        let (editor, fid, document, panel) = panel_focused_semantic_fixture();
        let (doc_buf, panel_buf, doc_cursor) = {
            let core = editor.core.borrow();
            (
                core.windows[&document].buffer_id,
                core.windows[&panel].buffer_id,
                core.windows[&document].cursor,
            )
        };
        assert_ne!(doc_buf, panel_buf, "fixture: distinct buffers");

        // #1 buffer-follow / BufferSnapshot re-send target.
        assert_eq!(
            document_buffer_to_follow(&editor, fid),
            Some(doc_buf),
            "#1: the follow target must be the DOCUMENT buffer, not the focused panel's"
        );

        // #3 CursorByte.
        assert_eq!(
            document_cursor_byte(&editor, fid),
            Some((doc_buf, doc_cursor)),
            "#3: CursorByte must describe the DOCUMENT surface"
        );

        // #21 is deliberately NOT asserted here. Round 3: pinning it at
        // this helper left the real producer free to regress — reverting
        // the call site inside `publish_buffer_snapshot_to_replicas`
        // kept both this test and the existing socket-pair test green.
        // It is pinned through the producer instead, in
        // `snapshot_publication_follows_the_document_under_a_focused_panel`.
        let _ = panel_buf;
    }

    /// Bottom-panel §1.3 #2 — the sharpest census case: the lazy CRDT
    /// upgrade BROADCASTS a snapshot, so keying it on focus would let
    /// focusing a fresh generated panel buffer swap every peer's mirror.
    #[cfg(feature = "crdt")]
    #[test]
    fn lazy_crdt_upgrade_never_targets_a_focused_panel_buffer() {
        let (editor, fid, document, panel) = panel_focused_semantic_fixture();
        let (doc_buf, panel_buf) = {
            let core = editor.core.borrow();
            (
                core.windows[&document].buffer_id,
                core.windows[&panel].buffer_id,
            )
        };

        let upgraded = ensure_active_buffer_crdt_backed(&editor, fid);
        assert_eq!(
            upgraded,
            Some(doc_buf),
            "#2: the upgrade must target the DOCUMENT buffer"
        );
        assert_ne!(
            upgraded,
            Some(panel_buf),
            "#2: focusing a panel must never trigger its buffer's upgrade+broadcast"
        );
    }

    /// Bottom-panel §1.3 #7 vs #8 — `Viewport` aligns WITHOUT moving
    /// focus; only `Pointer` activates. Driven through the real
    /// dispatcher seam.
    #[cfg(feature = "crdt")]
    #[test]
    fn viewport_aligns_the_document_without_taking_focus_from_the_panel() {
        let (mut editor, fid, document, panel) = panel_focused_semantic_fixture();
        let other = {
            let core = editor.core.borrow_mut();
            core.registry.borrow_mut().create("*other*")
        };

        dispatch_one_semantic_event(
            &mut editor,
            fid,
            FrontendEvent::Viewport {
                frontend_id: fid,
                buffer_id: other,
                visible: pmacs_protocol::ByteRange { start: 0, end: 0 },
                generation: 0,
            },
        );

        assert_eq!(
            editor.core.borrow().views[&fid].active,
            panel,
            "#7: a document Viewport must NOT move focus out of the panel"
        );
        assert_eq!(
            editor.core.borrow().windows[&document].buffer_id,
            other,
            "#7: it must still have ALIGNED the document window to the declared buffer"
        );
    }

    /// Shared fixture: a semantic frontend with a document window and a
    /// FOCUSED bottom panel. `panel_capable` is set explicitly because
    /// Stage 1 ships `false` for semantic sessions and 2B flips it for a
    /// v21-negotiated peer.
    #[cfg(feature = "crdt")]
    fn panel_focused_semantic_fixture() -> (
        crate::editor::EditorState,
        FrontendId,
        crate::window::WindowId,
        crate::window::WindowId,
    ) {
        use crate::window::{FrontendView, Layout, LayoutNode, Orientation, Window, WindowParams};

        let editor = crate::editor::EditorState::new();
        let fid = FrontendId(91);
        let (document, panel) = {
            let mut core = editor.core.borrow_mut();
            let doc_buf = core.active_window().buffer_id;
            let panel_buf = core.registry.borrow_mut().create("*panel*");
            let document = crate::window::WindowId::next();
            let panel = crate::window::WindowId::next();
            let (doc_view, panel_view) = {
                let reg = core.registry.borrow();
                (
                    crate::text_view::TextView::new(reg.get(doc_buf).expect("doc")),
                    crate::text_view::TextView::new(reg.get(panel_buf).expect("panel")),
                )
            };
            core.windows
                .insert(document, Window::new(document, doc_buf, doc_view));
            let mut panel_window = Window::new(panel, panel_buf, panel_view);
            let mut params = WindowParams::default();
            params.side = Some(crate::window::Side::Bottom);
            params.fixed_rows = Some(4);
            panel_window.params = params;
            core.windows.insert(panel, panel_window);
            core.register_frontend_view(
                fid,
                FrontendView {
                    layout: Layout {
                        root: LayoutNode::Split {
                            orientation: Orientation::Horizontal,
                            children: vec![LayoutNode::Leaf(document), LayoutNode::Leaf(panel)],
                            weights: vec![1, 1],
                        },
                    },
                    active: panel,
                    fold_projection: false,
                    panel_capable: true,
                    frame_geometry: None,
                    panel_hidden: false,
                },
            );
            (document, panel)
        };
        editor.sync_frame_geometry(fid, CellSize::new(24, 80));
        (editor, fid, document, panel)
    }

    /// Drive ONE authenticated semantic event through the real
    /// dispatcher. The session must be registered or the event is
    /// dropped at the uninstalled-session check before reaching any
    /// handler.
    #[cfg(feature = "crdt")]
    fn dispatch_one_semantic_event(
        editor: &mut crate::editor::EditorState,
        fid: FrontendId,
        event: FrontendEvent,
    ) {
        let mut render_states = HashMap::new();
        let mut semantic_states = HashMap::new();
        semantic_states.insert(fid, crate::semantic_render::SemanticRenderState::new(fid));
        let mut streams = HashMap::new();
        let mut term_sizes = HashMap::new();
        term_sizes.insert(fid, CellSize::new(24, 80));
        let mut last_idle = HashMap::new();
        let mut last_active = HashMap::new();
        let mut bells = HashMap::new();
        let mut registry = SessionRegistry::new();
        registry.register_session(
            fid,
            crate::presence::SessionState {
                negotiated_protocol_version: pmacs_protocol::PROTOCOL_VERSION,
                negotiated_capabilities: crate::protocol::NegotiatedCapabilities {
                    semantic_render: true,
                    crdt_replica: true,
                    ..Default::default()
                },
                color_slot: 0,
            },
        );
        handle_dispatcher_event(
            DispatcherEvent::FrontendEvent { source: fid, event },
            editor,
            &mut render_states,
            &mut semantic_states,
            &mut streams,
            &mut term_sizes,
            &mut last_idle,
            &mut last_active,
            &mut bells,
            &mut registry,
        );
    }

    /// Bottom-panel §1.3 #9 — Projection. The `Viewport` terminal-context
    /// gate asks "is this frontend's DOCUMENT surface a terminal", so a
    /// focused TERMINAL PANEL must not suppress the still-visible
    /// document's viewport.
    #[cfg(feature = "crdt")]
    #[test]
    fn a_focused_terminal_panel_does_not_suppress_the_document_viewport() {
        use crate::terminal::TerminalSpec;

        let (mut editor, fid, document, panel) = panel_focused_semantic_fixture();
        let other = editor.core.borrow().registry.borrow_mut().create("*other*");

        // A REAL terminal in the focused panel.
        let mut spec = TerminalSpec::new("/bin/sh");
        spec.rows = 10;
        spec.cols = 40;
        let term_buf = editor.open_terminal(spec).expect("a real terminal");
        editor
            .core
            .borrow_mut()
            .install_buffer_in_window(panel, term_buf)
            .expect("terminal into the panel");
        editor.core.borrow_mut().focus_window(fid, panel);

        dispatch_one_semantic_event(
            &mut editor,
            fid,
            FrontendEvent::Viewport {
                frontend_id: fid,
                buffer_id: other,
                visible: pmacs_protocol::ByteRange { start: 0, end: 0 },
                generation: 0,
            },
        );

        assert_eq!(
            editor.core.borrow().windows[&document].buffer_id,
            other,
            "#9: a focused TERMINAL panel must not suppress the document viewport —              the document window should still have aligned to the declared buffer"
        );
    }

    // -----------------------------------------------------------------
    // Bottom-panel Stage 2B-2 — inbound panel-event routing, driven
    // through `handle_dispatcher_event` (the real dispatcher seam).
    //
    // Deliberately NOT `crdt`-gated: CI never enables that feature, so a
    // gated pin is dark exactly where it needs to run.
    // -----------------------------------------------------------------

    /// A semantic, panel-capable frontend. `with_panel` decides whether
    /// it also owns a side window — `FrontendCellGeometry` must be
    /// accepted **without** one (Q#BP15a breaks the first-open cycle),
    /// while the two gesture events must not be.
    fn semantic_panel_view(
        editor: &crate::editor::EditorState,
        fid: FrontendId,
        with_panel: bool,
    ) -> (crate::window::WindowId, Option<crate::window::WindowId>) {
        use crate::window::{FrontendView, Layout, LayoutNode, Orientation, Window, WindowParams};

        let mut core = editor.core.borrow_mut();
        let doc_buf = core.active_window().buffer_id;
        let document = crate::window::WindowId::next();
        let doc_view = {
            let reg = core.registry.borrow();
            crate::text_view::TextView::new(reg.get(doc_buf).expect("doc"))
        };
        core.windows
            .insert(document, Window::new(document, doc_buf, doc_view));
        let panel = with_panel.then(|| {
            let panel_buf = core.registry.borrow_mut().create("*panel*");
            let panel_id = crate::window::WindowId::next();
            let panel_view = {
                let reg = core.registry.borrow();
                crate::text_view::TextView::new(reg.get(panel_buf).expect("panel"))
            };
            let mut window = Window::new(panel_id, panel_buf, panel_view);
            let mut params = WindowParams::default();
            params.side = Some(crate::window::Side::Bottom);
            params.fixed_rows = Some(4);
            window.params = params;
            core.windows.insert(panel_id, window);
            panel_id
        });
        let layout = match panel {
            Some(panel) => Layout {
                root: LayoutNode::Split {
                    orientation: Orientation::Horizontal,
                    children: vec![LayoutNode::Leaf(document), LayoutNode::Leaf(panel)],
                    weights: vec![1, 1],
                },
            },
            None => Layout::single(document),
        };
        core.register_frontend_view(
            fid,
            FrontendView {
                layout,
                active: document,
                fold_projection: false,
                // Stage 2B-2 is dark: production negotiation still sets
                // this `false` for every semantic session, so the
                // projection is exercised through a test-only view (the
                // framing's §7.2.2 posture).
                panel_capable: true,
                frame_geometry: None,
                panel_hidden: false,
            },
        );
        (document, panel)
    }

    fn session(version: u32, semantic: bool) -> crate::presence::SessionState {
        crate::presence::SessionState {
            negotiated_protocol_version: version,
            negotiated_capabilities: crate::protocol::NegotiatedCapabilities {
                semantic_render: semantic,
                crdt_replica: true,
                ..Default::default()
            },
            color_slot: 0,
        }
    }

    /// Drive one authenticated event through the real dispatcher while
    /// keeping the caller's projection state, so a test can ship a
    /// `PanelFrame` first and then send an event addressing it.
    ///
    /// The session is registered because the dispatcher drops any event
    /// from an uninstalled session before it reaches a handler.
    /// §5b G8e — the frontend a forged payload claims to be. Registered
    /// in every dispatch fixture at the MAPPED version, so "the claimed
    /// id speaks the other family" is a real condition rather than an
    /// absent session.
    const FORGERY_TARGET_FID: FrontendId = FrontendId(767);

    /// §5b G8e, the other direction: a frontend that speaks the LEGACY
    /// family, for a mapped session to try to borrow.
    const FORGERY_TARGET_LEGACY_FID: FrontendId = FrontendId(768);

    /// §5b — a session that negotiated the LEGACY panel family.
    ///
    /// These rows drive `FrontendEvent::PanelPointer`, which a `>= v25`
    /// session may not send at all, so they must say which family they
    /// are exercising. Naming it here also makes them explicit legacy
    /// positive controls rather than tests that happened to pass.
    /// Stated as a LITERAL, not `PANEL_MAPPING_MIN_VERSION - 1`:
    /// G6/G14 make the family boundary absolute, and arithmetic against
    /// a moving constant would drag these rows forward on the next bump
    /// — they would silently start testing v25 as "legacy".
    const LEGACY_PANEL_VERSION: u32 = 24;

    fn dispatch_panel_event(
        editor: &mut crate::editor::EditorState,
        fid: FrontendId,
        version: u32,
        semantic_states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
        render_states: &mut HashMap<FrontendId, RenderState>,
        event: FrontendEvent,
    ) {
        let mut streams = HashMap::new();
        let mut term_sizes = HashMap::new();
        term_sizes.insert(fid, CellSize::new(24, 80));
        let mut last_idle = HashMap::new();
        let mut last_active = HashMap::new();
        let mut bells = HashMap::new();
        let mut registry = SessionRegistry::new();
        registry.register_session(fid, session(version, !render_states.contains_key(&fid)));
        // §5b G8e — a SECOND session that speaks the mapped family, so a
        // payload claiming its id has something real to borrow. Without
        // it, a payload-keyed lookup would fail for want of a session
        // rather than for want of authority, and the mutation that
        // swaps `source` for the payload's id would be invisible.
        if fid != FORGERY_TARGET_FID {
            registry.register_session(
                FORGERY_TARGET_FID,
                session(pmacs_protocol::PANEL_MAPPING_MIN_VERSION, true),
            );
        }
        if fid != FORGERY_TARGET_LEGACY_FID {
            registry.register_session(
                FORGERY_TARGET_LEGACY_FID,
                session(LEGACY_PANEL_VERSION, true),
            );
        }
        handle_dispatcher_event(
            DispatcherEvent::FrontendEvent { source: fid, event },
            editor,
            render_states,
            semantic_states,
            &mut streams,
            &mut term_sizes,
            &mut last_idle,
            &mut last_active,
            &mut bells,
            &mut registry,
        );
    }

    fn geometry_event(claimed: FrontendId, epoch: u64, rows: u32, cols: u32) -> FrontendEvent {
        FrontendEvent::FrontendCellGeometry {
            frontend_id: claimed,
            geometry_epoch: epoch,
            total: CellSize::new(rows, cols),
        }
    }

    /// Criterion 50 (accept half) + Q#BP15a: the declaration is valid
    /// with no side window at all. Gating it on panel presence would
    /// deadlock the first open, because the daemon needs columns before
    /// it can paint a first frame.
    #[test]
    fn frontend_cell_geometry_is_accepted_without_a_side_window() {
        let mut editor = crate::editor::EditorState::new();
        let fid = FrontendId(701);
        semantic_panel_view(&editor, fid, false);
        let mut semantic_states = HashMap::new();
        semantic_states.insert(
            fid,
            crate::semantic_render::SemanticRenderState::for_peer(fid, PROTOCOL_VERSION),
        );

        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut semantic_states,
            &mut HashMap::new(),
            geometry_event(fid, 1, 40, 100),
        );

        let stored = editor.core.borrow().frame_geometry_for(fid);
        assert_eq!(
            stored.map(|geometry| (geometry.geometry_epoch, geometry.total)),
            Some((1, CellSize::new(40, 100))),
            "a panel-capable semantic source declares geometry with no side window"
        );
    }

    /// Criterion 50 (reject half): a GRID session has no panel band, so
    /// its declaration is dropped before it can reach a view.
    #[test]
    fn frontend_cell_geometry_from_a_grid_session_is_dropped() {
        let mut editor = crate::editor::EditorState::new();
        let fid = FrontendId(702);
        semantic_panel_view(&editor, fid, false);
        // A grid session: a `RenderState`, no semantic projection.
        let mut render_states = HashMap::new();
        render_states.insert(fid, RenderState::new(CellSize::new(24, 80)));
        // The grid arm would otherwise mint an epoch from its own attach
        // size, so start from a known state and assert the epoch never
        // answers the wire declaration.
        let before = editor.core.borrow().frame_geometry_for(fid);

        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut HashMap::new(),
            &mut render_states,
            geometry_event(fid, 9, 40, 100),
        );

        let after = editor.core.borrow().frame_geometry_for(fid);
        assert_eq!(
            before, after,
            "a grid session's panel declaration is dropped"
        );
        assert_eq!(after, None, "…and nothing was stored at all");
    }

    /// Criterion 50 (reject half): a peer that negotiated v20 never
    /// negotiated these variants, so its declaration is not trusted even
    /// though its view is panel-capable.
    #[test]
    fn frontend_cell_geometry_below_the_panel_version_is_dropped() {
        let mut editor = crate::editor::EditorState::new();
        let fid = FrontendId(703);
        semantic_panel_view(&editor, fid, false);
        let mut semantic_states = HashMap::new();
        semantic_states.insert(
            fid,
            crate::semantic_render::SemanticRenderState::for_peer(fid, PANEL_MIN_VERSION - 1),
        );

        dispatch_panel_event(
            &mut editor,
            fid,
            PANEL_MIN_VERSION - 1,
            &mut semantic_states,
            &mut HashMap::new(),
            geometry_event(fid, 1, 40, 100),
        );

        assert_eq!(
            editor.core.borrow().frame_geometry_for(fid),
            None,
            "a pre-panel semantic peer's declaration is dropped"
        );
    }

    /// Criterion 50 (reject half) + Q#BP13: capability is the gate, not
    /// only the wire version. A semantic view this daemon did not mark
    /// panel-capable declares nothing.
    #[test]
    fn frontend_cell_geometry_from_a_non_panel_capable_view_is_dropped() {
        let mut editor = crate::editor::EditorState::new();
        let fid = FrontendId(704);
        let view = build_fresh_frontend_view(&mut editor, false, false);
        editor.core.borrow_mut().register_frontend_view(fid, view);
        let mut semantic_states = HashMap::new();
        semantic_states.insert(
            fid,
            crate::semantic_render::SemanticRenderState::for_peer(fid, PROTOCOL_VERSION),
        );

        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut semantic_states,
            &mut HashMap::new(),
            geometry_event(fid, 1, 40, 100),
        );

        assert_eq!(
            editor.core.borrow().frame_geometry_for(fid),
            None,
            "a non-panel-capable semantic view declares no panel geometry"
        );
    }

    /// Criterion 50 (forged-source half): routing is by the
    /// authenticated transport source, never the payload's claimed id, so
    /// a forged id reaches no other frontend's view.
    #[test]
    fn a_forged_frontend_id_in_a_geometry_payload_addresses_nothing() {
        let mut editor = crate::editor::EditorState::new();
        let source = FrontendId(705);
        let victim = FrontendId(706);
        semantic_panel_view(&editor, source, false);
        semantic_panel_view(&editor, victim, false);
        let mut semantic_states = HashMap::new();
        semantic_states.insert(
            source,
            crate::semantic_render::SemanticRenderState::for_peer(source, PROTOCOL_VERSION),
        );

        dispatch_panel_event(
            &mut editor,
            source,
            PROTOCOL_VERSION,
            &mut semantic_states,
            &mut HashMap::new(),
            geometry_event(victim, 1, 40, 100),
        );

        let core = editor.core.borrow();
        assert_eq!(
            core.frame_geometry_for(victim),
            None,
            "the claimed id must not be able to declare another frontend's geometry"
        );
        assert!(
            core.frame_geometry_for(source).is_some(),
            "…while the authenticated source's own declaration still lands"
        );
    }

    /// Ship one real `PanelFrame` so the session holds a live `Present`
    /// declaration, and return its two epochs.
    fn shipped_declaration(
        editor: &crate::editor::EditorState,
        fid: FrontendId,
        semantic_states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
    ) -> (u64, u64) {
        let sem = semantic_states.get_mut(&fid).expect("semantic projection");
        let messages = sem.render_frame(editor);
        assert!(
            messages
                .iter()
                .any(|msg| matches!(msg, InstanceMessage::PanelFrame(_))),
            "fixture precondition: the frame must actually ship a panel declaration"
        );
        let frame = sem.panel_declaration().expect("a Present declaration");
        (frame.geometry_epoch, frame.panel_epoch)
    }

    // -----------------------------------------------------------------
    // §5b G6–G8 — the family gate, both directions, all four quadrants.
    //
    // Every row negotiates ONE version for both the session registry and
    // the retained producer: a control that negotiates two proves
    // nothing about either.
    // -----------------------------------------------------------------

    /// A panel session at one negotiated version: the editor, its
    /// producer, its render states, the side window, and the epochs a
    /// gesture must echo.
    type PanelSessionFixture = (
        crate::editor::EditorState,
        HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
        HashMap<FrontendId, RenderState>,
        crate::window::WindowId,
        crate::window::WindowId,
        (u64, u64),
    );

    /// Build a panel session at one negotiated version and ship its
    /// declaration, returning the epochs a gesture must echo.
    fn panel_session_at(version: u32, fid: FrontendId) -> PanelSessionFixture {
        let editor = crate::editor::EditorState::new();
        let (document, panel) = semantic_panel_view(&editor, fid, true);
        let panel = panel.expect("panel window");
        let mut semantic_states = HashMap::new();
        semantic_states.insert(
            fid,
            crate::semantic_render::SemanticRenderState::for_peer(fid, version),
        );
        editor.accept_semantic_frame_geometry(fid, 1, CellSize::new(24, 80));
        let epochs = shipped_declaration(&editor, fid, &mut semantic_states);
        (
            editor,
            semantic_states,
            HashMap::new(),
            document,
            panel,
            epochs,
        )
    }

    /// An edit this frontend did NOT make: the case `view_top` cannot
    /// catch, because nothing about the frontend's own state moves.
    fn foreign_edit(
        editor: &crate::editor::EditorState,
        buffer_id: crate::buffer::BufferId,
        text: &[u8],
    ) {
        let core = editor.core.borrow();
        let registry = core.registry.clone();
        let mut reg = registry.borrow_mut();
        reg.get_mut(buffer_id)
            .expect("the panel's buffer")
            .set_generated_contents(text)
            .expect("a generated-contents write is a plain content change");
    }

    /// The mapping generation this session's producer most recently
    /// stamped, which a mapped gesture must echo.
    fn stamped_generation(
        semantic_states: &HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
        fid: FrontendId,
    ) -> u64 {
        semantic_states
            .get(&fid)
            .expect("semantic projection")
            .panel_mapping_generation_peek()
            .expect("a stamped mapping generation")
    }

    fn legacy_pointer(
        fid: FrontendId,
        epochs: (u64, u64),
        buffer_id: crate::buffer::BufferId,
    ) -> FrontendEvent {
        FrontendEvent::PanelPointer {
            frontend_id: fid,
            geometry_epoch: epochs.0,
            panel_epoch: epochs.1,
            buffer_id,
            coord: pmacs_protocol::CellCoord::new(0, 0),
            kind: pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left),
            mods: pmacs_protocol::Modifiers::default(),
        }
    }

    fn mapped_pointer(
        fid: FrontendId,
        epochs: (u64, u64),
        buffer_id: crate::buffer::BufferId,
        mapping_generation: u64,
    ) -> FrontendEvent {
        FrontendEvent::PanelPointerMapped {
            frontend_id: fid,
            geometry_epoch: epochs.0,
            panel_epoch: epochs.1,
            buffer_id,
            coord: pmacs_protocol::CellCoord::new(0, 0),
            kind: pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left),
            mods: pmacs_protocol::Modifiers::default(),
            mapping_generation,
        }
    }

    /// §5b G6a — **legacy OUTBOUND**: a v24 peer receives exactly
    /// `Present`, never the mapped family.
    #[test]
    fn g6a_a_legacy_peer_receives_the_legacy_present_family() {
        let fid = FrontendId(760);
        let (_editor, semantic_states, _render, _document, _panel, _epochs) =
            panel_session_at(LEGACY_PANEL_VERSION, fid);
        let declaration = semantic_states
            .get(&fid)
            .expect("projection")
            .last_panel_payload_for_test();
        assert!(
            matches!(
                declaration,
                Some(pmacs_protocol::panel::PanelFramePayload::Present(_))
            ),
            "a v24 session must receive the legacy family and only it; \
             got {declaration:?}"
        );
    }

    /// §5b G7a — **mapped OUTBOUND**: a v25 peer receives
    /// `PresentMapped` carrying a live, nonzero generation.
    #[test]
    fn g7a_a_mapped_peer_receives_present_mapped_with_a_live_generation() {
        let fid = FrontendId(761);
        let (_editor, semantic_states, _render, _document, _panel, _epochs) =
            panel_session_at(PROTOCOL_VERSION, fid);
        let declaration = semantic_states
            .get(&fid)
            .expect("projection")
            .last_panel_payload_for_test();
        match declaration {
            Some(pmacs_protocol::panel::PanelFramePayload::PresentMapped {
                mapping_generation,
                ..
            }) => assert!(
                mapping_generation >= 1,
                "zero is the wire's uninitialised value and is refused on \
                 sight, so a stamped frame must never carry it"
            ),
            other => panic!("a v25 session must receive the mapped family; got {other:?}"),
        }
    }

    /// §5b G6b — **legacy INBOUND routing**: a current legacy gesture
    /// reaches the dispatcher and performs the landed focus activation.
    #[test]
    fn g6b_a_legacy_gesture_routes_and_activates() {
        let fid = FrontendId(762);
        let (mut editor, mut semantic_states, mut render, document, panel, epochs) =
            panel_session_at(LEGACY_PANEL_VERSION, fid);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
        // THIS view's active window, asserted directly. The ambient
        // `active_window_id()` tracks `active_frontend` too, so a row
        // reading it can be satisfied by a frontend switch that never
        // routed anything to the panel.
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            document,
            "precondition: this frontend's active window is the document"
        );

        dispatch_panel_event(
            &mut editor,
            fid,
            LEGACY_PANEL_VERSION,
            &mut semantic_states,
            &mut render,
            legacy_pointer(fid, epochs, buffer_id),
        );
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            panel,
            "a legacy gesture from a legacy session must still route and \
             activate — the gate must not break the family it kept"
        );
    }

    /// §5b G7b — **mapped INBOUND routing**: a current mapped gesture
    /// reaches the dispatcher and activates.
    #[test]
    fn g7b_a_mapped_gesture_routes_and_activates() {
        let fid = FrontendId(763);
        let (mut editor, mut semantic_states, mut render, document, panel, epochs) =
            panel_session_at(PROTOCOL_VERSION, fid);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
        let generation = stamped_generation(&semantic_states, fid);
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            document,
            "precondition: this frontend's active window is the document"
        );

        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut semantic_states,
            &mut render,
            mapped_pointer(fid, epochs, buffer_id, generation),
        );
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            panel,
            "a mapped gesture echoing the current generation must route"
        );
    }

    /// §5b G8a — a **bare `PanelPointer` from a v25 session is
    /// REFUSED**, not handled under legacy semantics.
    #[test]
    fn g8a_a_legacy_gesture_from_a_mapped_session_is_refused() {
        let fid = FrontendId(764);
        let (mut editor, mut semantic_states, mut render, document, panel, epochs) =
            panel_session_at(PROTOCOL_VERSION, fid);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            document,
            "precondition: this frontend's active window is the document"
        );

        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut semantic_states,
            &mut render,
            legacy_pointer(fid, epochs, buffer_id),
        );
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            document,
            "handling it under legacy semantics would leave the mapping \
             hole reachable by choosing a discriminant"
        );
    }

    /// §5b G8c — a **`PanelPointerMapped` from a v24 session is
    /// REFUSED**, even though a peer built from this crate can encode
    /// the discriminant. Negotiation is a gate, not a convention.
    #[test]
    fn g8c_a_mapped_gesture_from_a_legacy_session_is_refused() {
        let fid = FrontendId(765);
        let (mut editor, mut semantic_states, mut render, document, panel, epochs) =
            panel_session_at(LEGACY_PANEL_VERSION, fid);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            document,
            "precondition: this frontend's active window is the document"
        );

        dispatch_panel_event(
            &mut editor,
            fid,
            LEGACY_PANEL_VERSION,
            &mut semantic_states,
            &mut render,
            // Any generation at all: the family gate refuses before the
            // generation is even looked at.
            mapped_pointer(fid, epochs, buffer_id, 1),
        );
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            document,
            "inbound negotiation is a gate, not a sender convention"
        );
    }

    /// §5b G8e — **authenticated-session authority**: claiming another
    /// frontend whose session negotiated the desired family still
    /// refuses.
    ///
    /// The mutation this row exists for is one line: look the
    /// negotiation up by the payload's `frontend_id` instead of the
    /// authenticated `source`, and the forged cross-session claim
    /// succeeds.
    #[test]
    fn g8e_a_forged_frontend_id_cannot_borrow_another_sessions_family() {
        let legacy_fid = FrontendId(766);
        let mapped_fid = FORGERY_TARGET_FID;
        let (mut editor, mut semantic_states, mut render, document, panel, epochs) =
            panel_session_at(LEGACY_PANEL_VERSION, legacy_fid);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
        assert_eq!(
            editor.core.borrow().views[&legacy_fid].active,
            document,
            "precondition: this frontend's active window is the document"
        );

        // The AUTHENTICATED source is the legacy session; the payload
        // claims a frontend that speaks the mapped family.
        dispatch_panel_event(
            &mut editor,
            legacy_fid,
            LEGACY_PANEL_VERSION,
            &mut semantic_states,
            &mut render,
            mapped_pointer(mapped_fid, epochs, buffer_id, 1),
        );
        assert_eq!(
            editor.core.borrow().views[&legacy_fid].active,
            document,
            "the family comes from the AUTHENTICATED session; a payload \
             id is untrusted and must not borrow another's negotiation"
        );
    }

    /// §5b G8e, the OTHER direction — a mapped session cannot borrow a
    /// legacy identity to smuggle the bare variant through.
    ///
    /// Both directions, because an authority check that only holds one
    /// way is one a peer walks around by choosing which identity to
    /// forge. The claimed frontend has a REAL legacy session, so a
    /// payload-keyed lookup succeeds far enough to expose the defect
    /// rather than failing for want of a session.
    #[test]
    fn g8e_a_mapped_session_cannot_forge_a_legacy_identity() {
        let fid = FrontendId(769);
        let (mut editor, mut semantic_states, mut render, document, panel, epochs) =
            panel_session_at(PROTOCOL_VERSION, fid);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            document,
            "precondition: this frontend's active window is the document"
        );

        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut semantic_states,
            &mut render,
            // Authenticated as the MAPPED session; the payload claims a
            // frontend whose session speaks legacy.
            legacy_pointer(FORGERY_TARGET_LEGACY_FID, epochs, buffer_id),
        );
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            document,
            "a mapped session may not send the bare variant, and cannot \
             acquire permission by naming someone who may"
        );
    }

    /// §5b G10b — **zero is refused BEFORE the wheel exemption**.
    ///
    /// The ordering is the row. Zero is what a sender that never
    /// initialised the field produces; the exemption is a liveness
    /// carve-out for coordinate-free wheels. Run the carve-out first and
    /// a zeroed wheel faces no check at all — an inbound opt-out through
    /// the exempt path.
    ///
    /// The predicate is called directly because a wheel has **no
    /// dispatcher-visible effect on this base**: a document panel
    /// focuses on `Down` only, and no panel pointer coordinate is
    /// consumed anywhere, so an accepted wheel and a refused one are
    /// indistinguishable downstream. Asserting focus for a wheel would
    /// be a witness that proves nothing. The end-to-end leg below uses a
    /// press, which does have an effect, so the predicate is shown to be
    /// wired into the production arm rather than merely correct in
    /// isolation.
    #[test]
    fn g10b_generation_zero_is_refused_before_the_wheel_exemption_applies() {
        use pmacs_protocol::{MouseButton, MouseKind};
        let fid = FrontendId(774);
        let (mut editor, mut semantic_states, mut render, _document, panel, epochs) =
            panel_session_at(PROTOCOL_VERSION, fid);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
        let generation = stamped_generation(&semantic_states, fid);

        // The exempt kind with an INVALID generation: refused.
        assert!(
            !panel_mapping_is_current(&editor, &mut semantic_states, fid, MouseKind::ScrollDown, 0),
            "a zeroed wheel is refused: the exemption must not run first"
        );
        // A non-exempt kind with the same zero: also refused, so the
        // above is the zero and not the kind.
        assert!(!panel_mapping_is_current(
            &editor,
            &mut semantic_states,
            fid,
            MouseKind::Down(MouseButton::Left),
            0
        ));
        // POSITIVE CONTROL — the same wheel with a real generation
        // passes, so the refusal is not a dead predicate.
        assert!(
            panel_mapping_is_current(
                &editor,
                &mut semantic_states,
                fid,
                MouseKind::ScrollDown,
                generation
            ),
            "a wheel with a real generation passes"
        );

        // And the predicate is WIRED: a press is the kind whose
        // acceptance is visible, so it carries the end-to-end leg.
        let baseline = editor.core.borrow().views[&fid].active;
        assert_ne!(
            baseline, panel,
            "fixture: the panel must NOT already be focused, or the \
             refusal below is indistinguishable from acceptance"
        );
        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut semantic_states,
            &mut render,
            mapped_pointer(fid, epochs, buffer_id, 0),
        );
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            baseline,
            "a zeroed press never reaches the focus path"
        );
        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut semantic_states,
            &mut render,
            mapped_pointer(fid, epochs, buffer_id, generation),
        );
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            panel,
            "and a real one does"
        );
    }

    /// §5b G10b — a coordinate-free wheel is EXEMPT from the freshness
    /// comparison, so a second queued tick is not refused.
    ///
    /// Only the GATE is proven here. The framing's G12 — that both ticks
    /// apply their scrolls — is `panel-pointer-replay`'s, because no
    /// panel wheel moves a view on this base. What this pins is that the
    /// second tick is not refused, which is the half that would
    /// otherwise make G12 unreachable.
    ///
    /// The exemption must also NOT advance the key. Returning early
    /// before the read is what gives that; advancing on a wheel would
    /// make the wheel invalidate the press that follows it.
    #[test]
    fn g10b_a_stale_generation_on_a_wheel_is_exempt_but_fatal_on_a_press() {
        use pmacs_protocol::{MouseButton, MouseKind};
        let fid = FrontendId(775);
        let (editor, mut semantic_states, _render, _document, panel, _epochs) =
            panel_session_at(PROTOCOL_VERSION, fid);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
        let stale = stamped_generation(&semantic_states, fid);

        // Move the mapping, so `stale` now names a mapping that is gone —
        // exactly the state the first of two queued ticks leaves behind.
        foreign_edit(&editor, buffer_id, b"moved\n");

        // The wheel FIRST, while the key has not yet been re-read: the
        // exempt path must neither compare nor advance.
        let before = semantic_states[&fid]
            .panel_mapping_generation_peek()
            .expect("a stamped key");
        assert!(
            panel_mapping_is_current(
                &editor,
                &mut semantic_states,
                fid,
                MouseKind::ScrollDown,
                stale
            ),
            "the second queued tick still lands — without the exemption \
             the first tick advances the key and the panel scrolls once \
             and dies"
        );
        assert_eq!(
            semantic_states[&fid].panel_mapping_generation_peek(),
            Some(before),
            "and the exempt path does not advance the key: a wheel that \
             advanced it would invalidate the press that follows"
        );

        // NEGATIVE CONTROL — a press echoing the SAME stale value is
        // refused, so the acceptance above is the exemption and not a
        // check that never fires.
        assert!(
            !panel_mapping_is_current(
                &editor,
                &mut semantic_states,
                fid,
                MouseKind::Down(MouseButton::Left),
                stale
            ),
            "a press against a stale mapping is refused"
        );

        // Every coordinate-free wheel kind is exempt, not just the two
        // vertical ones.
        for kind in [
            MouseKind::ScrollUp,
            MouseKind::ScrollDown,
            MouseKind::ScrollLeft,
            MouseKind::ScrollRight,
        ] {
            assert!(
                panel_mapping_is_current(&editor, &mut semantic_states, fid, kind, stale),
                "{kind:?} is coordinate-free and exempt"
            );
        }
    }

    /// §5b G11a — **exhaustion fails CLOSED and does not leave a zombie
    /// band**.
    ///
    /// Three obligations, and each has its own failure: publish
    /// `Absent`, clear input authority, and LATCH so the next frame
    /// cannot resurrect the band.
    #[test]
    fn g11a_generation_exhaustion_publishes_absent_and_latches_for_the_session() {
        let fid = FrontendId(776);
        let (mut editor, mut semantic_states, mut render, _document, panel, epochs) =
            panel_session_at(PROTOCOL_VERSION, fid);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;

        // Precondition: a live band, and a gesture the exhaustion must
        // not silently strand.
        assert!(
            matches!(
                semantic_states[&fid].last_panel_payload_for_test(),
                Some(pmacs_protocol::PanelFramePayload::PresentMapped { .. })
            ),
            "fixture: the band is live and mapped"
        );

        // Park the key one below the ceiling against the CURRENT
        // snapshot, so the next mapping change is the advance that
        // overflows. Seeding the snapshot too keeps the "unchanged" arm
        // from firing instead.
        let snapshot = editor
            .panel_mapping_snapshot(fid)
            .expect("a presentable mapping");
        semantic_states
            .get_mut(&fid)
            .expect("projection")
            .seed_panel_mapping_generation_for_test(snapshot, u64::MAX);

        foreign_edit(&editor, buffer_id, b"one edit too many\n");
        let messages = semantic_states
            .get_mut(&fid)
            .expect("projection")
            .render_frame(&editor);

        assert!(
            messages.iter().any(|msg| matches!(
                msg,
                InstanceMessage::PanelFrame(pmacs_protocol::PanelFramePayload::Absent)
            )),
            "the band is published Absent: refusing input alone leaves a \
             stale panel painted and permanently inert"
        );
        assert!(
            semantic_states[&fid].panel_declaration().is_none(),
            "and input authority is cleared with it"
        );
        assert!(
            semantic_states[&fid].panel_mapping_generation_exhausted(),
            "and the session latches"
        );
        assert_eq!(
            semantic_states[&fid].panel_mapping_generation_peek(),
            None,
            "an exhausted session reports NO key: the stored pair still \
             holds the ceiling it stopped at, and naming it would \
             describe a key no frame carries and no gesture may echo"
        );

        // The NEXT frame must not resurrect the band. Without the latch
        // the snapshot is unchanged, the "unchanged" arm returns the
        // frozen ceiling, and a `PresentMapped` ships with a key that can
        // no longer distinguish anything.
        let next = semantic_states
            .get_mut(&fid)
            .expect("projection")
            .render_frame(&editor);
        assert!(
            !next.iter().any(|msg| matches!(
                msg,
                InstanceMessage::PanelFrame(
                    pmacs_protocol::PanelFramePayload::PresentMapped { .. }
                )
            )),
            "no zombie band on the following frame"
        );

        // And inbound stays refused for the rest of the session.
        let baseline = editor.core.borrow().views[&fid].active;
        assert_ne!(baseline, panel, "fixture: the panel is not focused");
        for echoed in [1, u64::MAX] {
            dispatch_panel_event(
                &mut editor,
                fid,
                PROTOCOL_VERSION,
                &mut semantic_states,
                &mut render,
                mapped_pointer(fid, epochs, buffer_id, echoed),
            );
            assert_eq!(
                editor.core.borrow().views[&fid].active,
                baseline,
                "an exhausted session accepts no gesture, whatever it echoes"
            );
        }
    }

    /// §5b G5a — the key advancing RAISES cancellation, without waiting
    /// for another pointer event.
    ///
    /// The daemon-side half. The EFFECTS — clearing an empty selection,
    /// clearing the click chain, delivering the child's release — are
    /// owed by `panel-pointer-replay`, which is the only branch where
    /// replay exists; this pins the trigger and the record it produces.
    #[test]
    fn g5a_a_mapping_change_cancels_a_live_gesture_with_no_further_event() {
        let fid = FrontendId(770);
        let (mut editor, mut semantic_states, mut render, _document, panel, epochs) =
            panel_session_at(PROTOCOL_VERSION, fid);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
        let generation = stamped_generation(&semantic_states, fid);

        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut semantic_states,
            &mut render,
            mapped_pointer(fid, epochs, buffer_id, generation),
        );
        assert!(
            semantic_states[&fid].has_accepted_gesture(),
            "fixture: an accepted left press arms the latch"
        );

        // A FOREIGN edit — nothing this frontend did, and no further
        // pointer event. The key moves on the next read.
        foreign_edit(&editor, buffer_id, b"moved\n");
        let snapshot = editor.panel_mapping_snapshot(fid);
        let state = semantic_states.get_mut(&fid).expect("projection");
        let advanced = state.panel_mapping_generation(snapshot);
        assert_ne!(advanced, Some(generation), "the key must have moved");

        assert!(
            !state.has_accepted_gesture(),
            "the advance itself ends the gesture — waiting for another \
             event loses the race where the successor frame lands first"
        );
        assert_eq!(
            state.panel_gesture_cancellations(),
            1,
            "and it records exactly one cancellation for replay to \
             terminate — an ordinary consume would leave this at zero"
        );
    }

    /// §5b — SUBSTRATE for the framing's G5c/G5d/G5g: the latch's
    /// arming rules, as this branch's inbound arms decide them.
    ///
    /// Deliberately not named for those rows. G5c, G5d and G5g are
    /// `panel-pointer-replay`'s per §5b's split table, and each asserts
    /// something about a synthetic release that does not exist on this
    /// branch. What is provable here is narrower and still worth
    /// pinning, because `update_accepted_gesture` decides it: which
    /// events arm, which consume, and that a consume is not a
    /// cancellation.
    #[test]
    fn g5_substrate_only_an_accepted_left_press_arms_and_a_release_consumes() {
        let fid = FrontendId(771);
        let (mut editor, mut semantic_states, mut render, _document, panel, epochs) =
            panel_session_at(PROTOCOL_VERSION, fid);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
        let generation = stamped_generation(&semantic_states, fid);
        let mut send = |kind, semantic_states: &mut HashMap<_, _>, editor: &mut _| {
            let mut event = mapped_pointer(fid, epochs, buffer_id, generation);
            if let FrontendEvent::PanelPointerMapped { kind: k, .. } = &mut event {
                *k = kind;
            }
            dispatch_panel_event(
                editor,
                fid,
                PROTOCOL_VERSION,
                semantic_states,
                &mut render,
                event,
            );
        };

        // G5d — a release with nothing armed is INERT.
        send(
            pmacs_protocol::MouseKind::Up(pmacs_protocol::MouseButton::Left),
            &mut semantic_states,
            &mut editor,
        );
        assert!(
            !semantic_states[&fid].has_accepted_gesture(),
            "a stale release terminates nothing, because nothing began"
        );

        // G5g — a RIGHT press does not arm.
        send(
            pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Right),
            &mut semantic_states,
            &mut editor,
        );
        assert!(
            !semantic_states[&fid].has_accepted_gesture(),
            "a right press opens a menu and ends there — arming would \
             manufacture a delayed release at the next authority loss"
        );

        // G5g — nor does a wheel step.
        send(
            pmacs_protocol::MouseKind::ScrollDown,
            &mut semantic_states,
            &mut editor,
        );
        assert!(!semantic_states[&fid].has_accepted_gesture());

        // Arming, then G5c — an ordinary release CONSUMES.
        send(
            pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left),
            &mut semantic_states,
            &mut editor,
        );
        assert!(semantic_states[&fid].has_accepted_gesture());
        send(
            pmacs_protocol::MouseKind::Up(pmacs_protocol::MouseButton::Left),
            &mut semantic_states,
            &mut editor,
        );
        assert!(
            !semantic_states[&fid].has_accepted_gesture(),
            "leaving it armed lets a later invalidation duplicate the \
             release for a button already up"
        );
        assert_eq!(
            semantic_states[&fid].panel_gesture_cancellations(),
            0,
            "an ordinary release is not a cancellation — counting it \
             would make replay deliver a release for a button the user \
             already lifted"
        );
    }

    /// §5b — SUBSTRATE: a second accepted press ENDS the gesture it
    /// replaces rather than overwriting it.
    ///
    /// Found by reading the arming path back, not by a failing row: a
    /// dropped `Up` — one lost to a closed outbox under a stall — is
    /// followed by the next press, and a plain overwrite discards the
    /// first gesture's record without counting it. Inert on this base,
    /// where records are only counted; once replay attaches a child
    /// release to each record, the discarded one leaves a button held
    /// down with nothing left to release it.
    #[test]
    fn g5_substrate_a_second_press_ends_the_gesture_it_replaces() {
        let fid = FrontendId(777);
        let (mut editor, mut semantic_states, mut render, _document, panel, epochs) =
            panel_session_at(PROTOCOL_VERSION, fid);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
        let generation = stamped_generation(&semantic_states, fid);
        let press = || mapped_pointer(fid, epochs, buffer_id, generation);

        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut semantic_states,
            &mut render,
            press(),
        );
        assert!(semantic_states[&fid].has_accepted_gesture());
        assert_eq!(
            semantic_states[&fid].panel_gesture_cancellations(),
            0,
            "fixture: the first press cancels nothing"
        );

        // The `Up` never arrives; the next press does.
        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut semantic_states,
            &mut render,
            press(),
        );
        assert!(
            semantic_states[&fid].has_accepted_gesture(),
            "the new gesture is armed"
        );
        assert_eq!(
            semantic_states[&fid].panel_gesture_cancellations(),
            1,
            "and the one it displaced was ENDED, not dropped on the floor"
        );
    }

    /// §5b — SUBSTRATE for the framing's G5p: per-frontend gesture
    /// ownership.
    ///
    /// A and B hold gestures on their own panels; B losing authority
    /// cancels B exactly once and leaves A's gesture intact. One global
    /// latch would make either frontend's lifecycle cancel or erase the
    /// other's.
    ///
    /// G5p itself is replay's per §5b's split table — it requires A's
    /// next valid Drag to still APPLY, which needs replay. The
    /// ownership shape underneath it is decided here, by putting the
    /// latch on `SemanticRenderState` rather than beside the dispatcher,
    /// so it is pinned here.
    #[test]
    fn g5_substrate_one_frontends_authority_loss_leaves_anothers_gesture_alone() {
        let fid_a = FrontendId(772);
        let fid_b = FrontendId(773);
        let (mut editor_a, mut states_a, mut render_a, _doc_a, panel_a, epochs_a) =
            panel_session_at(PROTOCOL_VERSION, fid_a);
        let buffer_a = editor_a.core.borrow().windows[&panel_a].buffer_id;
        let generation_a = stamped_generation(&states_a, fid_a);

        // B lives in the same projection map, so a global latch would be
        // shared between them.
        states_a.insert(
            fid_b,
            crate::semantic_render::SemanticRenderState::for_peer(fid_b, PROTOCOL_VERSION),
        );
        states_a.get_mut(&fid_b).expect("B").arm_accepted_gesture(
            crate::semantic_render::AcceptedPanelGesture {
                button: pmacs_protocol::MouseButton::Left,
                coord: pmacs_protocol::CellCoord::new(0, 0),
                buffer_id: buffer_a,
                domain: crate::editor::PanelGestureDomain::Document { window: panel_a },
            },
        );

        dispatch_panel_event(
            &mut editor_a,
            fid_a,
            PROTOCOL_VERSION,
            &mut states_a,
            &mut render_a,
            mapped_pointer(fid_a, epochs_a, buffer_a, generation_a),
        );
        assert!(states_a[&fid_a].has_accepted_gesture(), "A is armed");
        assert!(states_a[&fid_b].has_accepted_gesture(), "B is armed");

        // B loses authority.
        states_a
            .get_mut(&fid_b)
            .expect("B")
            .cancel_accepted_gesture();

        assert!(
            !states_a[&fid_b].has_accepted_gesture(),
            "B's own gesture ends"
        );
        assert_eq!(
            states_a[&fid_b].panel_gesture_cancellations(),
            1,
            "exactly once"
        );
        assert!(
            states_a[&fid_a].has_accepted_gesture(),
            "and A's survives — a global latch would have erased it"
        );
        assert_eq!(
            states_a[&fid_a].panel_gesture_cancellations(),
            0,
            "with no cancellation attributed to A"
        );
    }

    // -----------------------------------------------------------------
    // §5b review round 4 — THE LATCH FOLLOWS THE DISPATCH.
    //
    // Both inbound arms discarded the dispatcher's answer and updated
    // the accepted-gesture latch unconditionally. The ladder
    // authenticates the SENDER; only the dispatcher re-derives the
    // TARGET, so an event that clears every rung can still be refused —
    // for an out-of-grid coordinate, an absent side window, or a buffer
    // that is no longer the one shown there.
    //
    // Each row drives the refusal from a coordinate ONE PAST the last
    // row of the live grid, which also pins the dispatcher's `>=`
    // against a `>`. Each ends in a POSITIVE CONTROL differing from the
    // refused event ONLY in that coordinate: without one, a row would
    // pass just as well if some unrelated rung had dropped the event,
    // which is how a negative test of this shape usually rots.
    // -----------------------------------------------------------------

    /// Which inbound arm a latch row exercises.
    #[derive(Clone, Copy)]
    enum PanelArm {
        Legacy,
        Mapped,
    }

    impl PanelArm {
        fn version(self) -> u32 {
            match self {
                PanelArm::Legacy => LEGACY_PANEL_VERSION,
                PanelArm::Mapped => PROTOCOL_VERSION,
            }
        }
    }

    /// The mapping key the validator will compare against, read through
    /// the SAME accessor it reads through.
    ///
    /// Peeking the last STAMPED value instead would let a mapped row be
    /// refused at the mapping rung whenever the preceding dispatch moved
    /// the fingerprint — a ladder refusal wearing a dispatcher refusal's
    /// clothes. Legacy events carry no generation and never read this.
    fn live_generation(
        arm: PanelArm,
        editor: &crate::editor::EditorState,
        semantic_states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
        fid: FrontendId,
    ) -> u64 {
        match arm {
            PanelArm::Legacy => 0,
            PanelArm::Mapped => {
                let snapshot = editor.panel_mapping_snapshot(fid);
                semantic_states
                    .get_mut(&fid)
                    .expect("semantic projection")
                    .panel_mapping_generation(snapshot)
                    .expect("a stamped mapping generation")
            }
        }
    }

    /// One pointer event on whichever arm the row is testing.
    fn arm_pointer(
        arm: PanelArm,
        fid: FrontendId,
        epochs: (u64, u64),
        buffer_id: crate::buffer::BufferId,
        mapping_generation: u64,
        coord: pmacs_protocol::CellCoord,
        kind: pmacs_protocol::MouseKind,
    ) -> FrontendEvent {
        match arm {
            PanelArm::Legacy => FrontendEvent::PanelPointer {
                frontend_id: fid,
                geometry_epoch: epochs.0,
                panel_epoch: epochs.1,
                buffer_id,
                coord,
                kind,
                mods: pmacs_protocol::Modifiers::default(),
            },
            PanelArm::Mapped => FrontendEvent::PanelPointerMapped {
                frontend_id: fid,
                geometry_epoch: epochs.0,
                panel_epoch: epochs.1,
                buffer_id,
                coord,
                kind,
                mods: pmacs_protocol::Modifiers::default(),
                mapping_generation,
            },
        }
    }

    // -----------------------------------------------------------------
    // Parent 48 Q#BP-R4 — the lifecycle table's witnesses.
    //
    // Every row here reads a TARGET EFFECT, never the latch alone. The
    // framing is explicit that a latch-only assertion must not satisfy
    // these: emptying the latch is bookkeeping, and the defect being
    // fenced is precisely a gesture whose bookkeeping looks right while
    // the target heard nothing.
    // -----------------------------------------------------------------

    /// The panel's buffer, an in-content cell, and a cell on its CHROME
    /// (the mode line, which is the grid's last row per R-c).
    fn panel_buffer_and_chrome_coord(
        editor: &crate::editor::EditorState,
        fid: FrontendId,
        panel: crate::window::WindowId,
    ) -> (crate::buffer::BufferId, pmacs_protocol::CellCoord) {
        let core = editor.core.borrow();
        let grid = core.panel_grid_size(fid).expect("a live panel grid");
        let content_rows = grid.rows.saturating_sub(1);
        assert!(
            content_rows > 0,
            "fixture: the panel must have content rows"
        );
        (
            core.windows[&panel].buffer_id,
            pmacs_protocol::CellCoord::new(content_rows, 0),
        )
    }

    /// The side window's cursor, for reading a replayed effect.
    fn panel_cursor(editor: &crate::editor::EditorState, panel: crate::window::WindowId) -> u64 {
        editor.core.borrow().windows[&panel].cursor
    }

    /// P1 — a press on the band's MODE LINE begins nothing.
    ///
    /// The merge made this arm the latch, because `Consumed` and
    /// `Accepted` were the same `true`. The row reads the cursor as well
    /// as the latch: a chrome press must not move point either.
    #[test]
    fn r4_p1_a_chrome_press_neither_arms_nor_moves_point() {
        let fid = FrontendId(790);
        let (mut editor, mut states, mut render, _document, panel, epochs) =
            panel_session_at(PROTOCOL_VERSION, fid);
        let (buffer_id, chrome) = panel_buffer_and_chrome_coord(&editor, fid, panel);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let before = panel_cursor(&editor, panel);

        let generation = live_generation(PanelArm::Mapped, &editor, &mut states, fid);
        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut states,
            &mut render,
            arm_pointer(
                PanelArm::Mapped,
                fid,
                epochs,
                buffer_id,
                generation,
                chrome,
                press,
            ),
        );

        assert!(
            !states[&fid].has_accepted_gesture(),
            "a chrome press must not arm: the mode line is not content, \
             and a gesture armed there would be cancelled or released \
             for a press the target never saw"
        );
        assert_eq!(
            panel_cursor(&editor, panel),
            before,
            "and it must not move point --- `Consumed` means the panel \
             claimed the cell, not that it replayed anything"
        );
    }

    /// P7 — a release with no accepted press reaches nothing.
    ///
    /// This is the stale-tail case the pre-effect disposition exists
    /// for. The document `Up` arm clears an ACTIVE BUT EMPTY selection,
    /// so the fixture arms one and asserts it SURVIVES: an inert release
    /// must not run that clear.
    #[test]
    fn r4_p7_a_release_with_no_accepted_press_is_inert() {
        let fid = FrontendId(791);
        let (mut editor, mut states, mut render, _document, panel, epochs) =
            panel_session_at(PROTOCOL_VERSION, fid);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
        let release = pmacs_protocol::MouseKind::Up(pmacs_protocol::MouseButton::Left);
        let inside = pmacs_protocol::CellCoord::new(0, 0);

        // An empty selection is exactly what an accepted release would
        // clear, so it is the discriminator for "did the effect run".
        {
            let mut core = editor.core.borrow_mut();
            let cursor = core.windows[&panel].cursor;
            core.windows.get_mut(&panel).expect("panel").selection =
                Some(crate::window::Selection { anchor: cursor });
        }
        assert!(
            !states[&fid].has_accepted_gesture(),
            "fixture: no gesture is live"
        );

        let generation = live_generation(PanelArm::Mapped, &editor, &mut states, fid);
        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut states,
            &mut render,
            arm_pointer(
                PanelArm::Mapped,
                fid,
                epochs,
                buffer_id,
                generation,
                inside,
                release,
            ),
        );

        assert!(
            editor.core.borrow().windows[&panel].selection.is_some(),
            "a release with no accepted press must reach NOTHING --- it \
             cleared a selection it never began, which on a terminal is \
             a child tail for a press that never happened"
        );
    }

    /// P8 — a drag with no accepted press reaches nothing.
    ///
    /// The document `Drag` arm moves point unconditionally, so an
    /// orphan drag used to drive the cursor from a gesture that never
    /// began.
    #[test]
    fn r4_p8_an_orphan_drag_does_not_move_point() {
        let fid = FrontendId(792);
        let (mut editor, mut states, mut render, _document, panel, epochs) =
            panel_session_at(PROTOCOL_VERSION, fid);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
        let drag = pmacs_protocol::MouseKind::Drag(pmacs_protocol::MouseButton::Left);
        // THE PANEL BUFFER MUST HAVE CONTENT. `*panel*` is created empty,
        // so `panel_cell_byte` returns `None` for every interesting cell
        // and the drag cannot move point whether it is gated or not —
        // the row would pass vacuously. Caught by the gating mutation
        // failing to bite.
        foreign_edit(&editor, buffer_id, b"alpha beta gamma\ndelta\n");
        // A cell the cursor is NOT already on, or the row cannot fail.
        let elsewhere = pmacs_protocol::CellCoord::new(0, 6);
        let before = panel_cursor(&editor, panel);
        assert_eq!(before, 0, "fixture: point starts at the buffer head");

        let generation = live_generation(PanelArm::Mapped, &editor, &mut states, fid);
        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut states,
            &mut render,
            arm_pointer(
                PanelArm::Mapped,
                fid,
                epochs,
                buffer_id,
                generation,
                elsewhere,
                drag,
            ),
        );

        assert_eq!(
            panel_cursor(&editor, panel),
            before,
            "an orphan drag must not move point"
        );
        assert!(
            !states[&fid].has_accepted_gesture(),
            "and it must not manufacture a record by writing to one"
        );
    }

    /// P3 — a chrome release on a TERMINAL panel completes the gesture
    /// from the record.
    ///
    /// **The document leg cannot test this, and an earlier version of
    /// this row tried.** R-c lets document chrome `Up` fall THROUGH to
    /// content, so it classifies `Accepted` and takes the ordinary path;
    /// the row passed with the recorded completion deleted. Only a
    /// terminal panel returns `Consumed` for a chrome release — which is
    /// exactly the path the framing named, where the dispatcher returns
    /// before `apply_terminal_gesture`.
    ///
    /// The observable is the terminal view's DRAG STATE, not the latch:
    /// `finish_selection` is what takes it, so a gesture that ended
    /// without a delivered completion stays mid-drag forever.
    #[test]
    fn r4_p3_a_chrome_release_completes_a_terminal_gesture_from_the_record() {
        use crate::terminal::{TerminalSpec, view::TerminalViewKey};

        // LEGACY, deliberately. This row is about the Consumed/terminal
        // completion, which is family-independent — and the mapped arm
        // cannot express it, because reading the live mapping generation
        // ADVANCES the key, and §5b wired a key advance to cancel the
        // live gesture (G5a). The fixture would destroy the gesture it
        // is trying to complete, which is how an earlier version of this
        // row failed while the implementation was correct.
        let fid = FrontendId(793);
        let (mut editor, mut states, mut render, _document, panel, _epochs) =
            panel_session_at(LEGACY_PANEL_VERSION, fid);

        let mut spec = TerminalSpec::new("/bin/sh");
        spec.args = vec!["-c".into(), "sleep 30".into()];
        spec.rows = 4;
        spec.cols = 20;
        let terminal_buffer = editor
            .terminal_manager
            .borrow_mut()
            .open(
                spec,
                &mut editor.core.borrow_mut(),
                &mut editor.process_supervisor.borrow_mut(),
            )
            .expect("open panel terminal");
        {
            let mut core = editor.core.borrow_mut();
            let view = {
                let registry = core.registry.clone();
                let registry = registry.borrow();
                crate::text_view::TextView::new(
                    registry.get(terminal_buffer).expect("terminal buffer"),
                )
            };
            let window = core.windows.get_mut(&panel).expect("panel window");
            window.buffer_id = terminal_buffer;
            window.text_view = view;
        }
        // Re-ship the declaration so the epochs match the terminal panel.
        let epochs = shipped_declaration(&editor, fid, &mut states);
        let (buffer_id, chrome) = panel_buffer_and_chrome_coord(&editor, fid, panel);
        assert_eq!(
            buffer_id, terminal_buffer,
            "fixture: the panel is the terminal"
        );
        let inside = pmacs_protocol::CellCoord::new(0, 0);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let release = pmacs_protocol::MouseKind::Up(pmacs_protocol::MouseButton::Left);
        let key = TerminalViewKey::new(fid, panel, terminal_buffer);

        dispatch_panel_event(
            &mut editor,
            fid,
            LEGACY_PANEL_VERSION,
            &mut states,
            &mut render,
            arm_pointer(PanelArm::Legacy, fid, epochs, buffer_id, 0, inside, press),
        );
        assert!(
            states[&fid].has_accepted_gesture(),
            "fixture: the content press armed"
        );
        assert!(
            editor
                .terminal_manager
                .borrow()
                .view_is_dragging_for_test(key),
            "fixture: the press began a local terminal drag"
        );

        dispatch_panel_event(
            &mut editor,
            fid,
            LEGACY_PANEL_VERSION,
            &mut states,
            &mut render,
            arm_pointer(PanelArm::Legacy, fid, epochs, buffer_id, 0, chrome, release),
        );

        assert!(
            !editor
                .terminal_manager
                .borrow()
                .view_is_dragging_for_test(key),
            "the RECORDED completion must run: a terminal chrome release \
             returns before the replay path, so without it the drag never \
             finishes while the latch looks correctly empty"
        );
        assert!(
            !states[&fid].has_accepted_gesture(),
            "and the record is taken"
        );
        assert_eq!(
            states[&fid].panel_gesture_cancellations(),
            0,
            "a completion is not a cancellation"
        );
    }

    /// The exact SGR bytes a left press at cell (1, 2) produces:
    /// `ESC [ < 0 ; col+1 ; row+1 M`. Written as a LITERAL rather than
    /// built with the encoder's own formula, which would assert only
    /// that the encoder agrees with itself.
    const SGR_PRESS_1_2: &[u8] = b"\x1b[<0;3;2M";
    /// The matching release, `m` rather than `M`.
    const SGR_RELEASE_1_2: &[u8] = b"\x1b[<0;3;2m";
    /// The same release with SHIFT held: the button code gains 4.
    const SGR_RELEASE_1_2_SHIFT: &[u8] = b"\x1b[<4;3;2m";
    /// A left press at cell (2, 4), the replacement gesture's cell.
    const SGR_PRESS_2_4: &[u8] = b"\x1b[<0;5;3M";

    /// A panel session whose side window holds a live TERMINAL, with
    /// the send tap armed.
    ///
    /// Legacy, deliberately: reading the live mapping generation
    /// ADVANCES the key, and §5b wired a key advance to cancel the live
    /// gesture, so a mapped fixture destroys the gesture these rows are
    /// about.
    type TerminalPanelFixture = (
        crate::editor::EditorState,
        HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
        HashMap<FrontendId, RenderState>,
        crate::window::WindowId,
        crate::buffer::BufferId,
        (u64, u64),
    );

    fn terminal_panel_session(fid: FrontendId, reporting: bool) -> TerminalPanelFixture {
        terminal_panel_session_at(fid, reporting, LEGACY_PANEL_VERSION)
    }

    /// The same fixture at a chosen negotiated version.
    ///
    /// G5b's matrix runs every transition across BOTH families, so the
    /// version has to be a parameter rather than the legacy default the
    /// earlier rows could assume.
    fn terminal_panel_session_at(
        fid: FrontendId,
        reporting: bool,
        version: u32,
    ) -> TerminalPanelFixture {
        use crate::terminal::TerminalSpec;

        let (editor, mut states, render, _document, panel, _epochs) =
            panel_session_at(version, fid);
        let mut spec = TerminalSpec::new("/bin/sh");
        spec.args = vec!["-c".into(), "sleep 30".into()];
        spec.rows = 4;
        spec.cols = 20;
        let terminal_buffer = editor
            .terminal_manager
            .borrow_mut()
            .open(
                spec,
                &mut editor.core.borrow_mut(),
                &mut editor.process_supervisor.borrow_mut(),
            )
            .expect("open panel terminal");
        {
            let mut core = editor.core.borrow_mut();
            let view = {
                let registry = core.registry.clone();
                let registry = registry.borrow();
                crate::text_view::TextView::new(
                    registry.get(terminal_buffer).expect("terminal buffer"),
                )
            };
            let window = core.windows.get_mut(&panel).expect("panel window");
            window.buffer_id = terminal_buffer;
            window.text_view = view;
        }
        editor
            .terminal_manager
            .borrow_mut()
            .set_mouse_reporting_for_test(terminal_buffer, reporting);
        editor.terminal_manager.borrow().start_send_tap_for_test();
        let epochs = shipped_declaration(&editor, fid, &mut states);
        (editor, states, render, panel, terminal_buffer, epochs)
    }

    /// Everything the child received, in order.
    fn child_stream(editor: &crate::editor::EditorState) -> Vec<Vec<u8>> {
        editor
            .terminal_manager
            .borrow()
            .take_send_tap_for_test()
            .into_iter()
            .map(|(_, bytes)| bytes)
            .collect()
    }

    /// Send one legacy panel gesture.
    #[expect(
        clippy::too_many_arguments,
        reason = "one call shape for every G5k leg"
    )]
    fn send_panel(
        editor: &mut crate::editor::EditorState,
        states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
        render: &mut HashMap<FrontendId, RenderState>,
        fid: FrontendId,
        epochs: (u64, u64),
        buffer_id: crate::buffer::BufferId,
        coord: pmacs_protocol::CellCoord,
        kind: pmacs_protocol::MouseKind,
        mods: pmacs_protocol::Modifiers,
    ) {
        let mut event = arm_pointer(PanelArm::Legacy, fid, epochs, buffer_id, 0, coord, kind);
        if let FrontendEvent::PanelPointer { mods: slot, .. } = &mut event {
            *slot = mods;
        }
        dispatch_panel_event(editor, fid, LEGACY_PANEL_VERSION, states, render, event);
    }

    // -----------------------------------------------------------------
    // G5k — the terminal gesture-domain matrix.
    //
    // Four legs. In each, the press resolves a domain and then the
    // condition that chose it REVERSES before the tail. The gesture must
    // finish in the domain it began in; re-reading Shift, the scroll
    // position or the child's modes per event is the framing's named
    // mutation, and it either strands a child press or sends the child
    // an `Up` for a `Down` it never saw.
    // -----------------------------------------------------------------

    /// G5k(a) — child press, then the child turns REPORTING OFF: the
    /// release still reaches the child, and no local selection forms.
    #[test]
    fn g5k_a_reporting_off_mid_gesture_still_releases_to_the_child() {
        let fid = FrontendId(794);
        let (mut editor, mut states, mut render, panel, buffer_id, epochs) =
            terminal_panel_session(fid, true);
        let cell = pmacs_protocol::CellCoord::new(1, 2);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let release = pmacs_protocol::MouseKind::Up(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            press,
            none,
        );
        assert_eq!(
            child_stream(&editor),
            vec![SGR_PRESS_1_2.to_vec()],
            "fixture: the press reached the child, in SGR"
        );
        assert!(
            states[&fid]
                .accepted_gesture()
                .is_some_and(crate::semantic_render::AcceptedPanelGesture::reached_child),
            "fixture: the record says the child owns this gesture"
        );

        // The child stops reporting MID-GESTURE.
        editor
            .terminal_manager
            .borrow_mut()
            .set_mouse_reporting_for_test(buffer_id, false);

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            release,
            none,
        );

        assert_eq!(
            child_stream(&editor),
            vec![SGR_RELEASE_1_2.to_vec()],
            "the release must still reach the child AS A RELEASE at the \
             gesture's cell: it holds a button down that only this can \
             lift, and re-reading the modes here is what strands it"
        );
        let key = crate::terminal::view::TerminalViewKey::new(fid, panel, buffer_id);
        assert!(
            !editor
                .terminal_manager
                .borrow()
                .view_is_dragging_for_test(key),
            "and no local selection was started behind the child's back"
        );
    }

    /// G5k(b) — child press, then SHIFT is held before the release: the
    /// release still reaches the child.
    #[test]
    fn g5k_b_shift_before_the_release_still_releases_to_the_child() {
        let fid = FrontendId(795);
        let (mut editor, mut states, mut render, panel, buffer_id, epochs) =
            terminal_panel_session(fid, true);
        let cell = pmacs_protocol::CellCoord::new(1, 2);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let release = pmacs_protocol::MouseKind::Up(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();
        let shift = pmacs_protocol::Modifiers::SHIFT;

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            press,
            none,
        );
        assert_eq!(
            child_stream(&editor),
            vec![SGR_PRESS_1_2.to_vec()],
            "fixture: the press reached the child, in SGR"
        );

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            release,
            shift,
        );

        assert_eq!(
            child_stream(&editor),
            vec![SGR_RELEASE_1_2_SHIFT.to_vec()],
            "Shift is the LOCAL-HANDLING override for a NEW gesture, not \
             a way to abandon one already delivered to the child. The SGR \
             framing comes from the record; the modifier bits still report \
             live state, so the code is 4 rather than 0"
        );
        let key = crate::terminal::view::TerminalViewKey::new(fid, panel, buffer_id);
        assert!(
            !editor
                .terminal_manager
                .borrow()
                .view_is_dragging_for_test(key),
            "and no local selection was started"
        );
    }

    /// G5k(c) — SHIFT press starts a LOCAL gesture; releasing without
    /// Shift finishes locally and sends the child nothing.
    #[test]
    fn g5k_c_a_shift_started_gesture_finishes_locally() {
        let fid = FrontendId(796);
        let (mut editor, mut states, mut render, panel, buffer_id, epochs) =
            terminal_panel_session(fid, true);
        let cell = pmacs_protocol::CellCoord::new(1, 2);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let release = pmacs_protocol::MouseKind::Up(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();
        let shift = pmacs_protocol::Modifiers::SHIFT;
        let key = crate::terminal::view::TerminalViewKey::new(fid, panel, buffer_id);

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            press,
            shift,
        );
        assert!(
            child_stream(&editor).is_empty(),
            "fixture: a Shift press is handled locally"
        );
        assert!(
            editor
                .terminal_manager
                .borrow()
                .view_is_dragging_for_test(key),
            "fixture: it began a local drag"
        );

        // Shift released before the button.
        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            release,
            none,
        );

        assert!(
            child_stream(&editor).is_empty(),
            "the child must receive NOTHING: it never saw the press, so \
             an Up here is a release for a Down that never happened"
        );
        assert!(
            !editor
                .terminal_manager
                .borrow()
                .view_is_dragging_for_test(key),
            "and the local drag finishes"
        );
    }

    /// G5k(d) — a press taken locally because reporting was OFF stays
    /// local when the child turns reporting ON mid-gesture.
    #[test]
    fn g5k_d_reporting_on_mid_gesture_does_not_capture_a_local_gesture() {
        let fid = FrontendId(797);
        let (mut editor, mut states, mut render, panel, buffer_id, epochs) =
            terminal_panel_session(fid, false);
        let cell = pmacs_protocol::CellCoord::new(1, 2);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let release = pmacs_protocol::MouseKind::Up(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();
        let key = crate::terminal::view::TerminalViewKey::new(fid, panel, buffer_id);

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            press,
            none,
        );
        assert!(
            child_stream(&editor).is_empty(),
            "fixture: reporting is off, so the press is local"
        );
        assert!(
            editor
                .terminal_manager
                .borrow()
                .view_is_dragging_for_test(key),
            "fixture: it began a local drag"
        );

        // The child turns reporting ON mid-gesture.
        editor
            .terminal_manager
            .borrow_mut()
            .set_mouse_reporting_for_test(buffer_id, true);

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            release,
            none,
        );

        assert!(
            child_stream(&editor).is_empty(),
            "the child must receive NOTHING --- it never saw this press"
        );
        assert!(
            !editor
                .terminal_manager
                .borrow()
                .view_is_dragging_for_test(key),
            "and the local gesture still finishes locally"
        );
    }

    /// P3, reporting leg — a chrome release still delivers the child's
    /// release, in the RECORDED encoding.
    #[test]
    fn r4_p3_child_a_chrome_release_still_reaches_the_reporting_child() {
        let fid = FrontendId(798);
        let (mut editor, mut states, mut render, _panel, buffer_id, epochs) =
            terminal_panel_session(fid, true);
        let cell = pmacs_protocol::CellCoord::new(1, 2);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let release = pmacs_protocol::MouseKind::Up(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();
        let chrome = {
            let core = editor.core.borrow();
            let grid = core.panel_grid_size(fid).expect("grid");
            pmacs_protocol::CellCoord::new(grid.rows.saturating_sub(1), 0)
        };

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            press,
            none,
        );
        assert_eq!(
            child_stream(&editor),
            vec![SGR_PRESS_1_2.to_vec()],
            "fixture: the press reached the child, in SGR"
        );

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            chrome,
            release,
            none,
        );

        assert_eq!(
            child_stream(&editor),
            vec![SGR_RELEASE_1_2.to_vec()],
            "the chrome release must terminate the child's gesture AT THE \
             GESTURE'S OWN CELL, not the chrome cell it landed on (R-c2). \
             The terminal path returns before replay, so without the \
             recorded completion the child holds the button forever"
        );
        assert!(
            !states[&fid].has_accepted_gesture(),
            "and the record is taken"
        );
    }

    /// P9 — a document press that ANCHORS NOTHING does not arm.
    ///
    /// `panel_cell_byte` is `None` past the end of a short line and on
    /// an empty panel, and the press then places no cursor and opens no
    /// selection. Arming over it records a gesture whose completion has
    /// nothing to complete — and, once cancellations deliver effects,
    /// one that fires a release for a press that did nothing.
    #[test]
    fn r4_p9_a_document_press_that_anchors_nothing_does_not_arm() {
        let fid = FrontendId(802);
        let (mut editor, mut states, mut render, _document, panel, epochs) =
            panel_session_at(LEGACY_PANEL_VERSION, fid);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();
        // The panel grid is 4 rows, so content is rows 0..=2 and row 3
        // is the mode line. The `*panel*` buffer is created EMPTY --- one
        // zero-length line --- so row 1 is IN CONTENT and still past the
        // buffer's only line.
        //
        // Two cells that do NOT work, and the row asserts its way past
        // both: a column past the end of row 0 clamps to the line end
        // and yields byte 0, and any row >= 4 is out of grid, so the
        // press is refused before the document path is reached at all.
        let unanchorable = pmacs_protocol::CellCoord::new(1, 0);
        assert!(
            editor
                .classify_panel_pointer(fid, buffer_id, unanchorable, press)
                .outcome()
                == crate::editor::PanelPointerOutcome::Accepted,
            "fixture: the cell must be ACCEPTED content, or this row \
             passes because the press was refused for an unrelated reason"
        );
        assert!(
            editor
                .panel_cell_byte_for_test(panel, unanchorable)
                .is_none(),
            "fixture: this cell must genuinely have no byte behind it"
        );

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            unanchorable,
            press,
            none,
        );

        assert!(
            !states[&fid].has_accepted_gesture(),
            "a press that anchored nothing must not arm"
        );
    }

    /// P10 — a LOCAL terminal press that begins no drag does not arm.
    ///
    /// **The panel is wider than its terminal.** The fixture's band is
    /// 80 columns while the child's screen is 20, so columns 20..79 are
    /// painted padding: inside accepted panel content, and behind no
    /// terminal cell. `anchor_at` refuses them
    /// (`coord.col >= row.cells.len()`), `begin_selection` returns
    /// `false`, and nothing begins.
    ///
    /// An earlier round recorded this gate as UNWITNESSED, claiming
    /// `anchor_at` resolves every in-grid cell of a live view. That was
    /// measured on ROWS and generalised to cells, which is a different
    /// statement and a false one.
    #[test]
    fn r4_p10_a_local_press_that_begins_no_drag_does_not_arm() {
        let fid = FrontendId(803);
        let (mut editor, mut states, mut render, panel, buffer_id, epochs) =
            terminal_panel_session(fid, false);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();
        let key = crate::terminal::view::TerminalViewKey::new(fid, panel, buffer_id);
        // Column 20 of an 80-column band over a 20-column child.
        let padding = pmacs_protocol::CellCoord::new(1, 20);
        assert_eq!(
            editor
                .classify_panel_pointer(fid, buffer_id, padding, press)
                .outcome(),
            crate::editor::PanelPointerOutcome::Accepted,
            "fixture: the cell is ACCEPTED content, so a refusal cannot \
             be what this row observes"
        );

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            padding,
            press,
            none,
        );

        assert!(
            !editor
                .terminal_manager
                .borrow()
                .view_is_dragging_for_test(key),
            "fixture: no local drag began --- the cell is painted padding"
        );
        assert!(
            !states[&fid].has_accepted_gesture(),
            "so nothing may be armed: the record would name a drag that \
             does not exist, and its completion would have nothing to \
             finish"
        );
    }

    // -----------------------------------------------------------------
    // Q1-Q6 — the pending-release slot, and the ORDER it drains in.
    //
    // A cancellation raised inside frame production cannot deliver its
    // own release. The slot is where the record waits; these rows are
    // about it being paid, and paid at the right moment.
    // -----------------------------------------------------------------

    /// Drive a real `SessionDetached` through the dispatcher.
    fn detach_session(
        editor: &mut crate::editor::EditorState,
        semantic_states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
        render_states: &mut HashMap<FrontendId, RenderState>,
        fid: FrontendId,
    ) {
        let mut streams = HashMap::new();
        let mut term_sizes = HashMap::new();
        let mut last_idle = HashMap::new();
        let mut last_active = HashMap::new();
        let mut bells = HashMap::new();
        let mut registry = SessionRegistry::new();
        registry.register_session(fid, session(LEGACY_PANEL_VERSION, true));
        handle_dispatcher_event(
            DispatcherEvent::SessionDetached { frontend_id: fid },
            editor,
            render_states,
            semantic_states,
            &mut streams,
            &mut term_sizes,
            &mut last_idle,
            &mut last_active,
            &mut bells,
            &mut registry,
        );
    }

    /// Cancel the live gesture from INSIDE PROJECTION, by taking the
    /// panel away.
    ///
    /// `publish_absent_panel` cancels while `render_frame` is building
    /// the frame, which is the shape these rows are about: a
    /// cancellation with nowhere to deliver its release.
    ///
    /// A mapping-generation advance is the other such trigger and would
    /// read more naturally, but it cannot be driven on a TERMINAL
    /// panel: that key tracks the screen and anchor, not the buffer, so
    /// a foreign edit does not move it. `Absent` is family-independent
    /// and reaches the same parking path.
    /// Returns the epochs a later gesture must echo: re-showing the
    /// panel ships a NEW declaration, and the old epochs are stale.
    fn cancel_by_panel_absence(
        editor: &crate::editor::EditorState,
        states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
        fid: FrontendId,
    ) -> (u64, u64) {
        editor.hide_panel_for_test(fid);
        {
            let sem = states.get_mut(&fid).expect("projection");
            let _ = sem.render_frame(editor);
        }
        // Re-shown and re-declared, because these rows are about what
        // happens to the OWED RELEASE afterwards. A panel left `Absent`
        // fails the inbound ladder, so no later gesture would reach the
        // drain at all and the row would be observing the ladder rather
        // than the slot.
        editor.show_panel_for_test(fid);
        shipped_declaration(editor, fid, states)
    }

    /// Q1/Q2 — a cancelled gesture's release is PARKED and then PAID.
    ///
    /// The cancellation happens inside projection, where no target
    /// effect can run; dropping the record there is how the child ended
    /// up holding a button with nothing left to lift it.
    #[test]
    fn q1_q2_a_cancelled_gesture_release_is_parked_then_delivered() {
        let fid = FrontendId(810);
        let (mut editor, mut states, mut render, _panel, buffer_id, epochs) =
            terminal_panel_session(fid, true);
        let cell = pmacs_protocol::CellCoord::new(1, 2);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            press,
            none,
        );
        assert_eq!(
            child_stream(&editor),
            vec![SGR_PRESS_1_2.to_vec()],
            "fixture: the press reached the child"
        );

        let epochs = cancel_by_panel_absence(&editor, &mut states, fid);
        assert!(
            !states[&fid].has_accepted_gesture(),
            "fixture: the advance cancelled the gesture"
        );
        assert!(
            states[&fid].has_pending_release(),
            "Q1: the record must be PARKED, not returned into a context \
             that drops it"
        );
        assert!(
            child_stream(&editor).is_empty(),
            "and not delivered from inside projection, which cannot run \
             a target effect"
        );

        // A later panel event: the drain runs ahead of it.
        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            press,
            none,
        );

        let sent = child_stream(&editor);
        assert!(
            sent.contains(&SGR_RELEASE_1_2.to_vec()),
            "Q2: the parked release must be PAID --- parking it and never \
             draining leaves the child exactly as stranded, got {sent:?}"
        );
    }

    /// Q3 — the owed release reaches the child BEFORE the next press.
    ///
    /// Draining after the dispatch instead of before puts them on the
    /// wire reversed, which the child reads as a press followed by the
    /// release of the gesture before it.
    #[test]
    fn q3_the_owed_release_precedes_the_next_press() {
        let fid = FrontendId(811);
        let (mut editor, mut states, mut render, _panel, buffer_id, epochs) =
            terminal_panel_session(fid, true);
        let cell = pmacs_protocol::CellCoord::new(1, 2);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            press,
            none,
        );
        let epochs = cancel_by_panel_absence(&editor, &mut states, fid);
        let _ = child_stream(&editor);

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            press,
            none,
        );

        let sent = child_stream(&editor);
        let release_at = sent.iter().position(|b| b == SGR_RELEASE_1_2);
        let press_at = sent.iter().position(|b| b == SGR_PRESS_1_2);
        assert!(
            release_at.is_some() && press_at.is_some(),
            "fixture: both the owed release and the new press must be on \
             the wire, got {sent:?}"
        );
        assert!(
            release_at < press_at,
            "Q3: ORDER, not merely arrival --- the old gesture's release \
             must precede the new gesture's press, got {sent:?}"
        );
    }

    /// Q4 — detach pays what it owes before tearing the state down.
    ///
    /// `SessionDetached` drops `semantic_states` for the frontend, and
    /// there is no later opportunity: a release not delivered here is
    /// never delivered.
    #[test]
    fn q4_detach_delivers_an_owed_release_before_teardown() {
        let fid = FrontendId(812);
        let (mut editor, mut states, mut render, _panel, buffer_id, epochs) =
            terminal_panel_session(fid, true);
        let cell = pmacs_protocol::CellCoord::new(1, 2);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            press,
            none,
        );
        // Detach never sends another gesture, so the fresh epochs are
        // not needed here --- only the parked release is.
        let _ = cancel_by_panel_absence(&editor, &mut states, fid);
        assert!(
            states[&fid].has_pending_release(),
            "fixture: a release is owed"
        );
        let _ = child_stream(&editor);

        detach_session(&mut editor, &mut states, &mut render, fid);

        assert_eq!(
            child_stream(&editor),
            vec![SGR_RELEASE_1_2.to_vec()],
            "Q4: the owed release must be paid before teardown --- the \
             next statement in that arm drops the state holding it"
        );
        assert!(
            !states.contains_key(&fid),
            "fixture: and the teardown really did run"
        );
    }

    /// Q5 — a release raised by PROJECTION is paid before the frame
    /// that raised it can be written.
    ///
    /// This is the third drain, and the only one a unit row could not
    /// reach before: it sits inside the daemon's per-frontend frame
    /// loop. `project_semantic_frame` is that seam, extracted --- it
    /// returns the messages UNWRITTEN, so a caller holding them has by
    /// construction not sent the successor frame yet, and a release
    /// already delivered at that moment provably precedes it.
    ///
    /// Recorded as OWED through tasks 18 and 19; this closes it.
    #[test]
    fn q5_a_projection_raised_release_precedes_the_frame_that_raised_it() {
        let fid = FrontendId(870);
        let (mut editor, mut states, mut render, _panel, buffer_id, epochs) =
            terminal_panel_session_at(fid, true, PROTOCOL_VERSION);
        let cell = pmacs_protocol::CellCoord::new(1, 2);

        send_press(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            PROTOCOL_VERSION,
            epochs,
            buffer_id,
            cell,
        );
        assert_eq!(
            child_stream(&editor),
            vec![SGR_PRESS_1_2.to_vec()],
            "fixture: the press reached the child"
        );
        assert!(
            states[&fid].has_accepted_gesture(),
            "fixture: the gesture is live"
        );

        // The panel stops being presentable. `publish_absent_panel`
        // cancels from INSIDE projection, which cannot deliver.
        editor.hide_panel_for_test(fid);

        let messages = project_semantic_frame(&mut editor, &mut states, fid);

        assert_eq!(
            child_stream(&editor),
            vec![SGR_RELEASE_1_2.to_vec()],
            "the release must already be delivered when projection hands \
             its messages back --- the caller has not written them, so \
             this is the release preceding the successor frame, not \
             merely both arriving"
        );
        assert!(
            messages.iter().any(|message| matches!(
                message,
                InstanceMessage::PanelFrame(pmacs_protocol::panel::PanelFramePayload::Absent)
            )),
            "fixture: the unwritten messages must contain THE SUCCESSOR \
             FRAME --- the `Absent` payload whose own transition raised \
             the release. A non-empty vec proves nothing: any unrelated \
             semantic message would satisfy it, and the row would then \
             assert an ordering against a frame that was never there. \
             Got {messages:?}"
        );
        assert!(
            !states[&fid].has_pending_release(),
            "and nothing is left owed"
        );
    }

    /// Q6 — a SECOND PRESS with the first still live: the old release
    /// reaches the child before the new press.
    ///
    /// This is the ordering the entry drain alone cannot give. That
    /// drain looks for an OWED release, and a live gesture owes
    /// nothing yet — it is cancelled by arming, which happens after the
    /// replacement press has already reached the target. The child then
    /// saw `old press, new press, old release`: two presses
    /// outstanding, and a release arriving for the wrong one.
    ///
    /// An earlier version of this row never sent a second press while
    /// the first was live, so it could not observe any of that.
    #[test]
    fn q6_a_second_press_pays_the_first_before_it_lands() {
        let fid = FrontendId(813);
        let (mut editor, mut states, mut render, _panel, buffer_id, epochs) =
            terminal_panel_session(fid, true);
        let first = pmacs_protocol::CellCoord::new(1, 2);
        let second = pmacs_protocol::CellCoord::new(2, 4);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            first,
            press,
            none,
        );
        assert_eq!(
            child_stream(&editor),
            vec![SGR_PRESS_1_2.to_vec()],
            "fixture: the first press reached the child"
        );
        assert!(
            states[&fid].has_accepted_gesture(),
            "fixture: and it is still LIVE --- no release was sent"
        );

        // The second press, with the first never released.
        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            second,
            press,
            none,
        );

        assert_eq!(
            child_stream(&editor),
            vec![SGR_RELEASE_1_2.to_vec(), SGR_PRESS_2_4.to_vec()],
            "the first gesture's release must reach the child BEFORE the \
             replacement press --- the release is at the first gesture's \
             own cell, and it comes first"
        );
        assert!(
            states[&fid].has_accepted_gesture(),
            "and the replacement is armed"
        );
        assert!(
            !states[&fid].has_pending_release(),
            "with nothing left owed"
        );
    }

    // -----------------------------------------------------------------
    // G5b — the common authority-loss matrix, table-driven.
    //
    // Four transitions x two families x two targets. §5b wired `Absent`
    // and left these armed --- inert while nothing consumed the latch,
    // defects the moment cancellation gained an effect.
    //
    // Every quadrant asserts the EFFECT, drained explicitly: the exact
    // release bytes for a reporting terminal, the cleared empty
    // selection for a document, and an empty pending slot afterwards.
    // Stopping at `has_pending_release()` would pass while delivery or
    // recorded-domain routing was broken, which is what an earlier
    // version of these rows did.
    // -----------------------------------------------------------------

    /// Which authority loss a matrix row drives.
    #[derive(Clone, Copy, Debug)]
    enum LossCause {
        /// The panel stops being presentable at all.
        ///
        /// §5b wired this one, so it is the matrix's CONTROL as much as
        /// its fifth row: if the four this lane added regressed while
        /// `Absent` still worked, the matrix would say so.
        Absent,
        WindowReplaced,
        BufferReplaced,
        GeometrySameSize,
        Detach,
    }

    /// Which target the gesture was pressed on.
    #[derive(Clone, Copy, Debug)]
    enum LossTarget {
        Document,
        ReportingTerminal,
    }

    /// A panel session of the requested target kind and family.
    fn loss_fixture(fid: FrontendId, version: u32, target: LossTarget) -> TerminalPanelFixture {
        match target {
            LossTarget::ReportingTerminal => terminal_panel_session_at(fid, true, version),
            LossTarget::Document => {
                let (editor, mut states, render, _document, panel, _epochs) =
                    panel_session_at(version, fid);
                let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
                // A press must ANCHOR, or the row never arms.
                foreign_edit(&editor, buffer_id, b"alpha beta gamma\ndelta\n");
                let epochs = shipped_declaration(&editor, fid, &mut states);
                (editor, states, render, panel, buffer_id, epochs)
            }
        }
    }

    /// Send one press in whichever family `version` negotiated.
    #[expect(
        clippy::too_many_arguments,
        reason = "one call shape for every matrix quadrant"
    )]
    fn send_press(
        editor: &mut crate::editor::EditorState,
        states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
        render: &mut HashMap<FrontendId, RenderState>,
        fid: FrontendId,
        version: u32,
        epochs: (u64, u64),
        buffer_id: crate::buffer::BufferId,
        coord: pmacs_protocol::CellCoord,
    ) {
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();
        if version >= pmacs_protocol::PANEL_MAPPING_MIN_VERSION {
            let generation = live_generation(PanelArm::Mapped, editor, states, fid);
            let event = arm_pointer(
                PanelArm::Mapped,
                fid,
                epochs,
                buffer_id,
                generation,
                coord,
                press,
            );
            dispatch_panel_event(editor, fid, version, states, render, event);
        } else {
            send_panel(
                editor, states, render, fid, epochs, buffer_id, coord, press, none,
            );
        }
    }

    /// Replace the side window's buffer, keeping the window.
    fn replace_panel_buffer(editor: &crate::editor::EditorState, panel: crate::window::WindowId) {
        let mut core = editor.core.borrow_mut();
        let replacement = core.registry.borrow_mut().create("*replacement*");
        let view = {
            let registry = core.registry.clone();
            let registry = registry.borrow();
            crate::text_view::TextView::new(registry.get(replacement).expect("replacement"))
        };
        let window = core.windows.get_mut(&panel).expect("panel window");
        window.buffer_id = replacement;
        window.text_view = view;
    }

    /// Close and reopen the panel on the same buffer: a new window.
    fn replace_panel_window(
        editor: &crate::editor::EditorState,
        fid: FrontendId,
        panel: crate::window::WindowId,
        buffer_id: crate::buffer::BufferId,
    ) {
        {
            let mut core = editor.core.borrow_mut();
            core.active_frontend = fid;
            core.focus_window(fid, panel);
            assert!(core.close_active(), "fixture: the side window closes");
        }
        editor.reconcile_panel_layout(fid);
        {
            let mut core = editor.core.borrow_mut();
            let mut request = crate::editor_core::DisplayRequest::new(buffer_id);
            request.side = Some(crate::window::Side::Bottom);
            request.height = Some(4);
            core.display_buffer(fid, &request)
                .expect("reopen the panel");
        }
        editor.reconcile_panel_layout(fid);
        assert_ne!(
            editor.core.borrow().side_window_for(fid).expect("reopened"),
            panel,
            "fixture: the successor really is a different window"
        );
    }

    /// G5b — every transition, every family, every target.
    #[expect(
        clippy::too_many_lines,
        reason = "one table whose quadrants read together; splitting it \
                  hides which combinations are covered"
    )]
    #[test]
    fn g5b_the_authority_loss_matrix() {
        use LossCause::{Absent, BufferReplaced, Detach, GeometrySameSize, WindowReplaced};
        use LossTarget::{Document, ReportingTerminal};

        let mut next_fid = 830u64;
        let mut quadrants = 0usize;
        for cause in [
            Absent,
            WindowReplaced,
            BufferReplaced,
            GeometrySameSize,
            Detach,
        ] {
            for target in [Document, ReportingTerminal] {
                for version in [LEGACY_PANEL_VERSION, PROTOCOL_VERSION] {
                    let fid = FrontendId(next_fid);
                    next_fid += 1;
                    let label = format!("{cause:?}/{target:?}/v{version}");
                    quadrants += 1;

                    let (mut editor, mut states, mut render, panel, buffer_id, epochs) =
                        loss_fixture(fid, version, target);
                    // The terminal legs use (1, 2), whose exact SGR
                    // bytes the constants above pin. A document panel
                    // uses row 0: `foreign_edit` replaces the buffer's
                    // contents without refreshing the window's cached
                    // line index, so only the first display row resolves
                    // to a byte --- and a press that anchors nothing does
                    // not arm, which would make every document leg
                    // vacuous.
                    let cell = match target {
                        ReportingTerminal => pmacs_protocol::CellCoord::new(1, 2),
                        Document => pmacs_protocol::CellCoord::new(0, 2),
                    };

                    send_press(
                        &mut editor,
                        &mut states,
                        &mut render,
                        fid,
                        version,
                        epochs,
                        buffer_id,
                        cell,
                    );
                    assert!(
                        states[&fid].has_accepted_gesture(),
                        "{label}: fixture --- the press must arm, or the row \
                         proves nothing"
                    );
                    if matches!(target, ReportingTerminal) {
                        assert_eq!(
                            child_stream(&editor),
                            vec![SGR_PRESS_1_2.to_vec()],
                            "{label}: fixture --- the press reached the child"
                        );
                    }
                    let selection_before = editor.core.borrow().windows[&panel].selection;

                    match cause {
                        Absent => {
                            editor.hide_panel_for_test(fid);
                            render_and_redeclare(&editor, &mut states, fid);
                        }
                        WindowReplaced => {
                            replace_panel_window(&editor, fid, panel, buffer_id);
                            render_and_redeclare(&editor, &mut states, fid);
                        }
                        BufferReplaced => {
                            replace_panel_buffer(&editor, panel);
                            render_and_redeclare(&editor, &mut states, fid);
                        }
                        GeometrySameSize => {
                            editor.accept_semantic_frame_geometry(fid, 2, CellSize::new(24, 80));
                            render_and_redeclare(&editor, &mut states, fid);
                        }
                        Detach => {
                            detach_session(&mut editor, &mut states, &mut render, fid);
                        }
                    }
                    // Detach drains inside its own teardown; the producer
                    // transitions park and are paid at the next drain.
                    if !matches!(cause, Detach) {
                        assert!(
                            !states[&fid].has_accepted_gesture(),
                            "{label}: the gesture must END --- it belongs to \
                             a presentation that no longer exists"
                        );
                        drain_pending_release(&mut editor, &mut states, fid);
                        assert!(
                            !states[&fid].has_pending_release(),
                            "{label}: and the slot must be empty afterwards"
                        );
                    }

                    match target {
                        ReportingTerminal => assert_eq!(
                            child_stream(&editor),
                            vec![SGR_RELEASE_1_2.to_vec()],
                            "{label}: EXACTLY the recorded release, to the \
                             child the gesture was pressed on --- routing it \
                             by what occupies the panel now would tell the \
                             wrong child to lift a button"
                        ),
                        Document => {
                            assert!(
                                selection_before.is_some(),
                                "{label}: fixture --- the press anchored an \
                                 empty selection for the cancellation to clear"
                            );
                            let core = editor.core.borrow();
                            match core.windows.get(&panel) {
                                Some(window) => assert!(
                                    window.selection.is_none(),
                                    "{label}: the empty selection must be \
                                     cleared, or its stale anchor captures \
                                     the next shift-motion"
                                ),
                                // WINDOW REPLACED: the window the gesture
                                // belonged to is gone, so the completion
                                // has nothing left to clear and the
                                // gesture ENDING is the whole of the
                                // effect --- already asserted above. Said
                                // out loud rather than skipped, because a
                                // silently absent assertion is how a
                                // quadrant stops testing anything.
                                None => assert!(
                                    matches!(cause, WindowReplaced),
                                    "{label}: the panel window vanished for \
                                     a cause that should not remove it --- \
                                     `Absent` hides the panel and leaves the \
                                     window, so only a replacement may land here"
                                ),
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(
            quadrants, 20,
            "FIVE transitions x two families x two targets. Asserted \
             because a loop that silently stops covering a combination \
             passes exactly as loudly as one that covers them all"
        );
    }

    /// G5m — coincident invalidations produce ONE completion effect.
    ///
    /// Both composites the framing names, and each reads the effect
    /// rather than the cancellation count: a count of one proves the
    /// latch was taken once, not that exactly one release went out.
    #[test]
    fn g5m_coincident_invalidations_emit_one_release() {
        // (a) changed-size geometry, which moves the geometry epoch AND
        //     the mapping generation; (b) buffer replacement, which
        //     moves the identity AND the mapping.
        for (label, changed_size) in [("changed-size geometry", true), ("buffer replaced", false)] {
            let fid = FrontendId(if changed_size { 850 } else { 851 });
            let (mut editor, mut states, mut render, panel, buffer_id, epochs) =
                terminal_panel_session_at(fid, true, PROTOCOL_VERSION);
            let cell = pmacs_protocol::CellCoord::new(1, 2);

            send_press(
                &mut editor,
                &mut states,
                &mut render,
                fid,
                PROTOCOL_VERSION,
                epochs,
                buffer_id,
                cell,
            );
            assert_eq!(
                child_stream(&editor),
                vec![SGR_PRESS_1_2.to_vec()],
                "{label}: fixture --- the press reached the child"
            );

            // The composite is only a composite if the MAPPING moves
            // too. Peeked, not read through the authoritative accessor,
            // which would advance the key and manufacture the very
            // second cause this row is supposed to observe.
            let mapping_before = stamped_generation(&states, fid);
            if changed_size {
                editor.accept_semantic_frame_geometry(fid, 2, CellSize::new(20, 60));
            } else {
                replace_panel_buffer(&editor, panel);
            }
            render_and_redeclare(&editor, &mut states, fid);
            assert_ne!(
                stamped_generation(&states, fid),
                mapping_before,
                "{label}: fixture --- the mapping generation must actually \
                 ADVANCE, or this is a single-cause transition wearing a \
                 composite's name and the row proves nothing about \
                 coincidence"
            );
            drain_pending_release(&mut editor, &mut states, fid);

            assert_eq!(
                child_stream(&editor),
                vec![SGR_RELEASE_1_2.to_vec()],
                "{label}: TWO causes, ONE release --- they take the same \
                 latch, and a per-cause flag would send the child two \
                 releases for one press"
            );
            assert!(
                !states[&fid].has_pending_release(),
                "{label}: with nothing left owed"
            );
        }
    }

    /// G5j — document cancellation has two legs, and they differ.
    ///
    /// A press that never dragged leaves an ACTIVE BUT EMPTY selection
    /// whose stale anchor would capture the next shift-motion, so it is
    /// cleared. A press that DID drag leaves a real region the user
    /// selected, and cancelling the gesture must not take it away.
    #[test]
    fn g5j_document_cancellation_clears_only_an_empty_selection() {
        for dragged in [false, true] {
            let fid = FrontendId(if dragged { 861 } else { 860 });
            let (mut editor, mut states, mut render, panel, buffer_id, epochs) =
                loss_fixture(fid, LEGACY_PANEL_VERSION, LossTarget::Document);
            let drag = pmacs_protocol::MouseKind::Drag(pmacs_protocol::MouseButton::Left);
            let none = pmacs_protocol::Modifiers::default();

            send_press(
                &mut editor,
                &mut states,
                &mut render,
                fid,
                LEGACY_PANEL_VERSION,
                epochs,
                buffer_id,
                pmacs_protocol::CellCoord::new(0, 0),
            );
            if dragged {
                send_panel(
                    &mut editor,
                    &mut states,
                    &mut render,
                    fid,
                    epochs,
                    buffer_id,
                    pmacs_protocol::CellCoord::new(0, 5),
                    drag,
                    none,
                );
            }
            let before = {
                let core = editor.core.borrow();
                let window = &core.windows[&panel];
                (window.selection, window.cursor)
            };
            assert!(
                before.0.is_some(),
                "fixture: a selection is anchored either way"
            );

            editor.accept_semantic_frame_geometry(fid, 2, CellSize::new(24, 80));
            render_and_redeclare(&editor, &mut states, fid);
            drain_pending_release(&mut editor, &mut states, fid);

            let after = {
                let core = editor.core.borrow();
                let window = &core.windows[&panel];
                (window.selection, window.cursor)
            };
            if dragged {
                assert_eq!(
                    after, before,
                    "a REAL region survives cancellation, anchor and \
                     cursor exactly --- the user selected it, and ending \
                     the gesture is not a reason to discard it"
                );
            } else {
                assert!(
                    after.0.is_none(),
                    "an EMPTY selection is cleared, or its stale anchor \
                     captures the next shift-motion"
                );
                assert_eq!(after.1, before.1, "and clearing it does not move point");
            }
        }
    }

    /// Render one frame, which is where the producer notices an
    /// authority loss.
    fn render_and_redeclare(
        editor: &crate::editor::EditorState,
        states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
        fid: FrontendId,
    ) {
        let sem = states.get_mut(&fid).expect("projection");
        let _ = sem.render_frame(editor);
    }

    /// P12 — a panel WIDER THAN 512 COLUMNS still routes pointer input.
    ///
    /// A panel deliberately does not inherit the terminal's per-axis PTY
    /// caps (Bet B5'): a 4K surface at a small font is legitimately
    /// wider than `MAX_TERMINAL_COLS`, and the renderer clamps through
    /// `terminal_projection_size` so the band paints. Pointer routing
    /// passed the RAW panel width, and `view_status_for_size` refuses
    /// anything over the cap — so on exactly those panels a click
    /// squarely inside the visible terminal resolved to nothing and the
    /// whole band was dead to the mouse while looking perfectly normal.
    ///
    /// A POSITIVE control: the gesture must land, not merely fail
    /// safely.
    #[test]
    fn r4_p12_a_panel_wider_than_the_terminal_cap_still_routes_pointer_input() {
        let fid = FrontendId(805);
        let (mut editor, mut states, mut render, panel, buffer_id, _epochs) =
            terminal_panel_session(fid, true);

        // Re-declare the surface far wider than MAX_TERMINAL_COLS.
        let wide = u32::from(crate::terminal::MAX_TERMINAL_COLS) + 128;
        editor.accept_semantic_frame_geometry(fid, 2, CellSize::new(24, wide));
        let epochs = shipped_declaration(&editor, fid, &mut states);
        {
            let core = editor.core.borrow();
            let grid = core.panel_grid_size(fid).expect("a live panel grid");
            assert!(
                grid.cols > u32::from(crate::terminal::MAX_TERMINAL_COLS),
                "fixture: the band must actually exceed the terminal cap, \
                 got {} columns",
                grid.cols
            );
        }

        let cell = pmacs_protocol::CellCoord::new(1, 2);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();
        let _ = child_stream(&editor);

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            press,
            none,
        );

        assert_eq!(
            child_stream(&editor),
            vec![SGR_PRESS_1_2.to_vec()],
            "the press must reach the child on a wide band: routing has \
             to use the SAME projection clamp the renderer uses, or the \
             two disagree and the panel paints while ignoring the mouse"
        );
        assert!(
            states[&fid].has_accepted_gesture(),
            "and it must arm --- this is a positive control, so failing \
             safely is still failing"
        );
        let _ = panel;
    }

    /// P11 — a recorded LOCAL completion still runs with the panel
    /// HIDDEN.
    ///
    /// `panel_grid_size` is `None` for a hidden panel, so a completion
    /// that fetched the current geometry could not finish a drag during
    /// exactly the cancellations that need finishing. The viewport is
    /// recorded at the press for this reason, and the row hides the
    /// panel between the press and the completion to prove it.
    #[test]
    fn r4_p11_a_recorded_local_completion_survives_a_hidden_panel() {
        let fid = FrontendId(804);
        let (mut editor, mut states, mut render, panel, buffer_id, epochs) =
            terminal_panel_session(fid, false);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();
        let cell = pmacs_protocol::CellCoord::new(1, 2);
        let key = crate::terminal::view::TerminalViewKey::new(fid, panel, buffer_id);

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            press,
            none,
        );
        assert!(
            editor
                .terminal_manager
                .borrow()
                .view_is_dragging_for_test(key),
            "fixture: a local drag is live"
        );
        let record = states[&fid]
            .accepted_gesture()
            .copied()
            .expect("fixture: the gesture is armed");

        // The panel goes away. `panel_grid_size` is now `None`.
        editor.hide_panel_for_test(fid);
        assert!(
            editor.core.borrow().panel_grid_size(fid).is_none(),
            "fixture: the panel is hidden, so ambient geometry is gone"
        );

        editor.complete_panel_gesture(fid, &record, none);

        assert!(
            !editor
                .terminal_manager
                .borrow()
                .view_is_dragging_for_test(key),
            "the completion must still finish the drag --- it replays \
             against the viewport RECORDED at the press, because the \
             ambient one is exactly what a cancellation removes"
        );
    }

    /// P2, effect half — a REFUSED press reaches no target at all.
    ///
    /// **The FOCUS assertion is witnessed.** Removing the buffer check
    /// makes this press `Accepted`; it then activates the panel before
    /// replaying, and the focus assertion fails. An earlier version
    /// asserted the refusal BEFORE dispatch, which aborted the row
    /// first and made every effect assertion unreachable — a limit of
    /// ordering that I mistook for a limit of the type boundary. The
    /// classification is now checked LAST, so it still catches a row
    /// that has stopped testing a refusal.
    ///
    /// The controller and byte assertions remain **defence in depth**:
    /// the mutation that reaches them routes through a document buffer,
    /// which touches neither.
    ///
    /// §5b's four `g5_substrate_a_refused_*` rows read the latch and the
    /// cancellation count; none of them reads the target. A refusal that
    /// armed nothing while still sending the child a press, or starting
    /// a local drag, would pass every one of them.
    #[test]
    fn r4_p2_a_refused_press_reaches_no_target() {
        let fid = FrontendId(801);
        let (mut editor, mut states, mut render, panel, buffer_id, epochs) =
            terminal_panel_session(fid, true);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();
        // THE REFUSAL LEVER IS A WRONG BUFFER, at an IN-CONTENT cell.
        //
        // An out-of-grid cell cannot serve here: with the row bound
        // removed it becomes `on_chrome`, so the press classifies
        // `Consumed` and still reaches no target. The row would "bite"
        // on its own precondition while never exercising focus at all.
        // A foreign buffer at a content cell is the refusal whose
        // mis-gating yields `Accepted`, which is the misclassification
        // these assertions are written against.
        let foreign_buffer = editor
            .core
            .borrow()
            .registry
            .borrow_mut()
            .create("*not-the-panel*");
        let in_content = pmacs_protocol::CellCoord::new(1, 2);
        let key = crate::terminal::view::TerminalViewKey::new(fid, panel, buffer_id);
        // FOCUS is `views[&fid].active`, not `active_frontend`: the
        // latter says which frontend is current, and `focus_window`
        // moves the former. An earlier version of this row recorded the
        // wrong one and would have watched a panel steal focus without
        // noticing.
        let focused_before = editor.core.borrow().views[&fid].active;
        assert_ne!(
            focused_before, panel,
            "fixture: the panel must start PASSIVE, or 'focus did not \
             move to the panel' asserts nothing"
        );
        let controller_before = editor
            .terminal_manager
            .borrow()
            .controller_view_for_frontend(fid);
        // CAPTURED, not asserted yet. Asserting the refusal here aborted
        // the row before dispatch, so the effect assertions below could
        // never fail under the one mutation that reaches them. The
        // precondition still runs --- at the END --- so the row cannot go
        // vacuous either.
        let observed_outcome = editor
            .classify_panel_pointer(fid, foreign_buffer, in_content, press)
            .outcome();

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            foreign_buffer,
            in_content,
            press,
            none,
        );

        assert!(
            child_stream(&editor).is_empty(),
            "a refused press must send the child NOTHING --- a press it \
             receives is one it will expect a release for"
        );
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            focused_before,
            "and it must not FOCUS the panel: an accepted press \
             activates BEFORE it replays, so a misclassified one moves \
             focus to the panel whatever its replay then does with a \
             buffer that is not the one on screen"
        );
        assert_eq!(
            editor
                .terminal_manager
                .borrow()
                .controller_view_for_frontend(fid),
            controller_before,
            "and it must not claim the terminal CONTROLLER"
        );
        assert_eq!(
            observed_outcome,
            crate::editor::PanelPointerOutcome::Refused,
            "and the press really was refused --- checked LAST so that a \
             mutation which accepts it is caught by the effects above \
             rather than aborting the row here, while still failing if \
             this row ever stops testing a refusal at all"
        );
        assert!(
            !editor
                .terminal_manager
                .borrow()
                .view_is_dragging_for_test(key),
            "and it must not begin a local drag either"
        );
        assert!(!states[&fid].has_accepted_gesture(), "and it must not arm");
    }

    /// P5 — an accepted in-content release delivers EXACTLY ONE child
    /// release, never the ordinary one plus a record-driven one.
    #[test]
    fn r4_p5_an_accepted_release_reaches_the_child_exactly_once() {
        let fid = FrontendId(799);
        let (mut editor, mut states, mut render, _panel, buffer_id, epochs) =
            terminal_panel_session(fid, true);
        let cell = pmacs_protocol::CellCoord::new(1, 2);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let release = pmacs_protocol::MouseKind::Up(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            press,
            none,
        );
        assert_eq!(
            child_stream(&editor),
            vec![SGR_PRESS_1_2.to_vec()],
            "fixture: the press reached the child, in SGR"
        );

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            release,
            none,
        );

        assert_eq!(
            child_stream(&editor),
            vec![SGR_RELEASE_1_2.to_vec()],
            "EXACTLY ONE release, and its exact bytes: the in-content path \
             already completed the gesture, so running the recorded \
             completion as well sends the child two Ups for one Down"
        );
    }

    /// P4 — a REFUSED release performs no completion and retains the
    /// record, so a later authoritative cancellation can still end it.
    #[test]
    fn r4_p4_a_refused_release_neither_completes_nor_takes_the_record() {
        let fid = FrontendId(800);
        let (mut editor, mut states, mut render, _panel, buffer_id, epochs) =
            terminal_panel_session(fid, true);
        let cell = pmacs_protocol::CellCoord::new(1, 2);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let release = pmacs_protocol::MouseKind::Up(pmacs_protocol::MouseButton::Left);
        let none = pmacs_protocol::Modifiers::default();
        let outside = {
            let core = editor.core.borrow();
            let grid = core.panel_grid_size(fid).expect("grid");
            pmacs_protocol::CellCoord::new(grid.rows, 0)
        };

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            cell,
            press,
            none,
        );
        assert_eq!(
            child_stream(&editor),
            vec![SGR_PRESS_1_2.to_vec()],
            "fixture: the press reached the child, in SGR"
        );

        send_panel(
            &mut editor,
            &mut states,
            &mut render,
            fid,
            epochs,
            buffer_id,
            outside,
            release,
            none,
        );

        assert!(
            child_stream(&editor).is_empty(),
            "a refused release must deliver NO completion --- the daemon \
             cannot tell it concerns this gesture at all"
        );
        assert!(
            states[&fid].has_accepted_gesture(),
            "and it must RETAIN the record, so an authoritative \
             cancellation can still end the gesture properly"
        );
    }

    /// The panel's buffer, and a coordinate one row past its grid.
    fn panel_buffer_and_outside_coord(
        editor: &crate::editor::EditorState,
        fid: FrontendId,
        panel: crate::window::WindowId,
    ) -> (crate::buffer::BufferId, pmacs_protocol::CellCoord) {
        let core = editor.core.borrow();
        let grid = core.panel_grid_size(fid).expect("a live panel grid");
        (
            core.windows[&panel].buffer_id,
            pmacs_protocol::CellCoord::new(grid.rows, 0),
        )
    }

    /// A press the dispatcher refuses must leave the latch exactly as it
    /// found it — neither armed, nor displaced.
    fn a_refused_press_never_arms(arm: PanelArm, fid: FrontendId) {
        let (mut editor, mut states, mut render, _document, panel, epochs) =
            panel_session_at(arm.version(), fid);
        let (buffer_id, outside) = panel_buffer_and_outside_coord(&editor, fid, panel);
        let inside = pmacs_protocol::CellCoord::new(0, 0);
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let mut send =
            |editor: &mut crate::editor::EditorState,
             states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
             coord| {
                let generation = live_generation(arm, editor, states, fid);
                let event = arm_pointer(arm, fid, epochs, buffer_id, generation, coord, press);
                dispatch_panel_event(editor, fid, arm.version(), states, &mut render, event);
            };

        send(&mut editor, &mut states, outside);
        assert!(
            !states[&fid].has_accepted_gesture(),
            "a refused press must not arm: an authority loss would then \
             count a cancellation for a gesture that never began, and \
             replay would release a child that was never pressed"
        );
        assert_eq!(
            states[&fid].panel_gesture_cancellations(),
            0,
            "and it must not count one on the way in either"
        );

        send(&mut editor, &mut states, inside);
        assert!(
            states[&fid].has_accepted_gesture(),
            "control: the same event one row up clears the ladder, so the \
             refusal above came from the DISPATCHER and not from a rung"
        );

        // `arm_accepted_gesture` ends whatever it overwrites, so an
        // unconditional update also destroys a live gesture.
        send(&mut editor, &mut states, outside);
        assert!(
            states[&fid].has_accepted_gesture(),
            "the live gesture survives a refused press"
        );
        assert_eq!(
            states[&fid].panel_gesture_cancellations(),
            0,
            "and the refused press ends nothing"
        );
    }

    /// A release the dispatcher refuses must not consume a live gesture.
    fn a_refused_release_never_consumes(arm: PanelArm, fid: FrontendId) {
        let (mut editor, mut states, mut render, _document, panel, epochs) =
            panel_session_at(arm.version(), fid);
        let (buffer_id, outside) = panel_buffer_and_outside_coord(&editor, fid, panel);
        let inside = pmacs_protocol::CellCoord::new(0, 0);
        let mut send =
            |editor: &mut crate::editor::EditorState,
             states: &mut HashMap<FrontendId, crate::semantic_render::SemanticRenderState>,
             coord,
             kind| {
                let generation = live_generation(arm, editor, states, fid);
                let event = arm_pointer(arm, fid, epochs, buffer_id, generation, coord, kind);
                dispatch_panel_event(editor, fid, arm.version(), states, &mut render, event);
            };
        let press = pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left);
        let release = pmacs_protocol::MouseKind::Up(pmacs_protocol::MouseButton::Left);

        send(&mut editor, &mut states, inside, press);
        assert!(
            states[&fid].has_accepted_gesture(),
            "fixture: a real gesture is live"
        );

        send(&mut editor, &mut states, outside, release);
        assert!(
            states[&fid].has_accepted_gesture(),
            "a refused release must not consume the live gesture: the \
             authority loss that should have ended it would find nothing \
             armed, and the child would hold the button down for good"
        );
        assert_eq!(
            states[&fid].panel_gesture_cancellations(),
            0,
            "and a refused release ends nothing"
        );

        send(&mut editor, &mut states, inside, release);
        assert!(
            !states[&fid].has_accepted_gesture(),
            "control: the same release one row up DOES consume it, so the \
             refusal above came from the DISPATCHER and not from a rung"
        );
        assert_eq!(
            states[&fid].panel_gesture_cancellations(),
            0,
            "an ordinary release is a consume, never a cancellation"
        );
    }

    #[test]
    fn g5_substrate_a_refused_press_never_arms_on_the_legacy_arm() {
        a_refused_press_never_arms(PanelArm::Legacy, FrontendId(778));
    }

    #[test]
    fn g5_substrate_a_refused_press_never_arms_on_the_mapped_arm() {
        a_refused_press_never_arms(PanelArm::Mapped, FrontendId(779));
    }

    #[test]
    fn g5_substrate_a_refused_release_never_consumes_on_the_legacy_arm() {
        a_refused_release_never_consumes(PanelArm::Legacy, FrontendId(780));
    }

    #[test]
    fn g5_substrate_a_refused_release_never_consumes_on_the_mapped_arm() {
        a_refused_release_never_consumes(PanelArm::Mapped, FrontendId(781));
    }

    /// Criterion 50: a gesture from a source whose latest declaration is
    /// not a visible `Present` is dropped.
    #[test]
    fn a_panel_pointer_without_a_present_declaration_is_dropped() {
        let mut editor = crate::editor::EditorState::new();
        let fid = FrontendId(707);
        let (document, panel) = semantic_panel_view(&editor, fid, true);
        let panel = panel.expect("panel window");
        let mut semantic_states = HashMap::new();
        semantic_states.insert(
            fid,
            crate::semantic_render::SemanticRenderState::for_peer(fid, PROTOCOL_VERSION),
        );
        // No geometry declared yet, so nothing has been shipped and the
        // seeded baseline is `Absent`.
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;

        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut semantic_states,
            &mut HashMap::new(),
            FrontendEvent::PanelPointer {
                frontend_id: fid,
                geometry_epoch: 1,
                panel_epoch: 1,
                buffer_id,
                coord: pmacs_protocol::CellCoord::new(0, 0),
                kind: pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left),
                mods: pmacs_protocol::Modifiers::default(),
            },
        );

        assert_eq!(
            editor.core.borrow().views[&fid].active,
            document,
            "a gesture with no live Present declaration must not focus the panel"
        );
    }

    /// Criterion 49: the accepted case, and the two epoch races beside
    /// it. Without the accepted arm the drops above would pass for a
    /// dispatcher that ignores the event family entirely.
    #[test]
    fn panel_pointer_epochs_decide_focus() {
        let mut editor = crate::editor::EditorState::new();
        let fid = FrontendId(708);
        let (document, panel) = semantic_panel_view(&editor, fid, true);
        let panel = panel.expect("panel window");
        let mut semantic_states = HashMap::new();
        semantic_states.insert(
            fid,
            crate::semantic_render::SemanticRenderState::for_peer(fid, LEGACY_PANEL_VERSION),
        );
        editor.accept_semantic_frame_geometry(fid, 1, CellSize::new(24, 80));
        let (geometry_epoch, panel_epoch) = shipped_declaration(&editor, fid, &mut semantic_states);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
        let down = |geometry_epoch, panel_epoch| FrontendEvent::PanelPointer {
            frontend_id: fid,
            geometry_epoch,
            panel_epoch,
            buffer_id,
            coord: pmacs_protocol::CellCoord::new(0, 0),
            kind: pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left),
            mods: pmacs_protocol::Modifiers::default(),
        };

        for (label, event) in [
            (
                "stale geometry epoch",
                down(geometry_epoch + 1, panel_epoch),
            ),
            (
                "stale presentation epoch",
                down(geometry_epoch, panel_epoch + 1),
            ),
        ] {
            dispatch_panel_event(
                &mut editor,
                fid,
                LEGACY_PANEL_VERSION,
                &mut semantic_states,
                &mut HashMap::new(),
                event,
            );
            assert_eq!(
                editor.core.borrow().views[&fid].active,
                document,
                "{label}: the gesture must drop before it can move focus"
            );
        }

        dispatch_panel_event(
            &mut editor,
            fid,
            LEGACY_PANEL_VERSION,
            &mut semantic_states,
            &mut HashMap::new(),
            down(geometry_epoch, panel_epoch),
        );
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            panel,
            "a gesture matching BOTH epochs activates the panel (click-to-focus)"
        );
    }

    /// Criterion 49's font/scale/resize race, and the reason Q#BP16 step
    /// 3 compares the payload's geometry epoch against **both** the
    /// shipped declaration and the daemon's latest accepted one.
    ///
    /// A declaration accepted between two renders makes the two diverge:
    /// the frontend is still looking at a frame answering the old epoch,
    /// so a gesture matching that frame passes the session-side check
    /// alone. Only the daemon-side comparison catches it — which is
    /// exactly "an older retained frame neither paints nor accepts input
    /// after a new `geometry_epoch` until a matching `Present` arrives".
    #[test]
    fn a_gesture_answering_a_superseded_geometry_is_dropped() {
        let mut editor = crate::editor::EditorState::new();
        let fid = FrontendId(712);
        let (document, panel) = semantic_panel_view(&editor, fid, true);
        let panel = panel.expect("panel window");
        let mut semantic_states = HashMap::new();
        semantic_states.insert(
            fid,
            crate::semantic_render::SemanticRenderState::for_peer(fid, LEGACY_PANEL_VERSION),
        );
        editor.accept_semantic_frame_geometry(fid, 1, CellSize::new(24, 80));
        let (geometry_epoch, panel_epoch) = shipped_declaration(&editor, fid, &mut semantic_states);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;

        // Q#BP2S1: a font or scale change that leaves `CellSize`
        // IDENTICAL still advances the epoch, and no frame answering it
        // has been painted yet.
        assert_eq!(
            editor.accept_semantic_frame_geometry(fid, 2, CellSize::new(24, 80)),
            crate::editor_core::GeometryUpdate::Advanced
        );
        assert_eq!(
            semantic_states[&fid]
                .panel_declaration()
                .map(|frame| frame.geometry_epoch),
            Some(geometry_epoch),
            "fixture precondition: the SHIPPED declaration still answers the \
             old epoch, so the session-side check alone would accept"
        );

        dispatch_panel_event(
            &mut editor,
            fid,
            LEGACY_PANEL_VERSION,
            &mut semantic_states,
            &mut HashMap::new(),
            FrontendEvent::PanelPointer {
                frontend_id: fid,
                geometry_epoch,
                panel_epoch,
                buffer_id,
                coord: pmacs_protocol::CellCoord::new(0, 0),
                kind: pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left),
                mods: pmacs_protocol::Modifiers::default(),
            },
        );

        assert_eq!(
            editor.core.borrow().views[&fid].active,
            document,
            "a gesture hit-tested against superseded geometry must drop"
        );

        // The accepted counterpart: once a frame answering the new epoch
        // ships, the same gesture shape is honored again.
        let (fresh_geometry, fresh_panel) = shipped_declaration(&editor, fid, &mut semantic_states);
        assert_eq!(fresh_geometry, 2, "the new frame answers the new epoch");
        dispatch_panel_event(
            &mut editor,
            fid,
            LEGACY_PANEL_VERSION,
            &mut semantic_states,
            &mut HashMap::new(),
            FrontendEvent::PanelPointer {
                frontend_id: fid,
                geometry_epoch: fresh_geometry,
                panel_epoch: fresh_panel,
                buffer_id,
                coord: pmacs_protocol::CellCoord::new(0, 0),
                kind: pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left),
                mods: pmacs_protocol::Modifiers::default(),
            },
        );
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            panel,
            "…and a gesture answering the CURRENT geometry is accepted"
        );
    }

    /// Criteria 49/50 for the resize event: matching epochs move the
    /// stored request, a stale presentation epoch does not.
    #[test]
    fn panel_resize_rows_honors_both_epochs() {
        let mut editor = crate::editor::EditorState::new();
        let fid = FrontendId(709);
        let (_document, panel) = semantic_panel_view(&editor, fid, true);
        let panel = panel.expect("panel window");
        let mut semantic_states = HashMap::new();
        semantic_states.insert(
            fid,
            crate::semantic_render::SemanticRenderState::for_peer(fid, PROTOCOL_VERSION),
        );
        editor.accept_semantic_frame_geometry(fid, 1, CellSize::new(24, 80));
        let (geometry_epoch, panel_epoch) = shipped_declaration(&editor, fid, &mut semantic_states);
        let before = editor.core.borrow().windows[&panel].params.fixed_rows;

        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut semantic_states,
            &mut HashMap::new(),
            FrontendEvent::PanelResizeRows {
                frontend_id: fid,
                geometry_epoch,
                panel_epoch: panel_epoch + 1,
                rows: 9,
            },
        );
        assert_eq!(
            editor.core.borrow().windows[&panel].params.fixed_rows,
            before,
            "a stale presentation epoch must not resize the panel"
        );

        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut semantic_states,
            &mut HashMap::new(),
            FrontendEvent::PanelResizeRows {
                frontend_id: fid,
                geometry_epoch,
                panel_epoch,
                rows: 9,
            },
        );
        assert_eq!(
            editor.core.borrow().windows[&panel].params.fixed_rows,
            Some(9),
            "matching epochs move the stored request"
        );
    }

    /// Parent acceptance 40, at the seam it would actually break: a
    /// semantic frontend's `Resize` must not mint frame geometry, even
    /// when its view is panel-capable.
    ///
    /// Stage 1's gate was `panel_capable_for`, which was equivalent to
    /// "grid" only because no semantic session could be panel-capable.
    /// Once one can be, that gate feeds it exactly the placeholder
    /// Q#BP15a forbids — and the failure would show up as a wrongly
    /// sized first panel, far from this line.
    #[test]
    fn a_semantic_resize_does_not_mint_frame_geometry() {
        let mut editor = crate::editor::EditorState::new();
        let fid = FrontendId(710);
        semantic_panel_view(&editor, fid, true);
        let mut semantic_states = HashMap::new();
        semantic_states.insert(
            fid,
            crate::semantic_render::SemanticRenderState::for_peer(fid, PROTOCOL_VERSION),
        );

        dispatch_panel_event(
            &mut editor,
            fid,
            PROTOCOL_VERSION,
            &mut semantic_states,
            &mut HashMap::new(),
            FrontendEvent::Resize {
                frontend_id: fid,
                size: CellSize::new(24, 80),
            },
        );

        assert_eq!(
            editor.core.borrow().frame_geometry_for(fid),
            None,
            "40: only FrontendCellGeometry is a semantic frontend's \
             authoritative cell-equivalent declaration"
        );

        // The discriminating half: a GRID frontend's resize still is its
        // declaration, so the gate is not simply switched off.
        let grid = FrontendId(711);
        let view = build_fresh_frontend_view(&mut editor, true, true);
        editor.core.borrow_mut().register_frontend_view(grid, view);
        let mut render_states = HashMap::new();
        render_states.insert(grid, RenderState::new(CellSize::new(24, 80)));
        dispatch_panel_event(
            &mut editor,
            grid,
            PROTOCOL_VERSION,
            &mut HashMap::new(),
            &mut render_states,
            FrontendEvent::Resize {
                frontend_id: grid,
                size: CellSize::new(30, 90),
            },
        );
        assert_eq!(
            editor
                .core
                .borrow()
                .frame_geometry_for(grid)
                .map(|geometry| geometry.total),
            Some(CellSize::new(30, 90)),
            "a grid frontend's real frame size IS its declaration"
        );
    }

    // -----------------------------------------------------------------
    // Review round 1 — three findings the mutation pass could not reach,
    // because each is a behaviour that was never modelled rather than a
    // line that was written wrong.
    // -----------------------------------------------------------------

    /// R1-2: closing and reopening the SAME persistent buffer inside one
    /// dispatcher burst — before the next render can ship a new
    /// declaration — must not let a stale gesture address the successor.
    ///
    /// Same buffer, same size, same geometry: `buffer_id` and the grid
    /// bounds are identical on both sides, so the only thing that can
    /// tell the two presentations apart is the presentation identity
    /// itself (Q#BP16). Validating the last SHIPPED declaration alone is
    /// not enough — it still describes the dead window.
    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one close/reopen transaction plus both event kinds and the accepted counterpart"
    )]
    fn a_stale_panel_epoch_cannot_address_a_reopened_same_buffer_panel() {
        let mut editor = crate::editor::EditorState::new();
        let fid = FrontendId(713);
        let (document, panel) = semantic_panel_view(&editor, fid, true);
        let first_panel = panel.expect("panel window");
        let mut semantic_states = HashMap::new();
        semantic_states.insert(
            fid,
            crate::semantic_render::SemanticRenderState::for_peer(fid, LEGACY_PANEL_VERSION),
        );
        editor.accept_semantic_frame_geometry(fid, 1, CellSize::new(24, 80));
        let (geometry_epoch, panel_epoch) = shipped_declaration(&editor, fid, &mut semantic_states);
        let buffer_id = editor.core.borrow().windows[&first_panel].buffer_id;

        // Close and reopen the SAME buffer, with no render in between.
        {
            let mut core = editor.core.borrow_mut();
            core.active_frontend = fid;
            core.focus_window(fid, first_panel);
            assert!(
                core.close_active(),
                "closing a side window is legal even as the only other window"
            );
        }
        editor.reconcile_panel_layout(fid);
        {
            let mut core = editor.core.borrow_mut();
            let mut request = crate::editor_core::DisplayRequest::new(buffer_id);
            request.side = Some(crate::window::Side::Bottom);
            request.height = Some(4);
            core.display_buffer(fid, &request)
                .expect("reopen the panel");
        }
        editor.reconcile_panel_layout(fid);
        let second_panel = editor.core.borrow().side_window_for(fid).expect("reopened");
        assert_ne!(
            second_panel, first_panel,
            "fixture precondition: the successor is a different window"
        );
        assert_eq!(
            editor.core.borrow().windows[&second_panel].buffer_id,
            buffer_id,
            "…showing the SAME persistent buffer, which is what makes it \
             indistinguishable by buffer id"
        );
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            document,
            "fixture precondition: focus is on the document after the reopen"
        );

        dispatch_panel_event(
            &mut editor,
            fid,
            LEGACY_PANEL_VERSION,
            &mut semantic_states,
            &mut HashMap::new(),
            FrontendEvent::PanelPointer {
                frontend_id: fid,
                geometry_epoch,
                panel_epoch,
                buffer_id,
                coord: pmacs_protocol::CellCoord::new(0, 0),
                kind: pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left),
                mods: pmacs_protocol::Modifiers::default(),
            },
        );
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            document,
            "R1-2: a gesture aimed at the CLOSED presentation must not \
             activate its successor"
        );

        // …and the resize event follows the same ladder.
        let before = editor.core.borrow().windows[&second_panel]
            .params
            .fixed_rows;
        dispatch_panel_event(
            &mut editor,
            fid,
            LEGACY_PANEL_VERSION,
            &mut semantic_states,
            &mut HashMap::new(),
            FrontendEvent::PanelResizeRows {
                frontend_id: fid,
                geometry_epoch,
                panel_epoch,
                rows: 9,
            },
        );
        assert_eq!(
            editor.core.borrow().windows[&second_panel]
                .params
                .fixed_rows,
            before,
            "R1-2: nor resize it"
        );

        // The accepted counterpart: once a frame describing the successor
        // ships, the same gesture shape is honored.
        let (fresh_geometry, fresh_panel_epoch) =
            shipped_declaration(&editor, fid, &mut semantic_states);
        assert_ne!(
            fresh_panel_epoch, panel_epoch,
            "the successor took a fresh presentation identity"
        );
        dispatch_panel_event(
            &mut editor,
            fid,
            LEGACY_PANEL_VERSION,
            &mut semantic_states,
            &mut HashMap::new(),
            FrontendEvent::PanelPointer {
                frontend_id: fid,
                geometry_epoch: fresh_geometry,
                panel_epoch: fresh_panel_epoch,
                buffer_id,
                coord: pmacs_protocol::CellCoord::new(0, 0),
                kind: pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left),
                mods: pmacs_protocol::Modifiers::default(),
            },
        );
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            second_panel,
            "…and the CURRENT presentation is addressable"
        );
    }

    /// R2-5: Q#BP16 keeps scroll-without-focus for a NON-terminal panel.
    /// Non-`Move` activation is the terminal-specific clause, because the
    /// shared terminal adapter claims the controller for wheel steps too.
    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "the two panel kinds are the discriminating pair and must share one fixture shape"
    )]
    fn a_wheel_step_focuses_a_terminal_panel_but_not_a_document_panel() {
        use crate::terminal::TerminalSpec;

        // --- non-terminal panel: the wheel must NOT focus -------------
        let mut editor = crate::editor::EditorState::new();
        let fid = FrontendId(714);
        let (document, panel) = semantic_panel_view(&editor, fid, true);
        let panel = panel.expect("panel window");
        let mut semantic_states = HashMap::new();
        semantic_states.insert(
            fid,
            crate::semantic_render::SemanticRenderState::for_peer(fid, LEGACY_PANEL_VERSION),
        );
        editor.accept_semantic_frame_geometry(fid, 1, CellSize::new(24, 80));
        let (geometry_epoch, panel_epoch) = shipped_declaration(&editor, fid, &mut semantic_states);
        let buffer_id = editor.core.borrow().windows[&panel].buffer_id;
        let wheel = |buffer_id, geometry_epoch, panel_epoch| FrontendEvent::PanelPointer {
            frontend_id: fid,
            geometry_epoch,
            panel_epoch,
            buffer_id,
            coord: pmacs_protocol::CellCoord::new(0, 0),
            kind: pmacs_protocol::MouseKind::ScrollUp,
            mods: pmacs_protocol::Modifiers::default(),
        };

        dispatch_panel_event(
            &mut editor,
            fid,
            LEGACY_PANEL_VERSION,
            &mut semantic_states,
            &mut HashMap::new(),
            wheel(buffer_id, geometry_epoch, panel_epoch),
        );
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            document,
            "R2-5: wheel motion over a document panel scrolls without focus"
        );

        // …while a press still focuses it, so the assertion above is not
        // "panel pointers do nothing".
        dispatch_panel_event(
            &mut editor,
            fid,
            LEGACY_PANEL_VERSION,
            &mut semantic_states,
            &mut HashMap::new(),
            FrontendEvent::PanelPointer {
                frontend_id: fid,
                geometry_epoch,
                panel_epoch,
                buffer_id,
                coord: pmacs_protocol::CellCoord::new(0, 0),
                kind: pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left),
                mods: pmacs_protocol::Modifiers::default(),
            },
        );
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            panel,
            "click-to-focus is unchanged"
        );

        // --- terminal panel: every non-Move gesture DOES focus --------
        let mut editor = crate::editor::EditorState::new();
        let fid = FrontendId(715);
        let (document, panel) = semantic_panel_view(&editor, fid, true);
        let panel = panel.expect("panel window");
        let mut spec = TerminalSpec::new("/bin/sh");
        spec.args = vec!["-c".into(), "sleep 30".into()];
        spec.rows = 4;
        spec.cols = 20;
        let terminal_buffer = editor
            .terminal_manager
            .borrow_mut()
            .open(
                spec,
                &mut editor.core.borrow_mut(),
                &mut editor.process_supervisor.borrow_mut(),
            )
            .expect("open panel terminal");
        {
            let mut core = editor.core.borrow_mut();
            let text_view = {
                let registry = core.registry.clone();
                let registry = registry.borrow();
                crate::text_view::TextView::new(
                    registry.get(terminal_buffer).expect("terminal buffer"),
                )
            };
            let window = core.windows.get_mut(&panel).expect("panel window");
            window.buffer_id = terminal_buffer;
            window.text_view = text_view;
        }
        let mut semantic_states = HashMap::new();
        semantic_states.insert(
            fid,
            crate::semantic_render::SemanticRenderState::for_peer(fid, LEGACY_PANEL_VERSION),
        );
        editor.accept_semantic_frame_geometry(fid, 1, CellSize::new(24, 80));
        let (geometry_epoch, panel_epoch) = shipped_declaration(&editor, fid, &mut semantic_states);
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            document,
            "fixture precondition: the terminal panel starts passive"
        );

        dispatch_panel_event(
            &mut editor,
            fid,
            LEGACY_PANEL_VERSION,
            &mut semantic_states,
            &mut HashMap::new(),
            wheel(terminal_buffer, geometry_epoch, panel_epoch),
        );
        assert_eq!(
            editor.core.borrow().views[&fid].active,
            panel,
            "R2-5: a TERMINAL panel activates on every non-Move gesture, \
             because the shared adapter claims the controller for wheel \
             steps too"
        );
    }

    /// R1-3: a semantic frontend's PANEL terminal must be resized to the
    /// daemon-derived content grid before the child's output is drained.
    ///
    /// The two terminal-layout syncs are twins and must stay alternatives
    /// (that is why `sync_terminal_layouts_for_tick` exists), but the grid
    /// twin resolves through `controller_view_for_frontend` and therefore
    /// covers a side window for free, while the semantic twin consulted
    /// only the full-document declaration — which a panel terminal
    /// deliberately does not have.
    #[test]
    fn a_semantic_panel_terminal_is_resized_before_the_child_drain() {
        use crate::terminal::{TerminalSpec, view::TerminalViewKey};

        let mut editor = crate::editor::EditorState::new();
        let fid = FrontendId(716);
        let (_document, panel) = semantic_panel_view(&editor, fid, true);
        let panel = panel.expect("panel window");
        let mut spec = TerminalSpec::new("/bin/sh");
        spec.args = vec!["-c".into(), "sleep 30".into()];
        spec.rows = 4;
        spec.cols = 20;
        let terminal_buffer = editor
            .terminal_manager
            .borrow_mut()
            .open(
                spec,
                &mut editor.core.borrow_mut(),
                &mut editor.process_supervisor.borrow_mut(),
            )
            .expect("open panel terminal");
        {
            let mut core = editor.core.borrow_mut();
            let text_view = {
                let registry = core.registry.clone();
                let registry = registry.borrow();
                crate::text_view::TextView::new(
                    registry.get(terminal_buffer).expect("terminal buffer"),
                )
            };
            let window = core.windows.get_mut(&panel).expect("panel window");
            window.buffer_id = terminal_buffer;
            window.text_view = text_view;
        }
        // The panel view owns the child: only the durable controller's
        // declaration reaches the PTY, and the per-tick liveness pass
        // releases a controller whose window is not focused, so the panel
        // has to actually own focus. Registering the view first mirrors
        // what the first projection does.
        let key = TerminalViewKey::new(fid, panel, terminal_buffer);
        editor.core.borrow_mut().focus_window(fid, panel);
        {
            let mut manager = editor.terminal_manager.borrow_mut();
            manager.record_view_size(key, CellSize::new(4, 20));
            assert!(
                manager.claim_controller(key),
                "fixture precondition: the panel view controls the child"
            );
        }

        let mut semantic_states = HashMap::new();
        semantic_states.insert(
            fid,
            crate::semantic_render::SemanticRenderState::for_peer(fid, PROTOCOL_VERSION),
        );
        editor.accept_semantic_frame_geometry(fid, 1, CellSize::new(24, 120));
        assert_eq!(
            editor.core.borrow().panel_grid_size(fid),
            Some(CellSize::new(4, 120)),
            "fixture precondition: the daemon derives a 4x120 band, so its \
             CONTENT grid is 3x120"
        );
        assert_eq!(
            editor
                .terminal_manager
                .borrow()
                .screen_size(terminal_buffer),
            Some(CellSize::new(4, 20)),
            "fixture precondition: the child still has its opening size"
        );

        sync_terminal_layouts_for_tick(
            &mut editor,
            &[fid],
            &HashMap::from([(fid, CellSize::new(24, 120))]),
            &semantic_states,
        );

        assert_eq!(
            editor
                .terminal_manager
                .borrow()
                .screen_size(terminal_buffer),
            Some(CellSize::new(3, 120)),
            "R1-3: the panel terminal adopts the daemon-derived content \
             grid at the tick's layout step, BEFORE tick_processes drains \
             the child"
        );
    }

    /// Q#BP9's write-loop gate, independent of the producer's own flag.
    #[test]
    fn the_panel_frame_write_gate_rejects_v20_independently() {
        let frame = InstanceMessage::PanelFrame(pmacs_protocol::panel::PanelFramePayload::Absent);
        assert!(!peer_accepts_panel_message(PANEL_MIN_VERSION - 1, &frame));
        assert!(peer_accepts_panel_message(PANEL_MIN_VERSION, &frame));
        assert!(
            peer_accepts_panel_message(
                PANEL_MIN_VERSION - 1,
                &InstanceMessage::DispatchIdle { idle: true }
            ),
            "the filter must be scoped to the panel variant"
        );
    }
}
