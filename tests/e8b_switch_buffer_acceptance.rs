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
//!
//! The last row is E8b fix round 1's: the same daemon driven by the GPU
//! frontend itself (`pmacs-gpu --headless-probe`, every key through
//! `App::apply_keyboard`), reaching a dired buffer by its own name in
//! one session with the bare name and D18's four.
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
/// declared on and whether the window is known to show it, and the
/// names `StatusFacts` has told it.
struct Session {
    stream: UnixStream,
    rx: mpsc::Receiver<InstanceMessage>,
    fid: FrontendId,
    viewport: BufferId,
    aligned: bool,
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
            aligned: false,
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
        // A `CursorByte` names the window's buffer, so a declaration on
        // it is aligned at once; the first snapshot's buffer is not, until
        // the window is seen on it.
        session.declare(first, matches!(declare, Declare::Window));
        session
    }

    /// Declare a viewport on `buffer`, which also aligns the window to it;
    /// `aligned` when the window is already known to show it.
    fn declare(&mut self, buffer: BufferId, aligned: bool) {
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
        self.aligned = aligned;
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
    /// buffer's name. Until the window is seen on a declared buffer it was
    /// not known to show, a `CursorByte` naming another is cursor traffic
    /// the daemon sent before it read the declaration: it is counted as
    /// stale, not followed. Following it put the window back on the
    /// init's buffer in 9 of 48 runs of the first-snapshot row at
    /// `0001946`, eight at a time; ignoring it, 48 of 48 passed, and of 48
    /// that counted them 8 met one (E8b fix round 1). Returns the window's
    /// buffer, by name once known, every status band said, in order, and
    /// the stale count.
    fn watch(&mut self, window: Duration, done: Option<&dyn Fn(&Shown) -> bool>) -> Shown {
        let mut deadline = Instant::now() + done.map_or(window, |_| Duration::from_secs(20));
        let mut settled = false;
        let mut shown = None;
        let mut bands: Vec<String> = Vec::new();
        let mut stale = 0;
        let seen = |names: &HashMap<BufferId, String>,
                    shown: Option<BufferId>,
                    bands: &[String],
                    stale: usize| Shown {
            name: shown.map(|id| {
                names
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| format!("{id:?}, name not yet told"))
            }),
            bands: bands.to_vec(),
            stale,
        };
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.rx.recv_timeout(left) {
                Ok(InstanceMessage::CursorByte { buffer_id, .. }) => {
                    if buffer_id == self.viewport {
                        self.aligned = true;
                    } else if self.aligned {
                        self.declare(buffer_id, true);
                    } else {
                        stale += 1;
                        continue;
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
            if !settled && done.is_some_and(|d| d(&seen(&self.names, shown, &bands, stale))) {
                settled = true;
                deadline = Instant::now() + Duration::from_millis(500);
            }
        }
        seen(&self.names, shown, &bands, stale)
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

/// What a session saw: the window's buffer by name, the status band, and
/// how many `CursorByte`s it took as stale.
#[derive(Debug)]
struct Shown {
    name: Option<String>,
    bands: Vec<String>,
    stale: usize,
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
        "the mechanism: declaring the first snapshot put b.rs in the window, not a.rs ({a}); \
         {} cursor(s) sent before the declaration was read set aside",
        before.stale
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

/// The own-name session through a daemon (E8b fix round 1), laid out as
/// `tests/e8b_own_name_acceptance.rs` lays it out: `bin-tests` holding
/// `a.rs`, `b.rs` and `notes.txt`, `proj` holding `dired.lua`,
/// `src/lsp.rs` and `editor.rs` (shown), both in `nightowls`, which
/// spells `nts` loosely; `*scratch*` kept, no language server. The init
/// binds `C-c z` to `probe.where`, which writes `WHERE <the window's
/// buffer>` to the band: an oracle for the window that does not read
/// E8b.2's message. Returns the daemon and the paths as handed to it.
fn own_name_daemon() -> (TestDaemon, tempfile::TempDir, HashMap<&'static str, String>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("nightowls");
    std::fs::create_dir_all(root.join("bin-tests")).expect("bin-tests");
    std::fs::create_dir_all(root.join("proj/src")).expect("proj/src");
    let mut paths = HashMap::new();
    let mut visits = String::new();
    for (key, rel) in [
        ("a", "bin-tests/a.rs"),
        ("b", "bin-tests/b.rs"),
        ("notes", "bin-tests/notes.txt"),
        ("lua", "proj/dired.lua"),
        ("lsp_rs", "proj/src/lsp.rs"),
        ("editor", "proj/editor.rs"),
    ] {
        let path = root.join(rel);
        std::fs::write(&path, b"x\n").expect("write");
        let shown = path.display().to_string();
        let _ = writeln!(visits, "pmacs.buffer.find_or_open({shown:?})");
        paths.insert(key, shown);
    }
    let init = format!(
        "pmacs.lsp.config = {{}}\n\
         {visits}\
         pmacs.command.define {{\n\
           name = 'probe.where',\n\
           description = 'Say which buffer the window shows.',\n\
           fn = function()\n\
             pmacs.editor.set_status('WHERE ' .. pmacs.describe.buffer(pmacs.window.buffer()).name)\n\
           end,\n\
         }}\n\
         pmacs.keymap.bind {{ scope = 'global', sequence = 'C-c z', command = 'probe.where' }}\n"
    );
    let daemon = TestDaemon::spawn_with_env_and_init(
        &[
            ("PMACS_INSTANCE_SEMANTIC_RENDER", "1"),
            ("PMACS_INSTANCE_MULTI_FRONTEND", "1"),
        ],
        &init,
    );
    (daemon, dir, paths)
}

/// Run `pmacs-gpu --headless-probe` under the popup action with `steps`:
/// every `key:` step through `App::apply_keyboard`, the production key
/// path, and every `report:<label>` writing the band as
/// `<label>.status`. `None` when skipped (no `pmacs-gpu` binary or no
/// wgpu adapter), which panics under `PMACS_REQUIRE_GPU` so the gate's
/// GPU leg never passes vacuously.
fn run_gpu_probe(daemon: &TestDaemon, steps: &[String]) -> Option<HashMap<String, String>> {
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
    let report = daemon
        .socket_path()
        .parent()
        .expect("socket parent")
        .join("own-name.report");
    let output = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(daemon.socket_path())
        .arg(&report)
        .env("PMACS_GPU_PROBE_TYPE_TEXT", steps.join(" "))
        .env("PMACS_GPU_PROBE_ACTION", "popup")
        .env("PMACS_GPU_PROBE_DEADLINE_MS", "60000")
        .output()
        .expect("run the headless GPU probe");
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let no_adapter = output.status.code() == Some(3);
        assert!(
            no_adapter && !required,
            "headless GPU probe failed (status {:?}):\n{stderr}",
            output.status.code()
        );
        eprintln!("skipping the GPU probe: no wgpu adapter available");
        return None;
    }
    let text = std::fs::read_to_string(&report).expect("read the probe report");
    Some(
        text.lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
    )
}

/// Report the band as `label` once it begins with `prefix`. A key step
/// waits 250 ms, and a switch's message came later than that out of the
/// dired buffer, and later than 600 ms beside the suite's other rows.
fn report_steps(steps: &mut Vec<String>, prefix: &str, label: &str) {
    steps.push(format!("band:{prefix}"));
    steps.push(format!("report:{label}"));
}

/// `C-c z` reported as `<label>-where`: the window's buffer, by the
/// init's oracle.
fn where_steps(steps: &mut Vec<String>, label: &str) {
    steps.extend(["key:C-c".to_owned(), "key:z".to_owned()]);
    report_steps(steps, "WHERE", &format!("{label}-where"));
}

/// `C-x b`, `text`, RET reported as `label` once the band begins with
/// `said`, then [`where_steps`].
fn switch_steps(steps: &mut Vec<String>, text: &str, said: &str, label: &str) {
    steps.extend(["key:C-x".to_owned(), "key:b".to_owned()]);
    steps.extend(text.chars().map(|c| format!("key:{c}")));
    steps.push("key:ret".to_owned());
    report_steps(steps, said, label);
    where_steps(steps, label);
}

/// The band a report wrote under `label`, as the probe quotes it.
fn band(facts: &HashMap<String, String>, label: &str) -> String {
    facts
        .get(&format!("{label}.status"))
        .unwrap_or_else(|| panic!("the report has no {label}.status: {facts:?}"))
        .clone()
}

/// E8b fix round 1's GPU row: review 1's Medium 1 on the GPU frontend,
/// in one session with the bare name and D18's four, every key through
/// `App::apply_keyboard`. `C-x b dired RET` and `C-x b dir RET` reach the
/// dired buffer, `b.rs` reaches `b.rs` beside `a.rs`, `nts` reaches
/// `notes.txt` with the dired buffer's name holding `nts` too, `scr`
/// `*scratch*`, `lsp` `*lsp*` beside `src/lsp.rs`, and `zzz` is refused
/// with the window unchanged. Each switch is read twice: the band E8b.2
/// writes, and `C-c z`'s oracle for the window.
#[test]
fn e8b_fr1_the_gpu_reaches_a_dired_buffer_by_its_own_name_and_keeps_d18_s_four() {
    let (daemon, _dir, paths) = own_name_daemon();
    // The probe is ready once one snapshot has landed, and the GPU
    // declares a viewport on each bootstrap snapshot as it applies it,
    // which moves its window; where it settles varies from run to run,
    // so the row lets the bootstrap finish and then puts `editor.rs` in
    // the window by its name.
    let mut steps = vec!["quiet:2000".to_owned()];
    switch_steps(&mut steps, "editor.rs", "switch-buffer:", "start");
    steps.extend(["key:C-x", "key:d", "key:ret", "quiet:1500"].map(str::to_owned));
    where_steps(&mut steps, "dired-open");
    for (text, label) in [
        ("editor.rs", "editor"),
        ("dired", "dired"),
        ("editor.rs", "editor-2"),
        ("dir", "dir"),
        ("a.rs", "a"),
        ("b.rs", "bare"),
        ("nts", "nts"),
        ("scr", "scr"),
    ] {
        switch_steps(&mut steps, text, "switch-buffer:", label);
    }
    steps.extend(["key:C-c", "key:l", "quiet:500"].map(str::to_owned));
    where_steps(&mut steps, "lsp-open");
    switch_steps(&mut steps, "scr", "switch-buffer:", "scr-2");
    switch_steps(&mut steps, "lsp", "switch-buffer:", "lsp");
    switch_steps(&mut steps, "zzz", "no", "zzz");
    let Some(facts) = run_gpu_probe(&daemon, &steps) else {
        return;
    };
    assert_eq!(fact_or(&facts, "ready"), "true", "{facts:?}");
    assert_eq!(fact_or(&facts, "disconnect"), "", "{facts:?}");
    let where_ = |name: &str| format!("{:?}", format!("WHERE {name}"));
    let showing = |name: &str| format!("{:?}", format!("switch-buffer: showing {name}"));
    assert_eq!(band(&facts, "start-where"), where_(&paths["editor"]));
    assert!(
        band(&facts, "start").ends_with(&format!("showing {}\"", paths["editor"])),
        "{facts:?}"
    );
    let opened = band(&facts, "dired-open-where");
    let dired = opened
        .trim_matches('"')
        .strip_prefix("WHERE ")
        .filter(|n| n.starts_with("*dired:") && n.ends_with("/proj*"))
        .unwrap_or_else(|| panic!("fixture: C-x d RET opened a dired buffer; {opened}"))
        .to_owned();
    let expect = [
        ("editor", paths["editor"].clone()),
        ("dired", dired.clone()),
        ("editor-2", paths["editor"].clone()),
        ("dir", dired.clone()),
        ("a", paths["a"].clone()),
        ("bare", paths["b"].clone()),
        ("nts", paths["notes"].clone()),
        ("scr", "*scratch*".to_owned()),
        ("scr-2", "*scratch*".to_owned()),
        ("lsp", "*lsp*".to_owned()),
    ];
    for (label, name) in &expect {
        assert_eq!(
            (band(&facts, label), band(&facts, &format!("{label}-where"))),
            (showing(name), where_(name)),
            "{label}: the switch reached {name}; {facts:?}"
        );
    }
    assert_eq!(band(&facts, "lsp-open-where"), where_("*lsp*"));
    assert_eq!(band(&facts, "zzz"), format!("{:?}", "no buffer: zzz"));
    assert_eq!(
        band(&facts, "zzz-where"),
        where_("*lsp*"),
        "zzz switched nothing"
    );
    assert!(
        !facts.keys().any(|k| k.starts_with("step.")),
        "every step settled: {facts:?}"
    );
}

/// A report's top-level fact, `""` when absent.
fn fact_or<'a>(facts: &'a HashMap<String, String>, key: &str) -> &'a str {
    facts.get(key).map_or("", String::as_str)
}

/// `C-x k` from a window on `*scratch*`, its prefill deleted (`C-a` and
/// nine `C-d`), `text`, RET.
fn kill_steps(steps: &mut Vec<String>, text: &str) {
    steps.extend(["key:C-x", "key:k", "key:C-a"].map(str::to_owned));
    steps.extend(std::iter::repeat_n("key:C-d".to_owned(), "*scratch*".len()));
    steps.extend(text.chars().map(|c| format!("key:{c}")));
    steps.push("key:ret".to_owned());
}

/// E8b fix round 1's GPU row for review 1's Medium 2: `C-x k` by name
/// through `App::apply_keyboard`. `C-x k dired RET` kills the dired
/// buffer and the band names it; `dired.lua` is still there to switch
/// to. Until E8c the listing's generated text read as modified and this
/// row answered `y` to "kill anyway?"; E8c leaves the editor's own text
/// clean (#299), so it goes without a question. `C-x k b.rs RET` kills
/// `b.rs` beside `a.rs`, unmodified, without a question, and names it;
/// `a.rs` is still there.
#[test]
fn e8b_fr1_the_gpu_kills_the_buffer_its_own_name_names_and_says_so() {
    let (daemon, _dir, paths) = own_name_daemon();
    let mut steps = vec!["quiet:2000".to_owned()];
    switch_steps(&mut steps, "editor.rs", "switch-buffer:", "editor");
    steps.extend(["key:C-x", "key:d", "key:ret", "quiet:1500"].map(str::to_owned));
    where_steps(&mut steps, "dired-open");
    switch_steps(&mut steps, "scr", "switch-buffer:", "scr");
    kill_steps(&mut steps, "dired");
    report_steps(&mut steps, "kill-buffer:", "kill-dired");
    where_steps(&mut steps, "kill-dired");
    switch_steps(&mut steps, "dired.lua", "switch-buffer:", "lua");
    switch_steps(&mut steps, "scr", "switch-buffer:", "scr-2");
    kill_steps(&mut steps, "b.rs");
    report_steps(&mut steps, "kill-buffer:", "kill-b");
    where_steps(&mut steps, "kill-b");
    switch_steps(&mut steps, "a.rs", "switch-buffer:", "a");
    let Some(facts) = run_gpu_probe(&daemon, &steps) else {
        return;
    };
    assert_eq!(fact_or(&facts, "ready"), "true", "{facts:?}");
    assert_eq!(fact_or(&facts, "disconnect"), "", "{facts:?}");
    let quoted = |text: String| format!("{text:?}");
    let opened = band(&facts, "dired-open-where");
    let dired = opened
        .trim_matches('"')
        .strip_prefix("WHERE ")
        .filter(|n| n.starts_with("*dired:") && n.ends_with("/proj*"))
        .unwrap_or_else(|| panic!("fixture: C-x d RET opened a dired buffer; {opened}"))
        .to_owned();
    assert_eq!(
        band(&facts, "kill-dired"),
        quoted(format!("kill-buffer: killed {dired}")),
        "C-x k dired RET kills the dired buffer; {facts:?}"
    );
    assert_eq!(
        band(&facts, "kill-dired-where"),
        quoted("WHERE *scratch*".to_owned())
    );
    assert_eq!(
        band(&facts, "lua"),
        quoted(format!("switch-buffer: showing {}", paths["lua"])),
        "dired.lua was kept; {facts:?}"
    );
    assert_eq!(
        band(&facts, "kill-b"),
        quoted(format!("kill-buffer: killed {}", paths["b"])),
        "C-x k b.rs RET kills b.rs; {facts:?}"
    );
    assert_eq!(
        band(&facts, "a"),
        quoted(format!("switch-buffer: showing {}", paths["a"])),
        "a.rs was kept; {facts:?}"
    );
    assert!(
        !facts.keys().any(|k| k.starts_with("step.")),
        "every step settled: {facts:?}"
    );
}
