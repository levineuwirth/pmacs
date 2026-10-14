// tests/e8d_review1_probes.rs --- E8d review 1's behavioral probes.

//! The states a user reaches around E8d's six rows that the phase's
//! witnesses (`tests/e8d_drift_acceptance.rs`) did not choose. Each row
//! names the fix it probes; a row that fails at `d628b21` is a finding of
//! the review pass, and the fix round commits every row as a witness
//! rather than running and restoring it.
//!
//! - P1 (E8d.2): redo, which has no row of its own, carries an overlay's
//!   byte offset forward again.
//! - P2 (E8d.3): the SHIPPED help writer (`help.list-commands`, through
//!   the Lua `_show_help`) leaves `*help*` with no history, clamps a
//!   window's cursor on a shorter re-render, and `q` still leaves.
//! - P3 (E8d.3): the shipped writer's one `Replace` reaches a CRDT
//!   replica (the phase's replica row drove the Rust renderer, which no
//!   shipped command binds).
//! - P4 (E8d.1): the exit the fix names --- the user stopping the server
//!   --- from the respawn loop: does `stop` reach a terminal state the
//!   way the MCP copy's does?
//! - P5 (E8d.5): the two helpers `lsp.lua` exports, at the root, and the
//!   outermost rule under a `/` boundary.
//! - P6 (E8d.6): `q` in a side-window `*search-results*` whose quit
//!   action is `restore`, the shape the phase's row did not drive.

#![cfg(unix)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::async_runtime::AsyncRuntime;
use pmacs::editor::EditorState;
use pmacs::lsp::{
    LspClientState, LspEvent, LspEventKind, LspManager, LspRestartPolicy, LspServerId,
    LspServerSpec,
};
use pmacs::protocol::FrontendId;

#[path = "common/mod.rs"]
mod common;

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn ctrl(s: &mut EditorState, c: char) {
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char(c), KeyModifiers::CONTROL),
    );
}

fn alt(s: &mut EditorState, c: char) {
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::ALT));
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        press(s, KeyCode::Char(ch));
    }
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn active_text(s: &EditorState) -> String {
    eval(
        s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    )
}

fn active_name(s: &EditorState) -> String {
    eval(
        s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    )
}

fn named_text(s: &EditorState, name: &str) -> String {
    eval(
        s,
        &format!(
            "for _, id in ipairs(pmacs.buffer.list()) do
               if pmacs.describe.buffer(id).name == {name:?} then
                 return id:slice(0, id:len())
               end
             end
             return ''"
        ),
    )
}

fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&common::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    exec(&s, "pmacs.lsp.config = {}");
    s.sync_frame_geometry(FrontendId::LOCAL, pmacs::cell::CellSize::new(24, 80));
    s
}

fn undo(s: &mut EditorState) {
    ctrl(s, 'x');
    press(s, KeyCode::Char('u'));
}

/// `C-x r`, `buffer.redo`'s chord.
fn redo(s: &mut EditorState) {
    ctrl(s, 'x');
    press(s, KeyCode::Char('r'));
}

fn run_m_x(s: &mut EditorState, name: &str) {
    alt(s, 'x');
    type_str(s, name);
    press(s, KeyCode::Enter);
}

// ---------------------------------------------------------------------------
// P1 --- E8d.2, redo
// ---------------------------------------------------------------------------

struct OffsetOverlay {
    at: Arc<Mutex<u64>>,
}

impl pmacs::view::View for OffsetOverlay {
    fn on_edit(
        &mut self,
        _buf: &pmacs::buffer::Buffer,
        edit: &pmacs::rope::Edit,
    ) -> Result<(), pmacs::buffer::BufferError> {
        let mut at = self.at.lock().expect("offset mutex");
        let (start, end, new_len) = (edit.range.start, edit.range.end, edit.inserted_len);
        if *at >= end {
            *at = *at - (end - start) + new_len;
        } else if *at > start {
            *at = start;
        }
        Ok(())
    }

    fn kind(&self) -> &'static str {
        "e8d_review1_offset_overlay"
    }
}

