//! `scripts/gate` — the behaviour a shell script can be held to.
//!
//! Framing: `docs/gate-script-framing.md` §4 (revision 4, approved).
//!
//! # Why these tests exist, and why they are shaped like this
//!
//! The script exists to make two things unforgettable: a per-worktree
//! `CARGO_TARGET_DIR` (because cargo locks it exclusively, so shared
//! target directories make parallel worktrees *slower* than serial),
//! and the fixed gate suite itself, which had been retyped by hand and
//! gotten wrong twice in one session.
//!
//! # The recursion constraint shapes what is testable
//!
//! A test that ran `scripts/gate` for real would run the whole gate
//! suite **inside** the gate suite. So every test here drives a path
//! that **runs no gates** — which is stricter than "non-mutating", and
//! is why `--init` exists: asserting the ownership marker is written
//! needs something that *writes* it, a pure printer cannot, and a real
//! gate run must not. `--init` shares the gate path's routine, so this
//! is not a second implementation being tested.
//!
//! # The real managed root is unreachable from here
//!
//! Every test sets `PMACS_GATE_TARGET_ROOT` to a `tempdir`. That
//! override exists for this file. Nothing here can touch
//! `~/build/pmacs-gate-targets`, which matters most for the prune
//! tests — a prune bug is unrecoverable.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn gate() -> PathBuf {
    repo_root().join("scripts/gate")
}

