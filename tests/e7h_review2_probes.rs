// tests/e7h_review2_probes.rs --- E7h review 2, the states the phase's own
// witnesses did not choose.

//! Two rows that fail at the reviewed head `17c3c8f`, each ignored with the
//! finding it witnesses so that the fix round un-ignores it in the commit
//! that closes it, and their live controls. E7h's fix round 2 closed both
//! (`--no-renames` in the decision, the thirteen macros in the residual row)
//! and un-ignored them.
//!
//! The workflow's decision (`scripts/grammar-fuzz-needed --base`) lists a
//! change's paths with `git diff --name-only`, which pairs a moved file as a
//! rename and prints only its new path. A commit that moves `src/syntax.rs`,
//! the grammar table, into `src/syntax/mod.rs` and edits a registration in
//! the same commit decides `run=false`, and from then on the trigger list
//! names a file that no longer exists.
//!
//! The residual row (`tests/e7g_review1_probes.rs`) flags a scanner on the
//! pre-0.24 `array.h` only when it calls `array_push`, `array_grow_by`,
//! `array_extend` or `array_insert`. That header writes `contents` through
//! an `(Array *)` cast in nine more macros: `array_reserve` and
//! `array_delete`, which bash, haskell, html and python call; `array_splice`,
//! `array_erase`, `array_assign` and `array_swap`; and `array_push_all` and
//! the two `array_insert_sorted_*`, which reach the cast through
//! `array_extend` and `array_insert` and which the row's needles
//! (`"array_push("`, `"array_insert("`) do not match. A scanner on the old
//! header that writes through any of the nine without one of the four
//! executes the same write and is not flagged. No shipped scanner does that
//! today.
//!
//! The probes behind the review that are not test targets (the arm plants,
//! the guard under a cross build's environment, the one-keystroke pty probe,
//! the decision driven by real commits) are under `tests/e7h_review2/`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=probe",
            "-c",
            "user.email=probe@invalid",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A scratch repository holding this tree's grammar table at its path and
/// one other file, committed; returns it and the commit.
const TABLE: &str = "pmacs-syntax/src/lib.rs";

fn scratch_repo() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q"]);
    // The table moved from `src/syntax.rs` into `pmacs-syntax` at E7i.
    std::fs::create_dir_all(dir.path().join("pmacs-syntax/src")).unwrap();
    std::fs::create_dir_all(dir.path().join("docs")).unwrap();
    std::fs::write(dir.path().join(TABLE), read(TABLE)).unwrap();
    std::fs::write(dir.path().join("docs/notes.md"), "notes\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    let base = git(dir.path(), &["rev-parse", "HEAD"]);
    (dir, base)
}

/// `scripts/grammar-fuzz-needed --base BASE` in `dir`, as the workflow's
/// first step runs it: the `run=` line.
fn decide(dir: &Path, base: &str) -> String {
    let out = Command::new(root().join("scripts/grammar-fuzz-needed"))
        .args(["--base", base])
        .current_dir(dir)
        .output()
        .expect("grammar-fuzz-needed");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn e7h_review2_the_decision_controls_run_on_the_table_and_skip_on_docs() {
    // Control for the row below: an edit to the table in place runs, a
    // docs-only change skips.
    let (repo, base) = scratch_repo();
    let table = repo.path().join(TABLE);
    let text = std::fs::read_to_string(&table).unwrap();
    std::fs::write(
        &table,
        text.replacen("name: \"toml\"", "name: \"toml-probe\"", 1),
    )
    .unwrap();
    git(repo.path(), &["commit", "-q", "-am", "edit the table"]);
    let verdict = decide(repo.path(), &base);
    assert!(verdict.starts_with("run=true"), "{verdict}");

    let (repo, base) = scratch_repo();
    std::fs::write(repo.path().join("docs/notes.md"), "notes, edited\n").unwrap();
    git(repo.path(), &["commit", "-q", "-am", "docs"]);
    let verdict = decide(repo.path(), &base);
    assert!(verdict.starts_with("run=false"), "{verdict}");
}

#[test]
fn e7h_review2_the_decision_runs_when_the_grammar_table_moves_and_changes() {
    let (repo, base) = scratch_repo();
    // Moved to a path that is not itself a trigger, so only the deletion
    // of the old path can make the decision run.
    std::fs::create_dir_all(repo.path().join("src")).unwrap();
    git(repo.path(), &["mv", TABLE, "src/grammars.rs"]);
    let table = repo.path().join("src/grammars.rs");
    let text = std::fs::read_to_string(&table).unwrap();
    assert!(
        text.contains("name: \"toml\""),
        "control: the table registers toml"
    );
    std::fs::write(
        &table,
        text.replacen("name: \"toml\"", "name: \"toml-probe\"", 1),
    )
    .unwrap();
    git(
        repo.path(),
        &["commit", "-q", "-am", "move the table and rename a grammar"],
    );
    let verdict = decide(repo.path(), &base);
    assert!(
        verdict.starts_with("run=true"),
        "the commit deletes pmacs-syntax/src/lib.rs, a trigger path, and changes a registration, \
         but the decision was: {verdict}"
    );
}

/// The published pre-0.24 header, tree-sitter-haskell 0.23.1's, from the
/// cargo registry (the workspace still resolves the crate's source there for
/// `fuzz/corpora.tsv`'s seeds), or `None` where the registry has no copy.
fn published_old_header() -> Option<PathBuf> {
    let home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))?;
    let src = home.join("registry/src");
    std::fs::read_dir(&src).ok()?.flatten().find_map(|index| {
        let h = index
            .path()
            .join("tree-sitter-haskell-0.23.1/src/tree_sitter/array.h");
        h.is_file().then_some(h)
    })
}

