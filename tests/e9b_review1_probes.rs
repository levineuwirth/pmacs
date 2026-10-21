//! E9b review: mixed projections and overlapping losses keep each session independent.
#![cfg(feature = "crdt")]

mod common;

use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pmacs_protocol::cell::CellSize;
use pmacs_protocol::message::{
    ADVERTISED_PROTOCOL_VERSION, AttachRequest, FrontendCapabilities, FrontendEvent, Hello,
    InstanceMessage, Key, KeyEvent, Modifiers, PROTOCOL_VERSION, SessionBootstrapRequest,
};
use pmacs_protocol::transport::{read_message, write_message};
use pmacs_protocol::{BufferId, FrontendId, SESSION_MIN_VERSION, SessionToken};

use common::daemon::{TestDaemon, build_default_caps};

/// One attached semantic session: its writer, a channel of everything
/// the daemon wrote to it, its id and the token it was minted.
struct Session {
    stream: UnixStream,
    rx: mpsc::Receiver<InstanceMessage>,
    fid: FrontendId,
    token: Option<SessionToken>,
}

fn semantic_caps() -> FrontendCapabilities {
    FrontendCapabilities {
        multi_frontend: true,
        crdt_replica: true,
        semantic_render: true,
        ..build_default_caps()
    }
}

/// The handshake as `pmacs-gpu` performs it at `offer`: the v20
/// bootstrap for a semantic session, and at v27 the `Resume` step
/// carrying `resume`. Reads nothing past the handshake; the reader
/// thread does.
fn handshake(
    daemon: &TestDaemon,
    offer: u32,
    resume: Option<SessionToken>,
) -> (UnixStream, FrontendId) {
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("read timeout");
    let hello: Hello = read_message(&mut stream).expect("read Hello");
    assert_eq!(hello.protocol_version, ADVERTISED_PROTOCOL_VERSION);
    let fid = hello.assigned_frontend_id;
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
            initial_target: None,
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
    (stream, fid)
}

