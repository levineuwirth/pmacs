//! Arc 8 Stage 2 acceptance — multi-root LSP server affinity.
//!
//! `docs/lean4-mode-framing.md` Q#LN15, acceptance 13–21.
//!
//! This suite deliberately contains **no Lean content**. `ensure_server`
//! (`builtin/runtime/lsp.lua`) is the single server-affinity function for
//! every LSP language in pmacs, so the change is exercised through the
//! four languages that already shipped attach paths — rust, python, go,
//! typescript — driven against `pmacs_fake_lsp` so nothing here needs a
//! real toolchain on PATH.
//!
//! Every fixture calls `pmacs.project.set_search_boundary` at its own
//! tempdir root. Without it the marker walk climbs to the filesystem
//! root, and a stray `.git` above the temp directory would silently turn
//! the "markerless" cases into detected ones — the assertions would still
//! pass while testing nothing.

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

/// A fresh editor with the shipped language configs cleared, so the only
/// server any test can spawn is the fake one it configures itself.
fn editor() -> EditorState {
    let state = EditorState::new();
    exec(&state, "pmacs.lsp.config = {}");
    state
}

fn lua_str(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// Mirror of `file_uri_for` in `builtin/runtime/lsp.lua` and
/// `path_to_file_uri` in `src/lsp.rs`. Reimplemented rather than
/// imported so the test states the expected encoding independently of
/// the code under test.
fn file_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for ch in path.display().to_string().chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '/' | '-' | '_' | '.' | '~' | ':' => out.push(ch),
            _ => {
                use std::fmt::Write as _;
                let mut buf = [0u8; 4];
                for byte in ch.encode_utf8(&mut buf).as_bytes() {
                    let _ = write!(out, "%{byte:02X}");
                }
            }
        }
    }
    out
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    /// Canonicalized so the expected roots below compare equal to what
    /// `pmacs.project.detect` returns (it canonicalizes before walking,
    /// which matters on macOS where `/var` is a symlink to `/private/var`).
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

