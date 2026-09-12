//! The six things the documentation is held to, and no more: D14's
//! four, D27's split of the divergence register, and the registry's
//! counts of record beside their enumerations.
//!
//! 1. README's generated status block equals `scripts/anchor --print`
//!    byte for byte, so a hand-edited version number or a stale feature
//!    list fails by name. The block carries no count: D24 took the test
//!    and suite counts out of it, and rule 2 is what keeps counts out
//!    of README altogether.
//! 2. README prose outside that block carries no protocol version and
//!    no count about the tree; numbers about the tree live in the block,
//!    where they are derived.
//! 3. `CLAUDE.md` and `AGENTS.md` are identical; the gate stages they
//!    list are a prefix of `scripts/gate --print-plan`; and the
//!    commands they attribute to `--protocol` are exactly what that
//!    flag adds to it. The prefix alone cannot see a stage inserted
//!    mid-plan, which is how the prose came to describe one added
//!    stage after the script grew two.
//! 4. No archived path is referenced from `CLAUDE.md`, `scripts/`,
//!    `tests/` or `.github/`: history is not instruction.
//! 5. `docs/invariants.md` stays under its 300-line cap, and the
//!    declared-divergence register it no longer carries exists at
//!    `docs/divergences.md` and is named by `CLAUDE.md`. The two halves
//!    grow by opposite laws — the rules must stay short enough to be
//!    read whole, the register grows with every accepted difference —
//!    so under one cap the register squeezes the rules, and a rename
//!    of either file would otherwise leave a dead route in the
//!    instruction file no other rule here can see.
//! 6. Every count of record in `docs/ci-red-signatures.md` is a
//!    `Tally (<id>): …` line beside the enumeration it counts, and the
//!    stated number equals what that enumeration holds. Four
//!    consecutive phases shipped a count there that its own table or
//!    list contradicted, each found by a reviewer recounting by hand;
//!    the recount is now this file's. A run's verdict cell is the same
//!    rule in its own shape: `N jobs: a success, b skipped, c failure`
//!    must have a + b + c = N. The fifth phase wrote a green run as
//!    `19 jobs: 17 success, 1 skipped, ZERO failures` by striking the
//!    red's failure without incrementing its successes, and no tally
//!    form stood beside a verdict cell to catch it. The rule reads the
//!    registry of the tree under test, and a `pull_request` run tests
//!    the merge of the head into `main`: the rule's own first run
//!    reddened all six test legs on `main`'s uncorrected cell, and a
//!    rerun re-executed the same merge commit and reddened again, so a
//!    registry correction on `main` reaches a PR only through a new
//!    head. A count of record is fixed on `main` before the rule that
//!    asserts it lands on a branch, or the branch carries the red.
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

/// The commands between one pair of `<!-- name:begin -->` /
/// `<!-- name:end -->` markers, fence and indentation stripped.
fn marked_commands(doc: &str, marker: &str) -> Vec<String> {
    let begin = doc
        .find(&format!("<!-- {marker}:begin -->"))
        .unwrap_or_else(|| panic!("CLAUDE.md has a {marker}:begin marker"));
    let end = doc
        .find(&format!("<!-- {marker}:end -->"))
        .unwrap_or_else(|| panic!("CLAUDE.md has a {marker}:end marker"));
    doc[begin..end]
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("```"))
        .map(str::to_owned)
        .collect()
}

