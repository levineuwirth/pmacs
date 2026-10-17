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
    // The daemon's handshake breadcrumbs go to its stderr file, which a
    // failing row prints.
    let daemon = TestDaemon::spawn_with_env_and_init(&[("PMACS_ATTACH_DEBUG", "1")], &init);
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
    let required = std::env::var_os("PMACS_REQUIRE_GPU").is_some();
    let binary = Path::new(env!("CARGO_BIN_EXE_pmacs"))
        .parent()
        .expect("test binary directory")
        .join("pmacs-gpu");
    if !binary.exists() {
        assert!(
            !required,
            "PMACS_REQUIRE_GPU is set but {} is not built; build the workspace first",
            binary.display()
        );
        eprintln!("skipping the GPU probe: {} is not built", binary.display());
        return None;
    }
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
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let no_adapter = output.status.code() == Some(3);
        assert!(
            no_adapter && !required,
            "headless GPU probe failed (status {:?}, cut at {cut_at:?}):\n{stderr}",
            output.status.code()
        );
        eprintln!("skipping the GPU probe: no wgpu adapter available");
        return None;
    }
    assert!(
        cut_at.is_some(),
        "the probe never wrote its ready marker; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The probe's own log, shown by libtest when the row fails.
    eprintln!("probe stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    let text = std::fs::read_to_string(&report).expect("read the probe report");
    Some(
        text.lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
    )
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

// ---------------------------------------------------------------------------
// E9.3 --- quit
// ---------------------------------------------------------------------------

impl Session {
    /// `C-x C-c`.
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
/// daemon lives, and the view the quitter left is retained for its
/// token like any detach's.
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
    // And the quitter's view is retained for its token.
    let mut back = Session::attach(&daemon, PROTOCOL_VERSION, Some(token));
    assert_eq!(back.where_am_i().0, note, "the quit retained the view");
}

/// The last frontend attached is asked whether the daemon should stop
/// with it: `n` detaches it and the daemon lives; `y` stops the daemon,
/// which tells the frontend it is shutting down and exits.
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
    assert_eq!(
        last.ended(),
        Some(GoodbyeReason::ShuttingDown),
        "y stops the daemon, which says so"
    );
    let status = exit_within(&mut daemon, 10);
    assert!(status.success(), "a clean stop: {status:?}");
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
/// to for that long stops on its own; by default it never does.
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
    drop(session);
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
    drop(session);
    // Named window: 2 s with no frontend, the knob at its default.
    std::thread::sleep(Duration::from_secs(2));
    assert!(default.is_alive(), "by default an idle daemon stays up");
}
