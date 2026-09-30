// tests/e7g_grammar_fuzz_acceptance.rs --- E7g, a grammar's fuzz run as a
// condition of shipping it.

//! A bundled grammar ships only once `scripts/fuzz-grammars` has run over
//! it, and CI's `Grammar fuzz` job runs whenever the grammar set can have
//! changed. These rows keep that from being a memory: every grammar in
//! `BUILTIN_LANGUAGES` has a row in `fuzz/corpora.tsv` naming its real
//! sources, pinned to the crate version `Cargo.lock` resolves, so adding
//! or bumping a grammar edits that file; the harness drives every grammar
//! in the table; the workflow runs on every path that can change the set;
//! and the `fuzz` profile compiles the C the way the release does, since
//! the aliasing UB E7g found becomes an overflow only when GCC 16
//! optimizes it, and is invisible at -O0. The last rows hold what the fuzz
//! run itself found taking the editor down: YAML, which aborted and ships
//! since E7h from a copy with its bound fixed, and the JavaScript family,
//! which never returns and stays out.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::syntax::BUILTIN_LANGUAGES;

#[path = "common/iso.rs"]
mod iso;

fn read(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("{rel}: {e}"))
}

struct Row {
    grammar: String,
    krate: String,
    version: String,
    repository: String,
    commit: String,
    kind: String,
    paths: String,
}

fn corpora() -> Vec<Row> {
    read("fuzz/corpora.tsv")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty() && !l.starts_with("grammar\t"))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            assert_eq!(f.len(), 7, "seven tab-separated fields: {l:?}");
            Row {
                grammar: f[0].to_owned(),
                krate: f[1].to_owned(),
                version: f[2].to_owned(),
                repository: f[3].to_owned(),
                commit: f[4].to_owned(),
                kind: f[5].to_owned(),
                paths: f[6].to_owned(),
            }
        })
        .collect()
}

#[test]
fn e7g_every_bundled_grammar_has_real_sources_pinned_to_its_locked_crate() {
    let rows = corpora();
    let named: BTreeSet<&str> = rows.iter().map(|r| r.grammar.as_str()).collect();
    assert_eq!(named.len(), rows.len(), "one row per grammar");
    let table: BTreeSet<&str> = BUILTIN_LANGUAGES.iter().map(|l| l.name).collect();
    assert_eq!(
        named, table,
        "fuzz/corpora.tsv names exactly the grammars BUILTIN_LANGUAGES ships; \
         a grammar is added with its row and fuzzed before it ships (CLAUDE.md)"
    );
    let lock = read("Cargo.lock");
    for r in &rows {
        let entry = format!("name = \"{}\"\nversion = \"{}\"\n", r.krate, r.version);
        assert!(
            lock.contains(&entry),
            "{}: Cargo.lock resolves {} to another version than {}; a bumped \
             grammar is re-pinned here and fuzzed again",
            r.grammar,
            r.krate,
            r.version
        );
        assert!(!r.paths.is_empty(), "{}: names its paths", r.grammar);
        match r.kind.as_str() {
            "corpus" | "files" => {
                assert!(r.repository.starts_with("https://"), "{}", r.grammar);
                assert!(
                    r.commit.len() == 40 && r.commit.chars().all(|c| c.is_ascii_hexdigit()),
                    "{}: pinned to a full commit, not a tag that can move",
                    r.grammar
                );
            }
            "crate" => assert_eq!((r.repository.as_str(), r.commit.as_str()), ("-", "-")),
            other => panic!("{}: unknown kind {other}", r.grammar),
        }
    }
}

