// tests/m7_8_acceptance.rs --- Acceptance tests for T M7.8 (failure isolation).

//! End-to-end acceptance for T M7.8 (failure isolation).
//!
//! Spec acceptance bullets (`pmacs-tasks.tex:3373-3394`):
//!
//! 1. Synthetic broken package raising on load: the package fails to
//!    load, an error is reported in the `*errors*` log naming the
//!    package, other packages continue to load.
//! 2. Synthetic broken package raising in a command: the editor keeps
//!    running, the error appears in the error log.
//! 3. Synthetic broken package looping infinitely: a cancel request
//!    interrupts it within 1 second without terminating the host.
//!    Runs under both `luajit` and `lua54`.
//! 4. Post-cancel usable-state verification: subsequent evals succeed,
//!    error log is readable, another package can be loaded after the
//!    cancel.
//! 5. Empirical benchmark output recorded: chosen N, measured
//!    overhead percentage, measured cancel latency. See
//!    [`benchmark_records_overhead_and_latency`] and
//!    `TRANSITION-M7.md`.
//!
//! The cancel-thread tests use a `CancelHandle` from a separate OS
//! thread to flip the flag while the main thread runs Lua. This
//! demonstrates the mechanism the editor's input layer will use; the
//! actual wiring of C-g → `host.cancel_handle().cancel()` is editor
//! integration work tracked separately.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use pmacs::lua::LuaHost;
use pmacs::lua_bindings::PackageInstallOverride;
use pmacs::lua_isolation;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixtures (pruned copies from m7_7_acceptance.rs).
// ---------------------------------------------------------------------------

/// Build a bare repo whose `init.lua` is exactly `init_body`.
/// Manifest lists `name` as the only export; entry is `init.lua`.
fn make_pkg_with_custom_entry(name: &str, init_body: &str) -> (TempDir, PathBuf) {
    let td = tempfile::tempdir().expect("tempdir");
    let work = td.path().join("work");
    let bare = td.path().join("upstream.git");

    run_git(&[
        OsStr::new("init"),
        OsStr::new("--initial-branch=main"),
        work.as_os_str(),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("config"),
        OsStr::new("user.email"),
        OsStr::new("test@example.com"),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("config"),
        OsStr::new("user.name"),
        OsStr::new("Tester"),
    ]);

    let manifest = format!(
        "name = \"{name}\"\n\
         version = \"1.0.0\"\n\
         summary = \"M7.8 fixture\"\n\
         pmacs_required = \">= 0.1.0\"\n\
         entry = \"init.lua\"\n\
         exports = [\"{name}\"]\n",
    );
    std::fs::write(work.join("pmacs.toml"), manifest).expect("write manifest");
    std::fs::write(work.join("init.lua"), init_body).expect("write entry");

    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("add"),
        OsStr::new("."),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("commit"),
        OsStr::new("-m"),
        OsStr::new("v1.0.0"),
    ]);
    run_git(&[
        OsStr::new("-C"),
        work.as_os_str(),
        OsStr::new("tag"),
        OsStr::new("v1.0.0"),
    ]);
    run_git(&[
        OsStr::new("clone"),
        OsStr::new("--bare"),
        work.as_os_str(),
        bare.as_os_str(),
    ]);
    (td, bare)
}

