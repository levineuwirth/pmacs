//! Arc 8 Stage 3b acceptance — the Lean 4 language server.
//!
//! the archived lean4-mode framing Q#LN7, Q#LN8, Q#LN16; acceptance 22–28,
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
    let state = EditorState::new_with_roots(&crate::iso::roots());
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
    // And the repair was ATTEMPTED and recorded, so it is bounded rather
    // than spinning. Asserting on a field that no longer exists would
    // read as nil and pass for nothing — the vacuity shape this branch
    // keeps producing, so the assertion is on a positive count.
    let attempted: i64 = eval(
        &state,
        "local n = 0 for _ in pairs(pmacs.lean._probe.repaired) do n = n + 1 end return n",
    );
    assert_eq!(
        attempted, 1,
        "exactly one repair attempt was made and recorded, so a failing \
         fallback cannot retry every tick forever"
    );
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

// ---------------------------------------------------------------------------
// Round-3 review findings — asynchronous correlation.
//
// Both fail against 3377db0, where the suite was 25/25.
// ---------------------------------------------------------------------------

impl Fixture {
    /// A `lake` whose `serve` really works (it execs the fake LSP) but
    /// whose `--version` answers slowly with an old version. This is the
    /// ordering the previous fixtures could not produce: the primary
    /// INITIALIZES before the version verdict arrives.
    fn slow_version_lake(&self, rel: &str, server: &str, version_line: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  sleep 0.6\n  echo '{version_line}'\n  exit 0\nfi\nexec '{server}'\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}

/// The command backing the active buffer's attached server.
fn attached_command(state: &EditorState) -> String {
    eval(
        state,
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
    )
}

#[test]
fn r3_a_late_version_verdict_still_retires_an_initialized_primary() {
    // `probe.watching` is cleared the moment the server initializes. A
    // verdict arriving after that used to call `fire_latch(nil)`, which
    // retires nothing — `_attach_buffer` then returns the still-live
    // primary and the retry calls it success. Status and config would
    // say "fell back" while the buffer stayed put.
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let lake = fx.slow_version_lake("bin/lake", &fake_lsp_path(), "Lake version 3.0.0");
    let mut state = editor(&fx);
    with_fallback(&state, &lake);

    open(&state, &file);
    // Let the primary initialize first — the ordering that matters.
    tick_for(&mut state, 300);
    assert_eq!(
        attached_state(&state),
        "initialized",
        "precondition: the primary really did come up before the verdict"
    );
    assert_eq!(
        attached_command(&state),
        lake.display().to_string(),
        "precondition: and the buffer is on it"
    );

    // Now let the slow `--version` land and the fallback complete.
    tick_for(&mut state, 1200);

    assert_eq!(
        attached_command(&state),
        fake_lsp_path(),
        "a late version verdict must actually move the buffer to the \
         fallback, not just rewrite the config and claim it did"
    );
    // And the retired primary is not left running or respawning.
    let stale: i64 = eval(
        &state,
        r#"
        local rec = pmacs.lsp.active_attachment()
        local live = rec and tostring(rec.server) or ""
        local n = 0
        for _, s in ipairs(pmacs.lsp.list()) do
          if tostring(s.id) ~= live then
            local k = s.state and s.state.kind
            if k ~= "stopped" and k ~= "crashed" then n = n + 1 end
          end
        end
        return n
        "#,
    );
    assert_eq!(stale, 0, "the initialized primary was retired, not left up");
}

#[test]
fn r3_a_second_lean_buffer_does_not_steal_the_rebuild_target() {
    // `buf_key` was written on every Lean `buffer.after-load`, so a
    // second Lean file opened before the verdict became the rebuild
    // target while the latch still watched the FIRST buffer's server.
    //
    // Both files live in the SAME Lake package, so they share one server
    // and one root — which is what makes the mis-targeting observable as
    // a stranded buffer rather than as two independent servers.
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let first = fx.write("pkg/A.lean", "def a := 1\n");
    let second = fx.write("pkg/B.lean", "def b := 2\n");
    let lake = fx.lake_stub("bin/lake", "Lake version 3.0.0");
    let mut state = editor(&fx);
    with_fallback(&state, &lake);

    open(&state, &first);
    exec(&state, "_G.first_buf = pmacs.window.buffer()");
    // A second Lean buffer, opened before the probe's verdict lands.
    open(&state, &second);
    tick_for(&mut state, 500);

    // The armed target must still be the FIRST buffer.
    let target_is_first: bool = eval(
        &state,
        "return pmacs.lean._probe.buf_key == tostring(_G.first_buf)",
    );
    assert!(
        target_is_first,
        "the rebuild target is captured once, when the latch arms — a \
         later Lean buffer must not silently become the target"
    );

    // And the first buffer really does end up on the fallback.
    exec(&state, "pmacs.window.switch_buffer(_G.first_buf)");
    tick_for(&mut state, 600);
    assert_eq!(
        attached_command(&state),
        fake_lsp_path(),
        "the originating buffer is the one repaired"
    );
}

#[test]
fn r3_a_failing_wrapper_is_named_truthfully_not_as_lake_serve() {
    // The failure latch is command-agnostic, so its message must be too.
    // Telling a user that `lake serve` failed when they configured
    // `my-lean-wrapper` sends them to debug the wrong thing.
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let absent = fx.dir("bin/my-lean-wrapper");
    let mut state = editor(&fx);
    with_fallback(&state, &absent);

    open(&state, &file);
    settle(&mut state);

    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("my-lean-wrapper"),
        "the status names the command the user actually configured; saw \
         {status:?}"
    );
    assert!(
        !status.contains("lake serve"),
        "and does not attribute the failure to `lake serve`; saw {status:?}"
    );
}

