//! Attach-mode client: connect to a running pmacs daemon over a Unix
//! socket, negotiate `semantic_render + crdt_replica`, then pump
//! `InstanceMessage` frames onto the winit event loop.
//!
//! Session 3+ of the pmacs-gpu arc — see `docs/pmacs-gpu-design.md`.
//! Scope: handshake + decode the message stream + send a few
//! `FrontendEvent`s back (currently just `Viewport`; session 5+ adds
//! cursor/edit/focus). Importing the CRDT snapshot, applying live
//! ops, and reconstructing the rope happen on the main thread, where
//! the `LoroDoc` lives (it isn't trivially `Send`; cross-thread
//! shipping is the *decoded* `InstanceMessage`, not the doc state).
//!
//! The reader thread blocks on a single `read_message` per iteration;
//! every received message becomes an [`AttachEvent`] forwarded
//! through [`winit::event_loop::EventLoopProxy::send_event`], which
//! wakes the main loop so the frame logic can apply the message and
//! redraw.

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use pmacs_protocol::{
    AttachRequest, BufferId, ByteRange, CellCoord, CellSize, CrdtOp, FrontendCapabilities,
    FrontendEvent, FrontendId, Hello, InitialTarget, InitialTargetResult, InstanceMessage, Key,
    KeyEvent, Modifiers, MouseKind, PROTOCOL_VERSION, PointerKind, SUPPORTED_PROTOCOL_VERSIONS,
    SessionBootstrapRequest, TransportError, is_supported_protocol_version, read_message,
    write_message,
};
use winit::event_loop::EventLoopProxy;

use crate::AppEvent;

/// Private root-broker target operands, kept as exact Unix paths until the
/// protocol-v20 bootstrap frame is serialized.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitialTargetPaths {
    /// Absolute launcher working directory.
    pub cwd: PathBuf,
    /// Launcher-expanded target path.
    pub path: PathBuf,
}

/// Errors the attach client surfaces. Kept narrow on purpose: the
/// hello-world fallback is the right recovery for any of these in
/// session 3, so the caller's only job is to log + drop back to the
/// inert renderer.
#[derive(Debug)]
pub enum AttachClientError {
    /// Couldn't open the Unix socket.
    Connect(std::io::Error),
    /// Transport framing failed during the handshake.
    Handshake(TransportError),
    /// Server's `protocol_version` is outside `SUPPORTED_PROTOCOL_VERSIONS`.
    VersionMismatch { server: u32, client: u32 },
    /// The daemon's `Hello.instance_capabilities` doesn't advertise a
    /// capability a semantic frontend needs (audit F-003). Most commonly
    /// the daemon was built without `--features crdt`, so `crdt_replica`
    /// / `semantic_render` are `false`: negotiation would "succeed" but no
    /// `BufferSnapshot` ever arrives and the window sits on
    /// `(connecting...)` forever. We reject up front instead.
    CapabilityMismatch { missing: Vec<&'static str> },
    /// The requested target requires the protocol-v20 bootstrap envelope.
    InitialTargetUnsupported { server: u32 },
    /// The daemon rejected the target before a window was created.
    InitialTargetFailed { path: PathBuf, message: String },
    /// The daemon violated the target bootstrap ordering contract.
    InitialTargetProtocol(String),
}

impl AttachClientError {
    /// A short, actionable line to render *in the window* (the user may
    /// never see stderr). The `Display` impl carries the full detail for
    /// logs; this is the one-liner for the failed-attach placeholder.
    pub fn window_status(&self) -> String {
        match self {
            Self::CapabilityMismatch { .. } => {
                "daemon lacks CRDT support — restart it built with `--features crdt`".to_owned()
            }
            _ => "(attach failed; see stderr)".to_owned(),
        }
    }
}

impl std::fmt::Display for AttachClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(e) => write!(f, "connect to daemon socket failed: {e}"),
            Self::Handshake(e) => write!(f, "attach handshake failed: {e}"),
            Self::VersionMismatch { server, client } => write!(
                f,
                "daemon protocol version {server} not in client's supported set (this client = \
                 {client}, supports {SUPPORTED_PROTOCOL_VERSIONS:?})"
            ),
            Self::CapabilityMismatch { missing } => write!(
                f,
                "daemon does not advertise required capabilities {missing:?} — start the daemon \
                 built with the `crdt` feature (it advertises `crdt_replica` / `semantic_render` \
                 only on CRDT builds; without them no BufferSnapshot is ever sent)"
            ),
            Self::InitialTargetUnsupported { server } => write!(
                f,
                "initial target requires daemon protocol v20, but the live daemon speaks v{server}"
            ),
            Self::InitialTargetFailed { path, message } => {
                write!(
                    f,
                    "could not open initial target {}: {message}",
                    path.display()
                )
            }
            Self::InitialTargetProtocol(message) => {
                write!(f, "invalid initial-target bootstrap: {message}")
            }
        }
    }
}

impl std::error::Error for AttachClientError {}

/// Failure while connecting the managed GPU path.
#[derive(Debug)]
pub enum ManagedAttachError {
    /// The daemon connection reached the normal attach client and failed.
    Attach(AttachClientError),
    /// A refused socket path exists but is not a Unix socket.
    NonSocketPath(PathBuf),
    /// Inspecting a refused socket path failed.
    InspectSocket {
        /// Path whose entry type could not be inspected.
        path: PathBuf,
        /// Filesystem error from `metadata`.
        source: io::Error,
    },
    /// The requested daemon executable could not be started.
    SpawnDaemon {
        /// Executable supplied by the root broker.
        executable: PathBuf,
        /// Process-spawn failure.
        source: io::Error,
    },
    /// No attachable daemon appeared before the bounded deadline.
    StartupTimeout {
        /// Socket path that remained unreachable.
        socket: PathBuf,
        /// Most recent connect error.
        connect: io::Error,
        /// Observed daemon process outcome, when it exited early.
        daemon_status: Option<String>,
        /// Startup deadline used for this attempt.
        timeout: Duration,
    },
}

impl std::fmt::Display for ManagedAttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Attach(error) => error.fmt(f),
            Self::NonSocketPath(path) => write!(
                f,
                "refusing to start a daemon: socket path {} exists and is not a Unix socket",
                path.display()
            ),
            Self::InspectSocket { path, source } => write!(
                f,
                "cannot inspect refused socket path {}: {source}",
                path.display()
            ),
            Self::SpawnDaemon { executable, source } => write!(
                f,
                "could not start daemon executable {}: {source}",
                executable.display()
            ),
            Self::StartupTimeout {
                socket,
                connect,
                daemon_status,
                timeout,
            } => {
                write!(
                    f,
                    "daemon did not become attachable on {} within {timeout:?}: {connect}",
                    socket.display()
                )?;
                if let Some(status) = daemon_status {
                    write!(f, " (spawned daemon {status})")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ManagedAttachError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Attach(error) => Some(error),
            Self::InspectSocket { source, .. }
            | Self::SpawnDaemon { source, .. }
            | Self::StartupTimeout {
                connect: source, ..
            } => Some(source),
            Self::NonSocketPath(_) => None,
        }
    }
}

