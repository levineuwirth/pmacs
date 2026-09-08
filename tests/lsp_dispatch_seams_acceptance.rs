//! Arc 8 Stage 3a acceptance — LSP notification/response dispatch seams
//! and `pmacs.fs.canonicalize`.
//!
//! the archived lean4-mode framing Q#LN9 and Q#LN20, acceptance 29–34 plus
//! 34a/34b.
//!
//! This suite deliberately contains **no Lean content**.
//! `handle_server_requests` (`builtin/runtime/lsp.lua`) is the single
//! LSP event drain for every language in pmacs, so the change is
//! exercised through an already-shipped language driven against
//! `pmacs_fake_lsp`. A suite that reached the drain only through Lean
//! would understate the blast radius — the same reasoning that shaped
//! Stage 2's suite.
//!
//! Every fixture calls `pmacs.project.set_search_boundary` at its own
//! tempdir root, so a stray marker above the temp directory cannot make
//! a "markerless" case silently detected.

use std::path::{Path, PathBuf};

use pmacs::editor::EditorState;

fn exec(state: &EditorState, source: &str) {
    state.lua_host.lua().load(source.to_owned()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(state: &EditorState, source: &str) -> T {
    state.lua_host.lua().load(source.to_owned()).eval().unwrap()
}

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

/// A fresh editor with the shipped language configs cleared, so the only
/// server any test can spawn is the fake one it configures itself.
fn editor() -> EditorState {
    let state = EditorState::new_with_roots(&crate::iso::roots());
    exec(&state, "pmacs.lsp.config = {}");
    state
}

fn lua_str(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        Self { _dir: dir, root }
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn dir(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn bind(&self, state: &EditorState) {
        exec(
            state,
            &format!(
                "pmacs.project.set_search_boundary(\"{}\")",
                lua_str(&self.root)
            ),
        );
    }
}

fn configure(state: &EditorState, language: &str) {
    exec(
        state,
        &format!(
            "pmacs.lsp.config.{language} = {{ command = \"{}\" }}",
            fake_lsp_path()
        ),
    );
}

fn open(state: &EditorState, path: &Path) {
    exec(
        state,
        &format!("pmacs.buffer.find_or_open(\"{}\")", lua_str(path)),
    );
}

// --- readiness waits --------------------------------------------------------
//
// Each wait ticks the editor's frame order until its predicate holds or
// `ready::DEADLINE` passes, and reports the elapsed time and the last
// observed state when it does not (tests/common/ready.rs). The fixed
// `for _ in 0..8 { tick(); sleep(2ms) }` loop these replace bet 16 ms on the
// fake server's speed and lost that bet under load, with an assertion
// failure that named the result of the race and not the race.

/// Tick until exactly `n` servers are listed.
fn wait_count(state: &mut EditorState, n: i64) {
    ready::tick_until(
        state,
        &format!("{n} listed server(s)"),
        ready::DEADLINE,
        |s| {
            let count = server_count(s);
            if count == n {
                ready::Probe::Ready(())
            } else {
                ready::Probe::Pending(format!("{count} listed"))
            }
        },
    );
}

/// Tick until the active buffer has an attachment whose server has
/// initialized. A reuse assertion asks what the attach decided, so the
/// attachment is its readiness event; the server must also be
/// initialized, because the fixed settle this replaced gave the fake
/// server that time and three lean4 rows fail against a server still
/// starting. The server count is asserted afterwards.
fn wait_attached(state: &mut EditorState) {
    ready::tick_until(
        state,
        "an initialized attachment for the active buffer",
        ready::DEADLINE,
        |s| {
            let kind: String = eval(
                s,
                r#"
                local rec = pmacs.lsp.active_attachment()
                if not rec then return "none" end
                for _, srv in ipairs(pmacs.lsp.list()) do
                  if tostring(srv.id) == tostring(rec.server) then
                    return tostring(srv.state and srv.state.kind)
                  end
                end
                return "gone"
                "#,
            );
            if kind == "initialized" {
                ready::Probe::Ready(())
            } else {
                ready::Probe::Pending(format!("attachment state {kind}"))
            }
        },
    );
}

/// Tick until the Lua expression `source` evaluates to a value `accept`
/// approves of; returns that value.
fn wait_eval<T>(state: &mut EditorState, what: &str, source: &str, accept: impl Fn(&T) -> bool) -> T
where
    T: mlua::FromLuaMulti + std::fmt::Debug,
{
    ready::tick_until(state, what, ready::DEADLINE, |s| {
        let value: T = eval(s, source);
        if accept(&value) {
            ready::Probe::Ready(value)
        } else {
            ready::Probe::Pending(format!("{value:?}"))
        }
    })
}

/// A rust project with one file, an attached fake server, and the
/// probes below installed. Returns the opened file's path.
fn attached_rust(state: &mut EditorState, fx: &Fixture) -> PathBuf {
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let file = fx.write("proj/src/main.rs", "fn main() {}\nlet x = 1;\n");
    fx.bind(state);
    configure(state, "rust");
    open(state, &file);
    wait_eval::<String>(
        state,
        "the attachment initialized",
        r#"
        local rec = pmacs.lsp.active_attachment()
        if not rec then return "none" end
        for _, s in ipairs(pmacs.lsp.list()) do
          if tostring(s.id) == tostring(rec.server) then
            return tostring(s.state and s.state.kind)
          end
        end
        return "gone"
        "#,
        |kind| kind == "initialized",
    );
    file
}

/// The sid of the single live server, as a Lua expression fragment.
const THE_SID: &str = "pmacs.lsp.list()[1].id";

// ---------------------------------------------------------------------------
// Acceptance 29 — a notification reaches a registered subscriber.
// ---------------------------------------------------------------------------

#[test]
fn acc29_notification_reaches_a_registered_subscriber() {
    let fx = Fixture::new();
    let mut state = editor();
    // Registered BEFORE the open, so the didOpen-triggered `pmacs/echo`
    // is in the first drain.
    exec(
        &state,
        r#"
        _G.seen = {}
        pmacs.lsp.on_notification("pmacs/echo", function(sid, params)
          _G.seen[#_G.seen + 1] = tostring(params and params.uri)
        end)
        "#,
    );
    attached_rust(&mut state, &fx);

    let n = wait_eval::<i64>(
        &mut state,
        "the first pmacs/echo notification",
        "return #_G.seen",
        |n| *n >= 1,
    );
    assert!(
        n >= 1,
        "expected at least one pmacs/echo notification, got {n}"
    );
    let first: String = eval(&state, "return _G.seen[1]");
    assert!(
        first.starts_with("file://") && first.ends_with("main.rs"),
        "subscriber got the document uri; saw {first:?}"
    );
}

#[test]
fn acc29_subscriber_for_an_unsent_method_does_not_fire() {
    let fx = Fixture::new();
    let mut state = editor();
    exec(
        &state,
        r#"
        _G.hits = 0
        pmacs.lsp.on_notification("pmacs/never", function() _G.hits = _G.hits + 1 end)
        "#,
    );
    attached_rust(&mut state, &fx);

    // Non-vacuity for acc29: the seam is method-keyed, not a firehose.
    // Without this, a subscriber invoked for every notification would
    // pass the test above while being wrong.
    let hits: i64 = eval(&state, "return _G.hits");
    assert_eq!(hits, 0, "a subscriber must only fire for its own method");
}

// ---------------------------------------------------------------------------
// Acceptance 30 + 33 — dispatch integrity: with subscribers registered,
// a `workspace/applyEdit` request in the same drain is still handled.
//
// The fake server writes the applyEdit request and the executeCommand
// response back to back, so both land in one `events_take` batch. That
// co-occurrence is the point: a seam that consumed the batch, or that
// returned early, would starve the `request` arms that share it.
// ---------------------------------------------------------------------------

fn drive_apply_edit(state: &mut EditorState, file: &Path) {
    exec(
        state,
        &format!(
            r#"
            local sid = {THE_SID}
            local uri = "file://{}"
            _G.rid = pmacs.lsp.send_request(sid, "workspace/executeCommand", {{
              command = "pmacs.fake.applyEdit",
              arguments = {{ uri }},
            }})
            _G.response_hits = 0
            pmacs.lsp.on_response(sid, _G.rid, function(result, err)
              _G.response_hits = _G.response_hits + 1
            end)
            "#,
            lua_str(file)
        ),
    );
    wait_eval::<i64>(
        state,
        "the executeCommand response",
        "return _G.response_hits",
        |hits| *hits >= 1,
    );
    ready::tick_until(
        state,
        "the applied edit in the buffer",
        ready::DEADLINE,
        |s| {
            let text = buffer_text(s);
            if text.contains("ED2") {
                ready::Probe::Ready(())
            } else {
                ready::Probe::Pending(text)
            }
        },
    );
}

fn buffer_text(state: &EditorState) -> String {
    eval(
        state,
        "local b = pmacs.window.buffer() return b:slice(0, b:len())",
    )
}

#[test]
fn acc30_apply_edit_still_handled_with_a_notification_subscriber() {
    let fx = Fixture::new();
    let mut state = editor();
    exec(
        &state,
        r#"
        _G.notes = 0
        pmacs.lsp.on_notification("pmacs/echo", function() _G.notes = _G.notes + 1 end)
        "#,
    );
    let file = attached_rust(&mut state, &fx);
    wait_eval::<i64>(
        &mut state,
        "the notification subscriber firing",
        "return _G.notes",
        |notes| *notes >= 1,
    );

    drive_apply_edit(&mut state, &file);

    assert!(
        buffer_text(&state).contains("ED2"),
        "workspace/applyEdit must still be applied with a subscriber \
         registered; buffer was {:?}",
        buffer_text(&state)
    );
}

#[test]
fn acc33_apply_edit_still_handled_with_a_response_subscriber() {
    let fx = Fixture::new();
    let mut state = editor();
    let file = attached_rust(&mut state, &fx);
    drive_apply_edit(&mut state, &file);

    // Both halves in one drain: the response was delivered to its
    // one-shot AND the server-originated request was serviced.
    assert_eq!(
        eval::<i64>(&state, "return _G.response_hits"),
        1,
        "the executeCommand response reaches its one-shot"
    );
    assert!(
        buffer_text(&state).contains("ED2"),
        "workspace/applyEdit must still be applied with a response \
         subscriber registered; buffer was {:?}",
        buffer_text(&state)
    );
}

// ---------------------------------------------------------------------------
// Acceptance 31 — a raising subscriber does not stop later events in the
// same drain (and does not stop the `request` arms either).
// ---------------------------------------------------------------------------

#[test]
fn acc31_raising_notification_subscriber_does_not_stop_the_drain() {
    let fx = Fixture::new();
    let mut state = editor();
    exec(
        &state,
        r#"
        _G.second_hits = 0
        pmacs.lsp.on_notification("pmacs/echo", function()
          error("subscriber blew up")
        end)
        pmacs.lsp.on_notification("pmacs/echo", function()
          _G.second_hits = _G.second_hits + 1
        end)
        "#,
    );
    let file = attached_rust(&mut state, &fx);
    wait_eval::<i64>(
        &mut state,
        "the second subscriber firing past a raising one",
        "return _G.second_hits",
        |hits| *hits >= 1,
    );

    // And the shared `request` arms still run in a later drain.
    drive_apply_edit(&mut state, &file);
    assert!(
        buffer_text(&state).contains("ED2"),
        "a raising subscriber must not stop workspace/applyEdit"
    );
}

#[test]
fn acc33_raising_response_handler_does_not_stop_the_drain() {
    let fx = Fixture::new();
    let mut state = editor();
    let file = attached_rust(&mut state, &fx);
    exec(
        &state,
        &format!(
            r#"
            local sid = {THE_SID}
            _G.notes_after = 0
            pmacs.lsp.on_notification("pmacs/echo", function()
              _G.notes_after = _G.notes_after + 1
            end)
            local rid = pmacs.lsp.send_request(sid, "workspace/executeCommand", {{
              command = "pmacs.fake.applyEdit",
              arguments = {{ "file://{}" }},
            }})
            pmacs.lsp.on_response(sid, rid, function() error("handler blew up") end)
            "#,
            lua_str(&file)
        ),
    );
    ready::tick_until(
        &mut state,
        "the applied edit in the buffer",
        ready::DEADLINE,
        |s| {
            let text = buffer_text(s);
            if text.contains("ED2") {
                ready::Probe::Ready(())
            } else {
                ready::Probe::Pending(text)
            }
        },
    );

    assert!(
        buffer_text(&state).contains("ED2"),
        "a raising response handler must not stop workspace/applyEdit in \
         the same drain"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 32 — the one-shot is removed exactly once, whether or not
// the handler raises.
//
// Named for what it pins rather than for the framing's wording. Q#LN9
// specifies removal *before* invocation, and the implementation does
// that — but bite-testing showed the before/after ordering is not
// observable on its own: `pcall` catches the raise either way, so
// removal after the call is behaviorally identical unless a handler
// re-enters the drain, which nothing does. What IS observable, and what
// this pins, is that removal is **unconditional**: the bite that moves
// it inside `if ok then` fails here 2 != 1, because the surviving
// registration gets invoked a second time by the purge.
// ---------------------------------------------------------------------------

#[test]
fn acc32_response_one_shot_is_removed_even_when_the_handler_raises() {
    let fx = Fixture::new();
    let mut state = editor();
    attached_rust(&mut state, &fx);
    exec(
        &state,
        &format!(
            r#"
            local sid = {THE_SID}
            _G.calls = 0
            local rid = pmacs.lsp.send_request(sid, "test/ping", {{ v = 1 }})
            pmacs.lsp.on_response(sid, rid, function(result, err)
              _G.calls = _G.calls + 1
              error("handler raises after being removed")
            end)
            "#
        ),
    );
    wait_eval::<i64>(
        &mut state,
        "the one-shot reply",
        "return _G.calls",
        |calls| *calls >= 1,
    );
    assert_eq!(
        eval::<i64>(&state, "return _G.calls"),
        1,
        "the one-shot fires exactly once for its reply"
    );

    exec(&state, &format!("pmacs.lsp.stop({THE_SID})"));
    wait_eval::<bool>(
        &mut state,
        "the stopped server",
        r#"
        for _, s in ipairs(pmacs.lsp.list()) do
          local k = s.state and s.state.kind
          if k == "stopped" or k == "crashed" then return true end
        end
        return false
        "#,
        |stopped| *stopped,
    );
    assert_eq!(
        eval::<i64>(&state, "return _G.calls"),
        1,
        "a delivered one-shot must not be re-invoked by the purge — \
         removal is unconditional, not gated on a clean return"
    );
}

#[test]
fn acc32_response_carries_the_servers_result() {
    let fx = Fixture::new();
    let mut state = editor();
    attached_rust(&mut state, &fx);
    exec(
        &state,
        &format!(
            r#"
            local sid = {THE_SID}
            _G.echoed = nil
            _G.saw_err = "unset"
            local rid = pmacs.lsp.send_request(sid, "test/ping", {{ v = 42 }})
            pmacs.lsp.on_response(sid, rid, function(result, err)
              _G.echoed = result and result.echo and result.echo.v
              _G.saw_err = tostring(err)
            end)
            "#
        ),
    );
    wait_eval::<i64>(
        &mut state,
        "the echoed result",
        "return _G.echoed or -1",
        |v| *v != -1,
    );

    // Non-vacuity: without this the seam could "fire" with nil payloads
    // and every count-based assertion above would still pass.
    assert_eq!(
        eval::<i64>(&state, "return _G.echoed or -1"),
        42,
        "the handler receives the server's result payload"
    );
    assert_eq!(
        eval::<String>(&state, "return _G.saw_err"),
        "nil",
        "a successful reply passes nil for err"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 34 — the pending purge, driven off `pmacs.lsp.list()` and
// NOT off a death event seen in the drain.
//
// The second test is the load-bearing one. `handle_server_requests`
// builds its sid list from `attachments`, so a server that is in no
// attachment is never drained — and its `stopped` event is therefore
// never seen. A purge wired to that event leaks exactly there.
// ---------------------------------------------------------------------------

#[test]
fn acc34_purge_settles_a_pending_one_shot_when_the_server_dies() {
    let fx = Fixture::new();
    let mut state = editor();
    attached_rust(&mut state, &fx);
    exec(
        &state,
        &format!(
            r#"
            local sid = {THE_SID}
            _G.err_msg = "never called"
            -- A method the fake server answers only after a delay would
            -- be ideal; instead the server is stopped in the same breath,
            -- so the reply can never arrive.
            local rid = pmacs.lsp.send_request(sid, "test/slow", {{}})
            pmacs.lsp.on_response(sid, rid, function(result, err)
              _G.err_msg = tostring(err and err.message)
            end)
            pmacs.lsp.stop(sid)
            "#
        ),
    );
    wait_eval::<String>(
        &mut state,
        "the settled one-shot",
        "return _G.err_msg",
        |m| m != "never called",
    );

    let msg: String = eval(&state, "return _G.err_msg");
    assert!(
        msg.contains("server gone") || msg == "nil",
        "a pending one-shot must be settled, not left waiting; saw {msg:?}"
    );
    assert_ne!(
        msg, "never called",
        "the one-shot was never settled — it leaked"
    );
}

#[test]
fn acc34_purge_reaches_a_server_that_is_in_no_attachment() {
    let fx = Fixture::new();
    let mut state = editor();
    fx.bind(&state);
    // Spawned directly, never attached to a buffer. `attachments` is
    // empty, so `handle_server_requests` never visits this sid and its
    // `stopped` event is never drained.
    exec(
        &state,
        &format!(
            r#"
            _G.settled = "never called"
            local sid = pmacs.lsp.spawn({{
              label = "orphan",
              language_id = "rust",
              command = "{}",
              args = {{}},
            }})
            _G.orphan = sid
            "#,
            fake_lsp_path()
        ),
    );
    wait_eval::<String>(
        &mut state,
        "the orphan server initialized",
        r#"
        for _, s in ipairs(pmacs.lsp.list()) do
          if tostring(s.id) == tostring(_G.orphan) then
            return tostring(s.state and s.state.kind)
          end
        end
        return "gone"
        "#,
        |kind| kind == "initialized",
    );

    exec(
        &state,
        r#"
        local rid = pmacs.lsp.send_request(_G.orphan, "test/slow", {})
        pmacs.lsp.on_response(_G.orphan, rid, function(result, err)
          _G.settled = tostring(err and err.message)
        end)
        pmacs.lsp.stop(_G.orphan)
        "#,
    );
    wait_eval::<String>(
        &mut state,
        "the purged one-shot",
        "return _G.settled",
        |m| m != "never called",
    );

    let settled: String = eval(&state, "return _G.settled");
    assert_ne!(
        settled, "never called",
        "the purge must not depend on the drain reaching this server — \
         it is in no attachment, so the drain never does"
    );
    assert!(
        settled.contains("server gone"),
        "settled with the purge's error; saw {settled:?}"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 34a — `pmacs.fs.canonicalize` (Q#LN20).
// ---------------------------------------------------------------------------

#[test]
#[cfg(unix)]
fn acc34a_canonicalize_resolves_symlinks_and_dot_segments() {
    let fx = Fixture::new();
    fx.write("pkg/sub/a.txt", "x\n");
    // Built here rather than assumed: the whole point is the symlink.
    std::os::unix::fs::symlink(fx.dir("pkg"), fx.dir("linkpkg")).unwrap();
    let state = editor();

    let noncanon = format!("{}/sub/./../sub/a.txt", fx.dir("linkpkg").display());
    let got: String = eval(
        &state,
        &format!("return tostring(pmacs.fs.canonicalize(\"{noncanon}\"))"),
    );
    let want = fx.root.join("pkg/sub/a.txt").display().to_string();
    assert_eq!(got, want, "symlink and dot segments both resolved");

    // Falsification for 34b: the uncanonicalized spelling really is
    // different, so the affinity test below is not vacuous.
    assert_ne!(noncanon, want);
}

#[test]
fn acc34a_canonicalize_returns_nil_for_a_missing_path() {
    let fx = Fixture::new();
    let state = editor();
    let missing = fx.dir("nope/not-here").display().to_string();
    let got: String = eval(
        &state,
        &format!("return tostring(pmacs.fs.canonicalize(\"{missing}\"))"),
    );
    assert_eq!(
        got, "nil",
        "a nonexistent path declines rather than raising"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 34b — affinity survives a symlinked open.
//
// Asserted at the affinity layer, not just at the binding: the
// regression Q#LN20 exists to prevent is *two servers for one project*,
// and only this shape observes it.
// ---------------------------------------------------------------------------

fn server_count(state: &EditorState) -> i64 {
    eval(state, "return #pmacs.lsp.list()")
}

#[test]
#[cfg(unix)]
fn acc34b_canonicalizing_resolver_reuses_one_server_across_a_symlink() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let real = fx.write("proj/src/main.rs", "fn main() {}\n");
    std::os::unix::fs::symlink(fx.dir("proj"), fx.dir("linkproj")).unwrap();
    let linked = fx.dir("linkproj").join("src/main.rs");

    let mut state = editor();
    fx.bind(&state);
    exec(
        &state,
        &format!(
            r#"
            pmacs.lsp.config.rust = {{
              command = "{}",
              root = function(path)
                local dir = path:match("^(.*)/[^/]*$")
                if not dir then return nil end
                -- Walk up to the directory holding Cargo.toml, then
                -- canonicalize — the Q#LN8 shape Stage 3b will use.
                while dir and #dir > 0 do
                  local f = io.open(dir .. "/Cargo.toml", "r")
                  if f then
                    f:close()
                    return pmacs.fs.canonicalize(dir)
                  end
                  dir = dir:match("^(.*)/[^/]*$")
                end
                return nil
              end,
            }}
            "#,
            fake_lsp_path()
        ),
    );

    open(&state, &real);
    wait_count(&mut state, 1);
    assert_eq!(server_count(&state), 1, "the real path spawns one server");

    open(&state, &linked);
    wait_attached(&mut state);
    assert_eq!(
        server_count(&state),
        1,
        "the symlinked path must reuse the same server — two here is the \
         exact regression Q#LN20 exists to prevent"
    );
}

#[test]
#[cfg(unix)]
fn acc34b_falsified_by_a_resolver_that_skips_canonicalization() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let real = fx.write("proj/src/main.rs", "fn main() {}\n");
    std::os::unix::fs::symlink(fx.dir("proj"), fx.dir("linkproj")).unwrap();
    let linked = fx.dir("linkproj").join("src/main.rs");

    let mut state = editor();
    fx.bind(&state);
    // Same resolver, minus the canonicalize call. This is the bite: if
    // it also produced one server, the test above would be vacuous and
    // `pmacs.fs.canonicalize` would be doing nothing.
    exec(
        &state,
        &format!(
            r#"
            pmacs.lsp.config.rust = {{
              command = "{}",
              root = function(path)
                local dir = path:match("^(.*)/[^/]*$")
                while dir and #dir > 0 do
                  local f = io.open(dir .. "/Cargo.toml", "r")
                  if f then f:close() return dir end
                  dir = dir:match("^(.*)/[^/]*$")
                end
                return nil
              end,
            }}
            "#,
            fake_lsp_path()
        ),
    );

    open(&state, &real);
    wait_count(&mut state, 1);
    open(&state, &linked);
    wait_count(&mut state, 2);
    assert_eq!(
        server_count(&state),
        2,
        "without canonicalization the two spellings key differently and \
         spawn two servers — this is what 34b's positive case rules out"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 34a, non-UTF-8 arm — an unrepresentable resolution declines
// rather than returning a lossy string.
//
// Review finding on PR #167: `display().to_string()` substitutes U+FFFD,
// which would hand back a path that does not exist on disk. That is
// strictly worse than nil here, because the value becomes a
// server-affinity key via `file_uri_for` and would silently fail to
// round-trip. Bites against the `display()` form, which returns a
// non-nil string for this fixture.
//
// **Linux-gated, and `cfg(unix)` was not enough** — CI caught that.
// APFS enforces valid UTF-8 in filenames, so on macOS the `write` below
// fails with EILSEQ ("Illegal byte sequence") before the code under test
// is ever reached: the fixture cannot be built there. That is a
// filesystem refusing to represent the case, not a behavioral
// difference — the subject itself, `to_str()` returning None, is
// platform-independent Rust. Gated explicitly rather than skipped at
// runtime, so a future failure here is a real failure and not a silent
// no-op.
// ---------------------------------------------------------------------------

#[test]
#[cfg(target_os = "linux")]
fn acc34a_canonicalize_declines_a_non_utf8_resolution() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let fx = Fixture::new();
    // 0xFF is not valid UTF-8 in any position.
    let raw = OsStr::from_bytes(b"bad-\xffname");
    let target = fx.root.join(raw);
    std::fs::write(&target, "x\n").unwrap();
    // Reached through an ASCII symlink, so the *input* is representable
    // and only the resolved output is not — which is the case
    // `to_str()` has to catch and a UTF-8-only input check would miss.
    let link = fx.dir("ascii-link");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let state = editor();
    let got: String = eval(
        &state,
        &format!(
            "return tostring(pmacs.fs.canonicalize(\"{}\"))",
            lua_str(&link)
        ),
    );
    assert_eq!(
        got, "nil",
        "a resolution that lands on non-UTF-8 bytes must decline, not \
         return a U+FFFD-substituted path that exists nowhere"
    );
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
#[path = "common/ready.rs"]
mod ready;
