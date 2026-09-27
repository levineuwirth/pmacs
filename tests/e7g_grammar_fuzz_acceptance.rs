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
//! the miscompile E7g found is invisible at -O0. Two rows pin the grammar
//! the fuzz run itself found aborting the editor, YAML, out of the tree.

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
        "src/syntax.rs",
        "src/bin/pmacs_grammar_fuzz.rs",
        "scripts/fuzz-grammars",
        "fuzz/**",
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
fn e7g_tree_sitter_yaml_stays_unshipped() {
    // tree-sitter-yaml 0.7.2, the newest release, aborts the editor on
    // nested_yaml(254): its scanner's `serialize` checks the bound before
    // each 4-byte write of its indent stack, not after, so at 254 levels it
    // writes 1026 bytes into the runtime's 1024-byte buffer (2 bytes past
    // it, inside the parser's own allocation, where AddressSanitizer cannot
    // see) and returns 1026; the runtime's `assert(length <= 1024)` in
    // `ts_parser__external_scanner_serialize` then aborts. The release
    // pmacs dies with SIGABRT opening that file as `.yaml` and not as
    // `.txt`; 253 levels open fine. Upstream master has the same loop.
    //
    // Re-adding the crate needs a release whose scanner bounds that write
    // and is clean under `scripts/fuzz-grammars`; delete this row and the
    // one below in the commit that re-adds it, with the fuzz report cited.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
    assert!(
        lock.contains("name = \"tree-sitter-json\""),
        "control: the lock lists the grammars pmacs ships"
    );
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for (file, text) in [("Cargo.lock", &lock), ("Cargo.toml", &manifest)] {
        assert!(
            !text
                .lines()
                .any(|l| l.starts_with("tree-sitter-yaml") || l == "name = \"tree-sitter-yaml\""),
            "{file} names tree-sitter-yaml again. Its 0.7.2 scanner overruns \
             the serialization buffer on a file nested 254 levels deep and \
             the runtime aborts the editor (E7g); see this row's comment"
        );
    }
}

#[test]
fn e7g_a_yaml_file_nested_254_deep_opens_as_yaml_with_no_grammar() {
    // What E7g left, on the reproduction itself: the buffer is `yaml`
    // through the LSP filetype map, so yaml-language-server attaches, and
    // no grammar parses it. With the grammar back, this row aborts the
    // test binary.
    let dir = std::env::temp_dir().join(format!("pmacs-e7g-yaml-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("deep.yaml");
    std::fs::write(&file, nested_yaml(254)).unwrap();
    let mut s = EditorState::new_with_roots(&iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.clone()));
    let lua = |s: &EditorState, src: &str| s.lua_host.lua().load(src.to_owned()).exec().unwrap();
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
    assert_eq!(language.as_deref(), Some("yaml"));
    let until = Instant::now() + Duration::from_secs(1);
    while Instant::now() < until {
        s.tick_processes();
        s.tick_lsp();
        s.tick_async();
        std::thread::sleep(Duration::from_millis(5));
    }
    let tree: Option<String> = s
        .lua_host
        .lua()
        .load("local t = pmacs.parse.tree(pmacs.window.buffer()) return t and t:language() or nil")
        .eval()
        .unwrap();
    assert_eq!(tree, None, "no grammar parses a `.yaml` file");
}
