// m6_8_multi_repl_acceptance.rs --- T M6.8 acceptance gates.

//! Acceptance gates for T M6.8 (Multi-REPL support).
//!
//! Spec acceptance bullets:
//!
//! 1. Three REPL buffers running concurrently render and respond
//!    independently → [`m6_8_three_repls_render_independently`],
//!    [`m6_8_three_repls_respond_independently`],
//!    [`m6_8_close_one_does_not_affect_others`].
//! 2. Process supervisor handles multiple PTY children without
//!    resource leaks → [`m6_8_supervisor_reaps_all_children_across_cycles`].
//!
//! Plus structural isolation tests that aren't in the bullets but
//! catch the most likely shape of multi-REPL regression (accidental
//! module-level state):
//!
//! - [`m6_8_repls_have_independent_parser_state`][] — ANSI parser state
//!   per-handle. One REPL entering alt-screen does not flip another's
//!   `_alt_screen` flag.
//! - [`m6_8_repls_have_independent_scrollback_state`][] — M6.7 `_blocks`
//!   array per-handle. A submit on one REPL does not extend another's
//!   block list.
//! - [`m6_8_buffer_scoped_bindings_route_to_active_buffer`][] — keymap
//!   dispatch resolves `pmacs.repl.submit-current` against the active
//!   buffer. Switching active buffer (via `pmacs.window.switch_buffer`,
//!   the production path) re-targets the binding without touching any
//!   handle state directly.
//! - [`m6_8_after_tick_hook_drains_all_handles_per_tick`][] — a single
//!   `tick_processes` call drains pending events for every registered
//!   handle, not just the first/last. Catches the failure mode where
//!   REPL B feels sluggish under load when other REPLs are also
//!   active.
//!
//! # Test process choice
//!
//! Lua REPL throughout when a standalone `lua` or `luajit` executable
//! is present. The embedded Lua build dependency does not guarantee a
//! shell binary on CI images. Lua's REPL is deterministic in a way
//! bash/zsh/fish are not (prompt content varies; some shells reorder
//! echo and prompt under raw mode). The contract under test is
//! multi-REPL isolation, not shell behavior.
//!
//! # Termination semantics for the resource-leak test
//!
//! After `Handle:close()` followed by tick draining: the supervisor
//! sends SIGTERM, the process exits, `_on_exit` fires, and (per the
//! M6.9 audit fix) the package calls `pmacs.process.forget` to
//! release the supervisor's record. The post-drain status is
//! therefore **nil** (forgotten), not `terminated`.
//!
//! Pre-M6.9, the package did not call forget, so the supervisor
//! retained terminated process records forever — a real leak that
//! compounded across spawn-close cycles. The leak test catches the
//! regression directly: spawn-three / close-three repeated K times,
//! verify `pmacs.process.list()` has no leftover REPL entries at the
//! end. Status nil after close-and-drain is the *expected* clean-
//! shutdown outcome, not a supervisor bug.
//!
//! What would still indicate a supervisor bug: status nil *before*
//! the exit event fired (premature forgetting). The close-isolation
//! test runs its status query after the survivor REPLs have echoed,
//! by which time the closed REPL's exit has been processed; status
//! nil at that point is graceful cleanup, terminal status is also
//! acceptable (timing-dependent).

use pmacs::editor::EditorState;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

