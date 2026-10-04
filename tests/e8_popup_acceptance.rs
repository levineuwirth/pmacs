//! E8 — the hover and signature popup at the caret, through the
//! production dispatch of a real daemon (E8.2).
//!
//! Every row drives keys as a frontend sends them, `FrontendEvent::Key`
//! into the daemon's own dispatcher, against the fake language server,
//! and reads what the daemon writes to each session: the outermost seam a
//! semantic frontend has. A reader thread per session feeds a channel, so
//! a time-boxed drain never cuts a frame in half.
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
use pmacs_protocol::{ByteRange, FrontendId, MAX_POPUP_LINES, PopupFrame, PopupKind, PopupPayload};

use common::daemon::{TestDaemon, build_default_caps};

/// The source every row visits: `main` starts at byte 3, so the caret at
/// byte 0 sits inside the fake's hover range (`0..4`).
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

    /// Send one key as a frontend does.
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

    /// `C-c` then `ch`: the runtime's LSP chords.
    fn chord(&mut self, ch: char) {
        self.key(Key::Char('c'), Modifiers::CTRL);
        self.key(Key::Char(ch), Modifiers::NONE);
    }

    /// The first message `want` accepts, within 20 s.
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

    /// Every popup message that arrives within `window`.
    fn popups_within(&self, window: Duration) -> Vec<PopupPayload> {
        let deadline = Instant::now() + window;
        let mut seen = Vec::new();
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.rx.recv_timeout(left) {
                Ok(InstanceMessage::Popup(payload)) => seen.push(payload),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        seen
    }

    /// The next present popup, asking for it with `C-c ch` every 500 ms
    /// until one arrives: the server answers only once it has
    /// initialized, which the session cannot see.
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

    /// The next popup message, which must be `Absent`.
    fn wait_absent(&self, why: &str) {
        let payload = self.wait(why, |msg| match msg {
            InstanceMessage::Popup(p) => Some(p.clone()),
            _ => None,
        });
        assert_eq!(payload, PopupPayload::Absent, "{why}");
    }
}

/// An `init.lua` attaching the fake server to `.rs` files and visiting
/// `file`, `*scratch*` killed so an attaching frontend lands on the file
/// (the E7b chord suite's fixture), with `extra` appended.
fn init_for(file: &Path, mode: &str, env: &[(&str, &str)], extra: &str) -> String {
    let fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp");
    let mut env_lua = format!("PMACS_FAKE_LSP_MODE = '{mode}',");
    for (k, v) in env {
        let _ = write!(env_lua, " {k} = '{v}',");
    }
    format!(
        "pmacs.lsp.config = {{ rust = {{ command = {fake:?}, env = {{ {env_lua} }} }} }}\n\
         pmacs.buffer.find_or_open({:?})\n\
         for _, b in ipairs(pmacs.buffer.list()) do\n\
           if b:name() == '*scratch*' then pcall(pmacs.buffer.kill, b) end\n\
         end\n\
         {extra}\n",
        file.display().to_string()
    )
}

/// A daemon visiting a fresh `a.rs` holding [`SOURCE`] in a cargo
/// project, semantic and multi-frontend.
fn daemon_with(mode: &str, env: &[(&str, &str)], extra: &str) -> (TestDaemon, tempfile::TempDir) {
    daemon_with_source(SOURCE, mode, env, extra)
}

/// [`daemon_with`] on `source`.
fn daemon_with_source(
    source: &str,
    mode: &str,
    env: &[(&str, &str)],
    extra: &str,
) -> (TestDaemon, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("Cargo.toml"), b"[package]\nname=\"x\"\n").expect("write");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, source).expect("write a.rs");
    let daemon = TestDaemon::spawn_with_env_and_init(
        &[
            ("PMACS_INSTANCE_SEMANTIC_RENDER", "1"),
            ("PMACS_INSTANCE_MULTI_FRONTEND", "1"),
        ],
        &init_for(&file, mode, env, extra),
    );
    (daemon, dir)
}

/// The fake's hover, as the popup carries it.
const FAKE_HOVER: [&str; 3] = [
    "# pmacs-fake-lsp",
    "",
    "Synthetic hover content for the symbol under cursor.",
];

