//! E8b --- reaching a buffer by name, through a real daemon: keys sent
//! as a frontend sends them (`FrontendEvent::Key` into the daemon's own
//! dispatcher), and what the window shows read from what the daemon
//! writes the session. `CursorByte` names the window's buffer ("the
//! active buffer for this frontend is `buffer_id`"); a session learns
//! that buffer's name from the `StatusFacts` of a viewport declared on
//! it, as `pmacs-gpu` declares one after each snapshot; and
//! `StatusFacts.message` is the status band.
//!
//! E8b.3's probe. In E8 review 1's daemon session typing a buffer's full
//! path to `C-x b` was seen not to switch the window, which a ranking
//! fix cannot explain (#318's last paragraph). It reproduces, and not
//! through ranking: that session's window already showed the file it
//! typed. A no-target attach replicates every buffer in registry order,
//! review 1's harness declared its one viewport on the first snapshot
//! (`b.rs`, visited first), and a `Viewport` aligns the window to the
//! buffer it names (`align_primary_document_window`). So the window left
//! `a.rs`, the buffer the init showed, for `b.rs` before any key, and
//! `C-x b <b.rs's path> RET` selected the buffer already shown and said
//! nothing. The first row is that session; the second is the same keys
//! where the session declares the buffer its window shows. The harness's
//! declaration is #324's, filed with the scope its fix needs; the third
//! row is E8b.1's bare name through the daemon.
#![cfg(feature = "crdt")]

mod common;

use std::collections::HashMap;
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

/// `main` starts at byte 3, so the caret at byte 0 sits inside the
/// fake's hover range.
const SOURCE: &str = "fn main() { let value = 1; }\n";

/// Which buffer a session declares its first viewport on.
#[derive(Clone, Copy)]
enum Declare {
    /// The first `BufferSnapshot` of the bootstrap, as E8 review 1's
    /// harness did (`tests/e8_review1_probes.rs`, `Session::attach`).
    FirstSnapshot,
    /// The buffer the first `CursorByte` names: the window's own.
    Window,
}

/// One attached semantic session: its writer, a channel of everything
/// the daemon wrote it, its frontend id, the buffer its viewport is
/// declared on, and the names `StatusFacts` has told it.
struct Session {
    stream: UnixStream,
    rx: mpsc::Receiver<InstanceMessage>,
    fid: FrontendId,
    viewport: BufferId,
    names: HashMap<BufferId, String>,
}

impl Session {
    /// Attach at the current protocol with no initial target, declare a
    /// viewport as `declare` says, and start the reader thread.
    fn attach(daemon: &TestDaemon, declare: Declare) -> Self {
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
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        let mut session = Self {
            stream,
            rx,
            fid,
            viewport: BufferId::from_raw(0),
            names: HashMap::new(),
        };
        let first = match declare {
            Declare::FirstSnapshot => session.wait("the first BufferSnapshot", |msg| match msg {
                InstanceMessage::BufferSnapshot { buffer_id, .. } => Some(*buffer_id),
                _ => None,
            }),
            Declare::Window => session.wait("the first CursorByte", |msg| match msg {
                InstanceMessage::CursorByte { buffer_id, .. } => Some(*buffer_id),
                _ => None,
            }),
        };
        session.declare(first);
        session
    }

