// prepare_rename.rs --- T M4.5 LSP textDocument/prepareRename.

//! `textDocument/prepareRename` response state.
//!
//! Before prompting for a new name, the client can ask the server
//! whether the symbol at the cursor is renameable and what its extent
//! is. The response is a union:
//!
//!   * `null` — rename is **not** valid here.
//!   * `Range` — the symbol's range.
//!   * `{ range, placeholder }` — range plus a suggested initial
//!     value for the rename prompt.
//!   * `{ defaultBehavior: bool }` — the server defers the range
//!     computation to the client (use the word under the cursor).
//!
//! All shapes collapse into [`PrepareRenameResponse`]: `allowed` (the
//! one bit the rename flow gates on), an optional `placeholder` to
//! pre-fill the prompt, and an optional `range`. Like every other LSP
//! feature this is data only — the rename flow in `lsp.lua` reads the
//! store and decides whether to open the prompt.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// Parsed `textDocument/prepareRename` response.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PrepareRenameResponse {
    /// False iff the server returned `null` (or `defaultBehavior:
    /// false`) — rename must not proceed.
    pub allowed: bool,
    /// Suggested prompt pre-fill, when the server sent one.
    pub placeholder: Option<String>,
    /// The symbol range `(start_line, start_col, end_line, end_col)`
    /// when the server sent one (absent for the `defaultBehavior`
    /// shape — the client uses the cursor word).
    pub range: Option<(u32, u32, u32, u32)>,
}

fn parse_range(v: &Value) -> Option<(u32, u32, u32, u32)> {
    let start = v.get("start")?;
    let end = v.get("end")?;
    Some((
        start.get("line")?.as_u64()? as u32,
        start.get("character")?.as_u64()? as u32,
        end.get("line")?.as_u64()? as u32,
        end.get("character")?.as_u64()? as u32,
    ))
}

impl PrepareRenameResponse {
    /// Parse the `Range | { range, placeholder } | { defaultBehavior }
    /// | null` union. An unrecognised non-null shape is treated as
    /// "not allowed" rather than guessed at.
    #[must_use]
    pub fn from_lsp_value(v: &Value) -> Self {
        if v.is_null() {
            return Self::default();
        }
        // `{ defaultBehavior: bool }` — allowed iff the bool is true;
        // no explicit range (client uses the cursor word).
        if let Some(b) = v.get("defaultBehavior").and_then(Value::as_bool) {
            return Self {
                allowed: b,
                placeholder: None,
                range: None,
            };
        }
        // `{ range, placeholder? }`
        if let Some(r) = v.get("range") {
            return Self {
                allowed: true,
                placeholder: v
                    .get("placeholder")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                range: parse_range(r),
            };
        }
        // Bare `Range` (has `start` & `end`).
        if let Some(rng) = parse_range(v) {
            return Self {
                allowed: true,
                placeholder: None,
                range: Some(rng),
            };
        }
        Self::default()
    }
}

/// Per-server, per-uri prepareRename state.
#[derive(Default)]
pub struct PrepareRenameStore {
    by_key: HashMap<PrepareRenameKey, PrepareRenameResponse>,
}

/// Key into [`PrepareRenameStore`].
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct PrepareRenameKey {
    /// Decimal LSP server id.
    pub server: String,
    /// Document URI the request was made on.
    pub uri: String,
}

impl PrepareRenameKey {
    /// Construct a key.
    #[must_use]
    pub fn new(server: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            uri: uri.into(),
        }
    }
}

impl PrepareRenameStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the response at `key`.
    pub fn set(&mut self, key: PrepareRenameKey, response: PrepareRenameResponse) {
        self.by_key.insert(key, response);
    }

    /// Drop the entry at `key`.
    pub fn clear(&mut self, key: &PrepareRenameKey) {
        self.by_key.remove(key);
    }

    /// Look up the entry at `key`.
    #[must_use]
    pub fn get(&self, key: &PrepareRenameKey) -> Option<&PrepareRenameResponse> {
        self.by_key.get(key)
    }
}

/// Cheaply-cloneable shared handle.
pub type SharedPrepareRenameStore = Arc<Mutex<PrepareRenameStore>>;

/// Build a fresh shared store.
#[must_use]
pub fn make_shared_store() -> SharedPrepareRenameStore {
    Arc::new(Mutex::new(PrepareRenameStore::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn null_is_not_allowed() {
        let r = PrepareRenameResponse::from_lsp_value(&Value::Null);
        assert!(!r.allowed);
        assert!(r.placeholder.is_none());
        assert!(r.range.is_none());
    }

    #[test]
    fn bare_range_is_allowed() {
        let v = json!({
            "start": { "line": 2, "character": 4 },
            "end":   { "line": 2, "character": 9 }
        });
        let r = PrepareRenameResponse::from_lsp_value(&v);
        assert!(r.allowed);
        assert_eq!(r.range, Some((2, 4, 2, 9)));
        assert!(r.placeholder.is_none());
    }

    #[test]
    fn range_with_placeholder() {
        let v = json!({
            "range": {
                "start": { "line": 0, "character": 3 },
                "end":   { "line": 0, "character": 6 }
            },
            "placeholder": "foo"
        });
        let r = PrepareRenameResponse::from_lsp_value(&v);
        assert!(r.allowed);
        assert_eq!(r.placeholder.as_deref(), Some("foo"));
        assert_eq!(r.range, Some((0, 3, 0, 6)));
    }

    #[test]
    fn default_behavior_true_allowed_no_range() {
        let r = PrepareRenameResponse::from_lsp_value(&json!({ "defaultBehavior": true }));
        assert!(r.allowed);
        assert!(r.range.is_none());
        assert!(r.placeholder.is_none());
    }

    #[test]
    fn default_behavior_false_not_allowed() {
        let r = PrepareRenameResponse::from_lsp_value(&json!({ "defaultBehavior": false }));
        assert!(!r.allowed);
    }

    #[test]
    fn unknown_shape_not_allowed() {
        let r = PrepareRenameResponse::from_lsp_value(&json!({ "weird": 1 }));
        assert!(!r.allowed);
    }

    #[test]
    fn store_set_get_clear() {
        let mut s = PrepareRenameStore::new();
        let key = PrepareRenameKey::new("1", "file:///a");
        s.set(
            key.clone(),
            PrepareRenameResponse {
                allowed: true,
                placeholder: Some("x".into()),
                range: Some((0, 0, 0, 1)),
            },
        );
        assert!(s.get(&key).unwrap().allowed);
        s.clear(&key);
        assert!(s.get(&key).is_none());
    }
}
