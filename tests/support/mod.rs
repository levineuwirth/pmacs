//! Shared test-support helpers.
//!
//! Included by `#[path = "support/mod.rs"] mod support;` rather than
//! copied.
//!
//! **Why this is separate from `tests/common/`, which also exists.**
//! `tests/common/mod.rs` re-exports `daemon` and `pty` — real daemon
//! spawning and PTY plumbing. Including it to reach a six-line
//! environment check would compile that machinery into three test
//! binaries that spawn neither, for no benefit. `support` is the
//! dependency-free half: helpers any test binary can take without
//! taking a subsystem with them. Two directories is a cost worth
//! naming rather than leaving to be rediscovered; if a third appears,
//! consolidate instead of continuing the pattern. Files under `tests/` subdirectories are not compiled as
//! their own test binaries, so this costs nothing — and
//! `m6_8_multi_repl_acceptance.rs` previously carried a comment saying
//! cross-test-binary sharing "would need a fixture crate", which is not
//! so. A correct helper in one file and a degraded copy in another is
//! this suite's most repeated defect shape; sharing removes the way it
//! happens.

#![allow(dead_code)]

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

#[track_caller]
pub fn skip_or_fail(tool: &str, require_var: &str) {
    assert!(
        !armed(require_var),
        "{require_var} is set, but `{tool}` is not on PATH. \
         The CI step that installs it did not run, or installed it \
         somewhere not on PATH. This is a hard failure precisely so \
         the test cannot report green without executing."
    );
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
    eprintln!(
        "skipping: {tool} not on PATH (set {override_var} to override, \
         or {require_var} to make this fatal)"
    );
}
