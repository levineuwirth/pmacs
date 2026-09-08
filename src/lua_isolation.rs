// lua_isolation.rs --- T M7.8 cancellation hook.

//! VM-level cancellation for misbehaving Lua packages.
//!
//! Per spec §sec:packages-future / T M7.8: a *count hook* installed via
//! [`mlua::Lua::set_hook`] runs every N instructions, polls an
//! [`AtomicBool`], and aborts the running chunk if the flag is set. The
//! flag is flippable from any thread via [`CancelHandle`], so a thread
//! other than the VM's main thread (the spec calls out an
//! "input-watching" thread) can interrupt a hot-looping main-thread
//! package without signal-handler VM access.
//!
//! ## Design rationale
//!
//! Two alternatives were rejected during M7 planning (recorded here so
//! the choice doesn't drift on the next refactor):
//!
//! * *Coroutine scheduling.* Requires every package to yield
//!   cooperatively, which is exactly the property a misbehaving
//!   package lacks.
//! * *On-demand signal-installed hooks.* SIGINT can break out of
//!   native syscalls but cannot safely manipulate Lua VM state from
//!   a signal handler — signal-handler reentrancy into the VM
//!   allocator is unsafe.
//!
//! The chosen mechanism — install a count hook once at VM init,
//! poll an atomic flag — runs in-VM at instruction granularity, needs
//! no cooperation from package authors, and never touches the VM from
//! a signal handler.
//!
//! ## `LuaJIT` JIT-trace caveat
//!
//! `LuaJIT` honors `lua_sethook` from the interpreter, but JIT-compiled
//! traces do not call hooks. A tight loop the JIT has compiled
//! bypasses polling until the trace recorder eventually exits the
//! trace, which can take 100ms-1s under pathological JIT behavior.
//! Lua 5.4 has no JIT and the hook fires at instruction granularity
//! reliably. The acceptance test budget (1s) reflects the `LuaJIT`
//! caveat, not aspirational responsiveness.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mlua::{HookTriggers, Lua, VmState};
use thiserror::Error;

/// Default instruction budget for the count hook.
///
/// Picked empirically (T M7.8 acceptance bullet 5); the recorded
/// numbers live in `TRANSITION-M7.md`. Sits in the middle of the
/// spec-mandated 10K-100K range.
///
/// **Why N doesn't move overhead much.** Both lua54 and luajit (in
/// interpreter mode) decrement the hook counter on *every*
/// instruction; the hook callback only fires when the counter hits
/// zero. So the synthetic-loop overhead measured below is dominated
/// by the per-instruction decrement-and-compare, not by callback
/// frequency. Empirically, N=50K and N=100K give the same hook-on
/// elapsed time within measurement noise. We pick the lower end of
/// the working range so cancel latency is as tight as it can be.
///
/// Measured on `tests/m7_8_acceptance.rs::benchmark_records_overhead_and_latency`,
/// debug build, `local s = 0 for i = 1, 10M do s = s + 1 end`:
///
/// | flavor          | hook-on   | hook-off  | cancel latency |
/// |-----------------|-----------|-----------|----------------|
/// | lua54           | ~57ms     | ~29ms     | ~60µs          |
/// | luajit (no JIT) | ~32ms     | ~25ms     | ~100µs         |
///
/// The synthetic-loop overhead (30-100%) is the worst-case envelope
/// for an all-Lua tight loop. Real package code interleaves Rust
/// callbacks, table lookups, and string allocations whose own per-op
/// cost dwarfs the hook poll. Cancel latency, the user-facing
/// number, comfortably beats the spec's 1-second budget on both
/// flavors.
pub const DEFAULT_INSTRUCTION_BUDGET: u32 = 50_000;

/// Errors raised by the isolation layer.
///
/// Appears as the `source()` of an [`mlua::Error::ExternalError`]
/// returned by any function whose execution was running while the
/// cancel flag was set.
#[derive(Debug, Error)]
pub enum IsolationError {
    /// The cancel flag was observed set by the count hook.
    #[error("Lua execution cancelled (C-g)")]
    Cancelled,
}

