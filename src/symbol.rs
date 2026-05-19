// symbol.rs --- T M4.5: documentSymbol / workspace symbol.

//! Shared store for `textDocument/documentSymbol` and
//! `workspace/symbol`. Both ultimately describe "a named program
//! entity at a location", so one flat [`Symbol`] type serves both;
//! the only differences are (a) the request response can arrive in
//! two LSP shapes — hierarchical `DocumentSymbol[]` or flat
//! `SymbolInformation[]` / `WorkspaceSymbol[]` — handled by
//! [`SymbolResponse::from_lsp_value`], and (b) the store is keyed by
//! a [`SymbolScope`] so a per-document outline and a workspace query
//! don't collide.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// One symbol, flattened. `kind` is the raw LSP `SymbolKind` integer
/// (1..=26); consumers map it to a label. Coordinates are LSP-native
/// (the inbound position codec rewrites them to byte offsets before
/// this parses, for the document-symbol case where they belong to
/// the requested doc; workspace-symbol locations are cross-file and
/// pass through unconverted).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    /// Symbol name.
    pub name: String,
    /// Raw LSP `SymbolKind` (1..=26).
    pub kind: i64,
    /// Containing document URI.
    pub uri: String,
    /// Zero-based line of the symbol's name.
    pub line: u32,
    /// Zero-based column of the symbol's name.
    pub col: u32,
    /// `containerName` (flat shapes) or the parent chain joined with
    /// `::` (hierarchical), if any.
    pub container: Option<String>,
    /// Nesting depth in a hierarchical `DocumentSymbol` tree; 0 for
    /// flat shapes.
    pub depth: u32,
}

/// Parsed symbol response: a flat, source-ordered list.
#[derive(Clone, Debug, Default)]
pub struct SymbolResponse {
    /// Symbols in document/source order, parents before children.
    pub symbols: Vec<Symbol>,
}

fn range_start(v: &Value, key: &str) -> Option<(u32, u32)> {
    let start = v.get(key)?.get("start")?;
    Some((
        start.get("line")?.as_u64()? as u32,
        start.get("character")?.as_u64()? as u32,
    ))
}

impl SymbolResponse {
    /// Parse `DocumentSymbol[] | SymbolInformation[] |
    /// WorkspaceSymbol[] | null`. `default_uri` is the requested
    /// document — used for `DocumentSymbol`, which carries no URI
    /// (its positions are relative to the requested document).
    #[must_use]
    pub fn from_lsp_value(v: &Value, default_uri: &str) -> Self {
        let mut out = Vec::new();
        if let Some(arr) = v.as_array() {
            for item in arr {
                if item.get("location").is_some() {
                    // SymbolInformation / WorkspaceSymbol (flat).
                    Self::push_flat(item, &mut out);
                } else {
                    // DocumentSymbol (hierarchical).
                    Self::push_hier(item, default_uri, None, 0, &mut out);
                }
            }
        }
        Self { symbols: out }
    }

    fn push_flat(item: &Value, out: &mut Vec<Symbol>) {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            return;
        };
        let kind = item.get("kind").and_then(Value::as_i64).unwrap_or(0);
        let loc = item.get("location");
        let uri = loc
            .and_then(|l| l.get("uri"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        // WorkspaceSymbol may carry `location: { uri }` with no range.
        let (line, col) = loc.and_then(|l| range_start(l, "range")).unwrap_or((0, 0));
        let container = item
            .get("containerName")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);
        out.push(Symbol {
            name: name.to_owned(),
            kind,
            uri,
            line,
            col,
            container,
            depth: 0,
        });
    }

    fn push_hier(item: &Value, uri: &str, parent: Option<&str>, depth: u32, out: &mut Vec<Symbol>) {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            return;
        };
        let kind = item.get("kind").and_then(Value::as_i64).unwrap_or(0);
        // `selectionRange` is the name; `range` the whole body.
        let (line, col) = range_start(item, "selectionRange")
            .or_else(|| range_start(item, "range"))
            .unwrap_or((0, 0));
        out.push(Symbol {
            name: name.to_owned(),
            kind,
            uri: uri.to_owned(),
            line,
            col,
            container: parent.map(ToOwned::to_owned),
            depth,
        });
        if let Some(children) = item.get("children").and_then(Value::as_array) {
            let child_parent = match parent {
                Some(p) => format!("{p}::{name}"),
                None => name.to_owned(),
            };
            for c in children {
                Self::push_hier(c, uri, Some(&child_parent), depth + 1, out);
            }
        }
    }

    /// True iff the server returned no symbols.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }
}

/// What a stored [`SymbolResponse`] answers.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum SymbolScope {
    /// `textDocument/documentSymbol` for a document URI.
    Document(String),
    /// `workspace/symbol` for a query string.
    Workspace(String),
}

/// Key into [`SymbolStore`].
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct SymbolKey {
    /// Decimal LSP server id.
    pub server: String,
    /// Document URI or workspace query.
    pub scope: SymbolScope,
}

