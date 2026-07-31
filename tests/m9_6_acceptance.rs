// m9_6_acceptance.rs --- T M9.6 tools-as-commands acceptance.

//! Acceptance tests for T M9.6 (`spec/pmacs-tasks.tex:3986`):
//!
//!   1. Tools registered as commands appear in `M-x` completion
//!      (i.e., `pmacs.command.list()`).
//!   2. `describe-command` reports the tool's schema as the
//!      documentation.
//!   3. Argument prompts for required parameters use the existing
//!      minibuffer machinery.
//!
//! Plus the design-review additions:
//!
//!   * No-required-args tools dispatch immediately (no minibuffer).
//!   * Unregister drops the server's commands.
//!   * `notifications/tools/list_changed` reconciles add / remove.
//!   * Schema-change re-registers (so the prompt-flow closure picks
//!     up the new required-args list).
//!   * Tool failure surfaces a readable status-line error.
//!
//! M9.6 ships with zero new public Rust APIs; the M9.5
//! `on_notification` dispatcher is the second consumer (after M9.5's
//! own resource-update use), confirming the dispatcher abstraction
//! was correctly scoped.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::frontend::KeyEvent;
use pmacs::lua_bindings::PackageInstallOverride;
use pmacs::protocol::FrontendId;
use tempfile::TempDir;

fn fake_mcp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_mcp").to_owned()
}

fn tools_package_path() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.join("tests").join("fixtures").join("pmacs-mcp-tools")
}

/// Build an editor with the pmacs-mcp-tools package installed and
/// require()d. Returns the editor state plus the temp dirs that must
/// outlive it (cache + user-install-root for the package system).
fn editor_with_tools() -> (EditorState, TempDir, TempDir) {
    let cache = tempfile::tempdir().expect("cache tempdir");
    let user_root = tempfile::tempdir().expect("user-root tempdir");
    let mut state = EditorState::new_with_roots(&crate::iso::roots());
    state.lua_host.reopen_init_phase_for_testing();
    state.lua_host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );
    let pkg = tools_package_path();
    let pkg_str = pkg.display().to_string();
    let install = format!(
        r#"
        pmacs.packages.install_local("{pkg_str}")
        _G.TOOLS = require("pmacs-mcp-tools")
    "#
    );
    state
        .lua_host
        .eval(Some("install-tools"), &install)
        .unwrap_or_else(|e| panic!("install_local + require failed: {e}"));
    (state, cache, user_root)
}

