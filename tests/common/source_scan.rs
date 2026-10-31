// tests/common/source_scan.rs --- a source file read line by line, each
// line put in the function it sits in.

//! The scanner a source guard reads files with: a line holding a needle
//! that is not a comment and not a `use`, before the file's
//! `mod tests {`, and the innermost `fn <name>` line above it. Nothing
//! is parsed. Extracted at E10k (D37) from
//! `tests/daemon_write_consistency.rs`, E10d's guard over the daemon's
//! writes, for E10k.3's guard over the replicas' per-key reads.
//!
//! Included per file, the way `iso` is:
//!
//! ```ignore
//! #[path = "common/source_scan.rs"]
//! mod source_scan;
//! ```

#![allow(dead_code)] // not every including suite uses every helper

use std::path::PathBuf;

pub fn source(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The name a line defines, if it is a `fn` line: `fn name(`,
/// `pub fn name<`, `pub(crate) fn name(`, at any indentation.
pub fn defined_fn(line: &str) -> Option<&str> {
    let mut rest = line.trim_start();
    if let Some(after) = rest.strip_prefix("pub") {
        rest = after
            .trim_start_matches(|c: char| c != ' ')
            .strip_prefix(' ')?;
    }
    let rest = rest.strip_prefix("fn ")?;
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    (end > 0).then(|| &rest[..end])
}

/// Whether `code` holds `needle` as written, not as the start of a
/// longer name (`.stream` is not `.streams`).
pub fn holds(code: &str, needle: &str) -> bool {
    code.match_indices(needle).any(|(at, _)| {
        needle.ends_with('(')
            || !code[at + needle.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
    })
}

/// Every line of `source` before its test module holding `needle`, as
/// (function, 1-based line).
pub fn found(source: &str, needle: &str) -> Vec<(String, usize)> {
    let mut current: Option<String> = None;
    let mut out = Vec::new();
    for (i, line) in source.lines().enumerate() {
        if line == "mod tests {" {
            break;
        }
        if let Some(name) = defined_fn(line) {
            current = Some(name.to_owned());
        }
        let code = line.trim_start();
        if code.starts_with("//") || code.starts_with("use ") {
            continue;
        }
        if holds(code, needle) {
            out.push((
                current.clone().unwrap_or_else(|| "<no fn>".to_owned()),
                i + 1,
            ));
        }
    }
    out
}
