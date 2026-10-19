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
use std::path::Path;
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

#[path = "common/editor.rs"]
mod editor;
use editor::eval;
use pmacs::editor::EditorState;

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

impl Session {
    /// The socket goes with no `Detach`, as a lost window's does, and
    /// this returns once the daemon has handled the loss: its detach
    /// hook logged this session's id, after the view was retained. A
    /// token presented after that names a retained view; presented
    /// before it, it names a session the daemon still holds, and the
    /// new socket is refused (E9 fix round 3, review 3's Medium 1). The
    /// row waits on the loss itself, never on the resume it wants.
    fn lose(self, daemon: &TestDaemon) {
        let id = self.fid.0.to_string();
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
        drop(self);
        common::ready::expect_true(
            &format!("the daemon's detach arm for frontend {id}"),
            Duration::from_secs(10),
            || detached(daemon).contains(&id),
            || format!("detached: {:?}", detached(daemon)),
        );
    }
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
/// the note and moved its cursor loses its socket; once the daemon has
/// handled the loss, the next socket presenting the token starts on the
/// note at that cursor, presented once, while a socket presenting
/// nothing starts on LOCAL's document, and the consumed token resumes
/// nothing a second time.
///
/// E9 fix round 3 (review 3): `f991f7b` made this row present the token
/// again and again until the note came back, each miss leaving as a
/// window does, which no window does (the GUI took the first session it
/// got), so the row could not show that a real reconnect waits for the
/// old session's loss. It now waits for that loss itself, through the
/// daemon's detach hook ([`Session::lose`]), and presents the token
/// once: a retained-view unit of behavior. A token presented before the
/// loss is handled is refused, which
/// `e9_review1_a_token_presented_while_its_session_lives_resumes_nothing`
/// and review 3's GPU rows hold.
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
    first.lose(&daemon);

    // A fresh socket, nothing presented: LOCAL's document, as before E9.
    let mut fresh = Session::attach(&daemon, PROTOCOL_VERSION, None);
    assert_eq!(
        fresh.where_am_i(),
        (scratch, 0),
        "a fresh view is LOCAL's document"
    );