/// E8.2's core: `C-c h` opens the hover popup at the caret carrying the
/// server's whole text, on the session that asked and only there; it is
/// sent ONCE while unchanged, not once a frame; motion out of its range,
/// `C-g` and an edit each close it with an explicit `Absent`. And a v25
/// session, which never negotiated the variant, receives none of it,
/// even for a popup it opened itself.
#[test]
fn e8_2_hover_opens_once_closes_on_motion_cg_and_edit_and_never_reaches_v25() {
    let (daemon, _dir) = daemon_with("hover", &[], "");
    assert_eq!(PROTOCOL_VERSION, 26);
    let mut current = Session::attach(&daemon, PROTOCOL_VERSION);

    let frame = current.ask_until_present('h');
    assert_eq!(frame.kind, PopupKind::Hover);
    assert_eq!(frame.lines, FAKE_HOVER, "the server's whole text, as lines");
    assert_eq!(frame.anchor_byte, 0, "the start of the server's range");
    assert_eq!(frame.omitted_lines, 0);
    assert_eq!(frame.active_range, None);

    // The dedup: nothing changes for a second and a half of frames, so
    // nothing is resent.
    let resent = current.popups_within(Duration::from_millis(1500));
    assert!(
        resent.is_empty(),
        "an unchanged popup must not be resent each frame; got {} resends, the first {:?}",
        resent.len(),
        resent.first()
    );

    // Motion out of its range closes it: `C-e` puts the caret at the
    // line's end, far past `0..4`.
    current.key(Key::Char('e'), Modifiers::CTRL);
    current.wait_absent("motion out of the range closes the popup");

    // Back to the start; asking again opens it again.
    current.key(Key::Char('a'), Modifiers::CTRL);
    current.ask_until_present('h');
    current.key(Key::Char('g'), Modifiers::CTRL);
    current.wait_absent("C-g closes the popup");

    current.ask_until_present('h');
    current.key(Key::Char('x'), Modifiers::NONE);
    current.wait_absent("an edit closes the popup");

    // Motion back INTO the old range does not bring a closed popup back.
    current.key(Key::Char('a'), Modifiers::CTRL);
    let after = current.popups_within(Duration::from_millis(800));
    assert!(
        after.is_empty(),
        "a closed popup stays closed; got {after:?}"
    );

    // A v25 session: its own hover opens (the v26 session's open popup
    // closes, the one popup having moved to the v25 window, which is
    // the positive control) and the v25 session receives no `Popup`.
    current.ask_until_present('h');
    let mut legacy = Session::attach(&daemon, 25);
    legacy.chord('h');
    current.wait_absent("the popup moved to the v25 session's window");
    let leaked = legacy.popups_within(Duration::from_millis(1500));
    assert!(
        leaked.is_empty(),
        "a v25 session must never receive the v26 variant; got {leaked:?}"
    );
}

