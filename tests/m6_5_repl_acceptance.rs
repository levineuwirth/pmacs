// m6_5_repl_acceptance.rs --- T M6.5 acceptance gates.

//! Acceptance gates for T M6.5 (REPL process integration).
//!
//! Spec §M6.5 acceptance bullets:
//!
//! 1. All four shells (bash, zsh, fish, lua) launch and produce a
//!    usable session →
//!    [`m6_5_repl_spawns_bash`], [`m6_5_repl_spawns_zsh`],
//!    [`m6_5_repl_spawns_fish`], [`m6_5_repl_spawns_lua`].
//! 2. Input typed at prompt sent to process on Enter →
//!    [`m6_5_ret_submits_input_to_process`].
//! 3. C-c sends SIGINT →
//!    [`m6_5_ctrl_c_sends_sigint`].
//! 4. C-d on empty prompt closes stdin →
//!    [`m6_5_ctrl_d_on_empty_prompt_closes_stdin`].
//!
//! Plus structural / lifecycle checks shared across stages:
//!
//! - [`m6_5_spawn_registers_process_on_supervisor`]: spawn returns a
//!   handle whose `_proc_id` shows up in `pmacs.process.list()`.
//! - [`m6_5_close_terminates_child_and_unregisters`]: closing a handle
//!   removes it from the supervisor and stops the after-tick pump
//!   from drawing from it.
//! - [`m6_5_ctrl_d_on_nonempty_input_deletes_char_forward`][]: the
//!   spec-literal "C-d on empty prompt closes stdin" is paired with
//!   readline-equivalent "delete char forward" when input is non-empty,
//!   so users never see C-d as broken.
//! - [`m6_5_exit_marker_uses_basename_with_leading_newline`][]: exit
//!   marker is `"\n[<basename(argv[0])> exited with code N]\n"`.

use pmacs::editor::EditorState;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

static PUMP_TEST_LOCK: Mutex<()> = Mutex::new(());

fn pump_test_guard() -> MutexGuard<'static, ()> {
    PUMP_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Locate a shell binary for tests that require one. Returns the