#[test]
fn e7g_the_fuzz_job_runs_on_every_change_to_the_grammar_set() {
    let wf = read(".github/workflows/grammar-fuzz.yml");
    for path in [
        "Cargo.lock",
        "Cargo.toml",
        // E7h: the C flags every grammar compiles with, the grammars
        // carried in-repo under D36's amendment, and the query overlays
        // the harness's capture walk runs.
        ".cargo/config.toml",
        "src/syntax.rs",
        "src/bin/pmacs_grammar_fuzz.rs",
        "scripts/fuzz-grammars",
        "fuzz/**",
        "vendor/**",
        "builtin/queries/**",
        ".github/workflows/grammar-fuzz.yml",
    ] {
        let listed = wf.matches(&format!("      - {path}\n")).count();
        assert_eq!(
            listed, 2,
            "{path} triggers the job on pull requests and on main"
        );
    }
    assert!(
        wf.contains("workflow_dispatch:"),
        "the long form is dispatchable"
    );
    assert!(
        wf.contains("scripts/fuzz-grammars"),
        "the job runs the script"
    );
}

#[test]
fn e7g_the_fuzz_profile_compiles_the_grammars_as_they_ship() {
    let manifest = read("Cargo.toml");
    let section = |name: &str| {
        let start = manifest
            .find(&format!("[profile.{name}]\n"))
            .unwrap_or_else(|| panic!("[profile.{name}]"));
        let body = &manifest[start..];
        body[..body[1..].find("\n[").map_or(body.len(), |i| i + 1)].to_owned()
    };
    let fuzz = section("fuzz");
    assert!(fuzz.contains("inherits = \"release\""), "{fuzz}");
    assert!(
        !fuzz.contains("opt-level"),
        "the fuzz profile keeps release's opt-level, the one the grammars ship at"
    );
    assert!(section("release").contains("opt-level = 3"));
}

#[test]
fn e7g_the_harness_drives_every_bundled_grammar() {
    let out = Command::new(env!("CARGO_BIN_EXE_pmacs_grammar_fuzz"))
        .arg("list")
        .output()
        .unwrap();
    assert!(out.status.success());
    let listed: Vec<String> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| l.split('\t').next().unwrap().to_owned())
        .collect();
    let table: Vec<&str> = BUILTIN_LANGUAGES.iter().map(|l| l.name).collect();
    assert_eq!(listed, table);
}

/// The file the fuzz run reduced tree-sitter-yaml's abort to: a mapping
/// nested `depth` levels deep, one key a line, one space an indent.
fn nested_yaml(depth: usize) -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for i in 0..depth {
        let _ = writeln!(text, "{}k{i}:", " ".repeat(i));
    }
    text
}