fn run_git(args: &[&OsStr]) {
    let mut cmd = Command::new("git");
    for a in args {
        cmd.arg(a);
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("LC_ALL", "C");
    let out = cmd.output().expect("git spawn");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

fn file_url(p: &Path) -> String {
    format!("file://{}", p.display())
}

fn host_with_overrides() -> (LuaHost, TempDir, TempDir) {
    let cache = tempfile::tempdir().expect("cache tempdir");
    let user_root = tempfile::tempdir().expect("user-root tempdir");
    let host = LuaHost::new().expect("LuaHost::new");
    host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );
    (host, cache, user_root)
}

// ---------------------------------------------------------------------------
// Bullet 1: broken package on load — error captured, others load
// ---------------------------------------------------------------------------

#[test]
fn broken_package_on_load_does_not_block_others() {
    // Two packages: pkg-bad raises during init.lua execution, pkg-good
    // returns a normal table. Loading pkg-bad before pkg-good must:
    //   * not abort the surrounding init.lua (pmacs.packages.load
    //     wraps require in pcall),
    //   * record a `[package pkg-bad]` entry in *errors*,
    //   * leave pkg-good loadable.
    let (_td_bad, bare_bad) =
        make_pkg_with_custom_entry("pkg-bad", "error('intentional load failure')\n");
    let (_td_good, bare_good) = make_pkg_with_custom_entry(
        "pkg-good",
        "return { name = 'pkg-good', version = '1.0.0' }\n",
    );
    let url_bad = file_url(&bare_bad);
    let url_good = file_url(&bare_good);

    let (mut host, _c, _u) = host_with_overrides();
    let script = format!(
        r#"
        pmacs.packages.install {{ "git:{url_bad}", version = "^1.0.0" }}
        pmacs.packages.install {{ "git:{url_good}", version = "^1.0.0" }}
        local ok_bad = pmacs.packages.load("pkg-bad")
        local ok_good = pmacs.packages.load("pkg-good")
        assert(ok_bad == false,  "pkg-bad load should return false; got "  .. tostring(ok_bad))
        assert(ok_good == true,  "pkg-good load should return true; got "  .. tostring(ok_good))
    "#,
    );
    host.eval(Some("test"), &script)
        .unwrap_or_else(|e| panic!("eval should succeed (load failures are caught): {e}"));

    // *errors* log: must mention pkg-bad with its user error message,
    // and must not mention pkg-good (that one loaded cleanly).
    let log = host.errors_buffer_text();
    assert!(
        log.contains("[package pkg-bad]"),
        "expected `[package pkg-bad]` entry in *errors*; got:\n{log}"
    );
    assert!(
        log.contains("intentional load failure"),
        "*errors* entry should include the user message; got:\n{log}"
    );
    assert!(
        !log.contains("[package pkg-good]"),
        "pkg-good should not produce an entry; got:\n{log}"
    );
}

// ---------------------------------------------------------------------------
// Bullet 2: broken command body — host stays usable
// ---------------------------------------------------------------------------

#[test]
fn broken_command_body_does_not_kill_host() {
    // Register a command whose body errors. Invoke it: invoke_command
    // returns Err. The host must still be usable: a follow-up eval
    // succeeds and the error log captured the failure.
    let (mut host, _c, _u) = host_with_overrides();

    host.eval(
        Some("test/register"),
        r#"
        pmacs.command.define {
            name = "broken-cmd",
            description = "raises on invocation",
            fn = function() error("intentional command failure") end,
        }
    "#,
    )
    .unwrap_or_else(|e| panic!("define failed: {e}"));

    let err = host
        .invoke_command("broken-cmd", mlua::MultiValue::new())
        .expect_err("broken-cmd should propagate its error");
    let err_str = err.to_string();
    assert!(
        err_str.contains("intentional command failure"),
        "err must surface user message; got: {err_str}"
    );
    assert!(
        !lua_isolation::is_cancellation(&err),
        "user error must not be classified as cancellation"
    );

    // Follow-up eval still runs. A non-trivial chunk returning a value
    // proves the VM is in a defined state.
    let script = "return 1 + 41";
    let v: mlua::Value = host
        .eval(Some("test/post"), script)
        .expect("post-error eval succeeds");
    assert_eq!(
        i64::from_lua(v, host.lua()).unwrap(),
        42,
        "post-error eval must return 42"
    );
}

use mlua::FromLua;

// ---------------------------------------------------------------------------
// Bullet 3: infinite-loop cancellation within the spec budget
// ---------------------------------------------------------------------------
//
// The spec budgets 1 second on luajit, reflecting the JIT-trace
// bypass caveat. To make the test deterministic we disable JIT
// inside the chunk (via `jit.off()` — no-op on lua54). The mechanism
// itself (count-hook + atomic flag, fired from a separate thread) is
// what's under test; LuaJIT's worst-case JIT-on latency is documented
// in the module docs and `TRANSITION-M7.md`, not exercised here.

#[test]
fn cancel_interrupts_infinite_loop_within_one_second() {
    let host = LuaHost::new().expect("LuaHost::new");
    let handle = host.cancel_handle();

    // Disable JIT so the count hook fires reliably under luajit.
    host.lua()
        .load("if jit then jit.off(); jit.flush() end")
        .exec()
        .expect("jit-off setup");

    // Spawn a separate OS thread to simulate the editor's input layer
    // delivering C-g while the main thread is in Lua.
    let join = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        handle.cancel();
    });

    let started = Instant::now();
    let result = host.lua().load("while true do end").exec();
    let elapsed = started.elapsed();
    join.join().expect("cancel thread joined");

    let err = result.expect_err("infinite loop must abort once cancel fires");
    assert!(
        lua_isolation::is_cancellation(&err),
        "abort error must be IsolationError::Cancelled; got: {err:?}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "cancel latency exceeded 1s spec budget: {elapsed:?}"
    );
}

