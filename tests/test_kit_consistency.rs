//! The one thing the shared test kit is held to: a top-level `exec` or
//! `eval` lives in `tests/common/editor.rs`, and a suite that wants one
//! includes that module rather than declaring its own.
//!
//! E8e.1 put the body 67 suites carried byte for byte into
//! `tests/common/editor.rs`, and E8e.2 replaced those 134 definitions
//! with the include. The definitions that differ --- `to_owned` for
//! `to_string`, an `expect` naming the failing chunk, an `eval` fixed to
//! `String`, a panic quoting the source --- stay where they are until a
//! later batch has compared each class, and `ALLOWED` lists them, one
//! entry per file and name. A definition outside `common` that is not
//! on the list fails this test by file, line and name. An entry whose
//! file no longer defines the name fails too, so the list shrinks with
//! the batches that drain it instead of outliving them.
//!
//! One shape is read and nothing is counted: a line at column 0 that is
//! `fn exec` or `fn eval`, with or without a leading `pub`, followed by
//! `(` or `<`. `exec_err`, `eval_sync`, `try_exec` and a definition
//! indented inside a nested module are not that shape; the audit's
//! census (2026-10-13) was taken at column 0 and this is its rule.

use std::path::{Path, PathBuf};

/// Every top-level `exec` and `eval` still defined outside `common`, by
/// file under `tests/` and name, as of E8e.2. Remove an entry when its
/// definition goes; add one only for a definition whose difference from
/// the shared body carries meaning the batch that meets it has named.
const ALLOWED: &[(&str, &str)] = &[
    ("bottom_panel_stage2b_daemon_acceptance.rs", "exec"),
    ("buffer_list_acceptance.rs", "exec"),
    ("buffer_list_acceptance.rs", "eval"),
    ("diagnostics_panel_acceptance.rs", "exec"),
    ("diagnostics_panel_acceptance.rs", "eval"),
    ("discovery_acceptance.rs", "exec"),
    ("discovery_acceptance.rs", "eval"),
    ("e7g_review1_probes.rs", "exec"),
    ("e7g_review1_probes.rs", "eval"),
    ("e7g_review1_serialization_witnesses.rs", "exec"),
    ("e7g_review1_serialization_witnesses.rs", "eval"),
    ("e7h_grammar_gate_acceptance.rs", "exec"),
    ("e7h_grammar_gate_acceptance.rs", "eval"),
    ("e7i_review2_probes.rs", "exec"),
    ("e7i_review2_probes.rs", "eval"),
    ("e7i_review3_probes.rs", "exec"),
    ("e7i_review3_probes.rs", "eval"),
    ("e8_grid_popup_acceptance.rs", "exec"),
    ("e8_review1_real_server_probes.rs", "exec"),
    ("e8_review1_real_server_probes.rs", "eval"),
    ("e8_review2_real_server_probes.rs", "exec"),
    ("e8_review2_real_server_probes.rs", "eval"),
    ("errors_channel_acceptance.rs", "exec"),
    ("errors_channel_acceptance.rs", "eval"),
    ("hook_tokens_acceptance.rs", "exec"),
    ("hook_tokens_acceptance.rs", "eval"),
    ("lean4_server_acceptance.rs", "exec"),
    ("lean4_server_acceptance.rs", "eval"),
    ("lsp_crash_acceptance.rs", "exec"),
    ("lsp_crash_acceptance.rs", "eval"),
    ("lsp_dispatch_seams_acceptance.rs", "exec"),
    ("lsp_dispatch_seams_acceptance.rs", "eval"),
    ("lsp_latex_acceptance.rs", "exec"),
    ("lsp_latex_acceptance.rs", "eval"),
    ("lsp_multi_root_acceptance.rs", "exec"),
    ("lsp_multi_root_acceptance.rs", "eval"),
    ("lsp_spawn_guidance_acceptance.rs", "exec"),
    ("lsp_spawn_guidance_acceptance.rs", "eval"),
    ("resource_reconciliation_acceptance.rs", "exec"),
    ("resource_reconciliation_acceptance.rs", "eval"),
    ("statusline_segments_acceptance.rs", "exec"),
    ("statusline_segments_acceptance.rs", "eval"),
    ("terminal_config_acceptance.rs", "exec"),
    ("terminal_copy_mode_acceptance.rs", "exec"),
    ("terminal_copy_mode_acceptance.rs", "eval"),
    ("worker_identity_acceptance.rs", "exec"),
    ("worker_identity_acceptance.rs", "eval"),
];

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// The name a line defines at column 0, if it is `exec` or `eval`:
/// `fn exec(`, `fn eval<`, `pub fn exec(`, `pub(crate) fn eval<`.
fn top_level_definition(line: &str) -> Option<&'static str> {
    let rest = match line.strip_prefix("pub") {
        Some(after_pub) => after_pub
            .trim_start_matches(|c: char| c != ' ')
            .strip_prefix(' ')?,
        None => line,
    };
    let rest = rest.strip_prefix("fn ")?;
    ["exec", "eval"].into_iter().find(|name| {
        rest.strip_prefix(name)
            .is_some_and(|after| after.starts_with('(') || after.starts_with('<'))
    })
}

/// Every `.rs` file under `tests/` except those under `tests/common/`,
/// as a path relative to `tests/`, sorted.
fn files_outside_common(dir: &Path, prefix: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        let rel = prefix.join(entry.file_name());
        if path.is_dir() {
            if rel != Path::new("common") {
                files_outside_common(&path, &rel, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(rel);
        }
    }
    out.sort();
}

#[test]
fn a_top_level_exec_or_eval_outside_common_is_on_the_allowlist() {
    let mut files = Vec::new();
    files_outside_common(&tests_dir(), Path::new(""), &mut files);

    let mut found: Vec<(String, &'static str, usize)> = Vec::new();
    for rel in &files {
        let path = tests_dir().join(rel);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for (i, line) in text.lines().enumerate() {
            if let Some(name) = top_level_definition(line) {
                found.push((rel.to_string_lossy().into_owned(), name, i + 1));
            }
        }
    }

    let mut problems = Vec::new();
    for (file, name, line) in &found {
        if !ALLOWED.contains(&(file.as_str(), name)) {
            problems.push(format!(
                "tests/{file}:{line} defines a top-level `{name}` outside `common` and is not \
                 on the allowlist: include `common/editor.rs` instead \
                 (`#[path = \"common/editor.rs\"] mod editor; use editor::{name};`)"
            ));
        }
    }
    for (file, name) in ALLOWED {
        if !found.iter().any(|(f, n, _)| f == file && n == name) {
            problems.push(format!(
                "the allowlist names a top-level `{name}` in tests/{file} that the file no \
                 longer defines; remove the entry"
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
