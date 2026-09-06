//! The four things the documentation is held to, and no more.
//!
//! 1. README's generated status block equals `scripts/anchor --print`
//!    byte for byte, so a hand-edited version number or a stale feature
//!    list fails by name. The block carries no count: D24 took the test
//!    and suite counts out of it, and rule 2 is what keeps counts out
//!    of README altogether.
//! 2. README prose outside that block carries no protocol version and
//!    no count about the tree; numbers about the tree live in the block,
//!    where they are derived.
//! 3. `CLAUDE.md` and `AGENTS.md` are identical, and the gate stages
//!    they list are a prefix of `scripts/gate --print-plan`.
//! 4. No archived path is referenced from `CLAUDE.md`, `scripts/`,
//!    `tests/` or `.github/`: history is not instruction.
//!
//! Each assertion prints the offending line, so a red names its cause.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn run_script(rel: &str, args: &[&str]) -> String {
    let out = Command::new(repo_root().join(rel))
        .args(args)
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| panic!("run {rel}: {e}"));
    assert!(
        out.status.success(),
        "{rel} {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8")
}

/// The block between the anchor markers, markers included.
fn anchor_block(readme: &str) -> String {
    let begin = readme
        .find("<!-- anchor:begin -->")
        .expect("README has an anchor:begin marker");
    let end = readme
        .find("<!-- anchor:end -->")
        .expect("README has an anchor:end marker");
    assert!(begin < end, "anchor:begin must precede anchor:end");
    readme[begin..end + "<!-- anchor:end -->".len()].to_owned() + "\n"
}

/// Every row of the block is derived from a file in the tree, so the
/// comparison holds on every platform and every feature flavor. It did
/// not while the block carried a test count, which is a property of the
/// machine: that row was compared only under Linux and the default
/// features, and was the one row this test could not hold anyone to.
#[test]
fn readme_status_block_equals_the_anchor_script_output() {
    let readme = read("README.md");
    let block = anchor_block(&readme);
    let printed = run_script("scripts/anchor", &["--print"]);
    let expected: Vec<&str> = printed.lines().collect();
    let actual: Vec<&str> = block.lines().collect();
    assert_eq!(
        actual, expected,
        "README's anchor block differs from `scripts/anchor --print`; run \
         `scripts/anchor --write` and commit the result"
    );
}

/// The prose outside the block names no protocol version (`v20`, `v6
/// through v21`, `protocol version 25`) and no count about the tree
/// (`4,142 tests`, `112 suites`, `45 pins`). The block carries those.
#[test]
fn readme_prose_carries_no_protocol_version_and_no_tree_count() {
    let readme = read("README.md");
    let block = anchor_block(&readme);
    let prose = readme.replace(&block, "");
    let version = regex_lite(r"\bv\d{1,3}\b");
    let count =
        regex_lite(r"\b\d[\d,]*\s+(tests?|suites?|pins?|commands?|settings?|lines?|targets?)\b");
    let mut offending = Vec::new();
    for (n, line) in prose.lines().enumerate() {
        if version.is_match(line) || count.is_match(line) {
            offending.push(format!("{}: {line}", n + 1));
        }
    }
    assert!(
        offending.is_empty(),
        "README prose must not state a protocol version or a tree count; \
         put the fact in the anchor block instead:\n{}",
        offending.join("\n")
    );
}

#[test]
fn claude_md_and_agents_md_are_identical() {
    assert_eq!(
        read("CLAUDE.md"),
        read("AGENTS.md"),
        "CLAUDE.md and AGENTS.md must be byte-identical"
    );
}

/// The stages CLAUDE.md lists between its gate-plan markers are a prefix
/// of `scripts/gate --print-plan`, line for line, so the instruction
/// file cannot describe a gate the script does not run.
#[test]
fn claude_md_gate_stages_are_a_prefix_of_the_printed_plan() {
    let claude = read("CLAUDE.md");
    let begin = claude
        .find("<!-- gate-plan:begin -->")
        .expect("CLAUDE.md has a gate-plan:begin marker");
    let end = claude
        .find("<!-- gate-plan:end -->")
        .expect("CLAUDE.md has a gate-plan:end marker");
    let listed: Vec<String> = claude[begin..end]
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("```"))
        .map(str::to_owned)
        .collect();
    assert!(!listed.is_empty(), "CLAUDE.md lists at least one stage");
    let plan = run_script("scripts/gate", &["--print-plan"]);
    let printed: Vec<&str> = plan.lines().collect();
    assert!(
        printed.len() >= listed.len() && printed[..listed.len()] == listed[..],
        "CLAUDE.md's gate stages must be a prefix of `scripts/gate --print-plan`.\n\
         CLAUDE.md lists:\n  {}\nthe script prints:\n  {}",
        listed.join("\n  "),
        printed.join("\n  ")
    );
}

