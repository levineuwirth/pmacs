// tests/e8d_drift_acceptance.rs --- E8d, the drift fixes.

//! One row per fix of E8d: copies of one operation that had drifted
//! into defects ([[pmacs refactoring audit (2026-10-13)]], "Drift that
//! is already a defect"), each witnessed through the seam a user or a
//! peer reaches and bitten against `main`'s version of the one file the
//! fix touches (`scripts/bite`).
//!
//! - E8d.1: `LspManager::maybe_restart` discarded a failed respawn's
//!   error after the client was already `Starting`, so a server whose
//!   executable had gone sat `Starting` forever, nothing armed, nothing
//!   told; `McpManager::maybe_restart` surfaced the same failure as
//!   `Crashed` and re-armed the backoff.
//! - E8d.2: `EditorCore::undo` and `redo` told the text view alone and
//!   clamped the cursor alone, where every forward edit reaches every
//!   overlay and clamps the selection and `view_top` too.
//! - E8d.3: `*help*`, `*workers*` and `*pmacs-instance*` were replaced
//!   by a `Delete` then an `Insert` through `apply_edit`, never through
//!   `Buffer::set_generated_contents`, the one authorized write
//!   (`docs/invariants.md`), so they stayed writable with a history
//!   that reached the rope.
//! - E8d.4: the daemon's write-failure drop of a frontend removed the
//!   per-frontend maps and nothing else, where `SessionDetached` also
//!   cancels the live gesture, drops the command history, hands on the
//!   undo source and runs `frontend.detached`.
//! - E8d.5: `lean.lua`'s root walk carried the `parent_of` and
//!   `within_boundary` copies `lsp.lua`'s latex resolver had fixed:
//!   `/` was never examined, and a `/` boundary silenced the walk.
//! - E8d.6: `project-search.quit` lacked the side-window branch that
//!   `listview.quit`, `dired.quit` and `compile.quit` carry, so `q` in
//!   a bottom-panel `*search-results*` switched buffers inside the
//!   panel instead of quitting the window.

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

// ---------------------------------------------------------------------------
// The in-process editor, driven by the keys a user presses
// ---------------------------------------------------------------------------

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

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        press(s, KeyCode::Char(ch));
    }
}

#[path = "common/editor.rs"]
mod editor;
use editor::{eval, exec};

fn active_text(s: &EditorState) -> String {
    eval(
        s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    )
}

/// An isolated editor (`common::iso`): no user `init.lua`, no real
/// data root, no language server, and a declared frame geometry (a
/// grid frontend's frame size is its geometry declaration, which a
/// side window needs before it can be placed).
fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&common::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    exec(&s, "pmacs.lsp.config = {}");
    s.sync_frame_geometry(FrontendId::LOCAL, pmacs::cell::CellSize::new(24, 80));
    s
}

/// `C-x u`, the undo chord every route round-trips to the daemon.
fn undo(s: &mut EditorState) {
    ctrl(s, 'x');
    press(s, KeyCode::Char('u'));
}

// ---------------------------------------------------------------------------
// E8d.2 --- undo and redo notify everything
// ---------------------------------------------------------------------------

/// A window overlay holding one byte offset, carried through edits the
/// way `BufferStyleSpanTranslator` carries its spans: the shape of every
/// overlay with byte-offset state. It only hears the edits the window's
/// fan-out tells it.
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
        "e8d_offset_overlay"
    }
}

/// Typing before an overlay's offset carries it forward; undoing the
/// typing must carry it back. At `main` the undo told the text view
/// alone, so the overlay kept the offset the undone insert had given it.
#[test]
fn e8d_2_undo_carries_an_overlay_s_byte_offset_back() {
    let mut s = editor();
    type_str(&mut s, "0123456789");
    let at = Arc::new(Mutex::new(5_u64));
    s.core
        .borrow_mut()
        .active_window_mut()
        .push_overlay(Box::new(OffsetOverlay { at: at.clone() }));

    // Type two characters at the start of the line: a new undo group,
    // since the motion command between breaks the self-insert run.
    ctrl(&mut s, 'a');
    type_str(&mut s, "ab");
    assert_eq!(active_text(&s), "ab0123456789", "premise: the text landed");
    assert_eq!(
        *at.lock().unwrap(),
        7,
        "premise: the forward edit carried the overlay's offset"
    );

    undo(&mut s);
    assert_eq!(
        active_text(&s),
        "0123456789",
        "premise: the undo took the typing"
    );
    assert_eq!(
        *at.lock().unwrap(),
        5,
        "the undo is told to every overlay, so the offset is carried back"
    );
}

