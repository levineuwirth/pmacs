//! The long fuzz selection is the CLI's decision over a real Git diff.

use std::path::Path;
use std::process::Command;

struct Fixture {
    dir: tempfile::TempDir,
    base: String,
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(args)
        .current_dir(root)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut f = Self {
            dir,
            base: String::new(),
        };
        git(f.dir.path(), &["init", "--quiet"]);
        git(f.dir.path(), &["config", "core.quotePath", "true"]);
        git(f.dir.path(), &["config", "diff.renames", "true"]);
        f.write("Cargo.lock", "[[package]]\nname = \"tree-sitter\"\nversion = \"0.26.8\"\n\n[[package]]\nname = \"tree-sitter-md\"\nversion = \"0.5.3\"\n\n[[package]]\nname = \"tree-sitter-bash\"\nversion = \"0.25.1\"\n");
        f.write("fuzz/corpora.tsv", "grammar\tcrate\tversion\nmarkdown\ttree-sitter-md\t0.5.3\nmarkdown_inline\ttree-sitter-md\t0.5.3\nbash\ttree-sitter-bash\t0.25.1\n");
        f.write(
            "builtin/queries/markdown/highlights.scm",
            "; markdown overlay\n",
        );
        f.write("builtin/queries/bash/highlights.scm", "; bash overlay\n");
        f.write("vendor/tree-sitter-md/src/parser.c", "/* fixture */\n");
        git(
            f.dir.path(),
            &[
                "add",
                "Cargo.lock",
                "fuzz/corpora.tsv",
                "builtin/queries",
                "vendor",
            ],
        );
        git(f.dir.path(), &["commit", "--quiet", "-m", "fixture"]);
        f.base = git(f.dir.path(), &["rev-parse", "HEAD"]);
        f.expect(&[]);
        f
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn expect(&self, expected: &[&str]) {
        let out = Command::new(env!("CARGO_BIN_EXE_pmacs_grammar_fuzz"))
            .args(["changed", "--since", &self.base])
            .current_dir(self.dir.path())
            .output()
            .expect("the selection CLI runs");
        assert!(out.status.success(), "{out:?}");
        let text = String::from_utf8(out.stdout).unwrap();
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            expected,
            "long fuzz selection"
        );
    }
}

#[test]
fn issue336_query_edits_select_their_grammar() {
    let f = Fixture::new();
    f.write(
        "builtin/queries/markdown/highlights.scm",
        "; changed overlay\n",
    );
    f.expect(&["markdown"]);
}

#[test]
fn issue336_query_deletions_select_their_grammar() {
    let f = Fixture::new();
    std::fs::remove_file(f.dir.path().join("builtin/queries/bash/highlights.scm")).unwrap();
    f.expect(&["bash"]);
}

#[test]
fn issue336_new_query_selects_the_grammar_not_its_crate_sibling() {
    let f = Fixture::new();
    // Git normally quotes non-ASCII paths unless the diff uses -z.
    let path = "builtin/queries/markdown_inline/new λ.scm";
    f.write(path, "; new overlay\n");
    git(f.dir.path(), &["add", path]);
    f.expect(&["markdown_inline"]);
}

#[test]
fn issue336_query_rename_selects_both_grammar_directories() {
    let f = Fixture::new();
    git(
        f.dir.path(),
        &[
            "mv",
            "builtin/queries/markdown/highlights.scm",
            "builtin/queries/bash/moved.scm",
        ],
    );
    f.expect(&["bash", "markdown"]);
}

#[test]
fn issue336_queries_join_vendor_selection_and_ignore_unlisted_paths() {
    let f = Fixture::new();
    f.write(
        "builtin/queries/not-a-grammar/highlights.scm",
        "; unknown\n",
    );
    f.write("builtin/runtime/syntax.lua", "-- unrelated\n");
    git(
        f.dir.path(),
        &[
            "add",
            "builtin/queries/not-a-grammar",
            "builtin/runtime/syntax.lua",
        ],
    );
    f.expect(&[]);
    f.write("vendor/tree-sitter-md/src/parser.c", "/* changed */\n");
    f.write("builtin/queries/bash/highlights.scm", "; changed\n");
    f.expect(&["bash", "markdown", "markdown_inline"]);
}