// ---------------------------------------------------------------------------
// Round-4 review — the config swap is GLOBAL, so one repaired buffer is
// not a fallback. Both fail against 73587b0.
// ---------------------------------------------------------------------------

#[test]
fn r4_every_open_lean_buffer_is_repaired_not_just_the_armed_one() {
    // `pmacs.lsp.config.lean4` is a single entry; swapping its command
    // invalidates every buffer attached to the old one. Round 3 repaired
    // exactly `probe.buf_key` and cleared the retry, leaving every other
    // open Lean buffer on the retired server while status and config
    // both said "fell back".
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let first = fx.write("pkg/A.lean", "def a := 1\n");
    let second = fx.write("pkg/B.lean", "def b := 2\n");
    let lake = fx.lake_stub("bin/lake", "Lake version 3.0.0");
    let mut state = editor(&fx);
    with_fallback(&state, &lake);

    open(&state, &first);
    exec(&state, "_G.first_buf = pmacs.window.buffer()");
    open(&state, &second);
    exec(&state, "_G.second_buf = pmacs.window.buffer()");
    tick_for(&mut state, 700);

    // The armed (first) buffer.
    exec(&state, "pmacs.window.switch_buffer(_G.first_buf)");
    tick_for(&mut state, 500);
    assert_eq!(
        attached_command(&state),
        fake_lsp_path(),
        "the armed buffer is repaired"
    );

    // And the OTHER one, which round 3 stranded.
    exec(&state, "pmacs.window.switch_buffer(_G.second_buf)");
    tick_for(&mut state, 500);
    assert_eq!(
        attached_command(&state),
        fake_lsp_path(),
        "every open Lean buffer ends up on the fallback — repairing only \
         the armed target leaves this one on the retired server"
    );
}