impl From<AttachClientError> for ManagedAttachError {
    fn from(error: AttachClientError) -> Self {
        Self::Attach(error)
    }
}

#[derive(Debug, Default)]
struct DaemonProcessState {
    reaped: bool,
    wait_result: Option<String>,
}

/// Observable process facts for a daemon started by managed attach.
#[derive(Clone, Debug)]
pub struct ManagedDaemonFacts {
    spawned: bool,
    pid: Option<u32>,
    state: Arc<Mutex<DaemonProcessState>>,
}

impl ManagedDaemonFacts {
    fn existing() -> Self {
        Self {
            spawned: false,
            pid: None,
            state: Arc::new(Mutex::new(DaemonProcessState::default())),
        }
    }

    fn spawned(pid: u32) -> Self {
        Self {
            spawned: true,
            pid: Some(pid),
            state: Arc::new(Mutex::new(DaemonProcessState::default())),
        }
    }

    fn record_wait(&self, result: String) {
        let mut state = self.state.lock().expect("managed daemon state lock");
        state.reaped = true;
        state.wait_result = Some(result);
    }

    /// Whether this invocation started a daemon process.
    pub fn spawned_daemon(&self) -> bool {
        self.spawned
    }

    /// Process ID of the daemon this invocation started.
    pub fn daemon_pid(&self) -> Option<u32> {
        self.pid
    }

    /// Whether the started child has been observed with `wait`.
    pub fn daemon_reaped(&self) -> bool {
        self.state.lock().expect("managed daemon state lock").reaped
    }

    /// Recorded `wait` result for a completed child.
    pub fn daemon_wait_result(&self) -> Option<String> {
        self.state
            .lock()
            .expect("managed daemon state lock")
            .wait_result
            .clone()
    }
}

/// A successful attach plus lifecycle facts for any daemon it started.
pub struct ManagedAttach {
    /// Connected semantic attach client.
    pub client: AttachClient,
    /// Shared facts updated by the daemon child reaper.
    pub daemon: ManagedDaemonFacts,
}

/// The capabilities a semantic `pmacs-gpu` frontend requires the daemon to
/// advertise in `Hello.instance_capabilities`, and which of them this
/// daemon is missing (audit F-003). Empty ⇒ the attach can proceed.
fn missing_capabilities(caps: &pmacs_protocol::InstanceCapabilities) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !caps.multi_frontend {
        missing.push("multi_frontend");
    }
    if !caps.crdt_replica {
        missing.push("crdt_replica");
    }
    if !caps.semantic_render {
        missing.push("semantic_render");
    }
    missing
}

/// One decoded event forwarded from the reader thread to the main
/// loop. `Message` carries the entire `InstanceMessage`; `Disconnected`
/// fires once when the reader thread exits (clean EOF or transport
/// error — both look identical from the main thread's perspective).
#[derive(Debug)]
pub enum AttachEvent {
    /// A decoded message frame from the daemon.
    Message(Box<InstanceMessage>),
    /// The reader thread exited. Includes the disconnect reason for
    /// logging on the main thread.
    Disconnected(String),
}

/// Hard cap on queued **lossless** outbound events (audit F-008). A
/// daemon so stalled that this many keys / CRDT ops / pastes pile up is
/// effectively dead; past it we fail fast (see [`Outbox::enqueue`]) rather
/// than grow memory without bound. Coalesceable events (viewport / drag)
/// never count against it — they collapse into the queue tail. 8192 is far
/// above any human-paced burst, so it only trips on a genuine stall.
const OUTBOX_MAX: usize = 8192;

/// The kind tag of the two high-frequency events coalesced by
/// tail-replacement (F-008): scroll `Viewport`s and `Pointer` **drags**.
/// `None` marks a *lossless* event — `Key`, `CrdtOp`, `Paste`,
/// `MenuPointer`, and the discrete pointer clicks (`Down` / `Up` /
/// `DoubleDown` / `TripleDown` / `Context`) — which are ordered and never
/// dropped. Coalescing a drag is safe only against a *same-kind tail*, so
/// a `Down, Drag, Drag, Up` gesture keeps its `Down`/`Up` ordering.
fn coalesce_kind(event: &FrontendEvent) -> Option<u8> {
    match event {
        FrontendEvent::Viewport { .. } => Some(0),
        FrontendEvent::Pointer {
            kind: PointerKind::Drag,
            ..
        } => Some(1),
        // Vterm Stage 3 — terminal pointer MOVE and DRAG are the
        // high-frequency terminal gestures and coalesce like their
        // document twin. Press, release, and wheel stay lossless:
        // collapsing a wheel run would silently lose scroll distance,
        // and collapsing a press/release would break selection.
        FrontendEvent::TerminalPointer {
            kind: MouseKind::Move,
            ..
        } => Some(2),
        FrontendEvent::TerminalPointer {
            kind: MouseKind::Drag(_),
            ..
        } => Some(3),
        _ => None,
    }
}

/// Bounded, coalescing outbound queue drained by the writer thread
/// (audit F-008). Replaces the previous unbounded `mpsc::channel`, which
/// let a stalled daemon grow memory without limit and replay a backlog of
/// stale viewport/pointer traffic on recovery.
struct Outbox {
    queue: VecDeque<FrontendEvent>,
    /// Set once the writer gives up (socket error) or a lossless overflow
    /// trips fail-fast. Further [`Outbox::enqueue`] calls return `false`.
    closed: bool,
}

impl Outbox {
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            closed: false,
        }
    }

    /// Apply the F-008 enqueue policy. Returns `false` when the event
    /// can't be accepted — the outbox is `closed`, or a lossless append
    /// would exceed [`OUTBOX_MAX`] (which also *sets* `closed`: a clean
    /// disconnect → reconnect → fresh snapshot is more correct than
    /// silently dropping a `CrdtOp` and desyncing the optimistic replica).
    ///
    /// A coalesceable event (viewport / drag) whose kind matches the queue
    /// **tail** *replaces* it — collapsing a scroll or drag flood to O(1)
    /// without reordering across an intervening click or key.
    fn enqueue(&mut self, event: FrontendEvent) -> bool {
        if self.closed {
            return false;
        }
        if let Some(kind) = coalesce_kind(&event)
            && self.queue.back().and_then(coalesce_kind) == Some(kind)
        {
            *self
                .queue
                .back_mut()
                .expect("tail present when back() matched") = event;
            return true;
        }
        if self.queue.len() >= OUTBOX_MAX {
            self.closed = true;
            return false;
        }
        self.queue.push_back(event);
        true
    }
}

