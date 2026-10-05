//! E8 review round 2 --- behavioural probes through a real daemon and
//! the fake language server, on what fix round 1 changed: the status
//! line a session below v26 is told on the quiet paths.
//!
//! Fix round 1 (`ccc6be7`) made every path that would open a popup say
//! the answer's first line to a session that cannot show one, dwell
//! and the typed trigger characters included. The typed path has a
//! witness; dwell, off by default, has none, so the line it says could
//! go, or repeat, with every row green. The row below is that witness,
//! and counts what a resting caret is told.
#![cfg(feature = "crdt")]

mod common;

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
use pmacs_protocol::{ByteRange, FrontendId, PopupPayload};

use common::daemon::{TestDaemon, build_default_caps};

/// `main` starts at byte 3; the fake answers a hover anywhere.
const SOURCE: &str = "fn main() { let value = 1; }\n";

/// One attached semantic session, as in `tests/e8_review1_probes.rs`.
struct Session {
    stream: UnixStream,
    rx: mpsc::Receiver<InstanceMessage>,
    fid: FrontendId,
}

impl Session {
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

/// A daemon visiting `a.rs` (holding `source`, in a cargo project) with
/// the fake server attached, semantic and multi-frontend, and `extra`
/// run after.
fn daemon_with(source: &str, extra: &str) -> (TestDaemon, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("Cargo.toml"), b"[package]\nname=\"x\"\n").expect("write");
    let a = dir.path().join("a.rs");
    std::fs::write(&a, source).expect("write a.rs");
    let fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp");
    let init = format!(
        "pmacs.lsp.config = {{ rust = {{ command = {fake:?}, env = {{ PMACS_FAKE_LSP_MODE = 'hover' }} }} }}\n\
         pmacs.buffer.find_or_open({:?})\n\
         for _, b in ipairs(pmacs.buffer.list()) do\n\
           if b:name() == '*scratch*' then pcall(pmacs.buffer.kill, b) end\n\
         end\n\
         {extra}\n",
        Path::new(&a).display().to_string()
    );
    let daemon = TestDaemon::spawn_with_env_and_init(
        &[
            ("PMACS_INSTANCE_SEMANTIC_RENDER", "1"),
            ("PMACS_INSTANCE_MULTI_FRONTEND", "1"),
        ],
        &init,
    );
    (daemon, dir)
}

/// FAILS with the quiet hover's line removed (`hover_popup`'s
/// `if told then` read as `if told and not quiet then`), which every
/// other row passes. A v25 session with `lsp.hover-on-dwell` on: a
/// motion and a rest of `lsp.hover-dwell-ms` asks the hover quietly,
/// and the session, which can show no popup, is told the hover's first
/// line, once per rest and not again while the caret stays (the status
/// band is shared by every frontend, so a repeat would reach them all).
/// The control is the same session told the hover for `C-c h`.
#[test]
fn review2_a_v25_session_is_told_its_dwell_hover_once_per_rest() {
    let (daemon, _dir) = daemon_with(
        SOURCE,
        "pmacs.config.set('lsp.hover-on-dwell', true)\n\
         pmacs.config.set('lsp.hover-dwell-ms', 300)",
    );
    let mut legacy = Session::attach(&daemon, 25);
    const { assert!(PROTOCOL_VERSION > 25) };
    // Control, and the server up: `C-c h` until the line is told.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        legacy.chord('h');
        let seen = legacy.drain(Duration::from_millis(700));
        assert!(popups(&seen).is_empty(), "no Popup reaches v25");
        if statuses(&seen).iter().any(|m| m.contains("pmacs-fake-lsp")) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "control: a v25 session is told the hover for C-c h"
        );
    }
    // Let anything the asked hover started settle, then clear the line
    // with a message of no hover's own.
    let _ = legacy.drain(Duration::from_secs(1));
    legacy.chord('x');
    let _ = legacy.drain(Duration::from_secs(1));
    // A motion, then a rest of ten dwell periods.
    legacy.ctrl('f');
    let seen = legacy.drain(Duration::from_secs(3));
    assert!(popups(&seen).is_empty(), "no Popup reaches v25");
    // Each time the line becomes the hover's, from anything else: a
    // `StatusFacts` repeating an unchanged line is not a new telling.
    let all = statuses(&seen);
    let told = all
        .iter()
        .enumerate()
        .filter(|(i, m)| m.contains("pmacs-fake-lsp") && (*i == 0 || all[i - 1] != **m))
        .count();
    assert_eq!(
        told, 1,
        "a resting caret in a v25 session is told the dwell's hover once; statuses {all:?}"
    );
    // Resting on, the line is not told again: anything arriving now
    // repeats it at most.
    let more = statuses(&legacy.drain(Duration::from_secs(2)));
    let last = all.last().cloned().unwrap_or_default();
    assert!(
        more.iter().all(|m| *m == last),
        "and not again while it rests: {more:?} after {last:?}"
    );
}