fn settle(state: &mut EditorState) {
    for _ in 0..8 {
        state.tick_processes();
        state.tick_lsp();
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// One `language_id|root_uri|cwd|state` row per live server, sorted so
/// assertions do not depend on spawn order. Absent fields read as "".
fn rows(state: &EditorState) -> Vec<String> {
    let joined: String = eval(
        state,
        r#"
        local out = {}
        for _, s in ipairs(pmacs.lsp.list()) do
          out[#out + 1] = table.concat({
            s.language_id or "",
            s.root_uri or "",
            s.cwd or "",
            (s.state and s.state.kind) or "",
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

fn count(state: &EditorState) -> usize {
    let n: i64 = eval(state, "return #pmacs.lsp.list()");
    usize::try_from(n).expect("server count is non-negative")
}

// ---------------------------------------------------------------------------
// Acceptance 13 — `lsp.list()` rows carry `root_uri` and `cwd`.
// ---------------------------------------------------------------------------

#[test]
fn acc13_list_rows_carry_root_uri_and_cwd() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let file = fx.write("proj/src/main.rs", "fn main() {}\n");
    let mut state = editor();
    fx.bind(&state);
    configure(&state, "rust");
    open(&state, &file);
    settle(&mut state);

    let proj = fx.dir("proj");
    let rows = rows(&state);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let fields: Vec<&str> = rows[0].split('|').collect();
    assert_eq!(fields[0], "rust");
    assert_eq!(fields[1], file_uri(&proj), "root_uri must be the project root");
    assert_eq!(fields[2], proj.display().to_string(), "cwd must be the root");
}

// ---------------------------------------------------------------------------
// Acceptance 14 — two roots, same language, two servers.
// ---------------------------------------------------------------------------

#[test]
fn acc14_two_project_roots_of_one_language_spawn_two_servers() {
    let fx = Fixture::new();
    fx.write("a/Cargo.toml", "[package]\nname = \"a\"\n");
    fx.write("b/Cargo.toml", "[package]\nname = \"b\"\n");
    let first = fx.write("a/src/main.rs", "fn main() {}\n");
    let second = fx.write("b/src/main.rs", "fn main() {}\n");
    let mut state = editor();
    fx.bind(&state);
    configure(&state, "rust");
    open(&state, &first);
    settle(&mut state);
    open(&state, &second);
    settle(&mut state);

    let rows = rows(&state);
    assert_eq!(rows.len(), 2, "one server per project root: {rows:?}");
    let roots: Vec<&str> = rows.iter().map(|r| r.split('|').nth(1).unwrap()).collect();
    assert!(roots.contains(&file_uri(&fx.dir("a")).as_str()), "{roots:?}");
    assert!(roots.contains(&file_uri(&fx.dir("b")).as_str()), "{roots:?}");
}

// ---------------------------------------------------------------------------
// Acceptance 15 — same root, two files, one server. The pre-change
// behavior, pinned so the fix cannot degrade into "always spawn".
// ---------------------------------------------------------------------------

#[test]
fn acc15_two_files_in_one_root_reuse_a_single_server() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let first = fx.write("proj/src/main.rs", "fn main() {}\n");
    let second = fx.write("proj/src/other.rs", "pub fn other() {}\n");
    let mut state = editor();
    fx.bind(&state);
    configure(&state, "rust");
    open(&state, &first);
    settle(&mut state);
    open(&state, &second);
    settle(&mut state);

    let rows = rows(&state);
    assert_eq!(rows.len(), 1, "same root must reuse: {rows:?}");
    assert_eq!(rows[0].split('|').nth(1).unwrap(), file_uri(&fx.dir("proj")));
}

// ---------------------------------------------------------------------------
// Acceptance 16 — per-language regression pin. The single-root case is
// all the shipped attach paths ever exercised; it must be untouched.
// ---------------------------------------------------------------------------

#[test]
fn acc16_shipped_languages_are_unchanged_for_the_single_root_case() {
    // (language id, project marker, two source files under it)
    let cases: [(&str, &str, &str, &str); 4] = [
        ("rust", "Cargo.toml", "one.rs", "two.rs"),
        ("python", "pyproject.toml", "one.py", "two.py"),
        ("go", "go.mod", "one.go", "two.go"),
        ("typescript", "package.json", "one.ts", "two.ts"),
    ];
    for (language, marker, first_name, second_name) in cases {
        let fx = Fixture::new();
        fx.write(&format!("proj/{marker}"), "{}\n");
        let first = fx.write(&format!("proj/src/{first_name}"), "\n");
        let second = fx.write(&format!("proj/src/{second_name}"), "\n");
        let mut state = editor();
        fx.bind(&state);
        configure(&state, language);
        open(&state, &first);
        settle(&mut state);
        open(&state, &second);
        settle(&mut state);

        let rows = rows(&state);
        assert_eq!(rows.len(), 1, "{language}: expected one server, got {rows:?}");
        let fields: Vec<&str> = rows[0].split('|').collect();
        assert_eq!(fields[0], language, "{language}: language_id");
        assert_eq!(
            fields[1],
            file_uri(&fx.dir("proj")),
            "{language}: root must be the marker directory"
        );
    }
}

// ---------------------------------------------------------------------------
// Acceptance 17 — hoist pin. `project_root_for` now runs on the *reuse*
// path, and a function-valued `root` is memoized per directory.
// ---------------------------------------------------------------------------

#[test]
fn acc17_function_root_runs_on_the_reuse_path_and_memoizes_per_directory() {
    let fx = Fixture::new();
    let shared = fx.dir("shared");
    std::fs::create_dir_all(&shared).unwrap();
    let a1 = fx.write("one/a.rs", "fn a() {}\n");
    let a2 = fx.write("one/b.rs", "fn b() {}\n");
    let b1 = fx.write("two/c.rs", "fn c() {}\n");
    let mut state = editor();
    fx.bind(&state);
    // A resolver that answers the same root for every directory: the
    // second directory therefore REUSES the first directory's server,
    // which is exactly the path the hoist put the resolver on.
    exec(
        &state,
        &format!(
            r#"
            _G.ROOT_CALLS = 0
            pmacs.lsp.config.rust = {{
              command = "{}",
              root = function(_)
                _G.ROOT_CALLS = _G.ROOT_CALLS + 1
                return "{}"
              end,
            }}
            "#,
            fake_lsp_path(),
            lua_str(&shared)
        ),
    );

    open(&state, &a1);
    settle(&mut state);
    assert_eq!(eval::<i64>(&state, "return _G.ROOT_CALLS"), 1, "spawn path");

    // Same directory: served from the memo, so the count does not move.
    open(&state, &a2);
    settle(&mut state);
    assert_eq!(
        eval::<i64>(&state, "return _G.ROOT_CALLS"),
        1,
        "second file in the same directory must hit the memo"
    );

    // Different directory: the resolver runs again — proving the reuse
    // path resolves at all — but resolves to the same root, so no second
    // server appears.
    open(&state, &b1);
    settle(&mut state);
    assert_eq!(
        eval::<i64>(&state, "return _G.ROOT_CALLS"),
        2,
        "a new directory must consult the resolver on the reuse path"
    );
    let rows = rows(&state);
    assert_eq!(rows.len(), 1, "one resolved root, one server: {rows:?}");
    assert_eq!(rows[0].split('|').nth(1).unwrap(), file_uri(&shared));
}

// ---------------------------------------------------------------------------
// Acceptance 18 — a hand-spawned server carrying only `cwd` is not
// adopted by a root-bearing attach. A deliberate behavior change.
// ---------------------------------------------------------------------------

#[test]
fn acc18_hand_spawned_server_without_root_uri_is_not_adopted() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let file = fx.write("proj/src/main.rs", "fn main() {}\n");
    let proj = fx.dir("proj");
    let mut state = editor();
    fx.bind(&state);
    configure(&state, "rust");
    // Exactly what an init.lua would write: cwd, no root_uri.
    exec(
        &state,
        &format!(
            r#"
            pmacs.lsp.spawn({{
              label = "hand-rolled",
              language_id = "rust",
              command = "{}",
              cwd = "{}",
            }})
            "#,
            fake_lsp_path(),
            lua_str(&proj)
        ),
    );
    settle(&mut state);
    assert_eq!(count(&state), 1, "the hand-spawned server is up");

    open(&state, &file);
    settle(&mut state);

    let rows = rows(&state);
    assert_eq!(rows.len(), 2, "the attach must not adopt it: {rows:?}");
    let roots: Vec<&str> = rows.iter().map(|r| r.split('|').nth(1).unwrap()).collect();
    assert!(roots.contains(&""), "hand-spawned reads back nil: {roots:?}");
    assert!(
        roots.contains(&file_uri(&proj).as_str()),
        "the attach's own server carries the root: {roots:?}"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 19 — a dead server in the matching root is not reused.
// ---------------------------------------------------------------------------

#[test]
fn acc19_stopped_server_in_the_matching_root_is_not_reused() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let first = fx.write("proj/src/main.rs", "fn main() {}\n");
    let second = fx.write("proj/src/other.rs", "pub fn other() {}\n");
    let mut state = editor();
    fx.bind(&state);
    configure(&state, "rust");
    open(&state, &first);
    settle(&mut state);
    let original: i64 = eval(&state, "return pmacs.lsp.list()[1].id:raw()");

    exec(&state, "pmacs.lsp.stop(pmacs.lsp.list()[1].id)");
    for _ in 0..200 {
        settle(&mut state);
        let dead: bool = eval(
            &state,
            r#"
            for _, s in ipairs(pmacs.lsp.list()) do
              local k = s.state and s.state.kind
              if k == "stopped" or k == "crashed" then return true end
            end
            return false
            "#,
        );
        if dead {
            break;
        }
    }

    open(&state, &second);
    settle(&mut state);
    let live: i64 = eval(
        &state,
        r#"
        for _, s in ipairs(pmacs.lsp.list()) do
          local k = s.state and s.state.kind
          if k ~= "stopped" and k ~= "crashed" then
            return s.id:raw()
          end
        end
        return -1
        "#,
    );
    assert_ne!(live, -1, "a replacement server must exist");
    assert_ne!(live, original, "the dead server must not be reused");
}

// ---------------------------------------------------------------------------
// Acceptance 20 — the loose-file pin (Q#LN15 part 2). This is the
// no-change case, and the one a naive `(language_id, root)` key breaks.
// ---------------------------------------------------------------------------

#[test]
fn acc20_markerless_files_in_different_directories_share_one_server() {
    let fx = Fixture::new();
    let first = fx.write("loose_a/one.rs", "fn one() {}\n");
    let second = fx.write("loose_b/two.rs", "fn two() {}\n");
    let mut state = editor();
    fx.bind(&state);
    configure(&state, "rust");
    open(&state, &first);
    settle(&mut state);
    open(&state, &second);
    settle(&mut state);

    let rows = rows(&state);
    assert_eq!(
        rows.len(),
        1,
        "loose files must keep sharing one server: {rows:?}"
    );
    let fields: Vec<&str> = rows[0].split('|').collect();
    assert_eq!(fields[1], "", "the fallback root is not an affinity key");
    assert_eq!(
        fields[2],
        fx.dir("loose_a").display().to_string(),
        "cwd still carries the first file's directory"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 21 — detected and fallback are different servers, and the
// fallback one still carries its directory as `cwd`.
// ---------------------------------------------------------------------------

#[test]
fn acc21_detected_root_and_markerless_file_get_different_servers() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let inside = fx.write("proj/src/main.rs", "fn main() {}\n");
    let loose = fx.write("loose/stray.rs", "fn stray() {}\n");
    let mut state = editor();
    fx.bind(&state);
    configure(&state, "rust");
    open(&state, &inside);
    settle(&mut state);
    open(&state, &loose);
    settle(&mut state);

    let rows = rows(&state);
    assert_eq!(rows.len(), 2, "detected and fallback must differ: {rows:?}");
    let detected = rows
        .iter()
        .find(|r| r.split('|').nth(1).unwrap() == file_uri(&fx.dir("proj")))
        .unwrap_or_else(|| panic!("no server rooted at the project: {rows:?}"));
    assert_eq!(
        detected.split('|').nth(2).unwrap(),
        fx.dir("proj").display().to_string()
    );
    let fallback = rows
        .iter()
        .find(|r| r.split('|').nth(1).unwrap().is_empty())
        .unwrap_or_else(|| panic!("no rootless server: {rows:?}"));
    assert_eq!(
        fallback.split('|').nth(2).unwrap(),
        fx.dir("loose").display().to_string(),
        "the markerless server keeps the fallback directory as cwd"
    );
}