/// A selection anchor and a `view_top` past the end an undo leaves are
/// clamped, as every forward edit clamps them. At `main` the undo
/// clamped the cursor alone and left both dangling past the buffer.
#[test]
fn e8d_2_undo_clamps_the_selection_and_the_view_top() {
    let mut s = editor();
    type_str(&mut s, "one");
    // A script's insert of thirty more lines: one group, whoever undoes
    // next takes it (E6c).
    exec(
        &s,
        "local b = pmacs.window.buffer(); b:insert(b:len(), string.rep('\\nline', 30))",
    );
    let long = active_text(&s);
    assert_eq!(long.lines().count(), 31, "premise: thirty-one lines");
    let len = long.len() as u64;
    {
        let mut core = s.core.borrow_mut();
        let win = core.active_window_mut();
        win.cursor = 1;
        win.selection = Some(pmacs::window::Selection { anchor: len });
        win.view_top = 25;
    }

    undo(&mut s);
    assert_eq!(
        active_text(&s),
        "one",
        "premise: the undo took the thirty lines"
    );
    let core = s.core.borrow();
    let win = core.active_window();
    assert_eq!(
        win.selection.map(|sel| sel.anchor),
        Some(3),
        "the selection anchor is clamped to the new end (and kept, since it \
         did not collapse onto the cursor at 1); it was {len}"
    );
    assert_eq!(
        win.view_top, 0,
        "the view top is clamped to the last line; it was 25 of a one-line buffer"
    );
}

// ---------------------------------------------------------------------------
// E8d.1 --- the LSP respawn
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

/// Tick the supervisor and the manager, appending `sid`'s events to
/// `seen`, until `pred` holds over everything seen so far or `deadline`
/// lapses. One accumulating list across a row's phases, so an event
/// that lands in the same batch as the one a phase waited for is not
/// lost to the next phase's predicate.
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

fn restarting_attempt(evs: &[LspEvent], wanted: u32) -> bool {
    evs.iter()
        .any(|e| matches!(e.kind, LspEventKind::Restarting { attempt } if attempt == wanted))
}

fn respawn_failure(evs: &[LspEvent]) -> Option<String> {
    evs.iter().find_map(|e| match &e.kind {
        LspEventKind::Crashed { reason } if reason.starts_with("respawn failed: ") => {
            Some(reason.clone())
        }
        _ => None,
    })
}

/// A server crashes, and by the time its restart fires the executable
/// is gone. The respawn cannot spawn: the client is `Crashed` with a
/// `Crashed` event naming the respawn, and the backoff is re-armed so a
/// further attempt follows --- never `Starting` with nothing armed and
/// nothing told, which is where `main`'s `maybe_restart` left it.
#[test]
fn e8d_1_a_respawn_whose_executable_is_gone_is_told_as_a_crash_and_re_armed() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let exe = dir.path().join("dying-server");
    std::fs::write(&exe, "#!/bin/sh\nexit 3\n").expect("write the server");
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let (sup, mgr) = lsp_manager();
    mgr.borrow_mut()
        .set_restart_backoff(Duration::from_millis(100));
    let mut spec = LspServerSpec::new("dying", "rust", exe.to_str().expect("utf-8 path"));
    spec.restart = LspRestartPolicy::OnCrash;
    let sid = mgr.borrow_mut().spawn(spec).expect("spawn the server");

    // Premise: the first generation crashes (exit 3 before it answers
    // `initialize`) and the restart is armed by the ordinary exit path.
    let mut seen = Vec::new();
    let crashed = drain_lsp_until(&sup, &mgr, sid, &mut seen, Duration::from_secs(10), |evs| {
        evs.iter()
            .any(|e| matches!(e.kind, LspEventKind::Crashed { .. }))
    });
    assert!(
        crashed,
        "premise: the server's first generation crashes; events {seen:?}"
    );

    // The executable disappears before the backoff elapses.
    std::fs::remove_file(&exe).expect("remove the server");

    let fired = drain_lsp_until(&sup, &mgr, sid, &mut seen, Duration::from_secs(5), |evs| {
        restarting_attempt(evs, 2)
    });
    assert!(
        fired,
        "premise: the restart fires (attempt 2); events {seen:?}"
    );

    // The row: the failed respawn is a crash of its own, told and re-armed.
    let told = drain_lsp_until(&sup, &mgr, sid, &mut seen, Duration::from_secs(5), |evs| {
        respawn_failure(evs).is_some()
    });
    let state = mgr.borrow().state(sid).cloned();
    assert!(
        told,
        "a respawn that could not spawn must be told as `Crashed` \
         (reason `respawn failed: ...`); the client is {state:?} and \
         the events since the restart are {seen:?}"
    );
    assert!(
        matches!(state, Some(LspClientState::Crashed { ref reason, .. }) if reason.starts_with("respawn failed: ")),
        "the client is `Crashed` by the respawn's own failure, never left `Starting`; \
         state {state:?}, events {seen:?}"
    );

    // Re-armed: under `OnCrash` a third attempt follows the backoff.
    let rearmed = drain_lsp_until(&sup, &mgr, sid, &mut seen, Duration::from_secs(5), |evs| {
        restarting_attempt(evs, 3)
    });
    assert!(
        rearmed,
        "the backoff is re-armed after the failed respawn, so attempt 3 fires; \
         events {seen:?}, attempt {:?}",
        mgr.borrow().attempt(sid)
    );
    let _ = mgr.borrow_mut().stop(sid);
}

