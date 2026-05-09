// m9_7_acceptance.rs --- T M9.7 prompts-as-result-buffers acceptance.

//! Acceptance tests for T M9.7 (`spec/pmacs-tasks.tex:4005`):
//!
//!   1. Text-format prompt result renders as a plain buffer.
//!   2. Code-format result renders with the appropriate language's
//!      syntax highlighting (via tree-sitter from M4).
//!   3. Markdown-format result renders structured (headers, lists,
//!      code blocks).
//!
//! Plus the format-dispatch behavior:
//!
//!   * `_meta.format = unrecognized` falls back to text rendering with
//!     a warning routed through both `set_status` and `pmacs.error`.
//!   * Multi-message prompts render with `## <role>` level-2 headers.
//!   * Non-text content (image / resource) renders as a placeholder
//!     line rather than being silently dropped.
//!   * Re-invoking a prompt reuses the existing result buffer; cursor
//!     resets to (0, 0).
//!
//! Plus the lifecycle smoke tests (M9.6 patterns hold for M9.7):
//!
//!   * register → commands appear; unregister drops them.
//!   * `notifications/prompts/list_changed` reconciles add / remove.
//!
//! M9.7 ships with zero new public Rust APIs and zero new public Lua
//! `pmacs.mcp.*` APIs. The M9.5 `on_notification` dispatcher is now
//! its third consumer (after M9.5 itself and M9.6), confirming the
//! abstraction holds for the M9 prompt-family work. The one v0.1
//! surface expansion is the markdown grammar in
//! `BUILTIN_LANGUAGES` — a different dimension from API surface.
//!
//! Surface additions (test seams, leading-underscore — not stable
//! user-facing API):
//!
//!   * `pmacs.window._overlay_kinds()` — list of overlay-kind
//!     strings on the active window. Used to verify that
//!     `_attach_highlight` actually landed an overlay.
//!   * `pmacs.window._view_top()` / `_set_view_top(n)` — viewport
//!     introspection / poke for the re-invoke reset test.
//!   * `View::kind()` trait method (Rust-side) — defaults to
//!     `"unknown"`; `SyntaxHighlightView` overrides to
//!     `"syntax-highlight"`.
//!
//! Carry-forward (not M9.7's own work):
//!
//!   * `editor.describe-command` user-facing builtin in
//!     `builtin/commands/default.lua` closes an M9.6 acceptance gap
//!     (the user-facing entry point for the
//!     "`describe-command` reports the tool's schema" bullet).
//!     Lands here for convenience; attribution to M9.6.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pmacs::editor::EditorState;
use pmacs::lua_bindings::PackageInstallOverride;
use tempfile::TempDir;

fn fake_mcp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_mcp").to_owned()
}

fn prompts_package_path() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.join("tests")
        .join("fixtures")
        .join("pmacs-mcp-prompts")
}

/// Build an editor with the pmacs-mcp-prompts package installed and
/// require()d. Returns the editor state plus the temp dirs that must
/// outlive it.
fn editor_with_prompts() -> (EditorState, TempDir, TempDir) {
    let cache = tempfile::tempdir().expect("cache tempdir");
    let user_root = tempfile::tempdir().expect("user-root tempdir");
    let mut state = EditorState::new();
    state.lua_host.reopen_init_phase_for_testing();
    state.lua_host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );
    let pkg = prompts_package_path();
    let pkg_str = pkg.display().to_string();
    let install = format!(
        r#"
        pmacs.packages.install_local("{pkg_str}")
        _G.PROMPTS = require("pmacs-mcp-prompts")
    "#
    );
    state
        .lua_host
        .eval(Some("install-prompts"), &install)
        .unwrap_or_else(|e| panic!("install_local + require failed: {e}"));
    (state, cache, user_root)
}