/// History is not instruction: nothing under `docs/archive/`, and none
/// of the names that moved there, is referenced from the places a
/// session reads or runs.
#[test]
fn no_archived_path_is_referenced_from_instructions_scripts_tests_or_ci() {
    let patterns = [
        "docs/archive/",
        "docs/agent-handoff.md",
        "docs/active-work.md",
        "-framing.md",
    ];
    let out = Command::new("git")
        .args(["grep", "-n", "-F"])
        .args(patterns.iter().flat_map(|p| ["-e", p]))
        .args([
            "--",
            "CLAUDE.md",
            "AGENTS.md",
            "scripts",
            "tests",
            ".github",
        ])
        .current_dir(repo_root())
        .output()
        .expect("git grep");
    let hits = String::from_utf8_lossy(&out.stdout);
    // This file names the patterns it forbids; exclude its own lines.
    let hits: Vec<&str> = hits
        .lines()
        .filter(|l| !l.starts_with("tests/docs_consistency.rs:"))
        .collect();
    assert!(
        hits.is_empty(),
        "archived documents are referenced from instruction, script, test or CI files:\n{}",
        hits.join("\n")
    );
}

/// A tiny matcher for the two shapes this file needs, so the test does
/// not pull the `regex` crate into every test binary's dependency graph
/// for two patterns. `\b`, `\d`, `{m,n}`, `[\d,]*`, `\s+`, alternation
/// in parentheses and a trailing `?` on a literal are all it handles.
struct Lite {
    version: bool,
}

fn regex_lite(pattern: &str) -> Lite {
    Lite {
        version: pattern.starts_with(r"\bv"),
    }
}

impl Lite {
    fn is_match(&self, line: &str) -> bool {
        if self.version {
            version_in(line)
        } else {
            count_in(line)
        }
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `v` followed by one to three digits, as a whole word.
fn version_in(line: &str) -> bool {
    let chars: Vec<char> = line.chars().collect();
    for i in 0..chars.len() {
        if chars[i] != 'v' {
            continue;
        }
        if i > 0 && is_word(chars[i - 1]) {
            continue;
        }
        let digits = chars[i + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count();
        if (1..=3).contains(&digits) {
            let after = chars.get(i + 1 + digits);
            // `v1.1.0` is a release name, not a protocol version; a full
            // stop after the digits (`currently v20.`) is not.
            let release_name =
                after == Some(&'.') && chars.get(i + 2 + digits).is_some_and(char::is_ascii_digit);
            if after.is_none_or(|c| !is_word(*c)) && !release_name {
                return true;
            }
        }
    }
    false
}

/// A number followed by a counting noun about the tree.
fn count_in(line: &str) -> bool {
    const NOUNS: [&str; 7] = [
        "test", "suite", "pin", "command", "setting", "line", "target",
    ];
    let words: Vec<&str> = line.split_whitespace().collect();
    for pair in words.windows(2) {
        let number = pair[0].trim_matches(|c: char| !c.is_ascii_digit() && c != ',');
        if number.is_empty() || !number.chars().next().unwrap().is_ascii_digit() {
            continue;
        }
        if !number.chars().all(|c| c.is_ascii_digit() || c == ',') {
            continue;
        }
        let noun = pair[1]
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_ascii_lowercase();
        if NOUNS.iter().any(|n| noun == *n || noun == format!("{n}s")) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod matcher {
    use super::*;

    #[test]
    fn version_words_are_caught_and_release_names_are_not() {
        assert!(version_in("a typed protocol (currently v20)"));
        assert!(version_in("from v6 through **v21**"));
        assert!(version_in("in the Emacs tradition, currently v20."));
        assert!(!version_in("v1.1.0 shipped"));
        assert!(!version_in("the vault"));
        assert!(!version_in("pmacs-gpu --version reports"));
    }

    #[test]
    fn tree_counts_are_caught_and_ordinary_numbers_are_not() {
        assert!(count_in("carries 45 pins over steps"));
        assert!(count_in("4,142 tests in 121 suites"));
        assert!(!count_in("glibc 2.35 or newer"));
        assert!(!count_in("Ubuntu 22.04"));
        assert!(!count_in("an 8-column tab projection"));
    }
}