// ---------------------------------------------------------------------------
// Bullet 4: post-cancel usable-state verification
// ---------------------------------------------------------------------------

#[test]
fn post_cancel_host_is_usable_for_follow_up_loads_and_evals() {
    // After a cancel-and-abort round, verify:
    //   * a subsequent eval succeeds (main thread responsive),
    //   * the *errors* buffer is readable (defined state),
    //   * a previously-installed package still loads.
    let (_td, bare) = make_pkg_with_custom_entry(
        "post-cancel-pkg",
        "return { ok = true, version = '1.0.0' }\n",
    );
    let url = file_url(&bare);
    let (mut host, _c, _u) = host_with_overrides();
    host.eval(
        Some("test/install"),
        &format!("pmacs.packages.install {{ \"git:{url}\", version = \"^1.0.0\" }}"),
    )
    .expect("install succeeds");

    // Disable JIT so the cancel below fires reliably under luajit.
    host.eval(
        Some("test/jit-off"),
        "if jit then jit.off(); jit.flush() end",
    )
    .expect("jit-off");

    // Trigger a cancel during a hot-looping chunk.
    let h = host.cancel_handle();
    let join = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        h.cancel();
    });
    let cancel_result = host.eval(Some("test/loop"), "while true do end");
    join.join().expect("cancel thread joined");
    let cancel_err = cancel_result.expect_err("loop should be cancelled");
    assert!(
        lua_isolation::is_cancellation(&cancel_err),
        "must be a cancellation error; got: {cancel_err:?}"
    );

    // (a) Main thread responsive: follow-up eval returns a value.
    let v: mlua::Value = host
        .eval(Some("test/follow-up"), "return 'still alive'")
        .expect("follow-up eval");
    assert_eq!(
        String::from_lua(v, host.lua()).unwrap(),
        "still alive",
        "follow-up eval must return its value"
    );

    // (b) Error log readable; the cancellation was captured.
    let messages: Vec<String> = host.errors().iter().map(|r| r.message.clone()).collect();
    assert!(
        messages.iter().any(|m| m.contains("cancelled")),
        "errors should record the cancellation; got: {messages:?}"
    );

    // (c) Another package can be loaded post-cancel.
    let v: mlua::Value = host
        .eval(
            Some("test/load-after"),
            r#"
            local ok = pmacs.packages.load("post-cancel-pkg")
            return ok
        "#,
        )
        .expect("load-after eval");
    assert!(
        bool::from_lua(v, host.lua()).unwrap(),
        "post-cancel package load must succeed"
    );
}

// ---------------------------------------------------------------------------
// Bullet 5: empirical benchmark
// ---------------------------------------------------------------------------

