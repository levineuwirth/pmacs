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
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use pmacs_protocol::{
    AttachRequest, BufferId, ByteRange, CrdtOp, FrontendCapabilities, FrontendEvent, FrontendId,
    Hello, InstanceMessage, Key, KeyEvent, Modifiers, PROTOCOL_VERSION, PointerKind,
    SUPPORTED_PROTOCOL_VERSIONS, TransportError, is_supported_protocol_version, read_message,
    write_message,
};
use winit::event_loop::EventLoopProxy;

use crate::AppEvent;

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
        }
    }
}

impl std::error::Error for AttachClientError {}

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
pub fn connect(
    socket_path: &Path,
    proxy: EventLoopProxy<AppEvent>,
) -> Result<AttachClient, AttachClientError> {
    let stream = UnixStream::connect(socket_path).map_err(AttachClientError::Connect)?;

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

    // Split read/write halves for the reader thread + writer thread.
    // UnixStream clones share the underlying FD with independent
    // buffer state — safe to read on one clone while the other writes
    // (the FD is full-duplex).
    let mut read_stream = stream.try_clone().map_err(AttachClientError::Connect)?;
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
                        if proxy
                            .send_event(AppEvent::Attach(AttachEvent::Message(Box::new(msg))))
                            .is_err()
                        {
                            // Main loop torn down — quietly exit.
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = proxy
                            .send_event(AppEvent::Attach(AttachEvent::Disconnected(e.to_string())));
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
                        return;
                    }
                }
            }
        })
        .expect("spawn attach writer thread");

    Ok(AttachClient {
        outbox,
        frontend_id: hello.assigned_frontend_id,
        server_protocol_version: hello.protocol_version,
    })
}

/// Handle the main loop keeps after `connect` returns. It queues
/// `FrontendEvent`s for the attach writer thread.
pub struct AttachClient {
    /// Bounded, coalescing outbound queue (F-008) drained by the writer
    /// thread. `send_event` locks it, applies the enqueue policy, and
    /// wakes the writer via the paired `Condvar`.
    outbox: Arc<(Mutex<Outbox>, Condvar)>,
    /// Assigned by the daemon in the `Hello` response. Every
    /// `FrontendEvent` carries this so the daemon can route input back
    /// to the per-session `SemanticRenderState`.
    frontend_id: FrontendId,
    /// The daemon's `Hello.protocol_version`. Wire variants newer
    /// than the daemon (e.g. `Pointer`, v5) must be gated on this —
    /// an older daemon hard-errors decoding an unknown variant.
    server_protocol_version: u32,
}

impl AttachClient {
    /// Frontend id assigned by the daemon in the initial `Hello`.
    pub fn frontend_id(&self) -> FrontendId {
        self.frontend_id
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
    use pmacs_protocol::InstanceCapabilities;

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
}
