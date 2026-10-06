//! Shared test-support helpers.
//!
//! Included by `#[path = "support/mod.rs"] mod support;` rather than
//! copied. Files under `tests/` subdirectories are not compiled as
//! their own test binaries, so this costs nothing — and
//! `m6_8_multi_repl_acceptance.rs` previously carried a comment saying
//! cross-test-binary sharing "would need a fixture crate", which is not
//! so. A correct helper in one file and a degraded copy in another is
//! this suite's most repeated defect shape; sharing removes the way it
//! happens.
//!
//! **Why this is separate from `tests/common/`, which also exists.**
//! `tests/common/mod.rs` re-exports `daemon` and `pty` — real daemon
//! spawning and PTY plumbing. Including it to reach a six-line
//! environment check would compile that machinery into three test
//! binaries that spawn neither, for no benefit. `support` is the
//! dependency-free half: helpers any test binary can take without
//! taking a subsystem with them. Two directories is a cost worth
//! naming rather than leaving to be rediscovered; if a third appears,
//! consolidate instead of continuing the pattern.

#![allow(dead_code)]

/// E7c: a wire tee around a language server, for the measurements.
#[path = "wire_tee.rs"]
pub mod wire_tee;

/// Report a missing external tool, and turn the skip into a HARD
/// FAILURE when the environment has promised the tool is present.
///
/// The bare shape this replaces —
///
/// ```ignore
/// let Ok(_) = which_binary("gopls") else {
///     eprintln!("gopls not on PATH; skipping");
///     return;
/// };
/// ```
///
/// passes GREEN when the tool is absent, and is why a large block of
/// external-tool-gated tests had never once executed their bodies in
/// CI: nothing installed the tools, so every one of them reported
/// success without running. A suite that cannot tell "passed" from
/// "never ran" is worse than a missing suite, because it reads as
/// coverage.
///
/// `PMACS_REQUIRE_*` is the project's own fix, already load-bearing for
/// `PMACS_REQUIRE_GPU` in `vterm_stage3_acceptance`: CI installs the
/// tool, sets the variable, and absence becomes a failure that names
/// the step that should have provided it. Locally the variable is
/// unset, so the skip still works and nobody needs the whole toolchain
/// to run the suite.
///
/// Deliberately per-tool rather than one blanket variable: a tool that
/// must stay unarmed (because arming it would hang, or because CI does
/// not install it yet) keeps its own variable that CI never sets, and
/// that decision is then visible at the call site instead of buried in
/// a workflow file.
/// True when `var` is set to a non-empty value.
///
/// Emptiness matters, and the reason is a trap rather than a nicety.
/// The natural GitHub Actions idiom for a conditional environment
/// variable —
///
/// ```yaml
/// PMACS_REQUIRE_LSP: ${{ runner.os == 'Linux' && '1' || '' }}
/// ```
///
/// sets the variable to the EMPTY STRING on every other platform, not
/// to nothing. A bare `var_os(..).is_some()` is therefore true there,
/// which would arm the guard on exactly the runners that have none of
/// the tools installed and fail every one of them. Treating empty as
/// unset makes the common workflow spelling safe instead of subtly
/// wrong.
fn armed(var: &str) -> bool {
    std::env::var_os(var).is_some_and(|v| !v.is_empty())
}

/// Record that the calling row is returning without running, in the
/// file `PMACS_SKIP_LOG` names, one line per skip: the suite and the
/// row, the call site, and why.
///
/// libtest reports a row whose body returns as `ok` (`calc_result`),
/// and shows a passing row's output only under `--nocapture` or
/// `--show-output`, so the notice printed beside the return reaches no
/// log. A CI leg sets the variable and prints the file after its tests,
/// beside its arming report (`scripts/gate --print-arming-env`), so the
/// leg's log says which rows did not run and why. Unset, as it is
/// locally, this records nothing. A failed write is not the row's
/// failure, so it is reported on the notice and not asserted.
#[track_caller]
pub fn record_skip(why: &str) {
    let Some(path) = std::env::var_os("PMACS_SKIP_LOG").filter(|v| !v.is_empty()) else {
        return;
    };
    let caller = std::panic::Location::caller();
    let row = std::thread::current()
        .name()
        .unwrap_or("<unnamed thread>")
        .to_owned();
    let line = format!(
        "{}::{row} ({}:{}): {why}\n",
        env!("CARGO_CRATE_NAME"),
        caller.file(),
        caller.line()
    );
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
    if let Err(e) = written {
        eprintln!(
            "could not record this skip in {}: {e}",
            std::path::Path::new(&path).display()
        );
    }
}

#[track_caller]
pub fn skip_or_fail(tool: &str, require_var: &str) {
    assert!(
        !armed(require_var),
        "{require_var} is set, but `{tool}` is not on PATH. \
         The CI step that installs it did not run, or installed it \
         somewhere not on PATH. This is a hard failure precisely so \
         the test cannot report green without executing."
    );
    record_skip(&format!("`{tool}` not on PATH and {require_var} unset"));
    eprintln!("{tool} not on PATH; skipping (set {require_var} to make this fatal)");
}

/// As [`skip_or_fail`], for tools whose PATH lookup can be overridden
/// by a `PMACS_TEST_*` variable. The skip notice keeps naming that
/// override, because losing it would make the local escape hatch
/// undiscoverable — the REPL suites are routinely run on machines
/// without zsh or fish.
#[track_caller]
pub fn skip_or_fail_overridable(tool: &str, require_var: &str, override_var: &str) {
    assert!(
        !armed(require_var),
        "{require_var} is set, but `{tool}` is not on PATH and {override_var} \
         is unset or points at nothing. The CI step that installs it did not \
         run, or installed it somewhere not on PATH."
    );
    record_skip(&format!(
        "`{tool}` not on PATH, {override_var} unset or naming nothing, and {require_var} unset"
    ));
    eprintln!(
        "skipping: {tool} not on PATH (set {override_var} to override, \
         or {require_var} to make this fatal)"
    );
}