/// `EditorCore::redo` has no row of its own in the phase's suite; its
/// body is the same two calls as undo's. An overlay carried back by the
/// undo is carried forward again by the redo.
#[test]
fn p1_redo_carries_an_overlay_s_byte_offset_forward_again() {
    let mut s = editor();
    type_str(&mut s, "0123456789");
    let at = Arc::new(Mutex::new(5_u64));
    s.core
        .borrow_mut()
        .active_window_mut()
        .push_overlay(Box::new(OffsetOverlay { at: at.clone() }));
    ctrl(&mut s, 'a');
    type_str(&mut s, "ab");
    assert_eq!(*at.lock().unwrap(), 7, "premise: carried forward");
    undo(&mut s);
    assert_eq!(active_text(&s), "0123456789", "premise: undone");
    assert_eq!(
        *at.lock().unwrap(),
        5,
        "premise: carried back (E8d.2's row)"
    );

    redo(&mut s);
    assert_eq!(
        active_text(&s),
        "ab0123456789",
        "premise: the redo re-applied the typing"
    );
    assert_eq!(
        *at.lock().unwrap(),
        7,
        "the redo is told to every overlay, so the offset is carried forward again"
    );
}

// ---------------------------------------------------------------------------
// P2 --- E8d.3, the shipped writer in process
// ---------------------------------------------------------------------------

/// Every shipped help command writes `*help*` through the Lua
/// `_show_help`. After `help.list-commands`: `C-x u` leaves the text
/// (history cleared by the primitive); with the cursor at the end of the
/// long listing a shorter re-render through the same writer clamps it
/// (the Lua binding's fan-out is `notify_buffer_edit`); and `q`, the
/// buffer-scope binding, still leaves to the previous buffer although the
/// buffer is read-only.
#[test]
fn p2_help_through_the_shipped_writer_has_no_history_clamps_and_q_leaves() {
    let mut s = editor();
    run_m_x(&mut s, "help.list-commands");
    assert_eq!(active_name(&s), "*help*", "premise: the listing shows");
    let shown = named_text(&s, "*help*");
    assert!(
        shown.contains("help.list-commands"),
        "premise: the listing names its own command; got {shown:?}"
    );

    undo(&mut s);
    assert_eq!(
        named_text(&s, "*help*"),
        shown,
        "C-x u after the shipped writer leaves the listing intact"
    );

    alt(&mut s, '>');
    let at_end = s.core.borrow().active_window().cursor;
    assert_eq!(
        at_end,
        shown.len() as u64,
        "premise: the cursor is at the end"
    );
    exec(&s, "pmacs.editor._show_help('short\\n')");
    assert_eq!(
        named_text(&s, "*help*"),
        "short\n",
        "premise: re-rendered shorter"
    );
    let cursor = s.core.borrow().active_window().cursor;
    assert!(
        cursor <= 6,
        "a shorter re-render through the shipped writer clamps the window's cursor; it is {cursor}"
    );

    press(&mut s, KeyCode::Char('q'));
    assert_eq!(
        active_name(&s),
        "*scratch*",
        "`q` in the read-only *help* still leaves to the previous buffer"
    );
}

// ---------------------------------------------------------------------------
// P4 --- E8d.1, the exit the fix names
// ---------------------------------------------------------------------------

fn lsp_manager() -> (
    pmacs::lua_bindings::SharedProcessSupervisor,
    pmacs::lsp::SharedLspManager,
) {
    let sup = Rc::new(RefCell::new(pmacs::process::ProcessSupervisor::new()));
    let runtime = Rc::new(AsyncRuntime::with_pool_size(1));
    let mgr = Rc::new(RefCell::new(LspManager::new(sup.clone(), runtime)));
    (sup, mgr)
}

