//! Worker-pool teardown regression: dropping an `EditorState` must
//! release its worker threads even though the `Rc<AsyncRuntime>` is
//! trapped in Lua-VM reference cycles and never reaches refcount
//! zero (`EditorState::drop` → `AsyncRuntime::shutdown_workers`).
//!
//! Before the fix, every `EditorState` ever constructed leaked a
//! full `cores - 1` worker pool: the m4 acceptance suite (54 editor-
//! building tests) accumulated 1000+ live threads, each waking every
//! 100ms.

use pmacs::editor::EditorState;

/// Thread-count probe via /proc; Linux-only (macOS CI skips --- the
/// leak and the fix are platform-independent, the *probe* isn't).
#[cfg(target_os = "linux")]
fn live_threads() -> usize {
    std::fs::read_dir("/proc/self/task").map_or(0, std::iter::Iterator::count)
}

#[cfg(target_os = "linux")]
#[test]
fn editor_state_drop_releases_worker_threads() {
    let baseline = live_threads();
    for _ in 0..3 {
        let s = EditorState::new();
        drop(s);
    }
    // Joins are synchronous in drop; the small sleep only covers
    // detached per-process reaper threads finishing up.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let after = live_threads();
    assert!(
        after <= baseline + 2,
        "worker threads leak across EditorState drop: \
         baseline {baseline}, after 3 create/drop cycles {after}"
    );
}

/// Platform-independent variant: the pool reports itself dead after
/// an explicit shutdown, and shutdown is idempotent.
#[test]
fn explicit_shutdown_is_idempotent() {
    let s = EditorState::new();
    s.async_runtime.shutdown_workers();
    s.async_runtime.shutdown_workers(); // second call must not hang or panic
    drop(s); // drop runs shutdown a third time
}
