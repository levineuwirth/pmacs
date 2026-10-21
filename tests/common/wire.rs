//! A frontend session over a real daemon socket, driven as the frontends
//! drive it: the handshake, a reader thread feeding a channel, and the
//! keys, viewports and waits a row needs.
//!
//! Extracted at E10 under D37 from the semantic-session client the E9
//! suites and review probes each carry a copy of (the refactoring
//! audit's F3, "`struct Session` in 8 files"), so E10's suite adds no
//! ninth; those copies stay where they are, the review probes' because
//! a probe's harness is its evidence. Everything here is what that
//! client did, with two additions E10 needs: an initial target in the
//! bootstrap, and the messages read before the token kept in
//! [`WireSession::bootstrap`], since a restored window is told apart by
//! what its bootstrap carried.

use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pmacs_protocol::cell::CellSize;
use pmacs_protocol::message::{
    ADVERTISED_PROTOCOL_VERSION, AttachRequest, FrontendCapabilities, FrontendEvent, Hello,
    InitialTarget, InstanceMessage, Key, KeyEvent, Modifiers, PROTOCOL_VERSION,
    SessionBootstrapRequest,
};
use pmacs_protocol::transport::{read_message, write_message};
use pmacs_protocol::{BufferId, FrontendId, SESSION_MIN_VERSION, SessionToken};

use super::daemon::{TestDaemon, build_default_caps};

/// The capabilities `pmacs-gpu` declares: a multi-frontend CRDT replica
/// that renders semantically.
pub fn semantic_caps() -> FrontendCapabilities {
    FrontendCapabilities {
        multi_frontend: true,
        crdt_replica: true,
        semantic_render: true,
        ..build_default_caps()
    }
}

/// One attached session: its writer, a channel of everything the daemon
/// wrote to it after the bootstrap, its id, the token it was minted (a
/// semantic session at v27 or later) and what the daemon wrote before
/// that token.
pub struct WireSession {
    /// The socket's write side; a row may also shut it down.
    pub stream: UnixStream,
    /// Everything the daemon writes from the token on (or, for a session
    /// minted none, from its first message on).
    pub rx: mpsc::Receiver<InstanceMessage>,
    /// The id the daemon's `Hello` assigned.
    pub fid: FrontendId,
    /// The token the daemon minted this session, if it did.
    pub token: Option<SessionToken>,
    /// What the daemon wrote before the token, in order: the snapshots
    /// and the target's result of a semantic bootstrap. Empty for a
    /// session minted no token.
    pub bootstrap: Vec<InstanceMessage>,
}

impl WireSession {
    /// Attach a semantic session at `offer`, as `pmacs-gpu` does: the
    /// `AttachRequest`, the v20 bootstrap carrying `target`, and from
    /// [`SESSION_MIN_VERSION`] the `Resume` step presenting `resume`.
    /// Reads through the bootstrap to the `SessionToken` at v27, keeping
    /// what came before it, and declares no viewport: the row decides
    /// where its window looks.
    pub fn semantic(
        daemon: &TestDaemon,
        offer: u32,
        target: Option<InitialTarget>,
        resume: Option<SessionToken>,
    ) -> Self {
        let (mut stream, fid) = Self::hello(daemon);
        write_message(
            &mut stream,
            &AttachRequest {
                protocol_version: offer,
                frontend_capabilities: semantic_caps(),
                initial_size: CellSize::new(24, 80),
            },
        )
        .expect("AttachRequest");
        write_message(
            &mut stream,
            &SessionBootstrapRequest {
                initial_target: target,
            },
        )
        .expect("bootstrap");
        if offer >= SESSION_MIN_VERSION {
            write_message(
                &mut stream,
                &FrontendEvent::Resume {
                    frontend_id: fid,
                    token: resume,
                },
            )
            .expect("Resume");
        }
        let mut session = Self::reading(stream, fid);
        if offer >= SESSION_MIN_VERSION {
            let mut bootstrap = Vec::new();
            let token = session.wait("the SessionToken", |m| match m {
                InstanceMessage::SessionToken(t) => Some(*t),
                other => {
                    bootstrap.push(other.clone());
                    None
                }
            });
            session.token = Some(token);
            session.bootstrap = bootstrap;
        }
        session
    }

    /// Attach a grid session at this build's version, as the attached
    /// TUI does (no semantic bootstrap, no `Resume`), sized `size`, and
    /// wait for its first full-grid resync.
    pub fn grid(daemon: &TestDaemon, size: CellSize) -> Self {
        let (mut stream, fid) = Self::hello(daemon);
        let mut caps = semantic_caps();
        caps.semantic_render = false;
        write_message(
            &mut stream,
            &AttachRequest {
                protocol_version: PROTOCOL_VERSION,
                frontend_capabilities: caps,
                initial_size: size,
            },
        )
        .expect("AttachRequest");
        let mut grid = Self::reading(stream, fid);
        grid.wait("the grid's first resync", |m| {
            matches!(
                m,
                InstanceMessage::CellDelta {
                    full_grid: true,
                    ..
                }
            )
            .then_some(())
        });
        grid
    }

