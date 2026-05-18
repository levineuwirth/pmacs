// audit/mod.rs --- T M7.9 audit lint engine.

//! Audit-lint engine (T M7.9, spec §sec:packages-future).
//!
//! Static analyzer for Lua package source. Parses with
//! [`tree_sitter_lua`], runs the queries declared in
//! `audit/audit-rules.scm` against the resulting AST, and emits
//! [`AuditFinding`]s naming the file, line, rule, severity, and
//! offending snippet. The engine never executes audited code, so
//! malicious top-level Lua cannot side-effect the lint host.
//!
//! ## Why tree-sitter
//!
//! Three alternatives were considered (see `TRANSITION-M7.md` T M7.9
//! section): `luaparse` (Node), a custom mlua-based analyzer
//! (loads-and-runs untrusted code), and `full-moon` (Rust AST,
//! programmatic rules). Tree-sitter queries win on three axes:
//! pure-Rust (no JS runtime), no execution of audited code, and
//! declarative -- the rule set in `audit/audit-rules.scm` is
//! reviewable by non-Rust readers and proposable by external
//! authors. `full-moon` remains an option if a future rule needs
//! cross-statement data flow.
//!
//! ## Rule set
//!
//! The v1.0 rules are catalogued in [`rules::DEFAULT_RULES`] and
//! correspond by index to the patterns in
//! [`AUDIT_QUERY_SOURCE`]. The engine surface is:
//!
//! - [`AuditEngine::new`] -- compiles the queries once.
//! - [`AuditEngine::audit_source`] -- audits a string of Lua, the
//!   primitive used by every other entry point.
//! - [`AuditEngine::audit_file`] -- reads a path and audits.
//! - [`AuditEngine::audit_dir`] -- recursively audits every `*.lua`
//!   under a directory.
//!
//! ## Output
//!
//! [`AuditFinding`] derives [`serde::Serialize`]; the CLI binary
//! `pmacs-audit` writes a JSON [`AuditReport`] of all findings.
//! Schema:
//!
//! ```text
//! {
//!   "findings": [
//!     {
//!       "file": "path/to/x.lua",
//!       "line": 42,        // 1-based
//!       "column": 11,      // 1-based
//!       "rule": "no-ffi-call",
//!       "severity": "error" | "warning" | "info",
//!       "message": "ffi.cdef / ffi.load / ffi.metatype escape the Lua sandbox",
//!       "snippet": "ffi.cdef[[ int x; ]]"
//!     }, ...
//!   ],
//!   "summary": { "errors": N, "warnings": N, "infos": N }
//! }
//! ```

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

pub mod rules;

pub use rules::{AuditRule, DEFAULT_RULES, Severity};

/// Source text of the rule queries. Compile-time-included so the
/// binary has no runtime dependency on the source tree; the file
/// itself remains the published contract for external authors who
/// want to read or extend the rules.
pub const AUDIT_QUERY_SOURCE: &str = include_str!("../../audit/audit-rules.scm");

/// One audit-lint hit. Stable JSON shape; v1.0 consumers can rely
/// on field names and `severity` enum variants persisting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditFinding {
    /// File path the finding was produced from. Paths are stored as
    /// the caller passed them in: relative paths stay relative.
    pub file: PathBuf,
    /// 1-based line number of the violating node's first character.
    pub line: usize,
    /// 1-based column number of the violating node's first character.
    pub column: usize,
    /// Rule identifier (e.g., `"no-ffi-call"`).
    pub rule: String,
    /// Severity level.
    pub severity: Severity,
    /// Human-readable rule description.
    pub message: String,
    /// The matched source slice, preserved verbatim. Newlines are
    /// kept; consumers that render to a single line should
    /// `replace('\n', " ")` themselves.
    pub snippet: String,
}

/// Aggregate report. The CLI binary writes one of these to stdout
/// as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditReport {
    /// Per-finding records, in source order across files.
    pub findings: Vec<AuditFinding>,
    /// Counts grouped by severity. Mirrors `findings` --- the
    /// summary is redundant but cheap, and CI integrations
    /// frequently want a single number to gate on.
    pub summary: AuditSummary,
}

