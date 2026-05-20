//! Attach-mode client: connect to a running pmacs daemon over a Unix
//! socket, negotiate `semantic_render + crdt_replica`, then pump
//! `InstanceMessage` frames onto the winit event loop.
//!
//! Session 3 of the pmacs-gpu arc — see `docs/pmacs-gpu-design.md`.
//! Scope: handshake + decode the message stream. Importing the CRDT
//! snapshot, applying live ops, and reconstructing the rope happen
//! on the main thread, where the `LoroDoc` lives (it isn't trivially
//! `Send`; cross-thread shipping is the *decoded* `InstanceMessage`,
//! not the doc state).
//!
//! The reader thread blocks on a single `read_message` per iteration;
//! every received message becomes an [`AttachEvent`] forwarded
//! through [`winit::event_loop::EventLoopProxy::send_event`], which
//! wakes the main loop so the frame logic can apply the message and
//! redraw.

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::thread;

use pmacs_protocol::{
    AttachRequest, FrontendCapabilities, Hello, InstanceMessage, PROTOCOL_VERSION,
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
        }
    }
}

impl std::error::Error for AttachClientError {}

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

/// Connect, handshake, and spawn the reader thread.
///
/// Returns once the handshake has completed and the reader thread is
/// running. The reader thread owns the read half of the stream; the
/// returned [`AttachClient`] retains the write half so the main loop
/// can eventually emit `FrontendEvent`s back to the daemon (session 4
/// will need this — selection / viewport / edits travel that way).
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

    // Split read/write halves for the reader thread + main-thread
    // write path. UnixStream clones share the underlying FD with
    // independent buffer state — safe to read on one clone while the
    // other writes (the FD is full-duplex).
    let mut read_stream = stream.try_clone().map_err(AttachClientError::Connect)?;
    let write_stream = stream;

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

    Ok(AttachClient {
        _write_stream: Arc::new(write_stream),
    })
}

/// Handle the main loop keeps after `connect` returns. Session 3
/// holds only the write half (unused yet — session 4 wires events
/// back); the read half lives in the spawned reader thread. The
/// `Arc` is so the handle is `Clone` for future per-window cloning,
/// not because we need shared ownership today.
#[allow(dead_code)]
pub struct AttachClient {
    _write_stream: Arc<UnixStream>,
}
