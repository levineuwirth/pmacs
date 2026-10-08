//! E8b review round 1's daemon probe: E8 review 1's switching row as it
//! was at `44dfb24` (`tests/e8_review1_probes.rs`,
//! `review1_switching_the_windows_buffer_closes_the_popup`), its keys,
//! drains and harness unchanged, replayed against PR #325's head, with
//! everything the session was written recorded instead of one
//! `snapshot_in(&seen)`.
//!
//! What it is for: that row failed on CI's macOS legs, whose temporary
//! root (`/var/folders/36/tjdph2t965j8snz9_vkdnw0r0000gn/T/`, read from
//! the jobs' logs) holds no `b`, so there `b.rs` matched only `b.rs`;
//! and it failed on two Ubuntu luajit legs and `Test (crdt)`, and PASSED
//! on `Test (ubuntu-latest / lua54)` in both runs (37358723033 at
//! `44dfb24`, 37364522422's attempt 3 at `7505eea`). #324's mechanism
//! (the window already showed `b.rs`) predicts a failure wherever `b.rs`
//! selects `b.rs`. At the head `b.rs` always does (E8b.1), so a pass
//! here could only be a `BufferSnapshot` the switch did not send.
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
use pmacs_protocol::{BufferId, ByteRange, FrontendId, PopupPayload};

use common::daemon::{TestDaemon, build_default_caps};

const SOURCE: &str = "fn main() { let value = 1; }\n";

/// E8 review 1's harness at `44dfb24`: attach with no target, declare a
/// viewport on the first `BufferSnapshot`, read on a thread.
struct Session {
    stream: UnixStream,
    rx: mpsc::Receiver<(Instant, InstanceMessage)>,
    fid: FrontendId,
}

