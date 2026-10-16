//! E9 --- session identity: the token a daemon mints for a v27 semantic
//! session, the `Resume` step that presents it on a later socket, and
//! the view retained between the two, through the production handshake
//! and dispatch of a real daemon.
//!
//! Every row speaks the wire as `pmacs-gpu` does --- `Hello`,
//! `AttachRequest`, `SessionBootstrapRequest`, `Resume`, then keys as
//! `FrontendEvent::Key` into the daemon's own dispatcher --- and reads
//! what the daemon writes to each session, the outermost seam a semantic
//! frontend has. A reader thread per session feeds a channel, so a
//! time-boxed drain never cuts a frame in half.
#![cfg(feature = "crdt")]

mod common;

use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pmacs_protocol::cell::CellSize;
use pmacs_protocol::message::{
    ADVERTISED_PROTOCOL_VERSION, AttachRequest, FrontendCapabilities, FrontendEvent, GoodbyeReason,
    Hello, InstanceMessage, Key, KeyEvent, Modifiers, PROTOCOL_VERSION, SessionBootstrapRequest,
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
        session
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

    /// Drain until `pick` accepts a message, or ten seconds.
    fn wait<T>(&mut self, what: &str, mut pick: impl FnMut(&InstanceMessage) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "timed out waiting for {what}");
            match self
                .rx
                .recv_timeout(remaining.min(Duration::from_millis(200)))
            {
                Ok(msg) => {
                    if let Some(found) = pick(&msg) {
                        return found;
                    }
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

    /// Everything the daemon writes in `window`, by variant name. A
    /// window before a negative assertion, named with its length.
    fn drain_for(&mut self, window: Duration) -> Vec<String> {
        let deadline = Instant::now() + window;
        let mut seen = Vec::new();
        while let Ok(msg) = self
            .rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            seen.push(
                format!("{msg:?}")
                    .split(['(', ' ', '{'])
                    .next()
                    .unwrap_or("")
                    .to_owned(),
            );
            if Instant::now() >= deadline {
                break;
            }
        }
        seen
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
        self.wait("the window to reach the note", |m| match m {
            InstanceMessage::CursorByte { buffer_id, .. } if *buffer_id != current => {
                Some(*buffer_id)
            }
            _ => None,
        })
    }
}

/// A daemon whose init defines `e9.open`, opening the note written
/// beside its socket, plus `extra` config.
fn daemon_with_note(extra: &str) -> TestDaemon {
    let dir = tempfile::TempDir::new().expect("tempdir for the note");
    let note = dir.path().join("note.txt");
    std::fs::write(&note, "one\ntwo\n").expect("write the note");
    let init = format!(
        "pmacs.command.define {{ name = 'e9.open', description = 'open the note',\n\
           fn = function() pmacs.buffer.find_or_open({path:?}) end }}\n{extra}",
        path = note.to_str().expect("utf-8 path"),
    );
    let daemon = TestDaemon::spawn_with_config(&init);
    // The tempdir outlives the daemon: leak it into the process.
    std::mem::forget(dir);
    daemon
}

// ---------------------------------------------------------------------------
// E9.1 --- the token
// ---------------------------------------------------------------------------

/// A v27 semantic session is minted a token after its bootstrap; a v26
/// session, which never negotiated the family, is written none in the
/// first second of frames (a window before a negative assertion, named
/// with its length), and is never read for a `Resume` step: its
/// handshake is the v26 one and it is served.
#[test]
fn e9_1_a_v27_session_is_minted_a_token_and_a_v26_one_is_not() {
    let daemon = daemon_with_note("");
    let mut current = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let token = current.token.expect("the v27 session's token");
    assert!(!token.is_zero(), "a minted token is never all zero");
    let (document, _) = current.where_am_i();

    let mut legacy = Session::attach(&daemon, SESSION_MIN_VERSION - 1, None);
    assert_eq!(legacy.token, None);
    let (legacy_document, _) = legacy.where_am_i();
    assert_eq!(
        legacy_document, document,
        "both sessions start on LOCAL's document"
    );
    let seen = legacy.drain_for(Duration::from_secs(1));
    assert!(
        !seen.iter().any(|m| m == "SessionToken"),
        "a v26 session is written no token; the daemon wrote {seen:?}"
    );

    // A second v27 session is minted its own token.
    let other = Session::attach(&daemon, PROTOCOL_VERSION, None);
    assert_ne!(other.token, Some(token), "tokens are per session");
}

/// The view survives the socket under its token: a session that opened
/// the note and moved its cursor drops; the next socket presenting the
/// token starts on the note at that cursor, while a socket presenting
/// nothing starts on LOCAL's document, and the consumed token resumes
/// nothing a second time.
#[test]
fn e9_1_the_view_is_retained_across_detach_and_taken_back_by_its_token() {
    let daemon = daemon_with_note("");
    let mut first = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let token = first.token.expect("the first session's token");
    let (scratch, _) = first.where_am_i();
    let note = first.open_note(scratch);
    // `one\ntwo\n`: to the end of the first line.
    first.key(Key::Char('e'), Modifiers::CTRL);
    first.wait("the cursor at the end of the first line", |m| {
        matches!(m, InstanceMessage::CursorByte { buffer_id, byte_pos: 3 } if *buffer_id == note)
            .then_some(())
    });
    drop(first);

    // A fresh socket, nothing presented: LOCAL's document, as before E9.
    let mut fresh = Session::attach(&daemon, PROTOCOL_VERSION, None);
    assert_eq!(
        fresh.where_am_i(),
        (scratch, 0),
        "a fresh view is LOCAL's document"
    );

    // The token: the note, at the cursor the first session left.
    let mut resumed = Session::attach(&daemon, PROTOCOL_VERSION, Some(token));
    assert_eq!(
        resumed.where_am_i(),
        (note, 3),
        "the resumed session starts on the retained window"
    );
    let resumed_token = resumed
        .token
        .expect("a resumed session is minted its own token");
    assert_ne!(resumed_token, token, "the presented token is consumed");

    // Presenting the consumed token again is a fresh view.
    let mut again = Session::attach(&daemon, PROTOCOL_VERSION, Some(token));
    assert_eq!(
        again.where_am_i(),
        (scratch, 0),
        "a consumed token resumes nothing"
    );

    // And the resumed session's own token carries the same view on.
    drop(resumed);
    let mut third = Session::attach(&daemon, PROTOCOL_VERSION, Some(resumed_token));
    assert_eq!(third.where_am_i(), (note, 3), "the chain continues");
}

/// A retained view is kept for `session.view-retention-secs` and no
/// longer: with the knob at one second, a token presented two and a half
/// seconds after its session left starts on a fresh view.
#[test]
fn e9_1_a_retained_view_expires_after_the_retention_window() {
    let daemon = daemon_with_note("pmacs.config.set('session.view-retention-secs', 1)\n");
    let mut first = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let token = first.token.expect("token");
    let (scratch, _) = first.where_am_i();
    let note = first.open_note(scratch);
    assert_ne!(note, scratch);
    drop(first);
    // A window past the retention: one second of retention plus a
    // tick's slack, named here as 2.5 s.
    std::thread::sleep(Duration::from_millis(2500));
    let mut late = Session::attach(&daemon, PROTOCOL_VERSION, Some(token));
    assert_eq!(
        late.where_am_i(),
        (scratch, 0),
        "a token past its retention window resumes nothing"
    );
}

/// With the knob at zero nothing is retained: the token presented at
/// once resumes nothing.
#[test]
fn e9_1_zero_retention_keeps_no_view() {
    let daemon = daemon_with_note("pmacs.config.set('session.view-retention-secs', 0)\n");
    let mut first = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let token = first.token.expect("token");
    let (scratch, _) = first.where_am_i();
    first.open_note(scratch);
    drop(first);
    let mut next = Session::attach(&daemon, PROTOCOL_VERSION, Some(token));
    assert_eq!(next.where_am_i(), (scratch, 0));
}

/// A v27 semantic session that sends anything but `Resume` where its
/// step is due is refused as a protocol error, so a frontend built on a
/// stale handshake cannot be served out of step.
#[test]
fn e9_1_a_v27_session_that_skips_its_resume_step_is_refused() {
    let daemon = daemon_with_note("");
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("read timeout");
    let hello: Hello = read_message(&mut stream).expect("read Hello");
    let fid = hello.assigned_frontend_id;
    write_message(
        &mut stream,
        &AttachRequest {
            protocol_version: PROTOCOL_VERSION,
            frontend_capabilities: semantic_caps(),
            initial_size: CellSize::new(24, 80),
        },
    )
    .expect("AttachRequest");
    write_message(&mut stream, &SessionBootstrapRequest::default()).expect("bootstrap");
    write_message(&mut stream, &FrontendEvent::FocusGained(fid))
        .expect("a key where Resume was due");
    let answer: InstanceMessage = read_message(&mut stream).expect("the daemon's answer");
    assert!(
        matches!(
            answer,
            InstanceMessage::Goodbye(GoodbyeReason::ProtocolError)
        ),
        "the daemon refuses the out-of-step session; got {answer:?}"
    );
}

/// `Resume` is a handshake step and nothing else: one sent on an
/// established session's event stream is dropped, and the session is
/// still served afterwards.
#[test]
fn e9_1_a_resume_after_the_handshake_is_dropped_and_the_session_served() {
    let daemon = daemon_with_note("");
    let mut first = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let token = first.token.expect("token");
    let (scratch, _) = first.where_am_i();
    let note = first.open_note(scratch);
    drop(first);
    let mut second = Session::attach(&daemon, PROTOCOL_VERSION, None);
    assert_eq!(second.where_am_i(), (scratch, 0));
    write_message(
        &mut second.stream,
        &FrontendEvent::Resume {
            frontend_id: second.fid,
            token: Some(token),
        },
    )
    .expect("a late Resume");
    // Served still, and not moved: the note's retained view stays
    // retained for a session that presents the token in its handshake.
    second.type_str("z");
    second.wait("the typed character echoed", |m| {
        matches!(m, InstanceMessage::CursorByte { buffer_id, byte_pos: 1 } if *buffer_id == scratch)
            .then_some(())
    });
    let mut resumed = Session::attach(&daemon, PROTOCOL_VERSION, Some(token));
    assert_eq!(
        resumed.where_am_i().0,
        note,
        "the retained view was not consumed by the late Resume"
    );
}
