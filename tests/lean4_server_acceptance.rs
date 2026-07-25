//! Arc 8 Stage 3b acceptance — the Lean 4 language server.
//!
//! `docs/lean4-mode-framing.md` Q#LN7, Q#LN8, Q#LN16; acceptance 22–28,
//! 24a/24b, 35, 36, 36a, 37.
//!
//! No live toolchain required. The server side is `pmacs_fake_lsp`
//! configured under the `lean4` language id; the probe and latch are
//! driven through shell stubs the fixture writes, so nothing here needs
//! `lake`, `lean`, or an elan toolchain on PATH (§2.9).
//!
//! Every fixture sets `pmacs.project.set_search_boundary` at its own
//! tempdir root. Without it the `lean-toolchain` walk climbs to the
//! filesystem root and acceptance 23's outermost assertion stops being
//! hermetic.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

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

    fn mkdir(&self, rel: &str) -> PathBuf {
        let path = self.root.join(rel);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn dir(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// A `lean-toolchain` marker file. Content is irrelevant to the
    /// resolver by design (existence semantics), which 24b pins.
    fn toolchain(&self, rel_dir: &str, body: &str) {
        self.write(&format!("{rel_dir}/lean-toolchain"), body);
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

/// A fresh editor with every shipped language config cleared, then the
/// `lean4` entry rebuilt against the fake server while KEEPING the real
/// resolver. That combination is the point: the root rule under test is
/// production code, only the command is a stand-in.
fn editor(fx: &Fixture) -> EditorState {
    let state = EditorState::new();
    exec(&state, "pmacs.lsp.config = {}");
    exec(
        &state,
        &format!(
            r#"
            pmacs.lsp.config.lean4 = {{
              command = "{}",
              args = {{}},
              root = pmacs.lean.root_for,
            }}
            "#,
            fake_lsp_path()
        ),
    );
    fx.bind(&state);
    state
}

fn settle(state: &mut EditorState) {
    for _ in 0..10 {
        state.tick_processes();
        state.tick_lsp();
        state.tick_async();
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn open(state: &EditorState, path: &Path) {
    exec(
        state,
        &format!("pmacs.buffer.find_or_open(\"{}\")", lua_str(path)),
    );
}

/// `language_id|root_uri|cwd` for every live server, sorted.
fn rows(state: &EditorState) -> Vec<String> {
    let joined: String = eval(
        state,
        r#"
        local out = {}
        for _, s in ipairs(pmacs.lsp.list()) do
          out[#out + 1] = table.concat({
            s.language_id or "", s.root_uri or "", s.cwd or "",
          }, "|")
        end
        table.sort(out)
        return table.concat(out, "\n")
        "#,
    );
    if joined.is_empty() {
        Vec::new()
    } else {
        joined.lines().map(str::to_owned).collect()
    }
}

fn resolved_root(state: &EditorState, file: &Path) -> String {
    eval(
        state,
        &format!(
            "return tostring(pmacs.lean.root_for(\"{}\"))",
            lua_str(file)
        ),
    )
}

// ---------------------------------------------------------------------------
// Acceptance 22 — a Lean file in a Lake package spawns one server rooted
// at the package.
// ---------------------------------------------------------------------------

#[test]
fn acc22_lean_file_in_a_lake_package_spawns_one_server_at_the_package_root() {
    let fx = Fixture::new();
    fx.toolchain("pkg", "leanprover/lean4:v4.9.0\n");
    let file = fx.write("pkg/Pkg/Basic.lean", "def x : Nat := 1\n");
    let mut state = editor(&fx);
    open(&state, &file);
    settle(&mut state);

    let pkg = fx.dir("pkg").display().to_string();
    assert_eq!(
        rows(&state),
        vec![format!("lean4|file://{pkg}|{pkg}")],
        "one server, rooted and cwd'd at the Lake package"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 23 — outermost wins.
//
// The case `pmacs.project.detect` cannot express: it is innermost-wins by
// construction, so a dependency vendored under `.lake/packages` would get
// its own server and its own (wrong) view of the world.
// ---------------------------------------------------------------------------

#[test]
fn acc23_nested_toolchains_resolve_to_the_outermost_package() {
    let fx = Fixture::new();
    fx.toolchain("pkg", "leanprover/lean4:v4.9.0\n");
    fx.toolchain("pkg/.lake/packages/dep", "leanprover/lean4:v4.8.0\n");
    let inner = fx.write(
        "pkg/.lake/packages/dep/Dep/Core.lean",
        "def dep : Nat := 2\n",
    );
    let state = editor(&fx);

    assert_eq!(
        resolved_root(&state, &inner),
        fx.dir("pkg").display().to_string(),
        "a file under .lake/packages/dep belongs to the outer package"
    );
    // Non-vacuity: the inner marker really exists, so "outermost" is a
    // choice between two candidates rather than the only one found.
    assert!(fx.dir("pkg/.lake/packages/dep/lean-toolchain").exists());
}

// ---------------------------------------------------------------------------
// Acceptance 24 — the walk stops at the search boundary.
// ---------------------------------------------------------------------------

#[test]
fn acc24_walk_stops_at_the_search_boundary() {
    let fx = Fixture::new();
    // Boundary is the fixture root; this marker sits INSIDE it.
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    // And this one sits AT the fixture root, i.e. above `pkg` but still
    // within the boundary — it must win, being outermost.
    fx.toolchain(".", "v4.7.0\n");
    let state = editor(&fx);
    assert_eq!(
        resolved_root(&state, &file),
        fx.root.display().to_string(),
        "within the boundary, the outermost marker wins"
    );

    // Now move the boundary IN to `pkg`. The root-level marker is above
    // it and must not be reached.
    exec(
        &state,
        &format!(
            "pmacs.project.set_search_boundary(\"{}\")",
            lua_str(&fx.dir("pkg"))
        ),
    );
    assert_eq!(
        resolved_root(&state, &file),
        fx.dir("pkg").display().to_string(),
        "a marker above the boundary is not consulted"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 24a / 24b — the marker test, both directions.
//
// These two must each fail against the implementation that satisfies only
// the other. 24a bites the bare `io.open` truth test (which succeeds on a
// directory); 24b bites the read-a-byte-and-require-non-nil rule (which
// rejects an empty file at EOF).
// ---------------------------------------------------------------------------

#[test]
fn acc24a_a_lean_toolchain_directory_is_not_a_marker() {
    let fx = Fixture::new();
    fx.mkdir("pkg/lean-toolchain");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let state = editor(&fx);
    assert_eq!(
        resolved_root(&state, &file),
        "nil",
        "a `lean-toolchain` DIRECTORY must not mark a root"
    );
}

#[test]
fn acc24b_an_empty_lean_toolchain_file_is_a_marker() {
    let fx = Fixture::new();
    fx.toolchain("pkg", "");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let state = editor(&fx);
    assert_eq!(
        resolved_root(&state, &file),
        fx.dir("pkg").display().to_string(),
        "marker semantics are existence, not content — an empty \
         `lean-toolchain` still marks the package"
    );
    // Non-vacuity: the file really is empty.
    assert_eq!(
        std::fs::read(fx.dir("pkg/lean-toolchain")).unwrap().len(),
        0
    );
}

#[test]
fn acc24_resolver_declines_when_no_marker_exists() {
    let fx = Fixture::new();
    let file = fx.write("loose/A.lean", "def a := 1\n");
    let state = editor(&fx);
    assert_eq!(
        resolved_root(&state, &file),
        "nil",
        "no marker anywhere is a decline, which falls through to \
         `pmacs.project.detect`"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 25 — a string-valued root still works.
// ---------------------------------------------------------------------------

#[test]
fn acc25_string_valued_root_still_works() {
    let fx = Fixture::new();
    let pkg = fx.mkdir("elsewhere");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let mut state = editor(&fx);
    exec(
        &state,
        &format!("pmacs.lsp.config.lean4.root = \"{}\"", lua_str(&pkg)),
    );
    open(&state, &file);
    settle(&mut state);

    let want = pkg.display().to_string();
    assert_eq!(
        rows(&state),
        vec![format!("lean4|file://{want}|{want}")],
        "the Q#LN8 generalization is additive; a plain string still wins"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 26 — didOpen carries languageId = "lean4".
// ---------------------------------------------------------------------------

#[test]
fn acc26_did_open_carries_the_lean4_language_id() {
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let mut state = editor(&fx);
    open(&state, &file);
    settle(&mut state);

    let lang: String = eval(
        &state,
        "return tostring(pmacs.lsp.list()[1] and pmacs.lsp.list()[1].language_id)",
    );
    assert_eq!(
        lang, "lean4",
        "the grammar entry name is the didOpen language id (Q#LN2)"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 27 / 28 / 35 / 36 — the probe and the fallback latch.
//
// **Driven through the production path**, not by calling internals.
// Round 1's versions poked `_fire_latch` directly and asserted on config
// mutation, which proved nothing about whether a server ever starts —
// and acceptance 36 went further and asserted every server was terminal,
// pinning the ABSENCE of the fallback it claimed to test. These go
// `buffer.after-load` -> ticks -> probe drain -> latch -> re-attach, and
// assert the originally opened buffer ends up on a LIVE server.
//
// The stubs are real executables the fixture writes. `M.fallback` is a
// table precisely so it can point at `pmacs_fake_lsp` here.
// ---------------------------------------------------------------------------

impl Fixture {
    /// An executable shell stub. `serve` sleeps (so the "server" does not
    /// die and only the named failure mode is under test); `--version`
    /// prints `version_line`.
    fn lake_stub(&self, rel: &str, version_line: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo '{version_line}'\n  exit 0\nfi\nexec sleep 300\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}

/// Point `command` at `lake_cmd` and the latch's fallback at the fake
/// LSP server, so a fallback that fires produces a server that works.
fn with_fallback(state: &EditorState, lake_cmd: &Path) {
    exec(
        state,
        &format!(
            r#"
            pmacs.lsp.config.lean4.command = "{}"
            pmacs.lsp.config.lean4.args = {{ "serve" }}
            pmacs.lean._fallback = {{ command = "{}", args = {{}} }}
            "#,
            lua_str(lake_cmd),
            fake_lsp_path()
        ),
    );
}

/// The active buffer's attached server id, or "none".
fn attached_sid(state: &EditorState) -> String {
    eval(
        state,
        r#"
        local rec = pmacs.lsp.active_attachment()
        return rec and tostring(rec.server) or "none"
        "#,
    )
}

/// State kind of the active buffer's attached server, or "none".
fn attached_state(state: &EditorState) -> String {
    eval(
        state,
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
    )
}

#[test]
fn acc28_version_predicate_triggers_only_below_3_1() {
    let fx = Fixture::new();
    let state = editor(&fx);
    let check = |v: &str| -> bool {
        eval(
            &state,
            &format!("return pmacs.lean._version_below_3_1(\"{v}\")"),
        )
    };
    assert!(check("Lake version 3.0.0"), "3.0.0 is below 3.1");
    assert!(!check("Lake version 3.1.0"), "3.1.0 is not below 3.1");
    assert!(!check("Lake version 5.0.0-abc"), "5.0.0 is not below 3.1");
    assert!(check("Lake version 2.9.9"), "2.9.9 is below 3.1");
    assert!(
        !check("no default toolchain configured"),
        "an unparseable line must NOT trigger the fallback — that is the \
         elan-shim case, which the failure latch handles better"
    );
}

#[test]
fn acc28_an_old_lake_falls_back_and_the_buffer_lands_on_a_live_server() {
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let old_lake = fx.lake_stub("bin/lake", "Lake version 3.0.0");
    let mut state = editor(&fx);
    with_fallback(&state, &old_lake);

    open(&state, &file);
    settle(&mut state);
    // The stub's `serve` sleeps rather than dying, so ONLY the probe can
    // have caused a fallback here. That isolation is the point.
    for _ in 0..40 {
        state.tick_processes();
        state.tick_lsp();
        state.tick_async();
        std::thread::sleep(Duration::from_millis(5));
        if attached_state(&state) == "initialized" {
            break;
        }
    }

    assert_eq!(
        attached_state(&state),
        "initialized",
        "an old lake must leave the buffer on a LIVE fallback server, not \
         merely rewrite the config"
    );
    let cmd: String = eval(&state, "return pmacs.lsp.config.lean4.command");
    assert_eq!(cmd, fake_lsp_path(), "the fallback command is in effect");
}

#[test]
fn acc28_a_current_lake_does_not_trigger_the_fallback() {
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let new_lake = fx.lake_stub("bin/lake", "Lake version 3.1.0");
    let mut state = editor(&fx);
    with_fallback(&state, &new_lake);

    open(&state, &file);
    for _ in 0..20 {
        state.tick_processes();
        state.tick_lsp();
        state.tick_async();
        std::thread::sleep(Duration::from_millis(5));
    }

    // Non-vacuity against the test above: same harness, same stub shape,
    // only the version differs — so a latch that fired unconditionally
    // would be caught here.
    let cmd: String = eval(&state, "return pmacs.lsp.config.lean4.command");
    assert_eq!(
        cmd,
        new_lake.display().to_string(),
        "a current lake keeps its command; the probe must not fall back"
    );
    let latched: bool = eval(&state, "return pmacs.lean._probe.latched");
    assert!(!latched, "the latch did not arm");
}

#[test]
fn acc27_a_missing_lake_falls_back_and_the_buffer_lands_on_a_live_server() {
    // The case round 1 could not see at all: `ensure_server` swallows a
    // synchronous ENOENT and returns nil, so there is no attachment to
    // key off. This is also the most likely real-world failure — a user
    // with `lean` but no `lake`.
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let absent = fx.dir("bin/no-such-lake");
    let mut state = editor(&fx);
    with_fallback(&state, &absent);

    open(&state, &file);
    for _ in 0..40 {
        state.tick_processes();
        state.tick_lsp();
        state.tick_async();
        std::thread::sleep(Duration::from_millis(5));
        if attached_state(&state) == "initialized" {
            break;
        }
    }

    assert_eq!(
        attached_state(&state),
        "initialized",
        "a missing `lake` must fall back to a live server and re-attach \
         the buffer that was already open"
    );
    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("lean4"),
        "and it says so on the status line; saw {status:?}"
    );
}

#[test]
fn acc27_the_latch_is_one_shot_and_does_not_re_arm() {
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let absent = fx.dir("bin/no-such-lake");
    let mut state = editor(&fx);
    with_fallback(&state, &absent);

    open(&state, &file);
    settle(&mut state);
    let after_first: String = eval(&state, "return pmacs.lsp.config.lean4.command");
    assert_eq!(after_first, fake_lsp_path(), "the fallback fired once");

    // A user who deliberately sets something else after the fallback must
    // not have it silently replaced by a second firing.
    exec(&state, "pmacs.lsp.config.lean4.command = \"user-choice\"");
    exec(&state, "pmacs.lean._fire_latch(nil, \"a second failure\")");
    assert_eq!(
        eval::<String>(&state, "return pmacs.lsp.config.lean4.command"),
        "user-choice",
        "the latch never re-arms within a session"
    );
}

#[test]
fn acc35_latch_preserves_user_config_and_swaps_only_command_and_args() {
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let absent = fx.dir("bin/no-such-lake");
    let mut state = editor(&fx);
    with_fallback(&state, &absent);
    exec(
        &state,
        r"
        pmacs.lsp.config.lean4.settings = { lean = { verbose = true } }
        pmacs.lsp.config.lean4.init_options = { hasWidgets = false }
        _G.root_before = pmacs.lsp.config.lean4.root
        ",
    );

    open(&state, &file);
    settle(&mut state);

    let after: String = eval(
        &state,
        r#"
        local c = pmacs.lsp.config.lean4
        return table.concat({
          tostring(c.settings and c.settings.lean and c.settings.lean.verbose),
          tostring(c.init_options and c.init_options.hasWidgets),
          tostring(c.root == _G.root_before),
        }, "|")
        "#,
    );
    assert_eq!(
        after, "true|false|true",
        "settings, init_options and root survive the swap; only \
         command/args change"
    );
}

#[test]
fn acc36_latch_stops_the_failing_server_before_spawning_the_fallback() {
    // A stub whose `serve` exits immediately: the server dies before
    // `initialize` completes, which is the failure the latch polls for.
    // `RestartPolicy::OnCrash` would otherwise respawn it forever
    // underneath the latch, with no attempt ceiling.
    use std::os::unix::fs::PermissionsExt as _;
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let dying = fx.root.join("bin/dying-lake");
    std::fs::create_dir_all(dying.parent().unwrap()).unwrap();
    std::fs::write(
        &dying,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'Lake version 9.9.9'; exit 0; fi\nexit 3\n",
    )
    .unwrap();
    std::fs::set_permissions(&dying, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut state = editor(&fx);
    with_fallback(&state, &dying);
    open(&state, &file);
    for _ in 0..60 {
        state.tick_processes();
        state.tick_lsp();
        state.tick_async();
        std::thread::sleep(Duration::from_millis(5));
        if attached_state(&state) == "initialized" {
            break;
        }
    }

    // The load-bearing assertion: the buffer ends up on a LIVE server.
    assert_eq!(
        attached_state(&state),
        "initialized",
        "the failing server is stopped and the buffer re-attached to the \
         fallback — not left terminal"
    );
    // And the dead one really is stopped, so nothing is respawning it.
    let dying_still_running: bool = eval(
        &state,
        r#"
        local live = tostring(pmacs.lsp.active_attachment().server)
        for _, s in ipairs(pmacs.lsp.list()) do
          if tostring(s.id) ~= live then
            local k = s.state and s.state.kind
            if k ~= "stopped" and k ~= "crashed" then return true end
          end
        end
        return false
        "#,
    );
    assert!(
        !dying_still_running,
        "the failing server is not respawning underneath the latch"
    );
    assert_ne!(attached_sid(&state), "none");
}

// ---------------------------------------------------------------------------
// Acceptance 36a — attribution (COHERENCE §9 / §1.2).
// ---------------------------------------------------------------------------

#[test]
fn acc36a_latch_leaves_a_status_line_trace() {
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let absent = fx.dir("bin/no-such-lake");
    let mut state = editor(&fx);
    with_fallback(&state, &absent);

    open(&state, &file);
    settle(&mut state);

    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("lean4") && status.contains("falling back"),
        "the fallback names the language and says it fell back; saw {status:?}"
    );
    // The channel assertion is the point (COHERENCE §1.2): a report made
    // only through `pmacs.error` — undefined in production — would leave
    // this empty while the fallback itself still worked, so the user
    // would silently be on a different server than they configured.
    assert!(!status.is_empty());
}

#[test]
fn acc36a_probe_carries_a_lean_owned_process_label() {
    // `ProcessSpec.label` is the only identity a process has, and it is
    // what `pmacs.process.list` renders. Asserted on the spec the module
    // builds rather than on a live `lake`, which CI does not have.
    let fx = Fixture::new();
    let state = editor(&fx);
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("builtin/runtime/lean.lua"),
    )
    .unwrap();
    assert!(
        src.contains("label = \"lean:lake-version-probe\""),
        "the probe process is attributed to Lean by label"
    );
    // And it is genuinely lazy: no probe without an attachment.
    let procs: i64 = eval(&state, "return #pmacs.process.list()");
    assert_eq!(procs, 0, "configuring Lean does not start the probe");
}

// ---------------------------------------------------------------------------
// Acceptance 37 — waitForDiagnostics resolves through the response seam.
// ---------------------------------------------------------------------------

#[test]
fn acc37_wait_for_diagnostics_resolves_through_the_response_seam() {
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a : Nat := 1\n");
    let mut state = editor(&fx);
    open(&state, &file);
    settle(&mut state);

    exec(
        &state,
        r#"
        _G.settled = "never"
        local rec = pmacs.lsp.active_attachment()
        pmacs.lean.wait_for_diagnostics(rec.server, rec.uri, rec.version, function(err)
          _G.settled = tostring(err)
        end)
        "#,
    );
    settle(&mut state);

    assert_eq!(
        eval::<String>(&state, "return _G.settled"),
        "nil",
        "the reply reaches the callback with no error — this is the \
         Stage 3a response seam carrying its first production caller"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 29, Lean's side — `$/lean/fileProgress` reaches the module.
//
// Driven end-to-end through the real drain: the fake server's
// `leanprogress` mode emits the notification on didOpen. Calling the
// handler directly would pin nothing about the wiring, which is the only
// part that can break.
// ---------------------------------------------------------------------------

#[test]
fn file_progress_notification_is_recorded_for_its_document() {
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let mut state = editor(&fx);
    exec(
        &state,
        "pmacs.lsp.config.lean4.env = { PMACS_FAKE_LSP_MODE = \"leanprogress\" }",
    );

    // Nothing recorded before the server speaks — so the assertion below
    // cannot pass on a pre-populated table.
    let before: i64 = eval(
        &state,
        "local n = 0 for _ in pairs(pmacs.lean.file_progress) do n = n + 1 end return n",
    );
    assert_eq!(before, 0);

    open(&state, &file);
    settle(&mut state);

    let uri: String = eval(
        &state,
        r#"
        for k, v in pairs(pmacs.lean.file_progress) do
          if type(v) == "table" and v[1] and v[1].range then return k end
        end
        return "none"
        "#,
    );
    assert!(
        uri.starts_with("file://") && uri.ends_with("A.lean"),
        "the subscriber recorded the processing ranges under the \
         document uri; saw {uri:?}"
    );
}

// ---------------------------------------------------------------------------
// Q#LN20 in the Lean resolver — a symlinked open reuses one server.
// ---------------------------------------------------------------------------

#[test]
fn lean_root_is_canonical_so_a_symlinked_open_reuses_one_server() {
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let real = fx.write("pkg/A.lean", "def a := 1\n");
    std::os::unix::fs::symlink(fx.dir("pkg"), fx.dir("linkpkg")).unwrap();
    let linked = fx.dir("linkpkg").join("A.lean");

    let mut state = editor(&fx);
    open(&state, &real);
    settle(&mut state);
    assert_eq!(rows(&state).len(), 1, "the real path spawns one server");

    open(&state, &linked);
    settle(&mut state);
    assert_eq!(
        rows(&state).len(),
        1,
        "the symlinked path reuses it — the resolver canonicalizes, so \
         both spellings produce the same affinity key"
    );
}

// ---------------------------------------------------------------------------
// Round-2 review findings. Each of these fails against the code as it
// stood at cdaea66, where the focused suite was already 20/20 — the
// lifecycle defects were invisible to it.
// ---------------------------------------------------------------------------

/// Tick for at least `ms`, so a 500ms restart backoff actually elapses.
/// The round-2 defect was invisible precisely because the suite stopped
/// ticking as soon as the fallback initialized, ~300ms in.
fn tick_for(state: &mut EditorState, ms: u64) {
    let deadline = std::time::Instant::now() + Duration::from_millis(ms);
    while std::time::Instant::now() < deadline {
        state.tick_processes();
        state.tick_lsp();
        state.tick_async();
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn r2_crashed_primary_does_not_respawn_underneath_the_fallback() {
    // The crash schedules `next_restart_at`; `maybe_restart` fires after
    // the 500ms backoff with no attempt ceiling. Skipping the retire
    // call (round 2) left that armed, so the broken command kept
    // respawning under the live fallback — forever, unobserved.
    use std::os::unix::fs::PermissionsExt as _;
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let dying = fx.root.join("bin/dying-lake");
    std::fs::create_dir_all(dying.parent().unwrap()).unwrap();
    std::fs::write(&dying, "#!/bin/sh\nexit 3\n").unwrap();
    std::fs::set_permissions(&dying, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut state = editor(&fx);
    with_fallback(&state, &dying);
    open(&state, &file);
    // Well past one backoff.
    tick_for(&mut state, 1400);

    // **`attempt`, not liveness.** A respawning server spends most of
    // its life in `crashed` waiting out the backoff, so "no live
    // non-fallback server" is satisfied while it loops forever — that
    // weaker assertion passed against the round-2 code and caught
    // nothing. `attempt` increments on every spawn, so it counts the
    // respawns directly. A retired server is absent from the list
    // entirely (`forget` removes the client); one left with
    // `next_restart_at` armed climbs past 1.
    let worst_attempt: i64 = eval(
        &state,
        r#"
        local rec = pmacs.lsp.active_attachment()
        local live = rec and tostring(rec.server) or ""
        local worst = 0
        for _, s in ipairs(pmacs.lsp.list()) do
          if tostring(s.id) ~= live then
            local a = s.attempt or 0
            if a > worst then worst = a end
          end
        end
        return worst
        "#,
    );
    assert_eq!(
        worst_attempt, 0,
        "the retired primary is gone from the manager, not respawning          after the backoff (attempt > 0 means it is still there; > 1          means it respawned)"
    );
    assert_eq!(
        attached_state(&state),
        "initialized",
        "and the buffer is on the live fallback"
    );
}

#[test]
fn r2_reattach_targets_the_originating_buffer_not_whatever_is_active() {
    // `_attach_buffer` is an active-buffer-only seam and the latch's
    // verdict arrives asynchronously. Round 2 accepted "some attachment
    // now names a different server", which an unrelated Rust buffer
    // satisfies — clearing the retry and stranding the Lean buffer.
    //
    // **Driven through the PROBE**, not through a missing executable: a
    // missing command fails synchronously inside `buffer.after-load`,
    // where the Lean buffer is still active and the rebuild happens
    // inline, so the race cannot occur and the test proves nothing. The
    // probe's verdict lands on a later tick, which is the whole point.
    // The stub's `serve` sleeps, so only the probe can trigger anything.
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    fx.write("pkg/Cargo.toml", "[package]\nname = \"p\"\n");
    let lean_file = fx.write("pkg/A.lean", "def a := 1\n");
    let rust_file = fx.write("pkg/src/main.rs", "fn main() {}\n");
    let old_lake = fx.lake_stub("bin/lake", "Lake version 3.0.0");

    let mut state = editor(&fx);
    with_fallback(&state, &old_lake);
    // A working Rust server, so switching away lands on a real
    // attachment with a different server id — the decoy.
    exec(
        &state,
        &format!(
            "pmacs.lsp.config.rust = {{ command = \"{}\" }}",
            fake_lsp_path()
        ),
    );

    open(&state, &lean_file);
    exec(&state, "_G.lean_buf = pmacs.window.buffer()");
    // Switch away before the probe's verdict can land.
    open(&state, &rust_file);
    tick_for(&mut state, 500);

    // Come back with a buffer SWITCH, not `find_or_open`. Re-opening
    // fires `buffer.after-load`, which re-runs lsp.lua's own attach and
    // would repair the record no matter what the latch did.
    exec(&state, "pmacs.window.switch_buffer(_G.lean_buf)");
    tick_for(&mut state, 400);

    let lang: String = eval(
        &state,
        r#"
        local rec = pmacs.lsp.active_attachment()
        return rec and tostring(rec.language) or "none"
        "#,
    );
    assert_eq!(lang, "lean4", "we are back on the Lean buffer");

    // The observable that discriminates: WHICH command the Lean buffer's
    // server is running. A retry cleared by the decoy leaves it on the
    // original `lake` stub.
    let cmd: String = eval(
        &state,
        r#"
        local rec = pmacs.lsp.active_attachment()
        if not rec then return "none" end
        for _, s in ipairs(pmacs.lsp.list()) do
          if tostring(s.id) == tostring(rec.server) then
            return tostring(s.command)
          end
        end
        return "gone"
        "#,
    );
    assert_eq!(
        cmd,
        fake_lsp_path(),
        "the ORIGINATING Lean buffer ends up on the fallback — a decoy \
         Rust attachment must not satisfy the retry"
    );
}

#[test]
fn r2_a_failing_fallback_is_reported_once_and_does_not_retry_forever() {
    // Acceptance 27 promises a second failure surfaces rather than
    // loops. Round 2 retried `_attach_buffer` every tick with nothing
    // reported.
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let absent_primary = fx.dir("bin/no-such-lake");
    let absent_fallback = fx.dir("bin/no-such-lean");

    let mut state = editor(&fx);
    exec(
        &state,
        &format!(
            r#"
            pmacs.lsp.config.lean4.command = "{}"
            pmacs.lsp.config.lean4.args = {{ "serve" }}
            pmacs.lean._fallback = {{ command = "{}", args = {{}} }}
            "#,
            lua_str(&absent_primary),
            lua_str(&absent_fallback)
        ),
    );

    open(&state, &file);
    tick_for(&mut state, 300);

    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("did not start either"),
        "a failing fallback surfaces rather than retrying silently; saw \
         {status:?}"
    );
    // And the retry state is cleared, so it is not looping.
    let pending: String = eval(&state, "return tostring(pmacs.lean._probe.reattach_from)");
    assert_eq!(pending, "nil", "the retry is retired, not spinning");
}

#[test]
fn r2_a_working_wrapper_is_not_version_probed_as_lake() {
    // `version_below_3_1` encodes LAKE's output contract. Applying it to
    // an arbitrary wrapper is a category error: a working wrapper
    // reporting its own "wrapper 1.0" would be replaced despite its
    // server initializing fine.
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    // Named something other than `lake`, reporting a sub-3.1 version,
    // but which serves fine.
    let wrapper = fx.lake_stub("bin/my-lean-wrapper", "wrapper 1.0");
    let mut state = editor(&fx);
    with_fallback(&state, &wrapper);

    open(&state, &file);
    tick_for(&mut state, 400);

    let cmd: String = eval(&state, "return pmacs.lsp.config.lean4.command");
    assert_eq!(
        cmd,
        wrapper.display().to_string(),
        "a wrapper's own version string is not Lake's; the version probe \
         must not run against it"
    );
    let latched: bool = eval(&state, "return pmacs.lean._probe.latched");
    assert!(!latched, "and the latch stayed disarmed");
}

#[test]
fn r2_an_unconfigured_lean_server_is_disabled_not_failed() {
    // Setting `pmacs.lsp.config.lean4 = nil` means "off". Reporting that
    // `nil` could not start is a false alarm, and latching poisons the
    // session so a later configuration can never take effect.
    let fx = Fixture::new();
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let mut state = editor(&fx);
    exec(&state, "pmacs.lsp.config.lean4 = nil");
    exec(&state, "pmacs.editor.set_status(\"\")");

    open(&state, &file);
    settle(&mut state);

    assert_eq!(
        state.core.borrow().status.clone(),
        "",
        "an unconfigured Lean server reports nothing — it is disabled"
    );
    let latched: bool = eval(&state, "return pmacs.lean._probe.latched");
    assert!(
        !latched,
        "and the session is not poisoned: a later config must still work"
    );
}