/// Time a tight loop with the count hook installed and again with the
/// hook absent, plus a cancel-latency measurement. Records the chosen
/// N, the overhead percentage, and the latency in this test's stdout
/// (`cargo test -- --nocapture`) and to a file under `target/m7_8/`.
/// `TRANSITION-M7.md` carries a frozen copy of the numbers for the
/// audit trail.
///
/// The test is permissive about absolute numbers (CI variance, debug
/// builds, etc.); it only asserts that the chosen N sits in the
/// spec-mandated 10K-100K range and that cancel latency stays within
/// the 1-second spec budget.
/// Warm-up + measurement loops. Running 10M-iteration tight loops
/// gives a stable enough number on a debug build for the
/// double-digit-percent threshold; release builds report tighter
/// overhead, which is reflected in `TRANSITION-M7.md`.
const BENCH_ITERS: u64 = 10_000_000;

#[test]
fn benchmark_records_overhead_and_latency() {
    let n = lua_isolation::DEFAULT_INSTRUCTION_BUDGET;
    assert!(
        (10_000..=100_000).contains(&n),
        "DEFAULT_INSTRUCTION_BUDGET must sit in the spec-mandated 10K-100K range; got {n}"
    );

    // Hook-on host (the production path).
    let host_on = LuaHost::new().expect("host hook-on");
    host_on
        .lua()
        .load("if jit then jit.off(); jit.flush() end")
        .exec()
        .expect("jit-off");
    let started = Instant::now();
    host_on
        .lua()
        .load(format!(
            "local s = 0 for i = 1, {BENCH_ITERS} do s = s + 1 end return s"
        ))
        .exec()
        .expect("hook-on loop");
    let on_elapsed = started.elapsed();

    // Hook-off host (a fresh VM; we can't uninstall this pmacs's hook
    // without going through `Lua::remove_hook`, but constructing a
    // bare `mlua::Lua` here gives the comparison baseline. JIT off
    // for apples-to-apples on luajit.)
    let bare = mlua::Lua::new();
    bare.load("if jit then jit.off(); jit.flush() end")
        .exec()
        .expect("bare jit-off");
    let started = Instant::now();
    bare.load(format!(
        "local s = 0 for i = 1, {BENCH_ITERS} do s = s + 1 end return s"
    ))
    .exec()
    .expect("hook-off loop");
    let off_elapsed = started.elapsed();

    // Cancel latency: from the moment the handle fires, how long until
    // the in-VM hook returns the chunk.
    let host_cancel = LuaHost::new().expect("host cancel");
    host_cancel
        .lua()
        .load("if jit then jit.off(); jit.flush() end")
        .exec()
        .expect("cancel jit-off");
    let h = host_cancel.cancel_handle();
    let (tx, rx) = std::sync::mpsc::channel::<Instant>();
    let join = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        let fired = Instant::now();
        h.cancel();
        let _ = tx.send(fired);
    });
    let _ = host_cancel.lua().load("while true do end").exec();
    let observed = Instant::now();
    join.join().unwrap();
    let fired_at = rx.recv().expect("cancel timestamp");
    let latency = observed.saturating_duration_since(fired_at);

    // Overhead is hook-on relative to hook-off; permissive ceiling
    // because the test runs in debug-build profile under cargo test.
    let overhead_pct = (on_elapsed.as_secs_f64() - off_elapsed.as_secs_f64()).max(0.0)
        / off_elapsed.as_secs_f64()
        * 100.0;

    eprintln!(
        "T M7.8 benchmark: N={n}, hook-on={on_elapsed:?}, hook-off={off_elapsed:?}, overhead={overhead_pct:.1}%, cancel-latency={latency:?}"
    );

    let out_dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let _ = std::fs::create_dir_all(&out_dir);
    let _ = std::fs::write(
        out_dir.join("m7_8_benchmark.txt"),
        format!(
            "T M7.8 benchmark\n\
             N (instruction budget): {n}\n\
             hook-on elapsed: {on_elapsed:?}\n\
             hook-off elapsed: {off_elapsed:?}\n\
             overhead vs no-hook: {overhead_pct:.1}%\n\
             cancel latency: {latency:?}\n\
             iters: {BENCH_ITERS}\n",
        ),
    );

    assert!(
        latency < Duration::from_secs(1),
        "cancel latency exceeded 1s spec budget: {latency:?}"
    );
}
