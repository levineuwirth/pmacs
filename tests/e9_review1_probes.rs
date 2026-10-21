//! E9 review round 1 --- behavioral probes against the session rows,
//! through the production handshake and dispatch of a real daemon:
//! the states a user reaches that the handoff's witnesses did not
//! choose. Each row's doc comment says what it expects and whether it
//! fails at `e7c8fd5`, the reviewed head, and why; the rows that pass
//! there witness claims the handoff made by reading alone.
//!
//! The harness is `tests/e9_session_acceptance.rs`'s, repeated here so
//! the fix round can commit this file beside it unchanged.
#![cfg(feature = "crdt")]

mod common;

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pmacs_protocol::cell::CellSize;
use pmacs_protocol::message::{
    ADVERTISED_PROTOCOL_VERSION, AttachRequest, FrontendCapabilities, FrontendEvent, GoodbyeReason,
    Hello, InitialTarget, InitialTargetResult, InstanceMessage, Key, KeyEvent, Modifiers,
    PROTOCOL_VERSION, SessionBootstrapRequest,
};
use pmacs_protocol::transport::{read_message, write_message};
use pmacs_protocol::{
    BufferId, FrontendId, SESSION_MIN_VERSION, SESSION_TOKEN_BYTES, SessionToken,
};

use common::daemon::{NO_REOPEN, TestDaemon, build_default_caps};

struct Session {
    stream: UnixStream,
    rx: mpsc::Receiver<InstanceMessage>,
    fid: FrontendId,
    token: Option<SessionToken>,
    /// Every `BufferSnapshot` and every status message that passed
    /// through a wait, matched or not: a bootstrap writes its snapshots
    /// and its first status before the first `CursorByte`, which
    /// `where_am_i` consumes, so a row that asks what reached the
    /// session reads these rather than what came after.
    snapshots_seen: Vec<BufferId>,
    statuses_seen: Vec<String>,
}