fn drain_lsp_until<F: Fn(&[LspEvent]) -> bool>(
    sup: &pmacs::lua_bindings::SharedProcessSupervisor,
    mgr: &pmacs::lsp::SharedLspManager,
    sid: LspServerId,
    seen: &mut Vec<LspEvent>,
    deadline: Duration,
    pred: F,
) -> bool {
    let stop = Instant::now() + deadline;
    loop {
        sup.borrow_mut().tick();
        mgr.borrow_mut().tick();
        let mut evs = mgr.borrow_mut().take_events(sid);
        seen.append(&mut evs);
        if pred(seen) {
            return true;
        }
        if Instant::now() >= stop {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn respawn_failed(evs: &[LspEvent]) -> bool {
    evs.iter().any(|e| {
        matches!(&e.kind, LspEventKind::Crashed { reason } if reason.starts_with("respawn failed: "))
    })
}

/// A server whose executable is gone loops `Restarting`/`Crashed` every
/// backoff (E8d.1's chosen behavior, "bounded only by the user stopping
/// the server"). The row drives that exit: `pmacs.lsp.stop` on the
/// looping server. `McpManager::stop` on a `Crashed` client disarms the
/// restart and leaves the client terminal, so `forget` can take it; this
/// row asks the LSP copy for the same, and measures the loop's event
/// rate at the default backoff on the way.
#[test]
fn p4_stopping_a_server_whose_respawn_failed_reaches_a_terminal_state() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let exe = dir.path().join("dying-server");
    std::fs::write(&exe, "#!/bin/sh\nexit 3\n").expect("write the server");
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let (sup, mgr) = lsp_manager();
    // The DEFAULT backoff: the rate a user's `*lsp*` sees.
    let mut spec = LspServerSpec::new("dying", "rust", exe.to_str().expect("utf-8 path"));
    spec.restart = LspRestartPolicy::OnCrash;
    let sid = mgr.borrow_mut().spawn(spec).expect("spawn the server");
    let mut seen = Vec::new();
    assert!(
        drain_lsp_until(&sup, &mgr, sid, &mut seen, Duration::from_secs(10), |evs| {
            evs.iter()
                .any(|e| matches!(e.kind, LspEventKind::Crashed { .. }))
        }),
        "premise: the first generation crashes"
    );
    std::fs::remove_file(&exe).expect("remove the server");
    assert!(
        drain_lsp_until(
            &sup,
            &mgr,
            sid,
            &mut seen,
            Duration::from_secs(5),
            respawn_failed
        ),
        "premise: the respawn fails and is told (E8d.1's row)"
    );
    let before = seen.len();
    let _ = drain_lsp_until(&sup, &mgr, sid, &mut seen, Duration::from_secs(3), |_| {
        false
    });
    let during = seen.len() - before;
    eprintln!(
        "P4 measured: {during} events in 3 s at the default backoff while the executable is gone"
    );
    assert!(
        during >= 4,
        "premise: the loop is live at the default backoff ({during} events in 3 s)"
    );

    // The exit the fix names.
    mgr.borrow_mut().stop(sid).expect("stop the looping server");
    let _ = drain_lsp_until(&sup, &mgr, sid, &mut seen, Duration::from_secs(1), |_| {
        false
    });
    let state = mgr.borrow().state(sid).cloned();
    let terminal = matches!(
        &state,
        None | Some(LspClientState::Stopped { .. } | LspClientState::Crashed { .. })
    );
    assert!(
        terminal,
        "after `stop`, the server is gone or terminal (Stopped/Crashed), as the MCP copy's \
         `stop` leaves a crashed client; it is {state:?}, which `forget` refuses and \
         `ensure_server_for_project` counts as healthy"
    );
    let forgotten = mgr.borrow_mut().forget(sid);
    assert!(
        forgotten.is_ok() || state.is_none(),
        "and `forget` takes it: {forgotten:?}"
    );
}

// ---------------------------------------------------------------------------
// P5 --- E8d.5, the exported helpers at the root
// ---------------------------------------------------------------------------

fn lua_str(path: &std::path::Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// The two names `lsp.lua` exports are pinned by nothing but lean's
/// `assert`. Their contract at the root: a top-level directory's parent
/// is `/`, `/` has none, and a `/` boundary contains everything. And the
/// outermost rule survives the `/` boundary: nested markers under it
/// root at the outer one.
#[test]
fn p5_lsp_lua_s_exported_walk_helpers_reach_the_root_and_keep_outermost() {
    let s = editor();
    let parent_of_tmp: Option<String> = eval(&s, "return pmacs.lsp.parent_of('/tmp')");
    assert_eq!(
        parent_of_tmp.as_deref(),
        Some("/"),
        "a top-level directory's parent is `/`"
    );
    let parent_of_root: Option<String> = eval(&s, "return pmacs.lsp.parent_of('/')");
    assert_eq!(
        parent_of_root, None,
        "`/` has no parent, so the walk terminates"
    );
    let root_in_root: bool = eval(&s, "return pmacs.lsp.within_boundary('/', '/')");
    assert!(root_in_root, "`/` is within the `/` boundary");
    let deep_in_root: bool = eval(&s, "return pmacs.lsp.within_boundary('/tmp/x', '/')");
    assert!(deep_in_root, "every path is within the `/` boundary");
    let no_boundary: bool = eval(&s, "return pmacs.lsp.within_boundary('/tmp', nil)");
    assert!(no_boundary, "no boundary contains everything");
    let outside: bool = eval(&s, "return pmacs.lsp.within_boundary('/tmpx', '/tmp')");
    assert!(!outside, "a prefix that is not a component is outside");

    let dir = tempfile::tempdir().expect("tempdir");
    let base = std::fs::canonicalize(dir.path()).expect("canonical");
    let outer = base.join("pkg");
    let inner = outer.join("sub");
    std::fs::create_dir_all(&inner).expect("dirs");
    std::fs::write(outer.join("lean-toolchain"), "leanprover/lean4:v4.9.0\n").expect("outer");
    std::fs::write(inner.join("lean-toolchain"), "leanprover/lean4:v4.9.0\n").expect("inner");
    let file = inner.join("A.lean");
    std::fs::write(&file, "def a := 1\n").expect("file");
    exec(&s, "pmacs.project.set_search_boundary(\"/\")");
    let root: String = eval(
        &s,
        &format!(
            "return tostring(pmacs.lean.root_for(\"{}\"))",
            lua_str(&file)
        ),
    );
    assert_eq!(
        root,
        outer.display().to_string(),
        "under a `/` boundary the OUTERMOST marker still wins"
    );
}

// ---------------------------------------------------------------------------
// P6 --- E8d.6, the restore quit action
// ---------------------------------------------------------------------------

fn pump_until(s: &mut EditorState, timeout: Duration, pred: impl Fn(&EditorState) -> bool) -> bool {
    let stop = Instant::now() + timeout;
    while !pred(s) {
        if Instant::now() >= stop {
            return false;
        }
        s.tick_processes();
        s.tick_async();
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}

fn side_window(s: &EditorState) -> Option<pmacs::window::WindowId> {
    s.core.borrow().side_window_for(FrontendId::LOCAL)
}

fn side_window_buffer_name(s: &EditorState) -> Option<String> {
    let wid = side_window(s)?;
    let core = s.core.borrow();
    let win = core.windows.get(&wid)?;
    let reg = core.registry.borrow();
    reg.get(win.buffer_id).ok().map(|b| b.name().to_string())
}

/// A bottom panel already showing `*scratch*` takes `*search-results*`
/// (the slot reused, so its quit action is `restore`): `q` routes
/// through `window.quit`, which restores the panel's previous
/// presentation --- the side window stands, showing `*scratch*` again
/// --- rather than switching the panel slot to the previous buffer.
#[test]
fn p6_q_in_search_results_restores_the_side_window_s_previous_presentation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = std::fs::canonicalize(dir.path()).expect("canonical");
    let notes = root.join("notes.txt");
    std::fs::write(&notes, "a needle here\n").expect("notes");
    let notes_name = notes.display().to_string();
    let mut s = editor();
    exec(
        &s,
        &format!("pmacs.buffer.find_or_open(\"{}\")", lua_str(&notes)),
    );
    assert_eq!(active_name(&s), notes_name, "premise: the file is shown");
    exec(
        &s,
        "for _, id in ipairs(pmacs.buffer.list()) do
           if pmacs.describe.buffer(id).name == '*scratch*' then
             pmacs.window.display(id, { side = 'bottom', select = false })
           end
         end",
    );
    assert_eq!(
        side_window_buffer_name(&s).as_deref(),
        Some("*scratch*"),
        "premise: the bottom panel shows *scratch*"
    );
    assert_eq!(
        active_name(&s),
        notes_name,
        "premise: focus stayed on the file"
    );

    exec(
        &s,
        &format!(
            "pmacs.project.search('needle', {{ root = \"{}\" }})",
            lua_str(&root)
        ),
    );
    assert_eq!(active_name(&s), "*search-results*", "premise: results show");
    assert!(
        pump_until(&mut s, Duration::from_secs(20), |s| {
            named_text(s, "*search-results*").contains("notes.txt")
        }),
        "premise: the search lists its match"
    );
    press(&mut s, KeyCode::Char('q'));
    assert_eq!(
        active_name(&s),
        notes_name,
        "premise: the full-window `q` restores the file"
    );

    exec(
        &s,
        "for _, id in ipairs(pmacs.buffer.list()) do
           if pmacs.describe.buffer(id).name == '*search-results*' then
             pmacs.window.display(id, { side = 'bottom', select = true })
           end
         end",
    );
    assert_eq!(
        side_window_buffer_name(&s).as_deref(),
        Some("*search-results*"),
        "premise: the panel slot now shows the results"
    );
    assert_eq!(
        active_name(&s),
        "*search-results*",
        "premise: and is selected"
    );
    let quit_action: Option<String> = eval(&s, "return pmacs.window.params().quit_action");
    assert_eq!(
        quit_action.as_deref(),
        Some("restore"),
        "premise: reusing the slot made the quit action `restore`"
    );

    press(&mut s, KeyCode::Char('q'));
    assert!(
        side_window(&s).is_some(),
        "`q` with a `restore` quit action keeps the side window"
    );
    assert_eq!(
        side_window_buffer_name(&s).as_deref(),
        Some("*scratch*"),
        "and restores the panel's previous presentation"
    );
    assert_ne!(
        active_name(&s),
        "*search-results*",
        "the results are no longer what the user is in"
    );
}

// ---------------------------------------------------------------------------
// P3 --- E8d.3, the shipped writer on a daemon with a replica
// ---------------------------------------------------------------------------

#[cfg(feature = "crdt")]
mod daemon_rows {
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    use pmacs::buffer::BufferId;
    use pmacs::crdt::CrdtState;
    use pmacs::protocol::{FrontendEvent, FrontendId, InstanceMessage, Key, KeyEvent, Modifiers};
    use pmacs::transport::{read_message, write_message};

    use super::common::daemon::{TestDaemon, attach_multi};

    struct Replica {
        stream: UnixStream,
        fid: FrontendId,
        document: BufferId,
    }

    fn attach(daemon: &TestDaemon) -> Replica {
        let (hello, mut stream) = attach_multi(daemon);
        let fid = hello.assigned_frontend_id;
        let document = match read_message::<InstanceMessage>(&mut stream)
            .expect("read the initial BufferSnapshot")
        {
            InstanceMessage::BufferSnapshot { buffer_id, .. } => buffer_id,
            other => panic!("expected the initial BufferSnapshot, got {other:?}"),
        };
        Replica {
            stream,
            fid,
            document,
        }
    }

    fn send_key(replica: &mut Replica, key: Key, mods: Modifiers) {
        write_message(
            &mut replica.stream,
            &FrontendEvent::Key(KeyEvent {
                frontend_id: replica.fid,
                key,
                mods,
                timestamp_ns: 0,
            }),
        )
        .expect("send Key");
    }

    fn type_line(replica: &mut Replica, text: &str) {
        for ch in text.chars() {
            send_key(replica, Key::Char(ch), Modifiers::NONE);
        }
        send_key(replica, Key::Enter, Modifiers::NONE);
    }

    fn run_command(replica: &mut Replica, name: &str) {
        send_key(replica, Key::Char('x'), Modifiers::ALT);
        type_line(replica, name);
    }

    fn read_until<T>(
        replica: &mut Replica,
        timeout: Duration,
        what: &str,
        mut pick: impl FnMut(InstanceMessage) -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "timed out waiting for {what}");
            replica
                .stream
                .set_read_timeout(Some(remaining.min(Duration::from_millis(200))))
                .ok();
            if let Ok(message) = read_message::<InstanceMessage>(&mut replica.stream)
                && let Some(found) = pick(message)
            {
                return found;
            }
        }
    }

    /// The phase's replica row drove `pmacs.help.show_command`, which no
    /// shipped command binds. This one drives the two shipped commands
    /// (`help.list-commands`, then `help.describe-command` answering
    /// `cursor.left`), whose writes go through the Lua `_show_help` and
    /// the `pmacs.buffer.set_generated_contents` binding: the replica
    /// mirrors `*help*` from its snapshot, and the second write's
    /// `CrdtOp` converges the mirror on the description with the listing
    /// gone.
    #[test]
    fn p3_the_shipped_help_writer_s_replace_reaches_a_replica() {
        let daemon = TestDaemon::spawn();
        let mut replica = attach(&daemon);
        let document = replica.document;

        run_command(&mut replica, "help.list-commands");
        let (help, snapshot) = read_until(
            &mut replica,
            Duration::from_secs(10),
            "the *help* BufferSnapshot",
            |message| match message {
                InstanceMessage::BufferSnapshot {
                    buffer_id,
                    crdt_snapshot,
                } if buffer_id != document => Some((buffer_id, crdt_snapshot)),
                _ => None,
            },
        );
        let mirror = CrdtState::new(replica.fid.0).expect("CrdtState::new");
        mirror
            .import_snapshot(&snapshot)
            .expect("import the *help* snapshot");
        let first = mirror.materialize_string();
        assert!(
            first.contains("help.list-commands"),
            "premise: the replica mirrors the listing; got {first:?}"
        );

        run_command(&mut replica, "help.describe-command");
        type_line(&mut replica, "cursor.left");
        let text = read_until(
            &mut replica,
            Duration::from_secs(10),
            "the CrdtOp replacing *help* with the description",
            |message| match message {
                InstanceMessage::CrdtOp { buffer_id, op } if buffer_id == help => {
                    mirror.import_updates(&op.bytes).expect("import the op");
                    let text = mirror.materialize_string();
                    text.starts_with("cursor.left\n").then_some(text)
                }
                _ => None,
            },
        );
        assert!(
            !text.contains("help.list-commands"),
            "the shipped writer's whole replacement reached the mirror; got {text:?}"
        );
    }
}