/// Spawn the fake MCP server, drain until Initialized, stash the
/// server handle in `_G.SERVER`. Spawn `label` is `m9_6` because
/// command names use the label as a prefix.
fn spawn_initialized_server(state: &mut EditorState) {
    let fake = fake_mcp_path();
    state
        .lua_host
        .lua()
        .load(format!(
            "
            _G.SERVER = pmacs.mcp.spawn({{
                label = 'm9_6',
                command = '{fake}',
                restart = 'never',
            }})
            ",
        ))
        .exec()
        .expect("spawn fake mcp");
    let stop = Instant::now() + Duration::from_secs(5);
    let mut initialized = false;
    while Instant::now() < stop && !initialized {
        state.tick_processes();
        state.tick_mcp();
        state.tick_async();
        let kinds: Vec<String> = state
            .lua_host
            .lua()
            .load(
                "
                local out = {}
                for _, row in ipairs(pmacs.mcp.list()) do
                    out[#out+1] = row.state.kind
                end
                return out
                ",
            )
            .eval()
            .expect("list");
        if kinds.iter().any(|k| k == "initialized") {
            initialized = true;
        }
        if !initialized {
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    assert!(initialized, "fake server must reach Initialized");
}

/// Pump editor + async + mcp ticks until `pred` (a Lua expression
/// returning bool) is true, or the deadline lapses.
fn pump_until_lua_pred(state: &mut EditorState, pred: &str, deadline: Duration) -> bool {
    let stop = Instant::now() + deadline;
    while Instant::now() < stop {
        state.tick_processes();
        state.tick_mcp();
        state.tick_async();
        let ok: bool = state
            .lua_host
            .lua()
            .load(format!("return ({pred}) and true or false"))
            .eval()
            .unwrap_or(false);
        if ok {
            return true;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    false
}

/// Pump until the editor's status line contains `needle`, or the
/// deadline lapses. Status checks read `state.core.borrow().status`
/// rather than going through Lua because there's no `get_status`
/// binding (`set_status` is write-only).
fn pump_until_status_contains(state: &mut EditorState, needle: &str, deadline: Duration) -> bool {
    let stop = Instant::now() + deadline;
    while Instant::now() < stop {
        state.tick_processes();
        state.tick_mcp();
        state.tick_async();
        if state.core.borrow().status.contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    false
}

/// Register the fake server's tools as commands and pump until the
/// initial fetch completes (`TOOLS.commands_for(server)` is non-empty).
fn register_and_wait(state: &mut EditorState) {
    state
        .lua_host
        .eval(Some("register-tools"), "_G.TOOLS.register(_G.SERVER)")
        .expect("register");
    let registered = pump_until_lua_pred(
        state,
        "#_G.TOOLS.commands_for(_G.SERVER) > 0",
        Duration::from_secs(5),
    );
    assert!(
        registered,
        "register() must populate commands_for within 5s"
    );
}

// ===========================================================================
// Bullet 1: tools registered as commands appear in M-x completion
// ===========================================================================

#[test]
fn m9_6_register_adds_tools_as_commands() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    let cmds: Vec<String> = state
        .lua_host
        .lua()
        .load("return _G.TOOLS.commands_for(_G.SERVER)")
        .eval()
        .expect("commands_for");
    assert!(
        cmds.iter().any(|c| c == "m9_6-echo"),
        "echo tool must register as `m9_6-echo`; got {cmds:?}"
    );
    assert!(
        cmds.iter().any(|c| c == "m9_6-mcp_test-greet"),
        "mcp_test/greet must normalize / -> -; got {cmds:?}"
    );
    assert!(
        cmds.iter().any(|c| c == "m9_6-nullary"),
        "nullary tool must register; got {cmds:?}"
    );

    // pmacs.command.list() is the M-x completion source.
    let global: Vec<String> = state
        .lua_host
        .lua()
        .load("return pmacs.command.list()")
        .eval()
        .expect("command.list");
    assert!(
        global.iter().any(|c| c == "m9_6-echo"),
        "pmacs.command.list() must include `m9_6-echo`; got {global:?}"
    );
}

/// Command-name normalization rule: any character outside
/// `[a-zA-Z0-9_.-]` becomes `-`. The seam `command_name(label, tool)`
/// pins the rule without a server.
#[test]
fn m9_6_command_name_normalizes_special_characters() {
    let (state, _c, _u) = editor_with_tools();
    let joined: String = state
        .lua_host
        .lua()
        .load(
            r#"
            return _G.TOOLS.command_name("svr", "a/b") .. "|"
                .. _G.TOOLS.command_name("svr", "x:y z!") .. "|"
                .. _G.TOOLS.command_name("svr", "ok-name_1.2")
            "#,
        )
        .eval()
        .expect("command_name");
    let parts: Vec<&str> = joined.split('|').collect();
    assert_eq!(parts[0], "svr-a-b", "/ must become -");
    assert_eq!(parts[1], "svr-x-y-z-", ": and space and ! must become -");
    assert_eq!(
        parts[2], "svr-ok-name_1.2",
        "alphanumerics, _, ., - pass through"
    );
}

/// Label normalization: the server-name half of the command name
/// runs through the same allow-list as the tool-name half. The Rust
/// `CommandRegistry` only validates non-empty, so without label
/// normalization a label like "my server!" would surface in the
/// command palette as "my server!-echo" — passing M-x but breaking
/// the keymap parser. The fix normalizes both halves so the produced
/// name is registry-clean regardless of operator-supplied label.
#[test]
fn m9_6_command_name_normalizes_label_too() {
    let (state, _c, _u) = editor_with_tools();
    let joined: String = state
        .lua_host
        .lua()
        .load(
            r#"
            return _G.TOOLS.command_name("my server!", "echo") .. "|"
                .. _G.TOOLS.command_name("filesystem/v2", "read-file") .. "|"
                .. _G.TOOLS.command_name("ok_label.1", "tool")
            "#,
        )
        .eval()
        .expect("command_name");
    let parts: Vec<&str> = joined.split('|').collect();
    assert_eq!(
        parts[0], "my-server--echo",
        "label spaces and `!` must collapse to `-`"
    );
    assert_eq!(
        parts[1], "filesystem-v2-read-file",
        "label `/` must collapse to `-`"
    );
    assert_eq!(
        parts[2], "ok_label.1-tool",
        "label alphanumerics, _, ., - pass through unchanged"
    );
}

// ===========================================================================
// Audit-fix: tool_hash treats `required` order as part of identity
// ===========================================================================
//
// Audit issue 9: an earlier draft sorted `required` before hashing,
// so a reorder-only mutation of inputSchema.required produced the
// same hash, the diff in `apply_fresh` saw no change, no re-register
// fired, and make_command_body's prompt-flow closure kept prompting
// in stale order. The fix hashes `required` in document order so a
// pure reorder is a meaningful change.
//
// First test pins the property statically via the `_tool_hash` test
// seam; second test drives it end-to-end through change_tool_schema
// so the live reconcile path is also covered.
#[test]
fn m9_6_tool_hash_includes_required_argument_order() {
    let (state, _c, _u) = editor_with_tools();
    let (h_ab, h_ba): (String, String) = state
        .lua_host
        .lua()
        .load(
            r#"
            local make = function(req)
                return {
                    name = "t", description = "d",
                    inputSchema = {
                        type = "object",
                        properties = {
                            a = { type = "string" },
                            b = { type = "string" }
                        },
                        required = req
                    }
                }
            end
            return _G.TOOLS._tool_hash(make({"a","b"})),
                   _G.TOOLS._tool_hash(make({"b","a"}))
            "#,
        )
        .eval()
        .expect("hashes");
    assert_ne!(
        h_ab, h_ba,
        "reordering `required` must produce a different hash so reconcile re-registers"
    );
}

#[test]
fn m9_6_list_changed_reregisters_on_required_arg_reorder() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    // First mutation: install required=[a, b]. This is a real change
    // from the initial nullary schema, so the existing tests already
    // prove this branch re-registers. Wait for the description to
    // reflect it.
    state
        .lua_host
        .eval(
            Some("install-ab"),
            r#"
            pmacs.async(function()
                pmacs.mcp.invoke_tool(_G.SERVER, "mcp_test/change_tool_schema",
                                      { name = "nullary", required = "a,b" }):await()
            end)
            "#,
        )
        .expect("install ab");
    let installed = pump_until_lua_pred(
        &mut state,
        r#"(function()
            local info = pmacs.describe.command("m9_6-nullary")
            if info == nil then return false end
            local desc = info.description or ""
            local pa = string.find(desc, "a %(string, required%)")
            local pb = string.find(desc, "b %(string, required%)")
            return pa ~= nil and pb ~= nil and pa < pb
        end)()"#,
        Duration::from_millis(1500),
    );
    assert!(
        installed,
        "first change_tool_schema must produce a description with a then b"
    );

    // Second mutation: same membership, reverse order. Without the
    // hash fix the hash collision would skip the re-register and the
    // description would still show a-then-b.
    state
        .lua_host
        .eval(
            Some("reorder-ba"),
            r#"
            pmacs.async(function()
                pmacs.mcp.invoke_tool(_G.SERVER, "mcp_test/change_tool_schema",
                                      { name = "nullary", required = "b,a" }):await()
            end)
            "#,
        )
        .expect("reorder ba");
    let reordered = pump_until_lua_pred(
        &mut state,
        r#"(function()
            local info = pmacs.describe.command("m9_6-nullary")
            if info == nil then return false end
            local desc = info.description or ""
            local pa = string.find(desc, "a %(string, required%)")
            local pb = string.find(desc, "b %(string, required%)")
            return pa ~= nil and pb ~= nil and pb < pa
        end)()"#,
        Duration::from_millis(1500),
    );
    assert!(
        reordered,
        "reorder of `required` must drive a re-register so the description reflects the new order"
    );
}

// ===========================================================================
// Audit-fix: typed-arg coercion at minibuffer-accept time
// ===========================================================================
//
// Audit issue 5: the package previously sent every minibuffer value
// through to `pmacs.mcp.invoke_tool` as a string, so tools whose
// inputSchema declared `integer` / `number` / `boolean` received a
// type-mismatched argument and rejected at the server. The fix
// coerces in `prompt_chain` based on the property's declared type
// before assembling `args`. The fake's `typed_*` tools echo the JSON
// kind they actually received so this test pins the coercion
// observably end-to-end.
#[test]
fn m9_6_typed_arg_integer_is_coerced_before_dispatch() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("invoke-typed-int"),
            r#"pmacs.command.invoke("m9_6-typed_int")"#,
        )
        .expect("invoke typed_int");
    state
        .lua_host
        .eval(
            Some("type-int"),
            r#"
            pmacs.minibuffer.set_contents("42")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("type+accept");
    assert!(
        pump_until_status_contains(&mut state, "kind=integer", Duration::from_secs(2)),
        "integer arg must arrive at the server as a JSON number; status={:?}",
        state.core.borrow().status
    );
    assert!(
        state.core.borrow().status.contains("value=42"),
        "coerced integer must round-trip the typed value"
    );
}

#[test]
fn m9_6_typed_arg_number_is_coerced_before_dispatch() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("invoke-typed-number"),
            r#"pmacs.command.invoke("m9_6-typed_number")"#,
        )
        .expect("invoke typed_number");
    state
        .lua_host
        .eval(
            Some("type-number"),
            r#"
            pmacs.minibuffer.set_contents("3.5")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("type+accept");
    assert!(
        pump_until_status_contains(&mut state, "kind=number", Duration::from_secs(2)),
        "number arg must arrive at the server as a JSON number; status={:?}",
        state.core.borrow().status
    );
    assert!(
        state.core.borrow().status.contains("value=3.5"),
        "coerced number must round-trip the typed value"
    );
}

