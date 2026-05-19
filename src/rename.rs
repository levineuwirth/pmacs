// rename.rs --- T M4.5 LSP-backed rename / WorkspaceEdit state.

//! `textDocument/rename` (and any other) `WorkspaceEdit` state.
//!
//! A `WorkspaceEdit` may touch many files and, via `documentChanges`,
//! interleave text edits with filesystem *resource operations*
//! (create / rename / delete file). This module parses both carriers —
//!
//!   * `changes`: `{ uri: TextEdit[] }`
//!   * `documentChanges`: `(TextDocumentEdit | CreateFile |
//!     RenameFile | DeleteFile)[]`
//!
//! — into a single **ordered** [`WorkspaceOp`] list
//! ([`WorkspaceEditResponse::ops`]). Order is preserved exactly as the
//! server sent it, because the spec requires sequential application
//! (e.g. a `CreateFile` must precede the `TextDocumentEdit` that fills
//! the new file). The `changes` map, which carries no resource ops and
//! no inherent order, is emitted as URI-sorted edit ops for
//! determinism.
//!
//! The [`crate::formatting::TextEdit`] shape is reused verbatim (same
//! zero-based, UTF-16-column coordinates). As with
//! [`crate::formatting`], nothing here mutates the editor or the disk:
//! Lua reads the ordered ops and drives `pmacs.buffer.*` /
//! `pmacs.editor.*` (text edits) and `pmacs.buffer.apply_resource_op`
//! (filesystem ops) so the application strategy stays configurable.

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

/// One entry of a `WorkspaceEdit`, in server-sent order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceOp {
    /// Text edits for a single document.
    Edit(FileEdits),
    /// `CreateFile`. `overwrite` wins over `ignore_if_exists`.
    Create {
        /// URI to create.
        uri: String,
        /// Truncate if it already exists.
        overwrite: bool,
        /// No-op if it already exists (loses to `overwrite`).
        ignore_if_exists: bool,
    },
    /// `RenameFile`.
    Rename {
        /// Existing URI.
        old_uri: String,
        /// Destination URI.
        new_uri: String,
        /// Overwrite the destination if it exists.
        overwrite: bool,
        /// No-op if the destination exists (loses to `overwrite`).
        ignore_if_exists: bool,
    },
    /// `DeleteFile`.
    Delete {
        /// URI to delete.
        uri: String,
        /// Recurse into a directory.
        recursive: bool,
        /// Not an error if it is already gone.
        ignore_if_not_exists: bool,
    },
}

/// A parsed `WorkspaceEdit`: an ordered list of operations.
#[derive(Clone, Debug, Default)]
pub struct WorkspaceEditResponse {
    /// Operations in server-sent order.
    pub ops: Vec<WorkspaceOp>,
}

impl WorkspaceEditResponse {
    /// Parse `WorkspaceEdit | null`.
    ///
    /// Per the LSP spec `documentChanges` supersedes `changes` when
    /// both are present, so it is preferred. A `null` / shapeless
    /// result yields an empty response.
    #[must_use]
    pub fn from_lsp_value(v: &Value) -> Self {
        if let Some(dc) = v.get("documentChanges").and_then(Value::as_array) {
            return Self {
                ops: dc.iter().filter_map(parse_document_change).collect(),
            };
        }
        if let Some(changes) = v.get("changes").and_then(Value::as_object) {
            let mut edits: Vec<FileEdits> = changes
                .iter()
                .map(|(uri, e)| FileEdits {
                    uri: uri.clone(),
                    edits: parse_edit_array(e),
                })
                .collect();
            // `changes` has no inherent order; sort by URI so the
            // applier (and tests) see a deterministic sequence.
            edits.sort_by(|a, b| a.uri.cmp(&b.uri));
            return Self {
                ops: edits.into_iter().map(WorkspaceOp::Edit).collect(),
            };
        }
        Self::default()
    }

    /// True iff there is nothing to do — no ops, or only empty text
    /// edits. Any resource op makes this `false` (the edit is
    /// meaningful even with zero text changes).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ops
            .iter()
            .all(|op| matches!(op, WorkspaceOp::Edit(f) if f.edits.is_empty()))
    }

    /// Total text edits across every edit op.
    #[must_use]
    pub fn edit_count(&self) -> usize {
        self.ops
            .iter()
            .map(|op| match op {
                WorkspaceOp::Edit(f) => f.edits.len(),
                _ => 0,
            })
            .sum()
    }

    /// Number of filesystem resource ops (create/rename/delete).
    #[must_use]
    pub fn resource_op_count(&self) -> usize {
        self.ops
            .iter()
            .filter(|op| !matches!(op, WorkspaceOp::Edit(_)))
            .count()
    }

    /// The edit ops only, in order — the back-compat view for callers
    /// that just want per-file text edits.
    #[must_use]
    pub fn files(&self) -> Vec<&FileEdits> {
        self.ops
            .iter()
            .filter_map(|op| match op {
                WorkspaceOp::Edit(f) => Some(f),
                _ => None,
            })
            .collect()
    }
}