fn semantic_caps() -> FrontendCapabilities {
    FrontendCapabilities {
        multi_frontend: true,
        crdt_replica: true,
        semantic_render: true,
        ..build_default_caps()
    }
}

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
    fn attach(daemon: &TestDaemon, offer: u32, resume: Option<SessionToken>) -> Self {
        let (stream, fid) = handshake(daemon, offer, resume);
        Self::from_stream(stream, fid, offer)
    }

    fn from_stream(stream: UnixStream, fid: FrontendId, offer: u32) -> Self {
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
            snapshots_seen: Vec::new(),
            statuses_seen: Vec::new(),
        };
        if offer >= SESSION_MIN_VERSION {
            let token = session.wait("the SessionToken", |m| match m {
                InstanceMessage::SessionToken(t) => Some(*t),
                _ => None,
            });
            session.token = Some(token);
        }
        let (document, _) = session.where_am_i();
        session.declare_viewport(document);
        session
    }

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

    fn wait<T>(&mut self, what: &str, mut pick: impl FnMut(&InstanceMessage) -> Option<T>) -> T {
        self.wait_for(what, Duration::from_secs(10), &mut pick)
            .unwrap_or_else(|seen| {
                panic!("timed out waiting for {what}; the daemon wrote {seen:?}")
            })
    }

    /// `wait` with its own window; `Err` names what came meanwhile.
    fn wait_for<T>(
        &mut self,
        what: &str,
        window: Duration,
        pick: &mut impl FnMut(&InstanceMessage) -> Option<T>,
    ) -> Result<T, Vec<String>> {
        let deadline = Instant::now() + window;
        let mut seen: Vec<String> = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(seen);
            }
            match self
                .rx
                .recv_timeout(remaining.min(Duration::from_millis(200)))
            {
                Ok(msg) => {
                    match &msg {
                        InstanceMessage::BufferSnapshot { buffer_id, .. } => {
                            self.snapshots_seen.push(*buffer_id);
                        }
                        InstanceMessage::StatusFacts {
                            message: Some(message),
                            ..
                        } => self.statuses_seen.push(message.clone()),
                        _ => {}
                    }
                    if let Some(found) = pick(&msg) {
                        return Ok(found);
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

    fn where_am_i(&mut self) -> (BufferId, u64) {
        self.wait("a CursorByte", |m| match m {
            InstanceMessage::CursorByte {
                buffer_id,
                byte_pos,
            } => Some((*buffer_id, *byte_pos)),
            _ => None,
        })
    }

    fn run_command(&mut self, name: &str) {
        self.key(Key::Char('x'), Modifiers::ALT);
        self.type_str(name);
        self.key(Key::Enter, Modifiers::NONE);
    }

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

    fn quit_chord(&mut self) {
        self.key(Key::Char('x'), Modifiers::CTRL);
        self.key(Key::Char('c'), Modifiers::CTRL);
    }

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

/// A daemon whose init defines `e9.open` on a note beside its socket,
/// plus `extra`; returns the daemon and the directory the note is in.
fn daemon_with_note_in(extra: &str) -> (TestDaemon, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().expect("tempdir for the note");
    let note = dir.path().join("note.txt");
    std::fs::write(&note, "one\ntwo\n").expect("write the note");
    // E9 fix round 3: the detach hook logs each frontend torn down, so
    // a row can wait for a loss to have been handled (`Session::lose`).
    let init = format!(
        "pmacs.command.define {{ name = 'e9.open', description = 'open the note',\n\
           fn = function() pmacs.buffer.find_or_open({path:?}) end }}\n{DETACH_LOG}{extra}",
        path = note.to_str().expect("utf-8 path"),
    );
    let daemon = TestDaemon::spawn_with_env_and_init(&[("PMACS_ATTACH_DEBUG", "1")], &init);
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);
    (daemon, path)
}

/// A `frontend.detached` hook that logs the departed id into the
/// daemon's isolated home (its `HOME` is the socket's directory); the
/// hook runs last in the detach arm and edits nothing.
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
    /// E9 fix round 3 --- the socket goes, and this returns once the
    /// daemon has handled it (the detach hook logged this session's
    /// id): a token presented sooner names a session the daemon still
    /// holds, and is refused (review 3's Medium 1, the owner's ruling).
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

/// E9 fix round 3 --- present `token` through the handshake on a new
/// socket and read what the daemon writes next: `None` when it closes
/// the socket with nothing written (the refusal of a token whose
/// session it still holds), else the first message's variant, with the
/// frontend id `Hello` assigned the attempt. A read that times out (ten
/// seconds) is neither, and fails the row.
fn answer_to(daemon: &TestDaemon, token: SessionToken) -> (FrontendId, Option<String>) {
    let (mut stream, fid) = handshake(daemon, PROTOCOL_VERSION, Some(token));
    let first = match read_message::<InstanceMessage>(&mut stream) {
        Ok(message) => Some(
            format!("{message:?}")
                .split(['(', ' ', '{'])
                .next()
                .unwrap_or("")
                .to_owned(),
        ),
        Err(pmacs_protocol::transport::TransportError::Eof) => None,
        Err(other) => panic!("the daemon neither answered nor closed the attempt: {other}"),
    };
    (fid, first)
}

fn daemon_with_note(extra: &str) -> TestDaemon {
    daemon_with_note_in(extra).0
}

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

/// An init that records every `editor.before-quit` run in `marker`.
fn before_quit_marker(marker: &Path) -> String {
    format!(
        "pmacs.hook.add('editor.before-quit', function()\n\
           local f = assert(io.open({marker:?}, 'a'))\n\
           f:write('before-quit\\n')\n\
           f:close()\n\
         end)\n",
        marker = marker.to_str().expect("utf-8 path"),
    )
}

// ---------------------------------------------------------------------------
// E9.4 --- `--stop` and the quit's hooks
// ---------------------------------------------------------------------------

/// `pmacs --daemon --stop` is the sanctioned stop, and its usage text
/// says the autosave's recovery copies cover what it does not ask
/// about. The autosave's last sweep of unsaved text, the desktop's
/// session save and saveplace all ride `editor.before-quit`, which the
/// Lua `editor.quit` runs and the `Shutdown` arm (`core.quit = true`
/// from Rust) does not. FAILS at `e7c8fd5`: the marker is never
/// written, so the stop drops up to `autosave.interval-ms` of unsaved
/// text and the desktop session with it.
#[test]
fn e9_review1_stop_runs_the_before_quit_hooks() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let marker = dir.path().join("before-quit.log");
    let mut daemon = daemon_with_note(&before_quit_marker(&marker));
    let socket = daemon.socket_path().to_path_buf();
    let mut attached = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let (scratch, _) = attached.where_am_i();
    let note = attached.open_note(scratch);
    attached.type_str("x");
    attached.wait("the edit echoed", |m| {
        matches!(m, InstanceMessage::CursorByte { buffer_id, byte_pos: 1 } if *buffer_id == note)
            .then_some(())
    });

    let (status, _, err) = daemon_cli("--stop", &socket);
    assert!(status.success(), "stop exit: {status:?}; {err}");
    assert_eq!(attached.ended(), Some(GoodbyeReason::ShuttingDown));
    let exit = exit_within(&mut daemon, 10);
    assert!(exit.success(), "a clean stop: {exit:?}");
    assert!(
        marker.exists(),
        "--stop ran editor.before-quit, so the autosave's last sweep, the desktop and \
         saveplace had their turn; nothing at {}",
        marker.display()
    );
}

/// Control for the row above: the last frontend's `y` is the Lua
/// quit, which runs the hooks. Passes at `e7c8fd5`. Its `Goodbye` read
/// changed at E9's fix round 2 with the owner's ruling on review 1's
/// Medium 4: the quitter is told `Detached`, as on `n`, not
/// `ShuttingDown`.
#[test]
fn e9_review1_the_last_frontend_s_y_runs_the_before_quit_hooks() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let marker = dir.path().join("before-quit.log");
    let mut daemon = daemon_with_note(&before_quit_marker(&marker));
    let mut only = Session::attach(&daemon, PROTOCOL_VERSION, None);
    only.where_am_i();
    only.quit_chord();
    only.wait("the question", |m| match m {
        InstanceMessage::MinibufferPromptRows { prompt, .. }
            if prompt
                .as_deref()
                .is_some_and(|p| p.contains("stop the daemon")) =>
        {
            Some(())
        }
        _ => None,
    });
    only.key(Key::Char('y'), Modifiers::NONE);
    assert_eq!(only.ended(), Some(GoodbyeReason::Detached));
    let exit = exit_within(&mut daemon, 10);
    assert!(exit.success(), "{exit:?}");
    assert!(marker.exists(), "y is the quit and runs its hooks");
}