    // The token, once: the note, at the cursor the first session left.
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
    resumed.lose(&daemon);
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

/// E9 fix round 2 (review 2's Low 5) --- a window whose view expired
/// before it got back (the GUI's thirty attempts run to 751.5 s, past
/// the default 600 s retention), or that reached a restarted daemon,
/// starts on a fresh view and is told so on the status line its first
/// frame carries: until this round it landed somewhere else in silence.
#[test]
fn e9_1_a_window_whose_view_expired_is_told_it_starts_fresh() {
    let daemon = daemon_with_note("pmacs.config.set('session.view-retention-secs', 1)\n");
    let mut first = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let token = first.token.expect("token");
    let (scratch, _) = first.where_am_i();
    let note = first.open_note(scratch);
    assert_ne!(note, scratch);
    drop(first);
    std::thread::sleep(Duration::from_millis(2500));
    let mut late = Session::attach(&daemon, PROTOCOL_VERSION, Some(token));
    // The status first: a wait drops what it does not match, and the
    // first frame after the viewport's declaration carries it. The
    // cursor, which every tick repeats, after.
    let said = late.wait("the status saying the view is gone", |m| match m {
        InstanceMessage::StatusFacts {
            message: Some(message),
            ..
        } if message.contains("earlier view is gone") => Some(message.clone()),
        _ => None,
    });
    assert_eq!(
        late.where_am_i(),
        (scratch, 0),
        "a fresh view, not the note"
    );
    assert!(
        said.contains("kept 1 s") && said.contains("starting fresh"),
        "the status names the retention and what happened: {said:?}"
    );
}

/// With the knob at zero nothing is retained: the token presented once
/// the loss is handled resumes nothing. (Presented before it, the token
/// would name a live session and be refused, since E9 fix round 3, so
/// the row waits for the loss: `Session::lose`.)
#[test]
fn e9_1_zero_retention_keeps_no_view() {
    let daemon = daemon_with_note("pmacs.config.set('session.view-retention-secs', 0)\n");
    let mut first = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let token = first.token.expect("token");
    let (scratch, _) = first.where_am_i();
    first.open_note(scratch);
    first.lose(&daemon);
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

/// E9 fix round 2 (review 2's Low 4) --- the retention knob says whose
/// view it keeps, as `describe-variable` shows it: a frontend whose
/// socket was lost, since a frontend that detaches or quits keeps
/// nothing (fix round 1), and that a GUI window closed with `C-x C-c`
/// while lost is one of those, the daemon having heard no `Detach`. The
/// text said "a detached frontend's window" until this round.
#[test]
fn e9_1_the_retention_knob_says_it_keeps_a_lost_socket_s_view() {
    let editor = EditorState::new_with_roots(&common::iso::roots());
    let description: String = eval(
        &editor,
        "return pmacs.config.describe('session.view-retention-secs').description",
    );
    assert!(
        description.contains("a frontend whose socket was lost"),
        "{description}"
    );
    assert!(
        description.contains("a frontend that detaches or quits keeps nothing"),
        "{description}"
    );
    assert!(
        description.contains("closed with C-x C-c while lost counts as lost"),
        "{description}"
    );
    assert!(
        !description.contains("a detached frontend's window"),
        "{description}"
    );
}

// ---------------------------------------------------------------------------
// E9.2 --- GPU reconnect
// ---------------------------------------------------------------------------

/// A Unix-socket proxy the test can cut. It listens at `path`, forwards
/// each accepted connection to `target` through two copy threads, and
/// keeps every stream it opened so [`Proxy::cut`] can shut them all:
/// the daemon then sees EOF on its socket (a detach that retains the
/// view) and the frontend its reader's `Disconnected`, while the daemon
/// itself is untouched. A later connection is forwarded afresh.
struct Proxy {
    path: std::path::PathBuf,
    held: std::sync::Arc<std::sync::Mutex<Vec<UnixStream>>>,
}

impl Proxy {
    fn start(target: &Path, dir: &Path) -> Self {
        let path = dir.join("proxy.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind the proxy");
        let held = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let target = target.to_path_buf();
        let keep = std::sync::Arc::clone(&held);
        std::thread::spawn(move || {
            for accepted in listener.incoming() {
                let Ok(front) = accepted else { break };
                let Ok(back) = UnixStream::connect(&target) else {
                    break;
                };
                let (mut f_read, mut f_write) = (
                    front.try_clone().expect("clone"),
                    front.try_clone().expect("clone"),
                );
                let (mut b_read, mut b_write) = (
                    back.try_clone().expect("clone"),
                    back.try_clone().expect("clone"),
                );
                keep.lock().expect("held").push(front);
                keep.lock().expect("held").push(back);
                std::thread::spawn(move || {
                    let _ = std::io::copy(&mut f_read, &mut b_write);
                    let _ = b_write.shutdown(std::net::Shutdown::Write);
                });
                std::thread::spawn(move || {
                    let _ = std::io::copy(&mut b_read, &mut f_write);
                    let _ = f_write.shutdown(std::net::Shutdown::Write);
                });
            }
        });
        Self { path, held }
    }

    /// Shut every stream forwarded so far, both ways, at once.
    fn cut(&self) {
        for stream in self.held.lock().expect("held").drain(..) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// The `pmacs-gpu` binary beside this build's `pmacs`, or `None` when
/// it is not built, which panics under `PMACS_REQUIRE_GPU` so the gate's
/// GPU leg never passes vacuously.
fn gpu_probe_binary() -> Option<std::path::PathBuf> {
    let binary = Path::new(env!("CARGO_BIN_EXE_pmacs"))
        .parent()
        .expect("test binary directory")
        .join("pmacs-gpu");
    if binary.exists() {
        return Some(binary);
    }
    assert!(
        std::env::var_os("PMACS_REQUIRE_GPU").is_none(),
        "PMACS_REQUIRE_GPU is set but {} is not built; build the workspace first",
        binary.display()
    );
    eprintln!("skipping the GPU probe: {} is not built", binary.display());
    None
}

/// A finished headless probe's report as `key=value` facts, or `None`
/// when it had no wgpu adapter (status 3), which panics under
/// `PMACS_REQUIRE_GPU`; any other failure panics with the probe's
/// stderr and `context`.
fn probe_facts(
    output: &std::process::Output,
    report: &Path,
    context: &str,
) -> Option<std::collections::HashMap<String, String>> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let no_adapter = output.status.code() == Some(3);
        assert!(
            no_adapter && std::env::var_os("PMACS_REQUIRE_GPU").is_none(),
            "headless GPU probe failed (status {:?}, {context}):\n{stderr}",
            output.status.code()
        );
        eprintln!("skipping the GPU probe: no wgpu adapter available");
        return None;
    }
    // The probe's own log, shown by libtest when the row fails.
    eprintln!("probe stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    let text = std::fs::read_to_string(report).expect("read the probe report");
    Some(
        text.lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
    )
}

/// Run `pmacs-gpu --headless-probe` under the reconnect action against
/// `socket`, cutting `proxy` once the probe says it has typed, and
/// return the report's facts. `None` when skipped (no `pmacs-gpu`
/// binary or no wgpu adapter), which panics under `PMACS_REQUIRE_GPU`
/// so the gate's GPU leg never passes vacuously.
fn run_reconnect_probe(
    socket: &Path,
    proxy: &Proxy,
    dir: &Path,
    text: &str,
    command: Option<&str>,
) -> Option<std::collections::HashMap<String, String>> {
    let binary = gpu_probe_binary()?;
    let report = dir.join("reconnect.report");
    let mut ready = report.as_os_str().to_owned();
    ready.push(".ready");
    let ready = std::path::PathBuf::from(ready);
    let mut child = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(socket)
        .arg(&report)
        .env("PMACS_GPU_PROBE_TYPE_TEXT", text)
        .env("PMACS_GPU_PROBE_ACTION", "reconnect")
        .env("PMACS_GPU_PROBE_COMMAND", command.unwrap_or(""))
        .env("PMACS_GPU_PROBE_DEADLINE_MS", "60000")
        // The schedule's test curve: 20, 40, 80, ... ms, so the attempts
        // land inside the row rather than the production half second.
        .env(pmacs::attach_reconnect::PMACS_TEST_BACKOFF_SCALE_MS, "20")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn the headless GPU probe");
    // The probe says when its typing has been echoed; then the socket
    // goes.
    let deadline = Instant::now() + Duration::from_mins(1);
    while !ready.exists() && Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let cut_at = if ready.exists() {
        proxy.cut();
        Some(Instant::now())
    } else {
        None
    };
    let output = child
        .wait_with_output()
        .expect("wait for the headless GPU probe");
    let facts = probe_facts(&output, &report, &format!("cut at {cut_at:?}"))?;
    assert!(
        cut_at.is_some(),
        "the probe never wrote its ready marker; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(facts)
}

fn fact<'a>(facts: &'a std::collections::HashMap<String, String>, key: &str) -> &'a str {
    facts.get(key).map_or_else(
        || panic!("the report has no {key}; facts: {facts:?}"),
        String::as_str,
    )
}

/// The window loses its socket and gets its view back: the document
/// stays on screen with the band saying it is reconnecting, the chrome
/// is gone and keys are dropped meanwhile; the reconnect on the shared
/// schedule presents the token and lands on the same buffer at the same
/// cursor with a token of its own; and an op typed at the close reaches
/// the daemon through the close's drain.
#[test]
fn e9_2_the_gpu_reconnects_with_a_banner_and_resumes_its_view() {
    let daemon = daemon_with_note("");
    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::set_permissions(
        dir.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .expect("chmod");
    let proxy = Proxy::start(daemon.socket_path(), dir.path());
    let Some(facts) = run_reconnect_probe(&proxy.path, &proxy, dir.path(), "abc", Some("e9.open"))
    else {
        return;
    };
    eprintln!(
        "daemon log:\n{}",
        std::fs::read_to_string({
            let mut log = daemon.socket_path().as_os_str().to_os_string();
            log.push(".stderr.log");
            log
        })
        .unwrap_or_default()
    );
    eprintln!("facts: {facts:?}");
    assert_eq!(fact(&facts, "phase.done"), "true", "{facts:?}");
    assert_eq!(fact(&facts, "lost"), "true");
    assert!(
        fact(&facts, "banner.lost")
            .starts_with("\"(daemon closed the connection without a Goodbye); reconnecting in "),
        "the band names the loss and the next attempt: {}",
        fact(&facts, "banner.lost")
    );
    assert_eq!(
        fact(&facts, "text.during"),
        fact(&facts, "text.before"),
        "the document stays on screen while reconnecting"
    );
    assert_eq!(fact(&facts, "chrome.cleared"), "true");
    assert_eq!(fact(&facts, "client.dropped"), "true");
    assert_eq!(fact(&facts, "key.while.lost.dropped"), "true");
    assert_eq!(fact(&facts, "reconnected"), "true");
    assert!(
        fact(&facts, "attempts").parse::<u32>().expect("attempts") >= 1,
        "{facts:?}"
    );
    assert_eq!(
        fact(&facts, "banner.after"),
        "\"\"",
        "the banner goes with the reconnect"
    );
    assert_eq!(
        fact(&facts, "buffer.after"),
        fact(&facts, "buffer.before"),
        "the resumed session is on the note, not LOCAL's document"
    );
    assert_eq!(fact(&facts, "cursor.after"), fact(&facts, "cursor.before"));
    assert_eq!(fact(&facts, "text.after"), fact(&facts, "text.before"));
    assert_eq!(fact(&facts, "token.changed"), "true");
    assert_eq!(fact(&facts, "closed"), "true");
    assert_eq!(
        fact(&facts, "text.closing"),
        r#""abc!one\ntwo\n""#,
        "the mirror holds the character typed at the close"
    );

    // The op in flight at the close reached the daemon: a fresh session
    // reads the note with the typed text, the `!` included.
    let note = fact(&facts, "buffer.before")
        .parse::<u64>()
        .expect("buffer id");
    let mut reader = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let (scratch, _) = reader.where_am_i();
    reader.open_note(scratch);
    let snapshot = reader.wait("the note's snapshot", |m| match m {
        InstanceMessage::BufferSnapshot {
            buffer_id,
            crdt_snapshot,
        } if buffer_id.raw() == note => Some(crdt_snapshot.clone()),
        _ => None,
    });
    let mirror = pmacs::crdt::CrdtState::new(reader.fid.0).expect("CrdtState::new");
    mirror.import_snapshot(&snapshot).expect("import");
    let text = mirror.materialize_string();
    assert!(
        text.starts_with("abc!"),
        "the op typed at the close was drained before the socket closed; the daemon holds {text:?}"
    );
}

/// E9 fix round 3 --- a forwarding socket for review 3's ordering with
/// the window's last input in flight. The first connection forwards
/// both ways until [`InflightProxy::hold`], after which what the window
/// writes is kept back; at [`InflightProxy::cut`] the window's side
/// closes at once, the kept input is forwarded to the daemon only once
/// the window's first reconnect has connected through the proxy, and
/// the old socket's EOF follows `eof_delay` later. Later connections
/// forward both ways afresh. The timeline is kept for the failure text.
struct InflightProxy {
    path: std::path::PathBuf,
    first_front: std::sync::Arc<std::sync::Mutex<Option<UnixStream>>>,
    holding: std::sync::Arc<std::sync::atomic::AtomicBool>,
    kept: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    timeline: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl InflightProxy {
    fn start(target: &Path, dir: &Path, eof_delay: Duration) -> Self {
        use std::io::{Read as _, Write as _};
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        let path = dir.join("inflight.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind the proxy");
        let target = target.to_path_buf();
        let first_front = Arc::new(Mutex::new(None));
        let holding = Arc::new(AtomicBool::new(false));
        let kept = Arc::new(Mutex::new(Vec::new()));
        let timeline = Arc::new(Mutex::new(Vec::new()));
        let accepted = Arc::new(AtomicUsize::new(0));
        let started = Instant::now();
        let mark = {
            let timeline = Arc::clone(&timeline);
            move |what: String| {
                timeline
                    .lock()
                    .expect("timeline")
                    .push(format!("{:>6} ms {what}", started.elapsed().as_millis()));
            }
        };
        let (front_slot, hold, keep) = (
            Arc::clone(&first_front),
            Arc::clone(&holding),
            Arc::clone(&kept),
        );
        std::thread::spawn(move || {
            let mut later = Vec::new();
            for incoming in listener.incoming() {
                let Ok(front) = incoming else { break };
                let Ok(back) = UnixStream::connect(&target) else {
                    break;
                };
                let index = accepted.fetch_add(1, Ordering::SeqCst);
                mark(format!("connection {index} accepted"));
                let mut f_read = front.try_clone().expect("clone");
                let mut f_write = front.try_clone().expect("clone");
                let mut b_read = back.try_clone().expect("clone");
                let mut b_write = back.try_clone().expect("clone");
                std::thread::spawn(move || {
                    let _ = std::io::copy(&mut b_read, &mut f_write);
                    let _ = f_write.shutdown(std::net::Shutdown::Write);
                });
                if index > 0 {
                    std::thread::spawn(move || {
                        let _ = std::io::copy(&mut f_read, &mut b_write);
                        let _ = b_write.shutdown(std::net::Shutdown::Write);
                    });
                    later.push((front, back));
                    continue;
                }
                *front_slot.lock().expect("front") = Some(front);
                let (hold, keep, accepted, mark) = (
                    Arc::clone(&hold),
                    Arc::clone(&keep),
                    Arc::clone(&accepted),
                    mark.clone(),
                );
                std::thread::spawn(move || {
                    let _keep_back_open = back;
                    let mut buf = [0u8; 8192];
                    loop {
                        match f_read.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) if hold.load(Ordering::SeqCst) => {
                                keep.lock().expect("kept").extend_from_slice(&buf[..n]);
                            }
                            Ok(n) => {
                                if b_write.write_all(&buf[..n]).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    mark("the window's side closed".to_owned());
                    let deadline = Instant::now() + Duration::from_secs(10);
                    while accepted.load(Ordering::SeqCst) < 2 && Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    let input = std::mem::take(&mut *keep.lock().expect("kept"));
                    let _ = b_write.write_all(&input);
                    mark(format!("{} bytes in flight forwarded", input.len()));
                    std::thread::sleep(eof_delay);
                    mark("the old socket's EOF forwarded".to_owned());
                    let _ = b_write.shutdown(std::net::Shutdown::Write);
                });
            }
        });
        Self {
            path,
            first_front,
            holding,
            kept,
            timeline,
        }
    }

    /// Keep back what the window writes on the first connection.
    fn hold(&self) {
        self.holding
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Bytes kept back so far.
    fn kept_len(&self) -> usize {
        self.kept.lock().expect("kept").len()
    }

    /// Close the window's side of the first connection, both ways.
    fn cut(&self) {
        if let Some(front) = self.first_front.lock().expect("front").as_ref() {
            let _ = front.shutdown(std::net::Shutdown::Both);
        }
    }

    fn timeline(&self) -> Vec<String> {
        self.timeline.lock().expect("timeline").clone()
    }
}

/// Run `pmacs-gpu --headless-probe` under the reconnect action through
/// `proxy` with `XY` typed in flight: once the probe says `abc` echoed,
/// the proxy holds what the window writes and says so; once the probe
/// says it typed `XY` and the proxy has kept its bytes, steady for
/// 300 ms, the window's side is cut. Returns the report's facts and the
/// proxy's timeline, `None` when skipped (as [`run_reconnect_probe`]).
fn run_inflight_probe(
    proxy: &InflightProxy,
    dir: &Path,
) -> Option<(std::collections::HashMap<String, String>, Vec<String>)> {
    let binary = gpu_probe_binary()?;
    let report = dir.join("inflight.report");
    let marker = |suffix: &str| {
        let mut path = report.as_os_str().to_owned();
        path.push(suffix);
        std::path::PathBuf::from(path)
    };
    let mut child = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(&proxy.path)
        .arg(&report)
        .env("PMACS_GPU_PROBE_ACTION", "reconnect")
        .env("PMACS_GPU_PROBE_TYPE_TEXT", "abc")
        .env("PMACS_GPU_PROBE_COMMAND", "e9.open")
        .env("PMACS_GPU_PROBE_INFLIGHT_TEXT", "XY")
        .env("PMACS_GPU_PROBE_DEADLINE_MS", "30000")
        // The production half-second first delay, as review 3's rows.
        .env_remove(pmacs::attach_reconnect::PMACS_TEST_BACKOFF_SCALE_MS)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn the headless GPU probe");
    // Typed and echoed: hold the window's writes, and say so.
    let deadline = Instant::now() + Duration::from_secs(40);
    let mut stage = "ready";
    let mut cut = false;
    let mut kept_since: Option<(usize, Instant)> = None;
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        match stage {
            "ready" if marker(".ready").exists() => {
                proxy.hold();
                std::fs::write(marker(".hold"), "holding\n").expect("write the hold marker");
                stage = "inflight";
            }
            // `XY` typed: cut once the proxy has kept its bytes and no
            // more have come for 300 ms.
            "inflight" if marker(".inflight").exists() => {
                let len = proxy.kept_len();
                match kept_since {
                    Some((seen, since)) if seen == len && len > 0 => {
                        if since.elapsed() >= Duration::from_millis(300) {
                            proxy.cut();
                            cut = true;
                            stage = "cut";
                        }
                    }
                    _ => kept_since = Some((len, Instant::now())),
                }
            }
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if child.try_wait().expect("poll the probe").is_none() {
        let _ = child.kill();
    }
    let output = child.wait_with_output().expect("reap the probe");
    let timeline = proxy.timeline();
    let facts = probe_facts(&output, &report, &format!("stage {stage}; {timeline:?}"))?;
    eprintln!("facts: {facts:?}\nproxy: {timeline:#?}");
    assert!(cut, "the probe never typed in flight; {timeline:?}");
    Some((facts, timeline))
}

/// E9 fix round 3 (review 3's Medium 1, the owner's ruling of
/// 2026-10-19) --- the reason for refusal and retry: input the old
/// socket carried before its delayed EOF lands in the view the window
/// resumes. The window types `abc` on the note, then `XY` while the
/// proxy keeps back what it writes, so no daemon has seen `XY` when the
/// window loses its socket. The kept input reaches the daemon only once
/// the window's first reconnect (500 ms, the production schedule) has
/// connected, and the old socket's EOF 1.5 s after that: the first
/// attempt finds the old session still registered and is refused, the
/// band saying the daemon closed it before accepting it; the old
/// session takes `XY` into its view; its loss retains that view; and a
/// later attempt resumes it, at the cursor after `XY`, where the next
/// key lands. At `f991f7b` the first attempt was accepted as a fresh
/// view and the window resumed nowhere near `XY`; under a takeover
/// (the policy the owner declined) the view would have moved before the
/// input that was in flight for it arrived.
#[test]
fn e9_2_input_in_flight_at_a_loss_lands_in_the_view_the_window_resumes() {
    let daemon = daemon_with_note("");
    let dir = tempfile::TempDir::new().expect("tempdir");
    std::fs::set_permissions(
        dir.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .expect("chmod");
    let proxy = InflightProxy::start(
        daemon.socket_path(),
        dir.path(),
        Duration::from_millis(1500),
    );
    let Some((facts, timeline)) = run_inflight_probe(&proxy, dir.path()) else {
        return;
    };
    assert_eq!(fact(&facts, "phase.done"), "true", "{facts:?}");
    // Preconditions: typed on the note, `XY` on the window's mirror and
    // nowhere else when the socket went.
    assert_eq!(fact(&facts, "cursor.before"), "3", "{facts:?}");
    assert_eq!(fact(&facts, "text.before"), r#""abcone\ntwo\n""#);
    assert_eq!(fact(&facts, "text.inflight"), r#""abcXYone\ntwo\n""#);
    assert_eq!(fact(&facts, "lost"), "true");
    assert!(
        timeline
            .iter()
            .any(|line| line.contains("bytes in flight forwarded") && !line.contains(" 0 bytes")),
        "the window's in-flight input reached the daemon on the old socket: {timeline:?}"
    );
    // The first attempt met the live session and was refused; the band
    // says what the window saw, and counts the attempts made.
    let refused = fact(&facts, "refused").parse::<u32>().expect("refused");
    assert!(refused >= 1, "{facts:?}");
    assert!(
        fact(&facts, "banner.refused").contains(&format!(
            "reconnect attempt {refused} failed (the daemon closed the connection before \
             accepting the session)"
        )),
        "the band says what the window saw, after {refused} refused: {}",
        fact(&facts, "banner.refused")
    );
    assert_eq!(
        fact(&facts, "attempts"),
        (refused + 1).to_string(),
        "every attempt but the last was refused: {facts:?}"
    );
    // The view the window resumes holds the input that was in flight.
    assert_eq!(fact(&facts, "reconnected"), "true", "{facts:?}");
    assert_eq!(
        fact(&facts, "buffer.after"),
        fact(&facts, "buffer.before"),
        "{facts:?}"
    );
    assert_eq!(
        fact(&facts, "cursor.after"),
        "5",
        "the resumed cursor is after the in-flight XY: {facts:?}"
    );
    assert_eq!(fact(&facts, "text.after"), r#""abcXYone\ntwo\n""#);
    assert_eq!(
        fact(&facts, "text.closing"),
        r#""abcXY!one\ntwo\n""#,
        "the next key lands after the in-flight input"
    );
}

// ---------------------------------------------------------------------------
// E9.3 --- quit
// ---------------------------------------------------------------------------

impl Session {
    /// `C-x C-c`.
    /// Leave as a window closing does: `Detach`, then the socket goes.
    /// A departure the daemon is told of retains no view (E9, fix
    /// round 1); a socket that just goes is a loss, whose view is kept
    /// for `session.view-retention-secs`.
    fn close_as_a_window_does(mut self) {
        write_message(&mut self.stream, &FrontendEvent::Detach(self.fid)).expect("Detach");
    }

    fn quit_chord(&mut self) {
        self.key(Key::Char('x'), Modifiers::CTRL);
        self.key(Key::Char('c'), Modifiers::CTRL);
    }

    /// The session's close: `Goodbye(reason)`, or the daemon's EOF.
    fn ended(&mut self) -> Option<GoodbyeReason> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "the session did not end");
            match self.rx.recv_timeout(remaining) {
                Ok(InstanceMessage::Goodbye(reason)) => return Some(reason),
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return None,
            }
        }
    }
}

/// `C-x C-c` from one of two attached frontends detaches that frontend
/// alone: it is told `Goodbye(Detached)`, the other is served on, the
/// daemon lives, and the quitter's view is dropped, not retained: its
/// window closes on the `Goodbye`, so its token dies with it and a
/// session presenting the token starts fresh (review 1's Medium 2;
/// until fix round 1 this row read "retained for its token like any
/// detach's", the state the review found no one could resume).
#[test]
fn e9_3_quit_from_an_attached_frontend_detaches_it_and_nothing_else() {
    let mut daemon = daemon_with_note("");
    let mut first = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let token = first.token.expect("token");
    let (scratch, _) = first.where_am_i();
    let note = first.open_note(scratch);
    let mut second = Session::attach(&daemon, PROTOCOL_VERSION, None);
    second.where_am_i();

    first.quit_chord();
    assert_eq!(
        first.ended(),
        Some(GoodbyeReason::Detached),
        "the quitter is told it was detached"
    );
    // The other is served still, and the daemon is up.
    second.type_str("k");
    second.wait("the other frontend's echo", |m| {
        matches!(m, InstanceMessage::CursorByte { byte_pos: 1, .. }).then_some(())
    });
    assert!(
        daemon.is_alive(),
        "a frontend's quit does not stop the daemon"
    );
    // The quitter left on purpose: nothing is retained for its token.
    let mut back = Session::attach(&daemon, PROTOCOL_VERSION, Some(token));
    assert_eq!(
        back.where_am_i(),
        (scratch, 0),
        "a quit retains no view; the note ({note:?}) is not resumed"
    );
}

/// The last frontend attached is asked whether the daemon should stop
/// with it: `n` detaches it and the daemon lives; `y` stops the daemon,
/// which tells the quitter `Detached`, as `n` does, and exits (since
/// E9's fix round 2; it told it `ShuttingDown` before).
#[test]
fn e9_3_the_last_frontend_is_asked_and_its_answer_decides_the_daemon() {
    let mut daemon = daemon_with_note("");
    let mut only = Session::attach(&daemon, PROTOCOL_VERSION, None);
    only.where_am_i();
    only.quit_chord();
    let prompt = only.wait("the question", |m| match m {
        InstanceMessage::MinibufferPromptRows { prompt, .. } => prompt.clone(),
        _ => None,
    });
    assert!(
        prompt.contains("stop the daemon"),
        "the last frontend is asked about the daemon: {prompt:?}"
    );
    only.key(Key::Char('n'), Modifiers::NONE);
    assert_eq!(only.ended(), Some(GoodbyeReason::Detached), "n detaches");
    assert!(daemon.is_alive(), "n leaves the daemon up");

    let mut last = Session::attach(&daemon, PROTOCOL_VERSION, None);
    last.where_am_i();
    last.quit_chord();
    last.wait("the question again", |m| match m {
        InstanceMessage::MinibufferPromptRows { prompt, .. }
            if prompt
                .as_deref()
                .is_some_and(|p| p.contains("stop the daemon")) =>
        {
            Some(())
        }
        _ => None,
    });
    last.key(Key::Char('y'), Modifiers::NONE);
    // Changed at E9's fix round 2 with the owner's ruling on review 1's
    // Medium 4: the quitter is told `Detached`, as on `n`, so its window
    // closes silently; until then it was told `ShuttingDown` and the GPU
    // window stayed open under E2.3's notice. The daemon still stops.
    assert_eq!(
        last.ended(),
        Some(GoodbyeReason::Detached),
        "y stops the daemon and closes the quitter's window as n does"
    );
    let status = exit_within(&mut daemon, 10);
    assert!(status.success(), "a clean stop: {status:?}");
}

/// E9 fix round 2 (the owner's ruling on review 1's Medium 4) --- the
/// last GPU window's quit closes it, as its `n` would: through the
/// production `App` (the headless probe's `quit` action) `C-x C-c` is
/// asked "stop the daemon too?", `y` stops the daemon, and the window is
/// told `Detached` and asks to exit. Until this round the daemon told it
/// `ShuttingDown` and the window stayed open under E2.3's notice, a
/// window to close by hand after the quit the user asked for.
#[test]
fn e9_3_the_last_gpu_window_s_y_closes_it_and_stops_the_daemon() {
    let mut daemon = daemon_with_note("");
    let dir = tempfile::TempDir::new().expect("tempdir");
    let Some(facts) = run_quit_probe(daemon.socket_path(), dir.path()) else {
        return;
    };
    assert_eq!(
        fact(&facts, "asked"),
        "true",
        "the question reached the window: {facts:?}"
    );
    assert_eq!(
        fact(&facts, "goodbye"),
        "Some(Detached)",
        "the quitter is told it was detached: {facts:?}"
    );
    assert_eq!(
        fact(&facts, "exit_requested"),
        "true",
        "the window closes, not left open under a notice: {facts:?}"
    );
    let status = exit_within(&mut daemon, 10);
    assert!(status.success(), "y stopped the daemon cleanly: {status:?}");
}

/// Run `pmacs-gpu --headless-probe` under the quit action against
/// `socket` and return the report's facts; `None` when skipped, as
/// [`run_reconnect_probe`] is.
fn run_quit_probe(socket: &Path, dir: &Path) -> Option<std::collections::HashMap<String, String>> {
    let binary = gpu_probe_binary()?;
    let report = dir.join("quit.report");
    let output = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(socket)
        .arg(&report)
        .env("PMACS_GPU_PROBE_ACTION", "quit")
        .env("PMACS_GPU_PROBE_DEADLINE_MS", "30000")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("run the headless GPU probe");
    probe_facts(&output, &report, "the quit probe")
}

/// A v26 frontend's quit detaches it too, told nothing it could not
/// decode: its socket closes without a `Goodbye`.
#[test]
fn e9_3_a_v26_frontend_s_quit_detaches_it_with_no_goodbye() {
    let mut daemon = daemon_with_note("");
    let mut legacy = Session::attach(&daemon, SESSION_MIN_VERSION - 1, None);
    legacy.where_am_i();
    let mut other = Session::attach(&daemon, PROTOCOL_VERSION, None);
    other.where_am_i();
    legacy.quit_chord();
    assert_eq!(
        legacy.ended(),
        None,
        "closed, with no Goodbye a v26 peer could not decode"
    );
    assert!(daemon.is_alive());
    other.type_str("k");
    other.wait("the other frontend's echo", |m| {
        matches!(m, InstanceMessage::CursorByte { byte_pos: 1, .. }).then_some(())
    });
}

// ---------------------------------------------------------------------------
// E9.4 --- lifecycle
// ---------------------------------------------------------------------------

/// The daemon's exit status, waited for with a bound: a daemon that does
/// not exit within `secs` fails the row rather than hanging it.
fn exit_within(daemon: &mut TestDaemon, secs: u64) -> std::process::ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while daemon.is_alive() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !daemon.is_alive(),
        "the daemon did not exit within {secs} s"
    );
    daemon.wait_for_exit().expect("the daemon's exit status")
}

/// `pmacs --daemon <action> --socket <path>`: its status, stdout and stderr.
fn daemon_cli(action: &str, socket: &Path) -> (std::process::ExitStatus, String, String) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pmacs"))
        .arg("--daemon")
        .arg(action)
        .arg("--socket")
        .arg(socket)
        .output()
        .expect("run pmacs --daemon");
    (
        output.status,
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// `--status` reports the daemon at the socket from its `Hello`, and
/// `--stop` stops it over the socket: every attached frontend is told
/// it is shutting down, the process exits cleanly, the socket file is
/// gone, and `--status` then says so.
#[test]
fn e9_4_status_reports_and_stop_stops_over_the_socket() {
    let mut daemon = daemon_with_note("");
    let socket = daemon.socket_path().to_path_buf();
    let mut attached = Session::attach(&daemon, PROTOCOL_VERSION, None);
    attached.where_am_i();

    let (status, out, err) = daemon_cli("--status", &socket);
    assert!(status.success(), "status exit: {status:?}; {err}");
    assert!(
        out.contains(&format!("daemon at {}", socket.display()))
            && out.contains("protocol baseline v20")
            && out.contains("  up ")
            && out.contains("  cwd "),
        "the status names the daemon from its Hello:\n{out}"
    );

    let (status, out, err) = daemon_cli("--stop", &socket);
    assert!(status.success(), "stop exit: {status:?}; {err}");
    assert!(out.contains("stopped the daemon at"), "{out}");
    assert_eq!(
        attached.ended(),
        Some(GoodbyeReason::ShuttingDown),
        "the attached frontend is told"
    );
    let exit = exit_within(&mut daemon, 10);
    assert!(exit.success(), "a clean stop: {exit:?}");
    assert!(!socket.exists(), "the daemon removed its socket");

    let (status, _, err) = daemon_cli("--status", &socket);
    assert!(!status.success(), "no daemon: exit 1");
    assert!(err.contains("no daemon at"), "{err}");
}

/// A socket file nothing answers is a daemon's leftover: `--status`
/// says so and removes it, so the path is free for the next start; a
/// path that is not a socket is left alone and named.
#[test]
fn e9_4_a_stale_socket_is_removed_and_a_non_socket_left_alone() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let stale = dir.path().join("stale.sock");
    // A socket file whose listener is gone: bound, then dropped.
    drop(std::os::unix::net::UnixListener::bind(&stale).expect("bind"));
    assert!(stale.exists());
    let (status, _, err) = daemon_cli("--status", &stale);
    assert!(!status.success());
    assert!(
        err.contains("stale socket") && err.contains("removed"),
        "{err}"
    );
    assert!(!stale.exists(), "the stale socket file is gone");

    let plain = dir.path().join("plain.sock");
    std::fs::write(&plain, "not a socket").expect("write");
    let (status, _, err) = daemon_cli("--stop", &plain);
    assert!(!status.success());
    assert!(err.contains("is not a socket; left alone"), "{err}");
    assert!(plain.exists(), "a non-socket is never removed");
}

/// With `daemon.idle-exit-secs` set, a daemon no frontend is attached
/// to for that long stops on its own; by default it never does. The
/// frontend leaves as a window closes (`Detach`, then the socket): a
/// socket that just went would be a loss whose view is retained and
/// holds the clock for the retention (review 1's Medium 2, the row
/// `e9_review1_idle_exit_waits_while_a_lost_socket_s_view_is_retained`).
#[test]
fn e9_4_idle_exit_is_opt_in() {
    let mut daemon = daemon_with_note("pmacs.config.set('daemon.idle-exit-secs', 1)\n");
    let session = Session::attach(&daemon, PROTOCOL_VERSION, None);
    // Attached: no idle clock runs. A window before a negative
    // assertion, named with its length: 2 s, past the knob.
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        daemon.is_alive(),
        "an attached daemon stays up past the knob"
    );
    session.close_as_a_window_does();
    let deadline = Instant::now() + Duration::from_secs(10);
    while daemon.is_alive() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        exit_within(&mut daemon, 1).success(),
        "the idle daemon stopped within the window, cleanly"
    );

    let mut default = daemon_with_note("");
    let session = Session::attach(&default, PROTOCOL_VERSION, None);
    session.close_as_a_window_does();
    // Named window: 2 s with no frontend, the knob at its default.
    std::thread::sleep(Duration::from_secs(2));
    assert!(default.is_alive(), "by default an idle daemon stays up");
}

// ---------------------------------------------------------------------------
// E9.5 --- oversize snapshots
// ---------------------------------------------------------------------------

/// A daemon whose init defines `e9.open-big`, opening a file whose
/// snapshot is past `MAX_FRAME_BYTES` (twenty-four MiB of
/// pseudo-random printable text, which no snapshot encoding packs
/// under sixteen), and `e9.open-small`, a one-line file.
fn daemon_with_big_and_small() -> TestDaemon {
    let dir = tempfile::TempDir::new().expect("tempdir for the files");
    let big = dir.path().join("big.txt");
    let mut text = String::with_capacity(24 * 1024 * 1024 + 80);
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    while text.len() < 24 * 1024 * 1024 {
        // xorshift64: every byte a printable character, 95 of them.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        for byte in state.to_le_bytes() {
            text.push(char::from(b' ' + byte % 95));
        }
        if text.len() % 80 < 8 {
            text.push('\n');
        }
    }
    std::fs::write(&big, &text).expect("write the big file");
    let small = dir.path().join("small.txt");
    std::fs::write(&small, "small\n").expect("write the small file");
    let init = format!(
        "pmacs.command.define {{ name = 'e9.open-big', description = 'open the big file',\n\
           fn = function() pmacs.buffer.find_or_open({big:?}) end }}\n\
         pmacs.command.define {{ name = 'e9.open-small', description = 'open the small file',\n\
           fn = function() pmacs.buffer.find_or_open({small:?}) end }}\n",
        big = big.to_str().expect("utf-8"),
        small = small.to_str().expect("utf-8"),
    );
    let daemon = TestDaemon::spawn_with_config(&init);
    std::mem::forget(dir);
    daemon
}

/// A buffer whose snapshot is past the frame cap is refused with a
/// status every frontend reads, and the semantic frontend's window is
/// put back on the buffer it still mirrors: no snapshot of it arrives,
/// the band says what it weighs, and the daemon's cursor is back on
/// the document that was there. A buffer that fits arrives as before.
#[test]
fn e9_5_an_oversize_snapshot_is_refused_with_a_visible_status_not_skipped() {
    let daemon = daemon_with_big_and_small();
    let mut session = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let (scratch, _) = session.where_am_i();

    session.run_command("e9.open-big");
    // The refusal: the status names the weight and the window stays.
    let (message, seen) = {
        let mut seen = Vec::new();
        let message = session.wait("the refusal on the status line", |m| {
            seen.push(
                format!("{m:?}")
                    .split(['(', ' ', '{'])
                    .next()
                    .unwrap_or("")
                    .to_owned(),
            );
            match m {
                InstanceMessage::StatusFacts {
                    message: Some(message),
                    ..
                } if message.contains("a frontend can mirror") => Some(message.clone()),
                InstanceMessage::BufferSnapshot { buffer_id, .. } if *buffer_id != scratch => {
                    panic!("a snapshot of the oversize buffer {buffer_id:?} was sent")
                }
                _ => None,
            }
        });
        (message, seen)
    };
    assert!(
        message.contains("MiB, past the 16 MiB") && message.contains("this window stays on"),
        "the status says what weighed too much and where the window stayed: {message:?} (saw {seen:?})"
    );
    // E9 fix round 2 (review 2's Low 6): the size a user can square
    // with `ls` is the file's, 24 MiB; the snapshot past the frame is
    // named as the snapshot, never as the file.
    assert!(
        message.contains("big.txt is 24.0 MiB and its snapshot "),
        "the status names the file's own size and the snapshot's apart: {message:?}"
    );
    // The window is back on the document the frontend mirrors.
    session.type_str("k");
    session.wait("the echo on the kept document", |m| {
        matches!(m, InstanceMessage::CursorByte { buffer_id, byte_pos: 1 } if *buffer_id == scratch)
            .then_some(())
    });

    // Control: a buffer that fits arrives, and the window moves.
    session.run_command("e9.open-small");
    let small = session.wait("the small buffer's snapshot", |m| match m {
        InstanceMessage::BufferSnapshot { buffer_id, .. } if *buffer_id != scratch => {
            Some(*buffer_id)
        }
        _ => None,
    });
    session.wait("the cursor on the small buffer", |m| {
        matches!(m, InstanceMessage::CursorByte { buffer_id, .. } if *buffer_id == small)
            .then_some(())
    });
}