fn parse_document_change(entry: &Value) -> Option<WorkspaceOp> {
    // A resource op is tagged with a string `kind`; a
    // TextDocumentEdit has `textDocument` + `edits` and no `kind`.
    match entry.get("kind").and_then(Value::as_str) {
        Some("create") => Some(WorkspaceOp::Create {
            uri: entry.get("uri")?.as_str()?.to_owned(),
            overwrite: opt_bool(entry, "overwrite"),
            ignore_if_exists: opt_bool(entry, "ignoreIfExists"),
        }),
        Some("rename") => Some(WorkspaceOp::Rename {
            old_uri: entry.get("oldUri")?.as_str()?.to_owned(),
            new_uri: entry.get("newUri")?.as_str()?.to_owned(),
            overwrite: opt_bool(entry, "overwrite"),
            ignore_if_exists: opt_bool(entry, "ignoreIfExists"),
        }),
        Some("delete") => Some(WorkspaceOp::Delete {
            uri: entry.get("uri")?.as_str()?.to_owned(),
            recursive: opt_bool(entry, "recursive"),
            ignore_if_not_exists: opt_bool(entry, "ignoreIfNotExists"),
        }),
        // Unknown future resource kind — skip rather than misapply.
        Some(_) => None,
        None => {
            let uri = entry.get("textDocument")?.get("uri")?.as_str()?.to_owned();
            let edits = entry.get("edits").map(parse_edit_array).unwrap_or_default();
            Some(WorkspaceOp::Edit(FileEdits { uri, edits }))
        }
    }
}

fn opt_bool(entry: &Value, key: &str) -> bool {
    entry
        .get("options")
        .and_then(|o| o.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
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
        let files = r.files();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].uri, "file:///a.rs");
        assert_eq!(files[0].edits.len(), 2);
        assert_eq!(files[1].uri, "file:///b.rs");
        assert_eq!(r.edit_count(), 3);
        assert_eq!(r.resource_op_count(), 0);
    }

    #[test]
    fn document_changes_preserves_order_with_resource_ops() {
        let v = json!({
            "changes": { "file:///ignored.rs": [one_edit("NO")] },
            "documentChanges": [
                { "kind": "create", "uri": "file:///new.rs",
                  "options": { "ignoreIfExists": true } },
                { "textDocument": { "uri": "file:///new.rs", "version": 1 },
                  "edits": [one_edit("A")] },
                { "kind": "rename", "oldUri": "file:///a.rs",
                  "newUri": "file:///c.rs", "options": { "overwrite": true } },
                { "kind": "delete", "uri": "file:///d.rs",
                  "options": { "recursive": true, "ignoreIfNotExists": true } }
            ]
        });
        let r = WorkspaceEditResponse::from_lsp_value(&v);
        assert_eq!(r.ops.len(), 4);
        assert_eq!(r.resource_op_count(), 3);
        assert_eq!(r.edit_count(), 1);
        match &r.ops[0] {
            WorkspaceOp::Create {
                uri,
                overwrite,
                ignore_if_exists,
            } => {
                assert_eq!(uri, "file:///new.rs");
                assert!(!overwrite);
                assert!(ignore_if_exists);
            }
            other => panic!("expected Create, got {other:?}"),
        }
        match &r.ops[1] {
            WorkspaceOp::Edit(f) => assert_eq!(f.uri, "file:///new.rs"),
            other => panic!("expected Edit, got {other:?}"),
        }
        match &r.ops[2] {
            WorkspaceOp::Rename {
                old_uri,
                new_uri,
                overwrite,
                ..
            } => {
                assert_eq!(old_uri, "file:///a.rs");
                assert_eq!(new_uri, "file:///c.rs");
                assert!(overwrite);
            }
            other => panic!("expected Rename, got {other:?}"),
        }
        match &r.ops[3] {
            WorkspaceOp::Delete {
                uri,
                recursive,
                ignore_if_not_exists,
            } => {
                assert_eq!(uri, "file:///d.rs");
                assert!(recursive);
                assert!(ignore_if_not_exists);
            }
            other => panic!("expected Delete, got {other:?}"),
        }
    }

    #[test]
    fn unknown_resource_kind_is_skipped() {
        let v = json!({
            "documentChanges": [
                { "kind": "teleport", "uri": "file:///x" },
                { "textDocument": { "uri": "file:///a.rs", "version": 1 },
                  "edits": [one_edit("A")] }
            ]
        });
        let r = WorkspaceEditResponse::from_lsp_value(&v);
        assert_eq!(r.ops.len(), 1);
        assert_eq!(r.edit_count(), 1);
    }

    #[test]
    fn null_response_is_empty() {
        let r = WorkspaceEditResponse::from_lsp_value(&Value::Null);
        assert!(r.is_empty());
        assert_eq!(r.edit_count(), 0);
    }

    #[test]
    fn resource_only_edit_is_not_empty() {
        let v = json!({ "documentChanges": [
            { "kind": "delete", "uri": "file:///gone.rs" }
        ]});
        let r = WorkspaceEditResponse::from_lsp_value(&v);
        assert!(!r.is_empty());
        assert_eq!(r.edit_count(), 0);
        assert_eq!(r.resource_op_count(), 1);
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
        let f = r.files();
        assert_eq!(f[0].edits[0].new_text, "Q");
        assert_eq!(f[0].edits[0].start_line, 2);
    }

    #[test]
    fn store_set_get_clear() {
        let mut s = RenameStore::new();
        let key = RenameKey::new("1", "file:///a");
        s.set(
            key.clone(),
            WorkspaceEditResponse {
                ops: vec![WorkspaceOp::Edit(FileEdits {
                    uri: "file:///a".into(),
                    edits: vec![TextEdit {
                        start_line: 0,
                        start_col: 0,
                        end_line: 0,
                        end_col: 1,
                        new_text: "x".into(),
                    }],
                })],
            },
        );
        assert_eq!(s.get(&key).unwrap().files().len(), 1);
        s.clear(&key);
        assert!(s.get(&key).is_none());
    }
}
