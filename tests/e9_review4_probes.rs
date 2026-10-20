//! E9 review 4: a daemon write failure must not retire the old reader
//! before its remaining input and EOF. Both rows drive a real daemon
//! through the semantic wire; only the downstream half-close differs.
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

/// A daemon whose init defines `e9.open`, opening the note written
/// beside its socket, logs each frontend its detach arm tears down
/// ([`DETACH_LOG`], read by [`Session::lose`]), plus `extra` config.
fn daemon_with_note(extra: &str) -> TestDaemon {
    let dir = tempfile::TempDir::new().expect("tempdir for the note");
    let note = dir.path().join("note.txt");
    std::fs::write(&note, "one\ntwo\n").expect("write the note");
    let init = format!(
        "pmacs.command.define {{ name = 'e9.open', description = 'open the note',\n\
           fn = function() pmacs.buffer.find_or_open({path:?}) end }}\n{DETACH_LOG}{extra}",
        path = note.to_str().expect("utf-8 path"),
    );
    // The daemon's handshake breadcrumbs go to its stderr file, which a
    // failing row prints.
    let daemon = TestDaemon::spawn_with_env_and_init(&[("PMACS_ATTACH_DEBUG", "1")], &init);
    // The tempdir outlives the daemon: leak it into the process.
    std::mem::forget(dir);
    daemon
}

/// A `frontend.detached` hook that logs the departed id into the
/// daemon's isolated home (its `HOME` is the socket's directory), so a
/// row can wait for a loss to have been handled: the hook runs last in
/// the daemon's detach arm, after the view is retained or dropped. It
/// edits nothing (the undo suite's `DETACH_LOG_CONFIG`).
const DETACH_LOG: &str = "\
pmacs.hook.add('frontend.detached', function(fid)
  local f = assert(io.open(os.getenv('HOME') .. '/detached.log', 'a'))
  f:write(tostring(fid), '\\n')
  f:close()
end)
";

/// The frontend ids the daemon's detach arm has torn down, in order.
fn detached(daemon: &TestDaemon) -> Vec<String> {
    let log = daemon
        .socket_path()
        .parent()
        .expect("the socket's directory")
        .join("detached.log");
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn daemon_log(daemon: &TestDaemon) -> String {
    std::fs::read_to_string(format!("{}.stderr.log", daemon.socket_path().display()))
        .unwrap_or_default()
}

/// The failed-write arm must not retire the reader's identity before
/// its EOF. A forwarding proxy can lose its downstream direction while
/// upstream input is still in flight. The control keeps downstream open.
fn old_input_before_eof(fail_write: bool) {
    let daemon = daemon_with_note("pmacs.lsp.config = {}\n");
    let mut first = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let token = first.token.expect("token");
    let (scratch, _) = first.where_am_i();
    let note = first.open_note(scratch);
    first.type_str("abc");
    first.wait("abc applied", |m| {
        matches!(m,
            InstanceMessage::CursorByte { buffer_id, byte_pos: 3 } if *buffer_id == note
        )
        .then_some(())
    });
    if fail_write {
        first
            .stream
            .shutdown(std::net::Shutdown::Read)
            .expect("half close");
        // Re-declaring the same viewport leaves the cursor in place and
        // asks for a frame on the failed downstream direction.
        first.declare_viewport(note);
        common::ready::expect_true(
            "the daemon's failed write",
            Duration::from_secs(10),
            || daemon_log(&daemon).contains("Broken pipe"),
            || daemon_log(&daemon),
        );
    }
    // These writes succeed on the OLD connection before EOF. A refused
    // new connection cannot make up for discarding them at this reader.
    first.type_str("XY");
    first
        .stream
        .shutdown(std::net::Shutdown::Write)
        .expect("old EOF");
    let id = first.fid.0.to_string();
    common::ready::expect_true(
        "the old session's detach",
        Duration::from_secs(10),
        || detached(&daemon).contains(&id),
        || daemon_log(&daemon),
    );
    let mut resumed = Session::attach(&daemon, PROTOCOL_VERSION, Some(token));
    let where_now = resumed.where_am_i();
    eprintln!(
        "failed_write={fail_write}; resumed={where_now:?}; daemon log:\n{}",
        daemon_log(&daemon)
    );
    assert_eq!(
        where_now,
        (note, 5),
        "the two old-socket keys must precede the resumed cursor"
    );
}

#[test]
fn e9_review4_healthy_downstream_drains_input_before_resume() {
    old_input_before_eof(false);
}

/// Compiled on Linux alone (E9 fix round 4): the half-close makes the
/// daemon's write fail only where a write to a peer that stopped reading
/// is refused, and BSD lets it succeed (E8d.4's row, macOS CI run
/// 38558729706), so on macOS the row would wait for a failure that never
/// comes rather than test the drain.
#[cfg(target_os = "linux")]
#[test]
fn e9_review4_failed_write_must_drain_old_input_before_resume() {
    old_input_before_eof(true);
}
