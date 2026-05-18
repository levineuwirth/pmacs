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
use std::io::ErrorKind;
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
    AttachRequest, FrontendEvent, FrontendId, GoodbyeReason, Hello, InstanceCapabilities,
    InstanceIdentity, InstanceMessage, PROTOCOL_VERSION, SelectionSnapshot,
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
#[allow(clippy::needless_pass_by_value)]
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
    let color_slot =
        match nix::sys::socket::getsockopt(&stream, nix::sys::socket::sockopt::PeerCredentials) {
            Ok(cred) => daemon_state.color_slot_for_uid(cred.uid()),
            Err(_) => {
                // Fallback: use frontend_id-based slot; per-connection
                // stability only (no cross-reconnect within session).
                u8::try_from(frontend_id.0 % (crate::overlay_color::PALETTE_LEN as u64))
                    .unwrap_or(0)
            }
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
    let mut streams: HashMap<FrontendId, UnixStream> = HashMap::new();
    let mut term_sizes: HashMap<FrontendId, CellSize> = HashMap::new();
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
        let attached_fids: Vec<FrontendId> = render_states.keys().copied().collect();

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
                // `BufferSnapshot` for the newly-CRDT-backed buffer
                // to every currently-attached replica so their
                // `BufferMirror`s gain an entry for it. Without
                // this, replicas attached before the upgrade
                // permanently fall back to v0.1 round-trip on that
                // buffer.
                if let Some(upgraded) = ensure_active_buffer_crdt_backed(editor, *fid) {
                    broadcast_buffer_snapshot_to_replicas(
                        editor,
                        upgraded,
                        &session_registry,
                        &mut streams,
                    );
                }
            }
            #[cfg(not(feature = "crdt"))]
            {
                let _ = session_registry.session_state(*fid);
                let _ = ensure_active_buffer_crdt_backed(editor, *fid);
            }

            // T M10.9 — gather other-frontend presences for the
            // overlay paint. Reads `last_broadcast` (updated by the
            // sweep below); other-frontend snapshots lag by at most
            // one tick. Imperceptible at frame-rate cadence.
            let other_presences = session_registry.other_presences_for(*fid);
            let render_state = render_states
                .get_mut(fid)
                .expect("render_state present for attached fid");
            let messages = render_state.render_frame(editor, &other_presences);

            // T M10.6 per-frontend presence sweep. The snapshot is
            // computed from this frontend's view; the sweep then
            // produces broadcasts to OTHER multi-frontend recipients.
            let snapshot = build_presence_snapshot(editor, *fid);
            let broadcasts = session_registry.sweep(&[(*fid, snapshot)]);

            // Write frame messages to this frontend's stream.
            let mut write_failed = false;
            if let Some(stream) = streams.get_mut(fid) {
                for msg in &messages {
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
                term_sizes.remove(fid);
                session_registry.unregister_session(*fid);
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
                    &mut streams,
                    &mut term_sizes,
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
                        &mut streams,
                        &mut term_sizes,
                        &mut session_registry,
                    );
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        editor.tick_async();
        editor.tick_processes();
        editor.tick_lsp();
    }

    Ok(())
}

