//! `scripts/gate` — the behaviour a shell script can be held to.
//!
//! Framing: `docs/gate-script-framing.md` §4, and §2a for the
//! `TMPDIR` isolation these rows cover (revision 6).
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

/// The SIGINT-deliverability helper the gate and the panel suite share
/// (`docs/gpu-probe-sigint-framing.md` §7c).
fn sigint_helper() -> PathBuf {
    repo_root().join("scripts/check-sigint-deliverable")
}

/// Run `cmd` with `SIGINT` set to `SIG_IGN`, the way a shell that
/// backgrounds a job without job control does.
///
/// `trap "" INT` sets the ignore in the wrapper shell, and `SIG_IGN` is
/// inherited across `fork` **and survives `exec`** — which is the whole
/// mechanism under test, so simulating it this way exercises the real
/// thing rather than a stand-in.
fn under_ignored_sigint(
    program: &Path,
    args: &[&str],
    cwd: &Path,
    env: &[(&str, &str)],
) -> std::process::Output {
    // `exec "$@"` with the program and arguments passed POSITIONALLY.
    // Interpolating them into the script text would break on any path
    // containing a space or a shell metacharacter, and every path here
    // comes from a `tempdir` or `CARGO_MANIFEST_DIR` — neither of which
    // this test controls.
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("trap \"\" INT; exec \"$@\"")
        .arg("sh")
        .arg(program)
        .args(args)
        .current_dir(cwd);
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("spawn shell with SIGINT ignored")
}

