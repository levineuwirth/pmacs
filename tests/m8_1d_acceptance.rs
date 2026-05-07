// tests/m8_1d_acceptance.rs --- T M8.1d reload + on_unload acceptance.

//! Acceptance tests for `pmacs.packages.reload(name)` and
//! `pmacs.packages.on_unload(fn)`.
//!
//! Reload is the dev-loop completion: combined with M8.1c's
//! `install_local`, it lets the M8.2-M8.10 package author edit
//! source files on disk and observe the change without restarting
//! pmacs. The `on_unload` hook gives packages with non-Lua state
//! (running children, open file handles) a place to clean up.
//!
//! The contracts pinned here:
//!
//! - `reload(name)` clears `package.loaded[name]` (and submodules)
//!   then re-`require`s, so changes on disk become visible.
//! - `on_unload(fn)` registers a per-package callback recovered
//!   from the calling chunk's `_PACKAGE.name` env field --- no
//!   manual basename argument required from the package author.
//! - Hooks fire in registration order on the first reload after
//!   they were registered, then are cleared so the re-loaded
//!   chunk's fresh hooks own the next cycle.
//! - `reload` of a name not in the roster errors clearly.
//! - `on_unload` called from non-package code errors clearly with
//!   a pointer at the shutdown-hook alternative.

use std::path::PathBuf;

use pmacs::lua::LuaHost;
use pmacs::lua_bindings::PackageInstallOverride;
use tempfile::TempDir;