impl Session {
    fn attach(daemon: &TestDaemon) -> (Self, BufferId) {
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
                protocol_version: PROTOCOL_VERSION,
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
                if tx.send((Instant::now(), msg)).is_err() {
                    break;
                }
            }
        });
        let mut session = Self { stream, rx, fid };
        let deadline = Instant::now() + Duration::from_secs(20);
        let document = loop {
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("timed out waiting for the first BufferSnapshot");
            if let Ok((_, InstanceMessage::BufferSnapshot { buffer_id, .. })) =
                session.rx.recv_timeout(left)
            {
                break buffer_id;
            }
        };
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
        (session, document)
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

    fn drain(&self, window: Duration) -> Vec<(Instant, InstanceMessage)> {
        let deadline = Instant::now() + window;
        let mut out = Vec::new();
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.rx.recv_timeout(left) {
                Ok(m) => out.push(m),
                Err(_) => break,
            }
        }
        out
    }

    /// `C-c h` every half second until a `Present` arrives; returns
    /// everything read on the way, as review 1's `ask_until_present`
    /// read and dropped it.
    fn ask_until_present(&mut self) -> Vec<(Instant, InstanceMessage)> {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            self.ctrl('c');
            self.key(Key::Char('h'), Modifiers::NONE);
            let until = Instant::now() + Duration::from_millis(500);
            while let Some(left) = until.checked_duration_since(Instant::now()) {
                match self.rx.recv_timeout(left) {
                    Ok(m) => {
                        let present =
                            matches!(&m.1, InstanceMessage::Popup(PopupPayload::Present(_)));
                        seen.push(m);
                        if present {
                            return seen;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
        panic!("no hover popup arrived for C-c h");
    }
}

/// Review 1's init, and a `process.after-tick` hook appending each new
/// buffer's id and name to `log`, so a snapshot's buffer can be named.
fn init_for(files: &[&Path], log: &Path) -> String {
    let fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp");
    let mut visits = String::new();
    for f in files {
        let _ = writeln!(
            visits,
            "pmacs.buffer.find_or_open({:?})",
            f.display().to_string()
        );
    }
    format!(
        "pmacs.lsp.config = {{ rust = {{ command = {fake:?}, \
         env = {{ PMACS_FAKE_LSP_MODE = 'hover' }} }} }}\n\
         {visits}\
         for _, b in ipairs(pmacs.buffer.list()) do\n\
           if b:name() == '*scratch*' then pcall(pmacs.buffer.kill, b) end\n\
         end\n\
         local logged = {{}}\n\
         pmacs.hook.add('process.after-tick', function()\n\
           for _, id in ipairs(pmacs.buffer.list()) do\n\
             local k = tostring(id)\n\
             local name = pmacs.describe.buffer(id).name\n\
             if not logged[k] then\n\
               logged[k] = true\n\
               local f = io.open({log:?}, 'a')\n\
               f:write(k .. '\\t' .. name .. '\\n')\n\
               f:close()\n\
             end\n\
             if name == '*errors*' and id:len() ~= (logged.errors or 0) then\n\
               logged.errors = id:len()\n\
               local f = io.open({log:?} .. '.errors', 'w')\n\
               f:write(id:slice(0, id:len()))\n\
               f:close()\n\
             end\n\
           end\n\
         end)\n",
        log = log.display().to_string()
    )
}

/// What one replay of review 1's row saw.
#[derive(Debug)]
#[allow(dead_code, reason = "the fields are the report, read through Debug")]
struct Replay {
    first_declared: BufferId,
    snapshots_before_keys: Vec<BufferId>,
    snapshots_in_seen: Vec<(BufferId, u128)>,
    cursor_buffers_in_seen: Vec<BufferId>,
    bands_in_seen: Vec<String>,
    buffers: Vec<String>,
}

fn replay_once() -> Replay {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("Cargo.toml"), b"[package]\nname=\"x\"\n").expect("write");
    let a = dir.path().join("a.rs");
    let b = dir.path().join("b.rs");
    std::fs::write(&a, SOURCE).expect("write a.rs");
    std::fs::write(&b, SOURCE).expect("write b.rs");
    let daemon = TestDaemon::spawn_with_env_and_init(
        &[
            ("PMACS_INSTANCE_SEMANTIC_RENDER", "1"),
            ("PMACS_INSTANCE_MULTI_FRONTEND", "1"),
        ],
        &init_for(&[&b, &a], &dir.path().join("buffers.log")),
    );
    let (mut s, first_declared) = Session::attach(&daemon);
    let before = s.ask_until_present();
    // Review 1's row from here, unchanged: `C-x b`, 500 ms, `b.rs`
    // RET, 1500 ms, all of it `seen`.
    s.ctrl('x');
    s.typed("b");
    let keys_at = Instant::now();
    let mut seen = s.drain(Duration::from_millis(500));
    s.typed("b.rs");
    s.key(Key::Enter, Modifiers::NONE);
    seen.extend(s.drain(Duration::from_millis(1500)));
    let mut buffers: Vec<String> = std::fs::read_to_string(dir.path().join("buffers.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    if let Ok(errors) = std::fs::read_to_string(dir.path().join("buffers.log.errors")) {
        buffers.push(format!("*errors* holds: {errors}"));
    }
    Replay {
        buffers,
        first_declared,
        snapshots_before_keys: before
            .iter()
            .filter_map(|(_, m)| match m {
                InstanceMessage::BufferSnapshot { buffer_id, .. } => Some(*buffer_id),
                _ => None,
            })
            .collect(),
        snapshots_in_seen: seen
            .iter()
            .filter_map(|(t, m)| match m {
                InstanceMessage::BufferSnapshot { buffer_id, .. } => {
                    Some((*buffer_id, t.saturating_duration_since(keys_at).as_millis()))
                }
                _ => None,
            })
            .collect(),
        cursor_buffers_in_seen: seen
            .iter()
            .filter_map(|(_, m)| match m {
                InstanceMessage::CursorByte { buffer_id, .. } => Some(*buffer_id),
                _ => None,
            })
            .collect(),
        bands_in_seen: seen
            .iter()
            .filter_map(|(_, m)| match m {
                InstanceMessage::StatusFacts {
                    message: Some(msg), ..
                } => Some(msg.clone()),
                _ => None,
            })
            .collect(),
    }
}

/// PASSES at `0001946` if #324 is the whole of review 1's red: in each
/// of six replays the window already shows `b.rs` (the first snapshot,
/// declared), `C-x b b.rs RET` selects it, the band says `already
/// showing`, and no `BufferSnapshot` reaches `seen`, so review 1's
/// `snapshot_in(&seen)` fails every time. A replay that sees one is
/// printed with the buffer and its time after `C-x b`: a snapshot the
/// switch did not send would let that row pass with nothing switched,
/// which is the one shape that could explain its two passes on
/// `Test (ubuntu-latest / lua54)`.
#[test]
fn review1_s_switching_row_replayed_at_the_head_never_sees_a_snapshot() {
    let replays: Vec<Replay> = (0..6).map(|_| replay_once()).collect();
    let passing: Vec<&Replay> = replays
        .iter()
        .filter(|r| !r.snapshots_in_seen.is_empty())
        .collect();
    for r in &replays {
        eprintln!("REPLAY {r:?}");
    }
    assert!(
        replays.iter().all(|r| r
            .bands_in_seen
            .iter()
            .any(|m| m.contains("already showing"))),
        "each replay selected the buffer already shown: {replays:#?}"
    );
    assert!(
        passing.is_empty(),
        "review 1's row would have passed with nothing switched in {} of {}: {passing:#?}",
        passing.len(),
        replays.len()
    );
}