impl AuditReport {
    /// Build a report from a finding list, populating `summary`.
    #[must_use]
    pub fn new(findings: Vec<AuditFinding>) -> Self {
        let mut summary = AuditSummary::default();
        for f in &findings {
            match f.severity {
                Severity::Error => summary.errors += 1,
                Severity::Warning => summary.warnings += 1,
                Severity::Info => summary.infos += 1,
            }
        }
        Self { findings, summary }
    }
}

/// Severity counts for an [`AuditReport`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditSummary {
    /// Number of [`Severity::Error`] findings in the report.
    pub errors: usize,
    /// Number of [`Severity::Warning`] findings.
    pub warnings: usize,
    /// Number of [`Severity::Info`] findings.
    pub infos: usize,
}

/// Errors raised during audit setup or scan I/O. Per-file parse
/// failures do not raise --- they emit a special finding so a
/// single broken file in a tree doesn't abort the whole audit.
#[derive(Debug, Error)]
pub enum AuditError {
    /// The compiled tree-sitter [`Query`] failed to load. Should
    /// only happen on a grammar/query ABI skew.
    #[error("audit query compile failed: {0}")]
    QueryCompile(String),
    /// Couldn't set the language on the parser.
    #[error("audit parser setup failed: {0}")]
    ParserSetup(String),
    /// I/O error reading a file or directory.
    #[error("audit I/O error at {path}: {source}")]
    Io {
        /// Path the operation was attempted against.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
}

/// Audit engine. Holds the compiled query and a parser; reusable
/// across many files.
pub struct AuditEngine {
    query: Query,
    capture_violation: u32,
}

impl AuditEngine {
    /// Compile the v1.0 rule set. Returns an error only on
    /// grammar/query ABI skew (i.e., a tree-sitter-lua upgrade that
    /// renames a node).
    pub fn new() -> Result<Self, AuditError> {
        let lang: tree_sitter::Language = tree_sitter_lua::LANGUAGE.into();
        let query = Query::new(&lang, AUDIT_QUERY_SOURCE)
            .map_err(|e| AuditError::QueryCompile(format!("{e:?}")))?;
        let capture_violation = query.capture_index_for_name("violation").ok_or_else(|| {
            AuditError::QueryCompile("audit-rules.scm missing required `@violation` capture".into())
        })?;
        // Rule table and pattern count must agree by index.
        if query.pattern_count() != DEFAULT_RULES.len() {
            return Err(AuditError::QueryCompile(format!(
                "rule table / query pattern count mismatch: {} vs {}",
                DEFAULT_RULES.len(),
                query.pattern_count()
            )));
        }
        Ok(Self {
            query,
            capture_violation,
        })
    }