/// Connect, handshake, and spawn the reader thread.
///
/// Returns once the handshake has completed and the reader thread is
/// running. The reader thread owns the read half of the stream; a
/// writer thread owns the write half. The returned [`AttachClient`]
/// queues outbound `FrontendEvent`s so the winit UI thread never blocks
/// on daemon socket backpressure.
///
/// **Initial window size note** — `AttachRequest::initial_size` is
/// nominally a `CellSize` (rows × cols) anchored to the TUI. The
/// `pmacs-gpu` window isn't a cell grid; we send a placeholder of the
/// approximate cell count for the initial 800×200 window so the
/// daemon's initial render makes plausible space for content. This
/// is a small finding for session 3's audit (the wire-shape detail
/// "`AttachRequest`'s `CellSize` assumes a grid frontend"); resolution
/// classified under rule (iii) as deferred — a structural answer
/// belongs with Q#2's minimap variant or its own protocol thread,
/// not session 3's attach loop.
fn read_initial_target_bootstrap(
    stream: &mut UnixStream,
    display_path: PathBuf,
) -> Result<InstanceMessage, AttachClientError> {
    let mut snapshot = None;
    loop {
        let message: InstanceMessage =
            read_message(stream).map_err(AttachClientError::Handshake)?;
        match message {
            candidate @ InstanceMessage::BufferSnapshot { .. } if snapshot.is_none() => {
                snapshot = Some(candidate);
            }
            InstanceMessage::BufferSnapshot { .. } => {
                return Err(AttachClientError::InitialTargetProtocol(
                    "received more than one target snapshot".to_owned(),
                ));
            }
            InstanceMessage::InitialTargetResult(InitialTargetResult::Opened { buffer_id }) => {
                let Some(snapshot) = snapshot else {
                    return Err(AttachClientError::InitialTargetProtocol(
                        "Opened arrived before BufferSnapshot".to_owned(),
                    ));
                };
                let InstanceMessage::BufferSnapshot {
                    buffer_id: snapshot_buffer,
                    ..
                } = snapshot
                else {
                    unreachable!("bootstrap snapshot variant checked above");
                };
                if snapshot_buffer != buffer_id {
                    return Err(AttachClientError::InitialTargetProtocol(format!(
                        "Opened named {buffer_id:?}, snapshot named {snapshot_buffer:?}"
                    )));
                }
                return Ok(snapshot);
            }
            InstanceMessage::InitialTargetResult(InitialTargetResult::Failed { message }) => {
                return Err(AttachClientError::InitialTargetFailed {
                    path: display_path,
                    message,
                });
            }
            InstanceMessage::Goodbye(reason) => {
                return Err(AttachClientError::InitialTargetProtocol(format!(
                    "daemon closed bootstrap: {reason:?}"
                )));
            }
            other => {
                return Err(AttachClientError::InitialTargetProtocol(format!(
                    "unexpected {} before target readiness",
                    crate::instance_message_label(&other)
                )));
            }
        }
    }
}

pub fn connect(
    socket_path: &Path,
    proxy: EventLoopProxy<AppEvent>,
) -> Result<AttachClient, AttachClientError> {
    connect_with_sink(socket_path, move |event| {
        proxy.send_event(AppEvent::Attach(event)).is_ok()
    })
}

/// [`connect`], with the decoded-message destination left to the caller.
///
/// The winit path forwards to the event loop; the headless probe used by
/// the Stage 3 acceptance forwards to a channel. Both drive the SAME
/// handshake, capability gate, reader, writer, and outbox — a probe that
/// reimplemented any of that would prove nothing about the real client.
///
/// `sink` returns `false` when its destination is gone, which stops the
/// reader thread.
pub fn connect_with_sink(
    socket_path: &Path,
    sink: impl Fn(AttachEvent) -> bool + Send + 'static,
) -> Result<AttachClient, AttachClientError> {
    let stream = UnixStream::connect(socket_path).map_err(AttachClientError::Connect)?;
    connect_stream_with_sink(stream, None, sink)
}