/// The idle exit stops as `--stop` does, and the same hooks are owed
/// there. FAILS at `e7c8fd5` for the same reason as the stop row.
#[test]
fn e9_review1_idle_exit_runs_the_before_quit_hooks() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let marker = dir.path().join("before-quit.log");
    let mut daemon = daemon_with_note(&format!(
        "pmacs.config.set('daemon.idle-exit-secs', 1)\n\
         pmacs.config.set('session.view-retention-secs', 0)\n{}",
        before_quit_marker(&marker)
    ));
    let session = Session::attach(&daemon, PROTOCOL_VERSION, None);
    drop(session);
    let exit = exit_within(&mut daemon, 10);
    assert!(exit.success(), "{exit:?}");
    assert!(marker.exists(), "the idle exit ran editor.before-quit");
}

/// `Shutdown` from a session that negotiated v26 is refused: the
/// daemon lives and the session is served on. The handoff claims this
/// by reading ("refused at the dispatcher, the gate a literal"); no row
/// witnesses it. Passes at `e7c8fd5`.
#[test]
fn e9_review1_shutdown_from_a_v26_session_is_refused() {
    let mut daemon = daemon_with_note("");
    let mut legacy = Session::attach(&daemon, SESSION_MIN_VERSION - 1, None);
    let (scratch, _) = legacy.where_am_i();
    write_message(
        &mut legacy.stream,
        &FrontendEvent::Shutdown {
            frontend_id: legacy.fid,
        },
    )
    .expect("a Shutdown this crate can encode");
    // A window before the negative assertion, named: two seconds.
    std::thread::sleep(Duration::from_secs(2));
    assert!(daemon.is_alive(), "a v26 session cannot stop the daemon");
    legacy.type_str("k");
    legacy.wait("the session served on", |m| {
        matches!(m, InstanceMessage::CursorByte { buffer_id, byte_pos: 1 } if *buffer_id == scratch)
            .then_some(())
    });
}