    /// Declare a viewport on `buffer`, which also aligns the window to it.
    fn declare(&mut self, buffer: BufferId) {
        write_message(
            &mut self.stream,
            &FrontendEvent::Viewport {
                frontend_id: self.fid,
                buffer_id: buffer,
                visible: ByteRange { start: 0, end: 0 },
                generation: 0,
            },
        )
        .expect("declare a viewport");
        self.viewport = buffer;
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

    /// Read the session's messages, following the window, until `done`
    /// holds of what was seen (within 20 s), then half a second more for
    /// what follows it; or for `window` when `done` is `None`. A
    /// `CursorByte` naming a buffer other than the declared one is
    /// answered with a viewport on it (as `pmacs-gpu` declares one after
    /// the snapshot a switch brings), and every `StatusFacts` teaches a
    /// buffer's name. Returns the window's buffer, by name once known,
    /// and every status band said, in order.
    fn watch(&mut self, window: Duration, done: Option<&dyn Fn(&Shown) -> bool>) -> Shown {
        let mut deadline = Instant::now() + done.map_or(window, |_| Duration::from_secs(20));
        let mut settled = false;
        let mut shown = None;
        let mut bands: Vec<String> = Vec::new();
        let seen =
            |names: &HashMap<BufferId, String>, shown: Option<BufferId>, bands: &[String]| Shown {
                name: shown.map(|id| {
                    names
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| format!("{id:?}, name not yet told"))
                }),
                bands: bands.to_vec(),
            };
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.rx.recv_timeout(left) {
                Ok(InstanceMessage::CursorByte { buffer_id, .. }) => {
                    if buffer_id != self.viewport {
                        self.declare(buffer_id);
                    }
                    shown = Some(buffer_id);
                }
                Ok(InstanceMessage::StatusFacts {
                    buffer_id,
                    name,
                    message,
                    ..
                }) => {
                    self.names.insert(buffer_id, name);
                    if let Some(m) = message
                        && bands.last() != Some(&m)
                    {
                        bands.push(m);
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
            if !settled && done.is_some_and(|d| d(&seen(&self.names, shown, &bands))) {
                settled = true;
                deadline = Instant::now() + Duration::from_millis(500);
            }
        }
        seen(&self.names, shown, &bands)
    }

    /// `C-c h` until the hover popup is up.
    fn hover_until_present(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            self.ctrl('c');
            self.key(Key::Char('h'), Modifiers::NONE);
            let until = Instant::now() + Duration::from_millis(500);
            while let Some(left) = until.checked_duration_since(Instant::now()) {
                match self.rx.recv_timeout(left) {
                    Ok(InstanceMessage::Popup(PopupPayload::Present(_))) => return,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
        panic!("no hover popup arrived for C-c h");
    }

    /// The window's buffer by name, once a `StatusFacts` has told it.
    fn shown(&mut self) -> Shown {
        self.watch(
            Duration::ZERO,
            Some(&|v: &Shown| {
                v.name
                    .as_ref()
                    .is_some_and(|n| !n.ends_with("not yet told"))
            }),
        )
    }

    /// `C-x b`, half a second for the prompt, then `text` and RET, as E8
    /// review 1's row sent them; then what was seen until the window
    /// shows `want` and the band has spoken.
    fn switch_by_typing(&mut self, text: &str, want: &str) -> Shown {
        self.ctrl('x');
        self.typed("b");
        let _ = self.watch(Duration::from_millis(500), None);
        self.typed(text);
        self.key(Key::Enter, Modifiers::NONE);
        self.watch(
            Duration::ZERO,
            Some(&|v: &Shown| v.name.as_deref() == Some(want) && !v.bands.is_empty()),
        )
    }
}

/// What a session saw: the window's buffer by name, and the status band.
#[derive(Debug)]
struct Shown {
    name: Option<String>,
    bands: Vec<String>,
}

/// An `init.lua` attaching the fake server to `.rs` files, visiting
/// `files` in order (the last one shown), `*scratch*` killed.
fn init_for(files: &[&Path]) -> String {
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
         end\n"
    )
}

/// E8 review 1's fixture: a daemon visiting `b.rs` then `a.rs`, both
/// holding [`SOURCE`], in a cargo project in the directory `sub` of a
/// fresh temporary one, semantic and multi-frontend. Returns the two
/// paths as the daemon was handed them.
fn daemon_with(sub: &str) -> (TestDaemon, tempfile::TempDir, String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join(sub);
    std::fs::create_dir_all(&root).expect("project dir");
    std::fs::write(root.join("Cargo.toml"), b"[package]\nname=\"x\"\n").expect("write");
    let a = root.join("a.rs");
    let b = root.join("b.rs");
    std::fs::write(&a, SOURCE).expect("write a.rs");
    std::fs::write(&b, SOURCE).expect("write b.rs");
    let daemon = TestDaemon::spawn_with_env_and_init(
        &[
            ("PMACS_INSTANCE_SEMANTIC_RENDER", "1"),
            ("PMACS_INSTANCE_MULTI_FRONTEND", "1"),
        ],
        &init_for(&[&b, &a]),
    );
    (
        daemon,
        dir,
        a.display().to_string(),
        b.display().to_string(),
    )
}

/// E8b.3, E8 review 1's session as it was: its harness declares the
/// first snapshot's buffer, `b.rs`, so the window shows `b.rs` before
/// any key, though the init left `a.rs` shown; the hover opens there;
/// and `C-x b <b.rs's full path> RET` selects the buffer already shown.
/// The window stays, as review 1's daemon session saw, and the band now
/// says so (E8b.2) where it said nothing.
#[test]
fn e8b_3_review1_s_session_already_showed_the_file_whose_path_it_typed() {
    let (daemon, _dir, a, b) = daemon_with("");
    let mut s = Session::attach(&daemon, Declare::FirstSnapshot);
    let before = s.shown();
    assert_eq!(
        before.name.as_deref(),
        Some(b.as_str()),
        "the mechanism: declaring the first snapshot put b.rs in the window, not a.rs ({a})"
    );
    s.hover_until_present();
    let after = s.switch_by_typing(&b, &b);
    assert_eq!(after.name.as_deref(), Some(b.as_str()), "{after:?}");
    assert_eq!(
        after.bands,
        vec![format!("switch-buffer: already showing {b}")],
        "the band says the selection is the buffer already shown"
    );
}

/// E8b.3's control: the same keys under the open popup, where the
/// session declares the buffer its window shows. `a.rs` is shown, and
/// `C-x b <b.rs's full path> RET` puts `b.rs` in the window and says so.
#[test]
fn e8b_3_the_full_path_switches_a_daemon_window_under_an_open_popup() {
    let (daemon, _dir, a, b) = daemon_with("");
    let mut s = Session::attach(&daemon, Declare::Window);
    let before = s.shown();
    assert_eq!(before.name.as_deref(), Some(a.as_str()), "{before:?}");
    s.hover_until_present();
    let after = s.switch_by_typing(&b, &b);
    assert_eq!(after.name.as_deref(), Some(b.as_str()), "{after:?}");
    assert_eq!(after.bands, vec![format!("switch-buffer: showing {b}")]);
}

/// E8b.1 through the daemon: under a directory called `bin-tests`,
/// `b.rs` is also a subsequence of `a.rs`'s path, and `C-x b b.rs RET`
/// from `a.rs` reaches `b.rs`.
#[test]
fn e8b_1_a_bare_name_reaches_its_file_in_a_daemon_window() {
    let (daemon, _dir, a, b) = daemon_with("bin-tests");
    let mut s = Session::attach(&daemon, Declare::Window);
    let before = s.shown();
    assert_eq!(before.name.as_deref(), Some(a.as_str()), "{before:?}");
    let after = s.switch_by_typing("b.rs", &b);
    assert_eq!(after.name.as_deref(), Some(b.as_str()), "{after:?}");
    assert_eq!(after.bands, vec![format!("switch-buffer: showing {b}")]);
}