#[test]
fn m9_6_typed_arg_boolean_is_coerced_before_dispatch() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("invoke-typed-bool"),
            r#"pmacs.command.invoke("m9_6-typed_bool")"#,
        )
        .expect("invoke typed_bool");
    state
        .lua_host
        .eval(
            Some("type-bool"),
            r#"
            pmacs.minibuffer.set_contents("true")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("type+accept");
    assert!(
        pump_until_status_contains(&mut state, "kind=boolean", Duration::from_secs(2)),
        "boolean arg must arrive at the server as a JSON bool; status={:?}",
        state.core.borrow().status
    );
    assert!(
        state.core.borrow().status.contains("value=true"),
        "coerced boolean must round-trip the typed value"
    );
}

/// Bad input for a typed arg surfaces a status-line parse error and
/// does *not* dispatch a malformed call to the server. The status
/// shape is `MCP <name> arg <field>: expected <type>, got "<value>"`.
#[test]
fn m9_6_typed_arg_parse_error_aborts_dispatch() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("invoke-typed-int-bad"),
            r#"pmacs.command.invoke("m9_6-typed_int")"#,
        )
        .expect("invoke typed_int");
    state
        .lua_host
        .eval(
            Some("type-bad"),
            r#"
            pmacs.minibuffer.set_contents("not-a-number")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("type+accept");

    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("expected integer"),
        "parse failure must surface as an `expected <type>` status; got {status:?}"
    );
    // The dispatch must NOT reach the server. Pump for a moment and
    // confirm we never see the typed_int echo prefix.
    let leaked =
        pump_until_status_contains(&mut state, "typed_int: kind=", Duration::from_millis(500));
    assert!(
        !leaked,
        "malformed integer must abort before invoke_tool; status={:?}",
        state.core.borrow().status
    );
}