// ---------------------------------------------------------------------------
// E9.1 / E9.4 --- retention against departure and the idle clock
// ---------------------------------------------------------------------------

/// A frontend that leaves on purpose --- the GPU window's close sends
/// `FrontendEvent::Detach` (`AttachClient::detach`) and then exits, so
/// its token dies with it --- has nothing to resume, yet its view is
/// retained for `session.view-retention-secs` and its windows kept
/// live on every edit for ten minutes (E6d.5's cost) for no one. The
/// token presented after an explicit `Detach` should resume nothing.
/// FAILS at `e7c8fd5`: the explicit detach retains exactly as a lost
/// socket does, and the token resumes the note.
#[test]
fn e9_review1_a_deliberate_detach_retains_no_view() {
    let daemon = daemon_with_note(NO_REOPEN);
    let mut first = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let token = first.token.expect("token");
    let (scratch, _) = first.where_am_i();
    let note = first.open_note(scratch);
    assert_ne!(note, scratch);
    // As the window's close does it: `Detach`, then the socket goes;
    // the token is presented once the daemon has torn the session down
    // (E9 fix round 3: before that it names a live session, refused).
    write_message(&mut first.stream, &FrontendEvent::Detach(first.fid)).expect("Detach");
    first.lose(&daemon);
    let mut next = Session::attach(&daemon, PROTOCOL_VERSION, Some(token));
    assert_eq!(
        next.where_am_i(),
        (scratch, 0),
        "a frontend that detached on purpose left nothing to resume"
    );
}

/// The opt-in idle exit counts attached frontends only, so a daemon
/// whose one window is between sockets --- its view retained, the
/// window reconnecting on the schedule --- stops under it while the
/// retention promises the view for ten minutes; the window then
/// retries forever against a socket nothing answers (the reconnect
/// never respawns). With the knob at one second and a view retained
/// for a lost socket, the daemon should hold its idle clock for the
/// retention. FAILS at `e7c8fd5`: the daemon exits within the window.
#[test]
fn e9_review1_idle_exit_waits_while_a_lost_socket_s_view_is_retained() {
    let mut daemon = daemon_with_note(
        "pmacs.config.set('daemon.idle-exit-secs', 1)\n\
         pmacs.config.set('session.view-retention-secs', 30)\n",
    );
    let mut first = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let token = first.token.expect("token");
    let (scratch, _) = first.where_am_i();
    let note = first.open_note(scratch);
    // The socket is lost: no Detach, the stream just goes.
    drop(first);
    // Named window: four seconds, four times the knob.
    std::thread::sleep(Duration::from_secs(4));
    assert!(
        daemon.is_alive(),
        "a retained view holds the idle clock for its retention"
    );
    let mut back = Session::attach(&daemon, PROTOCOL_VERSION, Some(token));
    assert_eq!(back.where_am_i().0, note, "and the view is still there");
}

// ---------------------------------------------------------------------------
// E9.1 --- tokens in the states the rows did not choose
// ---------------------------------------------------------------------------

/// The all-zero token is "no token": a session presenting it is served
/// a fresh view, not refused. Passes at `e7c8fd5` (by reading:
/// `token.filter(|t| !t.is_zero())`).
#[test]
fn e9_review1_a_zero_token_is_a_fresh_view_and_the_session_is_served() {
    let daemon = daemon_with_note("");
    let zero = SessionToken([0; SESSION_TOKEN_BYTES]);
    let mut session = Session::attach(&daemon, PROTOCOL_VERSION, Some(zero));
    let (scratch, _) = session.where_am_i();
    let minted = session.token.expect("minted");
    assert!(!minted.is_zero());
    session.type_str("z");
    session.wait("served", |m| {
        matches!(m, InstanceMessage::CursorByte { buffer_id, byte_pos: 1 } if *buffer_id == scratch)
            .then_some(())
    });
}

