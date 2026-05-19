// semantic_tokens.rs --- T M4.5 LSP semantic tokens.

//! `textDocument/semanticTokens/full` response state.
//!
//! Semantic tokens are the server's type-aware classification of every
//! token in a document (this identifier is a *mutable* `variable`,
//! that one a `function.defaultLibrary`, …). The wire format is a flat
//! `data: number[]` whose every 5 ints describe one token **relative**
//! to the previous one:
//!
//! ```text
//!   [deltaLine, deltaStartChar, length, tokenType, tokenModifiers]
//! ```
//!
//! This module decodes that into a flat list of *absolute*
//! [`SemanticToken`]s, and parses the server's
//! `semanticTokensProvider.legend` so callers can resolve the
//! `token_type` / `token_modifiers` indices to names.
//!
//! Scope: this is the **LSP data layer only**. It is deliberately
//! independent of the M11 semantic-render protocol
//! ([`crate::semantic_render`] / [`crate::semantic_client`]), which
//! projects tree-sitter highlighting into the frontend wire families.
//! Wiring LSP tokens into rendering (a second styling authority,
//! priority vs. tree-sitter) is a separate rendering milestone; like
//! the other LSP features, nothing here paints — Lua reads the store.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// One decoded, **absolute**-positioned semantic token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticToken {
    /// Zero-based line.
    pub line: u32,
    /// Zero-based start column (UTF-16 code units, per LSP).
    pub start: u32,
    /// Token length in UTF-16 code units.
    pub length: u32,
    /// Index into the legend's `token_types`.
    pub token_type: u32,
    /// Bitset; bit `i` set ⇒ legend's `token_modifiers[i]` applies.
    pub token_modifiers: u32,
}

/// Parsed `textDocument/semanticTokens` response.
#[derive(Clone, Debug, Default)]
pub struct SemanticTokensResponse {
    /// Tokens in document order (decoded from the relative encoding).
    pub tokens: Vec<SemanticToken>,
    /// Opaque server cursor for a future delta request (unused by the
    /// v1 full-only path; surfaced for completeness).
    pub result_id: Option<String>,
}

impl SemanticTokensResponse {
    /// Parse `SemanticTokens | null`.
    ///
    /// `data` must be a flat array whose length is a multiple of 5; a
    /// trailing partial group (malformed server) is ignored rather
    /// than panicking. A `null` / shapeless result yields no tokens.
    #[must_use]
    pub fn from_lsp_value(v: &Value) -> Self {
        let result_id = v.get("resultId").and_then(Value::as_str).map(str::to_owned);
        let Some(data) = v.get("data").and_then(Value::as_array) else {
            return Self {
                tokens: Vec::new(),
                result_id,
            };
        };
        let ints: Vec<u32> = data
            .iter()
            .map(|n| n.as_u64().unwrap_or(0) as u32)
            .collect();
        let mut tokens = Vec::with_capacity(ints.len() / 5);
        let mut line = 0u32;
        let mut start = 0u32;
        for chunk in ints.chunks_exact(5) {
            let (d_line, d_start, length, tt, tm) =
                (chunk[0], chunk[1], chunk[2], chunk[3], chunk[4]);
            // deltaLine is relative to the previous token's line;
            // deltaStartChar is relative to the previous token's
            // start *iff* on the same line, else absolute from col 0.
            line += d_line;
            start = if d_line == 0 {
                start + d_start
            } else {
                d_start
            };
            tokens.push(SemanticToken {
                line,
                start,
                length,
                token_type: tt,
                token_modifiers: tm,
            });
        }
        Self { tokens, result_id }
    }

    /// True iff the server returned no tokens.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

/// The server's `semanticTokensProvider.legend`: the ordered name
/// tables the `token_type` index and `token_modifiers` bits map into.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SemanticTokensLegend {
    /// `token_type` index → name.
    pub token_types: Vec<String>,
    /// Modifier bit position → name.
    pub token_modifiers: Vec<String>,
}

impl SemanticTokensLegend {
    /// Pull the legend out of an `initialize` `ServerCapabilities`
    /// JSON value. Returns `None` if the server advertises no
    /// `semanticTokensProvider` (or it carries no `legend`).
    #[must_use]
    pub fn from_capabilities(caps: &Value) -> Option<Self> {
        let legend = caps.get("semanticTokensProvider")?.get("legend")?;
        let pull = |key: &str| -> Vec<String> {
            legend
                .get(key)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        Some(Self {
            token_types: pull("tokenTypes"),
            token_modifiers: pull("tokenModifiers"),
        })
    }

    /// Resolve a `token_type` index to its legend name.
    #[must_use]
    pub fn type_name(&self, index: u32) -> Option<&str> {
        self.token_types.get(index as usize).map(String::as_str)
    }

    /// Resolve a `token_modifiers` bitset to the set legend names,
    /// low bit first.
    #[must_use]
    pub fn modifier_names(&self, bits: u32) -> Vec<&str> {
        (0..self.token_modifiers.len())
            .filter(|i| bits & (1 << i) != 0)
            .map(|i| self.token_modifiers[i].as_str())
            .collect()
    }
}

/// Per-server, per-uri semantic-token state.
#[derive(Default)]
pub struct SemanticTokenStore {
    by_key: HashMap<SemanticTokenKey, SemanticTokensResponse>,
}

/// Key into [`SemanticTokenStore`].
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct SemanticTokenKey {
    /// Decimal LSP server id.
    pub server: String,
    /// Document URI the request was made on.
    pub uri: String,
}

impl SemanticTokenKey {
    /// Construct a key.
    #[must_use]
    pub fn new(server: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            uri: uri.into(),
        }
    }
}