/// A minimal git worktree holding a copy of `scripts/gate` and a
/// **stub** `check-sigint-deliverable`, so the gate's handling of each
/// helper status can be driven on its real path without touching the
/// checked-in helper.
fn gate_with_stub_helper(stub_body: &str, executable: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let scripts = dir.path().join("scripts");
    std::fs::create_dir_all(&scripts).expect("scripts dir");
    std::fs::copy(gate(), scripts.join("gate")).expect("copy gate");
    let helper = scripts.join("check-sigint-deliverable");
    std::fs::write(&helper, stub_body).expect("write stub helper");
    let mode = if executable { 0o755 } else { 0o644 };
    std::fs::set_permissions(&helper, std::os::unix::fs::PermissionsExt::from_mode(mode))
        .expect("chmod stub helper");
    std::fs::set_permissions(
        scripts.join("gate"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("chmod gate copy");
    let ok = Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .expect("git init");
    assert!(ok.success(), "the stub worktree must be a git worktree");
    dir
}

fn gate() -> PathBuf {
    repo_root().join("scripts/gate")
}

/// Run `scripts/gate` with an isolated managed root, from `cwd`.
/// A SHORT base for the roots these tests hand the gate, independent of
/// the ambient `TMPDIR`.
///
/// **Not `tempfile::tempdir()`'s default, and the reason is the socket
/// budget rather than taste.** When this suite runs inside a gate, the
/// ambient `TMPDIR` is already that gate's own (~47 bytes); rooting a
/// nested gate under it pushes the nested `TMPDIR` to ~70 bytes and
/// legitimately trips its own `SUN_LEN` guard. The suite would then
/// fail on a configuration it created rather than on the behaviour
/// under test — which is exactly how it failed once.
///
/// `/tmp` is named explicitly because it is short and this suite
/// already requires a Unix environment. These roots hold synthetic
/// plans and never fixtures, so `/tmp`'s contents are irrelevant to
/// them; the rows set `PMACS_GATE_ALLOW_ANCESTOR_MARKER` for that
/// reason.
fn short_root_base() -> PathBuf {
    PathBuf::from("/tmp")
}

fn run_in(cwd: &Path, root: &Path, args: &[&str]) -> (String, String, bool) {
    let out = Command::new(gate())
        .args(args)
        .current_dir(cwd)
        .env("PMACS_GATE_TARGET_ROOT", root)
        // Test-only, like PMACS_GATE_TARGET_ROOT itself. These roots are
        // `tempfile::tempdir()`s whose ancestors the suite does not
        // control — on a machine whose `/tmp` carries a marker, every
        // row here would otherwise be refused. The plans are synthetic,
        // so no markerless fixture exists for a marker to re-root. The
        // check is witnessed separately, by a row that does NOT set
        // this.
        .env("PMACS_GATE_ALLOW_ANCESTOR_MARKER", "1")
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

/// A6, gate consumer: a helper verdict of `error` (2) refuses the run
/// with the ERROR wording, and never claims `SIGINT` is ignored.
///
/// Driven through a stub worktree so the gate's real code path runs
/// against a controlled helper status; the helper's own classification
/// is covered by its own rows above.
#[test]
fn gate_refuses_on_helper_error_without_claiming_sigint_is_ignored() {
    let root = tempfile::tempdir().expect("tempdir");
    let repo = gate_with_stub_helper(
        "#!/bin/sh\necho 'pmacs: could not determine whether SIGINT is deliverable (probe status 42)' >&2\nexit 2\n",
        true,
    );
    let out = Command::new(repo.path().join("scripts/gate"))
        .arg("--self-test")
        .current_dir(repo.path())
        .env("PMACS_GATE_TARGET_ROOT", root.path())
        .output()
        .expect("run the stub-worktree gate");
    assert_eq!(
        out.status.code(),
        Some(2),
        "an error verdict exits 2, not 1"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    // This stub emits the error TEXT but no token, so under the pair
    // ABI it is a BOUNDARY error --- and its stderr is untrusted, hence
    // deliberately not surfaced.
    assert!(
        err.contains("SIGINT guard boundary error"),
        "an unvalidated pair is a boundary error: {err}"
    );
    assert!(
        !err.contains("could not determine"),
        "and the untrusted child stderr is NOT shown: {err}"
    );
    assert!(
        !err.contains("SIGINT is ignored"),
        "an undecidable probe must never be reported as ignored: {err}"
    );
    assert!(err.contains("no stage has run"), "and no stage ran: {err}");
}

/// A6, gate boundary: a helper that cannot be EXECUTED is an `error` at
/// the call boundary — mapped to 2 — never evidence that `SIGINT` is
/// ignored.
///
/// This is the case the original guard got wrong twice: under `set -e`
/// a bare invocation died before any mapping, and 126/127 would have
/// escaped raw.
#[test]
fn gate_maps_an_unexecutable_helper_to_error_not_ignored() {
    let root = tempfile::tempdir().expect("tempdir");
    let repo = gate_with_stub_helper("#!/bin/sh\nexit 0\n", false);
    let out = Command::new(repo.path().join("scripts/gate"))
        .arg("--self-test")
        .current_dir(repo.path())
        .env("PMACS_GATE_TARGET_ROOT", root.path())
        .output()
        .expect("run the stub-worktree gate");
    let err = String::from_utf8_lossy(&out.stderr);
    // The gate prints the raw probe status it saw. Carry it into every
    // assertion message: this row failed on macOS with exit 1 where 2
    // was expected, and the log could not say which status produced it
    // because the message discarded stderr.
    assert_eq!(
        out.status.code(),
        Some(2),
        "boundary failures map to 2; gate said:\n{err}"
    );
    assert!(
        err.contains("SIGINT guard boundary error"),
        "the boundary has its own wording: {err}"
    );
    assert!(
        err.contains("token=missing"),
        "and names the token state, not just the status: {err}"
    );
    assert!(
        !err.contains("SIGINT is ignored"),
        "an unrunnable guard is not evidence about the signal: {err}"
    );
}

/// One stub-worktree gate run against a controlled `(status, stdout)`
/// pair, returning the gate's own exit code and stderr.
///
/// The gate is the **shell consumer** of the pair ABI. Driving it
/// through a stub worktree exercises its real code path — including
/// the `set -eu` handling and the capture directory — against inputs
/// no real helper would produce.
fn gate_sees_pair(status: i32, stdout: &[u8], stderr_line: &str) -> (Option<i32>, String, PathBuf) {
    let root = tempfile::tempdir().expect("tempdir");
    let octal = stdout.iter().fold(String::new(), |mut acc, b| {
        use std::fmt::Write as _;
        let _ = write!(acc, "\\{b:03o}");
        acc
    });
    let repo = gate_with_stub_helper(
        &format!("#!/bin/sh\nprintf '{octal}'\n{stderr_line}exit {status}\n"),
        true,
    );
    let out = Command::new(repo.path().join("scripts/gate"))
        .arg("--self-test")
        .current_dir(repo.path())
        .env("PMACS_GATE_TARGET_ROOT", root.path())
        .env("TMPDIR", root.path())
        .output()
        .expect("run the stub-worktree gate");
    let code = out.status.code();
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    // Keep `root` alive for the residue check by returning its path
    // after leaking the handle: the caller inspects it, then it is
    // dropped with the TempDir at end of test.
    let path = root.keep();
    (code, err, path)
}

/// A6/A6b/A6c, shell consumer: the same 45 shared conformance cases the
/// Rust validator runs, so the two cannot diverge.
///
/// X2 is absent by construction — a shell `exec` failure becomes a
/// shell status, so the shell boundary cannot present a status-less
/// spawn error.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the bulk is the generated case list, which is data; splitting \
              it would put the cases and the expectations they encode in \
              different places"
)]
fn gate_validates_the_whole_shared_conformance_set() {
    const SAFE: &[u8] = b"pmacs-sigint-v1:safe";
    const IGNORED: &[u8] = b"pmacs-sigint-v1:ignored";
    const ERROR: &[u8] = b"pmacs-sigint-v1:error";
    let toks: [(&str, &[u8]); 3] = [("safe", SAFE), ("ignored", IGNORED), ("error", ERROR)];

    let mut cases: Vec<(String, i32, Vec<u8>, &str, bool)> = Vec::new();
    for (idx, (name, correct)) in toks.iter().enumerate() {
        let status = i32::try_from(idx).expect("0..=2");
        let mut lf = correct.to_vec();
        lf.push(b'\n');
        // Only status 0's correct token is `safe`; the correct token at
        // 1 and 2 is a VALIDATED verdict, which refuses with its own
        // status rather than continuing.
        let safe_here = status == 0;
        cases.push((
            format!("{status}/V/{name}/bare"),
            status,
            correct.to_vec(),
            "",
            safe_here,
        ));
        cases.push((format!("{status}/V/{name}/lf"), status, lf, "", safe_here));
        for (other, bytes) in &toks {
            if other == name {
                continue;
            }
            let mut olf = bytes.to_vec();
            olf.push(b'\n');
            cases.push((
                format!("{status}/M/{other}/bare"),
                status,
                bytes.to_vec(),
                "",
                false,
            ));
            cases.push((format!("{status}/M/{other}/lf"), status, olf, "", false));
        }
        let mut leading = vec![b'\n'];
        leading.extend_from_slice(correct);
        let mut extra = correct.to_vec();
        extra.extend_from_slice(b"\n\n");
        let mut spaces = b" ".to_vec();
        spaces.extend_from_slice(correct);
        spaces.push(b' ');
        let mut crlf = correct.to_vec();
        crlf.extend_from_slice(b"\r\n");
        let mut doubled = correct.to_vec();
        doubled.extend_from_slice(correct);
        let mut nul = correct.to_vec();
        nul.push(0);
        for (cls, bytes) in [
            ("E/empty", Vec::new()),
            ("U/unknown", b"pmacs-sigint-v2:safe".to_vec()),
            ("L/leading-lf", leading),
            ("X/extra-lf", extra),
            ("S/spaces", spaces),
            ("C/crlf", crlf),
            ("D/doubled", doubled),
            ("N/nul", nul),
        ] {
            cases.push((format!("{status}/{cls}"), status, bytes, "", false));
        }
    }
    cases.push(("X1/status-126".to_owned(), 126, SAFE.to_vec(), "", false));
    cases.push((
        "X3/ignored-text-no-token".to_owned(),
        1,
        Vec::new(),
        "echo 'pmacs: SIGINT is ignored; run this command with SIGINT deliverable' >&2\n",
        false,
    ));
    cases.push((
        "X4/stderr-noise".to_owned(),
        0,
        SAFE.to_vec(),
        "echo 'chatter on stderr' >&2\n",
        true,
    ));
    assert_eq!(cases.len(), 45, "the shared set is 45 cases");

    for (name, status, stdout, stderr_line, expect_pass) in cases {
        let (code, err, dir) = gate_sees_pair(status, &stdout, stderr_line);
        if expect_pass {
            // `safe` continues into the self-test plan, which exits
            // non-zero ON PURPOSE — what matters is that the guard did
            // not refuse.
            assert!(
                !err.contains("REFUSING TO RUN"),
                "case {name}: the guard must not refuse a validated safe pair: {err}"
            );
        } else {
            assert!(
                err.contains("REFUSING TO RUN"),
                "case {name}: the guard must refuse: {err}"
            );
            let validated = name.contains("/V/") && (status == 1 || status == 2);
            if validated {
                assert_eq!(
                    code,
                    Some(status),
                    "case {name}: validated verdicts pass their status through"
                );
            } else {
                assert_eq!(code, Some(2), "case {name}: boundary errors map to 2");
                assert!(
                    !err.contains("SIGINT is ignored"),
                    "case {name}: a boundary failure must not speak with the \
                     helper's voice: {err}"
                );
            }
        }
        // A8: no capture directory survives, on any path.
        let residue: Vec<_> = std::fs::read_dir(&dir)
            .expect("read tmpdir")
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with("pmacs-sigint."))
            .collect();
        assert!(residue.is_empty(), "case {name}: capture residue survived");
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// §7c: the helper answers `safe` when `SIGINT` is deliverable.
#[test]
fn sigint_helper_reports_safe_when_the_signal_is_deliverable() {
    let out = Command::new(sigint_helper())
        .output()
        .expect("run the sigint helper");
    assert_eq!(out.status.code(), Some(0), "safe is exit 0");
    assert!(
        out.stderr.is_empty(),
        "the safe path is silent, so a clean run says nothing: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// §7c: the helper answers `ignored` — exit 1, canonical wording — when
/// `SIGINT` is inherited as `SIG_IGN`.
#[test]
fn sigint_helper_reports_ignored_when_the_signal_is_inherited_ignored() {
    let out = under_ignored_sigint(&sigint_helper(), &[], &repo_root(), &[]);
    assert_eq!(out.status.code(), Some(1), "ignored is exit 1");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("SIGINT is ignored"),
        "the canonical ignored diagnosis is the helper's to own: {err}"
    );
}

/// §7c: the helper answers `error` — exit 2, a DISTINCT diagnosis — when
/// the probe cannot decide.
///
/// The probe shells out, so an empty `PATH` makes its inner `sh`
/// unfindable. This is the case a naive `exit 0` would misreport as
/// `ignored`, failing the caller for the wrong reason.
#[test]
fn sigint_helper_reports_error_and_never_ignored_when_the_probe_cannot_run() {
    let out = Command::new(sigint_helper())
        .env("PATH", "")
        .output()
        .expect("run the sigint helper with no PATH");
    assert_eq!(out.status.code(), Some(2), "error is exit 2, never 1");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("could not determine"),
        "error has its own wording: {err}"
    );
    assert!(
        !err.contains("SIGINT is ignored"),
        "error must NOT be reported as ignored --- they are different \
         problems, and conflating them is the defect the helper exists \
         to avoid: {err}"
    );
}

/// R-b: the gate refuses under ignored `SIGINT`, **before any stage**.
///
/// This is the row whose absence let a real bug ship: the first
/// implementation ran the helper as a bare command under `set -e`, so
/// the shell died at the non-zero exit and the refusal never printed;
/// the second captured `$?` inside `if !`, which is the status of the
/// negated condition — always zero — so the gate printed the diagnosis
/// and then ran the whole suite anyway. Both passed every other test in
/// this file.
#[test]
fn gate_refuses_to_start_when_sigint_is_ignored() {
    let root = tempfile::tempdir().expect("tempdir");
    // `--self-test`, NOT the ordinary gate. If the guard ever regresses,
    // this row must not launch eight real gate stages inside the gate
    // suite — the recursion constraint this file opens with. Self-test
    // drives the same runner over a hardcoded synthetic plan, so the
    // negative path stays bounded whatever the guard does.
    let out = under_ignored_sigint(
        &gate(),
        &["--self-test"],
        &repo_root(),
        &[("PMACS_GATE_TARGET_ROOT", &root.path().display().to_string())],
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "the helper's verdict passes through"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("SIGINT is ignored"),
        "the gate surfaces the helper's stderr unchanged rather than \
         inventing its own wording: {err}"
    );
    assert!(
        err.contains("no stage has run"),
        "and says the run is not a test failure: {err}"
    );
    let combined = format!("{}{err}", String::from_utf8_lossy(&out.stdout));
    assert!(
        !combined.contains("[01]"),
        "NO stage may run --- the guard sits before stage 1: {combined}"
    );
}

#[test]
fn the_plan_sweeps_the_workspace_and_never_only_the_tests() {
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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
///
/// **What this test cannot see: the names.** `--print-plan` strips them
/// (`emit_plan | cut -f2-`), so everything below is an assertion about
/// *commands in an order* — renaming the real build step to `sweep-crdt`
/// leaves it green. The step's **name** is asserted by
/// `the_crdt_build_step_carries_its_own_name_and_its_exact_command`
/// below, which reads the plan in the form the runner reads it.
#[test]
fn the_crdt_sweep_is_immediately_preceded_by_the_build_that_produces_its_binary() {
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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

/// **The witness that reaches the step it names**, and the reason this
/// lane needed a second round.
///
/// This lane exists to guarantee two things: that the crdt sweep is
/// preceded by the build producing its binary, and that a build failure
/// is attributed to **`build-crdt`** rather than to `sweep-crdt`. The
/// first round shipped with neither guaranteed, because **neither
/// witness could see a name**:
///
/// - `--print-plan` renders `emit_plan | cut -f2-`, so the ordering test
///   above compares commands and never sees the names beside them.
/// - `--self-test` hardcodes the string `build-crdt` inside its **own
///   synthetic** plan, so it proves things about the *runner* and
///   nothing about the real emitter.
///
/// Review demonstrated the consequence directly: **renaming the real
/// build step to `sweep-crdt` left both tests passing** — a plan that
/// reports a build failure under the sweep's name, which is exactly the
/// misattribution the separate step exists to prevent, sitting green.
///
/// So the pair is asserted **together, as one emitted line**, against
/// `--print-plan-named` — the plan in the form the runner reads it back
/// from `PLAN_FILE`. Name and command in the same `assert`, from the
/// real emitter, is what makes a rename unable to pass; either half
/// alone lets the other drift.
///
/// The mode is a *rendering*, not a seam: `PLAN_FILE` stays
/// uninjectable, because a test that supplied the runner's plan would
/// turn its `eval` into a general command executor — the defect the
/// `--acceptance` refusal below exists to prevent.
#[test]
fn the_crdt_build_step_carries_its_own_name_and_its_exact_command() {
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
    let build = "build-crdt\tcargo build --workspace --no-default-features --features luajit,crdt";
    let sweep =
        "sweep-crdt\tcargo test --workspace --features crdt --no-fail-fast -- --skip basedpyright";

    let (plan, err, ok) = run(root.path(), &["--protocol", "--print-plan-named"]);
    assert!(
        ok,
        "--protocol --print-plan-named must succeed; stderr:\n{err}"
    );
    let lines: Vec<&str> = plan.lines().collect();

    // Whole-line equality, not `contains`: the name, the tab, and the
    // command with nothing appended. A step is its (name, command) pair
    // and the plan is where both are decided.
    let b = lines.iter().position(|l| *l == build).unwrap_or_else(|| {
        panic!(
            "no plan line is exactly:\n  {build}\nA build step under a \
             different NAME misattributes its own failure; a build step \
             with different FEATURES hands the sweep a binary it cannot \
             use. Plan was:\n{plan}"
        )
    });
    // The sweep's own pair, for the same reason in the other direction:
    // asserting only the build's name lets a rename of the SWEEP slip
    // through the identical hole.
    let s = lines
        .iter()
        .position(|l| *l == sweep)
        .unwrap_or_else(|| panic!("no plan line is exactly:\n  {sweep}\nPlan was:\n{plan}"));

    assert_eq!(
        s,
        b + 1,
        "the build must be the step IMMEDIATELY before the crdt sweep — a \
         build merely somewhere earlier could be separated from it by a \
         step that rewrites the same target directory. Plan was:\n{plan}"
    );

    // Conditionality, on this rendering too: an ordinary lane must not
    // carry the step at all, not merely not carry its command.
    let (default_plan, _, ok) = run(root.path(), &["--print-plan-named"]);
    assert!(ok, "--print-plan-named must succeed");
    assert!(
        !default_plan.contains("build-crdt"),
        "the default sweep never builds pmacs-gpu and never needs it, so no \
         ordinary lane may pay for a workspace build; plan was:\n{default_plan}"
    );
}

/// **The new rendering must be the same plan, or the assertion above
/// pins a string only the test ever reads.**
///
/// `--print-plan-named` and `--print-plan` both call one emitter, and
/// the runner writes that same emitter to `PLAN_FILE` — so today they
/// cannot disagree. This pins that from outside, where a later edit
/// giving either mode its own plan text would be caught rather than
/// producing a witness that asserts a name the runner never uses.
///
/// It also pins the **shape** the runner depends on: the loop reads each
/// line with `IFS=<tab> read -r name cmd`, so a plan line without its
/// tab would silently run under an empty command.
#[test]
fn the_named_plan_is_the_printed_plan_with_its_names_removed() {
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");

    for flags in [
        vec![],
        vec!["--protocol"],
        vec!["--acceptance", "m4_acceptance"],
    ] {
        let mut named_args = flags.clone();
        named_args.push("--print-plan-named");
        let mut plain_args = flags.clone();
        plain_args.push("--print-plan");

        let (named, err, ok_named) = run(root.path(), &named_args);
        assert!(ok_named, "{named_args:?} must succeed; stderr:\n{err}");
        let (plain, err, ok_plain) = run(root.path(), &plain_args);
        assert!(ok_plain, "{plain_args:?} must succeed; stderr:\n{err}");

        let mut stripped = String::new();
        for l in named.lines() {
            let (_name, cmd) = l.split_once('\t').unwrap_or_else(|| {
                panic!(
                    "every plan line must be `name<TAB>command` — the runner \
                     splits on that tab, so a line without one runs an empty \
                     command under the whole line's name. Line was:\n  {l:?}"
                )
            });
            stripped.push_str(cmd);
            stripped.push('\n');
        }

        assert_eq!(
            stripped, plain,
            "the two renderings must be one plan; with {flags:?} they diverged"
        );
    }
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
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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

/// The isolated `TMPDIR` reaches a spawned CHILD, and is disk-backed
/// under the managed target root.
///
/// **The hazard is an ancestor marker, not a dirty temp directory.**
/// Project detection walks upward, so a stray `/tmp/.git` re-roots every
/// markerless `tempfile::tempdir()` fixture beneath it — which is how
/// two LSP file-watcher tests reddened a gate run on a lane whose whole
/// executable diff lived in a crate the failing binary does not link. A
/// fresh subdirectory *of* `/tmp` would inherit the same ancestors and
/// the same marker, so the directory has to live where the gate already
/// owns the path.
///
/// **Observed in a child process, deliberately.** The gate exporting a
/// variable proves only that the gate can export a variable; what the
/// suites need is that a process the runner spawns inherits it. The
/// self-test's first step reports its own `$TMPDIR`, so the assertion
/// reads a real child's environment out of a real gate log.
#[test]
fn the_isolated_tmpdir_reaches_a_spawned_child_under_the_managed_root() {
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
    let (out, err, _ok) = run(root.path(), &["--self-test"]);

    let announced = out
        .lines()
        .find_map(|l| {
            l.split_once("gate: tmpdir")
                .map(|(_, p)| p.trim().to_owned())
        })
        .unwrap_or_else(|| panic!("the gate must announce its TMPDIR; stdout:\n{out}"));

    // The contract is **under the managed gate root**, which is what
    // makes it disk-backed in production: the gate root sits beside the
    // build artifacts, not in `/tmp`.
    //
    // **Placement does NOT make it marker-free** — that is the false
    // inference the ancestor check exists to correct, and an earlier
    // version of this comment made it. A `.git` in `$HOME` or above
    // `$HOME/build` re-roots fixtures just as `/tmp/.git` did.
    // Marker-freeness is a separate, verified precondition; see
    // `the_ancestor_check_honours_marker_types`.
    //
    // Deliberately NOT asserting `!starts_with("/tmp/")` here. This
    // test's own root is a `tempfile::tempdir()`, so on a normal
    // machine it IS under `/tmp` and the gate's directory inherits
    // that — such an assertion would be testing where the fixture put
    // its root, not what the gate does, and would fail on correct code.
    let expected_prefix = root.path().join("").display().to_string();
    assert!(
        announced.starts_with(&expected_prefix),
        "the gate TMPDIR must live under the managed target root, which \
         the gate owns and prunes; expected a child of {expected_prefix}, \
         was {announced}"
    );
    // Its own named area under the root, so `prune` and a human can
    // both tell it from the ambient root and the logs. Asserted as the
    // exact parent rather than a substring: the directory is called
    // `tmp`, and a `contains("/tmp/")` check would also pass for a path
    // that merely happened to sit under a `/tmp` somewhere.
    assert_eq!(
        Path::new(&announced).parent().expect("tmpdir parent"),
        root.path().join("tmp"),
        "the per-run TMPDIR must sit directly under <root>/tmp; was {announced}"
    );

    // The child's own view, read out of the log the runner wrote.
    let log = Path::new(
        err.lines()
            .find_map(|l| l.split_once("log: ").map(|(_, p)| p.trim()))
            .unwrap_or_else(|| panic!("expected a log path; stderr:\n{err}")),
    )
    .parent()
    .expect("log directory")
    .join("01-self-pass.log");
    let seen =
        std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("read {}: {e}", log.display()));
    assert_eq!(
        seen.trim(),
        format!("gate-child-tmpdir={announced}"),
        "a spawned child must inherit exactly the announced TMPDIR"
    );
}

/// The `TMPDIR` is reaped when the run ends, like the ambient root.
///
/// Without this the gate would leak a directory per invocation into the
/// target root — the same accumulation `prune` exists to clean up, but
/// created by the tool itself and on every single run.
#[test]
fn the_isolated_tmpdir_is_reaped_when_the_run_ends() {
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
    let (out, _err, _ok) = run(root.path(), &["--self-test"]);

    let announced = out
        .lines()
        .find_map(|l| {
            l.split_once("gate: tmpdir")
                .map(|(_, p)| p.trim().to_owned())
        })
        .unwrap_or_else(|| panic!("the gate must announce its TMPDIR; stdout:\n{out}"));

    // The run above FAILED on purpose (the self-test's middle step), so
    // this also pins that the trap fires on the failure path — the path
    // a leak would actually happen on.
    assert!(
        !Path::new(&announced).exists(),
        "the exit trap must remove the TMPDIR even when a gate failed; \
         {announced} survived"
    );
    // The parent stays. It is SHARED between worktrees — it hangs off
    // the gate root, not the derived per-worktree target, for the
    // socket budget — and `--prune` never touches it, because prune
    // only considers directories carrying an ownership marker.
    assert!(
        Path::new(&announced)
            .parent()
            .expect("gate-tmp parent")
            .exists(),
        "only the per-run directory is reaped, not its parent"
    );
}

/// A project marker above the gate TMPDIR is REFUSED.
///
/// **Placement under a managed root is necessary, not sufficient.** A
/// `.git` in `$HOME`, any recognized marker above `$HOME/build`, or a
/// contaminated `PMACS_GATE_TARGET_ROOT` re-roots every markerless
/// fixture beneath it — which is the original defect, rebuilt one
/// directory up. Asserting only "the path sits under the configured
/// root" would prove placement and nothing about the hazard.
///
/// This row deliberately does **not** set
/// `PMACS_GATE_ALLOW_ANCESTOR_MARKER`, which is what every other row
/// here sets; it is the one place the check itself runs.
#[test]
fn a_project_marker_above_the_gate_tmpdir_is_refused() {
    // Built OUTSIDE the system temp dir on purpose: the point is to
    // control what sits above the root, and `/tmp` may already carry a
    // marker — which would make the row pass for the wrong reason.
    let base = tempfile::Builder::new()
        .prefix("gm-")
        .tempdir_in(short_root_base())
        .expect("base");
    let root = base.path().join("inner");
    std::fs::create_dir_all(&root).expect("root");
    std::fs::write(base.path().join("Cargo.toml"), "[package]\n").expect("marker");

    let out = std::process::Command::new(gate())
        .arg("--self-test")
        .current_dir(repo_root())
        .env("PMACS_GATE_TARGET_ROOT", &root)
        .env_remove("PMACS_GATE_ALLOW_ANCESTOR_MARKER")
        .env_remove("TMPDIR")
        .output()
        .expect("run gate");
    let err = String::from_utf8_lossy(&out.stderr);

    assert!(
        !out.status.success(),
        "a marker above the TMPDIR must refuse the run; stderr:\n{err}"
    );
    assert!(
        err.contains("a project marker sits above the gate TMPDIR"),
        "and must say so, naming the marker; stderr:\n{err}"
    );
    assert!(
        err.contains("Cargo.toml"),
        "the message must name the marker it found; stderr:\n{err}"
    );
}

/// A `.git` **FILE** above the root is accepted; a `.git` **DIRECTORY**
/// is refused.
///
/// **The types are the contract, not the names.** `match_marker`
/// (`src/project.rs`) requires `.git` to be a directory and the seven
/// language markers to be files, so an existence-only check would
/// reject ancestors project detection walks straight past. The case is
/// not exotic: **a git worktree has a `.git` FILE**, so every worktree
/// in this repository would have tripped an `[ -e ]` guard.
#[test]
fn the_ancestor_check_honours_marker_types() {
    // **Asserted on WHICH marker is named, not on whether a refusal
    // happened.** The ancestors of any base this test can create are
    // outside its control — `/tmp` may hold a real `.git` directory,
    // and the repo root holds a `Cargo.toml` — so "no refusal" is not
    // a claim it can make anywhere. "The refusal did not name MY file"
    // is, and it is the claim that actually distinguishes the two
    // types.
    let base = tempfile::Builder::new()
        .prefix("gt-")
        .tempdir_in(short_root_base())
        .expect("base");
    let root = base.path().join("inner");
    std::fs::create_dir_all(&root).expect("root");

    let run_it = || {
        std::process::Command::new(gate())
            .arg("--self-test")
            .current_dir(repo_root())
            .env("PMACS_GATE_TARGET_ROOT", &root)
            .env_remove("PMACS_GATE_ALLOW_ANCESTOR_MARKER")
            .env_remove("TMPDIR")
            .output()
            .expect("run gate")
    };

    // A `.git` FILE — a worktree — is not a project root to detection,
    // so it must not be one here either.
    let mine = base.path().join(".git");
    std::fs::write(&mine, "gitdir: /elsewhere\n").expect("git file");
    let err = String::from_utf8_lossy(&run_it().stderr).into_owned();
    assert!(
        !err.contains(&format!("{}", mine.display())),
        "a `.git` FILE must not be treated as a marker, but the gate \
         named it; stderr:\n{err}"
    );

    // The same name as a DIRECTORY is a real marker.
    // A LANGUAGE marker as a DIRECTORY: detection wants a file, so the
    // guard must ignore it. Without this row, reverting only the
    // language-marker arm to `[ -e ]` stays green — the `.git` halves
    // below constrain the directory-valued marker alone.
    std::fs::remove_file(&mine).expect("rm git file");
    let cargo_dir = base.path().join("Cargo.toml");
    std::fs::create_dir(&cargo_dir).expect("cargo dir");
    let err = String::from_utf8_lossy(&run_it().stderr).into_owned();
    assert!(
        !err.contains(&format!("{}", cargo_dir.display())),
        "a `Cargo.toml` DIRECTORY is not a marker to detection and must \
         not be one here; stderr:\n{err}"
    );
    std::fs::remove_dir(&cargo_dir).expect("rm cargo dir");

    std::fs::create_dir(&mine).expect("git dir");
    let out = run_it();
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        !out.status.success(),
        "a `.git` DIRECTORY must refuse the run"
    );
    assert!(
        err.contains(&format!("{}", mine.display())),
        "and must name it, not some other ancestor; stderr:\n{err}"
    );
}

/// The socket-path guard, **at the boundary**: 55 bytes accepted, 56
/// rejected, with the measured lengths asserted.
///
/// **Straddling the cutoff is not testing it.** An earlier version of
/// this row generated roughly 51- and 71-byte paths against a 55-byte
/// cutoff; raising `SUN_LEN_BUDGET` from 103 to 118 would have left
/// both green, so the row constrained the guard's existence and not its
/// value. These aim at 55 and 56 exactly and assert the byte lengths
/// they achieved, so a drifting budget fails here rather than in a
/// socket bind.
///
/// The budget is **103 usable bytes** — Darwin's `sun_path[104]` minus
/// its terminating NUL, the supported-platform floor — less the 48-byte
/// fixture reserve.
#[test]
fn the_socket_path_guard_holds_at_its_exact_boundary() {
    // The gate derives `<root>/tmp/XXXXXX`, i.e. root + 11 bytes.
    const DERIVED: usize = 11;
    const CUTOFF: usize = 103 - 48;

    let ok_root = root_of_len(CUTOFF - DERIVED);
    let out = run_guarded(ok_root.path());
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let announced = stdout
        .lines()
        .find_map(|l| {
            l.split_once("gate: tmpdir")
                .map(|(_, p)| p.trim().to_owned())
        })
        .unwrap_or_else(|| {
            panic!(
                "a root at the cutoff must be ACCEPTED and announce its \
                 TMPDIR; stdout:\n{stdout}stderr:\n{}",
                String::from_utf8_lossy(&out.stderr)
            )
        });
    assert_eq!(
        announced.len(),
        CUTOFF,
        "the accepted fixture must sit exactly ON the cutoff, not below \
         it — otherwise a widened budget still passes. Path: {announced}"
    );

    let over_root = root_of_len(CUTOFF - DERIVED + 1);
    let out = run_guarded(over_root.path());
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        !out.status.success() && err.contains("too long for a unix socket path"),
        "one byte past the cutoff must be REJECTED; stderr:\n{err}"
    );
    // Keyed on the line that reports the measurement, not on the first
    // `gate:   ` line — that one is the path.
    let measured = err
        .lines()
        .find(|l| l.contains("bytes, but a fixture needs"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<usize>().ok());
    assert_eq!(
        measured,
        Some(CUTOFF + 1),
        "and the refusal must report exactly one byte over; stderr:\n{err}"
    );

    // REJECTION MUST NOT LEAK, and **both** areas are checked. The
    // guard creates the ambient root and the TMPDIR before it can
    // measure anything, so a rejection that exits before the trap is
    // armed leaves both behind. Checking only `<root>/tmp` would pass
    // while `gate-ambient` leaked.
    let leaked = leaked_temp_areas(over_root.path());
    assert!(
        leaked.is_empty(),
        "a rejected run must reap BOTH created areas; found {leaked:?}"
    );
}

/// The guard counts BYTES, not characters — **under a UTF-8 locale**.
///
/// `${#var}` counts characters in a UTF-8 locale while `sun_path` is
/// byte-limited, so a multibyte path measures short and passes a check
/// it should fail. **The locale is set explicitly**: under an inherited
/// `LC_ALL=C`, `${#var}` already counts bytes and the character-counting
/// mutant would pass, making this row's verdict depend on the
/// environment rather than on the code.
#[test]
fn the_socket_path_guard_counts_bytes_not_characters() {
    // Each `é` is one character and two bytes, so this root is under
    // the cutoff by character count and over it by byte count — the
    // only shape that separates the two implementations.
    let root = tempfile::Builder::new()
        .prefix(&"é".repeat(22))
        .tempdir_in(short_root_base())
        .expect("multibyte root");
    let path = root.path().to_string_lossy().into_owned();
    let chars = path.chars().count();
    let bytes = path.len();
    assert!(
        chars + 11 <= 55 && bytes + 11 > 55,
        "fixture must straddle: {chars} chars (must pass) vs {bytes} \
         bytes (must fail), path {path}"
    );

    // **Chosen by BEHAVIOUR, not by name**, and the interpreter is part
    // of the choice: `${#x}` counting characters is a property of the
    // SHELL first and the locale second. `bash` counts characters under
    // a UTF-8 locale; `dash` counts bytes under every locale. Naming a
    // locale and running `/bin/sh` therefore proves nothing on its own —
    // where `/bin/sh` is `dash`, the character-counting mutant measures
    // bytes too, agrees with the fix, and this row passes vacuously.
    //
    // The gate is invoked THROUGH that shell rather than by its
    // shebang, because the configuration being pinned is a real one:
    // `#!/bin/sh` resolves to `bash` on Arch and on macOS, which is
    // exactly where a `${#VAR}` guard would miscount.
    let (shell, locale) = char_counting_shell();
    let out = std::process::Command::new(&shell)
        .arg(gate())
        .arg("--self-test")
        .current_dir(repo_root())
        .env("PMACS_GATE_TARGET_ROOT", root.path())
        .env("PMACS_GATE_ALLOW_ANCESTOR_MARKER", "1")
        .env("LC_ALL", &locale)
        .env("LANG", &locale)
        .env_remove("TMPDIR")
        .output()
        .expect("run gate");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        !out.status.success() && err.contains("too long for a unix socket path"),
        "a root over the BYTE budget must be refused even though it is \
         under the CHARACTER budget ({chars} chars, {bytes} bytes); \
         stderr:\n{err}"
    );
}

/// A `(shell, locale)` pair under which **`${#x}` counts CHARACTERS** —
/// established by asking that very shell, never by trusting a name.
///
/// **Both axes matter, and the shell matters more.** `${#x}` on a
/// two-byte character answers `1` under `bash` with a UTF-8 locale and
/// `2` under `bash` with `C` — but `dash` answers `2` under *every*
/// locale, because it has no multibyte handling at all. So naming a
/// locale and invoking `/bin/sh` establishes nothing: where `/bin/sh`
/// is `dash` (Debian and Ubuntu, including CI) the character-counting
/// mutant measures bytes too, agrees with the fix, and the row this
/// serves passes while proving nothing.
///
/// `/bin/sh` is tried first, so the real interpreter is used wherever
/// it qualifies — as it does on Arch and macOS, which is precisely
/// where a `${#VAR}` guard would miscount.
///
/// Locale candidates are the two conventional `C.UTF-8` spellings,
/// which are commonly usable without appearing in `locale -a`,
/// followed by every UTF-8 entry `locale -a` does report. **Fails
/// loudly when no pair qualifies** — a skip here would be
/// indistinguishable from a pass.
fn char_counting_shell() -> (String, String) {
    let installed = std::process::Command::new("locale")
        .arg("-a")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();

    let mut locales: Vec<String> = ["C.UTF-8".to_owned(), "C.utf8".to_owned()]
        .into_iter()
        .chain(
            installed
                .lines()
                .filter(|l| l.to_ascii_lowercase().contains("utf"))
                .map(str::to_owned),
        )
        .collect();
    locales.dedup();

    let shells = ["/bin/sh", "/bin/bash", "bash"];
    for shell in shells {
        for locale in &locales {
            let probe = std::process::Command::new(shell)
                .arg("-c")
                .arg("x=é; echo ${#x}")
                .env("LC_ALL", locale)
                .env("LANG", locale)
                .output();
            if let Ok(out) = probe
                && String::from_utf8_lossy(&out.stdout).trim() == "1"
            {
                return (shell.to_owned(), locale.clone());
            }
        }
    }
    panic!(
        "no available shell counts characters under any installed \
         locale, so the byte-versus-character distinction this row \
         exists to test cannot be established here. Tried shells \
         {shells:?} against locales {locales:?}"
    );
}

/// A root whose full path is exactly `total` bytes.
fn root_of_len(total: usize) -> tempfile::TempDir {
    let base = short_root_base();
    let fixed = base.display().to_string().len() + 1 + 6; // `<base>/` + XXXXXX
    let dir = tempfile::Builder::new()
        .prefix(&"a".repeat(total.saturating_sub(fixed)))
        .tempdir_in(&base)
        .expect("sized root");
    assert_eq!(
        dir.path().to_string_lossy().len(),
        total,
        "fixture must be exactly {total} bytes"
    );
    dir
}

/// Run the gate with the ancestor escape set (so only the LENGTH guard
/// can speak) and no ambient `TMPDIR`.
fn run_guarded(root: &Path) -> std::process::Output {
    std::process::Command::new(gate())
        .arg("--self-test")
        .current_dir(repo_root())
        .env("PMACS_GATE_TARGET_ROOT", root)
        .env("PMACS_GATE_ALLOW_ANCESTOR_MARKER", "1")
        .env_remove("TMPDIR")
        .output()
        .expect("run gate")
}

/// Leftovers in **both** areas a run creates: `<root>/tmp/*` and the
/// derived target's `gate-ambient/*`.
fn leaked_temp_areas(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root.join("tmp")) {
        out.extend(rd.filter_map(Result::ok).map(|e| e.path()));
    }
    // The ambient root DOES live under the derived per-worktree target
    // (unlike the tmp parent, which is shared), and its name is a hash
    // this test does not compute — so every `gate-ambient` beneath the
    // root is inspected.
    if let Ok(rd) = std::fs::read_dir(root) {
        for entry in rd.filter_map(Result::ok) {
            if let Ok(inner) = std::fs::read_dir(entry.path().join("gate-ambient")) {
                out.extend(inner.filter_map(Result::ok).map(|e| e.path()));
            }
        }
    }
    out
}

/// The ancestor walk is **canonical** and **does not word-split**.
///
/// Both properties were fixed without a witness, and reverting to the
/// obvious `for _anc in $(... dirname ...)` loop left every other row
/// green — so the suite constrained the check's existence and neither
/// of its two hard-won properties.
///
/// * **A space in the root** is torn into fragments by an unquoted
///   `$(...)` expansion, and the real ancestor is then never tested —
///   the guard passes on exactly the path it must reject.
/// * **A symlinked root** hides a marker under lexical `dirname` that
///   `detect_project` sees after canonicalization, so the gate and the
///   editor would disagree about the same tree.
#[test]
fn the_ancestor_walk_is_canonical_and_does_not_word_split() {
    // A space in the path, with a marker above it.
    let spaced = tempfile::Builder::new()
        .prefix("has space ")
        .tempdir_in(short_root_base())
        .expect("spaced base");
    assert!(
        spaced.path().to_string_lossy().contains(' '),
        "fixture must actually contain a space"
    );
    let marker = spaced.path().join(".git");
    std::fs::create_dir(&marker).expect("marker");
    let root = spaced.path().join("inner");
    std::fs::create_dir_all(&root).expect("root");

    let out = run_unescaped(&root);
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        err.contains(&format!("{}", marker.display())),
        "a marker above a root containing a SPACE must be found and \
         named; stderr:\n{err}"
    );

    // A symlinked root whose marker is only visible after resolving.
    let base = tempfile::Builder::new()
        .prefix("sym-")
        .tempdir_in(short_root_base())
        .expect("sym base");
    let real = base.path().join("real");
    std::fs::create_dir_all(real.join("inner")).expect("real tree");
    let hidden = real.join(".git");
    std::fs::create_dir(&hidden).expect("hidden marker");
    let link = base.path().join("link");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");

    let out = run_unescaped(&link.join("inner"));
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        !out.status.success(),
        "a marker reachable only after canonicalization must still \
         refuse; stderr:\n{err}"
    );
    assert!(
        err.contains(&format!("{}", hidden.display())),
        "and must name it at its RESOLVED path, which is what \
         `detect_project` would see; stderr:\n{err}"
    );
}

/// Run the gate with the ancestor check ACTIVE (no escape).
fn run_unescaped(root: &Path) -> std::process::Output {
    std::process::Command::new(gate())
        .arg("--self-test")
        .current_dir(repo_root())
        .env("PMACS_GATE_TARGET_ROOT", root)
        .env_remove("PMACS_GATE_ALLOW_ANCESTOR_MARKER")
        .env_remove("TMPDIR")
        .output()
        .expect("run gate")
}

/// The seam handoff §3 keeps authority over: a script cannot infer
/// which acceptance suites a change touched, so it runs what it is
/// handed — each one, in order.
#[test]
fn acceptance_suites_reach_the_plan_in_the_order_given() {
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
    let link_home = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
    let home = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
    let (orphan, lookalike, live) = prune_fixture(root.path());
    let outside = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");

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
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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
    let root = tempfile::Builder::new()
        .prefix("g-")
        .tempdir_in(short_root_base())
        .expect("tempdir");
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