impl SymbolKey {
    /// Key for a document outline.
    #[must_use]
    pub fn document(server: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            scope: SymbolScope::Document(uri.into()),
        }
    }

    /// Key for a workspace query.
    #[must_use]
    pub fn workspace(server: impl Into<String>, query: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            scope: SymbolScope::Workspace(query.into()),
        }
    }
}

/// Per-`(server, scope)` symbol state.
#[derive(Default)]
pub struct SymbolStore {
    by_key: HashMap<SymbolKey, SymbolResponse>,
}

impl SymbolStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the response at `key`.
    pub fn set(&mut self, key: SymbolKey, response: SymbolResponse) {
        self.by_key.insert(key, response);
    }

    /// Drop the entry at `key`.
    pub fn clear(&mut self, key: &SymbolKey) {
        self.by_key.remove(key);
    }

    /// Look up the entry at `key`.
    #[must_use]
    pub fn get(&self, key: &SymbolKey) -> Option<&SymbolResponse> {
        self.by_key.get(key)
    }
}

/// Cheaply-cloneable shared handle.
pub type SharedSymbolStore = Arc<Mutex<SymbolStore>>;

/// Build a fresh shared store.
#[must_use]
pub fn make_shared_store() -> SharedSymbolStore {
    Arc::new(Mutex::new(SymbolStore::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_hierarchical_document_symbols_with_depth_and_parent() {
        let v = json!([{
            "name": "Outer", "kind": 5,
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 9, "character": 0 } },
            "selectionRange": { "start": { "line": 0, "character": 6 }, "end": { "line": 0, "character": 11 } },
            "children": [{
                "name": "method", "kind": 6,
                "range": { "start": { "line": 2, "character": 2 }, "end": { "line": 4, "character": 2 } },
                "selectionRange": { "start": { "line": 2, "character": 7 }, "end": { "line": 2, "character": 13 } }
            }]
        }]);
        let r = SymbolResponse::from_lsp_value(&v, "file:///m.rs");
        assert_eq!(r.symbols.len(), 2);
        assert_eq!(r.symbols[0].name, "Outer");
        assert_eq!(r.symbols[0].depth, 0);
        assert_eq!((r.symbols[0].line, r.symbols[0].col), (0, 6)); // selectionRange
        assert_eq!(r.symbols[0].uri, "file:///m.rs"); // DocumentSymbol carries none
        assert_eq!(r.symbols[1].name, "method");
        assert_eq!(r.symbols[1].depth, 1);
        assert_eq!(r.symbols[1].container.as_deref(), Some("Outer"));
    }

    #[test]
    fn parses_flat_symbol_information() {
        let v = json!([{
            "name": "Thing", "kind": 23,
            "location": {
                "uri": "file:///a.rs",
                "range": { "start": { "line": 4, "character": 8 }, "end": { "line": 4, "character": 13 } }
            },
            "containerName": "modx"
        }]);
        let r = SymbolResponse::from_lsp_value(&v, "file:///ignored");
        assert_eq!(r.symbols.len(), 1);
        assert_eq!(r.symbols[0].uri, "file:///a.rs"); // from location, not default
        assert_eq!((r.symbols[0].line, r.symbols[0].col), (4, 8));
        assert_eq!(r.symbols[0].container.as_deref(), Some("modx"));
        assert_eq!(r.symbols[0].depth, 0);
    }

    #[test]
    fn workspace_symbol_without_range_defaults_to_origin() {
        // WorkspaceSymbol permits `location: { uri }` with no range.
        let v = json!([{ "name": "Z", "kind": 12, "location": { "uri": "file:///z.go" } }]);
        let r = SymbolResponse::from_lsp_value(&v, "");
        assert_eq!(r.symbols.len(), 1);
        assert_eq!(r.symbols[0].uri, "file:///z.go");
        assert_eq!((r.symbols[0].line, r.symbols[0].col), (0, 0));
    }

    #[test]
    fn null_is_empty_and_scopes_do_not_collide() {
        assert!(SymbolResponse::from_lsp_value(&Value::Null, "x").is_empty());
        let mut s = SymbolStore::new();
        let one = |n: &str| SymbolResponse {
            symbols: vec![Symbol {
                name: n.into(),
                kind: 1,
                uri: "u".into(),
                line: 0,
                col: 0,
                container: None,
                depth: 0,
            }],
        };
        s.set(SymbolKey::document("1", "file:///a"), one("doc"));
        s.set(SymbolKey::workspace("1", "file:///a"), one("ws"));
        assert_eq!(
            s.get(&SymbolKey::document("1", "file:///a"))
                .unwrap()
                .symbols[0]
                .name,
            "doc"
        );
        assert_eq!(
            s.get(&SymbolKey::workspace("1", "file:///a"))
                .unwrap()
                .symbols[0]
                .name,
            "ws"
        );
    }
}
