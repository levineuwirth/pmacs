//! E8 review round 1 --- behavioural probes against the popup rows,
//! through the production dispatch of a real daemon and the fake
//! language server: the states a user reaches that the handoff's
//! witnesses did not choose.
//!
//! Each row drives keys as a frontend sends them (`FrontendEvent::Key`
//! into the daemon's own dispatcher) and reads what the daemon writes to
//! each session, the outermost seam a semantic frontend has. The rows
//! that fail at `1ace8b3` say so in their doc comment, with the reason;
//! the rest are controls or rows the handoff claimed and did not
//! witness, which pass.
#![cfg(feature = "crdt")]

mod common;

use std::fmt::Write as _;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pmacs_protocol::cell::CellSize;
use pmacs_protocol::message::{
    ADVERTISED_PROTOCOL_VERSION, AttachRequest, FrontendCapabilities, FrontendEvent, Hello,
    InstanceMessage, Key, KeyEvent, Modifiers, PROTOCOL_VERSION, SessionBootstrapRequest,
};
use pmacs_protocol::transport::{read_message, write_message};
use pmacs_protocol::{ByteRange, FrontendId, PopupFrame, PopupKind, PopupPayload};

use common::daemon::{TestDaemon, build_default_caps};

/// `main` starts at byte 3, so the caret at byte 0 sits inside the
/// fake's hover range (`0..4`).
const SOURCE: &str = "fn main() { let value = 1; }\n";

/// One attached semantic session: its writer, a channel of everything
/// the daemon wrote it, and its frontend id.
struct Session {
    stream: UnixStream,
    rx: mpsc::Receiver<InstanceMessage>,
    fid: FrontendId,
}

impl Session {
    /// Attach offering exactly `offer`, declare a viewport so the
    /// projection producer is live, and start the reader thread.
    fn attach(daemon: &TestDaemon, offer: u32) -> Self {
        let mut stream = daemon.connect();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .expect("read timeout");
        let hello: Hello = read_message(&mut stream).expect("read Hello");
        assert_eq!(hello.protocol_version, ADVERTISED_PROTOCOL_VERSION);
        let fid = hello.assigned_frontend_id;
        write_message(
            &mut stream,
            &AttachRequest {
                protocol_version: offer,
                frontend_capabilities: FrontendCapabilities {
                    multi_frontend: true,
                    crdt_replica: true,
                    semantic_render: true,
                    ..build_default_caps()
                },
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
        let mut reader = stream.try_clone().expect("clone stream");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(msg) = read_message::<InstanceMessage>(&mut reader) {
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        let mut session = Self { stream, rx, fid };
        let document = session.wait("the first BufferSnapshot", |msg| match msg {
            InstanceMessage::BufferSnapshot { buffer_id, .. } => Some(*buffer_id),
            _ => None,
        });
        write_message(
            &mut session.stream,
            &FrontendEvent::Viewport {
                frontend_id: fid,
                buffer_id: document,
                visible: ByteRange { start: 0, end: 0 },
                generation: 0,
            },
        )
        .expect("declare a viewport");
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
        .expect("write key");
    }

    fn ctrl(&mut self, ch: char) {
        self.key(Key::Char(ch), Modifiers::CTRL);
    }

    fn typed(&mut self, text: &str) {
        for ch in text.chars() {
            self.key(Key::Char(ch), Modifiers::NONE);
        }
    }

    /// `C-c` then `ch`: the runtime's LSP chords.
    fn chord(&mut self, ch: char) {
        self.ctrl('c');
        self.key(Key::Char(ch), Modifiers::NONE);
    }

    fn wait<T>(&self, what: &str, mut want: impl FnMut(&InstanceMessage) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(20);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.rx.recv_timeout(left) {
                Ok(msg) => {
                    if let Some(found) = want(&msg) {
                        return found;
                    }
                }
                Err(_) => break,
            }
        }
        panic!("timed out waiting for {what}");
    }

    /// Every message within `window`.
    fn drain(&self, window: Duration) -> Vec<InstanceMessage> {
        let deadline = Instant::now() + window;
        let mut seen = Vec::new();
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.rx.recv_timeout(left) {
                Ok(msg) => seen.push(msg),
                Err(_) => break,
            }
        }
        seen
    }

    /// Messages, appended to `seen`, until a `BufferSnapshot` arrives
    /// (within 20 s), then one second more. A buffer change is closed by
    /// its snapshot, and a fixed window was short of it on four of CI's
    /// six test legs (run 37358723033, E8 fix round 1).
    fn until_snapshot(&self, seen: &mut Vec<InstanceMessage>) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !snapshot_in(seen) {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            match self.rx.recv_timeout(left) {
                Ok(msg) => seen.push(msg),
                Err(_) => break,
            }
        }
        seen.extend(self.drain(Duration::from_secs(1)));
    }