impl SemanticTokenStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the response at `key`.
    pub fn set(&mut self, key: SemanticTokenKey, response: SemanticTokensResponse) {
        self.by_key.insert(key, response);
    }

    /// Drop the entry at `key`.
    pub fn clear(&mut self, key: &SemanticTokenKey) {
        self.by_key.remove(key);
    }

    /// Look up the entry at `key`.
    #[must_use]
    pub fn get(&self, key: &SemanticTokenKey) -> Option<&SemanticTokensResponse> {
        self.by_key.get(key)
    }
}

/// Cheaply-cloneable shared handle.
pub type SharedSemanticTokenStore = Arc<Mutex<SemanticTokenStore>>;

/// Build a fresh shared store.
#[must_use]
pub fn make_shared_store() -> SharedSemanticTokenStore {
    Arc::new(Mutex::new(SemanticTokenStore::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decodes_relative_encoding_across_lines() {
        // Three tokens:
        //  - line 0, char 0, len 3, type 1, mods 0
        //  - same line, +5 chars  → char 5, len 2, type 2, mods 0
        //  - +2 lines, char 4 (absolute, deltaLine!=0), len 6, type 0,
        //    mods 0b101
        let v = json!({
            "resultId": "1",
            "data": [
                0, 0, 3, 1, 0,
                0, 5, 2, 2, 0,
                2, 4, 6, 0, 5
            ]
        });
        let r = SemanticTokensResponse::from_lsp_value(&v);
        assert_eq!(r.result_id.as_deref(), Some("1"));
        assert_eq!(r.tokens.len(), 3);
        assert_eq!(
            r.tokens[0],
            SemanticToken {
                line: 0,
                start: 0,
                length: 3,
                token_type: 1,
                token_modifiers: 0
            }
        );
        assert_eq!(
            r.tokens[1],
            SemanticToken {
                line: 0,
                start: 5,
                length: 2,
                token_type: 2,
                token_modifiers: 0
            }
        );
        assert_eq!(
            r.tokens[2],
            SemanticToken {
                line: 2,
                start: 4,
                length: 6,
                token_type: 0,
                token_modifiers: 5
            }
        );
    }

    #[test]
    fn trailing_partial_group_is_ignored() {
        let v = json!({ "data": [0, 0, 3, 1, 0, 9, 9] });
        let r = SemanticTokensResponse::from_lsp_value(&v);
        assert_eq!(r.tokens.len(), 1);
    }

    #[test]
    fn null_response_is_empty() {
        let r = SemanticTokensResponse::from_lsp_value(&Value::Null);
        assert!(r.is_empty());
        assert!(r.result_id.is_none());
    }

    #[test]
    fn legend_parses_and_resolves() {
        let caps = json!({
            "semanticTokensProvider": {
                "legend": {
                    "tokenTypes": ["namespace", "type", "function"],
                    "tokenModifiers": ["declaration", "readonly", "static"]
                },
                "full": true
            }
        });
        let legend = SemanticTokensLegend::from_capabilities(&caps).unwrap();
        assert_eq!(legend.type_name(2), Some("function"));
        assert_eq!(legend.type_name(99), None);
        // bits 0b101 = declaration + static
        assert_eq!(legend.modifier_names(0b101), vec!["declaration", "static"]);
        assert_eq!(legend.modifier_names(0), Vec::<&str>::new());
    }

    #[test]
    fn no_provider_yields_no_legend() {
        assert!(
            SemanticTokensLegend::from_capabilities(&json!({ "hoverProvider": true })).is_none()
        );
    }

    #[test]
    fn store_set_get_clear() {
        let mut s = SemanticTokenStore::new();
        let key = SemanticTokenKey::new("1", "file:///a");
        s.set(
            key.clone(),
            SemanticTokensResponse {
                tokens: vec![SemanticToken {
                    line: 0,
                    start: 0,
                    length: 1,
                    token_type: 0,
                    token_modifiers: 0,
                }],
                result_id: None,
            },
        );
        assert_eq!(s.get(&key).unwrap().tokens.len(), 1);
        s.clear(&key);
        assert!(s.get(&key).is_none());
    }
}