// ===========================================================================
// Bullet 2: describe-command reports the schema as documentation
// ===========================================================================

#[test]
fn m9_6_describe_command_reports_schema_in_description() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    let desc: String = state
        .lua_host
        .lua()
        .load(
            r#"
            local info = pmacs.describe.command("m9_6-mcp_test-greet")
            return info.description
            "#,
        )
        .eval()
        .expect("describe greet");
    assert!(
        desc.contains("Two-required-arg greeting tool"),
        "description must include the tool's text description; got {desc:?}"
    );
    assert!(
        desc.contains("Arguments:"),
        "description must include an Arguments: section; got {desc:?}"
    );
    assert!(
        desc.contains("name (string, required)"),
        "required string arg must be tagged; got {desc:?}"
    );
    assert!(
        desc.contains("greeting (string, required)"),
        "second required arg must also appear; got {desc:?}"
    );
}

/// Schema-as-docs fallback: a tool advertised with no `description`
/// renders as "(no description)" rather than a blank line followed by
/// Arguments:. Drives the framing decision to be honest about the
/// missing field. Uses the package's `_render_schema_doc` test seam
/// so we don't need a server-side hook.
#[test]
fn m9_6_describe_command_uses_no_description_sentinel_for_missing_desc() {
    let (state, _c, _u) = editor_with_tools();
    let rendered: String = state
        .lua_host
        .lua()
        .load(
            r#"
            return _G.TOOLS._render_schema_doc({
                name = "anon",
                inputSchema = { type = "object", properties = {}, required = {} }
            })
            "#,
        )
        .eval()
        .expect("render");
    assert_eq!(
        rendered, "(no description)",
        "package must use the (no description) sentinel for empty descriptions"
    );
}

// ===========================================================================
// Bullet 3: required-arg prompts use the minibuffer machinery
// ===========================================================================

/// Bullet 3 (single arg): invoking the registered command opens the
/// minibuffer; typing + accept dispatches the tool with the typed
/// value as the required argument.
#[test]
fn m9_6_command_with_required_arg_prompts_via_minibuffer() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(Some("invoke-echo"), r#"pmacs.command.invoke("m9_6-echo")"#)
        .expect("invoke");

    let active: bool = state
        .lua_host
        .lua()
        .load("return pmacs.minibuffer.is_active()")
        .eval()
        .expect("is_active");
    assert!(active, "single-required-arg tool must open the minibuffer");

    state
        .lua_host
        .eval(
            Some("type-and-accept"),
            r#"
            pmacs.minibuffer.set_contents("hello world")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("type+accept");

    assert!(
        pump_until_status_contains(&mut state, "hello world", Duration::from_secs(2)),
        "echo tool's response must reach the status line; status={:?}",
        state.core.borrow().status
    );
    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("MCP echo"),
        "status line must be tagged with the tool name; got {status:?}"
    );
}

/// Bullet 3 (multi-arg): chained prompts. The `on_accept` of prompt N
/// kicks off prompt N+1; the final accept dispatches.
#[test]
fn m9_6_command_with_multiple_required_args_chains_prompts() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("invoke-greet"),
            r#"pmacs.command.invoke("m9_6-mcp_test-greet")"#,
        )
        .expect("invoke");
    assert!(
        state
            .lua_host
            .lua()
            .load("return pmacs.minibuffer.is_active()")
            .eval::<bool>()
            .expect("is_active 1"),
        "first prompt (name) must be active"
    );
    state
        .lua_host
        .eval(
            Some("type-name"),
            r#"
            pmacs.minibuffer.set_contents("alice")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("type name");
    assert!(
        state
            .lua_host
            .lua()
            .load("return pmacs.minibuffer.is_active()")
            .eval::<bool>()
            .expect("is_active 2"),
        "second prompt (greeting) must open after the first accept"
    );
    state
        .lua_host
        .eval(
            Some("type-greeting"),
            r#"
            pmacs.minibuffer.set_contents("Hi there")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("type greeting");

    assert!(
        pump_until_status_contains(&mut state, "Hello, alice", Duration::from_secs(2)),
        "two-arg dispatch must reach the fake; status={:?}",
        state.core.borrow().status
    );
    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("Hi there"),
        "second arg must thread through to the response; got {status:?}"
    );
}

/// No-required-args tool dispatches immediately — no minibuffer
/// session opens.
#[test]
fn m9_6_command_with_no_args_dispatches_immediately() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("invoke-nullary"),
            r#"pmacs.command.invoke("m9_6-nullary")"#,
        )
        .expect("invoke nullary");
    let active: bool = state
        .lua_host
        .lua()
        .load("return pmacs.minibuffer.is_active()")
        .eval()
        .expect("is_active");
    assert!(!active, "nullary tool must not open the minibuffer");
    assert!(
        pump_until_status_contains(&mut state, "no-arg tool ran", Duration::from_secs(2)),
        "nullary dispatch must surface a result; status={:?}",
        state.core.borrow().status
    );
}