    fn popups_within(&self, window: Duration) -> Vec<PopupPayload> {
        popups(&self.drain(window))
    }

    /// The status-line messages within `window`.
    fn statuses_within(&self, window: Duration) -> Vec<String> {
        statuses(&self.drain(window))
    }

    fn ask_until_present(&mut self, ch: char) -> PopupFrame {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            self.chord(ch);
            let until = Instant::now() + Duration::from_millis(500);
            while let Some(left) = until.checked_duration_since(Instant::now()) {
                match self.rx.recv_timeout(left) {
                    Ok(InstanceMessage::Popup(PopupPayload::Present(frame))) => return frame,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
        panic!("no popup arrived for C-c {ch}");
    }
}

fn popups(msgs: &[InstanceMessage]) -> Vec<PopupPayload> {
    msgs.iter()
        .filter_map(|m| match m {
            InstanceMessage::Popup(p) => Some(p.clone()),
            _ => None,
        })
        .collect()
}

fn statuses(msgs: &[InstanceMessage]) -> Vec<String> {
    msgs.iter()
        .filter_map(|m| match m {
            InstanceMessage::StatusFacts {
                message: Some(m), ..
            } => Some(m.clone()),
            _ => None,
        })
        .collect()
}

/// An `init.lua` attaching the fake server to `.rs` files, visiting
/// `files` in order (the last one shown), `*scratch*` killed, `extra`
/// appended.
fn init_for(files: &[&Path], mode: &str, env: &[(&str, &str)], extra: &str) -> String {
    let fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp");
    let mut env_lua = format!("PMACS_FAKE_LSP_MODE = '{mode}',");
    for (k, v) in env {
        let _ = write!(env_lua, " {k} = '{v}',");
    }
    let mut visits = String::new();
    for f in files {
        let _ = writeln!(
            visits,
            "pmacs.buffer.find_or_open({:?})",
            f.display().to_string()
        );
    }
    format!(
        "pmacs.lsp.config = {{ rust = {{ command = {fake:?}, env = {{ {env_lua} }} }} }}\n\
         {visits}\
         for _, b in ipairs(pmacs.buffer.list()) do\n\
           if b:name() == '*scratch*' then pcall(pmacs.buffer.kill, b) end\n\
         end\n\
         {extra}\n"
    )
}

/// A daemon visiting `b.rs` then `a.rs` (shown), both holding `source`,
/// in a cargo project, semantic and multi-frontend.
fn daemon_with(
    source: &str,
    mode: &str,
    env: &[(&str, &str)],
    extra: &str,
) -> (TestDaemon, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("Cargo.toml"), b"[package]\nname=\"x\"\n").expect("write");
    let a = dir.path().join("a.rs");
    let b = dir.path().join("b.rs");
    std::fs::write(&a, source).expect("write a.rs");
    std::fs::write(&b, source).expect("write b.rs");
    let daemon = TestDaemon::spawn_with_env_and_init(
        &[
            ("PMACS_INSTANCE_SEMANTIC_RENDER", "1"),
            ("PMACS_INSTANCE_MULTI_FRONTEND", "1"),
        ],
        &init_for(&[&b, &a], mode, env, extra),
    );
    (daemon, dir)
}

/// FAILS at `1ace8b3`. A v25 semantic session --- a `pmacs-gpu` built
/// before E8 attached to a v26 daemon --- never negotiated `Popup`, so
/// the daemon rightly sends it none; but E8 also removed the
/// `LSP: <first line>` echo every frontend had, and put nothing in its
/// place for a peer that cannot take the popup. `C-c h` and `C-c s` on
/// that peer are silent: no popup, no status. Before E8 the same keys
/// set `LSP: # pmacs-fake-lsp` and `LSP: fn echo(...)` (the base's
/// `hover_at_cursor` and `signature_help_at_cursor`). The positive
/// control is the v26 session in the same daemon, which gets the popup.
#[test]
fn review1_a_v25_session_is_told_its_own_hover_and_signature() {
    let (daemon, _dir) = daemon_with(SOURCE, "sighelp", &[], "");
    let mut current = Session::attach(&daemon, PROTOCOL_VERSION);
    let frame = current.ask_until_present('h');
    assert_eq!(frame.lines[0], "# pmacs-fake-lsp", "the server answers");
    current.ctrl('g');
    let _ = current.drain(Duration::from_millis(500));

    let mut legacy = Session::attach(&daemon, 25);
    // Control: this session is told status messages at all.
    legacy.chord('x');
    let control = legacy.statuses_within(Duration::from_secs(2));
    assert!(
        !control.is_empty(),
        "control: a v25 session receives status messages (C-c x)"
    );
    legacy.chord('h');
    let seen = legacy.drain(Duration::from_secs(3));
    assert!(popups(&seen).is_empty(), "no Popup reaches v25");
    let told = statuses(&seen);
    assert!(
        told.iter().any(|m| m.contains("pmacs-fake-lsp")),
        "a v25 session pressing C-c h must be told the hover somehow (the \
         echo it had before E8); it was told {told:?}"
    );

    // Into the call, then `C-c s`.
    for _ in 0..8 {
        legacy.ctrl('f');
    }
    legacy.chord('s');
    let told = legacy.statuses_within(Duration::from_secs(3));
    assert!(
        told.iter().any(|m| m.contains("fn echo(")),
        "a v25 session pressing C-c s must be told the signature somehow; it \
         was told {told:?}"
    );
}

/// E8 fix round 1 (review 1's Medium 2): the signature a v25 session's
/// own typing asks for is told too. Typing `(` asks quietly (the
/// server's trigger character); before E8 that set `LSP: <label>` on
/// every frontend, and at `1ace8b3` a v25 session got neither the popup
/// nor the line. Quiet means no "no signature help", not silence. The
/// control is a v26 session typing the same, which is sent the popup.
#[test]
fn fr1_a_v25_session_is_told_the_signature_its_typing_asks_for() {
    let (daemon, _dir) = daemon_with(SOURCE, "sighelp", &[], "");
    let mut current = Session::attach(&daemon, PROTOCOL_VERSION);
    // The server is up once a hover is answered: the auto-trigger asks
    // nothing of a server that has not initialized.
    current.ask_until_present('h');
    current.ctrl('g');
    let _ = current.drain(Duration::from_millis(500));
    for _ in 0..7 {
        current.ctrl('f');
    }
    current.typed("(");
    let shown = current.popups_within(Duration::from_secs(3));
    assert!(
        shown
            .iter()
            .any(|p| matches!(p, PopupPayload::Present(f) if f.kind == PopupKind::Signature)),
        "control: a v26 session typing `(` is sent the signature popup: {shown:?}"
    );
    current.ctrl('g');
    let _ = current.drain(Duration::from_millis(500));

    let mut legacy = Session::attach(&daemon, 25);
    for _ in 0..7 {
        legacy.ctrl('f');
    }
    legacy.typed("(");
    let seen = legacy.drain(Duration::from_secs(3));
    assert!(popups(&seen).is_empty(), "no Popup reaches v25");
    let told = statuses(&seen);
    assert!(
        told.iter().any(|m| m.contains("fn echo(")),
        "a v25 session typing `(` must be told the signature; it was told {told:?}"
    );
}

/// PASSES at `1ace8b3`; the handoff states this close and witnesses
/// none. Focus leaving the popup's window closes it with an explicit
/// `Absent`, and focus coming back does not bring it back.
#[test]
fn review1_focus_leaving_the_window_closes_the_popup() {
    let (daemon, _dir) = daemon_with(SOURCE, "hover", &[], "");
    let mut s = Session::attach(&daemon, PROTOCOL_VERSION);
    s.ask_until_present('h');
    // `C-x 2` splits; `C-x o` moves focus to the other window.
    s.ctrl('x');
    s.typed("2");
    let split = s.popups_within(Duration::from_millis(800));
    s.ctrl('x');
    s.typed("o");
    let moved = s.popups_within(Duration::from_millis(1500));
    let all: Vec<_> = split.iter().chain(moved.iter()).cloned().collect();
    assert_eq!(
        all.last(),
        Some(&PopupPayload::Absent),
        "focus moving to another window closes the popup; split {split:?}, moved {moved:?}"
    );
    s.ctrl('x');
    s.typed("o");
    let back = s.popups_within(Duration::from_millis(1500));
    assert!(back.is_empty(), "closed, not hidden: {back:?}");
}

/// Whether `msgs` holds a `BufferSnapshot`: the frontend forgets its
/// popup at one (`pmacs-gpu`'s snapshot handler), and the producer
/// forgets what it sent, so a snapshot is the close for a buffer change
/// and no `Absent` follows it.
fn snapshot_in(msgs: &[InstanceMessage]) -> bool {
    msgs.iter()
        .any(|m| matches!(m, InstanceMessage::BufferSnapshot { .. }))
}

/// How many messages of each variant `msgs` holds, by name, for a
/// failure to say what did arrive.
fn kinds(msgs: &[InstanceMessage]) -> std::collections::BTreeMap<String, usize> {
    let mut out = std::collections::BTreeMap::new();
    for m in msgs {
        let name: String = format!("{m:?}")
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        *out.entry(name).or_insert(0) += 1;
    }
    out
}

fn presents(msgs: &[InstanceMessage]) -> usize {
    popups(msgs)
        .iter()
        .filter(|p| matches!(p, PopupPayload::Present(_)))
        .count()
}

/// PASSES at `1ace8b3`; the handoff states this close and witnesses
/// none. The popup's window showing another buffer (`C-x <right>`, as a
/// user switches) ends the popup: the switch's `BufferSnapshot` clears it on
/// the frontend, no `Present` follows, and switching back does not bring
/// it back (closed in the core, not hidden). No `Absent` is sent --- the
/// snapshot is the close, which the variant's doc comment in
/// `pmacs-protocol` does not say ("`Absent` is sent ... on a buffer or
/// window change").
#[test]
fn review1_switching_the_windows_buffer_closes_the_popup() {
    let (daemon, _dir) = daemon_with(SOURCE, "hover", &[], "");
    let mut s = Session::attach(&daemon, PROTOCOL_VERSION);
    let opened = s.ask_until_present('h');
    // E8 fix round 1: `C-x <right>` (`editor.next-buffer`) puts another
    // buffer in the window. The review's `C-x b b.rs RET` did not on CI's
    // runners: `C-x b` takes the top candidate and buffers are named by
    // path, so `b.rs` also matches `a.rs`'s path as a subsequence and the
    // window kept `a.rs` on four of six test legs.
    s.ctrl('x');
    let mut seen = s.drain(Duration::from_millis(300));
    s.key(Key::Right, Modifiers::NONE);
    let mut after = Vec::new();
    s.until_snapshot(&mut after);
    let cursors: std::collections::HashSet<_> = after
        .iter()
        .filter_map(|m| match m {
            InstanceMessage::CursorByte { buffer_id, .. } => Some(*buffer_id),
            _ => None,
        })
        .collect();
    assert!(
        snapshot_in(&after),
        "the window switched buffers; saw {:?}, cursors in {cursors:?}, the popup in {:?}",
        kinds(&after),
        opened.buffer_id
    );
    seen.extend(after);
    assert_eq!(
        presents(&seen),
        0,
        "nothing reopens it: {:?}",
        popups(&seen)
    );
    s.ctrl('x');
    let mut back = s.drain(Duration::from_millis(300));
    s.key(Key::Left, Modifiers::NONE);
    back.extend(s.drain(Duration::from_millis(1500)));
    // The session already holds `a.rs`'s replica, so the switch back
    // arrives as cursor traffic, not a snapshot.
    assert_eq!(
        presents(&back),
        0,
        "closed, not hidden: switching back reopens nothing: {:?}",
        popups(&back)
    );
}

/// PASSES at `1ace8b3`; the handoff reasons this close and witnesses
/// none. Killing the buffer under an open popup (`C-x k RET`) ends it:
/// the window's next buffer arrives as a snapshot and no `Present`
/// follows.
#[test]
fn review1_killing_the_buffer_closes_the_popup() {
    let (daemon, _dir) = daemon_with(SOURCE, "hover", &[], "");
    let mut s = Session::attach(&daemon, PROTOCOL_VERSION);
    s.ask_until_present('h');
    s.ctrl('x');
    s.typed("k");
    let mut seen = s.drain(Duration::from_millis(500));
    s.key(Key::Enter, Modifiers::NONE);
    let mut after = Vec::new();
    s.until_snapshot(&mut after);
    assert!(
        snapshot_in(&after),
        "the killed buffer's window shows another"
    );
    seen.extend(after);
    assert_eq!(
        presents(&seen),
        0,
        "nothing reopens it: {:?}",
        popups(&seen)
    );
}

/// PASSES at `1ace8b3`. The dedup counted on the wire, not read from
/// the code: one `Present` for a hover asked for repeatedly while the
/// answer is unchanged, none over frames the caret and text leave
/// alone; for a signature, one `Present` when it opens, none while the
/// argument is typed and a `,` re-asks with an unchanged answer, and
/// one `Absent` at `)`.
#[test]
fn review1_the_dedup_counted_on_the_wire() {
    let (daemon, _dir) = daemon_with(SOURCE, "hover", &[], "");
    let mut s = Session::attach(&daemon, PROTOCOL_VERSION);
    s.ask_until_present('h');
    // Three more asks for the same place, and frames from viewport
    // declarations that change nothing the popup depends on.
    for _ in 0..3 {
        s.chord('h');
        std::thread::sleep(Duration::from_millis(300));
    }
    let mut extra = s.popups_within(Duration::from_millis(1500));
    // `C-l` recenters: a frame that moves neither the caret nor the text.
    for _ in 0..3 {
        s.ctrl('l');
        extra.extend(s.popups_within(Duration::from_millis(300)));
    }
    extra.extend(s.popups_within(Duration::from_millis(1500)));
    assert!(
        extra.is_empty(),
        "an unchanged popup asked again is not resent; got {} messages: {extra:?}",
        extra.len()
    );
    drop(s);
    drop(daemon);

    let (daemon, _dir) = daemon_with(
        "\n",
        "sighelp",
        &[],
        "pmacs.config.set('editing.auto-pair', false)",
    );
    let mut s = Session::attach(&daemon, PROTOCOL_VERSION);
    s.typed("echo");
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut opened = 0;
    loop {
        assert!(Instant::now() < deadline, "typing `(` never opened it");
        s.typed("(");
        let got = s.popups_within(Duration::from_millis(800));
        opened += got
            .iter()
            .filter(|p| matches!(p, PopupPayload::Present(_)))
            .count();
        if opened > 0 {
            break;
        }
    }
    assert_eq!(opened, 1, "the signature opens with exactly one Present");
    s.typed("a");
    s.typed(",");
    s.typed("b");
    let typing = s.popups_within(Duration::from_millis(1500));
    assert!(
        typing.is_empty(),
        "typing the argument and an unchanged re-ask send nothing; got {typing:?}"
    );
    s.typed(")");
    let closed = s.popups_within(Duration::from_millis(1500));
    assert_eq!(closed, vec![PopupPayload::Absent], "one Absent at `)`");
}

/// PASSES at `1ace8b3`: `3bbed6d`'s generation check does not swallow a
/// real failure. A server that answers the hover with an error says so
/// on the status line, once, the request being the latest.
#[test]
fn review1_a_failing_hover_is_still_said_after_the_supersede_fix() {
    let (daemon, _dir) = daemon_with(SOURCE, "error", &[], "");
    let mut s = Session::attach(&daemon, PROTOCOL_VERSION);
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut told = Vec::new();
    while Instant::now() < deadline && !told.iter().any(|m: &String| m.contains("synthetic error"))
    {
        s.chord('h');
        told.extend(s.statuses_within(Duration::from_millis(600)));
    }
    assert!(
        told.iter().any(|m| m.contains("synthetic error")),
        "the server's error reaches the status line; told {told:?}"
    );
}

/// FAILS at `1ace8b3`. With `lsp.hover-on-dwell` on, a `C-c h` pressed
/// within half a second of a motion is superseded by the dwell's own
/// request for the same place (one generation counter for every popup
/// request, asked or automatic): the asked request's answer is dropped
/// by `3bbed6d`'s check, and the dwell's is quiet by design, so the user
/// who asked is told nothing --- here "LSP: no hover info", against a
/// server that has none and answers in 900 ms. The control is the same
/// sequence with dwell off, which says it.
#[test]
fn review1_dwell_does_not_swallow_an_asked_hovers_answer() {
    let env = [
        ("PMACS_FAKE_LSP_HOVER_LINES", "0"),
        ("PMACS_FAKE_LSP_HOVER_DELAY_MS", "900"),
    ];
    let run = |extra: &str| -> Vec<String> {
        let (daemon, _dir) = daemon_with(SOURCE, "hover", &env, extra);
        let mut s = Session::attach(&daemon, PROTOCOL_VERSION);
        // The server is up once an ask is answered.
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            assert!(Instant::now() < deadline, "the server never answered");
            s.chord('h');
            if s.statuses_within(Duration::from_secs(2))
                .iter()
                .any(|m| m == "LSP: no hover info")
            {
                break;
            }
        }
        // A different status, so the next "no hover info" is a change.
        s.chord('x');
        let _ = s.drain(Duration::from_millis(600));
        // A motion, then the ask at once.
        s.ctrl('f');
        s.chord('h');
        s.statuses_within(Duration::from_secs(4))
    };
    let off = run("");
    assert!(
        off.iter().any(|m| m == "LSP: no hover info"),
        "control: with dwell off the asked hover is answered; told {off:?}"
    );
    let on = run("pmacs.config.set('lsp.hover-on-dwell', true)");
    assert!(
        on.iter().any(|m| m == "LSP: no hover info"),
        "with dwell on, an asked hover must still be answered; told {on:?}"
    );
}

/// Sanity: the fake's signature is `PopupKind::Signature`.
#[test]
fn review1_probe_harness_sees_a_signature() {
    let (daemon, _dir) = daemon_with(SOURCE, "sighelp", &[], "");
    let mut s = Session::attach(&daemon, PROTOCOL_VERSION);
    for _ in 0..8 {
        s.ctrl('f');
    }
    let frame = s.ask_until_present('s');
    assert_eq!(frame.kind, PopupKind::Signature);
}