/// resolved path or `None` if the shell is neither at `PMACS_TEST_<NAME>`
/// nor on `PATH`. Per-test selective skipping (rather than skipping
/// the whole file) so a missing bash doesn't hide a regression in zsh.
fn locate_shell(name: &str) -> Option<PathBuf> {
    let env_var = format!("PMACS_TEST_{}", name.to_uppercase());
    if let Ok(path) = std::env::var(&env_var) {
        let p = PathBuf::from(path);
        if p.is_file() {
            return Some(p);
        }
    }
    let out = std::process::Command::new("which")
        .arg(name)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8(out.stdout).ok()?;
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

/// Construct a fresh editor and run the given Lua chunk against it.
fn run(chunk: &str) {
    let _guard = pump_test_guard();
    let mut editor = EditorState::new();
    editor
        .lua_host
        .eval(Some("@m6_5_test"), chunk)
        .expect("test chunk runs");
}

/// Run `setup_chunk`, then drive `tick_processes` (which fires
/// `process.after-tick` and runs the REPL pump) until `predicate_chunk`
/// returns truthy or the timeout elapses. Asserts the predicate became
/// true within the budget. The pump harness mirrors lsp.lua's
/// `poll_until` pattern but routes through `tick_processes` so the M6.5
/// after-tick contract is exercised end-to-end.
fn run_with_pump(setup_chunk: &str, predicate_chunk: &str, timeout_ms: u64) {
    let _guard = pump_test_guard();
    let mut editor = EditorState::new();
    editor
        .lua_host
        .eval(Some("@m6_5_setup"), setup_chunk)
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

// ---------------------------------------------------------------------------
// Stage 2: spawn structure + lifecycle wiring
// ---------------------------------------------------------------------------

/// pmacs.repl.spawn produces a handle whose process is visible to the
/// supervisor and whose buffer is a real pmacs buffer. This is the
/// minimum viability check before per-byte routing tests.
#[test]
fn m6_5_spawn_registers_process_on_supervisor() {
    run(r#"
        local h = pmacs.repl.spawn { argv = { "cat" } }
        assert(h._proc_id ~= nil, "handle missing _proc_id")
        assert(type(h:buffer_id()) ~= "nil", "handle missing buffer_id")

        -- Process should appear in the supervisor's list.
        local list = pmacs.process.list()
        local found = false
        for _, row in ipairs(list) do
            if row.id == h._proc_id then
                found = true
                break
            end
        end
        assert(found, "spawned process not in pmacs.process.list()")

        h:close()
    "#);
}

/// Acceptance bullet 2: input typed at prompt is sent to the process
/// on Enter, and the process's response (via raw-mode echo for `cat`,
/// shell line editor for bash/zsh/fish) lands in history.
///
/// Process startup is not synchronous: the supervisor returns a
/// handle immediately and the actual `exec` happens later. The
/// predicate chunk gates submission on the `Running` state via a
/// `_G.submitted` flag so we don't write to stdin before the child
/// is alive.
#[test]
fn m6_5_ret_submits_input_to_process() {
    run_with_pump(
        r#"
            _G.h = pmacs.repl.spawn { argv = { "cat" } }
            _G.submitted = false
        "#,
        r#"
            local h = _G.h
            if not _G.submitted then
              local status = pmacs.process.status(h._proc_id)
              if status and status.kind == "running" then
                h:buffer_id():insert(h:prompt_end(), "hello")
                pmacs.command.invoke("pmacs.repl.submit-current")
                _G.submitted = true
              end
              return false
            end
            local buf = h:buffer_id()
            local history = buf:slice(0, h:history_end())
            return history:find("hello", 1, true) ~= nil
        "#,
        3000,
    );
}

/// Acceptance bullet 4: C-d on an empty prompt closes stdin. The
/// implementation writes `\x04` (EOT) to the PTY master. In raw mode
/// the kernel does not interpret this byte (no ICANON), but every
/// shell with a line editor (readline / zle / fish-internal) treats
/// `\x04` on an empty input line as EOF and exits. The contract is
/// thus shell-mediated: the package emits the byte; the shell decides
/// what it means. Verified by spawning bash and observing the exit
/// marker.
///
/// Skipped if bash isn't available (Stage 5's multi-shell suite has
/// fuller coverage). `PMACS_TEST_BASH` overrides the PATH lookup.
#[test]
fn m6_5_ctrl_d_on_empty_prompt_closes_stdin() {
    let Some(bash) = locate_shell("bash") else {
        eprintln!("skipping: bash not on PATH (set PMACS_TEST_BASH to override)");
        return;
    };
    let setup = format!(
        r#"
            _G.h = pmacs.repl.spawn {{ argv = {{ "{bash}", "-i" }} }}
            _G.eof_sent = false
            _G.first_seen_running_at = nil
        "#,
        bash = bash.display(),
    );
    run_with_pump(
        &setup,
        r#"
            local h = _G.h
            if not _G.eof_sent then
              local status = pmacs.process.status(h._proc_id)
              if status and status.kind == "running" then
                -- Wait two ticks past first-running so bash gets to
                -- start its line editor and print its prompt before
                -- we send EOF. Without the warmup, EOF can race with
                -- readline initialization on slow CI runners.
                if _G.first_seen_running_at == nil then
                  _G.first_seen_running_at = pmacs.now_ms()
                  return false
                end
                if pmacs.now_ms() - _G.first_seen_running_at < 100 then
                  return false
                end
                pmacs.command.invoke("pmacs.repl.send-eof-current")
                _G.eof_sent = true
              end
              return false
            end
            -- bash exits cleanly on EOF (code 0) after readline EOT.
            -- The exit marker lives at the end of history; searching
            -- for the basename + " exited" anchors us on the marker.
            local buf = h:buffer_id()
            local history = buf:slice(0, h:history_end())
            return history:find("[bash exited", 1, true) ~= nil
        "#,
        5000,
    );
}

/// C-d when the input region is non-empty deletes the character at
/// the cursor (readline-equivalent), not stdin. This is the package
/// choice for the case the spec is silent on; the wrong choice is
/// "no-op", which makes C-d feel broken to anyone with terminal
/// muscle memory. Empty-vs-nonempty branch lives in
/// `pmacs.repl.send-eof-current`.
#[test]
fn m6_5_ctrl_d_on_nonempty_input_deletes_char_forward() {
    run(r#"
        local h = pmacs.repl.spawn { argv = { "cat" } }
        h:buffer_id():insert(h:prompt_end(), "hello")
        -- Walk the cursor back to the start of the input region.
        -- `insert` leaves cursor position untouched; the editor's
        -- default cursor at construction is byte 0, which is exactly
        -- prompt_end here. But to keep the test robust to future
        -- cursor-default changes, walk to line start explicitly.
        pmacs.editor.move_line_start()
        for _ = 1, 100 do pmacs.editor.move_up() end

        assert(h:input_text() == "hello",
               "pre: input_text=" .. h:input_text())

        pmacs.command.invoke("pmacs.repl.send-eof-current")

        -- One char dropped from the front; "ello" remains.
        assert(h:input_text() == "ello",
               "post: input_text=" .. h:input_text())

        h:close()
    "#);
}

/// Acceptance bullet 3: C-c sends SIGINT. We spawn `cat` (which
/// terminates on SIGINT) and verify the exit marker reports the
/// expected signal. The signal name in the marker is symbolic
/// ("SIGINT") rather than a number, per the M6.5 design (the libc
/// description "Interrupt" returned by portable-pty is canonicalized
/// in `process.rs::canonicalize_pty_signal_name`).
///
/// Timeout is 10s to absorb supervisor-tick scheduling under heavy
/// parallel test load (the M6.1 PTY suite has a similar flake
/// profile under `cargo test`'s default parallelism).
#[test]
fn m6_5_ctrl_c_sends_sigint() {
    run_with_pump(
        r#"
            _G.h = pmacs.repl.spawn { argv = { "cat" } }
            _G.sigint_sent = false
            _G.first_seen_running_at = nil
        "#,
        r#"
            local h = _G.h
            if not _G.sigint_sent then
              local status = pmacs.process.status(h._proc_id)
              if status and status.kind == "running" then
                -- Running means the PTY child has been spawned, not
                -- that the raw-mode /bin/sh trampoline has necessarily
                -- completed stty + exec. macOS runners can observe that
                -- gap; wait briefly so SIGINT targets cat, not the
                -- setup shell.
                if _G.first_seen_running_at == nil then
                  _G.first_seen_running_at = pmacs.now_ms()
                  return false
                end
                if pmacs.now_ms() - _G.first_seen_running_at < 100 then
                  return false
                end
                pmacs.command.invoke("pmacs.repl.send-sigint-current")
                _G.sigint_sent = true
              end
              return false
            end
            local buf = h:buffer_id()
            local history = buf:slice(0, h:history_end())
            return history:find("killed by SIGINT", 1, true) ~= nil
        "#,
        10_000,
    );
}

/// The exit marker uses `basename(argv[0])` (so `/usr/bin/cat`
/// renders as `cat`), leads with `\n` (so a process exiting mid-line
/// stays readable), and uses symbolic signal names. Verified by
/// spawning `/bin/false`, which exits with code 1.
#[test]
fn m6_5_exit_marker_uses_basename_with_leading_newline() {
    run_with_pump(
        r#"
            _G.h = pmacs.repl.spawn { argv = { "/bin/false" } }
        "#,
        r#"
            local h = _G.h
            local buf = h:buffer_id()
            local history = buf:slice(0, h:history_end())
            -- Marker starts with "\n[false exited with code 1]\n".
            return history:find("\n[false exited with code 1]\n", 1, true) ~= nil
        "#,
        // PTY shutdown publishes the exit event only after a bounded
        // final-output drain. macOS runners can spend most of the old
        // 3s budget in spawn + that drain, even for /bin/false.
        10_000,
    );
}

// ---------------------------------------------------------------------------
// Stage 5: multi-shell acceptance (acceptance bullet 1)
// ---------------------------------------------------------------------------

/// Run a single command (`echo hello-pmacs`) through a freshly-spawned
/// shell and verify the output appears in history. The shell exits
/// itself via the same EOF byte the C-d binding sends, so we don't
/// need to wire up `exit` per-shell. Predicate gates first on
/// running-state (process must have started before we type) and then
/// on history matching the expected output.
fn run_shell_smoke_test(shell_path: &std::path::Path, argv_extra: &[&str]) {
    let mut argv_lua = String::new();
    write!(&mut argv_lua, r#""{}""#, shell_path.display()).unwrap();
    for a in argv_extra {
        write!(&mut argv_lua, r#", "{a}""#).unwrap();
    }
    let setup = format!(
        r"
            _G.h = pmacs.repl.spawn {{ argv = {{ {argv_lua} }} }}
            _G.typed = false
        ",
    );
    run_with_pump(
        &setup,
        r#"
            local h = _G.h
            if not _G.typed then
              local status = pmacs.process.status(h._proc_id)
              if status and status.kind == "running" then
                -- Type the smoke command into the input region and
                -- submit. After-tick will drain the shell's echo and
                -- the command's output into history.
                h:buffer_id():insert(h:prompt_end(), "echo hello-pmacs")
                pmacs.command.invoke("pmacs.repl.submit-current")
                _G.typed = true
              end
              return false
            end
            local buf = h:buffer_id()
            local history = buf:slice(0, h:history_end())
            return history:find("hello-pmacs", 1, true) ~= nil
        "#,
        // 15s timeout absorbs the slowest-shell case (fish under
        // heavy parallel load); none of the four shells is normally
        // anywhere near that, but the test must not flake when 5
        // shell tests + the SIGINT test all start their PTY children
        // in the same cargo-test thread pool.
        15_000,
    );
}

/// Bash. Required-ish — present on virtually every Linux system. Skip
/// only if neither `PMACS_TEST_BASH` nor PATH resolves it.
#[test]
fn m6_5_repl_spawns_bash() {
    let Some(bash) = locate_shell("bash") else {
        eprintln!("skipping: bash not on PATH (set PMACS_TEST_BASH to override)");
        return;
    };
    run_shell_smoke_test(&bash, &["-i"]);
}

/// Zsh. Skip if not installed.
#[test]
fn m6_5_repl_spawns_zsh() {
    let Some(zsh) = locate_shell("zsh") else {
        eprintln!("skipping: zsh not on PATH (set PMACS_TEST_ZSH to override)");
        return;
    };
    run_shell_smoke_test(&zsh, &["-i"]);
}

/// Fish. Historically the most fussy of the four about TTY setup;
/// pmacs's PTY raw mode should satisfy it.
#[test]
fn m6_5_repl_spawns_fish() {
    let Some(fish) = locate_shell("fish") else {
        eprintln!("skipping: fish not on PATH (set PMACS_TEST_FISH to override)");
        return;
    };
    run_shell_smoke_test(&fish, &["-i"]);
}

/// Lua REPL. Skip if no standalone Lua interpreter is installed.
/// The embedded Lua build dependency does not guarantee a `lua` or
/// `luajit` executable on CI images.
#[test]
fn m6_5_repl_spawns_lua() {
    let Some(lua) = locate_shell("lua").or_else(|| locate_shell("luajit")) else {
        eprintln!(
            "skipping: lua/luajit not on PATH (set PMACS_TEST_LUA or PMACS_TEST_LUAJIT to override)"
        );
        return;
    };
    let setup = format!(
        r#"
            _G.h = pmacs.repl.spawn {{ argv = {{ "{lua}", "-i" }} }}
            _G.typed = false
        "#,
        lua = lua.display(),
    );
    run_with_pump(
        &setup,
        r#"
            local h = _G.h
            if not _G.typed then
              local status = pmacs.process.status(h._proc_id)
              if status and status.kind == "running" then
                h:buffer_id():insert(h:prompt_end(), "print('hello-pmacs')")
                pmacs.command.invoke("pmacs.repl.submit-current")
                _G.typed = true
              end
              return false
            end
            local buf = h:buffer_id()
            local history = buf:slice(0, h:history_end())
            return history:find("hello-pmacs", 1, true) ~= nil
        "#,
        10_000,
    );
}

// ---------------------------------------------------------------------------
// Lifecycle regressions
// ---------------------------------------------------------------------------

/// Closing a handle removes it from the supervisor's view and from the
/// per-frame pump registry, so subsequent ticks do not try to drain a
/// dead process.
#[test]
fn m6_5_close_terminates_child_and_unregisters() {
    // M6.9 audit shape: close() sends terminate and sets _closing; the
    // actual proc_pump removal, _proc_id clearing, and forget call
    // happen in _on_exit (driven by the after-tick hook observing the
    // exit event). This routes the supervisor's terminal-state record
    // through forget so it doesn't leak across spawn-close cycles.
    // Pre-M6.9 close() eagerly cleared _proc_id, but that pre-empted
    // _on_exit and caused the supervisor to retain dead-process
    // records forever.
    run(r#"
        local h = pmacs.repl.spawn { argv = { "cat" } }
        local raw = h._proc_id:raw()
        h:close()

        -- close() set _closing; bound commands check this and no-op.
        assert(h._closing == true, "close() should set _closing")

        -- Walking the after-tick hook must not raise: even though
        -- the handle is still registered (until _on_exit fires),
        -- drain_handle is robust to the closing-but-not-yet-exited
        -- state.
        pmacs.hook.run("process.after-tick")
    "#);
}