fn make_local_pkg_with_init(name: &str, init_body: &str) -> (TempDir, PathBuf) {
    let td = tempfile::tempdir().expect("tempdir");
    let dir = td.path().to_path_buf();
    let manifest = format!(
        "name = \"{name}\"\n\
         version = \"1.0.0\"\n\
         summary = \"M8.1d reload fixture: {name}\"\n\
         pmacs_required = \">= 0.1.0\"\n\
         entry = \"init.lua\"\n\
         exports = [\"{name}\"]\n"
    );
    std::fs::write(dir.join("pmacs.toml"), manifest).expect("write manifest");
    std::fs::write(dir.join("init.lua"), init_body).expect("write entry");
    (td, dir)
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
// T M8.1d --- the dev-loop test: edit on disk, reload, see new behavior
// ---------------------------------------------------------------------------

#[test]
fn reload_picks_up_edits_to_install_local_source_on_disk() {
    let (_pkg_td, pkg_path) = make_local_pkg_with_init(
        "reload-disk",
        "return { name = 'reload-disk', marker = 'before-edit' }\n",
    );
    let pkg_str = pkg_path.display().to_string();
    let (mut host, _c, _u) = host_with_overrides();

    let warm = format!(
        r#"
        pmacs.packages.install_local("{pkg_str}")
        local m = require("reload-disk")
        assert(m.marker == "before-edit", "warm-up should see 'before-edit'")
    "#
    );
    host.eval(Some("warm"), &warm)
        .unwrap_or_else(|e| panic!("warm-up failed: {e}"));

    // Edit the source on disk. This is the entire point of
    // install_local + reload: the editor running pmacs is
    // simulating the M8 package author iterating on their package
    // without restarting.
    std::fs::write(
        pkg_path.join("init.lua"),
        "return { name = 'reload-disk', marker = 'after-edit' }\n",
    )
    .expect("re-write entry");

    let reloaded = r#"
        local m = pmacs.packages.reload("reload-disk")
        assert(m.marker == "after-edit",
            "reload should yield 'after-edit', got " .. tostring(m.marker))
        -- And a regular require post-reload returns the same bytes.
        local m2 = require("reload-disk")
        assert(m2.marker == "after-edit", "post-reload require should also see 'after-edit'")
    "#;
    host.eval(Some("reload"), reloaded)
        .unwrap_or_else(|e| panic!("reload after edit failed: {e}"));
}

// ---------------------------------------------------------------------------
// T M8.1d --- on_unload runs in registration order on reload
// ---------------------------------------------------------------------------

#[test]
fn on_unload_hooks_run_in_registration_order_on_reload() {
    // Package's init registers two on_unload hooks that each push
    // a marker onto a global. Reload should see both fire in the
    // order they were registered.
    let (_pkg_td, pkg_path) = make_local_pkg_with_init(
        "reload-hooks",
        r#"
            _G.RELOAD_HOOK_LOG = _G.RELOAD_HOOK_LOG or {}
            pmacs.packages.on_unload(function()
                table.insert(_G.RELOAD_HOOK_LOG, "first")
            end)
            pmacs.packages.on_unload(function()
                table.insert(_G.RELOAD_HOOK_LOG, "second")
            end)
            return { name = 'reload-hooks' }
        "#,
    );
    let pkg_str = pkg_path.display().to_string();
    let (mut host, _c, _u) = host_with_overrides();

    let script = format!(
        r#"
        pmacs.packages.install_local("{pkg_str}")
        require("reload-hooks")  -- registers two on_unload hooks
        assert(#_G.RELOAD_HOOK_LOG == 0, "hooks haven't fired yet")

        pmacs.packages.reload("reload-hooks")
        assert(#_G.RELOAD_HOOK_LOG == 2, "exactly two hooks ran; got " .. #_G.RELOAD_HOOK_LOG)
        assert(_G.RELOAD_HOOK_LOG[1] == "first", "first hook should fire first")
        assert(_G.RELOAD_HOOK_LOG[2] == "second", "second hook should fire second")
    "#
    );
    host.eval(Some("hooks"), &script)
        .unwrap_or_else(|e| panic!("on_unload ordering failed: {e}"));
}

// ---------------------------------------------------------------------------
// T M8.1d --- hook list is consumed on reload (re-loaded chunk re-registers)
// ---------------------------------------------------------------------------

#[test]
fn on_unload_hooks_consumed_on_reload_then_freshly_registered() {
    // Without consume-on-reload, the hooks would accumulate: each
    // reload would fire all prior hooks plus the freshly-registered
    // ones. The contract is: each load registers fresh hooks; each
    // reload consumes whatever was registered since the last load.
    let (_pkg_td, pkg_path) = make_local_pkg_with_init(
        "reload-consume",
        r#"
            _G.CONSUME_LOG = _G.CONSUME_LOG or {}
            pmacs.packages.on_unload(function()
                table.insert(_G.CONSUME_LOG, "tick")
            end)
            return { name = 'reload-consume' }
        "#,
    );
    let pkg_str = pkg_path.display().to_string();
    let (mut host, _c, _u) = host_with_overrides();

    let script = format!(
        r#"
        pmacs.packages.install_local("{pkg_str}")
        require("reload-consume")            -- registers 1 hook
        pmacs.packages.reload("reload-consume")  -- runs 1, registers 1 (chunk re-runs)
        pmacs.packages.reload("reload-consume")  -- runs 1, registers 1
        pmacs.packages.reload("reload-consume")  -- runs 1, registers 1
        assert(#_G.CONSUME_LOG == 3,
            "expected 3 ticks across 3 reloads (hook list consumed each time); got " ..
            #_G.CONSUME_LOG)
    "#
    );
    host.eval(Some("consume"), &script)
        .unwrap_or_else(|e| panic!("hook consumption test failed: {e}"));
}

// ---------------------------------------------------------------------------
// T M8.1d --- reload of a package without any on_unload hooks works
// ---------------------------------------------------------------------------

#[test]
fn reload_of_package_with_no_on_unload_hooks_succeeds_silently() {
    let (_pkg_td, pkg_path) = make_local_pkg_with_init(
        "reload-no-hooks",
        "return { name = 'reload-no-hooks', marker = 1 }\n",
    );
    let pkg_str = pkg_path.display().to_string();
    let (mut host, _c, _u) = host_with_overrides();

    let script = format!(
        r#"
        pmacs.packages.install_local("{pkg_str}")
        local m1 = require("reload-no-hooks")
        local m2 = pmacs.packages.reload("reload-no-hooks")
        assert(m2.marker == 1, "reload without hooks must re-yield the same module")
    "#
    );
    host.eval(Some("no-hooks"), &script)
        .unwrap_or_else(|e| panic!("hookless reload failed: {e}"));
}

// ---------------------------------------------------------------------------
// T M8.1d --- reload of an unknown package name surfaces a clear error
// ---------------------------------------------------------------------------

#[test]
fn reload_of_unknown_package_name_returns_clear_error() {
    let (mut host, _c, _u) = host_with_overrides();
    let err = host
        .eval(Some("unknown"), r#"pmacs.packages.reload("no-such-pkg")"#)
        .expect_err("must error on unknown name");
    let msg = err.to_string();
    assert!(
        msg.contains("no-such-pkg"),
        "error must name the unknown package: {msg}"
    );
    assert!(
        msg.contains("installed()") || msg.contains("not in"),
        "error should point at the roster as the source of truth: {msg}"
    );
}

// ---------------------------------------------------------------------------
// T M8.1d --- on_unload called from non-package code errors clearly
// ---------------------------------------------------------------------------

#[test]
fn on_unload_called_outside_a_package_chunk_errors_clearly() {
    // Calling on_unload from `init.lua`'s top-level (or any chunk
    // that isn't a package's init) has no owning package to attach
    // the hook to. The error should name a real alternative API
    // (`pmacs.hook.add('editor.before-quit', ...)`) so the user
    // knows what to do.
    let (mut host, _c, _u) = host_with_overrides();
    let err = host
        .eval(Some("outside"), r"pmacs.packages.on_unload(function() end)")
        .expect_err("must error when called outside a package");
    let msg = err.to_string();
    assert!(
        msg.contains("on_unload") && msg.contains("package"),
        "error should explain the constraint: {msg}"
    );
    assert!(
        msg.contains("hook.add") && msg.contains("editor.before-quit"),
        "error should point at the actual hook API for non-package teardown: {msg}"
    );
}

// ---------------------------------------------------------------------------
// M8.1 H#1 regression --- reload clears the per-package _ENV cache so
// globals removed in source disappear after reload, not just the
// chunk's return value
// ---------------------------------------------------------------------------

#[test]
fn reload_clears_package_env_so_removed_globals_disappear() {
    // First load: writes a package-local global `_ENV.SECRET = "v1"`
    // (the chunk runs inside a per-package _ENV with __index=_G, so
    // bare assignment lands in the env).
    // Edit on disk: source no longer sets SECRET.
    // After reload, _ENV.SECRET must be nil. Without env-cache
    // clearing, the prior chunk's SECRET assignment would persist
    // because the env table outlives the chunk that wrote it ---
    // exactly the dev-loop trap the M8.1 review caught.
    let (_pkg_td, pkg_path) = make_local_pkg_with_init(
        "reload-env",
        r#"
            SECRET = "v1"
            return {
                read_secret = function() return SECRET end,
            }
        "#,
    );
    let pkg_str = pkg_path.display().to_string();
    let (mut host, _c, _u) = host_with_overrides();

    let warm = format!(
        r#"
        pmacs.packages.install_local("{pkg_str}")
        local m = require("reload-env")
        assert(m.read_secret() == "v1", "warm-up must see SECRET = 'v1'")
    "#
    );
    host.eval(Some("warm"), &warm)
        .unwrap_or_else(|e| panic!("warm-up failed: {e}"));

    // New version of the source: SECRET is gone, and the closure
    // reads it via _ENV.SECRET (which should now be nil).
    std::fs::write(
        pkg_path.join("init.lua"),
        r"
            return {
                read_secret = function() return SECRET end,
            }
        ",
    )
    .expect("re-write entry");

    let after = r#"
        local m = pmacs.packages.reload("reload-env")
        local got = m.read_secret()
        assert(got == nil,
            "after reload, SECRET should be nil (env cache cleared); " ..
            "got " .. tostring(got))
    "#;
    host.eval(Some("after"), after)
        .unwrap_or_else(|e| panic!("env-cache regression failed: {e}"));
}

// ---------------------------------------------------------------------------
// M8.1 M#2 regression --- on_unload cannot be spoofed by setting
// _PACKAGE in _G or any user-controlled table
// ---------------------------------------------------------------------------

#[test]
fn on_unload_rejects_forged_package_marker_in_global_env() {
    // Pre-fix, on_unload trusted callback.environment()._PACKAGE.name.
    // Top-level user code could set `_PACKAGE = { name = "victim" }`
    // in _G and then call on_unload to inject a hook against the
    // victim package. Post-fix, the binding identity-checks the env
    // table against the registered per-package envs in
    // pmacs.pkgenvs --- _G is never a registered env, so the call
    // errors regardless of any forged _PACKAGE field.
    let (mut host, _c, _u) = host_with_overrides();
    let err = host
        .eval(
            Some("forge"),
            r#"
                _PACKAGE = { name = "victim-package" }
                pmacs.packages.on_unload(function() end)
            "#,
        )
        .expect_err("forged _PACKAGE must not satisfy on_unload");
    let msg = err.to_string();
    assert!(
        msg.contains("on_unload"),
        "error must name on_unload: {msg}"
    );
    // The forged name must not appear in the error --- if it did,
    // the binding had read it (and registered the hook) before
    // erroring, which would defeat the security claim.
    assert!(
        !msg.contains("victim-package"),
        "forged _PACKAGE.name must not be readable: {msg}"
    );
}

// ---------------------------------------------------------------------------
// M8.1 2pass M#1 regression --- install_local replacement runs
// the prior load's on_unload hooks (and clears them so they don't
// fire alongside the new load's hooks on a later reload)
// ---------------------------------------------------------------------------

#[test]
fn install_local_replacement_runs_old_unload_hooks_then_clears_them() {
    let (a_td, _a_path) = make_local_pkg_with_init(
        "swap-hooks",
        r"
            _G.SWAP_LOG = _G.SWAP_LOG or {}
            pmacs.packages.on_unload(function()
                table.insert(_G.SWAP_LOG, 'A-unload')
            end)
            return { name = 'swap-hooks', version = 'A' }
        ",
    );
    let a_str = a_td.path().display().to_string();

    let (b_td, _b_path) = make_local_pkg_with_init(
        "swap-hooks",
        r"
            _G.SWAP_LOG = _G.SWAP_LOG or {}
            pmacs.packages.on_unload(function()
                table.insert(_G.SWAP_LOG, 'B-unload')
            end)
            return { name = 'swap-hooks', version = 'B' }
        ",
    );
    let b_str = b_td.path().display().to_string();

    let (mut host, _c, _u) = host_with_overrides();

    let script = format!(
        r#"
        pmacs.packages.install_local("{a_str}")
        local a = require("swap-hooks")
        assert(a.version == 'A', "first load yields A")
        assert(#_G.SWAP_LOG == 0, "no hooks fired yet")

        -- Replace A with B at the same basename. install_local
        -- should fire A's hooks before swapping in B; B's chunk
        -- hasn't run yet so it has no hooks of its own at this
        -- point.
        pmacs.packages.install_local("{b_str}")
        assert(#_G.SWAP_LOG == 1, "A's hook should fire on swap; got " .. #_G.SWAP_LOG)
        assert(_G.SWAP_LOG[1] == 'A-unload')

        -- Loading B registers a fresh hook against the B chunk's env.
        local b = require("swap-hooks")
        assert(b.version == 'B', "post-swap require yields B")

        -- Reload should run *only* B's hook --- A's hook list was
        -- drained at install_local time. Pre-fix, both A's and B's
        -- hooks would be in the registry and both would fire.
        pmacs.packages.reload("swap-hooks")
        assert(#_G.SWAP_LOG == 2,
            "reload after swap should fire only B's hook; got " .. #_G.SWAP_LOG)
        assert(_G.SWAP_LOG[2] == 'B-unload',
            "second log entry should be B-unload, got " .. tostring(_G.SWAP_LOG[2]))
    "#
    );
    host.eval(Some("swap"), &script)
        .unwrap_or_else(|e| panic!("install_local hook handoff failed: {e}"));
}

// ---------------------------------------------------------------------------
// M8.1 3pass M#2 regression --- a failing on_unload hook stays at
// the front of the registry so retry re-attempts the same cleanup
// ---------------------------------------------------------------------------

#[test]
fn reload_failing_hook_is_retried_until_it_succeeds() {
    // Hooks must be idempotent: a hook that fails on cycle 1 will
    // be re-attempted on cycle 2 (because the cleanup it represents
    // didn't complete). Only a successful call pops the hook from
    // the registry.
    //
    // Pre-fix the popped-then-called shape silently dropped the
    // failing hook, so cycle 2 jumped to hook 3 (skipping the
    // cleanup that actually failed). That meant a buggy
    // `worker:stop()` would never get retried; the package would
    // claim to have unloaded but leave its worker running.
    let (_pkg_td, pkg_path) = make_local_pkg_with_init(
        "reload-retry",
        r"
            _G.RETRY_LOG = _G.RETRY_LOG or {}
            _G.MIDDLE_HOOK_SHOULD_FAIL = (_G.MIDDLE_HOOK_SHOULD_FAIL == nil) and true or _G.MIDDLE_HOOK_SHOULD_FAIL
            pmacs.packages.on_unload(function()
                table.insert(_G.RETRY_LOG, '1')
            end)
            pmacs.packages.on_unload(function()
                table.insert(_G.RETRY_LOG, '2-attempt')
                if _G.MIDDLE_HOOK_SHOULD_FAIL then
                    error('middle hook intentional fail')
                end
            end)
            pmacs.packages.on_unload(function()
                table.insert(_G.RETRY_LOG, '3')
            end)
            return { name = 'reload-retry' }
        ",
    );
    let pkg_str = pkg_path.display().to_string();
    let (mut host, _c, _u) = host_with_overrides();

    // First reload: hook 1 runs (popped on success). Hook 2
    // errors --- it stays at the front of the registry. Hook 3
    // doesn't run.
    let first = format!(
        r#"
        pmacs.packages.install_local("{pkg_str}")
        require("reload-retry")
        local ok, err = pcall(pmacs.packages.reload, "reload-retry")
        assert(not ok, "first reload must fail because hook 2 errors")
        assert(#_G.RETRY_LOG == 2,
            "after first reload, exactly 2 entries (1 + 2-attempt); got " .. #_G.RETRY_LOG)
        assert(_G.RETRY_LOG[1] == '1')
        assert(_G.RETRY_LOG[2] == '2-attempt')
    "#
    );
    host.eval(Some("first"), &first)
        .unwrap_or_else(|e| panic!("first reload setup failed: {e}"));

    // Second reload, with the failure-flag cleared. Hook 2 (still
    // at the front) is RE-ATTEMPTED, this time succeeding. Hook 3
    // then runs. Hook 1 is NOT re-run (it succeeded on cycle 1
    // and was popped). The post-pop re-require registers fresh
    // 1/2/3 hooks against the new chunk but they don't run yet.
    let second = r#"
        _G.MIDDLE_HOOK_SHOULD_FAIL = false
        local ok = pcall(pmacs.packages.reload, "reload-retry")
        assert(ok, "second reload should succeed")
        -- Expected log delta: '2-attempt' (retry succeeds) then '3'.
        assert(#_G.RETRY_LOG == 4,
            "second reload should re-run hook 2 then run hook 3; got " ..
            #_G.RETRY_LOG ..
            " entries: " .. table.concat(_G.RETRY_LOG, ","))
        assert(_G.RETRY_LOG[3] == '2-attempt',
            "hook 2 must be retried, not skipped; got " .. _G.RETRY_LOG[3])
        assert(_G.RETRY_LOG[4] == '3',
            "hook 3 must run after hook 2 retry succeeds; got " .. _G.RETRY_LOG[4])
    "#;
    host.eval(Some("second"), second)
        .unwrap_or_else(|e| panic!("retry reload failed: {e}"));
}

// ---------------------------------------------------------------------------
// M8.1 4pass M#1 regression --- on_unload registrations during a
// cycle land in the next cycle, not the current one
// ---------------------------------------------------------------------------

#[test]
fn on_unload_registered_during_cycle_does_not_run_in_same_cycle() {
    // Pre-fix the cycle ran from the live registry; a hook that
    // called `on_unload(...)` from inside its body would extend
    // the current cycle. A self-replicating hook would loop
    // reload() forever.
    //
    // Post-fix the cycle runs from a snapshot drained at start;
    // new registrations land in the empty live registry slot.
    // Then `clear_package_env` drops them at end-of-cycle (they
    // hold the now-discarded chunk's env), and the freshly-
    // re-required chunk registers its own clean hooks for the
    // next cycle.
    //
    // Net effect: a self-replicating hook fires exactly once per
    // reload (the just-required chunk's registration), and the
    // cycle returns instead of hanging.
    let (_pkg_td, pkg_path) = make_local_pkg_with_init(
        "reload-self-replicate",
        r"
            _G.SELF_REPLICATE_LOG = _G.SELF_REPLICATE_LOG or {}
            local function clean_up()
                table.insert(_G.SELF_REPLICATE_LOG, 'tick')
                -- Re-register *this same closure*. A live-queue
                -- runner would loop forever here; a snapshot
                -- runner runs the queued hook once and lets the
                -- self-registration land in the registry for the
                -- next cycle (where it'll be discarded with the
                -- old env).
                pmacs.packages.on_unload(clean_up)
            end
            pmacs.packages.on_unload(clean_up)
            return { name = 'reload-self-replicate' }
        ",
    );
    let pkg_str = pkg_path.display().to_string();
    let (mut host, _c, _u) = host_with_overrides();

    let script = format!(
        r#"
        pmacs.packages.install_local("{pkg_str}")
        require("reload-self-replicate")

        -- Reload 1: snapshot is [clean_up], queue runs once,
        -- clean_up's body re-registers clean_up. clear_package_env
        -- then drops that surviving hook (its env is the now-
        -- discarded old env). require re-runs the chunk against
        -- the new env, registering a fresh clean_up.
        pmacs.packages.reload("reload-self-replicate")
        assert(#_G.SELF_REPLICATE_LOG == 1,
            "first reload must fire exactly once; got " .. #_G.SELF_REPLICATE_LOG)

        -- Reload 2: same shape. Fresh chunk's hook fires; the
        -- self-registered copy is dropped at clear time.
        pmacs.packages.reload("reload-self-replicate")
        assert(#_G.SELF_REPLICATE_LOG == 2,
            "second reload should fire exactly once more; got " ..
            #_G.SELF_REPLICATE_LOG)

        -- Reload 3 confirms the steady state.
        pmacs.packages.reload("reload-self-replicate")
        assert(#_G.SELF_REPLICATE_LOG == 3,
            "third reload should fire exactly once more; got " ..
            #_G.SELF_REPLICATE_LOG)

        -- Most importantly: every reload returned. A live-queue
        -- runner would have hung the test forever on reload 1.
    "#
    );
    host.eval(Some("self-replicate"), &script)
        .unwrap_or_else(|e| panic!("self-replicating-hook test failed: {e}"));
}

// ---------------------------------------------------------------------------
// M8.1 3pass H#1 regression --- install_local with a failing
// on_unload hook leaves the disk symlink unchanged
// ---------------------------------------------------------------------------

#[test]
fn install_local_failing_hook_leaves_symlink_unchanged() {
    // Install A, register an on_unload hook that always errors.
    // Try to install_local B at the same name. The hook fires
    // BEFORE the symlink swap (per the H#1 plan/commit split), so
    // its failure aborts install_local with disk untouched: the
    // symlink still points at A, the roster still tracks A,
    // require("name") still loads A, and the failing hook stays in
    // the registry for retry.
    let (a_td, _a_path) = make_local_pkg_with_init(
        "swap-fail",
        r"
            _G.SWAPFAIL_LOG = _G.SWAPFAIL_LOG or {}
            _G.SWAPFAIL_HOOK_FAILS = (_G.SWAPFAIL_HOOK_FAILS == nil) and true or _G.SWAPFAIL_HOOK_FAILS
            pmacs.packages.on_unload(function()
                table.insert(_G.SWAPFAIL_LOG, 'A-attempt')
                if _G.SWAPFAIL_HOOK_FAILS then
                    error('A unload intentional fail')
                end
            end)
            return { name = 'swap-fail', version = 'A' }
        ",
    );
    let a_str = a_td.path().display().to_string();

    let (b_td, _b_path) = make_local_pkg_with_init(
        "swap-fail",
        r"
            return { name = 'swap-fail', version = 'B' }
        ",
    );
    let b_str = b_td.path().display().to_string();

    let (mut host, _c, user_root) = host_with_overrides();

    // Install A and require it (registers the failing hook).
    let setup = format!(
        r#"
        pmacs.packages.install_local("{a_str}")
        local a = require("swap-fail")
        assert(a.version == 'A')
    "#
    );
    host.eval(Some("setup"), &setup)
        .unwrap_or_else(|e| panic!("setup failed: {e}"));

    let a_target = std::fs::read_link(user_root.path().join("swap-fail"))
        .expect("read symlink before swap attempt");

    // Try install_local B: must error because A's on_unload fails.
    let attempt = format!(r#"pmacs.packages.install_local("{b_str}")"#);
    let err = host
        .eval(Some("attempt"), &attempt)
        .expect_err("install_local must fail when A's unload hook errors");
    let msg = err.to_string();
    assert!(
        msg.contains("A unload intentional fail") || msg.contains("intentional fail"),
        "error must surface the failing hook's message; got {msg}"
    );

    // Disk is unchanged: the symlink still points at A's source.
    let after_target = std::fs::read_link(user_root.path().join("swap-fail"))
        .expect("read symlink after failed swap");
    assert_eq!(
        a_target, after_target,
        "symlink target must be unchanged after a failed install_local"
    );
    assert!(
        !user_root.path().join(".swap-fail.swap.tmp").exists(),
        "failed install_local must clean up the staged swap symlink"
    );

    // require still returns A (the existing module is cached).
    host.eval(
        Some("require_after"),
        r"
            local m = require('swap-fail')
            assert(m.version == 'A',
                'after failed swap, require must still return A; got ' .. tostring(m.version))
        ",
    )
    .unwrap_or_else(|e| panic!("post-failure require check failed: {e}"));

    // Now disable the hook failure and retry install_local B. The
    // hook re-attempts and succeeds; B is installed, require
    // returns B.
    let recovery = format!(
        r#"
        _G.SWAPFAIL_HOOK_FAILS = false
        pmacs.packages.install_local("{b_str}")
        local b = require('swap-fail')
        assert(b.version == 'B',
            'after recovery, require must return B; got ' .. tostring(b.version))
        -- Hook A-attempt should appear twice in the log: once for
        -- the failed first attempt, once for the successful retry.
        assert(#_G.SWAPFAIL_LOG == 2,
            'A-attempt should appear twice; got ' .. #_G.SWAPFAIL_LOG ..
            ' entries: ' .. table.concat(_G.SWAPFAIL_LOG, ','))
    "#
    );
    host.eval(Some("recovery"), &recovery)
        .unwrap_or_else(|e| panic!("recovery install_local failed: {e}"));
}

// ---------------------------------------------------------------------------
// T M8.1d --- reloadable packages can define commands and clean them up
// ---------------------------------------------------------------------------
//
// Regression for the M8.2 close-out finding: a package that defines
// commands at top level cannot be reloaded if it doesn't unregister
// them in its unload hook (the second chunk run hits DuplicateName).
// This test pins the contract end-to-end:
//
// 1. install_local a package that defines a command and registers
//    an on_unload hook to unregister it.
// 2. invoke the command, observe v1 behavior.
// 3. edit the package's command body on disk.
// 4. reload the package.
// 5. invoke the command, observe v2 behavior --- proving both that
//    the unload hook successfully cleared the slot and that re-define
//    succeeded.

#[test]
fn reload_works_for_package_that_defines_and_unregisters_commands() {
    let init_v1 = r#"
        local M = {}
        pmacs.command.define {
            name = "rwc.greet",
            description = "Greet (v1).",
            fn = function() return "hello-v1" end,
        }
        pmacs.packages.on_unload(function()
            pmacs.command.unregister("rwc.greet")
        end)
        return M
    "#;
    let (_pkg_td, pkg_path) = make_local_pkg_with_init("reload-with-commands", init_v1);
    let pkg_str = pkg_path.display().to_string();
    let (mut host, _c, _u) = host_with_overrides();

    let warm = format!(
        r#"
        pmacs.packages.install_local("{pkg_str}")
        require("reload-with-commands")
        local r = pmacs.command.invoke("rwc.greet")
        assert(r == "hello-v1", "v1 command should return 'hello-v1'; got " .. tostring(r))
    "#
    );
    host.eval(Some("warm-rwc"), &warm)
        .unwrap_or_else(|e| panic!("warm-up failed: {e}"));

    // Edit the package on disk: change command body.
    let init_v2 = r#"
        local M = {}
        pmacs.command.define {
            name = "rwc.greet",
            description = "Greet (v2).",
            fn = function() return "hello-v2" end,
        }
        pmacs.packages.on_unload(function()
            pmacs.command.unregister("rwc.greet")
        end)
        return M
    "#;
    std::fs::write(pkg_path.join("init.lua"), init_v2).expect("rewrite init.lua");

    let reload_chunk = r#"
        pmacs.packages.reload("reload-with-commands")
        local r = pmacs.command.invoke("rwc.greet")
        assert(r == "hello-v2",
            "after reload, command must return 'hello-v2'; got " .. tostring(r))
        -- describe.command should also pick up the new description.
        local info = pmacs.describe.command("rwc.greet")
        assert(info.description == "Greet (v2).",
            "describe.command must reflect the v2 description; got " ..
            tostring(info.description))
    "#;
    host.eval(Some("reload-rwc"), reload_chunk)
        .unwrap_or_else(|e| panic!("reload + invoke v2 failed: {e}"));
}

// Companion regression: post-init reload must work when a package's
// on_unload hook calls pmacs.command.unregister. The earlier
// command-defining reload test runs in init phase (LuaHost::new()
// leaves init open), which would mask an init-gated unregister.
// This test flips init-complete after install_local --- the same
// state the editor reaches at runtime --- so a future re-introduction
// of the gate would fail here.

#[test]
fn reload_after_init_complete_runs_unregister_in_unload_hook() {
    let init_v1 = r#"
        pmacs.command.define {
            name = "rwc-postinit.greet",
            description = "Greet (v1).",
            fn = function() return "post-v1" end,
        }
        pmacs.packages.on_unload(function()
            pmacs.command.unregister("rwc-postinit.greet")
        end)
        return {}
    "#;
    let (_pkg_td, pkg_path) = make_local_pkg_with_init("reload-postinit", init_v1);
    let pkg_str = pkg_path.display().to_string();
    let (mut host, _c, _u) = host_with_overrides();

    // Install during init phase (the editor's startup window).
    host.eval(
        Some("warm-postinit"),
        &format!(
            r#"
            pmacs.packages.install_local("{pkg_str}")
            require("reload-postinit")
        "#
        ),
    )
    .unwrap_or_else(|e| panic!("warm-up failed: {e}"));

    // Flip the init flag the way EditorState's startup sequence
    // does. From here on, install_local and other init-only APIs
    // would refuse --- but reload should still work, and the
    // unload hook's unregister call must succeed in this state.
    host.set_init_complete();

    // Edit on disk and reload from the post-init editor.
    let init_v2 = r#"
        pmacs.command.define {
            name = "rwc-postinit.greet",
            description = "Greet (v2).",
            fn = function() return "post-v2" end,
        }
        pmacs.packages.on_unload(function()
            pmacs.command.unregister("rwc-postinit.greet")
        end)
        return {}
    "#;
    std::fs::write(pkg_path.join("init.lua"), init_v2).expect("rewrite init.lua");

    host.eval(
        Some("postinit-reload"),
        r#"
            pmacs.packages.reload("reload-postinit")
            local r = pmacs.command.invoke("rwc-postinit.greet")
            assert(r == "post-v2",
                "post-init reload must update command body; got " .. tostring(r))
        "#,
    )
    .unwrap_or_else(|e| panic!("post-init reload failed: {e}"));
}

// Companion test: a package that *forgets* to unregister its
// commands hits DuplicateName on reload, with an actionable error.
// This is the failure mode the regression test above guards against.

#[test]
fn reload_of_command_defining_package_without_cleanup_errors_clearly() {
    let init_no_cleanup = r#"
        pmacs.command.define {
            name = "rwc-leak.greet",
            description = "Greet without cleanup.",
            fn = function() return "v1" end,
        }
        return {}
    "#;
    let (_pkg_td, pkg_path) = make_local_pkg_with_init("reload-leak", init_no_cleanup);
    let pkg_str = pkg_path.display().to_string();
    let (mut host, _c, _u) = host_with_overrides();

    host.eval(
        Some("warm-leak"),
        &format!(
            r#"
            pmacs.packages.install_local("{pkg_str}")
            require("reload-leak")
        "#
        ),
    )
    .unwrap_or_else(|e| panic!("install + require failed: {e}"));

    // No edit needed --- the second chunk run will collide.
    let err = host
        .eval(
            Some("reload-leak"),
            r#"pmacs.packages.reload("reload-leak")"#,
        )
        .expect_err("reload of command-leaking package must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("rwc-leak.greet") && msg.contains("already defined"),
        "error must point at the duplicate command: {msg}"
    );
}