/// Spawn the fake MCP server, drain until Initialized, stash the
/// server handle in `_G.SERVER`. Label is `m9_7` (used as the
/// command-name prefix and the result-buffer label half).
fn spawn_initialized_server(state: &mut EditorState) {
    let fake = fake_mcp_path();
    state
        .lua_host
        .lua()
        .load(format!(
            "
            _G.SERVER = pmacs.mcp.spawn({{
                label = 'm9_7',
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

/// Register the fake server's prompts and pump until the initial
/// fetch completes.
fn register_and_wait(state: &mut EditorState) {
    state
        .lua_host
        .eval(Some("register-prompts"), "_G.PROMPTS.register(_G.SERVER)")
        .expect("register");
    let registered = pump_until_lua_pred(
        state,
        "#_G.PROMPTS.commands_for(_G.SERVER) > 0",
        Duration::from_secs(5),
    );
    assert!(
        registered,
        "register() must populate commands_for within 5s"
    );
}

/// Read the entire body of the buffer named `name`, or return `None`
/// if no such buffer exists. Uses `describe.buffer(id).name` to find
/// the target, then `id:slice(0, id:len())` to read just that buffer's
/// body — keeps the cost at O(target buffer size) rather than
/// materializing every buffer's bytes.
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

// ===========================================================================
// Lifecycle smoke
// ===========================================================================

#[test]
fn m9_7_register_adds_prompts_as_commands() {
    let (mut state, _c, _u) = editor_with_prompts();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    let cmds: Vec<String> = state
        .lua_host
        .lua()
        .load("return _G.PROMPTS.commands_for(_G.SERVER)")
        .eval()
        .expect("commands_for");
    assert!(
        cmds.iter().any(|c| c == "m9_7-simple"),
        "simple prompt must register; got {cmds:?}"
    );
    assert!(
        cmds.iter().any(|c| c == "m9_7-code_review"),
        "code_review prompt must register; got {cmds:?}"
    );
    assert!(
        cmds.iter().any(|c| c == "m9_7-markdown_demo"),
        "markdown_demo prompt must register; got {cmds:?}"
    );

    let global: Vec<String> = state
        .lua_host
        .lua()
        .load("return pmacs.command.list()")
        .eval()
        .expect("command.list");
    assert!(
        global.iter().any(|c| c == "m9_7-simple"),
        "registered prompt commands must appear in pmacs.command.list(); got {global:?}"
    );
}

#[test]
fn m9_7_unregister_drops_commands() {
    let (mut state, _c, _u) = editor_with_prompts();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(Some("unregister"), "_G.PROMPTS.unregister(_G.SERVER)")
        .expect("unregister");

    let after: Vec<String> = state
        .lua_host
        .lua()
        .load("return _G.PROMPTS.commands_for(_G.SERVER)")
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
    assert!(
        !global.iter().any(|c| c == "m9_7-simple"),
        "unregister must remove m9_7-simple from pmacs.command.list(); got {global:?}"
    );
}

#[test]
fn m9_7_list_changed_reconciles_add_and_remove() {
    let (mut state, _c, _u) = editor_with_prompts();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("trigger-add"),
            r#"
            pmacs.async(function()
                pmacs.mcp.invoke_tool(_G.SERVER, "mcp_test/add_prompt",
                                      { name = "fresh_prompt" }):await()
            end)
            "#,
        )
        .expect("invoke add_prompt");

    let added = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, c in ipairs(_G.PROMPTS.commands_for(_G.SERVER)) do
                if c == "m9_7-fresh_prompt" then return true end
            end
            return false
        end)()"#,
        Duration::from_millis(1500),
    );
    assert!(
        added,
        "list_changed must drive registration of the new prompt within 1.5s"
    );

    state
        .lua_host
        .eval(
            Some("trigger-remove"),
            r#"
            pmacs.async(function()
                pmacs.mcp.invoke_tool(_G.SERVER, "mcp_test/remove_prompt",
                                      { name = "fresh_prompt" }):await()
            end)
            "#,
        )
        .expect("invoke remove_prompt");

    let removed = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, c in ipairs(_G.PROMPTS.commands_for(_G.SERVER)) do
                if c == "m9_7-fresh_prompt" then return false end
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

// ===========================================================================
// Bullet 1: text-format prompt renders as a plain buffer
// ===========================================================================

#[test]
fn m9_7_text_prompt_renders_as_plain_buffer() {
    let (mut state, _c, _u) = editor_with_prompts();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("invoke-simple"),
            r#"pmacs.command.invoke("m9_7-simple")"#,
        )
        .expect("invoke simple");

    // Wait for the result buffer to appear.
    let appeared = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_7:simple*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(appeared, "result buffer must be created");

    let body = buffer_body(&mut state, "*mcp:m9_7:simple*").expect("read body");
    assert!(
        body.contains("no-args prompt body"),
        "buffer must contain the prompt's text content; got {body:?}"
    );
    // Audit finding C1: single-message text-format prompts render
    // as a plain buffer per the spec — no `## user` ceremonial
    // header. Multi-message prompts still get role headers
    // (m9_7_multi_message_renders_with_role_headers covers that).
    assert!(
        !body.contains("## user"),
        "single-message text-format prompt must render plainly with no role header; got {body:?}"
    );
}

// ===========================================================================
// Bullet 2: code-format prompt with syntax highlighting
// ===========================================================================
//
// "Highlighting attached" is the M9.7-observable property. We don't
// inspect the actual rendered colors (that's M4's territory) — we
// verify that `_attach_highlight` succeeded and the parse view
// landed. The fake's `code_demo` prompt returns `_meta.format = "code"`
// + `_meta.language = "rust"`; the package routes it through
// `pmacs.parse._dispatch` + `_attach_highlight` for the rust grammar.

#[test]
fn m9_7_code_prompt_renders_with_syntax_highlight() {
    let (mut state, _c, _u) = editor_with_prompts();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("invoke-code-demo"),
            r#"pmacs.command.invoke("m9_7-code_demo")"#,
        )
        .expect("invoke code_demo");

    let appeared = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_7:code_demo*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(appeared, "code result buffer must be created");

    let body = buffer_body(&mut state, "*mcp:m9_7:code_demo*").expect("read body");
    assert!(
        body.contains("fn main()"),
        "code prompt must render the source body; got {body:?}"
    );

    // Active window must be on the code buffer (so the
    // `_attach_highlight` precondition was met) AND the active
    // window's overlay stack must contain a `syntax-highlight`
    // overlay. The latter is the actual proof that highlighting
    // attached — without the introspection seam this test was
    // accepting "no error" as proof, which a regression that
    // dropped the rust highlights query would still pass.
    let highlighted = pump_until_lua_pred(
        &mut state,
        r#"(function()
            local buf = pmacs.window.buffer()
            if buf == nil then return false end
            local d = pmacs.describe.buffer(buf)
            if d == nil or d.name ~= "*mcp:m9_7:code_demo*" then return false end
            for _, k in ipairs(pmacs.window._overlay_kinds()) do
                if k == "syntax-highlight" then return true end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(
        highlighted,
        "active window must carry a syntax-highlight overlay after code-format invoke"
    );
}

// ===========================================================================
// Bullet 3: markdown-format prompt with markdown highlighting
// ===========================================================================

#[test]
fn m9_7_markdown_prompt_renders_with_markdown_highlight() {
    let (mut state, _c, _u) = editor_with_prompts();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("invoke-md-demo"),
            r#"pmacs.command.invoke("m9_7-markdown_demo")"#,
        )
        .expect("invoke markdown_demo");

    let appeared = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_7:markdown_demo*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(appeared, "markdown result buffer must be created");

    let body = buffer_body(&mut state, "*mcp:m9_7:markdown_demo*").expect("read body");
    assert!(
        body.contains("# Heading One"),
        "markdown prompt must render the header content; got {body:?}"
    );
    assert!(
        body.contains("- bullet alpha"),
        "markdown prompt must render the list items; got {body:?}"
    );
    assert!(
        body.contains("```rust"),
        "markdown prompt must render the code fence; got {body:?}"
    );

    // Verify the markdown grammar is registered (M9.7's grammar
    // addition). `language_for_path` for a `.md` extension must
    // resolve to "markdown".
    let lang: Option<String> = state
        .lua_host
        .lua()
        .load(r#"return pmacs.parse.language_for_path("README.md")"#)
        .eval()
        .expect("language_for_path");
    assert_eq!(
        lang.as_deref(),
        Some("markdown"),
        "markdown grammar must be registered in BUILTIN_LANGUAGES"
    );

    // The markdown buffer must carry a syntax-highlight overlay.
    // Mirrors the code-format assertion: the actual proof that
    // markdown highlighting attached is the overlay landing on the
    // active window's stack, not just that the grammar exists.
    let highlighted = pump_until_lua_pred(
        &mut state,
        r#"(function()
            local buf = pmacs.window.buffer()
            if buf == nil then return false end
            local d = pmacs.describe.buffer(buf)
            if d == nil or d.name ~= "*mcp:m9_7:markdown_demo*" then return false end
            for _, k in ipairs(pmacs.window._overlay_kinds()) do
                if k == "syntax-highlight" then return true end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(
        highlighted,
        "active window must carry a syntax-highlight overlay after markdown-format invoke"
    );
}

// ===========================================================================
// Format-hint behavior
// ===========================================================================

#[test]
fn m9_7_unknown_format_falls_back_to_text_with_warning() {
    let (mut state, _c, _u) = editor_with_prompts();
    spawn_initialized_server(&mut state);

    // Stub pmacs.error so we can verify the warning routes through
    // the persistent log as well as set_status (M9.6 finding 10
    // carry-forward).
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

    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("invoke-unknown"),
            r#"pmacs.command.invoke("m9_7-unknown_format")"#,
        )
        .expect("invoke unknown_format");

    let appeared = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_7:unknown_format*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(
        appeared,
        "unknown-format buffer must still be created (text fallback)"
    );

    let body = buffer_body(&mut state, "*mcp:m9_7:unknown_format*").expect("read body");
    assert!(
        body.contains("body for an unrecognized format"),
        "unknown-format prompt must still render its text body; got {body:?}"
    );

    // The warning must reach pmacs.error.
    let logged: bool = state
        .lua_host
        .lua()
        .load(
            r#"
            for _, m in ipairs(_G.PMACS_ERROR_LOG) do
                if string.find(m, "unknown format hint", 1, true) ~= nil
                   and string.find(m, "unknown_format", 1, true) ~= nil then
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
        "unknown-format warning must reach pmacs.error so it survives past the next set_status"
    );
}

// ===========================================================================
// Audit finding E2: apply_fresh re-checks `state.cancelled` per
// iteration. Without the per-iteration check, a fast unregister
// landing mid-apply could re-register commands the user just
// dropped.
// ===========================================================================

#[test]
fn m9_7_apply_fresh_bails_out_when_cancelled_mid_loop() {
    let (state, _c, _u) = editor_with_prompts();

    // Pre-existing command names we'll watch — they must not appear
    // in `pmacs.command.list()` after `apply_fresh` runs against a
    // cancelled state.
    let registered_after_cancelled: bool = state
        .lua_host
        .lua()
        .load(
            r#"
            local s = _G.PROMPTS._make_test_state("m9_7e2")
            s.cancelled = true
            local fresh = {
                { name = "ghost1", description = "shouldn't land" },
                { name = "ghost2", description = "shouldn't land" },
                { name = "ghost3", description = "shouldn't land" },
            }
            -- server arg is unused because cancelled = true short-circuits
            -- before register_one ever runs.
            _G.PROMPTS._apply_fresh(s, nil, fresh)
            for _, c in ipairs(pmacs.command.list()) do
                if c == "m9_7e2-ghost1" or c == "m9_7e2-ghost2" or c == "m9_7e2-ghost3" then
                    return true
                end
            end
            for _, _ in pairs(s.prompts) do
                return true  -- nothing should be in the state's prompts table either
            end
            return false
            "#,
        )
        .eval()
        .expect("apply_fresh on cancelled state");
    assert!(
        !registered_after_cancelled,
        "apply_fresh must short-circuit on cancelled state — no commands or prompt-state entries"
    );
}

// ===========================================================================
// Audit finding C3: markdown floor test. Block-grammar-only
// highlighting is the v0.1 floor. Inline emphasis / links / inline
// code is unhighlighted today (M9.8+ work). The floor is: such a
// body must (a) render verbatim, (b) get a syntax-highlight overlay
// attached (the block grammar parses fine), and (c) not crash.
// ===========================================================================

#[test]
fn m9_7_markdown_inline_floor_renders_without_crashing() {
    let (mut state, _c, _u) = editor_with_prompts();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("invoke-md-inline"),
            r#"pmacs.command.invoke("m9_7-markdown_inline")"#,
        )
        .expect("invoke markdown_inline");

    let appeared = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_7:markdown_inline*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(appeared, "markdown_inline buffer must be created");

    // Body renders verbatim — inline syntax stays as literal bytes.
    let body = buffer_body(&mut state, "*mcp:m9_7:markdown_inline*").expect("read body");
    assert!(
        body.contains("**bold**"),
        "inline bold marker must render verbatim; got {body:?}"
    );
    assert!(
        body.contains("_emphasis_"),
        "inline emphasis marker must render verbatim; got {body:?}"
    );
    assert!(
        body.contains("[link](https://example.invalid/)"),
        "inline link must render verbatim; got {body:?}"
    );
    assert!(
        body.contains("`inline code`"),
        "inline code must render verbatim; got {body:?}"
    );

    // Block grammar still attaches an overlay (the document parses
    // fine even if inline spans are unhighlighted).
    let highlighted = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, k in ipairs(pmacs.window._overlay_kinds()) do
                if k == "syntax-highlight" then return true end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(
        highlighted,
        "block grammar must attach an overlay even when body has only inline syntax"
    );
}

// ===========================================================================
// Audit finding A2: buffer name normalization stays aligned with
// command name normalization. A label with a space and a prompt with
// a slash must produce a buffer name without raw spaces/slashes, and
// the result must match `command_name(label, prompt)` modulo the
// `*mcp:<label>:<prompt>*` framing.
// ===========================================================================

#[test]
fn m9_7_buffer_name_normalizes_label_and_prompt_halves() {
    let (state, _c, _u) = editor_with_prompts();
    let (cmd, buf): (String, String) = state
        .lua_host
        .lua()
        .load(
            r#"
            return _G.PROMPTS.command_name("my server!", "code/review"),
                   _G.PROMPTS.buffer_name("my server!", "code/review")
            "#,
        )
        .eval()
        .expect("normalized names");
    assert_eq!(
        cmd, "my-server--code-review",
        "command_name must normalize both halves"
    );
    assert_eq!(
        buf, "*mcp:my-server-:code-review*",
        "buffer_name must use the same normalization on each half"
    );
    // Cross-check: the part of buf between `*mcp:` and the second `:`
    // must be normalize_half(label); the part between the second `:`
    // and the trailing `*` must be normalize_half(prompt). And the two
    // halves joined by `-` (the separator command_name uses) should
    // reconstruct cmd.
    let label_half = "my-server-";
    let prompt_half = "code-review";
    assert_eq!(format!("{label_half}-{prompt_half}"), cmd);
    assert_eq!(format!("*mcp:{label_half}:{prompt_half}*"), buf);
}

// ===========================================================================
// Audit finding A1: code-format with an unregistered grammar must
// fall back to text rendering with a warning, not let the
// "unknown language" throw escape the async coroutine.
// ===========================================================================

#[test]
fn m9_7_code_unknown_language_falls_back_to_text_with_warning() {
    let (mut state, _c, _u) = editor_with_prompts();
    spawn_initialized_server(&mut state);

    // Same pmacs.error stub pattern as the unknown-format test so we
    // can verify the warning survives past the next set_status.
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

    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("invoke-code-unknown-lang"),
            r#"pmacs.command.invoke("m9_7-code_unknown_lang")"#,
        )
        .expect("invoke code_unknown_lang");

    let appeared = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_7:code_unknown_lang*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(
        appeared,
        "buffer must still be created when grammar is missing (text fallback)"
    );

    let body = buffer_body(&mut state, "*mcp:m9_7:code_unknown_lang*").expect("read body");
    assert!(
        body.contains("QaH!"),
        "body must render verbatim despite missing grammar; got {body:?}"
    );

    // The warning must reach pmacs.error and reference the language.
    let logged = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, m in ipairs(_G.PMACS_ERROR_LOG) do
                if string.find(m, "no grammar for", 1, true) ~= nil
                   and string.find(m, "klingon", 1, true) ~= nil then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(
        logged,
        "unknown-grammar warning must reach pmacs.error and name the language"
    );
}

#[test]
fn m9_7_multi_message_renders_with_role_headers() {
    let (mut state, _c, _u) = editor_with_prompts();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("invoke-multi"),
            r#"pmacs.command.invoke("m9_7-multi_message")"#,
        )
        .expect("invoke multi_message");

    let appeared = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_7:multi_message*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(appeared, "multi-message buffer must be created");

    let body = buffer_body(&mut state, "*mcp:m9_7:multi_message*").expect("read body");
    assert!(
        body.contains("## system"),
        "system role must surface as level-2 header; got {body:?}"
    );
    assert!(
        body.contains("## user"),
        "user role must surface as level-2 header; got {body:?}"
    );
    assert!(
        body.contains("## assistant"),
        "assistant role must surface as level-2 header; got {body:?}"
    );
    // Ordering: system before user before assistant (rendering must
    // preserve message-array order).
    let sys_pos = body.find("## system").expect("system position");
    let user_pos = body.find("## user").expect("user position");
    let asst_pos = body.find("## assistant").expect("assistant position");
    assert!(
        sys_pos < user_pos && user_pos < asst_pos,
        "messages must render in document order; got body={body:?}"
    );
}

