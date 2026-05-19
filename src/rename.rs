// rename.rs --- T M4.5 L2 LSP-backed rename / WorkspaceEdit state.

//! `textDocument/rename` response state.
//!
//! A rename answer is an LSP [`WorkspaceEdit`], which may touch many
//! files. This module parses both edit carriers —
//!
//!   * `changes`: `{ uri: TextEdit[] }`
//!   * `documentChanges`: `(TextDocumentEdit | resource-op)[]`
//!
//! — into a flat, per-file edit list ([`WorkspaceEditResponse`]). The
//! [`crate::formatting::TextEdit`] shape is reused verbatim (same
//! zero-based, UTF-16-column coordinates). Resource operations
//! (`create` / `rename` / `delete` file) are L4 work; they are skipped
//! here and counted in [`WorkspaceEditResponse::unsupported_ops`] so
//! the Lua surface can warn rather than silently drop a partial rename.
//!
//! Like [`crate::formatting`], there is no Rust-side editor mutation:
//! Lua reads the per-file lists and drives `pmacs.buffer.*` /
//! `pmacs.editor.*` so the application strategy stays configurable.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::formatting::TextEdit;

/// Edits the server wants applied to one document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEdits {
    /// Target document URI.
    pub uri: String,
    /// Edits for this document, in server order. Callers apply in
    /// reverse-start order so earlier offsets stay valid.
    pub edits: Vec<TextEdit>,
}

/// A parsed `WorkspaceEdit`: per-file edit lists plus a count of
/// resource operations we deliberately did not apply (L4).
#[derive(Clone, Debug, Default)]
pub struct WorkspaceEditResponse {
    /// One entry per touched document.
    pub files: Vec<FileEdits>,
    /// Number of `create` / `rename` / `delete` file operations the
    /// server requested that this layer does not yet apply.
    pub unsupported_ops: usize,
}

impl WorkspaceEditResponse {
    /// Parse `WorkspaceEdit | null`.
    ///
    /// Per the LSP spec `documentChanges` supersedes `changes` when
    /// both are present, so it is preferred. A `null` / shapeless
    /// result yields an empty response (rename produced nothing).
    #[must_use]
    pub fn from_lsp_value(v: &Value) -> Self {
        if let Some(dc) = v.get("documentChanges").and_then(Value::as_array) {
            return Self::from_document_changes(dc);
        }
        if let Some(changes) = v.get("changes").and_then(Value::as_object) {
            let mut files = Vec::with_capacity(changes.len());
            for (uri, edits) in changes {
                files.push(FileEdits {
                    uri: uri.clone(),
                    edits: parse_edit_array(edits),
                });
            }
            // Object iteration order is unspecified; sort by URI so the
            // applier (and tests) see a deterministic sequence.
            files.sort_by(|a, b| a.uri.cmp(&b.uri));
            return Self {
                files,
                unsupported_ops: 0,
            };
        }
        Self::default()
    }

    fn from_document_changes(dc: &[Value]) -> Self {
        let mut files = Vec::new();
        let mut unsupported_ops = 0;
        for entry in dc {
            // A resource operation is tagged with `kind`; a
            // TextDocumentEdit has a `textDocument` + `edits`.
            if entry.get("kind").and_then(Value::as_str).is_some() {
                unsupported_ops += 1;
                continue;
            }
            let Some(uri) = entry
                .get("textDocument")
                .and_then(|t| t.get("uri"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let edits = entry.get("edits").map(parse_edit_array).unwrap_or_default();
            files.push(FileEdits {
                uri: uri.to_owned(),
                edits,
            });
        }
        Self {
            files,
            unsupported_ops,
        }
    }

    /// True iff there is nothing to apply and nothing was skipped.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.iter().all(|f| f.edits.is_empty()) && self.unsupported_ops == 0
    }

    /// Total edits across every file.
    #[must_use]
    pub fn edit_count(&self) -> usize {
        self.files.iter().map(|f| f.edits.len()).sum()
    }
}

/// Parse a `(TextEdit | AnnotatedTextEdit)[]` value into edits,
/// dropping malformed entries. `AnnotatedTextEdit` is a `TextEdit`
/// with an extra `annotationId`; the range/newText parse identically.
fn parse_edit_array(v: &Value) -> Vec<TextEdit> {
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        if let Some(e) = parse_text_edit(item) {
            out.push(e);
        }
    }
    out
}

fn parse_text_edit(v: &Value) -> Option<TextEdit> {
    let range = v.get("range")?;
    let start = range.get("start")?;
    let end = range.get("end")?;
    let new_text = v.get("newText")?.as_str()?.to_owned();
    Some(TextEdit {
        start_line: start.get("line")?.as_u64()? as u32,
        start_col: start.get("character")?.as_u64()? as u32,
        end_line: end.get("line")?.as_u64()? as u32,
        end_col: end.get("character")?.as_u64()? as u32,
        new_text,
    })
}