/// Owner-side cancellation control for a Lua VM.
///
/// Wraps an [`Arc<AtomicBool>`] so the same flag is visible to the
/// count-hook closure inside the VM and to any [`CancelHandle`] held
/// by another thread. Reset semantics belong to the host: it observes
/// a cancellation in the eval result, logs it, then calls
/// [`CancelToken::reset`] before the next eval — so a cancel signal
/// is consumed exactly once.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    /// Construct a fresh, un-cancelled token.
    #[must_use]
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Flip the flag. Idempotent: a second `cancel` before a `reset`
    /// is a no-op (the flag was already set).
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    /// Whether a cancellation has been requested but not yet reset.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    /// Clear the flag. Called by the host after observing a
    /// cancellation in an eval result so the next eval starts fresh.
    pub fn reset(&self) {
        self.flag.store(false, Ordering::Relaxed);
    }

    /// Cross-thread write-only handle. The returned [`CancelHandle`]
    /// is `Send + Sync` and may be moved or cloned to other threads.
    #[must_use]
    pub fn handle(&self) -> CancelHandle {
        CancelHandle {
            flag: Arc::clone(&self.flag),
        }
    }
}

/// `Send + Sync` handle to a [`CancelToken`]'s flag.
///
/// Held by a thread other than the one running the Lua VM (e.g. the
/// editor's input-watching thread). Calling [`CancelHandle::cancel`]
/// sets the flag the in-VM count hook polls; the next time the hook
/// runs (i.e. within N instructions on lua54, possibly later on
/// LuaJIT-compiled traces — see module docs) the running chunk
/// aborts.
#[derive(Debug, Clone)]
pub struct CancelHandle {
    flag: Arc<AtomicBool>,
}

impl CancelHandle {
    /// Flip the cancel flag. See [`CancelToken::cancel`] for
    /// idempotence semantics.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    /// Whether a cancellation is currently pending. Useful for tests
    /// that need to verify the flag landed without racing the hook.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }
}

/// Install the count hook on `lua`. Should be called exactly once at
/// VM init.
///
/// The hook captures `token`'s flag (via [`Arc`] clone) and runs every
/// `n` instructions. On observing the flag set, it returns
/// [`mlua::Error::external`] wrapping [`IsolationError::Cancelled`],
/// which mlua propagates as the chunk's failure result.
///
/// The hook does *not* reset the flag; the host's eval-result handler
/// is responsible for calling [`CancelToken::reset`] after observing
/// a cancellation, so the signal is consumed exactly once.
///
/// `n` should be in the 10K-100K range per spec; see
/// [`DEFAULT_INSTRUCTION_BUDGET`] for the value pmacs ships.
pub fn install_cancel_hook(lua: &Lua, token: &CancelToken, n: u32) {
    let flag = Arc::clone(&token.flag);
    let triggers = HookTriggers::new().every_nth_instruction(n);
    lua.set_hook(triggers, move |_lua, _debug| {
        if flag.load(Ordering::Relaxed) {
            Err(mlua::Error::external(IsolationError::Cancelled))
        } else {
            Ok(VmState::Continue)
        }
    });
}