/// The stages CLAUDE.md lists between its gate-plan markers are a prefix
/// of `scripts/gate --print-plan`, line for line, so the instruction
/// file cannot describe a gate the script does not run.
#[test]
fn claude_md_gate_stages_are_a_prefix_of_the_printed_plan() {
    let listed = marked_commands(&read("CLAUDE.md"), "gate-plan");
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

/// The commands CLAUDE.md attributes to `--protocol` are **exactly** the
/// lines that flag adds to the printed plan.
///
/// The default block is pinned as a *prefix*, which by construction
/// cannot see a stage inserted into the middle of the plan: when
/// `--protocol` grew `clippy-luajit` (`16fddd4`) the prefix pin stayed
/// green while this file's prose still described the flag as adding one
/// stage, the sweep. So this one is an equality and it bites in both
/// directions --- a stage dropped from the script fails it as loudly as
/// a command retyped here.
#[test]
fn claude_md_protocol_commands_are_exactly_what_the_flag_adds() {
    let listed = marked_commands(&read("CLAUDE.md"), "gate-plan-protocol");
    assert!(
        !listed.is_empty(),
        "CLAUDE.md lists at least one --protocol command"
    );
    let default = run_script("scripts/gate", &["--print-plan"]);
    let protocol = run_script("scripts/gate", &["--protocol", "--print-plan"]);
    let added: Vec<String> = protocol
        .lines()
        .filter(|line| !default.lines().any(|d| d == *line))
        .map(str::to_owned)
        .collect();
    assert_eq!(
        added,
        listed,
        "CLAUDE.md's `--protocol` commands must be exactly the lines that flag \
         adds to `scripts/gate --print-plan`.\n\
         CLAUDE.md lists:\n  {}\nthe flag adds:\n  {}",
        listed.join("\n  "),
        added.join("\n  ")
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

/// E0.2's cap, pinned rather than counted by hand. It was stated in the
/// task row and nowhere in the tree, so the file reached 298 of 300
/// lines with nothing to say so; the number the failure prints is what
/// makes the cap actionable.
#[test]
fn invariants_md_stays_under_its_line_cap() {
    const CAP: usize = 300;
    let lines = read("docs/invariants.md").lines().count();
    assert!(
        lines <= CAP,
        "docs/invariants.md is {lines} lines against E0.2's cap of {CAP}. \
         The declared-divergence register is not what to cut: it lives in \
         docs/divergences.md, which has no cap (D27)."
    );
}

/// The register has to exist and to be reachable from the instruction
/// file, or the split just loses it: `CLAUDE.md` is the one route a
/// session takes into the repository's documentation, and a rename that
/// left the name behind would be invisible to every other rule here.
#[test]
fn divergences_md_exists_and_is_named_by_the_instruction_file() {
    let register = read("docs/divergences.md");
    assert!(
        register.contains("## ") || register.contains("- **"),
        "docs/divergences.md must carry the register, not just a heading:\n{register}"
    );
    let claude = read("CLAUDE.md");
    assert!(
        claude.contains("docs/divergences.md"),
        "CLAUDE.md must name docs/divergences.md; it is the only route \
         a session has to the declared-divergence register"
    );
}

// ---------------------------------------------------------------------------
// Rule 6: a count of record sits beside its enumeration, and they agree
// ---------------------------------------------------------------------------

/// One markdown table: its header cells and its body rows, every cell
/// trimmed with the backticks and bold markers stripped, so a value can
/// be named in a tally the way the table shows it.
struct Table {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
}

fn clean_cell(cell: &str) -> String {
    cell.trim()
        .trim_matches('*')
        .trim_matches('`')
        .trim()
        .to_owned()
}

fn is_table_line(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

/// `|---|:---:|`: the row between a table's header and its body.
fn is_separator_row(line: &str) -> bool {
    let inner = line.trim().trim_matches('|');
    !inner.is_empty() && inner.chars().all(|c| matches!(c, '-' | '|' | ':' | ' '))
}

fn table_cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(clean_cell)
        .collect()
}

/// The table whose first line is `lines[start]`.
fn parse_table(lines: &[&str], start: usize) -> Option<Table> {
    if !is_table_line(lines[start]) {
        return None;
    }
    let header = table_cells(lines[start]);
    let mut i = start + 1;
    if i < lines.len() && is_separator_row(lines[i]) {
        i += 1;
    }
    let mut rows = Vec::new();
    while i < lines.len() && is_table_line(lines[i]) {
        rows.push(table_cells(lines[i]));
        i += 1;
    }
    Some(Table { header, rows })
}

fn is_tally_line(line: &str) -> bool {
    line.starts_with("Tally (")
}

/// The first line after `at` that is neither blank nor another tally,
/// so several tallies can stand together above one enumeration.
fn next_block_start(lines: &[&str], at: usize) -> usize {
    let mut i = at + 1;
    while i < lines.len() && (lines[i].trim().is_empty() || is_tally_line(lines[i])) {
        i += 1;
    }
    i
}

fn table_below(lines: &[&str], at: usize) -> Result<Table, String> {
    let i = next_block_start(lines, at);
    if i >= lines.len() {
        return Err("no table follows it".to_owned());
    }
    parse_table(lines, i).ok_or_else(|| format!("what follows it is not a table: {}", lines[i]))
}

/// The number of `- ` items in the list that follows `at`; the list ends
/// at the first blank line, and a wrapped item's continuation lines are
/// not items.
fn list_below(lines: &[&str], at: usize) -> Result<usize, String> {
    let mut i = next_block_start(lines, at);
    if i >= lines.len() || !lines[i].starts_with("- ") {
        return Err("no list follows it".to_owned());
    }
    let mut items = 0;
    while i < lines.len() && !lines[i].trim().is_empty() {
        if lines[i].starts_with("- ") {
            items += 1;
        }
        i += 1;
    }
    Ok(items)
}

/// The nearest table that ends before `at`.
fn table_above(lines: &[&str], at: usize) -> Result<Table, String> {
    let mut end = at;
    while end > 0 && !is_table_line(lines[end - 1]) {
        end -= 1;
    }
    if end == 0 {
        return Err("no table precedes it".to_owned());
    }
    let mut start = end - 1;
    while start > 0 && is_table_line(lines[start - 1]) {
        start -= 1;
    }
    parse_table(lines, start).ok_or_else(|| "no table precedes it".to_owned())
}

fn number(s: &str) -> Result<f64, String> {
    let s = s.trim().trim_matches('*').trim();
    s.parse::<f64>()
        .map_err(|_| format!("`{s}` is not a number"))
}

fn count(s: &str) -> Result<usize, String> {
    let s = s.trim().trim_matches('*').trim();
    s.parse::<usize>()
        .map_err(|_| format!("`{s}` is not a count"))
}

/// The text between the first pair of backticks in `s`, and what follows
/// the closing one.
fn backticked(s: &str) -> Result<(&str, &str), String> {
    let start = s
        .find('`')
        .ok_or_else(|| format!("no backticked name in `{s}`"))?;
    let rest = &s[start + 1..];
    let end = rest
        .find('`')
        .ok_or_else(|| format!("unclosed backtick in `{s}`"))?;
    Ok((&rest[..end], &rest[end + 1..]))
}

fn column_index(table: &Table, column: &str) -> Result<usize, String> {
    table
        .header
        .iter()
        .position(|h| h == column)
        .ok_or_else(|| {
            format!(
                "the table has no `{column}` column; it has {:?}",
                table.header
            )
        })
}

fn agree(stated: usize, found: usize, what: &str) -> Result<(), String> {
    if stated == found {
        Ok(())
    } else {
        Err(format!("states {stated} but {what} {found}"))
    }
}

/// `<lo>–<hi>[ unit]`: the two ends of a stated range.
fn split_range(lhs: &str) -> Result<(f64, f64), String> {
    let token = lhs
        .split_whitespace()
        .next()
        .ok_or_else(|| format!("no range in `{lhs}`"))?;
    let (lo, hi) = token
        .split_once('–')
        .or_else(|| token.split_once("--"))
        .or_else(|| token.split_once('-'))
        .ok_or_else(|| format!("`{token}` is not a range"))?;
    Ok((number(lo)?, number(hi)?))
}

/// The claim after `Tally (<id>):`, in one of six forms:
///
/// - `N rows in the table below`
/// - `N items in the list below`
/// - `N rows of the table above with <column> = <value>` (the column
///   and the value each backticked)
/// - `N distinct values of <column> in the table above` (backticked)
/// - `N = a + b + …`
/// - `lo–hi[ unit] over v1, v2, …`
fn check_claim(lines: &[&str], at: usize, claim: &str) -> Result<(), String> {
    if let Some((lhs, rhs)) = claim.split_once(" over ") {
        let (lo, hi) = split_range(lhs)?;
        let values = rhs
            .split(',')
            .map(number)
            .collect::<Result<Vec<f64>, String>>()?;
        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if (lo - min).abs() > 1e-9 || (hi - max).abs() > 1e-9 {
            return Err(format!(
                "states {lo}–{hi} but the {} enumerated values range {min}–{max}",
                values.len()
            ));
        }
        return Ok(());
    }
    if let Some((lhs, rhs)) = claim.split_once(" of the table above with ") {
        let stated = count(lhs.split_whitespace().next().unwrap_or(""))?;
        let (column, rest) = backticked(rhs)?;
        let (value, _) = backticked(rest.trim().strip_prefix('=').unwrap_or(rest))?;
        let table = table_above(lines, at)?;
        let col = column_index(&table, column)?;
        let found = table
            .rows
            .iter()
            .filter(|r| r.get(col).is_some_and(|c| c == value))
            .count();
        return agree(
            stated,
            found,
            &format!("the rows whose `{column}` is `{value}` number"),
        );
    }
    if let Some((lhs, rhs)) = claim
        .split_once(" distinct values of ")
        .or_else(|| claim.split_once(" distinct value of "))
    {
        let stated = count(lhs)?;
        let (column, _) = backticked(rhs)?;
        let table = table_above(lines, at)?;
        let col = column_index(&table, column)?;
        let mut seen: Vec<&String> = table.rows.iter().filter_map(|r| r.get(col)).collect();
        seen.sort();
        seen.dedup();
        return agree(
            stated,
            seen.len(),
            &format!("the distinct `{column}` values number"),
        );
    }
    if let Some(lhs) = claim
        .strip_suffix(" rows in the table below")
        .or_else(|| claim.strip_suffix(" row in the table below"))
    {
        let stated = count(lhs)?;
        let table = table_below(lines, at)?;
        return agree(stated, table.rows.len(), "the table below holds");
    }
    if let Some(lhs) = claim
        .strip_suffix(" items in the list below")
        .or_else(|| claim.strip_suffix(" item in the list below"))
    {
        let stated = count(lhs)?;
        let found = list_below(lines, at)?;
        return agree(stated, found, "the list below holds");
    }
    if let Some((lhs, rhs)) = claim.split_once(" = ") {
        let stated = count(lhs)?;
        let found = rhs
            .split('+')
            .map(count)
            .collect::<Result<Vec<usize>, String>>()?
            .iter()
            .sum();
        return agree(stated, found, "the addends sum to");
    }
    Err(format!("`{claim}` is none of the six tally forms"))
}

/// Every `Tally (<id>): …` line in `doc`, checked; the failures name
/// the line, the id and the disagreement.
struct TallyReport {
    checked: usize,
    failures: Vec<String>,
}

fn check_tallies(doc: &str) -> TallyReport {
    let lines: Vec<&str> = doc.lines().collect();
    let mut report = TallyReport {
        checked: 0,
        failures: Vec::new(),
    };
    for (n, line) in lines.iter().enumerate() {
        let Some(rest) = line.strip_prefix("Tally (") else {
            continue;
        };
        report.checked += 1;
        let Some((id, claim)) = rest.split_once("):") else {
            report
                .failures
                .push(format!("{}: a tally line without `):` --- {line}", n + 1));
            continue;
        };
        let claim = claim.trim().trim_end_matches('.').trim();
        if let Err(why) = check_claim(&lines, n, claim) {
            report
                .failures
                .push(format!("{}: Tally ({id}) {why}\n    {line}", n + 1));
        }
    }
    report
}

/// A job-count word: digits, or the spelled-out small counts the
/// registry writes in capitals for emphasis (`ZERO failures`).
fn job_count(s: &str) -> Result<usize, String> {
    let s = s.trim().trim_matches('*').trim();
    match s.to_ascii_lowercase().as_str() {
        "zero" | "no" => Ok(0),
        "one" => Ok(1),
        "two" => Ok(2),
        "three" => Ok(3),
        _ => count(s),
    }
}

/// Every `N jobs: a <label>, b <label>, …` phrase in `doc` — a run's
/// verdict cell, or an attempt row's — checked: the parts must sum to
/// `N`. The phrase ends at the cell's closing `|`, a `;`, or the end of
/// the line; bold markers are ignored. A phrase whose parts do not
/// parse as `<count> <label>` pairs is itself a failure, so a cell in a
/// shape this cannot read is named rather than skipped.
fn check_job_cells(doc: &str) -> TallyReport {
    let mut report = TallyReport {
        checked: 0,
        failures: Vec::new(),
    };
    for (n, line) in doc.lines().enumerate() {
        let mut rest = line;
        while let Some(at) = rest.find(" jobs:") {
            let before = &rest[..at];
            let after = &rest[at + " jobs:".len()..];
            rest = after;
            let Some(total) = before
                .rsplit(|c: char| c.is_whitespace() || c == '|')
                .next()
                .and_then(|tok| job_count(tok).ok())
            else {
                continue;
            };
            let end = after.find(['|', ';']).unwrap_or(after.len());
            let phrase = after[..end].trim().trim_matches('*').trim();
            let parts: Vec<&str> = phrase.split(',').map(str::trim).collect();
            report.checked += 1;
            let mut sum = 0usize;
            let mut bad = None;
            for part in &parts {
                let mut words = part.split_whitespace();
                if let (Some(Ok(k)), Some(_)) = (words.next().map(job_count), words.next()) {
                    sum += k;
                } else {
                    bad = Some(*part);
                    break;
                }
            }
            if let Some(part) = bad {
                report.failures.push(format!(
                    "{}: `{part}` is not a `<count> <label>` part of a jobs cell\n    {line}",
                    n + 1
                ));
            } else if parts.len() < 2 || sum != total {
                report.failures.push(format!(
                    "{}: {total} jobs but the parts sum to {sum}\n    {line}",
                    n + 1
                ));
            }
        }
    }
    report
}

/// The registry's counts of record each sit beside the enumeration
/// they count, as a `Tally (<id>): …` line, and the stated number
/// equals what the enumeration holds. Four consecutive phases shipped a
/// count in that file its own enumeration contradicted (E2's family
/// at eleven over twelve rows; E3's `daemon_reships…` at three over
/// four; E4's #259 at 35–47 polls over samples of 16, 38, 47 and 35),
/// each caught by a reviewer recounting by hand; this makes the
/// recount the test's.
#[test]
fn ci_red_registry_counts_equal_their_enumerations() {
    let report = check_tallies(&read("docs/ci-red-signatures.md"));
    assert!(
        report.checked > 0,
        "docs/ci-red-signatures.md carries no `Tally (<id>):` line; \
         its counts of record are written as tallies beside their enumerations"
    );
    assert!(
        report.failures.is_empty(),
        "a count of record in docs/ci-red-signatures.md disagrees with its enumeration:\n{}",
        report.failures.join("\n")
    );
}

/// Every run verdict in the registry sums: `N jobs: a success, b
/// skipped, c failure` has a + b + c = N. E5 wrote the tip run
/// 34528196810 as `19 jobs: 17 success, 1 skipped, ZERO failures`
/// (it is 18, 1, 0) in the registry, the PR body and three vault
/// records at once, carrying the red head's 17 forward with its
/// failure struck; the sum is what a reader had to do by hand.
#[test]
fn ci_red_registry_job_cells_sum() {
    let report = check_job_cells(&read("docs/ci-red-signatures.md"));
    assert!(
        report.checked > 0,
        "docs/ci-red-signatures.md carries no `N jobs: …` cell; run verdicts are written in that form"
    );
    assert!(
        report.failures.is_empty(),
        "a run verdict in docs/ci-red-signatures.md does not sum:\n{}",
        report.failures.join("\n")
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

    /// One of each tally form, every one agreeing with its enumeration.
    const TALLIES: &str = "\
Tally (rows): 2 rows in the table below.

| run | who |
|---|---|
| 1 | `x` |
| 2 | **y** |

Tally (distinct): 2 distinct values of `who` in the table above.
Tally (with): 1 row of the table above with `who` = `x`.

Tally (items): 3 items in the list below.

- one
- two
  wrapped onto a second line
- three

Tally (sum): 5 = 2 + 3.

Tally (range): 1.5–4 s over 4, 1.5, 2.
";

    #[test]
    fn every_tally_form_is_checked_and_agrees() {
        let report = check_tallies(TALLIES);
        assert_eq!(report.checked, 6);
        assert!(report.failures.is_empty(), "{}", report.failures.join("\n"));
    }

    /// Each form, off by one, fails naming its id and nothing else.
    #[test]
    fn a_tally_that_disagrees_with_its_enumeration_fails_by_name() {
        let cases = [
            ("rows", "2 rows in", "3 rows in"),
            ("distinct", "2 distinct", "1 distinct"),
            ("with", "1 row of", "2 rows of"),
            ("items", "3 items", "2 items"),
            ("sum", "5 = 2 + 3", "6 = 2 + 3"),
            ("range", "1.5–4 s", "2–4 s"),
        ];
        for (id, good, bad) in cases {
            let doc = TALLIES.replacen(good, bad, 1);
            assert_ne!(doc, TALLIES, "{id}: the mutation must apply");
            let report = check_tallies(&doc);
            assert_eq!(
                report.failures.len(),
                1,
                "{id}: exactly one tally fails; got {:?}",
                report.failures
            );
            assert!(
                report.failures[0].contains(&format!("Tally ({id})")),
                "{id}: the failure names its tally; got {}",
                report.failures[0]
            );
        }
    }

    /// The three shapes the registry writes a run's job count in, every
    /// one summing; the checker reads all three.
    const JOB_CELLS: &str = "\
| verdict | 19 jobs: **18 success, 1 skipped, ZERO failures** |
| attempt 1 | created 14:40:38Z, closed 15:10:57Z; 19 jobs: **17 success, 1 skipped, 1 cancelled** |
| verdict | 18 jobs: **16 success, 1 failure, 1 skipped** |
";

    #[test]
    fn every_job_cell_shape_is_checked_and_sums() {
        let report = check_job_cells(JOB_CELLS);
        assert_eq!(report.checked, 3);
        assert!(report.failures.is_empty(), "{}", report.failures.join("\n"));
    }

    /// The cell `2a7f656` shipped for run 34528196810, verbatim: the
    /// red head's 17 carried forward with the failure struck. It fails
    /// naming its line and the sum it reached.
    #[test]
    fn the_pre_fix_tip_cell_fails_by_its_sum() {
        let doc = JOB_CELLS.replacen(
            "**18 success, 1 skipped, ZERO failures**",
            "**17 success, 1 skipped, ZERO failures**",
            1,
        );
        assert_ne!(doc, JOB_CELLS, "the mutation must apply");
        let report = check_job_cells(&doc);
        assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
        assert!(
            report.failures[0].starts_with("1: 19 jobs but the parts sum to 18"),
            "the failure names the line and the sum; got {}",
            report.failures[0]
        );
    }

    /// A cell whose parts are not `<count> <label>` pairs is named, not
    /// skipped, so a reshaped cell cannot slip past the sum.
    #[test]
    fn an_unreadable_job_cell_is_a_failure() {
        let report = check_job_cells("| verdict | 19 jobs: all green |\n");
        assert_eq!(report.checked, 1);
        assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
        assert!(report.failures[0].contains("is not a `<count> <label>` part"));
    }

    /// A tally with nothing under it, or in no known form, is itself a
    /// failure rather than a silent pass.
    #[test]
    fn a_tally_without_an_enumeration_or_in_no_known_form_fails() {
        let orphan = check_tallies("Tally (o): 2 rows in the table below.\n\nprose\n");
        assert_eq!(orphan.failures.len(), 1, "{:?}", orphan.failures);
        let unknown = check_tallies("Tally (u): two of them.\n");
        assert_eq!(unknown.failures.len(), 1, "{:?}", unknown.failures);
        assert!(unknown.failures[0].contains("none of the six tally forms"));
    }
}
