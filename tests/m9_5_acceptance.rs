// m9_5_acceptance.rs --- T M9.5 resources-as-buffers acceptance.

//! Acceptance tests for T M9.5 (`spec/pmacs-tasks.tex:3966`):
//!
//!   1. File resources render with appropriate content-type
//!      interpretation.
//!   2. Directory resources render as navigable buffers (degenerate
//!      case of dired-class from M8).
//!   3. Resource buffers refresh when the underlying resource
//!      changes (subscription, when supported).
//!
//! Plus the design-review additions:
//!
//!   * Server crash leaves buffer stale (no refresh polling errors).
//!   * Lua surface end-to-end through `pmacs.async`.
//!   * Lifecycle round-trip: open → close → re-open.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pmacs::editor::EditorState;
use pmacs::frontend::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use pmacs::lua_bindings::PackageInstallOverride;
use pmacs::protocol::FrontendId;
use tempfile::TempDir;

fn plain_key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    }
}

fn fake_mcp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_mcp").to_owned()
}

fn resources_package_path() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.join("tests")
        .join("fixtures")
        .join("pmacs-mcp-resources")
}

/// Build an editor with the pmacs-mcp-resources package installed
/// and require()d. Returns the editor state plus the temp dirs that
/// must outlive it (cache + user-install-root for the package
/// system).
fn editor_with_resources() -> (EditorState, TempDir, TempDir) {
    let cache = tempfile::tempdir().expect("cache tempdir");
    let user_root = tempfile::tempdir().expect("user-root tempdir");
    let mut state = EditorState::new();
    state.lua_host.reopen_init_phase_for_testing();
    state.lua_host.set_package_install_override(
        PackageInstallOverride::new()
            .with_cache_dir(cache.path().to_path_buf())
            .with_user_install_root(user_root.path().to_path_buf()),
    );
    let pkg = resources_package_path();
    let pkg_str = pkg.display().to_string();
    let install = format!(
        r#"
        pmacs.packages.install_local("{pkg_str}")
        _G.RES = require("pmacs-mcp-resources")
    "#
    );
    state
        .lua_host
        .eval(Some("install-resources"), &install)
        .unwrap_or_else(|e| panic!("install_local + require failed: {e}"));
    (state, cache, user_root)
}

