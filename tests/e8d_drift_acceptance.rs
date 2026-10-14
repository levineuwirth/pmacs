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

/// An isolated editor (`common::iso`): no user `init.lua`, no real
/// data root, no language server.
fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&common::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    exec(&s, "pmacs.lsp.config = {}");
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