#[allow(
    clippy::too_many_lines,
    reason = "the synchronous handshake and thread startup remain one ordered transport transaction"
)]
fn connect_stream_with_sink(
    stream: UnixStream,
    initial_target: Option<InitialTargetPaths>,
    sink: impl Fn(AttachEvent) -> bool + Send + 'static,
) -> Result<AttachClient, AttachClientError> {
    // Hello round-trip.
    let mut handshake_stream = stream.try_clone().map_err(AttachClientError::Connect)?;
    let hello: Hello = read_message(&mut handshake_stream).map_err(AttachClientError::Handshake)?;
    if !is_supported_protocol_version(hello.protocol_version) {
        return Err(AttachClientError::VersionMismatch {
            server: hello.protocol_version,
            client: PROTOCOL_VERSION,
        });
    }
    eprintln!(
        "pmacs-gpu: attached to daemon (protocol v{}, instance pmacs {})",
        hello.protocol_version, hello.instance_identity.pmacs_version
    );

    // Capability gate (audit F-003). The daemon advertises what it can do
    // in `Hello.instance_capabilities`; a semantic frontend needs
    // `multi_frontend` + `crdt_replica` + `semantic_render`. A daemon
    // built without `--features crdt` advertises those as `false`: the
    // handshake would otherwise "succeed", but no `BufferSnapshot` ever
    // arrives and the window sits on `(connecting...)` forever. Reject up
    // front with an actionable error rather than hanging silently. No
    // `AttachResponse` round-trip is needed — the daemon already told us
    // in `Hello`; we just have to check it against what we require.
    let missing = missing_capabilities(&hello.instance_capabilities);
    if !missing.is_empty() {
        return Err(AttachClientError::CapabilityMismatch { missing });
    }

    if initial_target.is_some() && hello.protocol_version < 20 {
        return Err(AttachClientError::InitialTargetUnsupported {
            server: hello.protocol_version,
        });
    }

    // AttachRequest — declare the capabilities a semantic frontend
    // needs. `multi_frontend` is included because the existing daemon
    // gates `crdt_replica` behind it (M10.x dependency).
    let req = AttachRequest {
        protocol_version: hello.protocol_version,
        frontend_capabilities: FrontendCapabilities {
            synchronized_output: false,
            unicode_smp: true,
            true_color: true,
            mouse: false,
            bracketed_paste: false,
            terminal_kind: Some("pmacs-gpu".to_owned()),
            multi_frontend: true,
            crdt_replica: true,
            semantic_render: true,
        },
        // Placeholder — see the doc comment above. Cell-shaped initial
        // size is awkward for a pixel frontend; for session 3 we send
        // approximate dimensions so the daemon's initial-render
        // ranging is plausible.
        initial_size: pmacs_protocol::CellSize::new(24, 80),
    };
    write_message(&mut handshake_stream, &req).map_err(AttachClientError::Handshake)?;

    let target_display_path = initial_target.as_ref().map(|target| target.path.clone());
    if hello.protocol_version >= 20 {
        let bootstrap = SessionBootstrapRequest {
            initial_target: initial_target.map(|target| InitialTarget {
                cwd: target.cwd.as_os_str().as_bytes().to_vec(),
                path: target.path.as_os_str().as_bytes().to_vec(),
            }),
        };
        write_message(&mut handshake_stream, &bootstrap).map_err(AttachClientError::Handshake)?;
    }
    let initial_message = match target_display_path {
        Some(path) => Some(read_initial_target_bootstrap(&mut handshake_stream, path)?),
        None => None,
    };

    // Split read/write halves for the reader thread + writer thread.
    // UnixStream clones share the underlying FD with independent
    // buffer state — safe to read on one clone while the other writes
    // (the FD is full-duplex).
    let mut read_stream = stream.try_clone().map_err(AttachClientError::Connect)?;
    // A third clone kept solely to *shut the socket down* (F-008). When the
    // outbox closes — a lossless overflow, or a writer write error — we
    // `shutdown(Both)` so the reader (blocked in `read_message` on its own
    // clone) wakes with EOF and fires `Disconnected`. That routes the
    // stall into the existing visible teardown instead of leaving the GPU
    // showing locally-applied edits the daemon never received. Clones share
    // the socket's file description, so a shutdown on any of them affects
    // all — and it half-closes toward the daemon so it sees the departure.
    let shutdown_handle = stream.try_clone().map_err(AttachClientError::Connect)?;
    let write_stream = stream;
    // Bounded, coalescing outbound queue (F-008) shared with the writer
    // thread; the `Condvar` wakes the writer when the UI thread enqueues.
    let outbox = Arc::new((Mutex::new(Outbox::new()), Condvar::new()));

    // Reader thread. Each iteration: block on read_message, decode,
    // forward via the event-loop proxy. Exits cleanly on EOF / any
    // transport error; the main thread receives a single Disconnected
    // event and drops back to the inert renderer.
    thread::Builder::new()
        .name("pmacs-gpu attach reader".into())
        .spawn(move || {
            loop {
                match read_message::<InstanceMessage>(&mut read_stream) {
                    Ok(msg) => {
                        if !sink(AttachEvent::Message(Box::new(msg))) {
                            // Destination torn down — quietly exit.
                            return;
                        }
                    }
                    Err(e) => {
                        sink(AttachEvent::Disconnected(e.to_string()));
                        return;
                    }
                }
            }
        })
        .expect("spawn attach reader thread");

    // Writer thread. Socket writes can block when the daemon falls
    // behind; doing them here keeps keyboard input, redraws, and
    // message application off that backpressure path. It waits on the
    // outbox condvar, takes the whole pending batch, and *releases the
    // lock before* the blocking writes so the UI thread can keep
    // enqueueing (and coalescing) meanwhile.
    let writer_outbox = Arc::clone(&outbox);
    thread::Builder::new()
        .name("pmacs-gpu attach writer".into())
        .spawn(move || {
            let mut write_stream = write_stream;
            let (lock, cvar) = &*writer_outbox;
            loop {
                let batch = {
                    let mut ob = lock.lock().expect("outbox lock");
                    while ob.queue.is_empty() && !ob.closed {
                        ob = cvar.wait(ob).expect("outbox condvar wait");
                    }
                    if ob.queue.is_empty() && ob.closed {
                        return;
                    }
                    std::mem::take(&mut ob.queue)
                };
                for event in batch {
                    if let Err(e) = write_message(&mut write_stream, &event) {
                        eprintln!("pmacs-gpu: attach writer stopped: {e}");
                        lock.lock().expect("outbox lock").closed = true;
                        // Wake the reader (and signal the daemon) so the
                        // session tears down visibly (F-008) rather than
                        // leaving the reader blocked on a dead socket.
                        let _ = write_stream.shutdown(std::net::Shutdown::Both);
                        return;
                    }
                }
            }
        })
        .expect("spawn attach writer thread");

    Ok(AttachClient {
        outbox,
        shutdown_handle,
        frontend_id: hello.assigned_frontend_id,
        server_protocol_version: hello.protocol_version,
        initial_message,
    })
}

const MANAGED_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const MANAGED_RETRY_INTERVAL: Duration = Duration::from_millis(50);

/// Managed attach carrying an optional pre-window initial target.
pub fn connect_managed_with_target(
    socket_path: &Path,
    daemon_executable: &Path,
    initial_target: Option<InitialTargetPaths>,
    proxy: EventLoopProxy<AppEvent>,
) -> Result<ManagedAttach, ManagedAttachError> {
    connect_managed_with_target_and_sink(
        socket_path,
        daemon_executable,
        initial_target,
        move |event| proxy.send_event(AppEvent::Attach(event)).is_ok(),
    )
}

/// Managed attach with both a target and caller-provided event sink.
pub fn connect_managed_with_target_and_sink(
    socket_path: &Path,
    daemon_executable: &Path,
    initial_target: Option<InitialTargetPaths>,
    sink: impl Fn(AttachEvent) -> bool + Send + 'static,
) -> Result<ManagedAttach, ManagedAttachError> {
    connect_managed_inner(
        socket_path,
        daemon_executable,
        |path| UnixStream::connect(path),
        spawn_daemon,
        MANAGED_STARTUP_TIMEOUT,
        MANAGED_RETRY_INTERVAL,
        initial_target,
        sink,
    )
}

fn spawn_daemon(daemon_executable: &Path, socket_path: &Path) -> io::Result<Child> {
    let mut command = Command::new(daemon_executable);
    command
        .arg("--daemon")
        .arg("--socket")
        .arg(socket_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.process_group(0);
    command.spawn()
}

fn initial_startup_authorized(
    socket_path: &Path,
    error: &io::Error,
) -> Result<bool, ManagedAttachError> {
    match error.kind() {
        io::ErrorKind::NotFound => Ok(true),
        io::ErrorKind::ConnectionRefused => match fs::metadata(socket_path) {
            Ok(metadata) if metadata.file_type().is_socket() => Ok(true),
            Ok(_) => Err(ManagedAttachError::NonSocketPath(socket_path.to_owned())),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(true),
            Err(source) => Err(ManagedAttachError::InspectSocket {
                path: socket_path.to_owned(),
                source,
            }),
        },
        _ => Ok(false),
    }
}

fn post_spawn_retryable(socket_path: &Path, error: &io::Error) -> Result<bool, ManagedAttachError> {
    match error.kind() {
        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock => Ok(true),
        _ => initial_startup_authorized(socket_path, error),
    }
}

fn start_daemon_reaper(mut child: Child, facts: ManagedDaemonFacts) {
    thread::Builder::new()
        .name("pmacs-gpu daemon reaper".into())
        .spawn(move || {
            let result = match child.wait() {
                Ok(status) => status.to_string(),
                Err(error) => format!("wait failed: {error}"),
            };
            facts.record_wait(result);
        })
        .expect("spawn managed daemon reaper thread");
}

#[allow(
    clippy::too_many_arguments,
    reason = "connector, spawner, timing, and sink stay injectable for deterministic lifecycle tests"
)]
fn connect_managed_inner<C, S, F>(
    socket_path: &Path,
    daemon_executable: &Path,
    mut connector: C,
    spawner: S,
    timeout: Duration,
    retry_interval: Duration,
    initial_target: Option<InitialTargetPaths>,
    sink: F,
) -> Result<ManagedAttach, ManagedAttachError>
where
    C: FnMut(&Path) -> io::Result<UnixStream>,
    S: FnOnce(&Path, &Path) -> io::Result<Child>,
    F: Fn(AttachEvent) -> bool + Send + 'static,
{
    match connector(socket_path) {
        Ok(stream) => {
            let client = connect_stream_with_sink(stream, initial_target, sink)?;
            return Ok(ManagedAttach {
                client,
                daemon: ManagedDaemonFacts::existing(),
            });
        }
        Err(error) => {
            if !initial_startup_authorized(socket_path, &error)? {
                return Err(AttachClientError::Connect(error).into());
            }
        }
    }

    let child = spawner(daemon_executable, socket_path).map_err(|source| {
        ManagedAttachError::SpawnDaemon {
            executable: daemon_executable.to_owned(),
            source,
        }
    })?;
    let daemon = ManagedDaemonFacts::spawned(child.id());
    start_daemon_reaper(child, daemon.clone());
    let deadline = Instant::now() + timeout;

    loop {
        match connector(socket_path) {
            Ok(stream) => {
                let client = connect_stream_with_sink(stream, initial_target, sink)?;
                return Ok(ManagedAttach { client, daemon });
            }
            Err(error) => {
                let retryable = post_spawn_retryable(socket_path, &error)?;
                if !retryable {
                    return Err(AttachClientError::Connect(error).into());
                }
                if Instant::now() >= deadline {
                    let daemon_status = daemon.daemon_wait_result();
                    return Err(ManagedAttachError::StartupTimeout {
                        socket: socket_path.to_owned(),
                        connect: error,
                        daemon_status,
                        timeout,
                    });
                }
                thread::sleep(
                    retry_interval.min(deadline.saturating_duration_since(Instant::now())),
                );
            }
        }
    }
}