    /// Connect and read the daemon's `Hello`, with a ten-second read
    /// timeout on the socket.
    fn hello(daemon: &TestDaemon) -> (UnixStream, FrontendId) {
        let mut stream = daemon.connect();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("read timeout");
        let hello: Hello = read_message(&mut stream).expect("read Hello");
        assert_eq!(hello.protocol_version, ADVERTISED_PROTOCOL_VERSION);
        (stream, hello.assigned_frontend_id)
    }

    /// Start the reader thread on `stream`.
    fn reading(stream: UnixStream, fid: FrontendId) -> Self {
        let mut reader = stream.try_clone().expect("clone stream");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(msg) = read_message::<InstanceMessage>(&mut reader) {
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        Self {
            stream,
            rx,
            fid,
            token: None,
            bootstrap: Vec::new(),
        }
    }

    /// The buffers the bootstrap sent snapshots of, in order.
    pub fn bootstrap_snapshots(&self) -> Vec<BufferId> {
        self.bootstrap
            .iter()
            .filter_map(|m| match m {
                InstanceMessage::BufferSnapshot { buffer_id, .. } => Some(*buffer_id),
                _ => None,
            })
            .collect()
    }

    /// Send one key.
    pub fn key(&mut self, key: Key, mods: Modifiers) {
        write_message(
            &mut self.stream,
            &FrontendEvent::Key(KeyEvent {
                frontend_id: self.fid,
                key,
                mods,
                timestamp_ns: 0,
            }),
        )
        .expect("send Key");
    }

    /// Type `text`, one character a key.
    pub fn type_str(&mut self, text: &str) {
        for ch in text.chars() {
            self.key(Key::Char(ch), Modifiers::NONE);
        }
    }

    /// `M-x <name> RET`.
    pub fn run_command(&mut self, name: &str) {
        self.key(Key::Char('x'), Modifiers::ALT);
        self.type_str(name);
        self.key(Key::Enter, Modifiers::NONE);
    }

    /// Declare a viewport on `buffer`, as `pmacs-gpu` does after every
    /// snapshot; a `Viewport` also moves the window to the buffer it
    /// names, and the semantic frames for it follow.
    pub fn declare_viewport(&mut self, buffer: BufferId) {
        write_message(
            &mut self.stream,
            &FrontendEvent::Viewport {
                frontend_id: self.fid,
                buffer_id: buffer,
                visible: pmacs_protocol::ByteRange {
                    start: 0,
                    end: 4096,
                },
                generation: 0,
            },
        )
        .expect("declare a viewport");
    }

    /// Drain until `pick` accepts a message, or ten seconds; the timeout
    /// names every message that came meanwhile.
    pub fn wait<T>(
        &mut self,
        what: &str,
        mut pick: impl FnMut(&InstanceMessage) -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut seen: Vec<String> = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting for {what}; the daemon wrote {seen:?}"
            );
            match self
                .rx
                .recv_timeout(remaining.min(Duration::from_millis(200)))
            {
                Ok(msg) => {
                    if let Some(found) = pick(&msg) {
                        return found;
                    }
                    let mut summary = format!("{msg:?}");
                    summary.truncate(80);
                    seen.push(summary);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("the daemon closed the session while waiting for {what}");
                }
            }
        }
    }

    /// The buffer and byte the daemon says this session's document
    /// window is at, from the next `CursorByte` it writes.
    pub fn where_am_i(&mut self) -> (BufferId, u64) {
        self.wait("a CursorByte", |m| match m {
            InstanceMessage::CursorByte {
                buffer_id,
                byte_pos,
            } => Some((*buffer_id, *byte_pos)),
            _ => None,
        })
    }

    /// Wait for a `CursorByte` naming exactly `buffer` at `byte`.
    pub fn cursor_at(&mut self, buffer: BufferId, byte: u64) {
        self.wait("the expected buffer and cursor", |m| {
            matches!(m,
            InstanceMessage::CursorByte { buffer_id, byte_pos }
            if *buffer_id == buffer && *byte_pos == byte)
            .then_some(())
        });
    }

    /// Leave as a window closing does: `Detach`, then the write side
    /// shut, so the daemon reads the departure before the EOF.
    pub fn depart(&mut self) {
        write_message(&mut self.stream, &FrontendEvent::Detach(self.fid)).expect("Detach");
        self.stream
            .shutdown(std::net::Shutdown::Write)
            .expect("shut the write side");
    }

    /// Lose the socket without a word: a loss, which the daemon retains
    /// the view of for the token.
    pub fn lose(&mut self) {
        self.stream
            .shutdown(std::net::Shutdown::Both)
            .expect("shut the socket");
    }
}