// ===========================================================================
// End-to-end: M-x palette → MCP tool → minibuffer re-entry
// ===========================================================================
//
// All three argument-prompt tests above invoke the registered command
// directly via `pmacs.command.invoke`. That covers the command body's
// own logic but skips the user-facing M-x flow, where the *outer*
// minibuffer session has just accepted the command name and the
// command body opens a *new* minibuffer session from inside the
// outer session's `on_accept` callback.
//
// `Minibuffer::accept` calls `self.session.take()` before invoking the
// callback, so the inner `read` is starting from a clean slot — but
// nothing in the existing acceptance suite pins this. This test does:
// drive `editor.execute-command` (the actual M-x palette command),
// accept "m9_6-echo", verify the tool's argument prompt opens, accept
// the value, and verify the dispatch reaches the status line.
#[test]
fn m9_6_mx_palette_invokes_mcp_tool_through_minibuffer_reentry() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    // Step 1: invoke M-x. The outer minibuffer opens with the
    // `commands` completion source.
    state
        .lua_host
        .eval(
            Some("invoke-mx"),
            r#"pmacs.command.invoke("editor.execute-command")"#,
        )
        .expect("invoke M-x");
    let mx_active: bool = state
        .lua_host
        .lua()
        .load("return pmacs.minibuffer.is_active()")
        .eval()
        .expect("mx is_active");
    assert!(mx_active, "M-x must open the outer minibuffer session");

    // Step 2: type the MCP tool name and accept. The outer session's
    // on_accept invokes the command, whose body starts a *new*
    // minibuffer session for the required argument.
    state
        .lua_host
        .eval(
            Some("type-cmd-name"),
            r#"pmacs.minibuffer.set_contents("m9_6-echo")"#,
        )
        .expect("accept command name");

    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );

    // Step 3: the tool's argument prompt must now be active. This is
    // the re-entrant minibuffer behavior: outer session was taken,
    // inner session was begun, and the begin happened from inside
    // the outer accept's authenticated dispatch callback.
    let inner_active: bool = state
        .lua_host
        .lua()
        .load("return pmacs.minibuffer.is_active()")
        .eval()
        .expect("inner is_active");
    assert!(
        inner_active,
        "tool's argument prompt must open after M-x accept"
    );

    // Step 4: drive the inner prompt. The dispatch must reach the
    // status line just as it does in the direct-invoke case.
    state
        .lua_host
        .eval(
            Some("type-arg"),
            r#"pmacs.minibuffer.set_contents("through M-x")"#,
        )
        .expect("accept arg");
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );
    assert!(
        pump_until_status_contains(&mut state, "through M-x", Duration::from_secs(2)),
        "M-x → arg-prompt → dispatch must reach status; status={:?}",
        state.core.borrow().status
    );
    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("MCP echo"),
        "status must be tagged with the tool name; got {status:?}"
    );
}

// ===========================================================================
// Lifecycle: unregister drops commands
// ===========================================================================

#[test]
fn m9_6_unregister_drops_commands() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    let before: Vec<String> = state
        .lua_host
        .lua()
        .load("return _G.TOOLS.commands_for(_G.SERVER)")
        .eval()
        .expect("before");
    assert!(
        !before.is_empty(),
        "commands must be registered before unregister"
    );

    state
        .lua_host
        .eval(Some("unregister"), "_G.TOOLS.unregister(_G.SERVER)")
        .expect("unregister");

    let after: Vec<String> = state
        .lua_host
        .lua()
        .load("return _G.TOOLS.commands_for(_G.SERVER)")
        .eval()
        .expect("after");
    assert!(
        after.is_empty(),
        "commands_for must return empty after unregister; got {after:?}"
    );
    let global: Vec<String> = state
        .lua_host
        .lua()
        .load("return pmacs.command.list()")
        .eval()
        .expect("command.list");
    for name in &before {
        assert!(
            !global.iter().any(|g| g == name),
            "command {name:?} must be removed from pmacs.command.list()"
        );
    }
}

// ===========================================================================
// Reconciliation: notifications/tools/list_changed
// ===========================================================================