#[test]
fn m9_7_non_text_content_renders_as_placeholder() {
    let (mut state, _c, _u) = editor_with_prompts();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("invoke-mixed"),
            r#"pmacs.command.invoke("m9_7-mixed_content")"#,
        )
        .expect("invoke mixed_content");

    let appeared = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_7:mixed_content*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(appeared, "mixed-content buffer must be created");

    let body = buffer_body(&mut state, "*mcp:m9_7:mixed_content*").expect("read body");
    assert!(
        body.contains("preamble line"),
        "text content must render verbatim; got {body:?}"
    );
    assert!(
        body.contains("[image: image/png]"),
        "image content must render as a placeholder line with mimeType; got {body:?}"
    );
}

#[test]
fn m9_7_buffer_reused_on_reinvoke() {
    let (mut state, _c, _u) = editor_with_prompts();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(Some("invoke-1"), r#"pmacs.command.invoke("m9_7-simple")"#)
        .expect("invoke 1");
    let appeared = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_7:simple*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(appeared, "buffer must be created on first invoke");

    // Snapshot the buffer count and the simple-buffer's identity.
    let buffer_count_1: i64 = state
        .lua_host
        .lua()
        .load("return #pmacs.buffer.list()")
        .eval()
        .expect("count 1");

    state
        .lua_host
        .eval(Some("invoke-2"), r#"pmacs.command.invoke("m9_7-simple")"#)
        .expect("invoke 2");

    // Pump and assert that no NEW buffer with this name was created.
    pump_until_lua_pred(
        &mut state,
        r#"(function()
            local count = 0
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_7:simple*" then
                    count = count + 1
                end
            end
            return count == 1
        end)()"#,
        Duration::from_secs(2),
    );

    let buffer_count_2: i64 = state
        .lua_host
        .lua()
        .load("return #pmacs.buffer.list()")
        .eval()
        .expect("count 2");
    assert_eq!(
        buffer_count_1, buffer_count_2,
        "re-invoking must reuse the buffer; total buffer count must not grow"
    );

    let simple_buffers: i64 = state
        .lua_host
        .lua()
        .load(
            r#"
            local count = 0
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_7:simple*" then
                    count = count + 1
                end
            end
            return count
            "#,
        )
        .eval()
        .expect("simple count");
    assert_eq!(
        simple_buffers, 1,
        "exactly one *mcp:m9_7:simple* buffer must exist after re-invoke"
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "exhaustive cursor / region / scroll / overlay reset assertions; splitting into helpers obscures the test's narrative"
)]
fn m9_7_buffer_state_resets_on_reinvoke() {
    let (mut state, _c, _u) = editor_with_prompts();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    // Invoke once, wait for buffer.
    state
        .lua_host
        .eval(
            Some("invoke-md-1"),
            r#"pmacs.command.invoke("m9_7-markdown_demo")"#,
        )
        .expect("invoke 1");
    let appeared = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_7:markdown_demo*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(appeared, "buffer must exist after first invoke");

    // Move the cursor to a non-zero position, anchor a selection
    // (region != None), and force the viewport down via the
    // `_set_view_top` test seam — the markdown demo body is short
    // enough that natural cursor motion wouldn't push view_top past
    // 0, so we poke it directly.
    state
        .lua_host
        .eval(
            Some("scroll-mid"),
            r"
            for _ = 1, 5 do pmacs.editor.move_down() end
            pmacs.editor.begin_selection(0)
            for _ = 1, 3 do pmacs.editor.move_down() end
            pmacs.window._set_view_top(3)
            ",
        )
        .expect("move_down + select + scroll");
    let mid_cursor: i64 = state
        .lua_host
        .lua()
        .load("return pmacs.editor.cursor()")
        .eval()
        .expect("cursor mid");
    assert!(
        mid_cursor > 0,
        "precondition: cursor must move past 0 before re-invoke (got {mid_cursor})"
    );
    let mid_view_top: i64 = state
        .lua_host
        .lua()
        .load("return pmacs.window._view_top()")
        .eval()
        .expect("view_top mid");
    assert_eq!(
        mid_view_top, 3,
        "precondition: view_top must be poked to 3 before re-invoke (got {mid_view_top})"
    );
    let region_active: bool = state
        .lua_host
        .lua()
        .load("return pmacs.editor.region() ~= nil")
        .eval()
        .expect("region mid");
    assert!(
        region_active,
        "precondition: a region must be active before re-invoke"
    );

    // Re-invoke. The package must reset cursor to 0, clear the
    // region, and reset view_top to 0 via the buffer switch (M9.7
    // commitment: cursor reset, region cleared, scroll reset).
    state
        .lua_host
        .eval(
            Some("invoke-md-2"),
            r#"pmacs.command.invoke("m9_7-markdown_demo")"#,
        )
        .expect("invoke 2");
    let all_reset = pump_until_lua_pred(
        &mut state,
        "pmacs.editor.cursor() == 0
            and pmacs.window._view_top() == 0
            and pmacs.editor.region() == nil",
        Duration::from_secs(2),
    );
    assert!(
        all_reset,
        "re-invoke must reset cursor, region, and view_top; got cursor={}, view_top={}, region_nil={}",
        state
            .lua_host
            .lua()
            .load("return pmacs.editor.cursor()")
            .eval::<i64>()
            .unwrap_or(-1),
        state
            .lua_host
            .lua()
            .load("return pmacs.window._view_top()")
            .eval::<i64>()
            .unwrap_or(-1),
        state
            .lua_host
            .lua()
            .load("return pmacs.editor.region() == nil")
            .eval::<bool>()
            .unwrap_or(false),
    );
}

// ===========================================================================
// Argument prompts
// ===========================================================================

#[test]
fn m9_7_command_with_required_arg_prompts_via_minibuffer() {
    let (mut state, _c, _u) = editor_with_prompts();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("invoke-code-review"),
            r#"pmacs.command.invoke("m9_7-code_review")"#,
        )
        .expect("invoke code_review");

    // First prompt: language.
    assert!(
        state
            .lua_host
            .lua()
            .load("return pmacs.minibuffer.is_active()")
            .eval::<bool>()
            .expect("is_active 1"),
        "first prompt (language) must be active"
    );
    state
        .lua_host
        .eval(
            Some("type-language"),
            r#"
            pmacs.minibuffer.set_contents("rust")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("type language");

    // Second prompt: source.
    assert!(
        state
            .lua_host
            .lua()
            .load("return pmacs.minibuffer.is_active()")
            .eval::<bool>()
            .expect("is_active 2"),
        "second prompt (source) must open after first accept"
    );
    state
        .lua_host
        .eval(
            Some("type-source"),
            r#"
            pmacs.minibuffer.set_contents("fn x() {}")
            pmacs.minibuffer.accept()
            "#,
        )
        .expect("type source");

    // The code_review prompt echoes args into the body text. Wait for
    // the buffer to render and assert both args round-tripped.
    let appeared = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_7:code_review*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(appeared, "code_review buffer must be created");

    let body = buffer_body(&mut state, "*mcp:m9_7:code_review*").expect("read body");
    assert!(
        body.contains("rust"),
        "language arg must thread to body; got {body:?}"
    );
    assert!(
        body.contains("fn x() {}"),
        "source arg must thread to body; got {body:?}"
    );
}

#[test]
fn m9_7_command_with_no_args_dispatches_immediately() {
    let (mut state, _c, _u) = editor_with_prompts();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    state
        .lua_host
        .eval(
            Some("invoke-simple"),
            r#"pmacs.command.invoke("m9_7-simple")"#,
        )
        .expect("invoke simple");

    let active: bool = state
        .lua_host
        .lua()
        .load("return pmacs.minibuffer.is_active()")
        .eval()
        .expect("is_active");
    assert!(!active, "no-arg prompt must not open the minibuffer");

    let appeared = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == "*mcp:m9_7:simple*" then
                    return true
                end
            end
            return false
        end)()"#,
        Duration::from_secs(2),
    );
    assert!(
        appeared,
        "no-arg prompt must dispatch immediately and produce the result buffer"
    );
}