#[test]
fn r4_a_second_project_roots_server_is_also_retired() {
    // Q#LN15 gives one server per project root, so a swap can invalidate
    // several. `probe.primary` names only the first; retiring only that
    // leaves the second root's server live on a command the config no
    // longer names.
    let fx = Fixture::new();
    fx.toolchain("one", "v4.9.0\n");
    fx.toolchain("two", "v4.9.0\n");
    let a = fx.write("one/A.lean", "def a := 1\n");
    let b = fx.write("two/B.lean", "def b := 2\n");
    let lake = fx.lake_stub("bin/lake", "Lake version 3.0.0");
    let mut state = editor(&fx);
    with_fallback(&state, &lake);

    open(&state, &a);
    open(&state, &b);
    // Two roots, two servers, before any verdict lands.
    let before: i64 = eval(&state, "return #pmacs.lsp.list()");
    assert_eq!(before, 2, "precondition: one server per root");

    tick_for(&mut state, 900);

    // No server may still be running the retired command.
    let stale_live: i64 = eval(
        &state,
        &format!(
            r#"
            local n = 0
            for _, s in ipairs(pmacs.lsp.list()) do
              if tostring(s.command) == "{}" then
                local k = s.state and s.state.kind
                if k ~= "stopped" and k ~= "crashed" then n = n + 1 end
              end
            end
            return n
            "#,
            lua_str(&lake)
        ),
    );
    assert_eq!(
        stale_live, 0,
        "every Lean server spawned from the old command is retired, not \
         just the one the probe happened to name"
    );
}

#[test]
fn r4_attribution_names_the_exact_command_and_its_arguments() {
    // Round 3 implemented argument-inclusive attribution but pinned only
    // "contains my-lean-wrapper" and "does not contain lake serve" — a
    // mutation dropping every argument still passed.
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let absent = fx.dir("bin/my-lean-wrapper");
    let mut state = editor(&fx);
    with_fallback(&state, &absent);
    exec(
        &state,
        "pmacs.lsp.config.lean4.args = { \"serve\", \"--quiet\" }",
    );

    open(&state, &file);
    settle(&mut state);

    let status = state.core.borrow().status.clone();
    let expected = format!("`{} serve --quiet`", absent.display());
    assert!(
        status.contains(&expected),
        "the status names the exact configured command AND its arguments;\n  \
         want substring: {expected}\n  saw: {status:?}"
    );
}

// ---------------------------------------------------------------------------
// Round-5 review. All fail against 7c37bdc.
// ---------------------------------------------------------------------------

#[test]
fn r5_a_fallback_that_dies_after_spawning_is_bounded_and_reported() {
    // The once-per-buffer guard bounds calls to `_attach_buffer`, not
    // the server it produced. `ensure_server` never forwards
    // `cfg.restart`, so the fallback inherits `OnCrash` and a binary
    // that exits before `initialize` is respawned forever — silently,
    // because `latched` has already disabled the primary's poll. The
    // prior failing-fallback test used a NONEXISTENT executable, which
    // only exercises synchronous ENOENT.
    use std::os::unix::fs::PermissionsExt as _;
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let absent_primary = fx.dir("bin/no-such-lake");
    let dying_fallback = fx.root.join("bin/dying-lean");
    std::fs::create_dir_all(dying_fallback.parent().unwrap()).unwrap();
    std::fs::write(&dying_fallback, "#!/bin/sh\nexit 4\n").unwrap();
    std::fs::set_permissions(&dying_fallback, std::fs::Permissions::from_mode(0o755)).unwrap();

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
            lua_str(&dying_fallback)
        ),
    );

    open(&state, &file);
    tick_for(&mut state, 1600);

    // Nothing may be respawning: `attempt` counts spawns per server.
    let worst_attempt: i64 = eval(
        &state,
        r"
        local worst = 0
        for _, s in ipairs(pmacs.lsp.list()) do
          local a = s.attempt or 0
          if a > worst then worst = a end
        end
        return worst
        ",
    );
    assert!(
        worst_attempt <= 1,
        "a dying fallback must not be respawned indefinitely; saw \
         attempt {worst_attempt}"
    );
    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("did not stay up") || status.contains("did not start"),
        "and the second failure is reported; saw {status:?}"
    );
}

