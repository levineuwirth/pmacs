//! Worker-pool teardown regression: dropping an `EditorState` must
//! release its worker threads even though the `Rc<AsyncRuntime>` is
//! trapped in Lua-VM reference cycles and never reaches refcount
//! zero (`EditorState::drop` → `AsyncRuntime::shutdown_workers`,
//! signal-only --- a join here deadlocks against workers blocked
//! handing replies to the main thread).
//!
//! Before the fix, every `EditorState` ever constructed leaked a
//! full `cores - 1` worker pool: the m4 acceptance suite (54 editor-
//! building tests) accumulated 1000+ live threads, each waking every
//! 100ms.
//!
//! The thread-count probe and the idempotence check share ONE test
//! function: they both build `EditorState`s, and as separate tests
//! libtest may run them concurrently, polluting the /proc-based
//! baseline on high-core machines.

use pmacs::editor::EditorState;

/// Thread-count probe via /proc; Linux-only (macOS CI runs the
/// non-Linux variant below --- the leak and the fix are platform-
/// independent, the *probe* isn't).
#[cfg(target_os = "linux")]
fn live_threads() -> usize {
    std::fs::read_dir("/proc/self/task").map_or(0, std::iter::Iterator::count)
}

#[cfg(target_os = "linux")]
#[test]
fn editor_state_drop_releases_workers_and_shutdown_is_idempotent() {
    let baseline = live_threads();
    for _ in 0..3 {
        let s = EditorState::new_with_roots(&crate::iso::roots());
        drop(s);
    }
    // Signal-only shutdown: parked workers exit within their 100ms
    // park timeout; give them a beat.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let after = live_threads();
    assert!(
        after <= baseline + 2,
        "worker threads leak across EditorState drop: \
         baseline {baseline}, after 3 create/drop cycles {after}"
    );

    // Idempotence: explicit shutdown twice, then drop runs it a
    // third time --- none may hang or panic.
    let s = EditorState::new_with_roots(&crate::iso::roots());
    s.async_runtime.shutdown_workers();
    s.async_runtime.shutdown_workers();
    drop(s);
}

/// Platform-independent idempotence check for hosts without /proc.
#[cfg(not(target_os = "linux"))]
#[test]
fn explicit_shutdown_is_idempotent() {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    s.async_runtime.shutdown_workers();
    s.async_runtime.shutdown_workers(); // second call must not hang or panic
    drop(s); // drop runs shutdown a third time
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