/// Spawn the fake MCP server, drain until Initialized, stash the
/// server handle in `_G.SERVER`.
fn spawn_initialized_server(state: &mut EditorState) {
    let fake = fake_mcp_path();
    state
        .lua_host
        .lua()
        .load(format!(
            "
            _G.SERVER = pmacs.mcp.spawn({{
                label = 'm9_5',
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

// ===========================================================================
// Bullet 1: file resources render with appropriate content-type
// ===========================================================================

/// Bullet 1a: text/plain renders. The fake's `mcp://text/doc.txt`
/// resource holds "initial doc body"; opening it produces a buffer
/// with that exact body.
#[test]
fn m9_5_text_resource_renders_as_buffer() {
    let (mut state, _cache, _user_root) = editor_with_resources();
    spawn_initialized_server(&mut state);
    state
        .lua_host
        .lua()
        .load(
            "
            _G._done = false
            _G._buf = nil
            _G._kind = nil
            _G._mime = nil
            pmacs.async(function()
                local buf = _G.RES.open(_G.SERVER, 'mcp://text/doc.txt')
                _G._buf = buf
                local s = _G.RES.__pmacs_mcp_resources_test_state(buf)
                _G._kind = s.kind
                _G._mime = s.mimeType
                _G._body = buf:slice(0, buf:len())
                _G._done = true
            end)
            ",
        )
        .exec()
        .expect("open coroutine");
    assert!(
        pump_until_lua_pred(&mut state, "_G._done", Duration::from_secs(5)),
        "open coroutine did not complete"
    );
    let body: String = state
        .lua_host
        .lua()
        .load("return _G._body")
        .eval()
        .expect("read body");
    let kind: String = state
        .lua_host
        .lua()
        .load("return _G._kind")
        .eval()
        .expect("read kind");
    let mime: String = state
        .lua_host
        .lua()
        .load("return _G._mime")
        .eval()
        .expect("read mime");
    assert_eq!(kind, "text", "text/plain must render as kind=text");
    assert_eq!(mime, "text/plain");
    assert!(
        body.contains("initial doc body"),
        "buffer must contain server's text; got {body:?}"
    );
}

/// Bullet 1b: text/markdown renders. Same path as 1a but the
/// mimeType is `text/markdown`; the renderer treats text/* as text.
#[test]
fn m9_5_markdown_resource_renders_as_buffer() {
    let (mut state, _cache, _user_root) = editor_with_resources();
    spawn_initialized_server(&mut state);
    state
        .lua_host
        .lua()
        .load(
            "
            _G._done = false
            pmacs.async(function()
                local buf = _G.RES.open(_G.SERVER, 'mcp://text/readme.md')
                local s = _G.RES.__pmacs_mcp_resources_test_state(buf)
                _G._kind = s.kind
                _G._mime = s.mimeType
                _G._body = buf:slice(0, buf:len())
                _G._done = true
            end)
            ",
        )
        .exec()
        .expect("open coroutine");
    assert!(
        pump_until_lua_pred(&mut state, "_G._done", Duration::from_secs(5)),
        "open coroutine did not complete"
    );
    let kind: String = state
        .lua_host
        .lua()
        .load("return _G._kind")
        .eval()
        .expect("read kind");
    let mime: String = state
        .lua_host
        .lua()
        .load("return _G._mime")
        .eval()
        .expect("read mime");
    let body: String = state
        .lua_host
        .lua()
        .load("return _G._body")
        .eval()
        .expect("read body");
    assert_eq!(kind, "text");
    assert_eq!(mime, "text/markdown");
    assert!(body.contains("# Readme"));
}

// ===========================================================================
// Bullet 2: directory resources render as navigable buffers
// ===========================================================================

/// Bullet 2a: directory renders as a list of children, one per line.
#[test]
fn m9_5_directory_resource_renders_with_children_per_line() {
    let (mut state, _cache, _user_root) = editor_with_resources();
    spawn_initialized_server(&mut state);
    state
        .lua_host
        .lua()
        .load(
            "
            _G._done = false
            pmacs.async(function()
                local buf = _G.RES.open(_G.SERVER, 'mcp://dir/')
                local s = _G.RES.__pmacs_mcp_resources_test_state(buf)
                _G._kind = s.kind
                _G._children = s.children
                _G._body = buf:slice(0, buf:len())
                _G._done = true
            end)
            ",
        )
        .exec()
        .expect("open coroutine");
    assert!(
        pump_until_lua_pred(&mut state, "_G._done", Duration::from_secs(5)),
        "open coroutine did not complete"
    );
    let kind: String = state
        .lua_host
        .lua()
        .load("return _G._kind")
        .eval()
        .expect("kind");
    let children: Vec<String> = state
        .lua_host
        .lua()
        .load("return _G._children")
        .eval()
        .expect("children");
    let body: String = state
        .lua_host
        .lua()
        .load("return _G._body")
        .eval()
        .expect("body");
    assert_eq!(kind, "directory");
    assert_eq!(
        children,
        vec![
            "mcp://text/doc.txt".to_owned(),
            "mcp://text/readme.md".to_owned()
        ]
    );
    // Body has one URI per line.
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 2, "directory body should be one URI per line");
    assert_eq!(lines[0], "mcp://text/doc.txt");
    assert_eq!(lines[1], "mcp://text/readme.md");
}

/// Bullet 2b: opening a child via the test seam (which is what the
/// RET keymap binding ultimately calls) produces a buffer for that
/// child URI.
#[test]
fn m9_5_directory_open_child_at_line_navigates() {
    let (mut state, _cache, _user_root) = editor_with_resources();
    spawn_initialized_server(&mut state);
    state
        .lua_host
        .lua()
        .load(
            "
            _G._done = false
            pmacs.async(function()
                local dir_buf = _G.RES.open(_G.SERVER, 'mcp://dir/')
                local child_buf = _G.RES.open_child_at_line(dir_buf, 1)
                _G._child_kind = _G.RES.__pmacs_mcp_resources_test_state(child_buf).kind
                _G._child_body = child_buf:slice(0, child_buf:len())
                _G._done = true
            end)
            ",
        )
        .exec()
        .expect("open + navigate coroutine");
    assert!(
        pump_until_lua_pred(&mut state, "_G._done", Duration::from_secs(5)),
        "navigate coroutine did not complete"
    );
    let kind: String = state
        .lua_host
        .lua()
        .load("return _G._child_kind")
        .eval()
        .expect("child kind");
    let body: String = state
        .lua_host
        .lua()
        .load("return _G._child_body")
        .eval()
        .expect("child body");
    assert_eq!(kind, "text");
    assert!(body.contains("initial doc body"));
}

/// Regression: per-buffer `_state` must survive a fresh
/// `BufferIdLua` userdata wrapping.
///
/// `pmacs.window.buffer()` and `pmacs.buffer.list()` return fresh
/// userdata wrappings on every call. The `pmacs-mcp-resources.open-
/// at-point` command body — bound to RET on directory resource
/// buffers — fetches the active buffer via `pmacs.window.buffer()`
/// and looks up `_state[buf]`. When `_state` is keyed by userdata
/// identity (Lua tables hash userdata by identity, not `__eq`), the
/// fresh wrapping misses the entry stored at create time and the
/// command body returns silently — RET becomes a no-op.
///
/// This test reproduces the lookup that the binding's command body
/// performs. It opens a directory, then re-fetches the buffer
/// userdata via `pmacs.buffer.list()` (yielding a fresh wrapping)
/// and asks the package's state seam to resolve it. With the fix
/// (state keyed by `tostring(buf)`) the lookup returns the
/// directory state; without it, the lookup returns nil.
///
/// Acceptance harness uses the underscore-prefixed test seam rather
/// than `dispatch_key(Enter)` because the command body has a
/// separate cursor-indexing concern (`cursor_line()` is 0-based but
/// `child_uri_at_cursor` checks `line < 1`) that's outside this
/// test's scope.
#[test]
fn m9_5_state_lookup_survives_fresh_buffer_userdata() {
    let (mut state, _cache, _user_root) = editor_with_resources();
    spawn_initialized_server(&mut state);

    state
        .lua_host
        .lua()
        .load(
            "
            _G._opened = false
            pmacs.async(function()
                _G.DIR = _G.RES.open(_G.SERVER, 'mcp://dir/')
                _G._opened = true
            end)
            ",
        )
        .exec()
        .expect("open dir coroutine");
    assert!(
        pump_until_lua_pred(&mut state, "_G._opened", Duration::from_secs(5)),
        "directory open did not complete"
    );

    // Re-fetch the directory buffer's userdata via pmacs.buffer.list();
    // this yields a *fresh* BufferIdLua wrapping that is __eq to the
    // one stored at create time but has a different identity. A
    // userdata-keyed `_state` table cannot find the entry through it.
    let resolved: bool = state
        .lua_host
        .lua()
        .load(
            r#"
            for _, id in ipairs(pmacs.buffer.list()) do
                local d = pmacs.describe.buffer(id)
                if d ~= nil and d.name == "*mcp:mcp://dir/*" then
                    local s = _G.RES.__pmacs_mcp_resources_test_state(id)
                    return s ~= nil and s.kind == "directory"
                end
            end
            return false
            "#,
        )
        .eval()
        .expect("state resolution");
    assert!(
        resolved,
        "_state lookup via a fresh BufferIdLua wrapping must resolve \
         to the directory state; a miss here means `_state` is keyed \
         by userdata identity rather than `tostring(buf)`, and the \
         RET keybinding's command body would be a silent no-op."
    );
}

/// Regression: pressing RET on the first line of a directory resource
/// buffer opens the first child URI.
///
/// The buffer renders one child URI per line.
/// `pmacs.editor.cursor_line()` is 0-based, so the cursor on the first
/// displayed line returns 0; `child_uri_at_cursor` must map that to
/// `s.children[1]` (Lua arrays are 1-indexed). An earlier draft
/// rejected `line < 1` and indexed `s.children[line]`, which made RET
/// on line 0 a no-op and shifted every other line up by one.
///
/// Drives `dispatch_key(Enter)` rather than `pmacs.command.invoke` so
/// the buffer-scoped RET binding is exercised end-to-end (per the
/// buffer-scope keybinding test pattern: `pmacs.command.invoke`
/// bypasses dispatch and would let a dead binding pass).
#[test]
fn m9_5_directory_ret_keybinding_opens_first_child() {
    let (mut state, _cache, _user_root) = editor_with_resources();
    spawn_initialized_server(&mut state);

    state
        .lua_host
        .lua()
        .load(
            "
            _G._opened = false
            pmacs.async(function()
                _G.DIR = _G.RES.open(_G.SERVER, 'mcp://dir/')
                _G._opened = true
            end)
            ",
        )
        .exec()
        .expect("open dir coroutine");
    assert!(
        pump_until_lua_pred(&mut state, "_G._opened", Duration::from_secs(5)),
        "directory open did not complete"
    );

    // Switch the active window to the directory buffer so the
    // buffer-scoped RET binding is in scope and `pmacs.window.buffer()`
    // returns the directory buffer when the command body fires.
    // `switch_buffer` resets the cursor to (0, 0), placing it on the
    // first line — which renders the first child URI.
    state
        .lua_host
        .lua()
        .load("pmacs.window.switch_buffer(_G.DIR)")
        .exec()
        .expect("switch to directory buffer");

    state.dispatch_key(FrontendId::LOCAL, plain_key(KeyCode::Enter));

    // The RET binding kicks off a `pmacs.async` that calls M.open on
    // the child URI under the cursor. Pump until the
    // `*mcp:mcp://text/doc.txt*` buffer (the fake's first directory
    // entry) exists with the expected body.
    let opened = pump_until_lua_pred(
        &mut state,
        r#"(function()
            for _, id in ipairs(pmacs.buffer.list()) do
                local d = pmacs.describe.buffer(id)
                if d ~= nil and d.name == "*mcp:mcp://text/doc.txt*" then
                    local body = id:slice(0, id:len())
                    if body:find("initial doc body", 1, true) then
                        return true
                    end
                end
            end
            return false
        end)()"#,
        Duration::from_secs(5),
    );
    assert!(
        opened,
        "RET on the first line of a directory buffer must open the first \
         child URI (`mcp://text/doc.txt`). A miss here means \
         `child_uri_at_cursor` mishandled the 0-based `cursor_line()` \
         (e.g. `line < 1` filter or 1-based array index against a \
         0-based cursor)."
    );
}

// ===========================================================================
// Bullet 3: subscription-driven refresh
// ===========================================================================

/// Bullet 3: a subscribed resource buffer refreshes when the server
/// emits notifications/resources/updated. The fake's
/// `mcp_test/trigger_update { uri, new_text }` tool deterministically
/// drives the update.
#[test]
fn m9_5_subscribed_resource_refreshes_on_update() {
    let (mut state, _cache, _user_root) = editor_with_resources();
    spawn_initialized_server(&mut state);

    // Open the resource and verify the initial body lands.
    state
        .lua_host
        .lua()
        .load(
            "
            _G._opened = false
            pmacs.async(function()
                _G.BUF = _G.RES.open(_G.SERVER, 'mcp://text/doc.txt')
                _G._opened = true
            end)
            ",
        )
        .exec()
        .expect("open");
    assert!(
        pump_until_lua_pred(&mut state, "_G._opened", Duration::from_secs(5)),
        "open did not complete"
    );

    // Verify subscription was established (server supports it).
    let subscribed: bool = state
        .lua_host
        .lua()
        .load(
            "
            local s = _G.RES.__pmacs_mcp_resources_test_state(_G.BUF)
            return s.subscribed
            ",
        )
        .eval()
        .expect("read subscribed");
    assert!(
        subscribed,
        "buffer must be subscribed (server advertises it)"
    );

    // Trigger an update via tools/call.
    state
        .lua_host
        .lua()
        .load(
            "
            _G._triggered = false
            pmacs.async(function()
                pmacs.mcp.invoke_tool(_G.SERVER, 'mcp_test/trigger_update', {
                    uri = 'mcp://text/doc.txt',
                    new_text = 'updated body via trigger',
                }):await()
                _G._triggered = true
            end)
            ",
        )
        .exec()
        .expect("trigger update");
    assert!(
        pump_until_lua_pred(&mut state, "_G._triggered", Duration::from_secs(5)),
        "trigger_update did not complete"
    );

    // The notifications/resources/updated handler dispatches an
    // async refetch; pump until the buffer's body changes.
    let refreshed = pump_until_lua_pred(
        &mut state,
        "(_G.BUF:slice(0, _G.BUF:len())):find('updated body via trigger', 1, true) ~= nil",
        Duration::from_secs(5),
    );
    let body: String = state
        .lua_host
        .lua()
        .load("return _G.BUF:slice(0, _G.BUF:len())")
        .eval()
        .expect("body");
    assert!(
        refreshed,
        "buffer must refresh with new text after notifications/resources/updated; got {body:?}"
    );
}

// ===========================================================================
// Crash-during-subscription
// ===========================================================================

/// When a subscribed buffer's server transitions to non-Initialized
/// (crashed/stopped/etc.), the buffer is marked stale rather than
/// throwing errors on every tick. Last-known content stays in the
/// buffer.
#[test]
fn m9_5_server_crash_marks_subscribed_buffer_stale() {
    let (mut state, _cache, _user_root) = editor_with_resources();
    spawn_initialized_server(&mut state);

    state
        .lua_host
        .lua()
        .load(
            "
            _G._opened = false
            pmacs.async(function()
                _G.BUF = _G.RES.open(_G.SERVER, 'mcp://text/doc.txt')
                _G._opened = true
            end)
            ",
        )
        .exec()
        .expect("open");
    assert!(
        pump_until_lua_pred(&mut state, "_G._opened", Duration::from_secs(5)),
        "open did not complete"
    );

    // Capture the body before the crash.
    let body_before: String = state
        .lua_host
        .lua()
        .load("return _G.BUF:slice(0, _G.BUF:len())")
        .eval()
        .expect("body before crash");
    assert!(body_before.contains("initial doc body"));

    // Force-stop the server (transitions to ShuttingDown -> Stopped).
    state
        .lua_host
        .lua()
        .load("pmacs.mcp.stop(_G.SERVER)")
        .exec()
        .expect("stop");

    // Pump until the lifecycle watchdog observes the transition and
    // marks the buffer stale.
    let became_stale = pump_until_lua_pred(
        &mut state,
        "_G.RES.is_stale(_G.BUF)",
        Duration::from_secs(5),
    );
    assert!(became_stale, "is_stale must return true after server stop");

    // Buffer content is still the last-known body (no clearing).
    let body_after: String = state
        .lua_host
        .lua()
        .load("return _G.BUF:slice(0, _G.BUF:len())")
        .eval()
        .expect("body after crash");
    assert_eq!(body_after, body_before);
}

// ===========================================================================
// Lifecycle round-trip
// ===========================================================================

/// open → close → re-open. Verifies (a) close cleans up the
/// (server, uri) -> buffer registry, (b) re-open establishes a
/// fresh subscription and returns a (potentially) new buffer, not a
/// stale-handle reuse.
#[test]
fn m9_5_open_close_reopen_lifecycle() {
    let (mut state, _cache, _user_root) = editor_with_resources();
    spawn_initialized_server(&mut state);

    state
        .lua_host
        .lua()
        .load(
            "
            _G._round1 = false
            _G._round2 = false
            pmacs.async(function()
                -- Round 1: open + close.
                _G.BUF1 = _G.RES.open(_G.SERVER, 'mcp://text/doc.txt')
                local r1 = _G.RES.__pmacs_mcp_resources_test_buffer_for(
                    _G.SERVER:raw(), 'mcp://text/doc.txt')
                _G._round1_registry_present = (r1 ~= nil)
                _G.RES.close(_G.BUF1)
                local r2 = _G.RES.__pmacs_mcp_resources_test_buffer_for(
                    _G.SERVER:raw(), 'mcp://text/doc.txt')
                _G._round1_registry_after_close = (r2 ~= nil)
                _G._round1 = true
            end)
            pmacs.async(function()
                -- Wait for round 1 to finish before round 2.
                local stop = false
                while not stop do
                    if _G._round1 then stop = true
                    else pmacs.workers.sleep(0):await() end
                end
                -- Round 2: re-open.
                _G.BUF2 = _G.RES.open(_G.SERVER, 'mcp://text/doc.txt')
                local s = _G.RES.__pmacs_mcp_resources_test_state(_G.BUF2)
                _G._round2_subscribed = s.subscribed
                _G._round2_body = _G.BUF2:slice(0, _G.BUF2:len())
                _G._round2 = true
            end)
            ",
        )
        .exec()
        .expect("dispatch lifecycle coroutines");

    assert!(
        pump_until_lua_pred(&mut state, "_G._round2", Duration::from_secs(10)),
        "lifecycle coroutines did not complete"
    );

    let r1_present: bool = state
        .lua_host
        .lua()
        .load("return _G._round1_registry_present")
        .eval()
        .expect("r1 present");
    let r1_after_close: bool = state
        .lua_host
        .lua()
        .load("return _G._round1_registry_after_close")
        .eval()
        .expect("r1 after close");
    let r2_subscribed: bool = state
        .lua_host
        .lua()
        .load("return _G._round2_subscribed")
        .eval()
        .expect("r2 subscribed");
    let r2_body: String = state
        .lua_host
        .lua()
        .load("return _G._round2_body")
        .eval()
        .expect("r2 body");

    assert!(
        r1_present,
        "after open, registry should contain the (server, uri) -> buffer entry"
    );
    assert!(
        !r1_after_close,
        "after close, registry should not contain the (server, uri) -> buffer entry"
    );
    assert!(
        r2_subscribed,
        "after re-open, the new buffer should be subscribed afresh"
    );
    assert!(
        r2_body.contains("initial doc body"),
        "after re-open, the new buffer should have current content; got {r2_body:?}"
    );
}

// ===========================================================================
// Pass-2 finding 1: query-result/table renderer
// ===========================================================================

/// Pass-2 finding 1: a resource with the table MIME shape
/// (`application/vnd.pmacs.mcp.table+json`) renders as a column-
/// aligned text table. The fake's `mcp://table/users.tbl` resource
/// has columns `[name, age]` and three rows.
#[test]
fn m9_5_table_resource_renders_as_column_aligned_text() {
    let (mut state, _cache, _user_root) = editor_with_resources();
    spawn_initialized_server(&mut state);
    state
        .lua_host
        .lua()
        .load(
            "
            _G._done = false
            pmacs.async(function()
                local buf = _G.RES.open(_G.SERVER, 'mcp://table/users.tbl')
                local s = _G.RES.__pmacs_mcp_resources_test_state(buf)
                _G._kind = s.kind
                _G._mime = s.mimeType
                _G._body = buf:slice(0, buf:len())
                _G._done = true
            end)
            ",
        )
        .exec()
        .expect("open table");
    assert!(
        pump_until_lua_pred(&mut state, "_G._done", Duration::from_secs(5)),
        "open coroutine did not complete"
    );
    let kind: String = state
        .lua_host
        .lua()
        .load("return _G._kind")
        .eval()
        .expect("kind");
    let mime: String = state
        .lua_host
        .lua()
        .load("return _G._mime")
        .eval()
        .expect("mime");
    let body: String = state
        .lua_host
        .lua()
        .load("return _G._body")
        .eval()
        .expect("body");
    assert_eq!(kind, "table", "table mime must render as kind=table");
    assert_eq!(mime, "application/vnd.pmacs.mcp.table+json");
    // Body should have a header row, separator, and three data rows.
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(
        lines.len(),
        5,
        "expected header + sep + 3 rows = 5 lines; got {body:?}"
    );
    // Header should mention both column names.
    assert!(lines[0].contains("name") && lines[0].contains("age"));
    // Separator should have at least one `+`.
    assert!(lines[1].contains('+'));
    // Data rows: names AND numeric ages appear in order. Pass-3
    // finding 2: the fake encodes ages as bareword numbers, not
    // quoted strings, so the rendered output must include `30`,
    // `25`, `42` alongside the names — proving the parser handles
    // mixed string/number cells without dropping any.
    assert!(
        lines[2].contains("alice") && lines[2].contains("30"),
        "row 0 must show both name and numeric age; got {:?}",
        lines[2]
    );
    assert!(
        lines[3].contains("bob") && lines[3].contains("25"),
        "row 1 must show both name and numeric age; got {:?}",
        lines[3]
    );
    assert!(
        lines[4].contains("carol") && lines[4].contains("42"),
        "row 2 must show both name and numeric age; got {:?}",
        lines[4]
    );
    // Column alignment: every line has the same length (padded).
    let header_len = lines[0].len();
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(
            line.len(),
            header_len,
            "line {i} differs in length: {line:?} vs header {:?}",
            lines[0]
        );
    }
}

// ===========================================================================
// Pass-2 finding 2: open() registry leak on initial-fetch failure
// ===========================================================================

/// Pass-2 finding 2: when the initial `read_resource` fails, the
/// package must not leave a half-initialized buffer in its
/// registry. The next call against the same (server, uri) must
/// re-attempt the fetch from scratch — this is what makes retry
/// semantics work.
///
/// The fake's `mcp://error/test` URI returns a JSON-RPC error, so
/// the first `open()` raises. The second `open()` against the same
/// URI (after the fake's error path is no longer in play) succeeds.
/// We use the `mcp://text/doc.txt` URI for the retry to verify
/// that registry cleanup is per-URI, not global.
#[test]
fn m9_5_open_failure_cleans_registry_so_retry_works() {
    let (mut state, _cache, _user_root) = editor_with_resources();
    spawn_initialized_server(&mut state);

    state
        .lua_host
        .lua()
        .load(
            "
            _G._first_failed = false
            _G._first_err = nil
            _G._second_ok = false
            _G._round = 0
            pmacs.async(function()
                local ok, err = pcall(function()
                    return _G.RES.open(_G.SERVER, 'mcp://error/test')
                end)
                _G._first_failed = (not ok)
                _G._first_err = err
                _G._round = 1
                -- Verify the failed open() didn't leak a registry
                -- entry: open() against the same URI again must
                -- re-attempt (and fail again, with the same error).
                local ok2, err2 = pcall(function()
                    return _G.RES.open(_G.SERVER, 'mcp://error/test')
                end)
                _G._second_failed = (not ok2)
                _G._second_err = err2
                -- And a different URI must work — verifying that
                -- the failure didn't poison the package globally.
                local ok3 = pcall(function()
                    _G.SECOND_BUF = _G.RES.open(_G.SERVER, 'mcp://text/doc.txt')
                end)
                _G._third_ok = ok3
                _G._round = 2
            end)
            ",
        )
        .exec()
        .expect("dispatch coroutines");
    assert!(
        pump_until_lua_pred(&mut state, "_G._round == 2", Duration::from_secs(5)),
        "coroutine did not finish all three rounds"
    );

    let first_failed: bool = state
        .lua_host
        .lua()
        .load("return _G._first_failed")
        .eval()
        .expect("first_failed");
    let second_failed: bool = state
        .lua_host
        .lua()
        .load("return _G._second_failed")
        .eval()
        .expect("second_failed");
    let third_ok: bool = state
        .lua_host
        .lua()
        .load("return _G._third_ok")
        .eval()
        .expect("third_ok");

    assert!(
        first_failed,
        "first open() against mcp://error/test must fail (server returns -32602)"
    );
    assert!(
        second_failed,
        "second open() against mcp://error/test must also fail — \
         if registry was leaking, this would return a half-initialized buffer instead"
    );
    assert!(
        third_ok,
        "open() against a working URI after the failure path must succeed"
    );

    // Verify the registry has no entry for the failing URI.
    let registry_empty: bool = state
        .lua_host
        .lua()
        .load(
            "
            return _G.RES.__pmacs_mcp_resources_test_buffer_for(
                _G.SERVER:raw(), 'mcp://error/test') == nil
            ",
        )
        .eval()
        .expect("registry empty for error uri");
    assert!(
        registry_empty,
        "after failure, registry must not contain an entry for mcp://error/test"
    );
}

/// Pass-3 finding 1: the failed `open()` must remove the editor
/// buffer it created, not just its package-state bookkeeping.
/// Otherwise repeated failed opens leave dead `*mcp:...*` buffers
/// in `pmacs.buffer.list()`.
#[test]
fn m9_5_open_failure_does_not_leak_editor_buffers() {
    let (mut state, _cache, _user_root) = editor_with_resources();
    spawn_initialized_server(&mut state);

    // Snapshot the buffer count before any failed open.
    let baseline: usize = state
        .lua_host
        .lua()
        .load("return #pmacs.buffer.list()")
        .eval()
        .expect("baseline buffer count");

    // Fail the same URI five times. With the leak fix, each failure
    // cleans up the buffer; without it, each failure adds a dead
    // `*mcp:mcp://error/test*` to the list.
    state
        .lua_host
        .lua()
        .load(
            "
            _G._fail_done = false
            _G._failures = 0
            pmacs.async(function()
                for _ = 1, 5 do
                    local ok = pcall(function()
                        return _G.RES.open(_G.SERVER, 'mcp://error/test')
                    end)
                    if not ok then
                        _G._failures = _G._failures + 1
                    end
                end
                _G._fail_done = true
            end)
            ",
        )
        .exec()
        .expect("dispatch failures");
    assert!(
        pump_until_lua_pred(&mut state, "_G._fail_done", Duration::from_secs(5)),
        "failure coroutine did not finish"
    );

    let failures: i64 = state
        .lua_host
        .lua()
        .load("return _G._failures")
        .eval()
        .expect("failures count");
    assert_eq!(failures, 5, "all five opens should have failed");

    // Buffer count after the failures must equal baseline. A leak
    // would show 5 extra `*mcp:mcp://error/test*` buffers.
    let after: usize = state
        .lua_host
        .lua()
        .load("return #pmacs.buffer.list()")
        .eval()
        .expect("after buffer count");
    assert_eq!(
        after, baseline,
        "buffer count must not grow across failed open() calls; \
         baseline {baseline}, after 5 failures {after} \
         (leak suspected if after > baseline)"
    );
}

// ===========================================================================
// Pass-2 finding 3: stale buffer recovery on server restart
// ===========================================================================

/// Pass-2 finding 3: when a subscribed buffer is stale (server
/// transitioned away) and the user re-opens the same URI after
/// the server is back to Initialized, the existing buffer is
/// recovered: refetched + re-subscribed + stale flag cleared. The
/// caller gets the same buffer handle but with fresh content.
///
/// Uses `OnCrash` + `mcp_test/crash` to exercise the manager-driven
/// auto-restart path (which preserves the `McpServerId` across
/// generations); the test verifies that recovery uses the same
/// buffer handle, not a new one.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "linear setup-crash-restart-reopen sequence; splitting fragments the test's narrative"
)]
fn m9_5_stale_buffer_recovers_on_reopen_after_restart() {
    let (mut state, _cache, _user_root) = editor_with_resources();

    // Spawn with OnCrash + a tighter restart back-off so the test
    // doesn't have to wait the default 500ms.
    let fake = fake_mcp_path();
    state
        .lua_host
        .lua()
        .load(format!(
            "
            _G.SERVER = pmacs.mcp.spawn({{
                label = 'm9_5_recovery',
                command = '{fake}',
                restart = 'on_crash',
            }})
            ",
        ))
        .exec()
        .expect("spawn fake mcp");

    // Drain to first Initialized.
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

    // Open the resource and confirm subscribed state.
    state
        .lua_host
        .lua()
        .load(
            "
            _G._opened = false
            pmacs.async(function()
                _G.BUF = _G.RES.open(_G.SERVER, 'mcp://text/doc.txt')
                _G._opened = true
            end)
            ",
        )
        .exec()
        .expect("open");
    assert!(
        pump_until_lua_pred(&mut state, "_G._opened", Duration::from_secs(5)),
        "open did not complete"
    );
    let subscribed: bool = state
        .lua_host
        .lua()
        .load("return _G.RES.__pmacs_mcp_resources_test_state(_G.BUF).subscribed")
        .eval()
        .expect("subscribed");
    assert!(subscribed, "buffer must be subscribed before crash");

    // Trigger a crash via mcp_test/crash. The fake exits 78
    // before responding; the manager observes the crash, OnCrash
    // policy triggers an auto-restart with the SAME McpServerId.
    state
        .lua_host
        .lua()
        .load(
            "
            _G._crash_done = false
            pmacs.async(function()
                local ok = pcall(function()
                    pmacs.mcp.invoke_tool(_G.SERVER, 'mcp_test/crash', {}):await()
                end)
                _G._crash_done = true
                _G._crash_call_ok = ok
            end)
            ",
        )
        .exec()
        .expect("dispatch crash");
    // Wait for the buffer to be marked stale and for the manager
    // to come back to Initialized (auto-restart).
    let stale_observed = pump_until_lua_pred(
        &mut state,
        "_G.RES.is_stale(_G.BUF)",
        Duration::from_secs(5),
    );
    assert!(stale_observed, "buffer must be marked stale after crash");
    let restarted = pump_until_lua_pred(
        &mut state,
        "(function()
            for _, row in ipairs(pmacs.mcp.list()) do
                if row.id:raw() == _G.SERVER:raw() and row.state.kind == 'initialized' then
                    return true
                end
            end
            return false
        end)()",
        Duration::from_secs(10),
    );
    assert!(
        restarted,
        "manager must auto-restart and reach Initialized again"
    );

    // Re-open the same URI on the same SERVER handle. The package
    // should recover the existing (now-stale) buffer: refetch +
    // re-subscribe + clear stale.
    state
        .lua_host
        .lua()
        .load(
            "
            _G._reopened = false
            pmacs.async(function()
                local buf2 = _G.RES.open(_G.SERVER, 'mcp://text/doc.txt')
                _G._reopen_buf_same = (buf2 == _G.BUF)
                _G._reopen_subscribed = _G.RES.__pmacs_mcp_resources_test_state(buf2).subscribed
                _G._reopen_stale = _G.RES.is_stale(buf2)
                _G._reopen_body = buf2:slice(0, buf2:len())
                _G._reopened = true
            end)
            ",
        )
        .exec()
        .expect("reopen");
    assert!(
        pump_until_lua_pred(&mut state, "_G._reopened", Duration::from_secs(5)),
        "reopen did not complete"
    );

    let same_buffer: bool = state
        .lua_host
        .lua()
        .load("return _G._reopen_buf_same")
        .eval()
        .expect("buf same");
    let reopen_subscribed: bool = state
        .lua_host
        .lua()
        .load("return _G._reopen_subscribed")
        .eval()
        .expect("subscribed");
    let reopen_stale: bool = state
        .lua_host
        .lua()
        .load("return _G._reopen_stale")
        .eval()
        .expect("stale");
    let reopen_body: String = state
        .lua_host
        .lua()
        .load("return _G._reopen_body")
        .eval()
        .expect("body");

    assert!(
        same_buffer,
        "re-open must return the same buffer handle (recovery, not new buffer)"
    );
    assert!(
        reopen_subscribed,
        "after re-open, the buffer must be re-subscribed against the restarted server"
    );
    assert!(
        !reopen_stale,
        "after re-open, the stale flag must be cleared"
    );
    assert!(
        reopen_body.contains("initial doc body"),
        "after re-open, the buffer must have fresh content; got {reopen_body:?}"
    );
}