#[test]
fn r5_a_user_spawned_lean_server_is_not_retired_by_the_fallback() {
    // Language id AND label are public caller-supplied values. Even a
    // user server that deliberately collides with the automatic path's
    // `default-lean4` display label is not derived from
    // `pmacs.lsp.config.lean4` and must not be stopped.
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let absent = fx.dir("bin/no-such-lake");
    let mut state = editor(&fx);
    with_fallback(&state, &absent);
    exec(
        &state,
        &format!(
            r#"
            _G.mine = pmacs.lsp.spawn({{
              label = "default-lean4",
              language_id = "lean4",
              command = "{}",
              args = {{}},
            }})
            "#,
            fake_lsp_path()
        ),
    );
    settle(&mut state);

    open(&state, &file);
    tick_for(&mut state, 600);

    let mine_alive: bool = eval(
        &state,
        r#"
        for _, s in ipairs(pmacs.lsp.list()) do
          if tostring(s.id) == tostring(_G.mine) then
            local k = s.state and s.state.kind
            return k ~= "stopped" and k ~= "crashed"
          end
        end
        return false
        "#,
    );
    assert!(
        mine_alive,
        "a user-spawned Lean server survives a config-driven fallback — \
         it was never derived from that config"
    );
}

#[test]
fn r5_no_swap_means_no_repair_attempts() {
    // When the config already names the fallback, `swap_to_fallback`
    // returns false and `fire_latch` returns early — but `latched` is
    // true, so a repair gated on `latched` retried the UNCHANGED
    // configuration and reported it as a fallback failure.
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let absent = fx.dir("bin/no-such-lean");
    let mut state = editor(&fx);
    // Config and fallback are the SAME missing command, so no swap is
    // possible.
    exec(
        &state,
        &format!(
            r#"
            pmacs.lsp.config.lean4.command = "{}"
            pmacs.lsp.config.lean4.args = {{}}
            pmacs.lean._fallback = {{ command = "{}", args = {{}} }}
            "#,
            lua_str(&absent),
            lua_str(&absent)
        ),
    );

    open(&state, &file);
    tick_for(&mut state, 400);

    let attempts: i64 = eval(&state, "return pmacs.lean._probe.repair_attempts");
    assert_eq!(
        attempts, 0,
        "no swap happened, so there is nothing to apply and no repair \
         should be attempted"
    );
    let status = state.core.borrow().status.clone();
    assert!(
        !status.contains("falling back"),
        "and nothing claims a fallback occurred; saw {status:?}"
    );
}

#[test]
fn r5_repair_is_attempted_at_most_once_per_buffer_by_count() {
    // Counting keys in the `repaired` table cannot distinguish
    // "once per buffer" from "every tick for one buffer" — the
    // cardinality stays 1 either way. Count the ATTEMPTS.
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
    // Many ticks; a per-tick retry would climb without bound.
    tick_for(&mut state, 900);

    let attempts: i64 = eval(&state, "return pmacs.lean._probe.repair_attempts");
    assert_eq!(
        attempts, 1,
        "exactly one repair attempt across many ticks for one buffer"
    );
}