/// Locate lua or luajit on the PATH. Both spawn a usable interactive
/// REPL via `-i`; the M6.8 tests don't care which. `PMACS_TEST_LUA`
/// or `PMACS_TEST_LUAJIT` overrides.
fn locate_lua() -> Option<PathBuf> {
    for name in ["lua", "luajit"] {
        let env_var = format!("PMACS_TEST_{}", name.to_uppercase());
        if let Ok(path) = std::env::var(&env_var) {
            let p = PathBuf::from(path);
            if p.is_file() {
                return Some(p);
            }
        }
        if let Ok(out) = std::process::Command::new("which").arg(name).output()
            && out.status.success()
            && let Ok(path) = String::from_utf8(out.stdout)
        {
            let path = path.trim();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    eprintln!(
        "skipping: lua/luajit not on PATH (set PMACS_TEST_LUA or PMACS_TEST_LUAJIT to override)"
    );
    None
}

/// Run setup, drive `tick_processes` until the predicate becomes
/// truthy, fail the test on timeout. Same shape as the M6.5 harness;
/// duplicated rather than shared because cross-test-binary sharing
/// would need a fixture crate.
fn run_with_pump(
    editor: &mut EditorState,
    setup_chunk: &str,
    predicate_chunk: &str,
    timeout_ms: u64,
) {
    editor
        .lua_host
        .eval(Some("@m6_8_setup"), setup_chunk)
        .expect("setup chunk runs");
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        editor.tick_processes();
        let truthy: bool = editor
            .lua_host
            .lua()
            .load(predicate_chunk)
            .eval()
            .expect("predicate chunk runs");
        if truthy {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "pump predicate did not become true within {timeout_ms}ms; \
             chunk:\n{predicate_chunk}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Spawn three lua REPLs and wait for all three to reach `running`.
/// Stores them as `_G.h1`, `_G.h2`, `_G.h3`. Returns when all three
/// processes have exec'd; subsequent operations can write to stdin
/// without racing the spawn.
fn spawn_three_and_wait_running(editor: &mut EditorState, lua: &Path) {
    let setup = format!(
        r#"
            _G.h1 = pmacs.repl.spawn {{ argv = {{ "{lua}", "-i" }} }}
            _G.h2 = pmacs.repl.spawn {{ argv = {{ "{lua}", "-i" }} }}
            _G.h3 = pmacs.repl.spawn {{ argv = {{ "{lua}", "-i" }} }}
        "#,
        lua = lua.display(),
    );
    run_with_pump(
        editor,
        &setup,
        r#"
            local function running(h)
                local s = pmacs.process.status(h._proc_id)
                return s and s.kind == "running"
            end
            return running(_G.h1) and running(_G.h2) and running(_G.h3)
        "#,
        5000,
    );
}

// ---------------------------------------------------------------------------
// Spec bullet 1: render and respond independently
// ---------------------------------------------------------------------------

/// Three REPLs spawned concurrently each have their own buffer; bytes
/// written to each end up in their own buffer's history, with no
/// cross-contamination. Verified at the byte level (each buffer has
/// the right unique marker, and only that marker).
#[test]
fn m6_8_three_repls_render_independently() {
    let Some(lua) = locate_lua() else {
        return;
    };
    let mut editor = EditorState::new();
    spawn_three_and_wait_running(&mut editor, &lua);

    // Write a unique marker through each REPL: `io.write("MARK_<n>\n")`
    // is a safe lua expression that produces deterministic output.
    // After the markers are echoed back, each buffer's history must
    // contain its own marker and not the others'.
    run_with_pump(
        &mut editor,
        r#"
            local function send(h, marker)
                pmacs.process.write_stdin(h._proc_id, "io.write('" .. marker .. "\\n')\n")
            end
            send(_G.h1, "MARK_ALPHA")
            send(_G.h2, "MARK_BETA")
            send(_G.h3, "MARK_GAMMA")
        "#,
        r#"
            local function has(h, m)
                return h:buffer_id():slice(0, h:history_end()):find(m, 1, true) ~= nil
            end
            return has(_G.h1, "MARK_ALPHA")
               and has(_G.h2, "MARK_BETA")
               and has(_G.h3, "MARK_GAMMA")
        "#,
        5000,
    );

    // Cross-contamination check: each buffer must contain only its own
    // marker, not its siblings'.
    let _: () = editor
        .lua_host
        .lua()
        .load(
            r#"
                local function has(h, m)
                    return h:buffer_id():slice(0, h:history_end()):find(m, 1, true) ~= nil
                end
                assert(not has(_G.h1, "MARK_BETA"),  "h1 leaked beta")
                assert(not has(_G.h1, "MARK_GAMMA"), "h1 leaked gamma")
                assert(not has(_G.h2, "MARK_ALPHA"), "h2 leaked alpha")
                assert(not has(_G.h2, "MARK_GAMMA"), "h2 leaked gamma")
                assert(not has(_G.h3, "MARK_ALPHA"), "h3 leaked alpha")
                assert(not has(_G.h3, "MARK_BETA"),  "h3 leaked beta")
                _G.h1:close(); _G.h2:close(); _G.h3:close()
            "#,
        )
        .exec()
        .expect("cross-contamination check");
}

/// Buffer-scoped RET binding routes input to the active buffer's
/// REPL: when h1's buffer is active, submit goes to h1; switch active
/// to h2's buffer and the same command targets h2.
///
/// Active-buffer mechanism: `pmacs.window.switch_buffer(buf)` is the
/// production code path (used by the package itself in `repl.spawn`
/// at line 184; verified to call `core.switch_active_buffer` in
/// `lua_bindings.rs::install_window_module`). The test exercises that
/// path; it does not synthesize active-buffer state by reaching into
/// internal tables.
#[test]
fn m6_8_three_repls_respond_independently() {
    let Some(lua) = locate_lua() else {
        return;
    };
    let mut editor = EditorState::new();
    spawn_three_and_wait_running(&mut editor, &lua);

    // Type into h1, switch to h2, type into h2, etc. Each typed marker
    // ends up in the corresponding REPL's buffer; no marker ends up in
    // a sibling's buffer.
    run_with_pump(
        &mut editor,
        r#"
            local function type_and_submit(h, line)
                pmacs.window.switch_buffer(h:buffer_id())
                local buf = h:buffer_id()
                buf:insert(buf:len(), line)
                pmacs.command.invoke("pmacs.repl.submit-current")
            end
            type_and_submit(_G.h1, "io.write('ROUTE_1\\n')")
            type_and_submit(_G.h2, "io.write('ROUTE_2\\n')")
            type_and_submit(_G.h3, "io.write('ROUTE_3\\n')")
        "#,
        r#"
            local function has(h, m)
                return h:buffer_id():slice(0, h:history_end()):find(m, 1, true) ~= nil
            end
            return has(_G.h1, "ROUTE_1")
               and has(_G.h2, "ROUTE_2")
               and has(_G.h3, "ROUTE_3")
        "#,
        5000,
    );

    // Cross-routing check: ROUTE_1 lives only in h1's history, etc.
    let _: () = editor
        .lua_host
        .lua()
        .load(
            r#"
                local function has(h, m)
                    return h:buffer_id():slice(0, h:history_end()):find(m, 1, true) ~= nil
                end
                assert(not has(_G.h1, "ROUTE_2"))
                assert(not has(_G.h1, "ROUTE_3"))
                assert(not has(_G.h2, "ROUTE_1"))
                assert(not has(_G.h2, "ROUTE_3"))
                assert(not has(_G.h3, "ROUTE_1"))
                assert(not has(_G.h3, "ROUTE_2"))
                _G.h1:close(); _G.h2:close(); _G.h3:close()
            "#,
        )
        .exec()
        .expect("cross-routing check");
}

/// Closing one REPL leaves the other two functional. Verifies that
/// the closed REPL's pump-removal does not de-register the after-tick
/// hook globally and that the supervisor's iteration over remaining
/// children is intact. The closed REPL's child receives SIGTERM and
/// reaches a terminal state; the survivors keep echoing.
#[test]
fn m6_8_close_one_does_not_affect_others() {
    let Some(lua) = locate_lua() else {
        return;
    };
    let mut editor = EditorState::new();
    spawn_three_and_wait_running(&mut editor, &lua);

    // Capture h2's proc_id before close; Handle:close clears the
    // handle's _proc_id field per the M6.5 contract, but the
    // supervisor still tracks the process and we want to query its
    // post-close state.
    run_with_pump(
        &mut editor,
        r#"
            _G.h2_proc_id = _G.h2._proc_id
            _G.h2:close()
            -- h1 and h3 should still respond. Send unique markers post-close.
            pmacs.process.write_stdin(_G.h1._proc_id, "io.write('SURVIVOR_1\\n')\n")
            pmacs.process.write_stdin(_G.h3._proc_id, "io.write('SURVIVOR_3\\n')\n")
        "#,
        r#"
            local function has(h, m)
                return h:buffer_id():slice(0, h:history_end()):find(m, 1, true) ~= nil
            end
            return has(_G.h1, "SURVIVOR_1") and has(_G.h3, "SURVIVOR_3")
        "#,
        5000,
    );

    let _: () = editor
        .lua_host
        .lua()
        .load(
            r#"
                -- h2's process should be in a terminal or forgotten state.
                -- Post-M6.9, _on_exit calls pmacs.process.forget; once the
                -- exit event fires, status flips to nil. Either outcome
                -- is graceful cleanup — what would indicate a bug is a
                -- non-terminal kind (running/starting/exiting hung).
                local s2 = pmacs.process.status(_G.h2_proc_id)
                if s2 ~= nil then
                    assert(s2.kind == "terminated" or s2.kind == "exiting",
                           "h2 should be terminal/exiting/forgotten; got "
                           .. tostring(s2.kind))
                end
                _G.h1:close(); _G.h3:close()
            "#,
        )
        .exec()
        .expect("close-one isolation check");
}

// ---------------------------------------------------------------------------
// Spec bullet 2: resource leak
// ---------------------------------------------------------------------------

/// Repeated spawn-three / close-three cycles. The leak detector:
/// `pmacs.process.list()` size must not grow across cycles. Pre-M6.9,
/// the supervisor retained terminated process records forever (no
/// auto-forget), so the list size grew by 3 per cycle; the M6.9 audit
/// fix added `pmacs.process.forget` to `_on_exit` so terminated
/// processes are released as their exit events fire.
///
/// K=10 cycles. A leak that accumulates one record per spawn-close
/// would show 30 leftover entries at the end; the gate asserts zero.
#[test]
fn m6_8_supervisor_reaps_all_children_across_cycles() {
    const CYCLES: usize = 10;
    let Some(lua) = locate_lua() else {
        return;
    };
    let mut editor = EditorState::new();

    // Baseline: list size before any spawning. The post-cycle list
    // size must equal this — no REPL processes left behind.
    let baseline_list_size: i64 = editor
        .lua_host
        .lua()
        .load("return #pmacs.process.list()")
        .eval()
        .expect("baseline list size");

    for cycle in 0..CYCLES {
        spawn_three_and_wait_running(&mut editor, &lua);
        // Capture proc_ids before close; Handle:close clears the
        // handle's _proc_id field per the M6.5 contract.
        editor
            .lua_host
            .eval(
                Some("@m6_8_close"),
                r"
                    _G.pid1 = _G.h1._proc_id
                    _G.pid2 = _G.h2._proc_id
                    _G.pid3 = _G.h3._proc_id
                    _G.h1:close(); _G.h2:close(); _G.h3:close()
                ",
            )
            .expect("close all three");

        // Pump until all three are forgotten (status nil) or terminal.
        // Either outcome indicates the supervisor has finished cleanup:
        // post-M6.9 the package calls forget in _on_exit, so status
        // typically transitions terminated → nil. Pre-M6.9 it stayed
        // terminated; the broader assertion accepts both transitions
        // so the test isn't fragile to ordering of the forget call
        // vs the predicate evaluation.
        run_with_pump(
            &mut editor,
            "",
            r#"
                local function done(pid)
                    local s = pmacs.process.status(pid)
                    if s == nil then return true end
                    return s.kind == "terminated"
                end
                return done(_G.pid1) and done(_G.pid2) and done(_G.pid3)
            "#,
            5000,
        );

        // For terminated outcomes, verify the kind is exited or
        // signaled (reject crashed: lua under SIGTERM should never
        // crash). Skip the check for nil (already forgotten —
        // exit-kind information is no longer available, but the
        // _on_exit path that called forget already received an
        // exited/signaled event).
        let _: () = editor
            .lua_host
            .lua()
            .load(
                r#"
                    local function check(pid, label)
                        local s = pmacs.process.status(pid)
                        if s == nil then return end  -- already forgotten
                        assert(s.kind == "terminated",
                               label .. ": non-terminal kind " .. tostring(s.kind))
                        assert(s.outcome == "exited" or s.outcome == "signaled",
                               label .. ": unexpected outcome " .. tostring(s.outcome))
                    end
                    check(_G.pid1, "h1"); check(_G.pid2, "h2"); check(_G.pid3, "h3")
                "#,
            )
            .exec()
            .unwrap_or_else(|e| panic!("cycle {cycle}: termination kind check: {e}"));
    }

    // Leak detector: after all cycles, the supervisor's process list
    // must have no REPL leftovers. Pre-M6.9 this would be CYCLES * 3
    // = 30 entries; the M6.9 audit fix brings it to 0.
    let final_list_size: i64 = editor
        .lua_host
        .lua()
        .load("return #pmacs.process.list()")
        .eval()
        .expect("final list size");
    assert_eq!(
        final_list_size,
        baseline_list_size,
        "supervisor leaked terminated process records: \
         baseline={baseline_list_size}, final={final_list_size}, \
         expected 0 leftover REPL entries (was {} = {CYCLES} cycles * 3 REPLs pre-M6.9)",
        CYCLES * 3
    );
}

// ---------------------------------------------------------------------------
// Structural isolation: parser, scrollback, bindings, drain
// ---------------------------------------------------------------------------

/// Each handle carries its own ANSI parser. Toggling alt-screen mode
/// on h1 (synthetic feed) does not flip h2's `_alt_screen` flag.
/// Catches the failure mode where parser state was accidentally
/// module-level. No process needed; pure synthetic test against
/// `pmacs.repl.create` (which the spawn path also calls).
#[test]
fn m6_8_repls_have_independent_parser_state() {
    let mut editor = EditorState::new();
    editor
        .lua_host
        .eval(
            Some("@m6_8_parser_iso"),
            r#"
        local h1 = pmacs.repl.create({ name = "*p1*" })
        local h2 = pmacs.repl.create({ name = "*p2*" })
        h1:append_output("\27[?1049h")  -- alt-screen enter on h1 only
        assert(h1:alt_screen_active(), "h1 should be in alt-screen")
        assert(not h2:alt_screen_active(), "h2 must not be in alt-screen")
        h1:append_output("\27[?1049l")
        assert(not h1:alt_screen_active())
        assert(not h2:alt_screen_active())
    "#,
        )
        .expect("parser-isolation chunk");
}

/// Each handle carries its own scrollback `_blocks` array. A submit
/// on h1 extends its block list; h2's block list stays untouched.
/// Catches the failure mode where the M6.7 block index was
/// accidentally shared.
#[test]
fn m6_8_repls_have_independent_scrollback_state() {
    let mut editor = EditorState::new();
    editor
        .lua_host
        .eval(
            Some("@m6_8_blocks_iso"),
            r#"
        local h1 = pmacs.repl.create({ name = "*b1*" })
        local h2 = pmacs.repl.create({ name = "*b2*" })
        h1:append_output("output for h1\n")
        h1:set_prompt("$ ")
        h1:submit()                    -- opens block 2 on h1
        assert(#h1._blocks == 2, "h1 should have 2 blocks; got " .. #h1._blocks)
        assert(#h2._blocks == 1,
               "h2 should still have 1 block; got " .. #h2._blocks)
    "#,
        )
        .expect("scrollback-isolation chunk");
}

/// `pmacs.repl.submit-current` resolves against the active buffer.
/// This test makes h1's buffer active, types, submits → h1's stdin
/// receives it. Switches active to h2, types, submits → h2's stdin
/// receives it (and h1's doesn't). The active-buffer mechanism is
/// the production `pmacs.window.switch_buffer`, not a back-door
/// table assignment.
#[test]
fn m6_8_buffer_scoped_bindings_route_to_active_buffer() {
    let Some(lua) = locate_lua() else {
        return;
    };
    let mut editor = EditorState::new();
    let setup = format!(
        r#"
            _G.h1 = pmacs.repl.spawn {{ argv = {{ "{lua}", "-i" }} }}
            _G.h2 = pmacs.repl.spawn {{ argv = {{ "{lua}", "-i" }} }}
        "#,
        lua = lua.display(),
    );
    run_with_pump(
        &mut editor,
        &setup,
        r#"
            local function running(h)
                local s = pmacs.process.status(h._proc_id)
                return s and s.kind == "running"
            end
            return running(_G.h1) and running(_G.h2)
        "#,
        5000,
    );

    // With h1 active, type and submit; "BIND_H1" should appear in h1.
    // With h2 active, type and submit; "BIND_H2" should appear in h2.
    run_with_pump(
        &mut editor,
        r#"
            pmacs.window.switch_buffer(_G.h1:buffer_id())
            local b1 = _G.h1:buffer_id()
            b1:insert(b1:len(), "io.write('BIND_H1\\n')")
            pmacs.command.invoke("pmacs.repl.submit-current")

            pmacs.window.switch_buffer(_G.h2:buffer_id())
            local b2 = _G.h2:buffer_id()
            b2:insert(b2:len(), "io.write('BIND_H2\\n')")
            pmacs.command.invoke("pmacs.repl.submit-current")
        "#,
        r#"
            local function has(h, m)
                return h:buffer_id():slice(0, h:history_end()):find(m, 1, true) ~= nil
            end
            return has(_G.h1, "BIND_H1") and has(_G.h2, "BIND_H2")
        "#,
        5000,
    );

    let _: () = editor
        .lua_host
        .lua()
        .load(
            r#"
                local function has(h, m)
                    return h:buffer_id():slice(0, h:history_end()):find(m, 1, true) ~= nil
                end
                assert(not has(_G.h1, "BIND_H2"), "binding leaked from h2 to h1")
                assert(not has(_G.h2, "BIND_H1"), "binding leaked from h1 to h2")
                _G.h1:close(); _G.h2:close()
            "#,
        )
        .exec()
        .expect("binding-routing isolation");
}

/// A single tick of `tick_processes` drains pending events for every
/// registered handle. The shape of the test:
///
/// 1. Spawn three; wait for all running.
/// 2. Write a unique marker to each REPL's stdin (synchronous).
/// 3. Sleep long enough for each process to echo back into its kernel
///    pipe and for the supervisor's reader thread to pick those bytes
///    off into the bounded channel.
/// 4. Drive **one** `tick_processes` call.
/// 5. Assert all three buffers grew by at least the marker's length
///    on that tick.
///
/// This catches the failure mode where the after-tick callback drains
/// REPL A but not B in the same tick because of an iteration bug or
/// short-circuit, which would manifest as REPL B feeling sluggish
/// when other REPLs are also active.
#[test]
fn m6_8_after_tick_hook_drains_all_handles_per_tick() {
    let Some(lua) = locate_lua() else {
        return;
    };
    let mut editor = EditorState::new();
    spawn_three_and_wait_running(&mut editor, &lua);

    // Capture pre-tick history_end on each handle.
    let pre_lengths: Vec<i64> = editor
        .lua_host
        .lua()
        .load(
            r"
                return { _G.h1:history_end(), _G.h2:history_end(), _G.h3:history_end() }
            ",
        )
        .eval::<mlua::Table>()
        .expect("pre-tick lengths")
        .sequence_values::<i64>()
        .map(|r| r.expect("seq value"))
        .collect();
    assert_eq!(pre_lengths.len(), 3, "expected 3 pre-tick lengths");

    // Write markers to all three stdins, synchronously and in the same
    // Lua chunk so all three writes happen before any tick fires.
    editor
        .lua_host
        .eval(
            Some("@m6_8_drain_writes"),
            r#"
                pmacs.process.write_stdin(_G.h1._proc_id, "io.write('TICK_M1\\n')\n")
                pmacs.process.write_stdin(_G.h2._proc_id, "io.write('TICK_M2\\n')\n")
                pmacs.process.write_stdin(_G.h3._proc_id, "io.write('TICK_M3\\n')\n")
            "#,
        )
        .expect("drain writes");

    // Sleep long enough for each lua REPL to echo + emit + the
    // supervisor's reader thread to receive into the bounded channel.
    // 500 ms is generous on local hardware (lua's interactive loop
    // responds in single-digit ms typically); CI is slower but well
    // under this bound.
    std::thread::sleep(Duration::from_millis(500));

    // ONE tick.
    editor.tick_processes();

    // Post-tick: every buffer's history_end must have grown.
    let post_lengths: Vec<i64> = editor
        .lua_host
        .lua()
        .load(
            r"
                return { _G.h1:history_end(), _G.h2:history_end(), _G.h3:history_end() }
            ",
        )
        .eval::<mlua::Table>()
        .expect("post-tick lengths")
        .sequence_values::<i64>()
        .map(|r| r.expect("seq value"))
        .collect();
    for i in 0..3 {
        assert!(
            post_lengths[i] > pre_lengths[i],
            "handle {} did not grow in single tick: pre={}, post={}",
            i + 1,
            pre_lengths[i],
            post_lengths[i]
        );
    }

    let _: () = editor
        .lua_host
        .lua()
        .load("_G.h1:close(); _G.h2:close(); _G.h3:close()")
        .exec()
        .expect("teardown");
}