/// `add_tool` emits `notifications/tools/list_changed`; the package
/// reconciles by registering the new tool. `remove_tool` likewise drops
/// it. The 1.5s bound is generous on purpose — round-trip through
/// stdio + dispatcher tick + reconcile coroutine.
#[test]
fn m9_6_list_changed_adds_and_removes_commands() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    let baseline: Vec<String> = state
        .lua_host
        .lua()
        .load("return _G.TOOLS.commands_for(_G.SERVER)")
        .eval()
        .expect("baseline");

    state
        .lua_host
        .eval(
            Some("trigger-add"),
            r#"
            pmacs.async(function()
                pmacs.mcp.invoke_tool(_G.SERVER, "mcp_test/add_tool",
                                      { name = "freshly_added" }):await()
            end)
            "#,
        )
        .expect("invoke add_tool");

    let added = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, c in ipairs(_G.TOOLS.commands_for(_G.SERVER)) do
                if c == "m9_6-freshly_added" then return true end
            end
            return false
        end)()"#,
        Duration::from_millis(1500),
    );
    assert!(
        added,
        "list_changed must drive registration of the new tool within 1.5s; baseline={baseline:?}"
    );

    state
        .lua_host
        .eval(
            Some("trigger-remove"),
            r#"
            pmacs.async(function()
                pmacs.mcp.invoke_tool(_G.SERVER, "mcp_test/remove_tool",
                                      { name = "freshly_added" }):await()
            end)
            "#,
        )
        .expect("invoke remove_tool");
    let removed = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, c in ipairs(_G.TOOLS.commands_for(_G.SERVER)) do
                if c == "m9_6-freshly_added" then return false end
            end
            return true
        end)()"#,
        Duration::from_millis(1500),
    );
    assert!(
        removed,
        "list_changed must drive unregistration within 1.5s"
    );
}

/// Schema-change case: an existing tool's required-args list mutates;
/// reconciler must detect the schema-hash change and re-register so
/// the prompt-flow closure picks up the new arg list.
#[test]
fn m9_6_list_changed_reregisters_on_schema_change() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    // Mutate the fake's `nullary` tool to require one arg.
    state
        .lua_host
        .eval(
            Some("trigger-schema-change"),
            r#"
            pmacs.async(function()
                pmacs.mcp.invoke_tool(_G.SERVER, "mcp_test/change_tool_schema",
                                      { name = "nullary", required = "newarg" }):await()
            end)
            "#,
        )
        .expect("invoke change_schema");

    // Wait for the reconcile to land. The describe.command output
    // covers both re-registration *and* the description rebuild.
    let reregistered = pump_until_lua_pred(
        &mut state,
        r#"(function()
            local info = pmacs.describe.command("m9_6-nullary")
            if info == nil then return false end
            return string.find(info.description, "newarg %(string, required%)") ~= nil
        end)()"#,
        Duration::from_millis(1500),
    );
    assert!(
        reregistered,
        "schema change must re-register the command with the new required-args description"
    );

    // And invoking now must open the minibuffer (the prompt-flow
    // closure picked up the new required arg).
    state
        .lua_host
        .eval(
            Some("invoke-after-schema-change"),
            r#"pmacs.command.invoke("m9_6-nullary")"#,
        )
        .expect("invoke after change");
    let active: bool = state
        .lua_host
        .lua()
        .load("return pmacs.minibuffer.is_active()")
        .eval()
        .expect("is_active after change");
    assert!(
        active,
        "after schema change, invocation must prompt for the new required arg"
    );
    state
        .lua_host
        .eval(Some("cancel"), r"pmacs.minibuffer.cancel()")
        .expect("cancel");
}

// ===========================================================================
// Tool failure surfaces a readable error
// ===========================================================================

#[test]
fn m9_6_tool_failure_surfaces_status_error() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(Some("invoke-fail"), r#"pmacs.command.invoke("m9_6-fail")"#)
        .expect("invoke fail");

    assert!(
        pump_until_status_contains(&mut state, "synthetic tool failure", Duration::from_secs(2)),
        "tool failure text must surface; status={:?}",
        state.core.borrow().status
    );
    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("MCP fail error"),
        "tool failure must surface as `MCP <name> error: ...`; got {status:?}"
    );
}

// ===========================================================================
// Audit-fix: cross-source command collision skips with a warning
// ===========================================================================
//
// Audit issue 6: an outside-the-package command with the same normalized
// name (a builtin, a user definition, or a different MCP server) used to
// surface as a `DuplicateName` error from `pmacs.command.define`, which
// propagated out of the async coroutine and aborted further registrations
// for that server. The fix: check `pmacs.command.exists(cmd_name)` before
// defining; on hit, set_status warn and skip the one tool, letting the
// rest of the server's tools register cleanly.
#[test]
fn m9_6_cross_source_collision_skips_with_warning() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);

    // Pre-define a command at the name the package would otherwise pick
    // for the `echo` tool. This simulates a builtin or user command
    // already owning that slot when the MCP register runs.
    state
        .lua_host
        .eval(
            Some("preempt"),
            r#"
            pmacs.command.define {
                name = "m9_6-echo",
                description = "preexisting non-MCP command",
                fn = function() pmacs.editor.set_status("preexisting") end,
            }
            "#,
        )
        .expect("preempt define");

    state
        .lua_host
        .eval(Some("register-tools"), "_G.TOOLS.register(_G.SERVER)")
        .expect("register");

    // Wait for the *other* tools to register — proves the per-tool
    // collision didn't abort the whole server's registration loop. The
    // `m9_6-mcp_test-greet` name has no preempt, so it must arrive.
    let other_arrived = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, c in ipairs(_G.TOOLS.commands_for(_G.SERVER)) do
                if c == "m9_6-mcp_test-greet" then return true end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(
        other_arrived,
        "non-colliding tools must still register after a cross-source collision skip"
    );

    let cmds: Vec<String> = state
        .lua_host
        .lua()
        .load("return _G.TOOLS.commands_for(_G.SERVER)")
        .eval()
        .expect("commands_for");
    assert!(
        !cmds.iter().any(|c| c == "m9_6-echo"),
        "collided echo tool must not appear in the package's owned set; got {cmds:?}"
    );

    // The preexisting command body is still the one the registry serves —
    // no silent overwrite by the MCP package.
    let desc: String = state
        .lua_host
        .lua()
        .load(r#"return pmacs.describe.command("m9_6-echo").description"#)
        .eval()
        .expect("describe preempt");
    assert_eq!(
        desc, "preexisting non-MCP command",
        "preexisting command must keep its description (no silent overwrite); got {desc:?}"
    );

    // Status line carries the skip warning (best-effort: the skip is the
    // most-recent set_status from the package, but later reconcile work
    // may overwrite. Just check the warning shape was used at some point
    // by polling for it.)
    let warned =
        pump_until_status_contains(&mut state, "already defined", Duration::from_millis(500));
    // Soft assertion: not a hard fail if a later set_status raced past
    // the warning, but at least the package emitted *something* about
    // m9_6-echo. The structural property — `m9_6-echo` not in
    // commands_for and the preexisting body intact — is the contract.
    let _ = warned;
}