/// The named behaviours at the bound: two hundred lines arrive whole;
/// a text past the bound arrives cut at it with the rest counted, never
/// silently ended; and an empty answer opens nothing and says so.
#[test]
fn e8_2_a_long_hover_arrives_whole_one_past_the_bound_is_counted_and_none_is_said() {
    let (daemon, _dir) = daemon_with("hover", &[("PMACS_FAKE_LSP_HOVER_LINES", "200")], "");
    let mut session = Session::attach(&daemon, PROTOCOL_VERSION);
    let frame = session.ask_until_present('h');
    assert_eq!(frame.lines.len(), 202, "the code line, a blank, 200 lines");
    assert_eq!(
        frame.lines[0], "fn hovered()",
        "the fence is markup and goes"
    );
    assert_eq!(frame.omitted_lines, 0);
    assert_eq!(frame.validate(), Ok(()));
    drop(session);
    drop(daemon);

    let (daemon, _dir) = daemon_with("hover", &[("PMACS_FAKE_LSP_HOVER_LINES", "300")], "");
    let mut session = Session::attach(&daemon, PROTOCOL_VERSION);
    let frame = session.ask_until_present('h');
    assert_eq!(frame.lines.len(), MAX_POPUP_LINES);
    assert_eq!(frame.omitted_lines as usize, 302 - MAX_POPUP_LINES);
    assert_eq!(frame.validate(), Ok(()));
    drop(session);
    drop(daemon);

    let (daemon, _dir) = daemon_with("hover", &[("PMACS_FAKE_LSP_HOVER_LINES", "0")], "");
    let mut session = Session::attach(&daemon, PROTOCOL_VERSION);
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut said = false;
    while !said && Instant::now() < deadline {
        session.chord('h');
        let until = Instant::now() + Duration::from_millis(500);
        while let Some(left) = until.checked_duration_since(Instant::now()) {
            match session.rx.recv_timeout(left) {
                Ok(InstanceMessage::Popup(p)) => panic!("an empty answer opened {p:?}"),
                Ok(InstanceMessage::StatusFacts {
                    message: Some(m), ..
                }) if m == "LSP: no hover info" => said = true,
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }
    assert!(said, "an empty answer says so on the status line");
}

/// An answer that lands after the caret moved describes a place the
/// user has left: it opens nothing and says nothing. The fake holds each
/// hover back 400 ms; the first ask proves it answers.
#[test]
fn e8_2_a_hover_answered_after_the_caret_moved_opens_nothing() {
    let (daemon, _dir) = daemon_with("hover", &[("PMACS_FAKE_LSP_HOVER_DELAY_MS", "400")], "");
    let mut session = Session::attach(&daemon, PROTOCOL_VERSION);
    session.ask_until_present('h');
    session.key(Key::Char('g'), Modifiers::CTRL);
    session.wait_absent("C-g");
    // Ask, then move inside the old range before the answer lands.
    session.chord('h');
    session.key(Key::Char('f'), Modifiers::CTRL);
    let late = session.popups_within(Duration::from_millis(1500));
    assert!(
        late.is_empty(),
        "an answer for a caret that moved must open nothing; got {late:?}"
    );
}

/// `C-c s` opens the signature popup, anchored at the call's `(`.
#[test]
fn e8_2_signature_help_opens_a_signature_popup_at_the_open_parenthesis() {
    let (daemon, _dir) = daemon_with("sighelp", &[], "");
    let mut session = Session::attach(&daemon, PROTOCOL_VERSION);
    // Into the call: `fn main(` is bytes 0..8, so the caret after `(`
    // is byte 8.
    for _ in 0..8 {
        session.key(Key::Char('f'), Modifiers::CTRL);
    }
    let frame = session.ask_until_present('s');
    assert_eq!(frame.kind, PopupKind::Signature);
    assert_eq!(
        frame.lines[0],
        "fn echo(name: &str, count: usize) -> String"
    );
    assert!(
        frame
            .lines
            .iter()
            .any(|l| l == "Echoes `name` `count` times."),
        "the signature's documentation follows; got {:?}",
        frame.lines
    );
    assert_eq!(frame.anchor_byte, 7, "the call's open parenthesis");
}

/// Dwell, behind `lsp.hover-on-dwell`: a motion followed by a rest opens
/// the hover with no chord; the rest after it, a `C-g`, and an edit do
/// not re-arm it; and with the setting off a rest opens nothing.
#[test]
fn e8_2_dwell_opens_after_motion_only_once_and_not_when_off() {
    let (daemon, _dir) = daemon_with("hover", &[], "pmacs.config.set('lsp.hover-on-dwell', true)");
    let mut session = Session::attach(&daemon, PROTOCOL_VERSION);
    // The first sight of the window does not arm: nothing for 1.5 s.
    let opened = session.popups_within(Duration::from_millis(1500));
    assert!(
        opened.is_empty(),
        "first sight must not arm; got {opened:?}"
    );
    // A motion (`C-f`, one byte on) arms; the rest opens it, without a
    // chord. Repeated, since the server may still be initializing when
    // the first rest fires: each motion re-arms. One motion per round,
    // never a there-and-back, which could land inside one tick and
    // leave the resting place where it was.
    let deadline = Instant::now() + Duration::from_secs(20);
    let frame = loop {
        assert!(Instant::now() < deadline, "dwell never opened the hover");
        session.key(Key::Char('f'), Modifiers::CTRL);
        let got = session.popups_within(Duration::from_millis(1500));
        if let Some(PopupPayload::Present(frame)) = got.into_iter().next() {
            break frame;
        }
    };
    assert_eq!(frame.lines, FAKE_HOVER);
    // Dismissed, it stays dismissed while the caret rests.
    session.key(Key::Char('g'), Modifiers::CTRL);
    session.wait_absent("C-g closes the dwell popup");
    let again = session.popups_within(Duration::from_millis(1500));
    assert!(
        again.is_empty(),
        "a resting caret must not re-arm; got {again:?}"
    );
    // An edit does not arm it either: typing must not summon the docs.
    session.key(Key::Char('x'), Modifiers::NONE);
    let typed = session.popups_within(Duration::from_millis(1500));
    assert!(
        typed.is_empty(),
        "an edit must not arm dwell; got {typed:?}"
    );
    drop(session);
    drop(daemon);

    // Off (the default): a motion and a rest open nothing.
    let (daemon, _dir) = daemon_with("hover", &[], "");
    let mut session = Session::attach(&daemon, PROTOCOL_VERSION);
    session.ask_until_present('h'); // the server is up
    session.key(Key::Char('g'), Modifiers::CTRL);
    session.wait_absent("C-g");
    session.key(Key::Char('f'), Modifiers::CTRL);
    let off = session.popups_within(Duration::from_millis(1500));
    assert!(off.is_empty(), "dwell is off by default; got {off:?}");
}

/// E8.5: typing a trigger character opens the signature popup with its
/// active parameter marked; the popup survives the characters typed
/// inside the call and `,`, which asks again; and a typed `)` closes
/// it. Auto-pairing is off, so `)` is typed and not stepped over: with
/// pairing on the step-over is a motion out of the range, which closes
/// it anyway.
#[test]
fn e8_5_the_signature_marks_its_parameter_survives_typing_and_closes_on_paren() {
    let (daemon, _dir) = daemon_with_source(
        "\n",
        "sighelp",
        &[],
        "pmacs.config.set('editing.auto-pair', false)",
    );
    let mut session = Session::attach(&daemon, PROTOCOL_VERSION);
    // The server answers once it has initialized; the first `(` may come
    // too early, so ask by typing until the popup opens.
    for ch in "echo".chars() {
        session.key(Key::Char(ch), Modifiers::NONE);
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    let frame = loop {
        assert!(
            Instant::now() < deadline,
            "typing `(` never opened the signature"
        );
        session.key(Key::Char('('), Modifiers::NONE);
        let got = session.popups_within(Duration::from_millis(800));
        if let Some(PopupPayload::Present(frame)) = got.into_iter().last() {
            break frame;
        }
    };
    assert_eq!(frame.kind, PopupKind::Signature);
    let label = &frame.lines[0];
    assert_eq!(label, "fn echo(name: &str, count: usize) -> String");
    let active = frame.active_range.expect("the active parameter is marked");
    assert_eq!(active.line, 0);
    assert_eq!(
        &label[active.start as usize..active.end as usize],
        "count: usize",
        "the server's active parameter, found after the `(`"
    );

    // Typing the argument keeps it: no `Absent`.
    session.key(Key::Char('a'), Modifiers::NONE);
    session.key(Key::Char(','), Modifiers::NONE);
    session.key(Key::Char('b'), Modifiers::NONE);
    let kept = session.popups_within(Duration::from_millis(1200));
    assert!(
        !kept.contains(&PopupPayload::Absent),
        "typing inside the call must not close the signature; got {kept:?}"
    );

    // `)` closes it.
    session.key(Key::Char(')'), Modifiers::NONE);
    session.wait_absent("a typed `)` closes the signature popup");
}

/// E8.5: a server's parameter offsets count its position encoding's
/// units, not bytes. Against a UTF-16 server (the fake negotiates none)
/// and a label with `ö` and `ß` before the parameter, the popup marks
/// `höhe: u8`; read as bytes, the same offsets would mark `e(höhe:`.
#[test]
fn e8_5_the_active_parameter_is_converted_from_the_servers_encoding() {
    let (daemon, _dir) = daemon_with("sighelp", &[("PMACS_FAKE_LSP_SIG_NONASCII", "1")], "");
    let mut session = Session::attach(&daemon, PROTOCOL_VERSION);
    for _ in 0..8 {
        session.key(Key::Char('f'), Modifiers::CTRL);
    }
    let frame = session.ask_until_present('s');
    let label = &frame.lines[0];
    assert_eq!(label, "fn größe(höhe: u8, b: u8)");
    let active = frame.active_range.expect("the active parameter is marked");
    assert_eq!(
        &label[active.start as usize..active.end as usize],
        "höhe: u8",
        "UTF-16 offsets 9..17 are bytes 11..20 of this label"
    );
}

/// A second `C-c h` while the first is in flight supersedes it, and the
/// superseded request says nothing: its cancellation is the user's own
/// newer question, not "server unavailable". Found by the witness on
/// `src/editor.rs`, where a probe asking every half second left that
/// status beside a good popup. The fake holds each answer 400 ms.
#[test]
fn e8_2_a_superseded_hover_says_nothing_and_the_latest_opens() {
    let (daemon, _dir) = daemon_with("hover", &[("PMACS_FAKE_LSP_HOVER_DELAY_MS", "400")], "");
    let mut session = Session::attach(&daemon, PROTOCOL_VERSION);
    session.ask_until_present('h');
    session.key(Key::Char('g'), Modifiers::CTRL);
    session.wait_absent("C-g");
    session.chord('h');
    session.chord('h');
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut opened = false;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match session.rx.recv_timeout(left) {
            Ok(InstanceMessage::Popup(PopupPayload::Present(_))) => opened = true,
            Ok(InstanceMessage::StatusFacts {
                message: Some(m), ..
            }) => assert!(
                !m.contains("unavailable") && !m.contains("cancelled"),
                "a superseded request must say nothing; the status said {m:?}"
            ),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(opened, "the latest request opens the popup");
}