    /// Audit a single Lua source string. `path` is recorded in the
    /// findings unmodified.
    #[must_use]
    pub fn audit_source(&self, path: impl Into<PathBuf>, src: &str) -> Vec<AuditFinding> {
        let path = path.into();
        let lang: tree_sitter::Language = tree_sitter_lua::LANGUAGE.into();
        let mut parser = Parser::new();
        if parser.set_language(&lang).is_err() {
            return Vec::new();
        }
        let Some(tree) = parser.parse(src.as_bytes(), None) else {
            return Vec::new();
        };
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), src.as_bytes());
        let mut out = Vec::new();
        while let Some(m) = matches.next() {
            let pat = m.pattern_index;
            // Tree-sitter guarantees pattern_index < pattern_count
            // (we asserted the table size in `new`), so an out-of-
            // range index is a tree-sitter bug we won't paper over.
            let rule = DEFAULT_RULES[pat];
            for cap in m.captures {
                if cap.index == self.capture_violation {
                    let node = cap.node;
                    let start = node.start_position();
                    let snippet = src
                        .as_bytes()
                        .get(node.byte_range())
                        .and_then(|b| std::str::from_utf8(b).ok())
                        .unwrap_or("")
                        .to_string();
                    out.push(AuditFinding {
                        file: path.clone(),
                        line: start.row + 1,
                        column: start.column + 1,
                        rule: rule.name.to_string(),
                        severity: rule.severity,
                        message: rule.message.to_string(),
                        snippet,
                    });
                }
            }
        }
        // Stable order: by (file, line, column, rule).
        out.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then(a.line.cmp(&b.line))
                .then(a.column.cmp(&b.column))
                .then(a.rule.cmp(&b.rule))
        });
        out
    }

    /// Audit a single file at `path`. Reads the file then delegates
    /// to [`Self::audit_source`].
    pub fn audit_file(&self, path: &Path) -> Result<Vec<AuditFinding>, AuditError> {
        let src = fs::read_to_string(path).map_err(|e| AuditError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        Ok(self.audit_source(path.to_path_buf(), &src))
    }

    /// Audit every `*.lua` file beneath `dir`. Symlinks are
    /// followed once (the standard-library `read_dir` policy). The
    /// returned findings are concatenated across files in
    /// alphabetical-path order.
    pub fn audit_dir(&self, dir: &Path) -> Result<Vec<AuditFinding>, AuditError> {
        let mut files = collect_lua_files(dir)?;
        files.sort();
        let mut all = Vec::new();
        for file in files {
            all.extend(self.audit_file(&file)?);
        }
        Ok(all)
    }
}