/// Handle one `DispatcherEvent`. Extracted so the dispatcher loop
/// can both timeout-recv and burst-drain via the same code path.
fn handle_dispatcher_event(
    event: DispatcherEvent,
    editor: &mut EditorState,
    render_states: &mut HashMap<FrontendId, RenderState>,
    streams: &mut HashMap<FrontendId, UnixStream>,
    term_sizes: &mut HashMap<FrontendId, CellSize>,
    session_registry: &mut SessionRegistry,
) {
    match event {
        DispatcherEvent::SessionEstablished {
            frontend_id,
            session_state,
            initial_size,
            mut write_stream,
        } => {
            // Register the frontend's view (M10.8 Day 3: fresh
            // scratch buffer view; future milestones may clone
            // LOCAL's view or take an explicit initial-buffer
            // argument).
            let scratch_view = build_fresh_frontend_view(editor);
            editor
                .core
                .borrow_mut()
                .register_frontend_view(frontend_id, scratch_view);

            // T M10.10: bootstrap the new frontend's `BufferMirror`
            // by sending one `BufferSnapshot` per CRDT-backed buffer.
            // Gated on the negotiated `crdt_replica` capability —
            // v0.1 / non-replica frontends never receive the variant
            // (postcard would hard-error on the unknown variant; see
            // M10.10-FRAMING.md Refinement 3). Ordering: snapshots
            // are sent BEFORE any CellDelta flows (the next per-tick
            // render is the first CellDelta source), so the mirror
            // is initialized before any local-edit path can
            // reference it.
            let crdt_replica = session_state.negotiated_capabilities.crdt_replica;
            if crdt_replica {
                send_buffer_snapshots(editor, &mut write_stream);
            }

            // Register the session in the registry (presence +
            // capability filters).
            session_registry.register_session(frontend_id, session_state);

            // Allocate per-frontend RenderState; force initial
            // full-grid sync so the first frame paints everything.
            let mut render_state = RenderState::new(initial_size);
            render_state.force_full_grid_resync();
            render_states.insert(frontend_id, render_state);
            streams.insert(frontend_id, write_stream);
            term_sizes.insert(frontend_id, initial_size);

            // Stamp active_frontend so the initial render's Lua
            // statusline code sees the right fid.
            editor.core.borrow_mut().active_frontend = frontend_id;
        }
        DispatcherEvent::FrontendEvent { source, event } => {
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
                _ => {
                    let term_size = *term_sizes
                        .get(&source)
                        .expect("term_size present for source");
                    let render_state = render_states
                        .get_mut(&source)
                        .expect("render_state present for source");
                    let mut term_size = term_size;
                    apply_event(editor, event, &mut term_size, render_state);
                    term_sizes.insert(source, term_size);
                }
            }
        }
        DispatcherEvent::SessionDetached { frontend_id } => {
            render_states.remove(&frontend_id);
            streams.remove(&frontend_id);
            term_sizes.remove(&frontend_id);
            session_registry.unregister_session(frontend_id);
            editor
                .core
                .borrow_mut()
                .unregister_frontend_view(frontend_id);
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
        if !buf.is_crdt_backed() {
            if let Err(e) = buf.upgrade_to_crdt(instance_peer_id) {
                eprintln!("pmacs: upgrade_to_crdt for {buffer_id:?} failed: {e:?}");
                continue;
            }
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
#[cfg(feature = "crdt")]
fn broadcast_buffer_snapshot_to_replicas(
    editor: &EditorState,
    buffer_id: crate::buffer::BufferId,
    session_registry: &SessionRegistry,
    streams: &mut HashMap<FrontendId, UnixStream>,
) {
    let snapshot_bytes = {
        let core = editor.core.borrow();
        let registry = core.registry.borrow();
        let Ok(buf) = registry.get(buffer_id) else {
            return;
        };
        let Some(crdt) = buf.crdt_state() else {
            return;
        };
        match crdt.export_snapshot() {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("pmacs: F29 export_snapshot for {buffer_id:?} failed: {e:?}");
                return;
            }
        }
    };
    let msg = InstanceMessage::BufferSnapshot {
        buffer_id,
        crdt_snapshot: snapshot_bytes,
    };
    for (fid, stream) in streams.iter_mut() {
        let is_replica = session_registry
            .session_state(*fid)
            .is_some_and(|s| s.negotiated_capabilities.crdt_replica);
        if !is_replica {
            continue;
        }
        if let Err(e) = write_message(stream, &msg) {
            eprintln!("pmacs: F29 send BufferSnapshot for {buffer_id:?} to {fid:?} failed: {e}");
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
    if let Ok(buf) = registry.get(buffer_id) {
        if buf
            .validate_remote_op_peer_ids(expected_peer_id, &op.bytes)
            .is_err()
        {
            return Err(
                "op.bytes carry CRDT ops attributed to a peer other than the authenticated source",
            );
        }
    }
    Ok(())
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
fn build_fresh_frontend_view(editor: &mut EditorState) -> crate::window::FrontendView {
    use crate::text_view::TextView;
    use crate::window::{FrontendView, Layout, Window, WindowId};
    let mut core = editor.core.borrow_mut();
    // T M10.9 — share LOCAL's buffer (don't create a fresh
    // scratch). M10.8's fresh-scratch behavior made overlays
    // never fire because attaching frontends were in distinct
    // buffers.
    let local_view = core
        .views
        .get(&FrontendId::LOCAL)
        .expect("LOCAL view present");
    let local_active_win_id = local_view.active;
    let buffer_id = core
        .windows
        .get(&local_active_win_id)
        .expect("LOCAL's active window present in core.windows")
        .buffer_id;
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
