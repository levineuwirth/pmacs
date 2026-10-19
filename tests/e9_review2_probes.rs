//! E9 review round 2 --- behavioral probes against fix round 1's
//! changes, through the production handshake and dispatch of a real
//! daemon: the states a user reaches that the fix round's witnesses
//! did not choose. Each row's doc comment says what it expects and
//! whether it fails at `e04aa9a`, the reviewed head, and why; the rows
//! that pass there witness claims the fix round made by reading alone.
//!
//! The harness is `tests/e9_review1_probes.rs`'s, repeated here so the
//! fix round can commit this file beside it unchanged.
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

struct Session {
    stream: UnixStream,
    rx: mpsc::Receiver<InstanceMessage>,
    fid: FrontendId,
    token: Option<SessionToken>,
    /// Every `BufferSnapshot` and every status message that passed
    /// through a wait, matched or not.
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
    patience: Duration,
) -> (UnixStream, FrontendId) {
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(patience))
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
        Self::attach_with_patience(daemon, offer, resume, Duration::from_secs(10))
    }

    /// `attach` with its own bound on the bootstrap, for a daemon whose
    /// sweep encodes more than ten seconds' worth of snapshots.
    fn attach_with_patience(
        daemon: &TestDaemon,
        offer: u32,
        resume: Option<SessionToken>,
        patience: Duration,
    ) -> Self {
        let (stream, fid) = handshake(daemon, offer, resume, patience);
        Self::from_stream(stream, fid, offer, patience)
    }

    fn from_stream(stream: UnixStream, fid: FrontendId, offer: u32, patience: Duration) -> Self {
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
            let token = session
                .wait_for("the SessionToken", patience, &mut |m| match m {
                    InstanceMessage::SessionToken(t) => Some(*t),
                    _ => None,
                })
                .unwrap_or_else(|seen| {
                    panic!("timed out waiting for the SessionToken; the daemon wrote {seen:?}")
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

    /// Leave as a window closing does: `Detach`, then the socket goes.
    fn close_as_a_window_does(mut self) {
        write_message(&mut self.stream, &FrontendEvent::Detach(self.fid)).expect("Detach");
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
/// plus `extra`.
fn daemon_with_note(extra: &str) -> TestDaemon {
    let dir = tempfile::TempDir::new().expect("tempdir for the note");
    let note = dir.path().join("note.txt");
    std::fs::write(&note, "one\ntwo\n").expect("write the note");
    let init = format!(
        "pmacs.command.define {{ name = 'e9.open', description = 'open the note',\n\
           fn = function() pmacs.buffer.find_or_open({path:?}) end }}\n{extra}",
        path = note.to_str().expect("utf-8 path"),
    );
    let daemon = TestDaemon::spawn_with_env_and_init(&[("PMACS_ATTACH_DEBUG", "1")], &init);
    std::mem::forget(dir);
    daemon
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

/// The daemon's file-backed stderr, as `tests/common/daemon.rs` places
/// it beside the socket.
fn daemon_log(socket: &Path) -> String {
    let mut path = socket.as_os_str().to_os_string();
    path.push(".stderr.log");
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Every regular file under `root` whose bytes contain `needle`.
fn files_containing(root: &Path, needle: &str) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file()
                && std::fs::read(&path).is_ok_and(|bytes| {
                    bytes
                        .windows(needle.len())
                        .any(|window| window == needle.as_bytes())
                })
            {
                found.push(path);
            }
        }
    }
    found
}

/// Type `text` into the note and wait for the daemon's cursor to echo
/// past it, so the edit is applied before whatever comes next.
fn edit_the_note(session: &mut Session, note: BufferId, text: &str) {
    session.type_str(text);
    let len = text.len() as u64;
    session.wait("the edit echoed", |m| {
        matches!(m, InstanceMessage::CursorByte { buffer_id, byte_pos } if *buffer_id == note && *byte_pos == len)
            .then_some(())
    });
}

// ---------------------------------------------------------------------------
// Medium 1 --- the hooks run; does the promise they stand for hold?
// ---------------------------------------------------------------------------

/// The usage text's promise for `--stop` is the autosave's last sweep
/// of unsaved text: the review's row showed the hook runs, this one
/// asks for the text. A session edits the note and never saves;
/// `--stop` stops the daemon; a recovery copy holding the typed text is
/// under the daemon's state root (the harness roots every store at the
/// socket's directory). The copies the sweep stages are installed by
/// the exit's own wait (`EditorState::drop`), which fix round 1 claims
/// by reading.
#[test]
fn e9_review2_stop_writes_the_recovery_copy_of_unsaved_text() {
    let mut daemon = daemon_with_note("");
    let socket = daemon.socket_path().to_path_buf();
    let root = socket.parent().expect("socket dir").to_path_buf();
    let mut attached = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let (scratch, _) = attached.where_am_i();
    let note = attached.open_note(scratch);
    let text = "unsaved-by-review2-stop";
    edit_the_note(&mut attached, note, text);
    assert!(
        files_containing(&root, text).is_empty(),
        "no copy before the stop (the periodic sweep is 30 s away)"
    );

    let (status, _, err) = daemon_cli("--stop", &socket);
    assert!(status.success(), "stop exit: {status:?}; {err}");
    assert_eq!(attached.ended(), Some(GoodbyeReason::ShuttingDown));
    let exit = exit_within(&mut daemon, 10);
    assert!(exit.success(), "a clean stop: {exit:?}");
    let copies = files_containing(&root, text);
    assert!(
        !copies.is_empty(),
        "--stop left no recovery copy of the unsaved text under {}; the daemon's log:\n{}",
        root.display(),
        daemon_log(&socket)
    );
}

/// The idle exit's version of the row above: the knob at one second,
/// the window leaves as a window does, the daemon stops on its own, and
/// the unsaved text it held is on disk.
#[test]
fn e9_review2_idle_exit_writes_the_recovery_copy_of_unsaved_text() {
    let mut daemon = daemon_with_note("pmacs.config.set('daemon.idle-exit-secs', 1)\n");
    let socket = daemon.socket_path().to_path_buf();
    let root = socket.parent().expect("socket dir").to_path_buf();
    let mut attached = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let (scratch, _) = attached.where_am_i();
    let note = attached.open_note(scratch);
    let text = "unsaved-by-review2-idle";
    edit_the_note(&mut attached, note, text);
    attached.close_as_a_window_does();
    let exit = exit_within(&mut daemon, 10);
    assert!(exit.success(), "a clean idle exit: {exit:?}");
    let copies = files_containing(&root, text);
    assert!(
        !copies.is_empty(),
        "the idle exit left no recovery copy of the unsaved text under {}; the daemon's log:\n{}",
        root.display(),
        daemon_log(&socket)
    );
}

/// A subscriber of `editor.before-quit` that vetoes: the Lua quit would
/// stop, `--stop` says so and stops anyway (fix round 1's stated rule,
/// by reading). The daemon exits 0 and its log carries the sentence.
#[test]
fn e9_review2_a_before_quit_veto_is_said_and_does_not_hold_the_stop() {
    let mut daemon =
        daemon_with_note("pmacs.hook.add('editor.before-quit', function() return false end)\n");
    let socket = daemon.socket_path().to_path_buf();
    let mut attached = Session::attach(&daemon, PROTOCOL_VERSION, None);
    attached.where_am_i();
    let (status, out, err) = daemon_cli("--stop", &socket);
    assert!(status.success(), "stop exit: {status:?}; {err}");
    assert!(out.contains("stopped the daemon at"), "{out}");
    assert_eq!(attached.ended(), Some(GoodbyeReason::ShuttingDown));
    let exit = exit_within(&mut daemon, 10);
    assert!(exit.success(), "a clean stop over the veto: {exit:?}");
    let log = daemon_log(&socket);
    assert!(
        log.contains("editor.before-quit vetoed --stop; stopping anyway"),
        "the veto is said in the daemon's log:\n{log}"
    );
}

// ---------------------------------------------------------------------------
// Medium 2 --- departures the fix round's rows did not take
// ---------------------------------------------------------------------------

/// The last frontend's `n` is a departure through the quit's request
/// (`detach_requested`, marked departing): the token it held resumes
/// nothing. The E9.3 last-frontend row reads the `Goodbye` and the
/// daemon's life, not what the token resumes.
#[test]
fn e9_review2_the_last_frontend_s_n_retains_no_view() {
    let mut daemon = daemon_with_note("");
    let mut only = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let token = only.token.expect("token");
    let (scratch, _) = only.where_am_i();
    let note = only.open_note(scratch);
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
    only.key(Key::Char('n'), Modifiers::NONE);
    assert_eq!(only.ended(), Some(GoodbyeReason::Detached), "n detaches");
    assert!(daemon.is_alive(), "n leaves the daemon up");
    let mut back = Session::attach(&daemon, PROTOCOL_VERSION, Some(token));
    assert_eq!(
        back.where_am_i(),
        (scratch, 0),
        "n is a departure: the note ({note:?}) is not resumed"
    );
}

/// A window loses its socket (the view retained, the idle clock held),
/// reconnects under its token, then closes on purpose: the clock runs
/// from that close, and a managed daemon on a one-second knob is gone
/// within a few seconds, not held for the retention it no longer owes.
#[test]
fn e9_review2_the_idle_clock_runs_once_a_resumed_window_leaves_on_purpose() {
    let mut daemon = daemon_with_note(
        "pmacs.config.set('daemon.idle-exit-secs', 1)\n\
         pmacs.config.set('session.view-retention-secs', 60)\n",
    );
    let mut first = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let token = first.token.expect("token");
    let (scratch, _) = first.where_am_i();
    let note = first.open_note(scratch);
    // The socket is lost: no Detach, the stream just goes.
    drop(first);
    std::thread::sleep(Duration::from_secs(3));
    assert!(daemon.is_alive(), "the retained view holds the clock");
    let mut back = Session::attach(&daemon, PROTOCOL_VERSION, Some(token));
    assert_eq!(back.where_am_i().0, note, "resumed on the note");
    back.close_as_a_window_does();
    let exit = exit_within(&mut daemon, 8);
    assert!(
        exit.success(),
        "a clean idle exit after the departure: {exit:?}"
    );
}

// ---------------------------------------------------------------------------
// Medium 3 --- the fallback the refusal sends a window to
// ---------------------------------------------------------------------------

/// Twenty-four MiB of pseudo-random printable text at `path`, past the
/// frame (the E9.5 row's file).
fn write_big(path: &Path) {
    let mut text = String::with_capacity(24 * 1024 * 1024 + 80);
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15 ^ (path.as_os_str().len() as u64);
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
    std::fs::write(path, &text).expect("write the big file");
}

/// A daemon whose init opens `big.txt` into LOCAL's window after
/// recording `*scratch*`, and then kills `*scratch*`, plus `extra`.
fn daemon_showing_big_without_scratch(extra: &str) -> TestDaemon {
    let dir = tempfile::TempDir::new().expect("tempdir for the files");
    let big = dir.path().join("big.txt");
    write_big(&big);
    let init = format!(
        "local scratch = pmacs.window.buffer()\n\
         pmacs.buffer.find_or_open({big:?})\n\
         pmacs.buffer.kill(scratch)\n{extra}",
        big = big.to_str().expect("utf-8"),
    );
    let daemon = TestDaemon::spawn_with_config(&init);
    std::mem::forget(dir);
    daemon
}

/// What a fresh attach is left on after its bootstrap: the window's
/// buffer, whether a snapshot of it reached the session within the
/// bootstrap and two seconds of frames, and the statuses.
fn bootstrap_outcome(session: &mut Session) -> (BufferId, bool) {
    let (document, _) = session.where_am_i();
    let _ = session.wait_for("two seconds of frames", Duration::from_secs(2), &mut |_| {
        None::<()>
    });
    (document, session.snapshots_seen.contains(&document))
}

/// `*scratch*` killed earlier, the daemon's window on the big file: the
/// refusal's fallback is a `*scratch*` created on the spot, which the
/// sweep (already run) never snapshotted. Fix round 1 says the tick
/// loop's follow path sends it on the first tick, by reading. The
/// window must be on a buffer whose snapshot reached the session.
#[test]
fn e9_review2_a_fresh_attach_with_scratch_killed_lands_on_a_buffer_that_arrived() {
    let daemon = daemon_showing_big_without_scratch("");
    let mut session = Session::attach(&daemon, PROTOCOL_VERSION, None);
    let (document, arrived) = bootstrap_outcome(&mut session);
    assert!(
        arrived,
        "the window is on {document:?} and no snapshot of it reached the session (snapshots \
         of {:?} did); statuses {:?}; the daemon's log:\n{}",
        session.snapshots_seen,
        session.statuses_seen,
        daemon_log(daemon.socket_path())
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

/// Two oversize buffers and no `*scratch*`: the refusal's fallback is
/// "any other buffer", which is the other oversize one. The window is
/// then on a buffer no snapshot can reach, the follow path refuses it
/// every tick with no buffer to go back to (`last_active_buffer_sent`
/// is never set for the session), and the daemon's log takes a line
/// per tick. A fallback that is itself past the frame is no fallback.
/// Expected to FAIL at `e04aa9a` on the first assertion; the count of
/// refusals in the log is printed beside it.
#[test]
fn e9_review2_a_fallback_that_is_itself_oversize_leaves_the_window_blank() {
    let dir = tempfile::TempDir::new().expect("tempdir for the files");
    let first = dir.path().join("first.txt");
    let second = dir.path().join("second.txt");
    write_big(&first);
    write_big(&second);
    let init = format!(
        "local scratch = pmacs.window.buffer()\n\
         pmacs.buffer.find_or_open({first:?})\n\
         pmacs.buffer.find_or_open({second:?})\n\
         pmacs.buffer.kill(scratch)\n",
        first = first.to_str().expect("utf-8"),
        second = second.to_str().expect("utf-8"),
    );
    let daemon = TestDaemon::spawn_with_config(&init);
    std::mem::forget(dir);
    // Two oversize buffers to upgrade and encode in the sweep: the
    // bootstrap takes longer than the harness's ten seconds, which is
    // itself a measurement (the cost of E9.5's encoding, unmeasured).
    let started = Instant::now();
    let mut session =
        Session::attach_with_patience(&daemon, PROTOCOL_VERSION, None, Duration::from_mins(2));
    let bootstrap = started.elapsed();
    let (document, arrived) = bootstrap_outcome(&mut session);
    let log = daemon_log(daemon.socket_path());
    let refusals = log.matches("cannot be shown here").count();
    assert!(
        arrived,
        "the window is on {document:?} and no snapshot of it reached the session (snapshots \
         of {:?} did); statuses {:?}; the daemon's log refused it {refusals} times in two \
         seconds (\"cannot be shown here\"); the bootstrap took {bootstrap:?}",
        session.snapshots_seen, session.statuses_seen,
    );
}

// ---------------------------------------------------------------------------
// High 1 --- the classifier through the CLI on a path that is neither
// ---------------------------------------------------------------------------

/// A directory at the socket path is not a socket: `--status` and
/// `--stop` name it and leave it, and neither spawns or removes
/// anything. The E9.4 row covers a plain file; the classifier's
/// `Ok(_)` arm is every other kind.
#[test]
fn e9_review2_a_directory_at_the_socket_path_is_named_and_left_alone() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let not_a_socket = dir.path().join("pmacs.sock");
    std::fs::create_dir(&not_a_socket).expect("mkdir");
    for action in ["--status", "--stop"] {
        let (status, _, err) = daemon_cli(action, &not_a_socket);
        assert!(!status.success(), "{action} fails on a directory");
        assert!(
            err.contains("exists and is not a socket; left alone"),
            "{action}: {err}"
        );
        assert!(not_a_socket.is_dir(), "{action} left the directory");
    }
}