#[test]
fn r5_a_dead_attachment_is_never_handed_to_a_command() {
    // Buffers live in other frontends get no `buffer.after-switch` here,
    // so an eager sweep keyed on the ambient active buffer cannot reach
    // them. Healing at the point of USE is frontend-agnostic:
    // `attached_for_active` must not return a record whose server is
    // gone.
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let mut state = editor(&fx);
    // A working primary, so we get a live attachment first.
    exec(
        &state,
        &format!("pmacs.lsp.config.lean4.command = \"{}\"", fake_lsp_path()),
    );
    open(&state, &file);
    settle(&mut state);
    let first: String = attached_sid(&state);
    assert_ne!(first, "none", "precondition: attached");

    // Retire it out from under the buffer, as the latch does globally,
    // WITHOUT any switch or repair tick.
    exec(
        &state,
        r"
        local rec = pmacs.lsp.active_attachment()
        pcall(pmacs.lsp.stop, rec.server)
        ",
    );
    for _ in 0..40 {
        state.tick_processes();
        state.tick_lsp();
        std::thread::sleep(Duration::from_millis(5));
    }

    // Now a command resolves its attachment. It must not get the dead
    // one; it must rebuild.
    // `attachment_for_request` is deliberately non-attaching, so a dead
    // record must read as "no attachment" rather than being handed over.
    let for_request: String = eval(
        &state,
        r#"
        local rec = pmacs.lsp.attachment_for_request()
        if not rec then return "none" end
        for _, s in ipairs(pmacs.lsp.list()) do
          if tostring(s.id) == tostring(rec.server) then
            return tostring(s.state and s.state.kind)
          end
        end
        return "gone"
        "#,
    );
    assert_eq!(
        for_request, "none",
        "a non-attaching resolve must not hand back a dead server"
    );

    // And the attaching path rebuilds rather than returning the corpse.
    let rebuilt: String = eval(
        &state,
        r#"
        pmacs.lsp._attach_buffer()
        local rec = pmacs.lsp.active_attachment()
        if not rec then return "none" end
        for _, s in ipairs(pmacs.lsp.list()) do
          if tostring(s.id) == tostring(rec.server) then
            return tostring(s.state and s.state.kind)
          end
        end
        return "gone"
        "#,
    );
    assert!(
        rebuilt != "stopped" && rebuilt != "crashed" && rebuilt != "gone" && rebuilt != "none",
        "the attaching path rebuilds against a live server; saw \
         {rebuilt:?}"
    );
}

// ---------------------------------------------------------------------------
// Round-6 review. Each is a direct counterexample against 19f48d4.
// ---------------------------------------------------------------------------

#[test]
fn r6_the_shipped_lean_command_rebuilds_a_dead_attachment() {
    // The round-5 test called `attachment_for_request` and
    // `_attach_buffer` directly, while the shipped Lean command read the
    // raw `active_attachment` and still handed its request to a stopped
    // server. Drive the production command this time.
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let mut state = editor(&fx);
    open(&state, &file);
    settle(&mut state);

    exec(
        &state,
        r"
        local rec = pmacs.lsp.active_attachment()
        assert(rec)
        pmacs.lsp.stop(rec.server)
        ",
    );
    tick_for(&mut state, 200);

    exec(
        &state,
        r#"pmacs.command.invoke("lean.wait-for-diagnostics")"#,
    );
    let kind: String = eval(
        &state,
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
    );
    assert!(
        kind != "stopped" && kind != "crashed" && kind != "gone" && kind != "none",
        "the shipped command must resolve through the command-safe \
         attachment path; saw {kind:?}"
    );
    tick_for(&mut state, 500);
    let status = state.core.borrow().status.clone();
    assert_eq!(
        status, "lean: elaboration complete",
        "the rebuilt command path must deliver the request, not merely \
         replace the attachment"
    );
}

#[test]
fn r6_every_spawned_fallback_server_is_bounded() {
    // A scalar fallback watch covers only one Q#LN15 root. The second
    // server can also be created directly by lsp.lua's after-load path,
    // bypassing `repair_active_if_stale` entirely.
    use std::os::unix::fs::PermissionsExt as _;

    let fx = Fixture::new();
    fx.toolchain("one", "v4.9.0\n");
    fx.toolchain("two", "v4.9.0\n");
    let first = fx.write("one/A.lean", "def a := 1\n");
    let second = fx.write("two/B.lean", "def b := 2\n");
    let absent_primary = fx.dir("bin/no-such-lake");
    let dying_fallback = fx.root.join("bin/dying-lean");
    std::fs::create_dir_all(dying_fallback.parent().unwrap()).unwrap();
    std::fs::write(&dying_fallback, "#!/bin/sh\nexit 4\n").unwrap();
    std::fs::set_permissions(&dying_fallback, std::fs::Permissions::from_mode(0o755)).unwrap();

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
            lua_str(&dying_fallback)
        ),
    );

    open(&state, &first);
    open(&state, &second);
    tick_for(&mut state, 1600);

    let worst_attempt: i64 = eval(
        &state,
        r"
        local worst = 0
        for _, s in ipairs(pmacs.lsp.list()) do
          if s.language_id == 'lean4' and (s.attempt or 0) > worst then
            worst = s.attempt
          end
        end
        return worst
        ",
    );
    assert!(
        worst_attempt <= 1,
        "every fallback server must be bounded; an unwatched root \
         reached attempt {worst_attempt}"
    );
}