/// Run `scripts/gate` with an isolated managed root, from `cwd`.
fn run_in(cwd: &Path, root: &Path, args: &[&str]) -> (String, String, bool) {
    let out = Command::new(gate())
        .args(args)
        .current_dir(cwd)
        .env("PMACS_GATE_TARGET_ROOT", root)
        .output()
        .expect("run scripts/gate");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

fn run(root: &Path, args: &[&str]) -> (String, String, bool) {
    run_in(&repo_root(), root, args)
}

// --- The plan matches handoff §3 ----------------------------------------
//
// This is the direct test of the framing's named drift risk (Q#GS2):
// the script is authoritative for the FIXED gates, so if it drifts from
// §3, nothing else in the repository would notice. `--print-plan`
// exists to make that checkable without executing anything.

#[test]
fn the_plan_sweeps_the_workspace_and_never_only_the_tests() {
    let root = tempfile::tempdir().expect("tempdir");
    let (plan, _, ok) = run(root.path(), &["--print-plan"]);
    assert!(ok, "--print-plan must succeed");

    assert!(
        plan.contains("cargo test --workspace --no-fail-fast -- --skip basedpyright"),
        "the sweep must be --workspace; plan was:\n{plan}"
    );
    // The specific mistake §3 warns about: `--tests` selects 108 targets
    // where `--workspace` selects 110, dropping `pmacs_protocol` and
    // `pmacs_gpu`. A lane that had just written protocol tests swept
    // without running them.
    assert!(
        !plan.contains("--tests"),
        "`--tests` silently drops the protocol and GPU crates; plan was:\n{plan}"
    );
    assert!(
        plan.contains("cargo fmt --check")
            && plan.contains("cargo clippy --workspace --all-targets -- -D warnings")
            && plan.contains("git diff --check"),
        "plan was:\n{plan}"
    );
}

#[test]
fn the_plan_runs_the_library_tests_in_both_feature_configurations() {
    let root = tempfile::tempdir().expect("tempdir");
    let (plan, _, _) = run(root.path(), &["--print-plan"]);
    assert!(plan.contains("cargo test --lib\n"), "plan was:\n{plan}");
    assert!(
        plan.contains("cargo test --lib --features crdt"),
        "the CRDT LIBRARY tests are unconditional — only the crdt \
         WORKSPACE sweep is gated on --protocol; plan was:\n{plan}"
    );
}

/// §3: "Touching `PROTOCOL_VERSION` STRENGTHENS the sweep line. It does
/// not replace it." So `--protocol` must *add* a sweep, leaving the
/// default one in place — and the default run must not carry it.
#[test]
fn the_crdt_workspace_sweep_is_added_by_protocol_and_absent_without_it() {
    let root = tempfile::tempdir().expect("tempdir");
    let crdt_sweep = "cargo test --workspace --features crdt --no-fail-fast";

    let (default_plan, _, _) = run(root.path(), &["--print-plan"]);
    assert!(
        !default_plan.contains(crdt_sweep),
        "a normal lane must not pay for the CRDT workspace sweep; plan was:\n{default_plan}"
    );

    let (proto_plan, _, _) = run(root.path(), &["--protocol", "--print-plan"]);
    assert!(
        proto_plan.contains(crdt_sweep),
        "--protocol must add the CRDT workspace sweep; plan was:\n{proto_plan}"
    );
    assert!(
        proto_plan.contains("cargo test --workspace --no-fail-fast -- --skip basedpyright"),
        "STRENGTHENS, not replaces — the default sweep must survive; plan was:\n{proto_plan}"
    );
}

/// **The precondition the plan did not encode**, and the reason a green
/// `--protocol` run could mean nothing.
///
/// The crdt workspace sweep spawns `pmacs-gpu` as a *process*, and no
/// `cargo test` run produces that binary — `pmacs-gpu` has no `tests/`
/// directory, so cargo never uplifts its bin to `debug/pmacs-gpu`. On a
/// cold target directory the sweep fails twelve
/// `gpu_invocation_acceptance::crdt::*` tests on *"build pmacs-gpu
/// before this acceptance suite"*. Before per-worktree target
/// directories (#225) every worktree shared one that nearly always
/// already held the binary, so the precondition was satisfied **by
/// accident** — and the hazard was never the red gate, it was a green
/// one decided by the build directory rather than by the diff.
///
/// **The exact command is asserted, not just the step's name and
/// position.** A `build-crdt` running plain `cargo build` would sit in
/// the right place under the right name and leave the gate exactly as
/// unsound: the crdt sweep needs *those* features, and the wrong ones
/// produce a binary the sweep cannot use.
#[test]
fn the_crdt_sweep_is_immediately_preceded_by_the_build_that_produces_its_binary() {
    let root = tempfile::tempdir().expect("tempdir");
    let build = "cargo build --workspace --no-default-features --features luajit,crdt";
    let crdt_sweep = "cargo test --workspace --features crdt --no-fail-fast -- --skip basedpyright";

    let (plan, err, ok) = run(root.path(), &["--protocol", "--print-plan"]);
    assert!(ok, "--protocol --print-plan must succeed; stderr:\n{err}");

    let b = plan
        .find(build)
        .unwrap_or_else(|| panic!("the crdt sweep's build is missing; plan was:\n{plan}"));
    let s = plan
        .find(crdt_sweep)
        .unwrap_or_else(|| panic!("the crdt sweep is missing; plan was:\n{plan}"));

    // Ordering is asserted BEFORE the slice below, which would
    // otherwise panic with a byte-offset message ("begin > end (427 >
    // 282)") that names neither step. Mutation-tested: emitting the
    // build *after* the sweep produced exactly that, and a gate test
    // whose failure has to be decoded is a gate test nobody trusts.
    assert!(
        b < s,
        "the build must run BEFORE the crdt sweep, not after it — a sweep \
         that builds its own precondition afterwards has already failed; \
         plan was:\n{plan}"
    );
    // IMMEDIATELY before: one newline between them and nothing else. A
    // build that merely appears *somewhere* earlier could be separated
    // from the sweep by a step that rewrites the same target directory.
    assert_eq!(
        &plan[b + build.len()..s],
        "\n",
        "the build must run IMMEDIATELY before the crdt sweep; plan was:\n{plan}"
    );
}

/// **Conditionality, settled by measurement rather than by reading** —
/// which is the whole methodological point of this lane, since the
/// defect it repairs was a precondition nobody checked.
///
/// Measured 2026-08-09 on a disposable target directory, with
/// `debug/pmacs-gpu` asserted **absent** before each run and each sweep
/// run alone from that same cold state: the default sweep exited **0**
/// and left `debug/pmacs-gpu` **still absent** — it never builds the
/// binary and never needs it — while the crdt sweep exited **101** with
/// exactly twelve `gpu_invocation_acceptance::crdt::*` failures.
///
/// So an unconditional build would be a real cost paid for nothing on
/// every ordinary lane.
#[test]
fn the_crdt_build_is_absent_without_protocol() {
    let root = tempfile::tempdir().expect("tempdir");
    let (plan, _, ok) = run(root.path(), &["--print-plan"]);
    assert!(ok, "--print-plan must succeed");
    assert!(
        !plan.contains("cargo build"),
        "the default sweep passes on a tree with no pmacs-gpu at all, so a \
         normal lane must not pay for a workspace build; plan was:\n{plan}"
    );
}

/// **The attribution and continuation criteria, made observable.**
///
/// Everything else in this file drives a no-gates path, so it can prove
/// a step's name and its order and **nothing** about what the runner
/// does when a step fails. `--self-test` closes that gap by handing the
/// *real* runner loop a hardcoded three-line plan — a passing step, a
/// failing one named `build-crdt`, and a passing sentinel after it.
///
/// **Why `build-crdt` must be its own step** is exactly what this
/// witnesses: folded into the sweep as `cargo build … && cargo test …`,
/// a *build* failure would be reported under the name `sweep-crdt` — a
/// wrong attribution in the one place this script exists to be
/// trustworthy about.
///
/// **The sentinel assertion is the load-bearing one.** With the failure
/// last, a runner that aborts and one that continues produce identical
/// output, so a two-line witness would pass on a runner doing the
/// opposite of the stated `--no-fail-fast` policy. The sentinel's own
/// log existing is the only thing that separates them — delete that
/// assertion and this test stops testing continuation at all.
///
/// The plan is a literal inside the script on purpose. Making
/// `PLAN_FILE` injectable would let this test supply its own commands,
/// and would turn the runner's `eval` into a general command executor —
/// the same defect the `--acceptance` refusal above exists to prevent.
#[test]
fn self_test_names_the_failing_gate_and_the_suite_continues_past_it() {
    let root = tempfile::tempdir().expect("tempdir");
    let (out, err, ok) = run(root.path(), &["--self-test"]);

    assert!(
        !ok,
        "a plan containing a failing step must exit non-zero; stdout:\n{out}stderr:\n{err}"
    );
    assert!(
        out.contains("build-crdt"),
        "the failing gate must be named as it runs; stdout:\n{out}"
    );
    assert!(
        err.contains("FAILED: build-crdt"),
        "the failing gate must be listed under FAILED: by its OWN name; stderr:\n{err}"
    );

    // The runner claims a log path for the failure. Assert the file is
    // actually there: a tool that prints a path it did not write is
    // worse than one that prints nothing, because the absence is only
    // discovered while chasing a real failure.
    let claimed = err
        .lines()
        .find_map(|l| l.split_once("log: ").map(|(_, path)| path.trim()))
        .unwrap_or_else(|| panic!("the failing gate's log path must be printed; stderr:\n{err}"));
    assert!(
        claimed.ends_with("02-build-crdt.log"),
        "the log must be numbered and named for the gate that failed; was {claimed}"
    );
    assert!(
        Path::new(claimed).is_file(),
        "the runner must WRITE the log it claims at {claimed}"
    );

    let logdir = Path::new(claimed)
        .parent()
        .expect("the log lives in a log directory");
    assert!(
        logdir.join("01-self-pass.log").is_file(),
        "the step before the failure must have its own log; dir was {}",
        logdir.display()
    );
    // THE ASSERTION THE WHOLE MODE EXISTS FOR.
    assert!(
        logdir.join("03-self-sentinel.log").is_file(),
        "the suite must CONTINUE past a failed gate — the sentinel after \
         build-crdt wrote no log, so this runner ABORTED. Stdout:\n{out}"
    );
    assert!(
        out.contains("self-sentinel"),
        "the sentinel must be reported like any other gate; stdout:\n{out}"
    );
}

/// The seam handoff §3 keeps authority over: a script cannot infer
/// which acceptance suites a change touched, so it runs what it is
/// handed — each one, in order.
#[test]
fn acceptance_suites_reach_the_plan_in_the_order_given() {
    let root = tempfile::tempdir().expect("tempdir");
    let (plan, _, _) = run(
        root.path(),
        &[
            "--acceptance",
            "alpha_acceptance",
            "--acceptance",
            "beta_acceptance",
            "--print-plan",
        ],
    );
    let a = plan
        .find("cargo test --test alpha_acceptance")
        .unwrap_or_else(|| panic!("alpha missing from plan:\n{plan}"));
    let b = plan
        .find("cargo test --test beta_acceptance")
        .unwrap_or_else(|| panic!("beta missing from plan:\n{plan}"));
    assert!(
        a < b,
        "suites must keep their given order; plan was:\n{plan}"
    );
}

// --- Derivation, marker, canonical paths --------------------------------

#[test]
fn printing_the_target_dir_creates_nothing() {
    let root = tempfile::tempdir().expect("tempdir");
    let (dir, _, ok) = run(root.path(), &["--print-target-dir"]);
    assert!(ok, "--print-target-dir must succeed");
    assert!(!dir.trim().is_empty(), "it must print a path");
    assert!(
        !Path::new(dir.trim()).exists(),
        "--print-target-dir must be pure — it printed {dir} and created it"
    );
}

#[test]
fn init_writes_the_ownership_marker_and_is_idempotent() {
    let root = tempfile::tempdir().expect("tempdir");
    let (dir, _, ok) = run(root.path(), &["--init"]);
    assert!(ok, "--init must succeed");
    let dir = PathBuf::from(dir.trim());

    let marker = dir.join(".pmacs-gate-target");
    assert!(marker.is_file(), "the ownership marker must exist");
    let owner = std::fs::read_to_string(&marker).expect("read marker");
    // Canonical form (§2.5): what prune compares against.
    let expected = repo_root().canonicalize().expect("canonicalize repo root");
    assert_eq!(
        owner.trim(),
        expected.to_string_lossy(),
        "the marker must record the CANONICAL worktree path"
    );

    run(root.path(), &["--init"]);
    let n = std::fs::read_dir(root.path())
        .expect("read root")
        .filter(|e| e.as_ref().is_ok_and(|e| e.path().is_dir()))
        .count();
    assert_eq!(n, 1, "--init must be idempotent");
}

/// **The case that deletes a live lane's artifacts if derivation is not
/// canonical.** Reaching one worktree through a symlink must derive the
/// same directory. If the hash came from an uncanonicalized `$PWD`, the
/// symlinked spelling would derive a *different* directory whose marker
/// records the *canonical* path — a second build directory for a live
/// worktree, indistinguishable from an orphan.
///
/// **This currently passes for a reason the script does not control**,
/// and saying so is more useful than implying otherwise: measured here,
/// `git rev-parse --show-toplevel` already returns a resolved physical
/// path, so the derivation is canonical before `canon()` touches it.
/// Removing `canon()` does not make this test fail today. It pins the
/// **property**, which is what must hold — not the mechanism, which is
/// belt-and-braces against git's behaviour not being contractual.
#[test]
fn a_symlinked_spelling_of_a_worktree_derives_the_same_directory() {
    let root = tempfile::tempdir().expect("tempdir");
    let link_home = tempfile::tempdir().expect("tempdir");
    let link = link_home.path().join("via-symlink");
    if std::os::unix::fs::symlink(repo_root(), &link).is_err() {
        return; // no symlink support; nothing to assert
    }

    let (direct, _, _) = run(root.path(), &["--print-target-dir"]);
    let (through_link, _, _) = run_in(&link, root.path(), &["--print-target-dir"]);
    assert_eq!(
        direct.trim(),
        through_link.trim(),
        "two spellings of one worktree must share one build directory"
    );
}

// --- Pruning ------------------------------------------------------------

/// Build a managed root holding three entries: one orphan (eligible),
/// one unmarked look-alike, and one owned by a live worktree.
fn prune_fixture(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let orphan = root.join("gone-00000000");
    std::fs::create_dir_all(&orphan).expect("mkdir orphan");
    std::fs::write(
        orphan.join(".pmacs-gate-target"),
        format!("{}\n", root.join("no-such-worktree").display()),
    )
    .expect("write orphan marker");

    let lookalike = root.join("pmacs-deadbeef");
    std::fs::create_dir_all(&lookalike).expect("mkdir lookalike");

    let (live, _, _) = run(root, &["--init"]);
    (orphan, lookalike, PathBuf::from(live.trim()))
}

#[test]
fn prune_is_a_dry_run_by_default_and_deletes_nothing() {
    let root = tempfile::tempdir().expect("tempdir");
    let (orphan, lookalike, live) = prune_fixture(root.path());

    let (out, _, ok) = run(root.path(), &["--prune"]);
    assert!(ok, "--prune must succeed");
    assert!(
        out.contains("WOULD delete") && out.contains(&orphan.to_string_lossy().to_string()),
        "the orphan must be named; output was:\n{out}"
    );
    assert!(
        orphan.exists() && lookalike.exists() && live.exists(),
        "a dry run must delete nothing"
    );
}

#[test]
fn force_deletes_only_the_orphan() {
    let root = tempfile::tempdir().expect("tempdir");
    let (orphan, lookalike, live) = prune_fixture(root.path());

    let (out, _, ok) = run(root.path(), &["--prune", "--force"]);
    assert!(ok, "output was:\n{out}");
    assert!(!orphan.exists(), "the orphan must be gone");
    assert!(
        lookalike.exists(),
        "a directory that merely RESEMBLES a managed one must never be touched"
    );
    assert!(live.exists(), "a live worktree's directory must survive");
}

/// **A `prunable` worktree record counts as DEAD**, and nothing else in
/// this suite would catch getting it wrong.
///
/// `git worktree list --porcelain` keeps reporting a worktree that was
/// registered but whose directory was deleted without
/// `git worktree remove` — it adds a `prunable <reason>` line to that
/// record. Treating every *listed* path as live would make exactly the
/// directories most worth reclaiming permanently ineligible, silently.
///
/// The other prune tests use a marker pointing at a path git never knew
/// about, so they cannot distinguish "absent from the list" from "listed
/// but prunable". This one registers a real worktree first.
///
/// **Guarded twice, deliberately.** `live_worktrees` also drops any path
/// it cannot enter, so a deleted directory is excluded even if the
/// `prunable` line were ignored — which is why mutating that line away
/// does not fail this test. The check stays because `prunable` is
/// reported for causes *other* than a missing directory (a gitdir file
/// pointing elsewhere, for one), and those the path filter would miss.
/// Deregisters probe worktrees on the way out **even if an assertion
/// panics**. Cleanup written after the asserts would be skipped by the
/// unwind, leaving the real repository carrying a stale record.
struct WorktreePruneGuard;

impl Drop for WorktreePruneGuard {
    fn drop(&mut self) {
        let _ = Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(repo_root())
            .output();
    }
}

#[test]
fn a_registered_worktree_whose_directory_was_deleted_is_prunable() {
    let root = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    let wt = home.path().join("gate-prunable-probe");

    let added = Command::new("git")
        .args(["worktree", "add", "-q", "--detach"])
        .arg(&wt)
        .arg("HEAD")
        .current_dir(repo_root())
        .output()
        .expect("git worktree add");
    // A HARD failure, not a silent return. Skipping here would make the
    // one test that covers `prunable` handling report green on a machine
    // where it never ran — the failure mode this whole suite exists to
    // avoid.
    assert!(
        added.status.success(),
        "could not register a probe worktree, so this test proved nothing:\n{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let _guard = WorktreePruneGuard;

    let (dir, _, ok) = run_in(&wt, root.path(), &["--init"]);
    let dir = PathBuf::from(dir.trim());
    assert!(ok && dir.is_dir(), "--init in the probe worktree");

    // Deleted WITHOUT `git worktree remove`: still registered, and now
    // reported with a `prunable` line.
    std::fs::remove_dir_all(&wt).expect("remove the worktree directory");

    let (out, _, ok) = run(root.path(), &["--prune", "--force"]);
    assert!(ok, "prune must succeed; output was:\n{out}");
    assert!(
        !dir.exists(),
        "a `prunable` record is not a live worktree — its build directory \
         must be reclaimable, or orphans accumulate forever. Output was:\n{out}"
    );
}

// --- Refusals: the two ways prune and the plan could do harm -----------

/// **The data-loss case.** Pruning decides what to delete by subtracting
/// the live worktree set from the managed root. Run from outside any
/// repository, that set cannot be established — and the first version of
/// this script masked the failure with `|| true`, making the set *empty*,
/// which marks **every** managed directory an orphan. `--prune --force`
/// would then have deleted all of them, including live lanes' artifacts.
///
/// The correct answer to "I cannot tell what is live" is to refuse.
///
/// **Two guards, deliberately redundant.** The script refuses both when
/// `git rev-parse --show-toplevel` fails and when `live_worktrees`
/// cannot enumerate — either alone satisfies this test, so mutating
/// away one at a time reads as "vacuous". Removing **both** fails it.
/// Recorded so a later reader does not delete one of them on the
/// grounds that no test noticed.
#[test]
fn prune_outside_a_repository_refuses_and_every_directory_survives() {
    let root = tempfile::tempdir().expect("tempdir");
    let (orphan, lookalike, live) = prune_fixture(root.path());
    let outside = tempfile::tempdir().expect("tempdir");

    // Sanity: the fixture's orphan really is eligible from inside a repo.
    let (inside, _, _) = run(root.path(), &["--prune"]);
    assert!(
        inside.contains("WOULD delete"),
        "fixture is not discriminating — nothing was eligible:\n{inside}"
    );

    let (out, err, ok) = run_in(outside.path(), root.path(), &["--prune", "--force"]);
    assert!(
        !ok,
        "pruning from outside a repository must FAIL, not proceed:\n{out}{err}"
    );
    assert!(
        err.contains("refusing to prune"),
        "the refusal must say why; stderr was:\n{err}"
    );
    assert!(
        orphan.exists() && lookalike.exists() && live.exists(),
        "nothing may be deleted when the live set is unknown"
    );
}

/// `--acceptance` is interpolated into a command the runner evaluates,
/// so a name carrying shell metacharacters is an injection. It must be
/// refused rather than escaped, and refused at parse time — before any
/// gate runs.
#[test]
fn acceptance_names_with_shell_metacharacters_are_refused() {
    let root = tempfile::tempdir().expect("tempdir");
    let canary = root.path().join("canary");
    std::fs::write(&canary, "intact").expect("write canary");

    let hostile = [
        format!("x; rm -f {}", canary.display()),
        format!("x$(rm -f {})", canary.display()),
        "x`id`".to_string(),
        "x && id".to_string(),
        "../escape".to_string(),
        "x y".to_string(),
        "-flag".to_string(),
    ];

    for name in &hostile {
        let (out, err, ok) = run(root.path(), &["--acceptance", name, "--print-plan"]);
        assert!(
            !ok,
            "must refuse acceptance name {name:?}; stdout was:\n{out}"
        );
        assert!(
            err.contains("refusing acceptance suite name") || err.contains("may not start with"),
            "refusal for {name:?} must say why; stderr was:\n{err}"
        );
        assert!(
            !out.contains("rm -f") && !out.contains("id"),
            "a hostile name must never reach the plan; stdout was:\n{out}"
        );
    }

    assert_eq!(
        std::fs::read_to_string(&canary).expect("read canary"),
        "intact",
        "no injected command may have executed"
    );
}

/// A well-formed name still works — otherwise the validator could pass
/// the test above by rejecting everything.
#[test]
fn ordinary_acceptance_names_are_accepted() {
    let root = tempfile::tempdir().expect("tempdir");
    for name in ["m4_acceptance", "gate-script", "abc123_x"] {
        let (plan, err, ok) = run(root.path(), &["--acceptance", name, "--print-plan"]);
        assert!(ok, "{name} must be accepted; stderr was:\n{err}");
        assert!(
            plan.contains(&format!("cargo test --test {name}")),
            "plan was:\n{plan}"
        );
    }
}

/// The marker is documented as one line. Reading only its first line
/// would accept a corrupted or hand-edited file and then delete a
/// directory on the strength of a file the script did not understand.
#[test]
fn a_multi_line_marker_is_refused_rather_than_read_head_first() {
    let root = tempfile::tempdir().expect("tempdir");
    let bad = root.path().join("bad-00000000");
    std::fs::create_dir_all(&bad).expect("mkdir");
    std::fs::write(
        bad.join(".pmacs-gate-target"),
        format!(
            "{}\nstray second line\n",
            root.path().join("gone").display()
        ),
    )
    .expect("write marker");

    let (out, _, ok) = run(root.path(), &["--prune", "--force"]);
    assert!(ok, "output was:\n{out}");
    assert!(
        bad.exists(),
        "a malformed marker must not authorise deletion"
    );
    assert!(
        out.contains("not exactly one line"),
        "the skip reason must name the problem; output was:\n{out}"
    );
}

/// Skips are reported with reasons. A prune that quietly ignores things
/// is how one learns too late that the marker was never written.
#[test]
fn skipped_directories_are_reported_with_a_reason() {
    let root = tempfile::tempdir().expect("tempdir");
    let (_, lookalike, _) = prune_fixture(root.path());

    let (out, _, _) = run(root.path(), &["--prune"]);
    assert!(
        out.contains(&lookalike.to_string_lossy().to_string())
            && out.contains("no readable .pmacs-gate-target"),
        "the unmarked directory must be named with its reason; output was:\n{out}"
    );
    assert!(
        out.contains("worktree is live"),
        "the live one must be named with its reason too; output was:\n{out}"
    );
}