/// The `array_*` macros whose expansion passes `(Array *)(self)`, directly or
/// through another such macro (`array_push_all` through `array_extend`),
/// from the header's text: a `#define array_<name>(` line opens a macro,
/// which runs to the first line not ending in a backslash.
fn casting_macros(header: &str) -> Vec<String> {
    let mut defs: Vec<(String, String)> = Vec::new();
    let mut current: Option<(String, String)> = None;
    for line in header.lines() {
        if current.is_none()
            && let Some(rest) = line.trim_start().strip_prefix("#define array_")
        {
            let name = rest.split(['(', ' ']).next().unwrap_or("");
            current = Some((format!("array_{name}"), String::new()));
        }
        if let Some((_, body)) = current.as_mut() {
            body.push_str(line);
            body.push('\n');
            if !line.trim_end().ends_with('\\') {
                defs.push(current.take().unwrap());
            }
        }
    }
    let mut casts: Vec<bool> = defs
        .iter()
        .map(|(_, b)| b.contains("(Array *)(self)"))
        .collect();
    loop {
        let mut changed = false;
        for i in 0..defs.len() {
            if !casts[i]
                && (0..defs.len())
                    .any(|j| casts[j] && defs[i].1.contains(&format!("{}(", defs[j].0)))
            {
                casts[i] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    defs.into_iter()
        .zip(casts)
        .filter_map(|((name, _), c)| c.then_some(name))
        .collect()
}

#[test]
fn e7h_review2_the_old_header_casts_in_thirteen_macros() {
    // Control for the row below: what the old header casts through.
    let Some(header) = published_old_header() else {
        eprintln!("skipped: tree-sitter-haskell 0.23.1 is not in the cargo registry");
        return;
    };
    let macros = casting_macros(&std::fs::read_to_string(header).unwrap());
    assert_eq!(
        macros,
        [
            "array_reserve",
            "array_delete",
            "array_push",
            "array_grow_by",
            "array_push_all",
            "array_extend",
            "array_splice",
            "array_insert",
            "array_erase",
            "array_assign",
            "array_swap",
            "array_insert_sorted_with",
            "array_insert_sorted_by"
        ],
        "the pre-0.24 header's casting macros"
    );
}

#[test]
fn e7h_review2_the_residual_row_flags_every_macro_that_writes_through_the_cast() {
    let Some(header) = published_old_header() else {
        eprintln!("skipped: tree-sitter-haskell 0.23.1 is not in the cargo registry");
        return;
    };
    let macros = casting_macros(&std::fs::read_to_string(header).unwrap());
    assert!(!macros.is_empty(), "control: the header was read");
    let row = read("tests/e7g_review1_probes.rs");
    let missed: Vec<&String> = macros
        .iter()
        .filter(|m| !row.contains(&format!("\"{m}(\"")))
        .collect();
    assert!(
        missed.is_empty(),
        "the residual row does not look for {missed:?}, each of which writes `contents` \
         through `(Array *)(self)` in the pre-0.24 header"
    );
}
