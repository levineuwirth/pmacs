// m9_8_acceptance.rs --- T M9.8 AI-assistance package acceptance.

//! Acceptance tests for T M9.8 (`spec/pmacs-tasks.tex:4022`):
//!
//!   1. Single-buffer context: command on a function selects the
//!      function as context, sends a prompt, displays response.
//!   2. Multi-buffer context: command on a project selects relevant
//!      files, sends a combined prompt, displays response.
//!   3. Configurable MCP server: changing the configured server
//!      changes the model without code changes.
//!
//! Plus the architectural-claim tests:
//!
//!   * Server pluggability: configure to server A, invoke; re-configure
//!     to server B, invoke *same command*, verify the response now
//!     comes from B. No code changes between the two invocations.
//!   * Composition with M9.7: ai.* commands land their results in the
//!     M9.7 `*mcp:<label>:<prompt>*` buffer namespace via the
//!     `pmacs-mcp-prompts.render` public function (promoted from
//!     internal on M9.8's request as the second consumer).
//!
//! M9.8 ships zero new public Rust APIs and zero new public
//! `pmacs.mcp.*` Lua functions. The package's only AI-domain
//! dependency is the configured MCP server it talks to; no model-
//! specific code. The M9.5 dispatcher → M9.6 → M9.7 → M9.8
//! composition chain validates "AI is a transport binding, not a
//! feature."

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pmacs::editor::EditorState;
use pmacs::lua_bindings::PackageInstallOverride;
use tempfile::TempDir;

fn fake_mcp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_mcp").to_owned()
}

fn ai_package_path() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.join("tests").join("fixtures").join("pmacs-mcp-ai")
}

fn prompts_package_path() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.join("tests")
        .join("fixtures")
        .join("pmacs-mcp-prompts")
}

/// Build an editor with the pmacs-mcp-prompts AND pmacs-mcp-ai
/// packages installed and require()d (the AI package depends on the
/// prompts package via require). Returns the editor state plus the
/// temp dirs that must outlive it.
fn editor_with_ai() -> (EditorState, TempDir, TempDir) {
    let cache = tempfile::tempdir().expect("cache tempdir");
    let user_root = tempfile::tempdir().expect("user-root tempdir");
    let mut state = EditorState::new();
    state.lua_host.reopen_init_phase_for_testing();
    state.lua_host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );
    let prompts_pkg = prompts_package_path().display().to_string();
    let ai_pkg = ai_package_path().display().to_string();
    let install = format!(
        r#"
        pmacs.packages.install_local("{prompts_pkg}")
        pmacs.packages.install_local("{ai_pkg}")
        _G.PROMPTS = require("pmacs-mcp-prompts")
        _G.AI      = require("pmacs-mcp-ai")
    "#
    );
    state
        .lua_host
        .eval(Some("install-ai"), &install)
        .unwrap_or_else(|e| panic!("install_local + require failed: {e}"));
    (state, cache, user_root)
}

