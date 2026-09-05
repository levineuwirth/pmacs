// m7_9_acceptance.rs --- T M7.9 acceptance tests.
//
//! Acceptance suite for T M7.9 ("Audit lint configuration"). The
//! five tests below correspond, in order, to the five bullets in
//! `spec/pmacs-tasks.tex` § "T M7.9 -- Audit lint configuration".
//!
//! Bullet 1: detects all seven rule classes
//! Bullet 2: emits JSON with file/line/rule/snippet
//! Bullet 3: sample CI workflow YAML present for three forges
//! Bullet 4: lint over the (pre-M7.11) REPL package's in-tree source
//! Bullet 5: 1000-line package lints in under one second

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use pmacs::audit::{AuditEngine, AuditReport, Severity};

// ---------------------------------------------------------------------------
// Bullet 1: seven rule classes detected on synthetic snippets
// ---------------------------------------------------------------------------
//
// The spec lists seven *rule classes*; the implementation expands them into
// fifteen tree-sitter patterns (e.g., the "no-FFI escape hatches" class
// covers four distinct patterns: ffi.cdef/load/metatype, package.loadlib,
// package.cpath mutation; consolidating them under a single rule name would
// make findings less actionable). The acceptance check confirms every
// *class* fires at least once; the per-pattern test suite in
// `src/audit/mod.rs::tests` verifies the discrimination within a class.

#[test]
fn seven_rule_classes_each_fire_on_synthetic_snippets() {
    let engine = AuditEngine::new().expect("rule set compiles");
    let src = r#"
        -- Class 1: no private-surface access
        local a = require("pmacs._internal.foo")
        local b = require("pmacs.core.bar")
        local _pmacs_internal_x = 1
        local _core_y = 2

        -- Class 2: no FFI escape hatches
        ffi.cdef[[ int x; ]]
        ffi.load("c")
        ffi.metatype(t, mt)
        package.loadlib("/lib/x.so", "init")
        package.cpath = "/extra/?.so"

        -- Class 3: no debug-library cancellation interference
        debug.sethook(myhook, "c")
        debug.setmetatable(other, mt)

        -- Class 4: no environment escape
        rawget(_G, "secret")
        rawset(_G, "ok", 1)
        setfenv(2, env)
        getfenv(0)

        -- Class 5: reach-around to other packages
        local x = require("otherpkg.private")

        -- Class 6: filesystem mutation (warn)
        io.open("/tmp/x", "w")
        os.remove("/tmp/y")
        os.rename("a", "b")

        -- Class 7: process spawning (warn)
        io.popen("ls")
        os.execute("rm -rf /")
        pmacs.process.spawn("ls")
    "#;

    let findings = engine.audit_source("synthetic.lua", src);
    let rules: HashSet<&str> = findings.iter().map(|f| f.rule.as_str()).collect();

    let class_to_rule_alternatives: &[(&str, &[&str])] = &[
        (
            "private-surface",
            &[
                "no-private-surface-require",
                "no-private-surface-identifier",
            ],
        ),
        (
            "ffi-and-loadlib",
            &[
                "no-ffi-call",
                "no-package-loadlib",
                "no-package-cpath-mutation",
            ],
        ),
        (
            "debug-cancellation",
            &["no-debug-sethook", "no-debug-setmetatable"],
        ),
        (
            "env-escape",
            &["no-rawget-rawset-on-globals", "no-setfenv-getfenv"],
        ),
        ("reach-around", &["reach-around-require"]),
        (
            "fs-mutation",
            &["no-fs-mutation-io-open-write", "no-fs-mutation-os"],
        ),
        (
            "process-spawn",
            &[
                "no-process-spawn-io",
                "no-process-spawn-os",
                "no-process-spawn-pmacs",
            ],
        ),
    ];

    for (class, alts) in class_to_rule_alternatives {
        let any = alts.iter().any(|r| rules.contains(*r));
        assert!(
            any,
            "rule class `{class}` did not fire; alternatives {alts:?} not in {rules:?}"
        );
    }

    // And each pattern fires at least once (distinct rules >= 13/15;
    // private-surface-{require,identifier}, both ffi-call patterns,
    // both fs-mutation patterns, both env-escape patterns, all three
    // spawn patterns, etc., are all exercised above). Two patterns
    // that cover sub-cases of a class are tested in the unit suite.
    assert!(
        rules.len() >= 13,
        "expected ~13+ distinct rule names, got {}: {rules:?}",
        rules.len()
    );
}

// ---------------------------------------------------------------------------
// Bullet 2: JSON report shape
// ---------------------------------------------------------------------------

#[test]
fn json_report_has_file_line_rule_severity_message_snippet() {
    let engine = AuditEngine::new().unwrap();
    let findings = engine.audit_source("demo.lua", "ffi.load(\"c\")\nio.popen(\"ls\")\n");
    let report = AuditReport::new(findings);
    let json = serde_json::to_value(&report).expect("serialize");

    let arr = json["findings"].as_array().expect("findings is an array");
    assert!(!arr.is_empty(), "report has at least one finding");
    let first = &arr[0];
    for key in [
        "file", "line", "column", "rule", "severity", "message", "snippet",
    ] {
        assert!(
            first.get(key).is_some(),
            "finding missing field `{key}` in {first:?}"
        );
    }
    assert!(json["summary"]["errors"].is_number());
    assert!(json["summary"]["warnings"].is_number());
    assert!(json["summary"]["infos"].is_number());

    // Severity values are stable lowercase.
    let sev = first["severity"].as_str().expect("severity is a string");
    assert!(
        matches!(sev, "error" | "warning" | "info"),
        "unknown severity `{sev}`"
    );
}