/// A token presented by another socket while the session it names is
/// still registered resumes nothing and disturbs nothing: the daemon
/// closes the new socket before writing anything, on every attempt,
/// and runs no detach for it (it never registered it); the first keeps
/// its window; once the first is lost its view is retained and a socket
/// presenting the token takes it.
///
/// Changed at E9 fix round 3 by the owner's ruling of 2026-10-19 on
/// review 3's Medium 1 (refusal and retry). The row expected a fresh
/// view for the second socket, the policy at `e7c8fd5`, and that is the
/// state review 3 showed a real reconnect reaching whenever the window
/// comes back before the daemon has seen its old socket's loss: a fresh
/// view accepted, the window's token replaced, the view it came back
/// for abandoned while still there. The ruling refuses the new socket
/// instead, so the old one is read to its end and its view retained,
/// and the window retries; a copied token whose original window stays
/// alive is refused on every attempt, as here, and the GUI stops at its
/// cap. The second presentation's view here would have been LOCAL's
/// document; the third socket's resume is unchanged.
#[test]
fn e9_review1_a_token_presented_while_its_session_lives_resumes_nothing() {
    let daemon = daemon_with_note("");
    let mut first = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let token = first.token.expect("token");
    let (scratch, _) = first.where_am_i();
    let note = first.open_note(scratch);

    let mut refused = Vec::new();
    for attempt in 1..=3 {
        let (fid, answer) = answer_to(&daemon, token);
        assert_eq!(
            answer, None,
            "attempt {attempt}: a live token is refused, the socket closed with nothing written"
        );
        refused.push(fid.0.to_string());
    }
    // The first is untouched: its typing echoes on the note.
    first.type_str("q");
    first.wait("the first session's echo on the note", |m| {
        matches!(m, InstanceMessage::CursorByte { buffer_id, byte_pos: 1 } if *buffer_id == note)
            .then_some(())
    });
    first.lose(&daemon);
    // The detach arm ran for the first and for no refused attempt: a
    // refusal registers nothing, so its reader's end tears nothing down.
    let torn_down = detached(&daemon);
    assert!(
        refused.iter().all(|fid| !torn_down.contains(fid)),
        "no refused attempt was ever a session: refused {refused:?}, detached {torn_down:?}"
    );
    let mut third = Session::attach(&daemon, PROTOCOL_VERSION, Some(token));
    assert_eq!(
        third.where_am_i(),
        (note, 1),
        "lost afterwards, the view is retained"
    );
}

/// The last frontend's question is pending when its socket is lost.
/// The minibuffer is the daemon's, not the session's, so the prompt
/// outlives the socket; the session that resumes under the token must
/// see the question it is still inside, and its `n` must detach it
/// with the daemon up. Expectation at `e7c8fd5` unknown by reading:
/// the row documents what a reconnecting window meets.
#[test]
fn e9_review1_the_last_frontend_s_question_survives_the_socket_and_is_answerable() {
    let mut daemon = daemon_with_note("");
    let mut only = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let token = only.token.expect("token");
    only.where_am_i();
    only.quit_chord();
    only.wait("the question", |m| match m {
        InstanceMessage::MinibufferPromptRows { prompt, .. }
            if prompt
                .as_deref()
                .is_some_and(|p| p.contains("stop the daemon")) =>
        {
            Some(())
        }
        _ => None,
    });
    // E9 fix round 3: the token is presented once the loss is handled;
    // sooner it names a live session, and is refused.
    only.lose(&daemon);
    let mut back = Session::attach(&daemon, PROTOCOL_VERSION, Some(token));
    let seen = back.wait_for(
        "the pending question",
        Duration::from_secs(3),
        &mut |m| match m {
            InstanceMessage::MinibufferPromptRows { prompt, .. }
                if prompt
                    .as_deref()
                    .is_some_and(|p| p.contains("stop the daemon")) =>
            {
                Some(())
            }
            _ => None,
        },
    );
    assert!(
        seen.is_ok(),
        "the resumed session is shown the question it is inside; it saw {:?}",
        seen.err()
    );
    back.key(Key::Char('n'), Modifiers::NONE);
    assert_eq!(back.ended(), Some(GoodbyeReason::Detached), "n detaches");
    assert!(daemon.is_alive());
}