impl Session {
    /// Attach at `offer`, presenting `resume`, and read through the
    /// bootstrap: at v27 up to the `SessionToken`, which the daemon
    /// writes after its snapshots and before any frame.
    fn attach(daemon: &TestDaemon, offer: u32, resume: Option<SessionToken>) -> Self {
        let (stream, fid) = handshake(daemon, offer, resume);
        let mut reader = stream.try_clone().expect("clone stream");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(msg) = read_message::<InstanceMessage>(&mut reader) {
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        let mut session = Self {
            stream,
            rx,
            fid,
            token: None,
        };
        if offer >= SESSION_MIN_VERSION {
            let token = session.wait("the SessionToken", |m| match m {
                InstanceMessage::SessionToken(t) => Some(*t),
                _ => None,
            });
            session.token = Some(token);
        }
        // The daemon emits nothing semantic (status facts, minibuffer
        // rows) to a session until it declares a viewport, and a
        // `Viewport` also moves the window to the buffer it names: so
        // it is declared on the buffer the daemon's own cursor says the
        // window is on, never on the last snapshot of the bootstrap
        // sweep, which is whichever buffer the registry listed last.
        let (document, _) = session.where_am_i();
        session.declare_viewport(document);
        session
    }

    /// Declare a viewport on `buffer`, as `pmacs-gpu` does after every
    /// snapshot; the semantic frames for it follow.
    fn declare_viewport(&mut self, buffer: BufferId) {
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

    fn key(&mut self, key: Key, mods: Modifiers) {
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

    fn type_str(&mut self, text: &str) {
        for ch in text.chars() {
            self.key(Key::Char(ch), Modifiers::NONE);
        }
    }

    /// Drain until `pick` accepts a message, or ten seconds; the timeout
    /// names every message that came meanwhile.
    fn wait<T>(&mut self, what: &str, mut pick: impl FnMut(&InstanceMessage) -> Option<T>) -> T {
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
    fn where_am_i(&mut self) -> (BufferId, u64) {
        self.wait("a CursorByte", |m| match m {
            InstanceMessage::CursorByte {
                buffer_id,
                byte_pos,
            } => Some((*buffer_id, *byte_pos)),
            _ => None,
        })
    }

    /// `M-x <name> RET`.
    fn run_command(&mut self, name: &str) {
        self.key(Key::Char('x'), Modifiers::ALT);
        self.type_str(name);
        self.key(Key::Enter, Modifiers::NONE);
    }

    /// Open the daemon's note through `e9.open` (the init's command over
    /// `pmacs.buffer.find_or_open`) in this session's window, and wait
    /// until the daemon's cursor says the window left `current`.
    fn open_note(&mut self, current: BufferId) -> BufferId {
        self.run_command("e9.open");
        let note = self.wait("the window to reach the note", |m| match m {
            InstanceMessage::CursorByte { buffer_id, .. } if *buffer_id != current => {
                Some(*buffer_id)
            }
            _ => None,
        });
        self.declare_viewport(note);
        note
    }
}

const DETACH_LOG: &str = "\
pmacs.lsp.config = {}
pmacs.hook.add('frontend.detached', function(fid)
  local f = assert(io.open(os.getenv('HOME') .. '/detached.log', 'a'))
  f:write(tostring(fid), '\\n')
  f:close()
end)
local path = os.getenv('HOME') .. '/note.txt'
local f = assert(io.open(path, 'w')); f:write('one\\ntwo\\n'); f:close()
pmacs.command.define { name = 'e9.open', description = 'open the note',
  fn = function() pmacs.buffer.find_or_open(path) end }
";

fn daemon() -> TestDaemon {
    TestDaemon::spawn_with_env_and_init(&[("PMACS_ATTACH_DEBUG", "1")], DETACH_LOG)
}

fn detached(daemon: &TestDaemon) -> Vec<String> {
    std::fs::read_to_string(daemon.socket_path().parent().unwrap().join("detached.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn wait_for_detach(daemon: &TestDaemon, fid: FrontendId) {
    common::ready::expect_true(
        "teardown hook",
        Duration::from_secs(10),
        || detached(daemon).contains(&fid.0.to_string()),
        || format!("{:?}", detached(daemon)),
    );
}

impl Session {
    fn grid(daemon: &TestDaemon) -> Self {
        let mut stream = daemon.connect();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let hello: Hello = read_message(&mut stream).unwrap();
        let fid = hello.assigned_frontend_id;
        let mut caps = semantic_caps();
        caps.semantic_render = false;
        write_message(
            &mut stream,
            &AttachRequest {
                protocol_version: PROTOCOL_VERSION,
                frontend_capabilities: caps,
                initial_size: CellSize::new(19, 67),
            },
        )
        .unwrap();
        // Grid sessions send neither the semantic bootstrap nor Resume.
        let mut reader = stream.try_clone().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(msg) = read_message::<InstanceMessage>(&mut reader) {
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        let mut grid = Self {
            stream,
            rx,
            fid,
            token: None,
        };
        grid.wait("grid's initial resync", |m| {
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

    fn cursor_at(&mut self, buffer: BufferId, byte: u64) {
        self.wait("the expected buffer and cursor", |m| {
            matches!(m,
            InstanceMessage::CursorByte { buffer_id, byte_pos }
            if *buffer_id == buffer && *byte_pos == byte)
            .then_some(())
        });
    }

    fn idle(&mut self, expected: bool) {
        self.wait("this session's idle transition", |m| {
            matches!(m,
            InstanceMessage::DispatchIdle { idle } if *idle == expected)
            .then_some(())
        });
    }

    fn lose(&mut self, daemon: &TestDaemon) {
        self.stream.shutdown(std::net::Shutdown::Both).unwrap();
        wait_for_detach(daemon, self.fid);
    }

    #[cfg(target_os = "linux")]
    fn begin_drain(&mut self, daemon: &TestDaemon, buffer: BufferId) {
        self.stream.shutdown(std::net::Shutdown::Read).unwrap();
        self.declare_viewport(buffer);
        let path = format!("{}.stderr.log", daemon.socket_path().display());
        common::ready::expect_true(
            "this window's failed write",
            Duration::from_secs(10),
            || {
                std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .lines()
                    .any(|line| {
                        line.contains(&format!("{:?}", self.fid)) && line.contains("Broken pipe")
                    })
            },
            || std::fs::read_to_string(&path).unwrap_or_default(),
        );
    }
}

#[test]
fn e9b_review1_grid_departure_preserves_a_semantic_prefix_and_view() {
    let daemon = daemon();
    let mut semantic = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let (scratch, _) = semantic.where_am_i();
    let note = semantic.open_note(scratch);
    semantic.type_str("abc");
    semantic.cursor_at(note, 3);
    let mut grid = Session::grid(&daemon);
    grid.cursor_at(scratch, 0);
    semantic.key(Key::Char('x'), Modifiers::CTRL);
    semantic.idle(false);
    // Another projection's keys must not finish or clear this pending prefix.
    grid.type_str("g");
    grid.cursor_at(scratch, 1);
    write_message(&mut grid.stream, &FrontendEvent::Detach(grid.fid)).unwrap();
    grid.stream.shutdown(std::net::Shutdown::Write).unwrap();
    wait_for_detach(&daemon, grid.fid);
    // Completing C-x C-f must still invoke find-file. A cleared prefix
    // would turn C-f into ordinary cursor motion instead.
    semantic.key(Key::Char('f'), Modifiers::CTRL);
    semantic.wait("the preserved prefix invokes find-file", |m| match m {
        InstanceMessage::MinibufferPromptRows {
            prompt: Some(prompt),
            ..
        } if prompt.to_lowercase().contains("find file") => Some(()),
        _ => None,
    });
    semantic.key(Key::Char('g'), Modifiers::CTRL);
    semantic.idle(true);
    semantic.type_str("Z");
    semantic.cursor_at(note, 4);
    let token = semantic.token.unwrap();
    semantic.lose(&daemon);
    let mut resumed = Session::attach(&daemon, PROTOCOL_VERSION, Some(token));
    assert_eq!(resumed.where_am_i(), (note, 4));
    resumed.type_str("!");
    resumed.cursor_at(note, 5);
    assert_eq!(
        detached(&daemon)
            .iter()
            .filter(|id| **id == grid.fid.0.to_string())
            .count(),
        1
    );
}

#[cfg(target_os = "linux")]
#[test]
fn e9b_review1_overlapping_drains_resume_two_distinct_views_in_reverse_order() {
    let daemon = daemon();
    let mut a = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let (scratch, _) = a.where_am_i();
    let note = a.open_note(scratch);
    a.type_str("abc");
    a.cursor_at(note, 3);
    let mut b = Session::attach(&daemon, PROTOCOL_VERSION, None);
    b.cursor_at(scratch, 0);
    b.type_str("q");
    b.cursor_at(scratch, 1);
    let (ta, tb) = (a.token.unwrap(), b.token.unwrap());
    assert_ne!(ta, tb);
    a.begin_drain(&daemon, note);
    b.begin_drain(&daemon, scratch);
    // Both outputs are now broken; input still belongs to its own view.
    a.type_str("XY");
    b.type_str("ZZ");
    a.stream.shutdown(std::net::Shutdown::Write).unwrap();
    b.stream.shutdown(std::net::Shutdown::Write).unwrap();
    wait_for_detach(&daemon, a.fid);
    wait_for_detach(&daemon, b.fid);
    let mut rb = Session::attach(&daemon, PROTOCOL_VERSION, Some(tb));
    assert_eq!(rb.where_am_i(), (scratch, 3));
    let mut ra = Session::attach(&daemon, PROTOCOL_VERSION, Some(ta));
    assert_eq!(ra.where_am_i(), (note, 5));
    // Teardown and fresh baselines must leave both resumed outputs usable.
    rb.type_str("!");
    ra.type_str("?");
    rb.cursor_at(scratch, 4);
    ra.cursor_at(note, 6);
    assert_eq!(detached(&daemon).len(), 2);
}
