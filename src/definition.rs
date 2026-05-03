// definition.rs --- T M4.12 LSP-backed go-to-definition.

//! `textDocument/definition` response state.
//!
//! Mirrors [`crate::hover`] / [`crate::completion`]: a tiny shared store
//! keyed by `(server, uri)` holding the parsed response. Lua surfaces it
//! as `pmacs.lsp.definition.locations`. Unlike the hover store, the
//! response is a list of locations (LSP returns `Location | Location[] |
//! LocationLink[] | null`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// One target of a go-to-definition lookup. Coordinates are LSP-native:
/// zero-based line, UTF-16-code-unit column, `file://`-style URI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DefinitionLocation {
    /// Target document URI.
    pub uri: String,
    /// Zero-based line.
    pub line: u32,
    /// Zero-based column (UTF-16 code units, per LSP).
    pub col: u32,
}

/// Parsed definition response: zero or more locations. An empty list is
/// distinct from "no response yet" (the response simply found nothing).
#[derive(Clone, Debug, Default)]
pub struct DefinitionResponse {
    /// Targets the server returned, in source order.
    pub locations: Vec<DefinitionLocation>,
}

impl DefinitionResponse {
    /// Parse `Location | Location[] | LocationLink[] | null` into a
    /// flat location list.
    #[must_use]
    pub fn from_lsp_value(v: &Value) -> Self {
        if v.is_null() {
            return Self::default();
        }
        if let Some(arr) = v.as_array() {
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                if let Some(loc) = parse_location(item).or_else(|| parse_location_link(item)) {
                    out.push(loc);
                }
            }
            return Self { locations: out };
        }
        if let Some(loc) = parse_location(v) {
            return Self {
                locations: vec![loc],
            };
        }
        Self::default()
    }

    /// True iff the server returned no targets.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.locations.is_empty()
    }
}

fn parse_location(v: &Value) -> Option<DefinitionLocation> {
    let uri = v.get("uri")?.as_str()?.to_owned();
    let range = v.get("range")?;
    let start = range.get("start")?;
    Some(DefinitionLocation {
        uri,
        line: start.get("line")?.as_u64()? as u32,
        col: start.get("character")?.as_u64()? as u32,
    })
}

fn parse_location_link(v: &Value) -> Option<DefinitionLocation> {
    let uri = v.get("targetUri")?.as_str()?.to_owned();
    // Prefer `targetSelectionRange` (the highlighted name), fall back to
    // `targetRange` (the full body) — both are documented as required
    // by LSP, but real-world servers occasionally omit one.
    let range = v
        .get("targetSelectionRange")
        .or_else(|| v.get("targetRange"))?;
    let start = range.get("start")?;
    Some(DefinitionLocation {
        uri,
        line: start.get("line")?.as_u64()? as u32,
        col: start.get("character")?.as_u64()? as u32,
    })
}

/// Per-server, per-uri definition state.
#[derive(Default)]
pub struct DefinitionStore {
    by_key: HashMap<DefinitionKey, DefinitionResponse>,
}

/// Key into [`DefinitionStore`].
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct DefinitionKey {
    /// Decimal LSP server id.
    pub server: String,
    /// Document URI.
    pub uri: String,
}

impl DefinitionKey {
    /// Construct a key.
    #[must_use]
    pub fn new(server: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            uri: uri.into(),
        }
    }
}

impl DefinitionStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the response at `key`.
    pub fn set(&mut self, key: DefinitionKey, response: DefinitionResponse) {
        self.by_key.insert(key, response);
    }

    /// Drop the entry at `key`.
    pub fn clear(&mut self, key: &DefinitionKey) {
        self.by_key.remove(key);
    }

    /// Look up the entry at `key`.
    #[must_use]
    pub fn get(&self, key: &DefinitionKey) -> Option<&DefinitionResponse> {
        self.by_key.get(key)
    }
}

/// Cheaply-cloneable shared handle.
pub type SharedDefinitionStore = Arc<Mutex<DefinitionStore>>;

/// Build a fresh shared store.
#[must_use]
pub fn make_shared_store() -> SharedDefinitionStore {
    Arc::new(Mutex::new(DefinitionStore::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_single_location() {
        let v = json!({
            "uri": "file:///x.rs",
            "range": {
                "start": { "line": 10, "character": 4 },
                "end":   { "line": 10, "character": 9 }
            }
        });
        let r = DefinitionResponse::from_lsp_value(&v);
        assert_eq!(r.locations.len(), 1);
        assert_eq!(r.locations[0].uri, "file:///x.rs");
        assert_eq!(r.locations[0].line, 10);
        assert_eq!(r.locations[0].col, 4);
    }

    #[test]
    fn parses_location_array() {
        let v = json!([
            {
                "uri": "file:///a.rs",
                "range": {
                    "start": { "line": 1, "character": 0 },
                    "end":   { "line": 1, "character": 5 }
                }
            },
            {
                "uri": "file:///b.rs",
                "range": {
                    "start": { "line": 7, "character": 2 },
                    "end":   { "line": 7, "character": 9 }
                }
            }
        ]);
        let r = DefinitionResponse::from_lsp_value(&v);
        assert_eq!(r.locations.len(), 2);
        assert_eq!(r.locations[1].line, 7);
    }

    #[test]
    fn parses_location_link_array_prefers_selection_range() {
        let v = json!([
            {
                "targetUri": "file:///x.rs",
                "targetRange": {
                    "start": { "line": 5, "character": 0 },
                    "end":   { "line": 9, "character": 1 }
                },
                "targetSelectionRange": {
                    "start": { "line": 5, "character": 7 },
                    "end":   { "line": 5, "character": 12 }
                }
            }
        ]);
        let r = DefinitionResponse::from_lsp_value(&v);
        assert_eq!(r.locations.len(), 1);
        assert_eq!(r.locations[0].line, 5);
        assert_eq!(r.locations[0].col, 7);
    }

    #[test]
    fn null_response_is_empty() {
        let r = DefinitionResponse::from_lsp_value(&Value::Null);
        assert!(r.is_empty());
    }

    #[test]
    fn store_set_get_clear() {
        let mut s = DefinitionStore::new();
        let key = DefinitionKey::new("1", "file:///a");
        s.set(
            key.clone(),
            DefinitionResponse {
                locations: vec![DefinitionLocation {
                    uri: "file:///b".into(),
                    line: 1,
                    col: 2,
                }],
            },
        );
        assert_eq!(s.get(&key).unwrap().locations.len(), 1);
        s.clear(&key);
        assert!(s.get(&key).is_none());
    }
}