/// Per-server, per-origin-uri rename response state. The key URI is
/// the document the rename was *requested* on (the symbol's home
/// file), not the files the edit happens to touch.
#[derive(Default)]
pub struct RenameStore {
    by_key: HashMap<RenameKey, WorkspaceEditResponse>,
}

/// Key into [`RenameStore`].
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct RenameKey {
    /// Decimal LSP server id.
    pub server: String,
    /// The URI the rename was requested on.
    pub uri: String,
}

impl RenameKey {
    /// Construct a key.
    #[must_use]
    pub fn new(server: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            uri: uri.into(),
        }
    }
}

impl RenameStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the response at `key`.
    pub fn set(&mut self, key: RenameKey, response: WorkspaceEditResponse) {
        self.by_key.insert(key, response);
    }

    /// Drop the entry at `key`.
    pub fn clear(&mut self, key: &RenameKey) {
        self.by_key.remove(key);
    }

    /// Look up the entry at `key`.
    #[must_use]
    pub fn get(&self, key: &RenameKey) -> Option<&WorkspaceEditResponse> {
        self.by_key.get(key)
    }
}

/// Cheaply-cloneable shared handle.
pub type SharedRenameStore = Arc<Mutex<RenameStore>>;

/// Build a fresh shared store.
#[must_use]
pub fn make_shared_store() -> SharedRenameStore {
    Arc::new(Mutex::new(RenameStore::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn one_edit(new_text: &str) -> Value {
        json!({
            "range": {
                "start": { "line": 0, "character": 3 },
                "end":   { "line": 0, "character": 6 }
            },
            "newText": new_text
        })
    }

    #[test]
    fn parses_changes_map_sorted_by_uri() {
        let v = json!({
            "changes": {
                "file:///b.rs": [one_edit("Y")],
                "file:///a.rs": [one_edit("X"), one_edit("Z")]
            }
        });
        let r = WorkspaceEditResponse::from_lsp_value(&v);
        assert_eq!(r.files.len(), 2);
        assert_eq!(r.files[0].uri, "file:///a.rs");
        assert_eq!(r.files[0].edits.len(), 2);
        assert_eq!(r.files[1].uri, "file:///b.rs");
        assert_eq!(r.edit_count(), 3);
        assert_eq!(r.unsupported_ops, 0);
    }

    #[test]
    fn parses_document_changes_and_prefers_it_over_changes() {
        let v = json!({
            "changes": { "file:///ignored.rs": [one_edit("NO")] },
            "documentChanges": [
                {
                    "textDocument": { "uri": "file:///a.rs", "version": 1 },
                    "edits": [one_edit("A")]
                }
            ]
        });
        let r = WorkspaceEditResponse::from_lsp_value(&v);
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.files[0].uri, "file:///a.rs");
        assert_eq!(r.files[0].edits[0].new_text, "A");
    }

    #[test]
    fn resource_ops_are_counted_not_applied() {
        let v = json!({
            "documentChanges": [
                { "kind": "create", "uri": "file:///new.rs" },
                {
                    "textDocument": { "uri": "file:///a.rs", "version": 2 },
                    "edits": [one_edit("A")]
                },
                { "kind": "rename", "oldUri": "file:///a.rs", "newUri": "file:///c.rs" }
            ]
        });
        let r = WorkspaceEditResponse::from_lsp_value(&v);
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.unsupported_ops, 2);
        assert!(!r.is_empty());
    }

    #[test]
    fn null_response_is_empty() {
        let r = WorkspaceEditResponse::from_lsp_value(&Value::Null);
        assert!(r.is_empty());
        assert_eq!(r.edit_count(), 0);
    }

    #[test]
    fn annotated_text_edit_parses_like_plain() {
        let v = json!({
            "documentChanges": [{
                "textDocument": { "uri": "file:///a.rs", "version": 1 },
                "edits": [{
                    "range": {
                        "start": { "line": 2, "character": 0 },
                        "end":   { "line": 2, "character": 4 }
                    },
                    "newText": "Q",
                    "annotationId": "ann1"
                }]
            }]
        });
        let r = WorkspaceEditResponse::from_lsp_value(&v);
        assert_eq!(r.files[0].edits[0].new_text, "Q");
        assert_eq!(r.files[0].edits[0].start_line, 2);
    }

    #[test]
    fn store_set_get_clear() {
        let mut s = RenameStore::new();
        let key = RenameKey::new("1", "file:///a");
        s.set(
            key.clone(),
            WorkspaceEditResponse {
                files: vec![FileEdits {
                    uri: "file:///a".into(),
                    edits: vec![TextEdit {
                        start_line: 0,
                        start_col: 0,
                        end_line: 0,
                        end_col: 1,
                        new_text: "x".into(),
                    }],
                }],
                unsupported_ops: 0,
            },
        );
        assert_eq!(s.get(&key).unwrap().files.len(), 1);
        s.clear(&key);
        assert!(s.get(&key).is_none());
    }
}