/// Handle the main loop keeps after `connect` returns. It queues
/// `FrontendEvent`s for the attach writer thread.
pub struct AttachClient {
    /// Bounded, coalescing outbound queue (F-008) drained by the writer
    /// thread. `send_event` locks it, applies the enqueue policy, and
    /// wakes the writer via the paired `Condvar`.
    outbox: Arc<(Mutex<Outbox>, Condvar)>,
    /// Socket clone used only to `shutdown(Both)` when the outbox closes
    /// (F-008), so the reader wakes and the session tears down visibly
    /// instead of diverging silently. See [`connect`].
    shutdown_handle: UnixStream,
    /// Assigned by the daemon in the `Hello` response. Every
    /// `FrontendEvent` carries this so the daemon can route input back
    /// to the per-session `SemanticRenderState`.
    frontend_id: FrontendId,
    /// The daemon's `Hello.protocol_version`. Wire variants newer
    /// than the daemon (e.g. `Pointer`, v5) must be gated on this —
    /// an older daemon hard-errors decoding an unknown variant.
    server_protocol_version: u32,
    /// Target snapshot retained across the pre-window readiness barrier.
    initial_message: Option<InstanceMessage>,
}

impl AttachClient {
    /// Frontend id assigned by the daemon in the initial `Hello`.
    pub fn frontend_id(&self) -> FrontendId {
        self.frontend_id
    }

    /// Take the target snapshot that must be applied before first redraw.
    pub fn take_initial_message(&mut self) -> Option<InstanceMessage> {
        self.initial_message.take()
    }

    /// Send a `FrontendEvent::Viewport` to the daemon. The daemon's
    /// `SemanticRenderState::set_viewport` feeds the spans producer;
    /// without this call the daemon ships no `StyleSpans` for the
    /// buffer (no declared viewport ⇒ no scoped styling).
    pub fn send_viewport(
        &self,
        buffer_id: BufferId,
        visible: ByteRange,
        generation: u64,
    ) -> Result<(), TransportError> {
        self.send_event(FrontendEvent::Viewport {
            frontend_id: self.frontend_id,
            buffer_id,
            visible,
            generation,
        })
    }

    /// Send a `FrontendEvent::Key` to the daemon (session B1). The
    /// daemon routes it through `dispatch_key` — the same keymap +
    /// command + Lua stack the TUI drives — so cursor motion and (in
    /// later sessions) edits are produced entirely instance-side; the
    /// resulting `CursorByte` / `CrdtOp` come back over the attach
    /// stream. `timestamp_ns` is 0 (no capture clock plumbed yet; the
    /// daemon does not depend on it).
    pub fn send_key(&self, key: Key, mods: Modifiers) -> Result<(), TransportError> {
        self.send_event(FrontendEvent::Key(KeyEvent {
            frontend_id: self.frontend_id,
            key,
            mods,
            timestamp_ns: 0,
        }))
    }

    /// Send a `FrontendEvent::Pointer` (session M-2): a locally
    /// hit-tested gesture in source bytes. Callers gate on
    /// [`Self::server_protocol_version`] `>= 5`.
    pub fn send_pointer(
        &self,
        buffer_id: BufferId,
        byte: u64,
        kind: PointerKind,
        mods: Modifiers,
    ) -> Result<(), TransportError> {
        self.send_event(FrontendEvent::Pointer {
            frontend_id: self.frontend_id,
            buffer_id,
            byte,
            kind,
            mods,
        })
    }

    /// Send a `FrontendEvent::Paste` (Q#CM6) carrying OS-clipboard
    /// bytes read locally via `arboard` on Ctrl-V. The daemon inserts it
    /// at the cursor (replacing any region) and refreshes its clipboard
    /// slot, exactly as it handles the TUI's bracketed paste.
    pub fn send_paste(&self, data: Vec<u8>) -> Result<(), TransportError> {
        self.send_event(FrontendEvent::Paste {
            frontend_id: self.frontend_id,
            data,
        })
    }

    /// Send a `FrontendEvent::TerminalResize` (Vterm Stage 3): the
    /// terminal-cell geometry this frontend has on screen. Callers gate
    /// on [`Self::server_protocol_version`] `>= 19`.
    ///
    /// Cells, never pixels — the frontend divides its own drawable
    /// rectangle by its own metrics, keeping the no-pixels contract the
    /// document `Viewport` established.
    pub fn send_terminal_resize(
        &self,
        buffer_id: BufferId,
        size: CellSize,
    ) -> Result<(), TransportError> {
        self.send_event(FrontendEvent::TerminalResize {
            frontend_id: self.frontend_id,
            buffer_id,
            size,
        })
    }

    /// Send a `FrontendEvent::TerminalPointer` (Vterm Stage 3): a
    /// gesture hit-tested locally to a terminal cell. Callers gate on
    /// [`Self::server_protocol_version`] `>= 19`.
    pub fn send_terminal_pointer(
        &self,
        buffer_id: BufferId,
        coord: CellCoord,
        kind: MouseKind,
        mods: Modifiers,
    ) -> Result<(), TransportError> {
        self.send_event(FrontendEvent::TerminalPointer {
            frontend_id: self.frontend_id,
            buffer_id,
            coord,
            kind,
            mods,
        })
    }

