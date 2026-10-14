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

#![cfg(unix)]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use pmacs::async_runtime::AsyncRuntime;
use pmacs::lsp::{
    LspClientState, LspEvent, LspEventKind, LspManager, LspRestartPolicy, LspServerId,
    LspServerSpec,
};

#[path = "common/mod.rs"]
mod common;

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