#[test]
fn e7h_a_yaml_file_nested_254_deep_opens_and_parses_as_yaml() {
    // tree-sitter-yaml 0.7.2 aborted the editor on nested_yaml(254) (E7g):
    // its scanner's `serialize` checks the bound before each 4-byte write
    // of its indent stack, not after, so at 254 levels it writes 1026 bytes
    // into the runtime's 1024 and the runtime's assert aborts. Under D36 as
    // amended it ships since E7h from `vendor/tree-sitter-yaml` with that
    // check made whole, after a clean 600 s fuzz run on GCC 16; the lock
    // resolving to the copy is pinned in
    // tests/e7g_review1_serialization_witnesses.rs. Here the reproduction
    // itself: at 254 and 300 levels the file is `yaml` and a YAML tree
    // settles. With the unpatched crate back, this row aborts the test
    // binary.
    for depth in [254, 300] {
        let dir =
            std::env::temp_dir().join(format!("pmacs-e7h-yaml-{depth}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("deep.yaml");
        std::fs::write(&file, nested_yaml(depth)).unwrap();
        let mut s = EditorState::new_with_roots(&iso::roots());
        s.lua_host.lua().remove_app_data::<StateDir>();
        s.lua_host.lua().set_app_data(StateDir(dir.clone()));
        let lua =
            |s: &EditorState, src: &str| s.lua_host.lua().load(src.to_owned()).exec().unwrap();
        lua(&s, "pmacs.lsp.config = {}");
        lua(
            &s,
            &format!(
                "pmacs.buffer.find_or_open({:?})",
                file.display().to_string()
            ),
        );
        let language: Option<String> = s
            .lua_host
            .lua()
            .load("return pmacs.parse.buffer_language(pmacs.window.buffer())")
            .eval()
            .unwrap();
        assert_eq!(language.as_deref(), Some("yaml"), "{depth} levels");
        let until = Instant::now() + Duration::from_secs(10);
        let mut parsed: Option<String> = None;
        while Instant::now() < until && parsed.is_none() {
            s.tick_processes();
            s.tick_lsp();
            s.tick_async();
            parsed = s
                .lua_host
                .lua()
                .load("local t = pmacs.parse.tree(pmacs.window.buffer()) return t and t:language() or nil")
                .eval()
                .unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            parsed.as_deref(),
            Some("yaml"),
            "a YAML tree settles at {depth} levels"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn e7g_the_javascript_family_stays_unshipped() {
    // tree-sitter-javascript 0.25.0 never returns from JS_HANG, 24 bytes of
    // unclosed brackets across four lines, and its RSS climbs about 17 MB a
    // second while it tries; the TypeScript and TSX grammars of
    // tree-sitter-typescript 0.23.2 do the same on it, and TSX alone on
    // TSX_HANG. tree-sitter 0.27.0's runtime does too. In the editor one
    // parse worker spins until the daemon restarts, or memory runs out.
    // E7h.5's full-length run found a third input TSX never returns from,
    // 74 bytes under 8 edits, on which TypeScript and JavaScript finish in
    // a second; it is kept, with its replay line, in
    // `fuzz/repro/typescriptreact-hang-74.input` and `fuzz/repro/README.md`.
    //
    // JavaScript stays as a dev-dependency for the local-facts witnesses'
    // fixed fixtures; re-shipping either crate needs a release clean under
    // `scripts/fuzz-grammars`, and this row deleted in that commit.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
    assert!(
        !lock.contains("name = \"tree-sitter-typescript\""),
        "Cargo.lock names tree-sitter-typescript again; see this row's comment"
    );
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let deps = manifest
        .split("\n[dependencies]\n")
        .nth(1)
        .and_then(|rest| rest.split("\n[").next())
        .expect("a [dependencies] table");
    for krate in ["tree-sitter-javascript", "tree-sitter-typescript"] {
        assert!(
            !deps.lines().any(|l| l.starts_with(krate)),
            "{krate} is a dependency again. It never returns from {JS_HANG:?} \
             (E7g); see this row's comment"
        );
    }
    let dir = std::env::temp_dir().join(format!("pmacs-e7g-js-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.clone()));
    s.lua_host
        .lua()
        .load("pmacs.lsp.config = {}")
        .exec()
        .unwrap();
    for (name, text, id) in [
        ("hang.js", JS_HANG, "javascript"),
        ("hang.tsx", TSX_HANG, "typescriptreact"),
    ] {
        let file = dir.join(name);
        std::fs::write(&file, text).unwrap();
        let (grammar, language): (Option<String>, Option<String>) = s
            .lua_host
            .lua()
            .load(format!(
                "pmacs.buffer.find_or_open({:?})
                 return pmacs.parse.language_for_path({name:?}),
                        pmacs.parse.buffer_language(pmacs.window.buffer())",
                file.display().to_string()
            ))
            .eval()
            .unwrap();
        assert_eq!(grammar, None, "no grammar claims {name}");
        assert_eq!(language.as_deref(), Some(id), "{name} is {id} by filetype");
    }
}

/// The fuzz run's JavaScript hang, as found: never returns under the
/// javascript, typescript and tsx grammars.
const JS_HANG: &str = "[t,t[t\n[at\n[ ,at\n[ ,5 ];";

/// TSX alone never returns on an open paren, thirty newlines and a close
/// bracket; twenty-nine parse in a tenth of a millisecond.
const TSX_HANG: &str = "(\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n]";
