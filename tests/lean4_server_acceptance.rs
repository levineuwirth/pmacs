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
// Acceptance 28 (probe) — the version predicate.
//
// The parse is unit-tested directly because the spawn path is timing-
// bound; the latch's *effect* is pinned separately below.
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Acceptance 27 / 35 / 36 — the fallback latch.
// ---------------------------------------------------------------------------

#[test]
fn acc35_latch_preserves_user_config_and_swaps_only_command_and_args() {
    let fx = Fixture::new();
    let state = editor(&fx);
    // A user's init.lua settings, on the shipped shape.
    exec(
        &state,
        r#"
        pmacs.lsp.config.lean4.command = "lake"
        pmacs.lsp.config.lean4.args = { "serve" }
        pmacs.lsp.config.lean4.env = { MYVAR = "1" }
        pmacs.lsp.config.lean4.settings = { lean = { verbose = true } }
        pmacs.lsp.config.lean4.init_options = { hasWidgets = false }
        _G.root_before = pmacs.lsp.config.lean4.root
        pmacs.lean._fire_latch(nil, "test")
        "#,
    );

    let after: String = eval(
        &state,
        r#"
        local c = pmacs.lsp.config.lean4
        return table.concat({
          tostring(c.command),
          tostring(c.args and c.args[1]),
          tostring(c.env and c.env.MYVAR),
          tostring(c.settings and c.settings.lean and c.settings.lean.verbose),
          tostring(c.init_options and c.init_options.hasWidgets),
          tostring(c.root == _G.root_before),
        }, "|")
        "#,
    );
    assert_eq!(
        after, "lean|--server|1|true|false|true",
        "only command/args change; env, settings, init_options and root \
         survive the swap"
    );
}

#[test]
fn acc27_the_latch_is_one_shot_and_does_not_re_arm() {
    let fx = Fixture::new();
    let state = editor(&fx);
    exec(
        &state,
        r#"
        pmacs.lsp.config.lean4.command = "lake"
        pmacs.lsp.config.lean4.args = { "serve" }
        pmacs.lean._fire_latch(nil, "first failure")
        _G.after_first = pmacs.lsp.config.lean4.command
        -- A second failure must not rewrite the command again; if it did,
        -- a user who deliberately set something else after the fallback
        -- would have it silently replaced.
        pmacs.lsp.config.lean4.command = "user-choice"
        pmacs.lean._fire_latch(nil, "second failure")
        _G.after_second = pmacs.lsp.config.lean4.command
        "#,
    );
    assert_eq!(eval::<String>(&state, "return _G.after_first"), "lean");
    assert_eq!(
        eval::<String>(&state, "return _G.after_second"),
        "user-choice",
        "the latch never re-arms within a session"
    );
}

#[test]
fn acc36_latch_stops_the_failing_server_before_spawning_the_fallback() {
    let fx = Fixture::new();
    fx.toolchain("pkg", "v4.9.0\n");
    let file = fx.write("pkg/A.lean", "def a := 1\n");
    let mut state = editor(&fx);
    open(&state, &file);
    settle(&mut state);
    assert_eq!(rows(&state).len(), 1, "precondition: one server is up");

    // Fire the latch against the live server, exactly as `poll_latch`
    // would. `pmacs.lsp.stop` sets `restart = Never` on the way out —
    // which is what prevents `RestartPolicy::OnCrash` from respawning the
    // broken command underneath the latch, forever, with no attempt cap.
    exec(
        &state,
        r#"
        pmacs.lsp.config.lean4.command = "lake"
        pmacs.lsp.config.lean4.args = { "serve" }
        pmacs.lean._fire_latch(pmacs.lsp.list()[1].id, "failed to start")
        "#,
    );
    settle(&mut state);

    let terminal: bool = eval(
        &state,
        r#"
        for _, s in ipairs(pmacs.lsp.list()) do
          local k = s.state and s.state.kind
          if k ~= "stopped" and k ~= "crashed" then return false end
        end
        return true
        "#,
    );
    assert!(
        terminal,
        "the failing server is stopped, not left to be respawned under \
         the latch"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 36a — attribution (COHERENCE §9 / §1.2).
// ---------------------------------------------------------------------------

#[test]
fn acc36a_latch_leaves_a_status_line_trace() {
    let fx = Fixture::new();
    let state = editor(&fx);
    exec(
        &state,
        r#"
        pmacs.lsp.config.lean4.command = "lake"
        pmacs.lsp.config.lean4.args = { "serve" }
        pmacs.lean._fire_latch(nil, "`lake serve` failed to start")
        "#,
    );
    let status = state.core.borrow().status.clone();
    assert!(
        status.contains("lean4") && status.contains("lean --server"),
        "the fallback names itself and what it fell back to; saw {status:?}"
    );
    // The channel assertion is the point (COHERENCE §1.2): a report made
    // only through `pmacs.error` — undefined in production — would leave
    // this empty while every other assertion here still passed.
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
        pmacs.lean.wait_for_diagnostics(rec.server, rec.uri, function(err)
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