// ---------------------------------------------------------------------------
// E9.5 --- the paths the row did not take
// ---------------------------------------------------------------------------

/// A daemon whose init opens `big.txt` (twenty-four MiB of pseudo-random
/// printable text, past the frame) into LOCAL's window, plus `extra`.
fn daemon_showing_big(extra: &str) -> (TestDaemon, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().expect("tempdir for the files");
    let big = dir.path().join("big.txt");
    let mut text = String::with_capacity(24 * 1024 * 1024 + 80);
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    while text.len() < 24 * 1024 * 1024 {
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
    let init = format!(
        "pmacs.buffer.find_or_open({big:?})\n{extra}",
        big = big.to_str().expect("utf-8"),
    );
    let daemon = TestDaemon::spawn_with_config(&init);
    let path = big.clone();
    std::mem::forget(dir);
    (daemon, path)
}

/// A fresh session clones LOCAL's document, and the daemon's own
/// window is on the big file: the attach sweep leaves its snapshot out
/// with a line in the daemon's log and nothing else, so the session
/// starts on a buffer no snapshot reached --- the pre-E9.5 state the
/// row names as its falsifier, on the path every `pmacs --gpu` with no
/// file takes. The window must be put on a buffer whose snapshot
/// reached the session, and the status must say what weighed too much,
/// within the bootstrap. FAILS at `e7c8fd5`: the cursor sits on the big
/// buffer, no snapshot of it came, and no `StatusFacts` names it. (The
/// review's first text of this row read only what came after the first
/// `CursorByte`, where a bootstrap's snapshots and first status never
/// are; rewritten at fix round 1 to read what reached the session.)
#[test]
fn e9_review1_a_fresh_attach_onto_an_oversize_document_is_refused_visibly() {
    let (daemon, _big) = daemon_showing_big("");
    let mut session = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let (document, _) = session.where_am_i();
    // Two more seconds of frames, named: the status is in the first
    // frame after the viewport declaration or not at all.
    let _ = session.wait_for("two seconds of frames", Duration::from_secs(2), &mut |_| {
        None::<()>
    });
    assert!(
        session.snapshots_seen.contains(&document),
        "the session's window is on {document:?} and no snapshot of it reached the session \
         (snapshots of {:?} did); its bootstrap left it on the oversize buffer",
        session.snapshots_seen
    );
    assert!(
        session
            .statuses_seen
            .iter()
            .any(|m| m.contains("a frontend can mirror")),
        "no status said what weighed too much; the statuses were {:?}",
        session.statuses_seen
    );
}

/// `pmacs --gpu big.txt`: the initial target past the frame is a
/// failed target with the sizes, never a session left at
/// `(connecting...)`. Passes at `e7c8fd5` (by reading).
#[test]
fn e9_review1_an_oversize_initial_target_fails_with_the_sizes() {
    use std::os::unix::ffi::OsStrExt as _;
    let (daemon, big) = daemon_showing_big("");
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
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
    write_message(
        &mut stream,
        &SessionBootstrapRequest {
            initial_target: Some(InitialTarget {
                cwd: big.parent().expect("dir").as_os_str().as_bytes().to_vec(),
                path: big.as_os_str().as_bytes().to_vec(),
            }),
        },
    )
    .expect("bootstrap");
    write_message(
        &mut stream,
        &FrontendEvent::Resume {
            frontend_id: fid,
            token: None,
        },
    )
    .expect("Resume");
    let first: InstanceMessage = read_message(&mut stream).expect("the target's answer");
    match first {
        InstanceMessage::InitialTargetResult(InitialTargetResult::Failed { message }) => {
            assert!(
                message.contains("a frontend can mirror"),
                "the failure names the sizes: {message:?}"
            );
        }
        other => panic!("expected a failed target with the sizes; got {other:?}"),
    }
}