#[test]
fn r6_point_of_use_healing_does_not_duplicate_a_restarting_server() {
    // A crashed OnCrash server still has `next_restart_at` armed.
    // Spawning a fresh id beside it produces two same-root servers when
    // the old one restarts. Use Rust so this pins the general lsp.lua
    // seam independently of Lean's fallback lifecycle.
    let fx = Fixture::new();
    let file = fx.write("A.rs", "fn main() {}\n");
    let mut state = editor(&fx);
    exec(
        &state,
        &format!(
            r#"
            pmacs.lsp.config.rust = {{
              command = "{}",
              args = {{}},
              env = {{ PMACS_FAKE_LSP_MODE = "crash" }},
            }}
            "#,
            fake_lsp_path()
        ),
    );
    open(&state, &file);

    let mut crashed = false;
    for _ in 0..100 {
        state.tick_processes();
        state.tick_lsp();
        crashed = eval(
            &state,
            r#"
            for _, s in ipairs(pmacs.lsp.list()) do
              if s.language_id == "rust"
                  and s.state and s.state.kind == "crashed" then
                return true
              end
            end
            return false
            "#,
        );
        if crashed {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(crashed, "precondition: the attached server crashed");

    exec(&state, "pmacs.lsp.hover_at_cursor()");
    let rust_servers: i64 = eval(
        &state,
        r#"
        local n = 0
        for _, s in ipairs(pmacs.lsp.list()) do
          if s.language_id == "rust" then n = n + 1 end
        end
        return n
        "#,
    );
    assert_eq!(
        rust_servers, 1,
        "healing must cancel the old id's armed restart before spawning \
         its replacement"
    );
}

#[test]
fn r6_no_swap_retires_only_the_failed_root() {
    // When config already equals the fallback, no shared config changed.
    // One root's failure must not globally retire another root's healthy
    // instance of the same cwd-sensitive command.
    use std::os::unix::fs::PermissionsExt as _;

    let fx = Fixture::new();
    fx.toolchain("bad", "v4.9.0\n");
    fx.toolchain("good", "v4.9.0\n");
    let bad = fx.write("bad/A.lean", "def a := 1\n");
    let good = fx.write("good/B.lean", "def b := 2\n");
    let wrapper = fx.root.join("bin/root-sensitive-lean");
    std::fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\ncase \"$PWD\" in */bad) exit 4;; esac\nexec \"{}\"\n",
            fake_lsp_path()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut state = editor(&fx);
    exec(
        &state,
        &format!(
            r#"
            pmacs.lsp.config.lean4.command = "{}"
            pmacs.lsp.config.lean4.args = {{}}
            pmacs.lean._fallback = {{ command = "{}", args = {{}} }}
            "#,
            lua_str(&wrapper),
            lua_str(&wrapper)
        ),
    );
    open(&state, &bad);
    open(&state, &good);
    tick_for(&mut state, 700);

    let good_alive: bool = eval(
        &state,
        r#"
        for _, s in ipairs(pmacs.lsp.list()) do
          if s.cwd and s.cwd:match("/good$") then
            local k = s.state and s.state.kind
            return k ~= "stopped" and k ~= "crashed"
          end
        end
        return false
        "#,
    );
    assert!(
        good_alive,
        "one root's failure must not stop another root when no config \
         swap occurred"
    );
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
