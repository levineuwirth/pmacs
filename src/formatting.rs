// formatting.rs --- T M4.12 LSP-backed document formatting.

//! `textDocument/formatting` response state.
//!
//! Mirrors [`crate::definition`]: a per-`(server, uri)` store of the
//! parsed LSP `TextEdit[]`. Lua reads the list and applies edits in
//! reverse order via `pmacs.editor.*` primitives — no Rust-side editor
//! integration here, so users can override the application strategy
//! (for example: format-on-save vs. on-demand) entirely from config.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// One LSP `TextEdit`. Coordinates are zero-based; columns are UTF-16
/// code units per LSP. Empty `new_text` denotes a pure deletion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEdit {
    /// Zero-based start line.
    pub start_line: u32,
    /// Zero-based start column (UTF-16 code units).
    pub start_col: u32,
    /// Zero-based end line.
    pub end_line: u32,
    /// Zero-based end column (UTF-16 code units).
    pub end_col: u32,
    /// Replacement text. Empty means "delete the range".
    pub new_text: String,
}

/// Parsed formatting response: zero or more text edits in source order.
/// Callers must apply edits in reverse order so earlier edit positions
/// remain valid after later edits are applied.
#[derive(Clone, Debug, Default)]
pub struct FormattingResponse {
    /// Edits the server returned, in source order.
    pub edits: Vec<TextEdit>,
}

impl FormattingResponse {
    /// Parse `TextEdit[] | null` into a flat edit list. A `null`
    /// response yields an empty list.
    #[must_use]
    pub fn from_lsp_value(v: &Value) -> Self {
        let Some(arr) = v.as_array() else {
            return Self::default();
        };
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            if let Some(edit) = parse_text_edit(item) {
                out.push(edit);
            }
        }
        Self { edits: out }
    }

    /// True iff the server returned no edits.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }
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

/// Per-server, per-uri formatting response state.
#[derive(Default)]
pub struct FormattingStore {
    by_key: HashMap<FormattingKey, FormattingResponse>,
}

/// Key into [`FormattingStore`].
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct FormattingKey {
    /// Decimal LSP server id.
    pub server: String,
    /// Document URI.
    pub uri: String,
}

impl FormattingKey {
    /// Construct a key.
    #[must_use]
    pub fn new(server: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            uri: uri.into(),
        }
    }
}

impl FormattingStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the response at `key`.
    pub fn set(&mut self, key: FormattingKey, response: FormattingResponse) {
        self.by_key.insert(key, response);
    }

    /// Drop the entry at `key`.
    pub fn clear(&mut self, key: &FormattingKey) {
        self.by_key.remove(key);
    }

    /// Look up the entry at `key`.
    #[must_use]
    pub fn get(&self, key: &FormattingKey) -> Option<&FormattingResponse> {
        self.by_key.get(key)
    }
}

/// Cheaply-cloneable shared handle.
pub type SharedFormattingStore = Arc<Mutex<FormattingStore>>;

/// Build a fresh shared store.
#[must_use]
pub fn make_shared_store() -> SharedFormattingStore {
    Arc::new(Mutex::new(FormattingStore::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_text_edit_array() {
        let v = json!([
            {
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end":   { "line": 0, "character": 4 }
                },
                "newText": "    "
            },
            {
                "range": {
                    "start": { "line": 3, "character": 7 },
                    "end":   { "line": 3, "character": 7 }
                },
                "newText": ";"
            }
        ]);
        let r = FormattingResponse::from_lsp_value(&v);
        assert_eq!(r.edits.len(), 2);
        assert_eq!(r.edits[0].new_text, "    ");
        assert_eq!(r.edits[1].start_col, 7);
        assert_eq!(r.edits[1].new_text, ";");
    }

    #[test]
    fn null_response_is_empty() {
        let r = FormattingResponse::from_lsp_value(&Value::Null);
        assert!(r.is_empty());
    }

    #[test]
    fn pure_deletion_parses() {
        let v = json!([{
            "range": {
                "start": { "line": 1, "character": 0 },
                "end":   { "line": 2, "character": 0 }
            },
            "newText": ""
        }]);
        let r = FormattingResponse::from_lsp_value(&v);
        assert_eq!(r.edits.len(), 1);
        assert!(r.edits[0].new_text.is_empty());
    }

    #[test]
    fn store_set_get_clear() {
        let mut s = FormattingStore::new();
        let key = FormattingKey::new("1", "file:///a");
        s.set(
            key.clone(),
            FormattingResponse {
                edits: vec![TextEdit {
                    start_line: 0,
                    start_col: 0,
                    end_line: 0,
                    end_col: 1,
                    new_text: "x".into(),
                }],
            },
        );
        assert_eq!(s.get(&key).unwrap().edits.len(), 1);
        s.clear(&key);
        assert!(s.get(&key).is_none());
    }
}
