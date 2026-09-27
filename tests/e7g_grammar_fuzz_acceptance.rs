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
//! the miscompile E7g found is invisible at -O0.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use pmacs::syntax::BUILTIN_LANGUAGES;

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