// ---------------------------------------------------------------------------
// E8d.3 --- generated buffers through the primitive
// ---------------------------------------------------------------------------

fn alt(s: &mut EditorState, c: char) {
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::ALT));
}

fn active_name(s: &EditorState) -> String {
    eval(
        s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    )
}

/// The text of the buffer named `name`, or empty when there is none.
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

/// Whether the buffer named `name` reads as modified.
fn named_modified(s: &EditorState, name: &str) -> bool {
    eval(
        s,
        &format!(
            "for _, id in ipairs(pmacs.buffer.list()) do
               local d = pmacs.describe.buffer(id)
               if d.name == {name:?} then return d.modified end
             end
             return false"
        ),
    )
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

/// `*help*` through its Rust renderer (`pmacs.help.show_command`, the
/// seam a config or `M-:` reaches; no shipped command binds it): `M-x
/// buffer.undo` leaves it intact (its history is cleared by the
/// primitive, where the delete-then-insert pair left two groups for the
/// undo to pop), an ordinary insert is refused (read-only), and the Lua
/// renderer (`pmacs.editor._show_help`, behind `help.list-commands` and
/// the describe commands) still writes it, through the same primitive.
#[test]
fn e8d_3_help_is_read_only_with_no_history_and_both_renderers_write_it() {
    let mut s = editor();
    exec(
        &s,
        "pmacs.window.switch_buffer(pmacs.help.show_command('cursor.left'))",
    );
    assert_eq!(active_name(&s), "*help*", "premise: help shows");
    let shown = named_text(&s, "*help*");
    assert!(
        shown.contains("Command: cursor.left"),
        "premise: the Rust renderer wrote the command's help; got {shown:?}"
    );

    undo(&mut s);
    assert_eq!(
        named_text(&s, "*help*"),
        shown,
        "M-x buffer.undo leaves the generated text intact; status {:?}",
        status(&s)
    );

    type_str(&mut s, "NOTE");
    assert_eq!(
        named_text(&s, "*help*"),
        shown,
        "an ordinary insert into *help* is refused"
    );
    assert!(!named_modified(&s, "*help*"), "and it reads clean");

    // The Lua renderer writes the same buffer through the same primitive.
    exec(&s, "pmacs.editor._show_help('E8d: the Lua renderer\\n')");
    assert_eq!(
        named_text(&s, "*help*"),
        "E8d: the Lua renderer\n",
        "the Lua renderer replaces the read-only *help*"
    );
    type_str(&mut s, "NOTE");
    assert_eq!(
        named_text(&s, "*help*"),
        "E8d: the Lua renderer\n",
        "and leaves it read-only"
    );
}

/// `*workers*` and `*pmacs-instance*`, through their commands: each is
/// read-only after its render and an insert is refused.
#[test]
fn e8d_3_workers_and_instance_buffers_are_read_only() {
    for (command, name) in [
        ("editor.list-workers", "*workers*"),
        ("editor.describe-instance-buffer", "*pmacs-instance*"),
    ] {
        let mut s = editor();
        alt(&mut s, 'x');
        type_str(&mut s, command);
        press(&mut s, KeyCode::Enter);
        assert_eq!(active_name(&s), name, "premise: {command} shows {name}");
        let shown = named_text(&s, name);
        assert!(!shown.is_empty(), "premise: {name} holds its text");
        type_str(&mut s, "NOTE");
        assert_eq!(
            named_text(&s, name),
            shown,
            "an ordinary insert into {name} is refused"
        );
        undo(&mut s);
        assert_eq!(
            named_text(&s, name),
            shown,
            "and M-x buffer.undo leaves {name} intact"
        );
    }
}

// ---------------------------------------------------------------------------
// A real daemon and synthetic replicas (the m10_11 idiom), for the rows
// whose subject is the daemon's own behavior.
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

    pub struct Replica {
        pub stream: UnixStream,
        pub fid: FrontendId,
        /// The daemon's first snapshot: the document buffer the replica
        /// attached to.
        pub document: BufferId,
    }

    /// Attach a multi-frontend, CRDT-replica frontend and read the
    /// daemon's initial `BufferSnapshot`, which it always sends first.
    pub fn attach(daemon: &TestDaemon) -> Replica {
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

    pub fn send_key(replica: &mut Replica, key: Key, mods: Modifiers) {
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

    /// `M-x <name> RET`.
    pub fn run_command(replica: &mut Replica, name: &str) {
        send_key(replica, Key::Char('x'), Modifiers::ALT);
        for ch in name.chars() {
            send_key(replica, Key::Char(ch), Modifiers::NONE);
        }
        send_key(replica, Key::Enter, Modifiers::NONE);
    }

    /// Read the stream until `pick` accepts a message, or `timeout`
    /// lapses (panicking, naming `what`).
    pub fn read_until<T>(
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

    /// A daemon whose config defines two commands rendering `*help*`
    /// through its Rust renderer, so a replica reaches the renderer by
    /// `M-x` alone.
    const HELP_COMMANDS_CONFIG: &str = "\
pmacs.command.define { name = 'e8d.help-a', description = 'cursor.left in *help*',
  fn = function() pmacs.window.switch_buffer(pmacs.help.show_command('cursor.left')) end }
pmacs.command.define { name = 'e8d.help-b', description = 'cursor.right in *help*',
  fn = function() pmacs.window.switch_buffer(pmacs.help.show_command('cursor.right')) end }
";

    /// The CRDT fan-out of the primitive's one `Replace` reaches a
    /// replica: the replica mirrors `*help*` from its snapshot, and the
    /// next render's edit converges the mirror on the new text.
    #[test]
    fn e8d_3_the_one_replace_edit_of_help_reaches_a_replica() {
        let daemon = TestDaemon::spawn_with_config(HELP_COMMANDS_CONFIG);
        let mut replica = attach(&daemon);
        let document = replica.document;

        run_command(&mut replica, "e8d.help-a");
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
            first.contains("Command: cursor.left"),
            "premise: the replica mirrors *help*; got {first:?}"
        );

        run_command(&mut replica, "e8d.help-b");
        let text = read_until(
            &mut replica,
            Duration::from_secs(10),
            "the CrdtOp replacing *help*",
            |message| match message {
                InstanceMessage::CrdtOp { buffer_id, op } if buffer_id == help => {
                    mirror.import_updates(&op.bytes).expect("import the op");
                    let text = mirror.materialize_string();
                    text.contains("Command: cursor.right").then_some(text)
                }
                _ => None,
            },
        );
        assert!(
            !text.contains("Command: cursor.left"),
            "the whole replacement reached the mirror; got {text:?}"
        );
    }

    /// The write-failure drop is reached only where a write to a peer
    /// that shut its read half fails: Linux answers it with EPIPE, so the
    /// daemon's next frame to the frontend fails while its reader thread
    /// still blocks. On macOS CI (`Test (macos-latest / lua54)`, run
    /// 38558729706) the same row waited its 5 s with an empty detach log:
    /// BSD lets the write succeed, the frontend is never dropped, and the
    /// precondition cannot be made there. The row is compiled on Linux
    /// alone rather than reported `ok` for a drop it never saw.
    #[cfg(target_os = "linux")]
    mod write_failure {
        use super::*;
        // -----------------------------------------------------------------------
        // E8d.4 --- one teardown
        // -----------------------------------------------------------------------

        /// A `frontend.detached` hook that logs the departed id into the
        /// daemon's isolated home (the undo-across-peers suite's idiom), so
        /// a test can count the daemon's detaches without editing a buffer.
        const DETACH_LOG_CONFIG: &str = "\
pmacs.hook.add('frontend.detached', function(fid)
  local f = assert(io.open(os.getenv('HOME') .. '/detached.log', 'a'))
  f:write(tostring(fid), '\\n')
  f:close()
end)
";

        fn detach_log(daemon: &TestDaemon) -> std::path::PathBuf {
            daemon
                .socket_path()
                .parent()
                .expect("socket parent")
                .join("detached.log")
        }

        fn detaches_of(daemon: &TestDaemon, fid: FrontendId) -> usize {
            std::fs::read_to_string(detach_log(daemon))
                .unwrap_or_default()
                .lines()
                .filter(|line| *line == fid.0.to_string())
                .count()
        }

        /// Type one character on `replica` and wait for the daemon to answer
        /// it with any message: the daemon is serving this frontend.
        fn type_and_hear(replica: &mut Replica, ch: char) {
            send_key(replica, Key::Char(ch), Modifiers::NONE);
            read_until(
                replica,
                Duration::from_secs(5),
                "a frame after typing",
                |m| {
                    matches!(
                        m,
                        InstanceMessage::CellDelta { .. }
                            | InstanceMessage::CrdtOp { .. }
                            | InstanceMessage::CursorByte { .. }
                    )
                    .then_some(())
                },
            );
        }

        /// A frontend whose socket refuses the daemon's writes while its
        /// reader stays open (the peer shut its read half down, nothing
        /// more) is torn down once. Since E9's fix round 4 (the owner's
        /// ruling of 2026-10-20) the failed write stops the output and the
        /// teardown follows at the reader's EOF or, as here, where the
        /// peer never closes, at `pmacs::daemon::WRITE_FAILURE_DRAIN`
        /// (two seconds, inside this row's five); that teardown is the
        /// whole detach: `frontend.detached` runs for it, once. The
        /// reader thread's own `SessionDetached`, when the socket closes,
        /// finds no session and changes nothing.
        #[test]
        fn e8d_4_a_frontend_dropped_by_a_failed_write_is_detached_exactly_once() {
            let daemon = TestDaemon::spawn_with_config(DETACH_LOG_CONFIG);
            let broken = attach(&daemon);
            let mut live = attach(&daemon);
            let broken_fid = broken.fid;
            // Both are served before anything breaks.
            type_and_hear(&mut live, 'a');

            // The daemon's next write to `broken` fails with EPIPE; its
            // reader thread keeps blocking on a socket nobody writes to.
            broken
                .stream
                .shutdown(std::net::Shutdown::Read)
                .expect("shut the read half");
            // A shared-buffer edit by the live frontend repaints every
            // frontend, so the daemon writes to the broken one.
            type_and_hear(&mut live, 'b');

            // Since E9's fix round 4 the teardown is the drain's, at its
            // deadline, this peer sending no EOF. Meanwhile `live` is read
            // as a frontend reads, a cursor a tick: a socket left to fill
            // blocks the dispatcher's write to it (the daemon writes a
            // frontend with no timeout), and with it the loop the
            // deadline is read on. Every replica is written its cursor
            // each tick, so `broken`'s next write fails without a key.
            let deadline = Instant::now() + Duration::from_secs(5);
            while detaches_of(&daemon, broken_fid) == 0 && Instant::now() < deadline {
                read_until(
                    &mut live,
                    Duration::from_secs(5),
                    "live's next cursor",
                    |m| matches!(m, InstanceMessage::CursorByte { .. }).then_some(()),
                );
            }
            assert_eq!(
                detaches_of(&daemon, broken_fid),
                1,
                "the failed write detaches the frontend the whole way: `frontend.detached` \
             ran once for {} (log {:?})",
                broken_fid.0,
                std::fs::read_to_string(detach_log(&daemon)).unwrap_or_default()
            );

            // Now the socket closes for good: the reader thread ends and
            // sends `SessionDetached`, a no-op on the missing session.
            drop(broken);
            let settle = Instant::now() + Duration::from_secs(2);
            while Instant::now() < settle {
                type_and_hear(&mut live, 'd');
                std::thread::sleep(Duration::from_millis(100));
            }
            assert_eq!(
                detaches_of(&daemon, broken_fid),
                1,
                "a later SessionDetached for the dropped frontend changes nothing (log {:?})",
                std::fs::read_to_string(detach_log(&daemon)).unwrap_or_default()
            );
            // The live frontend's own detach is still told, once, when it
            // leaves: the no-op is per missing session, not per daemon.
            let live_fid = live.fid;
            drop(live);
            let deadline = Instant::now() + Duration::from_secs(5);
            while detaches_of(&daemon, live_fid) == 0 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            assert_eq!(
                detaches_of(&daemon, live_fid),
                1,
                "the live frontend's detach"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// E8d.5 --- lean's root walk
// ---------------------------------------------------------------------------

fn lua_str(path: &std::path::Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// Under a `/` search boundary, a `lean-toolchain` beside the file
/// roots it at its directory. At `main` the boundary test compared
/// string prefixes, so `/` (needle `//`) put every ancestor out of
/// bounds and the walk returned nothing.
#[test]
fn e8d_5_a_lean_toolchain_under_a_root_boundary_roots_the_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pkg = std::fs::canonicalize(dir.path())
        .expect("canonical")
        .join("pkg");
    std::fs::create_dir_all(&pkg).expect("pkg");
    std::fs::write(pkg.join("lean-toolchain"), "leanprover/lean4:v4.9.0\n").expect("marker");
    let file = pkg.join("A.lean");
    std::fs::write(&file, "def a := 1\n").expect("file");

    let s = editor();
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
        pkg.display().to_string(),
        "the walk under a `/` boundary reaches the marker beside the file"
    );
}

// ---------------------------------------------------------------------------
// E8d.6 --- project-search.quit
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

/// `q` in `*search-results*`: in a full window it restores the
/// previous buffer, as before; in a side window it quits the window
/// (`pmacs.window.quit`), as `listview.quit`, `dired.quit` and
/// `compile.quit` do. At `main` the side-window `q` switched the panel
/// slot to the previous buffer and left the side window standing.
#[test]
fn e8d_6_q_in_search_results_quits_a_side_window_and_restores_a_full_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = std::fs::canonicalize(dir.path()).expect("canonical");
    let notes = root.join("notes.txt");
    std::fs::write(&notes, "a needle here\n").expect("notes");
    // A file buffer's name is its absolute path.
    let notes_name = notes.display().to_string();
    let mut s = editor();
    exec(
        &s,
        &format!("pmacs.buffer.find_or_open(\"{}\")", lua_str(&notes)),
    );
    assert_eq!(active_name(&s), notes_name, "premise: the file is shown");
    exec(
        &s,
        &format!(
            "pmacs.project.search('needle', {{ root = \"{}\" }})",
            lua_str(&root)
        ),
    );
    assert_eq!(
        active_name(&s),
        "*search-results*",
        "premise: the search shows its results"
    );
    assert!(
        pump_until(&mut s, Duration::from_secs(20), |s| {
            named_text(s, "*search-results*").contains("notes.txt")
        }),
        "premise: the search lists its match"
    );

    // A full window: `q` restores the previous buffer.
    press(&mut s, KeyCode::Char('q'));
    assert_eq!(
        active_name(&s),
        notes_name,
        "`q` in a full window restores the buffer the search was run from"
    );

    // A side window: `q` quits the window.
    exec(
        &s,
        "for _, id in ipairs(pmacs.buffer.list()) do
           if pmacs.describe.buffer(id).name == '*search-results*' then
             pmacs.window.display(id, { side = 'bottom', select = true })
           end
         end",
    );
    assert!(
        side_window(&s).is_some(),
        "premise: the results are in a side window"
    );
    assert_eq!(
        active_name(&s),
        "*search-results*",
        "premise: and it is selected"
    );
    press(&mut s, KeyCode::Char('q'));
    assert_eq!(
        side_window(&s),
        None,
        "`q` in a side window routes through window.quit, which deletes it"
    );
    assert_eq!(
        active_name(&s),
        notes_name,
        "and focus lands back in the document window"
    );
}