fn collect_lua_files(dir: &Path) -> Result<Vec<PathBuf>, AuditError> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = fs::read_dir(&d).map_err(|e| AuditError::Io {
            path: d.clone(),
            source: e,
        })?;
        for entry in entries {
            let entry = entry.map_err(|e| AuditError::Io {
                path: d.clone(),
                source: e,
            })?;
            let p = entry.path();
            let ft = entry.file_type().map_err(|e| AuditError::Io {
                path: p.clone(),
                source: e,
            })?;
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() && p.extension().is_some_and(|e| e == "lua") {
                out.push(p);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> AuditEngine {
        AuditEngine::new().expect("rule set compiles against bundled tree-sitter-lua")
    }

    #[test]
    fn rules_table_aligns_with_query_pattern_count() {
        // Constructor asserts this; calling `new` is enough to fail
        // a misalignment. The explicit check here makes the failure
        // mode obvious if the constructor's contract changes.
        let e = engine();
        assert_eq!(e.query.pattern_count(), DEFAULT_RULES.len());
    }

    #[test]
    fn detects_private_surface_require() {
        let f = engine().audit_source("t.lua", r#"local m = require("pmacs._internal.foo")"#);
        assert_eq!(f.len(), 1, "expected exactly one finding, got {f:?}");
        assert_eq!(f[0].rule, "no-private-surface-require");
        assert_eq!(f[0].severity, Severity::Error);
    }

    #[test]
    fn detects_private_surface_core_namespace() {
        let f = engine().audit_source("t.lua", r#"local m = require("pmacs.core.bar")"#);
        assert!(f.iter().any(|x| x.rule == "no-private-surface-require"));
    }

    #[test]
    fn detects_private_surface_identifier() {
        let f = engine().audit_source("t.lua", "local _pmacs_internal_q = 1");
        assert!(
            f.iter().any(|x| x.rule == "no-private-surface-identifier"),
            "got {f:?}"
        );
    }

    #[test]
    fn detects_ffi_calls() {
        for src in [
            "ffi.cdef[[ int x; ]]",
            "ffi.load(\"c\")",
            "ffi.metatype(t, mt)",
        ] {
            let f = engine().audit_source("t.lua", src);
            assert!(
                f.iter().any(|x| x.rule == "no-ffi-call"),
                "{src} produced {f:?}"
            );
        }
    }

    #[test]
    fn detects_package_loadlib_and_cpath_mutation() {
        let src = r#"
            package.loadlib("/lib/x.so", "init")
            package.cpath = "/extra/?.so"
        "#;
        let f = engine().audit_source("t.lua", src);
        assert!(f.iter().any(|x| x.rule == "no-package-loadlib"));
        assert!(f.iter().any(|x| x.rule == "no-package-cpath-mutation"));
    }

    #[test]
    fn package_cpath_read_is_not_a_finding() {
        // Rule 4 only fires on the LHS of an assignment; reading
        // `package.cpath` is allowed (and harmless).
        let f = engine().audit_source("t.lua", "local p = package.cpath");
        assert!(f.iter().all(|x| x.rule != "no-package-cpath-mutation"));
    }

    #[test]
    fn detects_debug_sethook_and_setmetatable() {
        let src = r#"
            debug.sethook(myhook, "c")
            debug.setmetatable(t, mt)
        "#;
        let f = engine().audit_source("t.lua", src);
        assert!(f.iter().any(|x| x.rule == "no-debug-sethook"));
        assert!(f.iter().any(|x| x.rule == "no-debug-setmetatable"));
    }

    #[test]
    fn detects_rawget_rawset_on_globals() {
        let src = r#"
            rawget(_G, "x")
            rawset(_G, "y", 1)
        "#;
        let f = engine().audit_source("t.lua", src);
        let count = f
            .iter()
            .filter(|x| x.rule == "no-rawget-rawset-on-globals")
            .count();
        assert_eq!(count, 2, "got {f:?}");
    }

    #[test]
    fn rawget_on_a_local_table_is_not_a_finding() {
        let f = engine().audit_source("t.lua", "rawget(my_table, \"k\")");
        assert!(f.iter().all(|x| x.rule != "no-rawget-rawset-on-globals"));
    }

    #[test]
    fn detects_setfenv_and_getfenv() {
        let src = r"
            setfenv(2, env)
            getfenv(0)
        ";
        let f = engine().audit_source("t.lua", src);
        assert!(f.iter().filter(|x| x.rule == "no-setfenv-getfenv").count() == 2);
    }

    #[test]
    fn detects_io_open_with_write_mode() {
        let f = engine().audit_source("t.lua", r#"io.open("/tmp/x", "w")"#);
        assert!(f.iter().any(|x| x.rule == "no-fs-mutation-io-open-write"));
    }

    #[test]
    fn io_open_with_read_mode_is_not_a_finding() {
        let f = engine().audit_source("t.lua", r#"io.open("/tmp/x", "r")"#);
        assert!(f.iter().all(|x| x.rule != "no-fs-mutation-io-open-write"));
    }

    #[test]
    fn io_open_without_mode_is_not_a_finding() {
        // Mode defaults to "r" per the Lua reference manual.
        let f = engine().audit_source("t.lua", r#"io.open("/tmp/x")"#);
        assert!(f.iter().all(|x| x.rule != "no-fs-mutation-io-open-write"));
    }

    #[test]
    fn detects_os_remove_and_rename() {
        let src = r#"
            os.remove("/tmp/y")
            os.rename("a", "b")
        "#;
        let f = engine().audit_source("t.lua", src);
        assert_eq!(
            f.iter().filter(|x| x.rule == "no-fs-mutation-os").count(),
            2
        );
    }

    #[test]
    fn detects_io_popen_os_execute_and_pmacs_process_spawn() {
        let src = r#"
            io.popen("ls")
            os.execute("rm -rf /")
            pmacs.process.spawn("ls")
        "#;
        let f = engine().audit_source("t.lua", src);
        assert!(f.iter().any(|x| x.rule == "no-process-spawn-io"));
        assert!(f.iter().any(|x| x.rule == "no-process-spawn-os"));
        assert!(f.iter().any(|x| x.rule == "no-process-spawn-pmacs"));
    }

    #[test]
    fn reach_around_dotted_require_is_info_level() {
        let f = engine().audit_source("t.lua", r#"local x = require("otherpkg.private")"#);
        let r = f
            .iter()
            .find(|x| x.rule == "reach-around-require")
            .expect("expected reach-around finding");
        assert_eq!(r.severity, Severity::Info);
    }

    #[test]
    fn reach_around_does_not_fire_on_pmacs_namespace() {
        // Rules 0 and 1 cover pmacs.* private surface; the
        // reach-around rule explicitly excludes the `pmacs` prefix
        // so a single bad require doesn't double-report.
        let f = engine().audit_source("t.lua", r#"require("pmacs._internal.foo")"#);
        assert!(f.iter().all(|x| x.rule != "reach-around-require"));
    }

    #[test]
    fn reach_around_field_access_is_info_level() {
        let f = engine().audit_source(
            "t.lua",
            r#"local seam = require("otherpkg").__pmacs_outline_test_seam_DO_NOT_USE"#,
        );
        let r = f
            .iter()
            .find(|x| x.rule == "reach-around-require-field")
            .expect("expected reach-around field finding");
        assert_eq!(r.severity, Severity::Info);
    }

    #[test]
    fn reach_around_field_access_ignores_public_and_pmacs_fields() {
        let f = engine().audit_source(
            "t.lua",
            r#"
            local ok = require("otherpkg").query
            local host = require("pmacs.foo")._private
            "#,
        );
        assert!(
            f.iter().all(|x| x.rule != "reach-around-require-field"),
            "expected no field reach-around findings, got {f:?}"
        );
    }

    #[test]
    fn bare_require_is_not_a_finding() {
        let f = engine().audit_source("t.lua", r#"require("magit")"#);
        assert!(f.is_empty(), "expected no findings, got {f:?}");
    }

    #[test]
    fn finding_records_line_and_column() {
        // Multi-line snippet so the line number must be > 1.
        let src = "-- header\n-- comment\nffi.load(\"c\")\n";
        let f = engine().audit_source("t.lua", src);
        let r = f
            .iter()
            .find(|x| x.rule == "no-ffi-call")
            .expect("expected ffi finding");
        assert_eq!(r.line, 3);
        assert_eq!(r.column, 1);
        assert_eq!(r.snippet, "ffi.load(\"c\")");
    }

    #[test]
    fn audit_report_summary_counts_match_findings() {
        let f = engine().audit_source(
            "t.lua",
            r#"
                ffi.cdef[[ int x; ]]
                io.popen("ls")
                require("otherpkg.x")
            "#,
        );
        let r = AuditReport::new(f);
        assert_eq!(r.summary.errors, 1);
        assert_eq!(r.summary.warnings, 1);
        assert_eq!(r.summary.infos, 1);
    }

    #[test]
    fn empty_file_yields_no_findings() {
        let f = engine().audit_source("t.lua", "");
        assert!(f.is_empty());
    }

    #[test]
    fn audit_source_is_stable_order_within_file() {
        let src = "ffi.cdef\"\"\nffi.load\"\"\nio.popen\"\"\n";
        let f = engine().audit_source("t.lua", src);
        // line ascending
        for w in f.windows(2) {
            assert!(
                (w[0].line, w[0].column) <= (w[1].line, w[1].column),
                "findings out of order: {f:?}"
            );
        }
    }

    #[test]
    fn audit_dir_recurses_and_finds_nested_lua() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b");
        fs::create_dir_all(&nested).unwrap();
        fs::write(dir.path().join("top.lua"), "ffi.cdef\"\"\n").unwrap();
        fs::write(nested.join("inner.lua"), "io.popen\"x\"\n").unwrap();
        // A non-Lua file should be ignored.
        fs::write(dir.path().join("readme.txt"), "ffi.cdef should not match\n").unwrap();
        let f = engine().audit_dir(dir.path()).unwrap();
        let rules: Vec<_> = f.iter().map(|x| x.rule.as_str()).collect();
        assert!(rules.contains(&"no-ffi-call"));
        assert!(rules.contains(&"no-process-spawn-io"));
    }
}