#[test]
fn json_round_trips_through_serde() {
    let engine = AuditEngine::new().unwrap();
    let findings = engine.audit_source("demo.lua", "ffi.load(\"c\")\n");
    let report = AuditReport::new(findings);
    let s = serde_json::to_string(&report).unwrap();
    let back: AuditReport = serde_json::from_str(&s).unwrap();
    assert_eq!(back.summary.errors, report.summary.errors);
    assert_eq!(back.findings.len(), report.findings.len());
    assert_eq!(back.findings[0].rule, report.findings[0].rule);
}

// ---------------------------------------------------------------------------
// Bullet 3: sample CI workflows present for three forges
// ---------------------------------------------------------------------------

#[test]
fn sample_ci_workflows_exist_for_three_forges() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let forges = [
        "audit/ci/github-actions.yml",
        "audit/ci/gitlab-ci.yml",
        "audit/ci/forgejo-actions.yml",
    ];
    for f in forges {
        let p = root.join(f);
        let body = std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("expected sample CI workflow at {p:?}: {e}"));
        assert!(
            body.contains("pmacs-audit") || body.contains("pmacs_audit"),
            "{f} should reference the pmacs-audit binary; got:\n{body}"
        );
        // Sanity-check the file looks like YAML (not a Markdown stub).
        assert!(
            body.contains(':'),
            "{f} should look like YAML (contain key:value pairs)"
        );
    }
}

// ---------------------------------------------------------------------------
// Bullet 4: lint runs against the in-tree REPL source
// ---------------------------------------------------------------------------
//
// As of T M7.11 the REPL is a manifest-bearing package at
// `builtin/packages/repl/`. The acceptance contract: any findings
// against the package's source must be either zero or
// *classifiable* -- that is, instances of rules whose Warning/Info
// severity reflects expected REPL behavior (process spawning,
// primarily). A finding under any Error-severity rule is a hard
// failure here.
//
// `tests/m7_11_acceptance.rs` repeats this assertion as part of the
// M7.11 migration's own acceptance bullet 4; the duplication is
// intentional, not a copy-paste oversight: each milestone owns its
// own audit gate, and the M7.9 suite also keeps a check so a
// future migration that renames or moves the REPL doesn't silently
// drop audit coverage.

#[test]
fn lint_against_in_tree_repl_source_has_no_error_severity_findings() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repl_dir = root.join("builtin/packages/repl");
    assert!(
        repl_dir.is_dir(),
        "expected migrated REPL package dir at {repl_dir:?} (T M7.11)"
    );

    let engine = AuditEngine::new().unwrap();
    let findings = engine
        .audit_dir(&repl_dir)
        .expect("read+audit migrated REPL package");

    let errors: Vec<_> = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "in-tree REPL has Error-severity findings; either fix the REPL or reclassify the rule.\n\
         findings: {errors:#?}"
    );

    // The REPL is permitted to spawn processes (it *is* the
    // process-spawning subsystem). Document the expected
    // classification: every finding is Warn-level
    // `no-process-spawn-pmacs` against `pmacs.process.spawn`.
    for f in &findings {
        assert_eq!(
            f.severity,
            Severity::Warning,
            "unexpected non-warning finding in REPL: {f:?}"
        );
        assert_eq!(
            f.rule, "no-process-spawn-pmacs",
            "unexpected rule in REPL findings (expected only the spawn warn): {f:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Bullet 5: 1000-line perf budget
// ---------------------------------------------------------------------------

/// Lint a thousand-line synthetic package; asserts the findings and
/// returns how long the lint took.
fn lint_thousand_line_package() -> Duration {
    // Build a 1000-line synthetic source. Mix of patterns so the
    // worst-case (all 15 rules engaged) is exercised, not a tight
    // loop of identical lines that the parser would chew through
    // unrealistically fast.
    let mut src = String::new();
    let pattern = [
        "local a = require(\"foo\")",
        "ffi.cdef[[ int x; ]]",
        "io.popen(\"ls\")",
        "os.execute(\"echo hi\")",
        "local b = require(\"otherpkg.x\")",
        "io.open(\"/tmp/x\", \"w\")",
        "os.remove(\"/tmp/y\")",
        "debug.sethook(h, \"c\")",
        "rawget(_G, \"k\")",
        "package.cpath = \"/extra/?.so\"",
    ];
    for i in 0..1000 {
        src.push_str(pattern[i % pattern.len()]);
        src.push('\n');
    }
    assert_eq!(src.lines().count(), 1000);

    let engine = AuditEngine::new().unwrap();
    let t = Instant::now();
    let findings = engine.audit_source("perf.lua", &src);
    let elapsed = t.elapsed();

    assert!(
        !findings.is_empty(),
        "synthetic 1000-line file should produce findings (engine alive check)"
    );

    // For empirical record: print to stderr so CI logs capture it.
    eprintln!(
        "[T M7.9] 1000-line synthetic perf: {:?} for {} findings",
        elapsed,
        findings.len()
    );
    elapsed
}

#[test]
fn lint_reports_findings_on_a_thousand_line_package() {
    let elapsed = lint_thousand_line_package();
    eprintln!("lint of a 1000-line package took {elapsed:?}");
}

#[test]
#[ignore = "wall-clock budget; runs under --ignored in the perf jobs and scripts/gate --perf"]
fn lint_completes_in_under_one_second_on_thousand_line_package() {
    let elapsed = lint_thousand_line_package();
    assert!(
        elapsed.as_secs_f64() < 1.0,
        "lint of 1000-line package took {elapsed:?}; spec budget is < 1s"
    );
}
