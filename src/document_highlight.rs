// document_highlight.rs --- T M4.5: textDocument/documentHighlight.

//! Per-`(server, uri)` store for `textDocument/documentHighlight`:
//! the ranges in the current document that refer to the same symbol
//! as the cursor (the basis for "highlight all occurrences"). Same
//! request/store/Lua shape as the other M4.5 features; the inbound
//! position codec rewrites the ranges to byte offsets before this
//! parses, so consumers stay byte-uniform.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// One highlighted occurrence. `kind` is the raw LSP
/// `DocumentHighlightKind` (1 = Text, 2 = Read, 3 = Write); absent
/// defaults to 1 (Text) per spec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Highlight {
    /// Zero-based start line.
    pub start_line: u32,
    /// Zero-based start column.
    pub start_col: u32,
    /// Zero-based end line.
    pub end_line: u32,
    /// Zero-based end column.
    pub end_col: u32,
    /// `DocumentHighlightKind` (1 Text / 2 Read / 3 Write).
    pub kind: i64,
}

/// Parsed response: zero or more occurrences in source order.
#[derive(Clone, Debug, Default)]
pub struct DocumentHighlightResponse {
    /// Occurrences the server returned.
    pub highlights: Vec<Highlight>,
}

impl DocumentHighlightResponse {
    /// Parse `DocumentHighlight[] | null`.
    #[must_use]
    pub fn from_lsp_value(v: &Value) -> Self {
        let mut out = Vec::new();
        if let Some(arr) = v.as_array() {
            for item in arr {
                let Some(range) = item.get("range") else {
                    continue;
                };
                let (Some(start), Some(end)) = (range.get("start"), range.get("end")) else {
                    continue;
                };
                let g = |p: &Value, k: &str| p.get(k).and_then(Value::as_u64).map(|n| n as u32);
                let (Some(sl), Some(sc), Some(el), Some(ec)) = (
                    g(start, "line"),
                    g(start, "character"),
                    g(end, "line"),
                    g(end, "character"),
                ) else {
                    continue;
                };
                out.push(Highlight {
                    start_line: sl,
                    start_col: sc,
                    end_line: el,
                    end_col: ec,
                    kind: item.get("kind").and_then(Value::as_i64).unwrap_or(1),
                });
            }
        }
        Self { highlights: out }
    }

    /// True iff the server returned nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.highlights.is_empty()
    }
}

/// Key into [`DocumentHighlightStore`].
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct DocumentHighlightKey {
    /// Decimal LSP server id.
    pub server: String,
    /// Document URI.
    pub uri: String,
}

impl DocumentHighlightKey {
    /// Construct a key.
    #[must_use]
    pub fn new(server: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            uri: uri.into(),
        }
    }
}

/// Per-`(server, uri)` highlight state.
#[derive(Default)]
pub struct DocumentHighlightStore {
    by_key: HashMap<DocumentHighlightKey, DocumentHighlightResponse>,
}

impl DocumentHighlightStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the response at `key`.
    pub fn set(&mut self, key: DocumentHighlightKey, response: DocumentHighlightResponse) {
        self.by_key.insert(key, response);
    }

    /// Drop the entry at `key`.
    pub fn clear(&mut self, key: &DocumentHighlightKey) {
        self.by_key.remove(key);
    }

    /// Look up the entry at `key`.
    #[must_use]
    pub fn get(&self, key: &DocumentHighlightKey) -> Option<&DocumentHighlightResponse> {
        self.by_key.get(key)
    }
}

/// Cheaply-cloneable shared handle.
pub type SharedDocumentHighlightStore = Arc<Mutex<DocumentHighlightStore>>;

/// Build a fresh shared store.
#[must_use]
pub fn make_shared_store() -> SharedDocumentHighlightStore {
    Arc::new(Mutex::new(DocumentHighlightStore::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_highlights_and_defaults_kind_to_text() {
        let v = json!([
            {
                "range": { "start": { "line": 1, "character": 2 }, "end": { "line": 1, "character": 5 } },
                "kind": 3
            },
            {
                "range": { "start": { "line": 4, "character": 0 }, "end": { "line": 4, "character": 3 } }
            }
        ]);
        let r = DocumentHighlightResponse::from_lsp_value(&v);
        assert_eq!(r.highlights.len(), 2);
        assert_eq!(r.highlights[0].kind, 3); // Write
        assert_eq!(
            (
                r.highlights[0].start_line,
                r.highlights[0].start_col,
                r.highlights[0].end_col
            ),
            (1, 2, 5)
        );
        assert_eq!(r.highlights[1].kind, 1); // absent ⇒ Text
    }

    #[test]
    fn null_is_empty_and_store_round_trips() {
        assert!(DocumentHighlightResponse::from_lsp_value(&Value::Null).is_empty());
        let mut s = DocumentHighlightStore::new();
        let key = DocumentHighlightKey::new("1", "file:///a");
        s.set(
            key.clone(),
            DocumentHighlightResponse {
                highlights: vec![Highlight {
                    start_line: 0,
                    start_col: 0,
                    end_line: 0,
                    end_col: 1,
                    kind: 2,
                }],
            },
        );
        assert_eq!(s.get(&key).unwrap().highlights.len(), 1);
        s.clear(&key);
        assert!(s.get(&key).is_none());
    }
}
