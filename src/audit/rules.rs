// audit/rules.rs --- Rule metadata table aligned with audit-rules.scm.

//! Rule metadata for the audit lint (T M7.9).
//!
//! [`DEFAULT_RULES`] is a fixed-order array; each entry corresponds
//! by index to the `pattern_index` produced by
//! [`tree_sitter::QueryMatch::pattern_index`] when running the
//! [`crate::audit::AUDIT_QUERY_SOURCE`] query against a Lua AST. The
//! ordering here mirrors the comments in
//! `audit/audit-rules.scm`; a unit test asserts that the pattern
//! count and the table length agree at compile time.

use serde::{Deserialize, Serialize};

/// Rule severity. The CLI exits non-zero if any [`Severity::Error`]
/// finding is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Hard violation. Forbidden by the v1.0 rule set; the audit
    /// reviewer cannot classify these away without changing the
    /// rule set itself.
    Error,
    /// Soft violation. Permitted only for packages whose manifest
    /// declares the relevant capability (filesystem write, process
    /// spawn). The classification mechanism that promotes these to
    /// errors lives in M11.
    Warning,
    /// Informational finding. The reviewer must classify each one;
    /// the lint cannot decide on its own (e.g., reach-around to
    /// another package's private surface requires knowing that
    /// package's `exports`).
    Info,
}

/// Static rule metadata. The `query` field exists for documentation
/// only --- the actual matching uses the compiled
/// [`tree_sitter::Query`] from `audit/audit-rules.scm`. We keep the
/// human-readable rule shape here so JSON consumers can render
/// findings without re-parsing the .scm file.
#[derive(Debug, Clone, Copy)]
pub struct AuditRule {
    /// Stable identifier referenced in CI configuration and the
    /// JSON report. Lowercase, hyphen-separated.
    pub name: &'static str,
    /// Severity at which this rule fires.
    pub severity: Severity,
    /// One-line human-readable explanation.
    pub message: &'static str,
}

/// The v1.0 rule table. Order **must** mirror `audit-rules.scm`'s
/// pattern order; an index mismatch silently misclassifies findings.
///
/// The unit test `default_rules_count_matches_query_pattern_count`
/// asserts the alignment at runtime.
pub const DEFAULT_RULES: &[AuditRule] = &[
    // 0
    AuditRule {
        name: "no-private-surface-require",
        severity: Severity::Error,
        message: "require() of pmacs._internal.* / pmacs.core.* surface is forbidden",
    },
    // 1
    AuditRule {
        name: "no-private-surface-identifier",
        severity: Severity::Error,
        message: "identifiers prefixed with `_pmacs_internal_` or `_core_` are private",
    },
    // 2
    AuditRule {
        name: "no-ffi-call",
        severity: Severity::Error,
        message: "ffi.cdef / ffi.load / ffi.metatype escape the Lua sandbox",
    },
    // 3
    AuditRule {
        name: "no-package-loadlib",
        severity: Severity::Error,
        message: "package.loadlib loads native code outside the package sandbox",
    },
    // 4
    AuditRule {
        name: "no-package-cpath-mutation",
        severity: Severity::Error,
        message: "mutating package.cpath extends the C-loader search path",
    },
    // 5
    AuditRule {
        name: "no-debug-sethook",
        severity: Severity::Error,
        message: "debug.sethook would override the M7.8 cancellation hook",
    },
    // 6
    AuditRule {
        name: "no-debug-setmetatable",
        severity: Severity::Error,
        message: "debug.setmetatable bypasses normal metatable rules",
    },
    // 7
    AuditRule {
        name: "no-rawget-rawset-on-globals",
        severity: Severity::Error,
        message: "rawget(_G,...) / rawset(_G,...) escape per-package _ENV sandboxing",
    },
    // 8
    AuditRule {
        name: "no-setfenv-getfenv",
        severity: Severity::Error,
        message: "setfenv / getfenv manipulate other packages' environments",
    },
    // 9
    AuditRule {
        name: "no-fs-mutation-io-open-write",
        severity: Severity::Warning,
        message: "io.open(..., \"w|a|+\") writes the filesystem; declare fs access",
    },
    // 10
    AuditRule {
        name: "no-fs-mutation-os",
        severity: Severity::Warning,
        message: "os.remove / os.rename mutate the filesystem; declare fs access",
    },
    // 11
    AuditRule {
        name: "no-process-spawn-io",
        severity: Severity::Warning,
        message: "io.popen spawns a child process; declare process access",
    },
    // 12
    AuditRule {
        name: "no-process-spawn-os",
        severity: Severity::Warning,
        message: "os.execute spawns a child process; declare process access",
    },
    // 13
    AuditRule {
        name: "no-process-spawn-pmacs",
        severity: Severity::Warning,
        message: "pmacs.process.spawn launches a managed child; declare process access",
    },
    // 14
    AuditRule {
        name: "reach-around-require",
        severity: Severity::Info,
        message: "dotted require may target another package's non-exported submodule",
    },
    // 15
    AuditRule {
        name: "reach-around-require-field",
        severity: Severity::Info,
        message: "private-looking field access on require() may reach another package's internals",
    },
];