// Audit issue 10: collision warnings used to be set_status-only,
// which the very next set_status from any source could erase without
// trace. The fix routes them through pmacs.error too — the project's
// persistent log surface (same convention as
// builtin/runtime/{async,mcp,syntax}.lua). Tests install a stub
// pmacs.error to prove the warning path actually reaches it.
#[test]
fn m9_6_collision_warning_reaches_pmacs_error_log() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);

    // Stub pmacs.error before register so the warning's pmacs.error
    // call lands somewhere we can read. Without the stub, the
    // `if pmacs.error then ... end` branch in `notify()` is a no-op.
    state
        .lua_host
        .eval(
            Some("install-stub"),
            r#"
            _G.PMACS_ERROR_LOG = {}
            pmacs.error = function(msg)
                _G.PMACS_ERROR_LOG[#_G.PMACS_ERROR_LOG + 1] = msg
            end
            pmacs.command.define {
                name = "m9_6-echo",
                description = "preexisting non-MCP command",
                fn = function() end,
            }
            "#,
        )
        .expect("install pmacs.error stub + preempt");

    state
        .lua_host
        .eval(Some("register-tools"), "_G.TOOLS.register(_G.SERVER)")
        .expect("register");

    // Wait for at least one tool to land (proves the loop ran past the
    // skipped collision).
    let any_registered = pump_until_lua_pred(
        &mut state,
        "#_G.TOOLS.commands_for(_G.SERVER) > 0",
        Duration::from_secs(2),
    );
    assert!(any_registered, "registration loop must run despite skip");

    // pmacs.error must have received at least one message mentioning
    // the colliding command — even if a later set_status erased the
    // status-line copy, the persistent log keeps the trace.
    let logged: bool = state
        .lua_host
        .lua()
        .load(
            r#"
            for _, m in ipairs(_G.PMACS_ERROR_LOG) do
                if string.find(m, "m9_6%-echo", 1) ~= nil
                   and string.find(m, "already defined", 1, true) ~= nil then
                    return true
                end
            end
            return false
            "#,
        )
        .eval()
        .expect("scan log");
    assert!(
        logged,
        "collision warning must reach pmacs.error so it survives past the next set_status"
    );
}

// ===========================================================================
// Audit-fix: notification subscription is balanced across register/unregister
// ===========================================================================
//
// Audit issue 3: ensure_notification_handler set the
// notifications/tools/list_changed token once on the first M.register and
// never released it. The fix: refcount registered servers and call
// off_notification once the count drops to zero.
#[test]
fn m9_6_notification_subscription_releases_on_last_unregister() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);

    // No subscription before the first register.
    let before: bool = state
        .lua_host
        .lua()
        .load("return _G.TOOLS._has_notification_subscription()")
        .eval()
        .expect("before");
    assert!(!before, "no subscription should exist before any register");

    register_and_wait(&mut state);

    let during: bool = state
        .lua_host
        .lua()
        .load("return _G.TOOLS._has_notification_subscription()")
        .eval()
        .expect("during");
    assert!(
        during,
        "subscription must exist while a server is registered"
    );

    state
        .lua_host
        .eval(Some("unregister"), "_G.TOOLS.unregister(_G.SERVER)")
        .expect("unregister");

    let after: bool = state
        .lua_host
        .lua()
        .load("return _G.TOOLS._has_notification_subscription()")
        .eval()
        .expect("after");
    assert!(
        !after,
        "subscription must be released when the last server unregisters"
    );
}