// ===========================================================================
// describe-command surfaces the prompt schema (mirrors M9.6 finding 1)
// ===========================================================================

#[test]
fn m9_7_describe_command_reports_prompt_schema_in_description() {
    let (mut state, _c, _u) = editor_with_prompts();
    spawn_initialized_server(&mut state);
    register_and_wait(&mut state);

    let desc: String = state
        .lua_host
        .lua()
        .load(
            r#"
            local info = pmacs.describe.command("m9_7-code_review")
            return info.description
            "#,
        )
        .eval()
        .expect("describe code_review");
    assert!(
        desc.contains("Review this {language} code"),
        "description must include the prompt's text description; got {desc:?}"
    );
    assert!(
        desc.contains("Arguments:"),
        "description must include an Arguments: section; got {desc:?}"
    );
    assert!(
        desc.contains("language (string, required)"),
        "required arg must be tagged; got {desc:?}"
    );
    assert!(
        desc.contains("source (string, required)"),
        "second required arg must also appear; got {desc:?}"
    );

    // No-args prompt: schema doc has just the description, no
    // Arguments section.
    let simple_desc: String = state
        .lua_host
        .lua()
        .load(
            r#"
            local info = pmacs.describe.command("m9_7-simple")
            return info.description
            "#,
        )
        .eval()
        .expect("describe simple");
    assert!(
        !simple_desc.contains("Arguments:"),
        "no-args prompt must not emit an Arguments: section; got {simple_desc:?}"
    );
}

// ===========================================================================
// Static seam: prompt_hash includes argument order as identity
// ===========================================================================
//
// M9.6 finding 4 carry-forward — required-arg order is part of the
// hash so a reorder triggers re-registration. Pinned via the
// `_prompt_hash` test seam without spinning up a server.

#[test]
fn m9_7_prompt_hash_includes_required_argument_order() {
    let (state, _c, _u) = editor_with_prompts();
    let (h_ab, h_ba): (String, String) = state
        .lua_host
        .lua()
        .load(
            r#"
            local make = function(req)
                local args = {}
                for i, name in ipairs(req) do
                    args[i] = { name = name, required = true }
                end
                return { name = "p", description = "d", arguments = args }
            end
            return _G.PROMPTS._prompt_hash(make({"a","b"})),
                   _G.PROMPTS._prompt_hash(make({"b","a"}))
            "#,
        )
        .eval()
        .expect("hashes");
    assert_ne!(
        h_ab, h_ba,
        "reordering required args must change the prompt hash so reconcile re-registers"
    );
}