/// Whether `err` (or any error in its [`mlua::Error::chain`]) is an
/// [`IsolationError::Cancelled`].
///
/// Used by the host's eval-result path to distinguish a cancellation
/// from an arbitrary package error: cancellations get logged as
/// `(cancelled)` and the flag is reset, whereas a syntax / runtime
/// error from package code gets logged with its message and leaves
/// the flag alone.
///
/// Walks via [`mlua::Error::chain`] (which knows how to peel
/// `CallbackError`'s `cause` and `WithContext`'s `cause`) plus, at
/// each step, [`mlua::Error::downcast_ref`] for the
/// [`mlua::Error::ExternalError`] payload. mlua's
/// `std::error::Error::source` impl returns `None` on `CallbackError`
/// (line 337 of mlua 0.10's error.rs), so a plain `source`-walk
/// would miss the cancellation when the hook fires inside a Lua
/// chunk loaded by `Lua::load(...).exec()`.
#[must_use]
pub fn is_cancellation(err: &mlua::Error) -> bool {
    if err.downcast_ref::<IsolationError>().is_some() {
        return true;
    }
    for cause in err.chain() {
        if cause.downcast_ref::<IsolationError>().is_some() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    /// Disable `LuaJIT` JIT compilation for unit tests that exercise the
    /// count hook on tight loops. Without this, `LuaJIT` compiles
    /// `while true do end` into a trace that bypasses the hook
    /// entirely (per the module docs' caveat). On lua54 this is a
    /// no-op — the `jit` table doesn't exist there.
    fn disable_jit_if_present(lua: &Lua) {
        // `jit.off()` (no args) disables the JIT compiler globally,
        // ensuring the count hook fires on every chunk this VM runs
        // afterwards. Without this, LuaJIT would compile the test
        // bodies' tight loops into traces that bypass the hook
        // (per the module docs' caveat). The acceptance test
        // `tests/m7_8_acceptance.rs` exercises the JIT-on path with
        // the spec-mandated 1s budget.
        //
        // `jit.flush()` purges any traces compiled before the
        // off-toggle (e.g. from earlier inits in the same VM).
        let _ = lua.load("if jit then jit.off(); jit.flush() end").exec();
    }

    #[test]
    fn token_starts_uncancelled() {
        let t = CancelToken::new();
        assert!(!t.is_cancelled());
    }

    #[test]
    fn cancel_then_reset_round_trips() {
        let t = CancelToken::new();
        t.cancel();
        assert!(t.is_cancelled());
        t.reset();
        assert!(!t.is_cancelled());
    }

    #[test]
    fn handle_writes_visible_to_token() {
        let t = CancelToken::new();
        let h = t.handle();
        h.cancel();
        assert!(t.is_cancelled());
    }

    #[test]
    fn cross_thread_cancel_via_handle() {
        let t = CancelToken::new();
        let h = t.handle();
        let join = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            h.cancel();
        });
        // Spin until the other thread fires (bounded by sane wall
        // clock — if this loops past 1s, the cross-thread visibility
        // is broken).
        let start = std::time::Instant::now();
        while !t.is_cancelled() {
            assert!(
                start.elapsed() < Duration::from_secs(1),
                "cross-thread cancel never observed (waited {:?})",
                start.elapsed()
            );
            thread::yield_now();
        }
        join.join().unwrap();
    }

    #[test]
    fn hook_aborts_infinite_loop() {
        let lua = Lua::new();
        disable_jit_if_present(&lua);
        let token = CancelToken::new();
        install_cancel_hook(&lua, &token, 1000);
        let h = token.handle();
        // Cancel from another thread while the loop runs on this one.
        let join = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            h.cancel();
        });
        let result = lua.load("while true do end").exec();
        join.join().unwrap();
        let err = result.expect_err("infinite loop should be cancelled");
        assert!(
            is_cancellation(&err),
            "expected IsolationError::Cancelled, got: {err:?}"
        );
    }

    #[test]
    fn hook_does_not_fire_when_flag_clear() {
        let lua = Lua::new();
        let token = CancelToken::new();
        install_cancel_hook(&lua, &token, 100);
        // Should run to completion without firing the hook.
        let r: i64 = lua
            .load(
                "local s = 0 \
                 for i = 1, 10000 do s = s + i end \
                 return s",
            )
            .eval()
            .expect("uncancelled chunk should run");
        assert_eq!(r, 50_005_000);
    }

    #[test]
    fn reset_after_cancel_allows_next_eval() {
        let lua = Lua::new();
        disable_jit_if_present(&lua);
        let token = CancelToken::new();
        install_cancel_hook(&lua, &token, 1000);
        let h = token.handle();
        // Trigger a cancel.
        let join = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            h.cancel();
        });
        let r1 = lua.load("while true do end").exec();
        join.join().unwrap();
        assert!(r1.is_err());
        // Reset and run another chunk; should succeed.
        token.reset();
        let r2: i64 = lua.load("return 42").eval().expect("post-reset eval");
        assert_eq!(r2, 42);
    }

    #[test]
    fn is_cancellation_walks_source_chain() {
        let token = CancelToken::new();
        token.cancel();
        let lua = Lua::new();
        disable_jit_if_present(&lua);
        install_cancel_hook(&lua, &token, 100);
        let err = lua
            .load("for i=1,1000000 do end")
            .exec()
            .expect_err("flag pre-set; should cancel");
        assert!(is_cancellation(&err));
    }

    #[test]
    fn is_cancellation_false_for_unrelated_error() {
        let lua = Lua::new();
        let err = lua
            .load("error('not a cancellation')")
            .exec()
            .expect_err("user error");
        assert!(!is_cancellation(&err));
    }
}