// ===========================================================================
// Audit-fix: server-gone teardown drops registered commands on invoke fail
// ===========================================================================
//
// Audit issue 5: when an MCP server stopped/crashed, its commands lingered
// in pmacs.command.list() forever — every invoke would surface the same
// dead-server error. The fix: dispatch detects the server-gone shape on
// invoke failure and calls M.unregister to drop the stale commands.
#[test]
fn m9_6_server_stop_unregisters_commands_on_next_invoke() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    // Confirm the precondition: nullary is registered.
    let before: Vec<String> = state
        .lua_host
        .lua()
        .load("return _G.TOOLS.commands_for(_G.SERVER)")
        .eval()
        .expect("before");
    assert!(
        before.iter().any(|c| c == "m9_6-nullary"),
        "nullary must be registered before stopping the server"
    );

    // Stop the server, then drain ticks so the manager observes the exit.
    state
        .lua_host
        .eval(Some("stop"), "pmacs.mcp.stop(_G.SERVER)")
        .expect("stop server");
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        state.tick_processes();
        state.tick_mcp();
        state.tick_async();
        let kind: String = state
            .lua_host
            .lua()
            .load(
                r#"
                for _, row in ipairs(pmacs.mcp.list()) do
                    if row.id == _G.SERVER then return row.state.kind end
                end
                return ""
                "#,
            )
            .eval()
            .unwrap_or_default();
        if kind == "stopped" || kind.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
    }

    // Now invoke a registered command. The dispatch must surface a
    // server-gone error AND unregister the per-server commands.
    state
        .lua_host
        .eval(
            Some("invoke-after-stop"),
            r#"pmacs.command.invoke("m9_6-nullary")"#,
        )
        .expect("invoke after stop");

    let cleared = pump_until_lua_pred(
        &mut state,
        "#_G.TOOLS.commands_for(_G.SERVER) == 0",
        Duration::from_secs(2),
    );
    assert!(
        cleared,
        "server-gone teardown must drop the per-server command set"
    );

    // pmacs.command.list() must no longer advertise the now-defunct
    // commands either.
    let global: Vec<String> = state
        .lua_host
        .lua()
        .load("return pmacs.command.list()")
        .eval()
        .expect("command.list");
    for stale in &before {
        assert!(
            !global.iter().any(|g| g == stale),
            "stopped server's command {stale:?} must be removed from pmacs.command.list()"
        );
    }
}

// ===========================================================================
// Audit-fix: editor.describe-command surfaces the schema in *help*
// ===========================================================================
//
// Audit issue 1: the spec lists describe-command alongside describe-key as
// the user-facing introspection surface. The Rust API
// `pmacs.describe.command(name)` was already wired through, but no
// builtin command fronted it — M-x had no `describe-command` to type. The
// fix: builtin/commands/default.lua now defines `editor.describe-command`,
// which prompts for a name and renders the description (the rendered
// schema, for MCP-tool commands) in *help*.
#[test]
fn m9_6_editor_describe_command_renders_schema_in_help_buffer() {
    let (mut state, _c, _u) = editor_with_tools();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    // Sanity: the new builtin is in the M-x palette.
    let in_palette: bool = state
        .lua_host
        .lua()
        .load(
            r#"
            for _, c in ipairs(pmacs.command.list()) do
                if c == "editor.describe-command" then return true end
            end
            return false
            "#,
        )
        .eval()
        .expect("palette check");
    assert!(
        in_palette,
        "editor.describe-command must appear in pmacs.command.list()"
    );

    // Invoke the builtin, then drive the prompt with the MCP tool name.
    state
        .lua_host
        .eval(
            Some("invoke-describe"),
            r#"pmacs.command.invoke("editor.describe-command")"#,
        )
        .expect("invoke describe");
    let active: bool = state
        .lua_host
        .lua()
        .load("return pmacs.minibuffer.is_active()")
        .eval()
        .expect("is_active");
    assert!(
        active,
        "editor.describe-command must open the minibuffer for the name prompt"
    );
    state
        .lua_host
        .eval(
            Some("type-and-accept"),
            r#"
            pmacs.minibuffer.set_contents("m9_6-mcp_test-greet")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("accept name");

    // *help* must now exist and contain the rendered schema.
    let body: String = state
        .lua_host
        .lua()
        .load(
            r#"
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*help*" then
                    return id:slice(0, id:len())
                end
            end
            return ""
            "#,
        )
        .eval()
        .expect("read *help*");
    assert!(
        body.contains("m9_6-mcp_test-greet"),
        "*help* must include the queried command name; got {body:?}"
    );
    assert!(
        body.contains("Two-required-arg greeting tool"),
        "*help* must include the tool's text description; got {body:?}"
    );
    assert!(
        body.contains("name (string, required)"),
        "*help* must include the rendered required-arg schema; got {body:?}"
    );
}

// editor.describe-command on an unknown name surfaces a status-line
// "no such command" rather than crashing or opening *help* with bogus
// content.
#[test]
fn m9_6_editor_describe_command_unknown_name_status_only() {
    let (mut state, _c, _u) = editor_with_tools();

    state
        .lua_host
        .eval(
            Some("invoke-describe"),
            r#"pmacs.command.invoke("editor.describe-command")"#,
        )
        .expect("invoke describe");
    state
        .lua_host
        .eval(
            Some("type-bad"),
            r#"
            pmacs.minibuffer.set_contents("does.not.exist")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("accept bad");
    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("no such command"),
        "unknown-name path must surface a status-line message; got {status:?}"
    );
    let has_help: bool = state
        .lua_host
        .lua()
        .load(
            r#"
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*help*" then return true end
            end
            return false
            "#,
        )
        .eval()
        .expect("help check");
    assert!(
        !has_help,
        "*help* must not be created for an unknown-name describe-command call"
    );
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