fn spawn_initialized_server_with_label(state: &mut EditorState, label: &str, slot: &str) {
    let fake = fake_mcp_path();
    state
        .lua_host
        .lua()
        .load(format!(
            "
            _G.{slot} = pmacs.mcp.spawn({{
                label = '{label}',
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
            .load(format!(
                "
                local out = {{}}
                for _, row in ipairs(pmacs.mcp.list()) do
                    if row.label == '{label}' then out[#out+1] = row.state.kind end
                end
                return out
                ",
            ))
            .eval()
            .expect("list");
        if kinds.iter().any(|k| k == "initialized") {
            initialized = true;
        }
        if !initialized {
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    assert!(
        initialized,
        "fake server with label {label} must reach Initialized"
    );
}

fn spawn_initialized_server(state: &mut EditorState) {
    spawn_initialized_server_with_label(state, "m9_8", "SERVER");
}

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

fn buffer_body(state: &mut EditorState, name: &str) -> Option<String> {
    let body: String = state
        .lua_host
        .lua()
        .load(format!(
            r#"
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "{name}" then
                    return id:slice(0, id:len())
                end
            end
            return ""
            "#
        ))
        .eval()
        .ok()?;
    if body.is_empty() { None } else { Some(body) }
}

fn configure_default(state: &mut EditorState, server_label: &str) {
    let cmd = format!(
        r#"
        _G.AI.configure {{
            server_label = "{server_label}",
            prompts = {{
                fn      = "review_function",
                project = "review_project",
                ask     = "ask_freeform",
            }},
        }}
        "#
    );
    state
        .lua_host
        .eval(Some("configure"), &cmd)
        .expect("configure");
}

// ===========================================================================
// Configure / re-configure / unconfigure lifecycle
// ===========================================================================

#[test]
fn m9_8_configure_defines_three_commands() {
    let (mut state, _c, _u) = editor_with_ai();
    spawn_initialized_server(&mut state);
    configure_default(&mut state, "m9_8");

    let cmds: Vec<String> = state
        .lua_host
        .lua()
        .load("return pmacs.command.list()")
        .eval()
        .expect("command.list");
    for expected in ["ai.ask-about-function", "ai.ask-about-project", "ai.ask"] {
        assert!(
            cmds.iter().any(|c| c == expected),
            "{expected} must be defined after configure; got {cmds:?}"
        );
    }
}

#[test]
fn m9_8_unconfigure_drops_commands() {
    let (mut state, _c, _u) = editor_with_ai();
    spawn_initialized_server(&mut state);
    configure_default(&mut state, "m9_8");
    state
        .lua_host
        .eval(Some("unconfigure"), "_G.AI.unconfigure()")
        .expect("unconfigure");

    let cmds: Vec<String> = state
        .lua_host
        .lua()
        .load("return pmacs.command.list()")
        .eval()
        .expect("command.list");
    for unwanted in ["ai.ask-about-function", "ai.ask-about-project", "ai.ask"] {
        assert!(
            !cmds.iter().any(|c| c == unwanted),
            "{unwanted} must be removed by unconfigure; got {cmds:?}"
        );
    }
    let cfg_nil: bool = state
        .lua_host
        .lua()
        .load("return _G.AI._config() == nil")
        .eval()
        .expect("config nil");
    assert!(cfg_nil, "unconfigure must clear `_config`");
}

// `configure` with an empty `prompts = {}` table leaves the
// per-flow prompt slots unset. Each ai.* command must surface a
// helpful status naming the missing slot rather than failing
// opaquely.
#[test]
fn m9_8_invoke_with_missing_prompt_slot_surfaces_helpful_status() {
    let (mut state, _c, _u) = editor_with_ai();
    spawn_initialized_server(&mut state);
    state
        .lua_host
        .eval(
            Some("partial-configure"),
            r#"_G.AI.configure { server_label = "m9_8", prompts = {} }"#,
        )
        .expect("partial configure");
    state
        .lua_host
        .eval(Some("invoke"), r#"pmacs.command.invoke("ai.ask")"#)
        .expect("invoke");
    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("no `prompts.ask`"),
        "missing-prompt path must surface a helpful status; got {status:?}"
    );
}

// ===========================================================================
// Bullet 1: single-buffer context (function selection via tree-sitter)
// ===========================================================================

#[test]
fn m9_8_ask_about_function_sends_function_context() {
    let (mut state, _c, _u) = editor_with_ai();
    spawn_initialized_server(&mut state);
    configure_default(&mut state, "m9_8");

    // Create a Rust buffer with two functions; place cursor inside
    // the second one. After parse, the enclosing-function walk must
    // pick the second function's source as the context.
    state
        .lua_host
        .eval(
            Some("setup-buffer"),
            r#"
            local source = "fn alpha() {\n    let x: i32 = 1;\n}\n\nfn beta() {\n    let y: i32 = 2;\n}\n"
            local buf = pmacs.buffer.from_bytes("demo.rs", source)
            pmacs.window.switch_buffer(buf)
            pmacs.parse._parse_now(buf, "rust")
            -- Place cursor inside `fn beta()`'s body. The "let y" line
            -- starts at byte ~50; we just walk forward to land in the
            -- function. Buffer-start is byte 0; move to a known offset
            -- via the cursor seam isn't exposed, so step via move_down.
            for _ = 1, 5 do pmacs.editor.move_down() end
            for _ = 1, 4 do pmacs.editor.move_right() end
            "#,
        )
        .expect("setup buffer");

    state
        .lua_host
        .eval(
            Some("invoke"),
            r#"pmacs.command.invoke("ai.ask-about-function")"#,
        )
        .expect("invoke");

    // Result lands in *mcp:m9_8:review_function* (the M9.7 buffer
    // namespace, demonstrating composition).
    let appeared = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_8:review_function*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(appeared, "review_function result buffer must be created");

    let body = buffer_body(&mut state, "*mcp:m9_8:review_function*").expect("read body");
    assert!(
        body.contains("fn beta()"),
        "function context must include the enclosing function source; got {body:?}"
    );
    assert!(
        !body.contains("fn alpha()"),
        "context must be the *enclosing* function only — alpha must not leak in; got {body:?}"
    );
    assert!(
        body.contains("demo.rs"),
        "file_path arg must thread to the response; got {body:?}"
    );
}

#[test]
fn m9_8_ask_about_function_no_enclosing_node_surfaces_status() {
    let (mut state, _c, _u) = editor_with_ai();
    spawn_initialized_server(&mut state);
    configure_default(&mut state, "m9_8");

    // Buffer with no functions at all. Cursor at start.
    state
        .lua_host
        .eval(
            Some("setup-buffer"),
            r#"
            local buf = pmacs.buffer.from_bytes("empty.rs", "// just a comment\n")
            pmacs.window.switch_buffer(buf)
            pmacs.parse._parse_now(buf, "rust")
            "#,
        )
        .expect("setup buffer");
    state
        .lua_host
        .eval(
            Some("invoke"),
            r#"pmacs.command.invoke("ai.ask-about-function")"#,
        )
        .expect("invoke");
    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("no enclosing function"),
        "no-enclosing-function path must surface a helpful status; got {status:?}"
    );
}

// ===========================================================================
// Bullet 2: multi-buffer context (project files as structured array)
// ===========================================================================

#[test]
fn m9_8_ask_about_project_sends_files_array() {
    let (mut state, _c, _u) = editor_with_ai();
    spawn_initialized_server(&mut state);
    configure_default(&mut state, "m9_8");

    state
        .lua_host
        .eval(
            Some("open-buffers"),
            r#"
            -- Three file-backed buffers — should all be sent.
            pmacs.buffer.from_bytes("a.rs", "fn a() {}")
            pmacs.buffer.from_bytes("b.rs", "fn b() {}")
            pmacs.buffer.from_bytes("c.lua", "function c() end")
            -- A star-buffer that MUST be excluded (the Q2 rule).
            pmacs.buffer.create("*help*")
            "#,
        )
        .expect("open buffers");

    state
        .lua_host
        .eval(
            Some("invoke"),
            r#"pmacs.command.invoke("ai.ask-about-project")"#,
        )
        .expect("invoke");

    let appeared = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_8:review_project*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(appeared, "review_project result buffer must be created");

    let body = buffer_body(&mut state, "*mcp:m9_8:review_project*").expect("read body");
    assert!(
        body.contains("## a.rs") && body.contains("## b.rs") && body.contains("## c.lua"),
        "all three file-backed buffers must appear in the project review body; got {body:?}"
    );
    assert!(
        !body.contains("*help*"),
        "star-buffers must be excluded from project context; got {body:?}"
    );
    assert!(
        body.contains("fn a()") && body.contains("fn b()") && body.contains("function c()"),
        "file contents must thread through; got {body:?}"
    );
}

#[test]
fn m9_8_collect_project_files_seam_excludes_star_buffers() {
    let (mut state, _c, _u) = editor_with_ai();

    state
        .lua_host
        .eval(
            Some("open-buffers"),
            r#"
            pmacs.buffer.from_bytes("real.rs", "content")
            pmacs.buffer.create("*help*")
            pmacs.buffer.create("*scratch*")
            pmacs.buffer.create("*mcp:foo:bar*")
            "#,
        )
        .expect("open buffers");

    let names: Vec<String> = state
        .lua_host
        .lua()
        .load(
            r"
            local out = {}
            for _, entry in ipairs(_G.AI._collect_project_files()) do
                out[#out+1] = entry.path
            end
            return out
            ",
        )
        .eval()
        .expect("collect");

    assert!(
        names.iter().any(|n| n == "real.rs"),
        "file-backed buffer must be present; got {names:?}"
    );
    for star in ["*help*", "*scratch*", "*mcp:foo:bar*"] {
        assert!(
            !names.iter().any(|n| n == star),
            "{star} must be excluded; got {names:?}"
        );
    }
}

// ===========================================================================
// Bullet 3: configurable server (the architecturally important test)
// ===========================================================================

#[test]
fn m9_8_server_pluggability_a_then_b_routes_command_without_code_changes() {
    let (mut state, _c, _u) = editor_with_ai();
    // Two distinct fake-MCP servers, different labels. Each fake will
    // respond to `ask_freeform` echoing the question back; we route
    // through the AI package's same `ai.ask` command and verify the
    // result buffer for each runs against a *different* label-prefixed
    // buffer name.
    spawn_initialized_server_with_label(&mut state, "model_a", "SERVER_A");
    spawn_initialized_server_with_label(&mut state, "model_b", "SERVER_B");

    // Configure to A; invoke; expect *mcp:model_a:ask_freeform*.
    state
        .lua_host
        .eval(
            Some("configure-a"),
            r#"
            _G.AI.configure {
                server_label = "model_a",
                prompts = { ask = "ask_freeform" },
            }
            pmacs.command.invoke("ai.ask")
            pmacs.minibuffer.set_contents("hello A")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("invoke A");

    let landed_a = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:model_a:ask_freeform*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(
        landed_a,
        "first invoke must land in the model_a-labelled result buffer"
    );

    // Re-configure to B; invoke the SAME COMMAND; expect a *new*
    // result buffer at *mcp:model_b:ask_freeform*. Crucially: no code
    // changes between the two ai.ask invocations — only configure.
    state
        .lua_host
        .eval(
            Some("configure-b"),
            r#"
            _G.AI.configure {
                server_label = "model_b",
                prompts = { ask = "ask_freeform" },
            }
            pmacs.command.invoke("ai.ask")
            pmacs.minibuffer.set_contents("hello B")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("invoke B");

    let landed_b = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:model_b:ask_freeform*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(
        landed_b,
        "second invoke (after re-configure) must land in the model_b-labelled buffer — server pluggability"
    );

    // Both buffers exist concurrently — re-configure didn't kill the
    // first result. The user can compare runs.
    let a_still_there = buffer_body(&mut state, "*mcp:model_a:ask_freeform*").is_some();
    assert!(
        a_still_there,
        "re-configure must not destroy the prior server's result buffer"
    );
}

#[test]
fn m9_8_reconfigure_replaces_label_without_redefining_commands() {
    let (mut state, _c, _u) = editor_with_ai();
    spawn_initialized_server(&mut state);
    configure_default(&mut state, "m9_8");

    // Snapshot command count before re-configure.
    let count_before: i64 = state
        .lua_host
        .lua()
        .load(
            r#"
            local n = 0
            for _, c in ipairs(pmacs.command.list()) do
                if c == "ai.ask-about-function" or c == "ai.ask-about-project" or c == "ai.ask" then
                    n = n + 1
                end
            end
            return n
            "#,
        )
        .eval()
        .expect("count before");
    assert_eq!(count_before, 3, "all three ai.* commands must be defined");

    // Re-configure with a different label.
    state
        .lua_host
        .eval(
            Some("reconfigure"),
            r#"_G.AI.configure { server_label = "m9_8_alt", prompts = { ask = "ask_freeform" } }"#,
        )
        .expect("reconfigure");

    let count_after: i64 = state
        .lua_host
        .lua()
        .load(
            r#"
            local n = 0
            for _, c in ipairs(pmacs.command.list()) do
                if c == "ai.ask-about-function" or c == "ai.ask-about-project" or c == "ai.ask" then
                    n = n + 1
                end
            end
            return n
            "#,
        )
        .eval()
        .expect("count after");
    assert_eq!(
        count_after, 3,
        "re-configure must NOT redefine commands (still all three present)"
    );

    let cfg_label: String = state
        .lua_host
        .lua()
        .load("return _G.AI._config().server_label")
        .eval()
        .expect("read label");
    assert_eq!(
        cfg_label, "m9_8_alt",
        "re-configure must update the server label"
    );
}

// ===========================================================================
// Composition: ai.* lands in M9.7's buffer namespace via M.render
// ===========================================================================

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "two-path composition test (M9.7 path + M9.8 path) needs both setups inline; splitting hides the per-path side effects the test is verifying"
)]
fn m9_8_composes_with_m9_7_render_into_same_buffer() {
    let (mut state, _c, _u) = editor_with_ai();
    spawn_initialized_server(&mut state);

    // Register M9.7's auto-prompt commands too, so the same prompt is
    // reachable from both M9.8's `ai.ask` and M9.7's `m9_8-ask_freeform`.
    state
        .lua_host
        .eval(Some("register-prompts"), "_G.PROMPTS.register(_G.SERVER)")
        .expect("register");
    pump_until_lua_pred(
        &mut state,
        "#_G.PROMPTS.commands_for(_G.SERVER) > 0",
        Duration::from_secs(2),
    );
    configure_default(&mut state, "m9_8");

    // Invoke via M9.7's auto-registered command first.
    state
        .lua_host
        .eval(
            Some("invoke-via-m9_7"),
            r#"
            pmacs.command.invoke("m9_8-ask_freeform")
            pmacs.minibuffer.set_contents("from m9_7 path")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("invoke via m9_7");
    pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_8:ask_freeform*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );

    let count_after_first: i64 = state
        .lua_host
        .lua()
        .load(
            r#"
            local n = 0
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_8:ask_freeform*" then n = n + 1 end
            end
            return n
            "#,
        )
        .eval()
        .expect("count after first");
    assert_eq!(
        count_after_first, 1,
        "exactly one result buffer after first"
    );

    // Now invoke via M9.8's `ai.ask`. The result must land in the
    // SAME buffer — M9.8 composes with M9.7's render, not parallel.
    state
        .lua_host
        .eval(
            Some("invoke-via-m9_8"),
            r#"
            pmacs.command.invoke("ai.ask")
            pmacs.minibuffer.set_contents("from m9_8 path")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("invoke via m9_8");
    // Wait for the M9.8-path response to land — body should contain
    // "from m9_8 path", which proves the M9.8 invoke ran AND landed
    // in the same buffer (not a new one).
    let landed_m9_8 = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_8:ask_freeform*" then
                    local body = id:slice(0, id:len())
                    if string.find(body, "from m9_8 path", 1, true) ~= nil then
                        return true
                    end
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(
        landed_m9_8,
        "M9.8's ai.ask must repaint the same *mcp:m9_8:ask_freeform* buffer with the new response"
    );

    // After the second invoke settles, the count of matching buffers
    // must still be exactly one — composition (not parallel).
    let single_buffer: i64 = state
        .lua_host
        .lua()
        .load(
            r#"
            local n = 0
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_8:ask_freeform*" then n = n + 1 end
            end
            return n
            "#,
        )
        .eval()
        .expect("count single");
    assert_eq!(
        single_buffer, 1,
        "M9.8's ai.ask must reuse the same buffer M9.7 created — composition (got {single_buffer} buffers)"
    );
}

// ===========================================================================
// Tree-sitter walk seam
// ===========================================================================

#[test]
fn m9_8_find_enclosing_function_seam_returns_deepest_match() {
    let (state, _c, _u) = editor_with_ai();

    let (kind, language): (String, String) = state
        .lua_host
        .lua()
        .load(
            r#"
            local source = "fn outer() {\n    fn inner() { let x = 1; }\n}\n"
            local buf = pmacs.buffer.from_bytes("nested.rs", source)
            pmacs.window.switch_buffer(buf)
            pmacs.parse._parse_now(buf, "rust")
            -- Byte position: the `let x = 1` text is around byte 30+.
            -- find_enclosing_function should pick `inner`, not `outer`.
            local target_text = "let x"
            local source_text = source
            local idx = string.find(source_text, target_text)
            local node, lang = _G.AI._find_enclosing_function(buf, idx - 1)
            if node == nil then return "nil", lang or "" end
            return node:type(), lang
            "#,
        )
        .eval()
        .expect("walk");
    assert_eq!(kind, "function_item", "must find a Rust function_item");
    assert_eq!(language, "rust", "must report rust as language");

    // Verify it's the *inner* function by checking its byte range
    // doesn't span the whole buffer.
    let inner_only: bool = state
        .lua_host
        .lua()
        .load(
            r#"
            local source = "fn outer() {\n    fn inner() { let x = 1; }\n}\n"
            local buf = pmacs.window.buffer()
            pmacs.parse._parse_now(buf, "rust")
            local idx = string.find(source, "let x") - 1
            local node = _G.AI._find_enclosing_function(buf, idx)
            -- inner's range is shorter than the whole source.
            return node:end_byte() - node:start_byte() < #source
            "#,
        )
        .eval()
        .expect("inner");
    assert!(inner_only, "deepest match must pick `inner`, not `outer`");
}

// ===========================================================================
// Error paths
// ===========================================================================

// Configure to a label that no live server advertises. Commands
// register, but invoke can't resolve the label and surfaces a helpful
// status. The complementary "configured server vanishes mid-flight"
// path is exercised by `m9_8_server_gone_clears_config`.
#[test]
fn m9_8_invoke_with_bogus_server_label_surfaces_status() {
    let (mut state, _c, _u) = editor_with_ai();
    spawn_initialized_server(&mut state);
    state
        .lua_host
        .eval(
            Some("configure-bogus"),
            r#"_G.AI.configure { server_label = "no-such-server", prompts = { ask = "ask_freeform" } }"#,
        )
        .expect("configure");
    state
        .lua_host
        .eval(Some("invoke"), r#"pmacs.command.invoke("ai.ask")"#)
        .expect("invoke");
    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("no MCP server with label"),
        "missing-server path must surface a helpful status; got {status:?}"
    );
}

// F2: When the configured server vanishes mid-dispatch (`get_prompt`
// fails with "unknown server" / "not ready for requests"), `dispatch`
// clears `_config` so subsequent invocations surface the
// configure-needed message rather than the same dead-server error on
// every retry. Pinned by stop'ing the server, then invoking ai.ask
// and asserting `_config()` is now nil.
#[test]
fn m9_8_server_gone_clears_config() {
    let (mut state, _c, _u) = editor_with_ai();
    spawn_initialized_server(&mut state);
    configure_default(&mut state, "m9_8");
    assert!(
        !state
            .lua_host
            .lua()
            .load("return _G.AI._config() == nil")
            .eval::<bool>()
            .expect("config not nil precondition"),
        "config must be set before stopping server"
    );

    // Stop the server and pump until the manager observes the exit.
    state
        .lua_host
        .eval(Some("stop"), "pmacs.mcp.stop(_G.SERVER)")
        .expect("stop server");
    let stop = Instant::now() + Duration::from_secs(2);
    while Instant::now() < stop {
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

    // Invoke ai.ask. Dispatch must observe server-gone and clear
    // `_config`. Provide minibuffer input so ask_body's chain proceeds
    // to dispatch (otherwise it just opens the minibuffer and waits).
    state
        .lua_host
        .eval(
            Some("invoke-after-stop"),
            r#"
            pmacs.command.invoke("ai.ask")
            pmacs.minibuffer.set_contents("anything")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("invoke after stop");
    let cleared = pump_until_lua_pred(&mut state, "_G.AI._config() == nil", Duration::from_secs(2));
    assert!(
        cleared,
        "server-gone error must clear _config so the next invoke prompts the user to reconfigure"
    );
}

// F4: Boundary — tree-sitter `end_byte` is exclusive, but
// `find_enclosing` includes `byte_pos == end_byte` as enclosing.
// Cursor at the position just past the close-brace of a function
// still gets that function as context. Pinned so the boundary
// behavior is intentional, not accidental.
#[test]
fn m9_8_find_enclosing_at_end_byte_includes_node() {
    let (state, _c, _u) = editor_with_ai();

    let inclusive: bool = state
        .lua_host
        .lua()
        .load(
            r#"
            local source = "fn alone() {}\n"
            local buf = pmacs.buffer.from_bytes("end_byte.rs", source)
            pmacs.window.switch_buffer(buf)
            pmacs.parse._parse_now(buf, "rust")
            local tree = pmacs.parse.tree(buf)
            -- Walk to the function_item node and read its end_byte.
            local function find(node)
                if node:type() == "function_item" then return node end
                for _, c in ipairs(node:children()) do
                    local found = find(c)
                    if found ~= nil then return found end
                end
                return nil
            end
            local fn_node = find(tree:root())
            local eb = fn_node:end_byte()
            -- Probe `_find_enclosing_function` AT eb (inclusive boundary).
            local node, _, fail_kind = _G.AI._find_enclosing_function(buf, eb)
            return fail_kind == nil and node ~= nil and node:type() == "function_item"
            "#,
        )
        .eval()
        .expect("inclusive boundary probe");
    assert!(
        inclusive,
        "cursor at exact end_byte must still be considered enclosing (right edge inclusive)"
    );
}

// F5: When the buffer has no parse view yet, the seam returns
// `(nil, nil, "no_tree")` and the body surfaces a distinct status
// message. Without this distinction, a cursor in an unparsed buffer
// got "no enclosing function at cursor" — misleading because the
// cursor may have been right inside a function whose syntax tree
// just hadn't been built yet.
#[test]
fn m9_8_unparsed_buffer_surfaces_buffer_not_parsed_status() {
    let (mut state, _c, _u) = editor_with_ai();
    spawn_initialized_server(&mut state);
    configure_default(&mut state, "m9_8");

    // Create a buffer but do NOT call `_parse_now` — no parse tree
    // exists for it.
    state
        .lua_host
        .eval(
            Some("setup-unparsed"),
            r#"
            local buf = pmacs.buffer.from_bytes("never_parsed.rs", "fn x() {}")
            pmacs.window.switch_buffer(buf)
            "#,
        )
        .expect("setup unparsed");
    state
        .lua_host
        .eval(
            Some("invoke"),
            r#"pmacs.command.invoke("ai.ask-about-function")"#,
        )
        .expect("invoke");
    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("buffer not parsed yet"),
        "no-parse-tree path must surface a distinct status; got {status:?}"
    );
}

// F1: Project payload size warning. When the collected files exceed
// the soft warning threshold, the package emits a notify so the user
// knows they're about to send a large payload. Collection still
// proceeds — the threshold is informational, not a hard cap.
#[test]
fn m9_8_oversized_project_payload_emits_size_warning() {
    let (mut state, _c, _u) = editor_with_ai();

    // Install a stub `pmacs.error` to capture the warning routed
    // through the `notify` helper (M9.6 finding 10 carry-forward
    // pattern: warnings hit both set_status and pmacs.error).
    state
        .lua_host
        .eval(
            Some("install-error-stub"),
            r"
            _G.PMACS_ERROR_LOG = {}
            pmacs.error = function(msg)
                _G.PMACS_ERROR_LOG[#_G.PMACS_ERROR_LOG + 1] = msg
            end
            ",
        )
        .expect("install error stub");

    // Lower the threshold so the test can produce an "oversized"
    // payload with a few small buffers — keeps the test fast and
    // doesn't depend on the package's default-threshold value.
    state
        .lua_host
        .eval(
            Some("lower-threshold"),
            r"_G.AI._PROJECT_PAYLOAD_WARN_BYTES = 64",
        )
        .expect("lower threshold");

    // Two buffers whose combined content + paths comfortably exceed
    // the lowered threshold.
    state
        .lua_host
        .eval(
            Some("open-buffers"),
            r#"
            pmacs.buffer.from_bytes("file_one.rs", string.rep("a", 60))
            pmacs.buffer.from_bytes("file_two.rs", string.rep("b", 60))
            "#,
        )
        .expect("open buffers");

    state
        .lua_host
        .eval(Some("collect"), "_G.AI._collect_project_files()")
        .expect("collect");

    let logged: bool = state
        .lua_host
        .lua()
        .load(
            r#"
            for _, m in ipairs(_G.PMACS_ERROR_LOG) do
                if string.find(m, "warning threshold", 1, true) ~= nil then
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
        "oversized project payload must surface a size warning through pmacs.error"
    );
}

#[test]
fn m9_8_configured_prompt_missing_on_server_surfaces_error() {
    let (mut state, _c, _u) = editor_with_ai();
    spawn_initialized_server(&mut state);
    // Configure a prompt name that the fake server doesn't advertise.
    state
        .lua_host
        .eval(
            Some("configure-bogus-prompt"),
            r#"_G.AI.configure { server_label = "m9_8", prompts = { ask = "no_such_prompt" } }"#,
        )
        .expect("configure");
    state
        .lua_host
        .eval(
            Some("invoke"),
            r#"
            pmacs.command.invoke("ai.ask")
            pmacs.minibuffer.set_contents("anything")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("invoke");
    // The fake returns -32602 unknown prompt; ai dispatch surfaces it
    // as "ai no_such_prompt error: ...".
    let stop = Instant::now() + Duration::from_secs(2);
    let mut saw_err = false;
    while Instant::now() < stop {
        state.tick_processes();
        state.tick_mcp();
        state.tick_async();
        if state.core.borrow().status.contains("unknown prompt") {
            saw_err = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    assert!(
        saw_err,
        "missing-prompt-on-server path must surface the JSON-RPC error message; got status={:?}",
        state.core.borrow().status
    );
}