    /// Send a `FrontendEvent::MenuPointer` (Q#CM1) — open-menu
    /// navigation hit-tested locally against the popup we drew. `index`
    /// is the row the pointer is over (`None` = off the menu); `invoke`
    /// marks a click (invoke the row, or dismiss when `index` is `None`).
    pub fn send_menu_pointer(
        &self,
        index: Option<u32>,
        invoke: bool,
    ) -> Result<(), TransportError> {
        self.send_event(FrontendEvent::MenuPointer {
            frontend_id: self.frontend_id,
            index,
            invoke,
        })
    }

    /// The daemon's negotiated wire version from `Hello`.
    pub fn server_protocol_version(&self) -> u32 {
        self.server_protocol_version
    }

    /// Send a locally-authored CRDT operation to the daemon. The GPU
    /// uses this for idle plain-text insertion after applying the same
    /// op to its local Loro replica, avoiding a Key round trip on the
    /// hot typing path.
    pub fn send_crdt_op(&self, buffer_id: BufferId, op: CrdtOp) -> Result<(), TransportError> {
        self.send_event(FrontendEvent::CrdtOp {
            frontend_id: self.frontend_id,
            buffer_id,
            op,
        })
    }

    fn send_event(&self, event: FrontendEvent) -> Result<(), TransportError> {
        let (lock, cvar) = &*self.outbox;
        let mut ob = lock.lock().expect("outbox lock");
        if ob.enqueue(event) {
            // Drop the guard before notifying so the woken writer doesn't
            // immediately re-block on a still-held lock.
            drop(ob);
            cvar.notify_one();
            Ok(())
        } else {
            // Refusing a lossless event prevents replica divergence against
            // a stalled daemon (or an earlier writer failure).
            // Tear the session down actively (F-008): shut the socket so
            // the reader wakes with EOF and fires `Disconnected`, giving a
            // visible "(daemon disconnected)" instead of a GPU that keeps
            // showing optimistic edits the daemon never received. Idempotent
            // — a second shutdown just returns `NotConnected`, ignored.
            drop(ob);
            let _ = self.shutdown_handle.shutdown(std::net::Shutdown::Both);
            Err(TransportError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "attach writer stopped or outbound queue overflowed",
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    use pmacs_protocol::{InstanceCapabilities, InstanceIdentity, MouseButton};

    fn caps(
        multi_frontend: bool,
        crdt_replica: bool,
        semantic_render: bool,
    ) -> InstanceCapabilities {
        InstanceCapabilities {
            multi_frontend,
            crdt_replica,
            semantic_render,
        }
    }

    fn hello(protocol_version: u32) -> Hello {
        Hello {
            protocol_version,
            assigned_frontend_id: FrontendId(7),
            instance_identity: InstanceIdentity {
                pmacs_version: "test".to_owned(),
                build_hash: None,
                instance_name: None,
                uptime_secs: 0,
                working_directory: "/tmp".to_owned(),
            },
            instance_capabilities: caps(true, true, true),
        }
    }

    #[test]
    fn initial_target_bootstrap_is_synchronous_byte_exact_and_snapshot_first() {
        let (client_stream, mut server_stream) = UnixStream::pair().expect("socketpair");
        let raw_path = OsString::from_vec(vec![b'n', b'o', b't', b'e', 0xff]);
        let paths = InitialTargetPaths {
            cwd: PathBuf::from("/launcher"),
            path: PathBuf::from(&raw_path),
        };
        let expected = paths.clone();
        let server = thread::spawn(move || {
            write_message(&mut server_stream, &hello(PROTOCOL_VERSION)).expect("write Hello");
            let _: AttachRequest = read_message(&mut server_stream).expect("read AttachRequest");
            let bootstrap: SessionBootstrapRequest =
                read_message(&mut server_stream).expect("read bootstrap");
            let target = bootstrap.initial_target.expect("initial target");
            assert_eq!(target.cwd, expected.cwd.as_os_str().as_bytes());
            assert_eq!(target.path, expected.path.as_os_str().as_bytes());

            let buffer_id = BufferId::from_raw(41);
            write_message(
                &mut server_stream,
                &InstanceMessage::BufferSnapshot {
                    buffer_id,
                    crdt_snapshot: vec![1, 2, 3],
                },
            )
            .expect("write target snapshot");
            write_message(
                &mut server_stream,
                &InstanceMessage::InitialTargetResult(InitialTargetResult::Opened { buffer_id }),
            )
            .expect("write target result");
        });

        let mut client = connect_stream_with_sink(client_stream, Some(paths), |_| true)
            .expect("target bootstrap");
        assert!(matches!(
            client.take_initial_message(),
            Some(InstanceMessage::BufferSnapshot {
                buffer_id,
                crdt_snapshot,
            }) if buffer_id == BufferId::from_raw(41) && crdt_snapshot == [1, 2, 3]
        ));
        assert!(client.take_initial_message().is_none());
        server.join().expect("bootstrap server");
    }

    #[test]
    fn initial_target_fails_before_attach_on_legacy_protocol() {
        let (client_stream, mut server_stream) = UnixStream::pair().expect("socketpair");
        let server = thread::spawn(move || {
            write_message(&mut server_stream, &hello(19)).expect("write legacy Hello");
        });
        let Err(error) = connect_stream_with_sink(
            client_stream,
            Some(InitialTargetPaths {
                cwd: PathBuf::from("/launcher"),
                path: PathBuf::from("note"),
            }),
            |_| true,
        ) else {
            panic!("legacy target must fail");
        };
        assert!(matches!(
            error,
            AttachClientError::InitialTargetUnsupported { server: 19 }
        ));
        server.join().expect("legacy server");
    }

    #[test]
    fn full_crdt_daemon_advertises_everything_required() {
        // A daemon built with `--features crdt` advertises all three — the
        // attach proceeds (F-003).
        assert!(missing_capabilities(&caps(true, true, true)).is_empty());
    }

    #[test]
    fn non_crdt_daemon_is_rejected_with_the_missing_caps() {
        // The exact silent-hang case: no crdt build ⇒ crdt_replica /
        // semantic_render false. We name them so the error is actionable.
        let missing = missing_capabilities(&caps(true, false, false));
        assert_eq!(missing, vec!["crdt_replica", "semantic_render"]);

        // A single-frontend daemon is also unusable for a GPU attach.
        assert_eq!(
            missing_capabilities(&caps(false, true, true)),
            vec!["multi_frontend"]
        );
    }

    #[test]
    fn capability_mismatch_status_line_is_actionable() {
        // The in-window line (not stderr) must point at the fix.
        let err = AttachClientError::CapabilityMismatch {
            missing: vec!["crdt_replica"],
        };
        let status = err.window_status();
        assert!(
            status.contains("crdt"),
            "status should name the fix: {status}"
        );
        // Other errors keep the generic placeholder.
        let other = AttachClientError::VersionMismatch {
            server: 99,
            client: 6,
        };
        assert_eq!(other.window_status(), "(attach failed; see stderr)");
    }

    // --- F-008: bounded, coalescing outbox --------------------------------

    fn fe_viewport(generation: u64) -> FrontendEvent {
        FrontendEvent::Viewport {
            frontend_id: FrontendId::LOCAL,
            buffer_id: BufferId::from_raw(1),
            visible: ByteRange { start: 0, end: 10 },
            generation,
        }
    }

    fn fe_pointer(kind: PointerKind, byte: u64) -> FrontendEvent {
        FrontendEvent::Pointer {
            frontend_id: FrontendId::LOCAL,
            buffer_id: BufferId::from_raw(1),
            byte,
            kind,
            mods: Modifiers::NONE,
        }
    }

    fn fe_key(c: char) -> FrontendEvent {
        FrontendEvent::Key(KeyEvent {
            frontend_id: FrontendId::LOCAL,
            key: Key::Char(c),
            mods: Modifiers::NONE,
            timestamp_ns: 0,
        })
    }

    #[test]
    fn consecutive_viewports_coalesce_to_the_latest() {
        let mut ob = Outbox::new();
        assert!(ob.enqueue(fe_viewport(1)));
        assert!(ob.enqueue(fe_viewport(2)));
        assert!(ob.enqueue(fe_viewport(3)));
        // A scroll flood collapses to one — the newest generation.
        assert_eq!(ob.queue.len(), 1);
        match &ob.queue[0] {
            FrontendEvent::Viewport { generation, .. } => assert_eq!(*generation, 3),
            other => panic!("expected a viewport, got {other:?}"),
        }
    }

    #[test]
    fn drags_coalesce_but_clicks_keep_their_order() {
        let mut ob = Outbox::new();
        // A press, a run of motion, and a release.
        ob.enqueue(fe_pointer(PointerKind::Down, 0));
        ob.enqueue(fe_pointer(PointerKind::Drag, 1));
        ob.enqueue(fe_pointer(PointerKind::Drag, 2));
        ob.enqueue(fe_pointer(PointerKind::Drag, 3));
        ob.enqueue(fe_pointer(PointerKind::Up, 4));
        // Down, one coalesced Drag (latest byte), Up — order preserved, the
        // gesture is intact; the drag run collapsed to O(1).
        assert_eq!(ob.queue.len(), 3);
        assert!(matches!(
            &ob.queue[0],
            FrontendEvent::Pointer {
                kind: PointerKind::Down,
                ..
            }
        ));
        assert!(matches!(
            &ob.queue[1],
            FrontendEvent::Pointer {
                kind: PointerKind::Drag,
                byte: 3,
                ..
            }
        ));
        assert!(matches!(
            &ob.queue[2],
            FrontendEvent::Pointer {
                kind: PointerKind::Up,
                ..
            }
        ));
    }

    fn fe_terminal_pointer(kind: MouseKind, row: u32, col: u32) -> FrontendEvent {
        FrontendEvent::TerminalPointer {
            frontend_id: FrontendId(1),
            buffer_id: BufferId::from_raw(1),
            coord: CellCoord::new(row, col),
            kind,
            mods: Modifiers::NONE,
        }
    }

    /// Acceptance 34: terminal move/drag runs coalesce to the latest
    /// cell, while press, release, and wheel stay lossless and ordered.
    #[test]
    fn terminal_motion_coalesces_but_presses_and_wheels_stay_lossless() {
        let mut ob = Outbox::new();
        ob.enqueue(fe_terminal_pointer(
            MouseKind::Down(MouseButton::Left),
            0,
            0,
        ));
        ob.enqueue(fe_terminal_pointer(
            MouseKind::Drag(MouseButton::Left),
            0,
            1,
        ));
        ob.enqueue(fe_terminal_pointer(
            MouseKind::Drag(MouseButton::Left),
            0,
            2,
        ));
        ob.enqueue(fe_terminal_pointer(
            MouseKind::Drag(MouseButton::Left),
            0,
            3,
        ));
        ob.enqueue(fe_terminal_pointer(MouseKind::Up(MouseButton::Left), 0, 3));
        assert_eq!(ob.queue.len(), 3, "the drag run collapsed to one");
        assert!(matches!(
            &ob.queue[1],
            FrontendEvent::TerminalPointer {
                kind: MouseKind::Drag(MouseButton::Left),
                coord: CellCoord { row: 0, col: 3 },
                ..
            }
        ));

        // Wheel ticks carry scroll DISTANCE; collapsing a run would
        // silently lose scrollback rows.
        let mut wheel = Outbox::new();
        wheel.enqueue(fe_terminal_pointer(MouseKind::ScrollUp, 1, 1));
        wheel.enqueue(fe_terminal_pointer(MouseKind::ScrollUp, 1, 1));
        wheel.enqueue(fe_terminal_pointer(MouseKind::ScrollUp, 1, 1));
        assert_eq!(wheel.queue.len(), 3, "wheel ticks are lossless");

        // Hover motion coalesces, but not across a different kind.
        let mut moves = Outbox::new();
        moves.enqueue(fe_terminal_pointer(MouseKind::Move, 2, 1));
        moves.enqueue(fe_terminal_pointer(MouseKind::Move, 2, 2));
        assert_eq!(moves.queue.len(), 1);
        moves.enqueue(fe_terminal_pointer(MouseKind::ScrollDown, 2, 2));
        moves.enqueue(fe_terminal_pointer(MouseKind::Move, 2, 3));
        assert_eq!(moves.queue.len(), 3);
    }

    #[test]
    fn coalescing_only_collapses_a_same_kind_tail() {
        let mut ob = Outbox::new();
        // Keys between viewports break the run: nothing coalesces, order
        // and every lossless event are preserved.
        ob.enqueue(fe_key('a'));
        ob.enqueue(fe_viewport(1));
        ob.enqueue(fe_key('b'));
        ob.enqueue(fe_viewport(2));
        assert_eq!(ob.queue.len(), 4);
        // A drag does not fold into a Down tail (different kind).
        let mut ob2 = Outbox::new();
        ob2.enqueue(fe_pointer(PointerKind::Down, 0));
        ob2.enqueue(fe_pointer(PointerKind::Drag, 1));
        assert_eq!(ob2.queue.len(), 2);
    }

    #[test]
    fn lossless_overflow_fails_fast_and_closes() {
        let mut ob = Outbox::new();
        for i in 0..OUTBOX_MAX {
            assert!(ob.enqueue(fe_key('x')), "fill up to the cap (i={i})");
        }
        // The cap-crossing lossless event is rejected and the outbox is
        // now closed — a clean disconnect beats dropping a CrdtOp silently.
        assert!(!ob.enqueue(fe_key('y')));
        assert!(ob.closed);
        // Once closed, everything is refused (including coalesceable kinds).
        assert!(!ob.enqueue(fe_viewport(1)));
    }

    #[test]
    fn coalescing_does_not_count_against_the_cap() {
        let mut ob = Outbox::new();
        // Even a huge scroll flood stays at one queued event, so it can
        // never trip the overflow fail-fast.
        for g in 0..(OUTBOX_MAX as u64 * 4) {
            assert!(ob.enqueue(fe_viewport(g)));
        }
        assert_eq!(ob.queue.len(), 1);
        assert!(!ob.closed);
    }

    #[test]
    fn a_closed_outbox_shuts_the_socket_down_to_wake_the_reader() {
        use std::io::Read;
        // A socketpair stands in for the daemon connection; `a` is the peer
        // a blocked reader would be reading from.
        let (mut a, b) = UnixStream::pair().expect("socketpair");
        // The post-overflow state: the outbox is already closed.
        let mut outbox = Outbox::new();
        outbox.closed = true;
        let client = AttachClient {
            outbox: Arc::new((Mutex::new(outbox), Condvar::new())),
            shutdown_handle: b,
            frontend_id: FrontendId::LOCAL,
            server_protocol_version: PROTOCOL_VERSION,
            initial_message: None,
        };
        // A send against the closed outbox fails *and* shuts the socket
        // down (F-008 fail-fast is now a real teardown, not just a flag).
        assert!(client.send_event(fe_key('x')).is_err());
        // The peer reads EOF: a real blocked reader would wake here and
        // fire Disconnected, instead of the session diverging silently.
        let mut buf = [0u8; 8];
        assert_eq!(
            a.read(&mut buf).expect("read peer"),
            0,
            "peer should see EOF after the shutdown"
        );
    }
    #[test]
    fn managed_attach_starts_only_for_absent_or_refused_sockets() {
        let temp = tempfile::tempdir().expect("tempdir");
        let socket_path = temp.path().join("managed.sock");
        let socket = socket_path.as_path();
        assert!(
            initial_startup_authorized(socket, &io::Error::new(io::ErrorKind::NotFound, "absent"))
                .expect("classify absent socket")
        );
        assert!(
            initial_startup_authorized(
                socket,
                &io::Error::new(io::ErrorKind::ConnectionRefused, "refused")
            )
            .expect("classify vanished socket")
        );
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::Interrupted,
            io::ErrorKind::WouldBlock,
            io::ErrorKind::InvalidInput,
        ] {
            assert!(
                !initial_startup_authorized(socket, &io::Error::new(kind, "final"))
                    .expect("classify final connect error"),
                "{kind:?} must not authorize daemon startup"
            );
        }
    }

    #[test]
    fn managed_retry_adds_only_interrupted_and_would_block() {
        let temp = tempfile::tempdir().expect("tempdir");
        let socket_path = temp.path().join("managed.sock");
        let socket = socket_path.as_path();
        for kind in [io::ErrorKind::Interrupted, io::ErrorKind::WouldBlock] {
            assert!(
                post_spawn_retryable(socket, &io::Error::new(kind, "transient"))
                    .expect("classify transient retry")
            );
        }
        assert!(
            !post_spawn_retryable(
                socket,
                &io::Error::new(io::ErrorKind::PermissionDenied, "final")
            )
            .expect("classify final retry error")
        );
    }

    #[test]
    fn managed_attach_refuses_a_non_socket_path_without_spawning() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let result = connect_managed_inner(
            &path,
            Path::new("/unused/pmacs"),
            |_| {
                Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    "synthetic refused connect",
                ))
            },
            |_, _| panic!("non-socket path must not spawn"),
            Duration::from_millis(1),
            Duration::from_millis(1),
            None,
            |_| false,
        );
        assert!(matches!(
            result,
            Err(ManagedAttachError::NonSocketPath(rejected)) if rejected == path
        ));
    }

    #[test]
    fn managed_attach_fails_closed_on_non_retryable_connect_errors() {
        let temp = tempfile::tempdir().expect("tempdir");
        let socket = temp.path().join("managed.sock");
        let result = connect_managed_inner(
            &socket,
            Path::new("/unused/pmacs"),
            |_| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "synthetic permission failure",
                ))
            },
            |_, _| panic!("permission failure must not spawn"),
            Duration::from_millis(1),
            Duration::from_millis(1),
            None,
            |_| false,
        );
        assert!(matches!(
            result,
            Err(ManagedAttachError::Attach(AttachClientError::Connect(error)))
                if error.kind() == io::ErrorKind::PermissionDenied
        ));
    }

    #[test]
    fn managed_retry_survives_transients_and_uses_the_successful_stream() {
        let temp = tempfile::tempdir().expect("tempdir");
        let socket = temp.path().join("managed.sock");
        let (client_stream, mut server_stream) = UnixStream::pair().expect("socket pair");
        let server = thread::spawn(move || {
            let hello = Hello {
                protocol_version: PROTOCOL_VERSION,
                assigned_frontend_id: FrontendId::LOCAL,
                instance_identity: InstanceIdentity {
                    pmacs_version: "managed-retry-test".to_owned(),
                    build_hash: None,
                    instance_name: None,
                    uptime_secs: 0,
                    working_directory: "/tmp".to_owned(),
                },
                instance_capabilities: caps(true, true, true),
            };
            write_message(&mut server_stream, &hello).expect("write Hello");
            let _: AttachRequest =
                read_message(&mut server_stream).expect("read real AttachRequest");
        });
        let mut attempts = 0;
        let mut client_stream = Some(client_stream);
        let managed = connect_managed_inner(
            &socket,
            Path::new("/bin/sh"),
            |_| {
                attempts += 1;
                match attempts {
                    1 => Err(io::Error::new(io::ErrorKind::NotFound, "initial miss")),
                    2 => Err(io::Error::new(io::ErrorKind::Interrupted, "signal")),
                    3 => Err(io::Error::new(io::ErrorKind::WouldBlock, "backlog")),
                    4 => Ok(client_stream.take().expect("single successful stream")),
                    _ => panic!("unexpected connection attempt"),
                }
            },
            |_, _| Command::new("/bin/sh").args(["-c", "exit 0"]).spawn(),
            Duration::from_secs(1),
            Duration::ZERO,
            None,
            |_| true,
        )
        .expect("transient sequence must attach");
        assert_eq!(attempts, 4);
        assert!(managed.daemon.spawned_daemon());
        assert_eq!(managed.client.server_protocol_version(), PROTOCOL_VERSION);
        server.join().expect("handshake server");
    }

    #[test]
    fn startup_timeout_reports_the_configured_duration() {
        let error = ManagedAttachError::StartupTimeout {
            socket: PathBuf::from("/tmp/unused.sock"),
            connect: io::Error::new(io::ErrorKind::NotFound, "still absent"),
            daemon_status: Some("exit status: 17".to_owned()),
            timeout: Duration::from_millis(1),
        };
        let message = error.to_string();
        assert!(
            message.contains("1ms"),
            "unexpected timeout message: {message}"
        );
        assert!(!message.contains("5 seconds"));
    }
}
