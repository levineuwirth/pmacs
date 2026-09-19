// inlay_hint.rs --- T M4.5 LSP inlay hints.

//! `textDocument/inlayHint` response state.
//!
//! An inlay hint is a small annotation the server wants drawn *inline*
//! at a position (a parameter name before an argument, an inferred
//! type after a `let`). The label is either a plain string or an
//! array of `InlayHintLabelPart`s (each carrying its own `value`,
//! tooltip, location, command); this module flattens the parts'
//! `value`s into one display string — the part-level interactivity
//! (go-to-def on a type in a hint) is later UX, like the hover panel.
//!
//! Mirrors [`crate::formatting`] / [`crate::code_action`]: a parsed,
//! per-`(server, uri)` store. Nothing here renders: the inline
//! virtual-text renderer is a separate milestone (the cell-overlay
//! model does not yet reflow real glyphs around inserted columns).
//! Lua reads [`InlayHintStore`] and surfaces hints; a render layer can
//! subscribe to the same store when it lands.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// LSP `InlayHintKind`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InlayHintKind {
    /// `1` — an inferred type.
    Type,
    /// `2` — a parameter name.
    Parameter,
}

impl InlayHintKind {
    fn from_lsp(n: u64) -> Option<Self> {
        match n {
            1 => Some(Self::Type),
            2 => Some(Self::Parameter),
            _ => None,
        }
    }

    /// Lowercase wire-ish label for the Lua surface.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Parameter => "parameter",
        }
    }
}

/// One parsed inlay hint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InlayHint {
    /// Zero-based line the hint sits on.
    pub line: u32,
    /// Zero-based column (UTF-16 code units, per LSP).
    pub col: u32,
    /// Display text (label string, or all label parts concatenated).
    pub label: String,
    /// Hint kind, if the server classified it.
    pub kind: Option<InlayHintKind>,
    /// Render a space before the label.
    pub padding_left: bool,
    /// Render a space after the label.
    pub padding_right: bool,
    /// Plain-text tooltip, if any (`MarkupContent` is flattened to
    /// its `value`).
    pub tooltip: Option<String>,
    /// E7c.3: the byte the hint sits at in the document's current
    /// text --- resolved against the text the request was answered
    /// for and carried across every edit since, as a token is --- or
    /// `None` when the document has no edit log or the edits since
    /// that text are no longer held; then `line` and `col` are what a
    /// consumer has, describing the text as answered.
    pub at: Option<u64>,
}

/// Parsed `textDocument/inlayHint` response.
#[derive(Clone, Debug, Default)]
pub struct InlayHintResponse {
    /// Hints in server order.
    pub hints: Vec<InlayHint>,
}

impl InlayHintResponse {
    /// Parse `InlayHint[] | null`. A `null` / non-array result yields
    /// an empty list.
    #[must_use]
    pub fn from_lsp_value(v: &Value) -> Self {
        let Some(arr) = v.as_array() else {
            return Self::default();
        };
        let mut hints = Vec::with_capacity(arr.len());
        for item in arr {
            if let Some(h) = parse_hint(item) {
                hints.push(h);
            }
        }
        Self { hints }
    }

    /// True iff the server returned no hints.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hints.is_empty()
    }
}

/// `label: string | InlayHintLabelPart[]` → one string.
fn parse_label(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_owned());
    }
    let parts = v.as_array()?;
    let mut out = String::new();
    for p in parts {
        if let Some(s) = p.get("value").and_then(Value::as_str) {
            out.push_str(s);
        }
    }
    Some(out)
}

/// `tooltip: string | MarkupContent` → plain text.
fn parse_tooltip(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_owned());
    }
    v.get("value").and_then(Value::as_str).map(str::to_owned)
}

fn parse_hint(v: &Value) -> Option<InlayHint> {
    let pos = v.get("position")?;
    let line = pos.get("line")?.as_u64()? as u32;
    let col = pos.get("character")?.as_u64()? as u32;
    let label = parse_label(v.get("label")?)?;
    Some(InlayHint {
        line,
        col,
        label,
        kind: v
            .get("kind")
            .and_then(Value::as_u64)
            .and_then(InlayHintKind::from_lsp),
        padding_left: v
            .get("paddingLeft")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        padding_right: v
            .get("paddingRight")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        tooltip: v.get("tooltip").and_then(parse_tooltip),
        at: None,
    })
}

/// Per-server, per-uri inlay-hint state.
#[derive(Default)]
pub struct InlayHintStore {
    by_key: HashMap<InlayHintKey, InlayHintResponse>,
    stale_uris: HashSet<String>,
    /// URIs whose hints carry byte anchors an edit recorder keeps
    /// current (E7c.3): `mark_stale` is a no-op for them and
    /// `is_stale` false, since a stale set is shifted, not hidden.
    logged_uris: HashSet<String>,
}

/// Key into [`InlayHintStore`].
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct InlayHintKey {
    /// Decimal LSP server id.
    pub server: String,
    /// Document URI the request was made on.
    pub uri: String,
}

impl InlayHintKey {
    /// Construct a key.
    #[must_use]
    pub fn new(server: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            uri: uri.into(),
        }
    }
}

impl InlayHintStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the response at `key`.
    pub fn set(&mut self, key: InlayHintKey, response: InlayHintResponse) {
        self.stale_uris.remove(&key.uri);
        self.by_key.insert(key, response);
    }

    /// Drop the entry at `key`. Also clears the stale flag for that
    /// URI when no other server has hint data for it.
    pub fn clear(&mut self, key: &InlayHintKey) {
        self.by_key.remove(key);
        if !self.by_key.keys().any(|k| k.uri == key.uri) {
            self.stale_uris.remove(&key.uri);
        }
    }

    /// Mark all inlay-hint entries for `uri` stale. Called when a
    /// `textDocument/didChange` is sent so renderers do not paint
    /// zero-width adornments at byte anchors from pre-edit text.
    pub fn mark_stale(&mut self, uri: impl Into<String>) {
        let uri = uri.into();
        if self.logged_uris.contains(&uri) {
            return;
        }
        self.stale_uris.insert(uri);
    }

    /// Declare that an edit recorder reports `uri`'s edits to this
    /// store (E7c.3): from here its hints are carried, never hidden,
    /// and `mark_stale` is a no-op for it. Idempotent.
    pub fn note_logged(&mut self, uri: impl Into<String>) {
        let uri = uri.into();
        self.stale_uris.remove(&uri);
        self.logged_uris.insert(uri);
    }

    /// Carry every hint anchor held for `uri` across `edit` (E7c.3),
    /// called by the recorder on every edit the buffer takes. An
    /// anchor moves as a range end does: text typed exactly at it
    /// goes before it, so a type hint stays after the identifier it
    /// annotates as the identifier grows. A hint whose anchor the edit
    /// deleted --- strictly inside the replaced text --- is dropped
    /// until the next answer (E7c fix 2), as a diagnostic about deleted
    /// text is; one at either end of the deletion is kept and shifted.
    pub fn translate_edit(&mut self, uri: &str, edit: crate::semantic_tokens::DocumentEdit) {
        if edit.old_end == edit.start && edit.inserted_len == 0 {
            return;
        }
        for (key, resp) in &mut self.by_key {
            if key.uri != uri {
                continue;
            }
            resp.hints
                .retain(|h| !h.at.is_some_and(|at| edit.deletes(at, at)));
            for h in &mut resp.hints {
                if let Some(at) = h.at {
                    h.at = Some(edit.translate_end(at));
                }
            }
        }
    }

    /// `true` iff `uri` has inlay-hint data that should not be
    /// rendered against the current buffer text.
    #[must_use]
    pub fn is_stale(&self, uri: &str) -> bool {
        self.stale_uris.contains(uri)
    }

    /// Look up the entry at `key`.
    #[must_use]
    pub fn get(&self, key: &InlayHintKey) -> Option<&InlayHintResponse> {
        self.by_key.get(key)
    }

    /// Look up the entry for `uri` regardless of which server keyed
    /// it. Mirrors [`crate::semantic_tokens::SemanticTokenStore::for_uri`]:
    /// the lowest (numeric) server id wins for determinism across
    /// `HashMap` order. No server is returned — unlike semantic
    /// tokens, inlay-hint positions are already pmacs byte offsets by
    /// the time they reach the store (the absorb path's
    /// `inbound_converted` rewrites the `Position`-shaped
    /// `InlayHint.position`), so the producer needs no per-server
    /// encoding to place them.
    #[must_use]
    pub fn for_uri(&self, uri: &str) -> Option<&InlayHintResponse> {
        self.by_key
            .iter()
            .filter(|(k, _)| k.uri == uri)
            .min_by_key(|(k, _)| {
                k.server
                    .parse::<u64>()
                    .map_or((u64::MAX, k.server.as_str()), |n| (n, ""))
            })
            .map(|(_, v)| v)
    }
}

/// Cheaply-cloneable shared handle.
pub type SharedInlayHintStore = Arc<Mutex<InlayHintStore>>;

/// Build a fresh shared store.
#[must_use]
pub fn make_shared_store() -> SharedInlayHintStore {
    Arc::new(Mutex::new(InlayHintStore::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_string_label_type_hint() {
        let v = json!([{
            "position": { "line": 3, "character": 12 },
            "label": ": String",
            "kind": 1,
            "paddingLeft": false,
            "paddingRight": true,
            "tooltip": "inferred type"
        }]);
        let r = InlayHintResponse::from_lsp_value(&v);
        assert_eq!(r.hints.len(), 1);
        let h = &r.hints[0];
        assert_eq!((h.line, h.col), (3, 12));
        assert_eq!(h.label, ": String");
        assert_eq!(h.kind, Some(InlayHintKind::Type));
        assert!(!h.padding_left);
        assert!(h.padding_right);
        assert_eq!(h.tooltip.as_deref(), Some("inferred type"));
    }

    #[test]
    fn concatenates_label_parts_for_parameter_hint() {
        let v = json!([{
            "position": { "line": 0, "character": 7 },
            "label": [ { "value": "name" }, { "value": ":" } ],
            "kind": 2,
            "tooltip": { "kind": "markdown", "value": "the param" }
        }]);
        let r = InlayHintResponse::from_lsp_value(&v);
        let h = &r.hints[0];
        assert_eq!(h.label, "name:");
        assert_eq!(h.kind, Some(InlayHintKind::Parameter));
        assert_eq!(h.tooltip.as_deref(), Some("the param"));
    }

    #[test]
    fn unknown_kind_and_missing_optionals_default() {
        let v = json!([{
            "position": { "line": 1, "character": 1 },
            "label": "x",
            "kind": 99
        }]);
        let r = InlayHintResponse::from_lsp_value(&v);
        let h = &r.hints[0];
        assert_eq!(h.kind, None);
        assert!(!h.padding_left);
        assert!(!h.padding_right);
        assert!(h.tooltip.is_none());
    }

    #[test]
    fn null_response_is_empty() {
        assert!(InlayHintResponse::from_lsp_value(&Value::Null).is_empty());
    }

    #[test]
    fn store_set_get_clear() {
        let mut s = InlayHintStore::new();
        let key = InlayHintKey::new("1", "file:///a");
        s.set(
            key.clone(),
            InlayHintResponse {
                hints: vec![InlayHint {
                    line: 0,
                    col: 0,
                    label: "h".into(),
                    kind: None,
                    padding_left: false,
                    padding_right: false,
                    tooltip: None,
                    at: None,
                }],
            },
        );
        assert_eq!(s.get(&key).unwrap().hints.len(), 1);
        s.clear(&key);
        assert!(s.get(&key).is_none());
    }

    #[test]
    fn stale_flag_clears_on_set_and_final_clear() {
        let mut s = InlayHintStore::new();
        let key = InlayHintKey::new("1", "file:///a");
        let response = InlayHintResponse {
            hints: vec![InlayHint {
                line: 0,
                col: 0,
                label: "h".into(),
                kind: None,
                padding_left: false,
                padding_right: false,
                tooltip: None,
                at: None,
            }],
        };

        s.set(key.clone(), response.clone());
        s.mark_stale("file:///a");
        assert!(s.is_stale("file:///a"));

        s.set(key.clone(), response);
        assert!(
            !s.is_stale("file:///a"),
            "fresh inlay hints clear stale flag"
        );

        s.mark_stale("file:///a");
        s.clear(&key);
        assert!(
            !s.is_stale("file:///a"),
            "clearing final hint entry clears stale flag"
        );
    }

    #[test]
    fn for_uri_filters_by_uri_and_picks_lowest_server() {
        let mk = |label: &str| InlayHintResponse {
            hints: vec![InlayHint {
                line: 0,
                col: 0,
                label: label.into(),
                kind: None,
                padding_left: false,
                padding_right: false,
                tooltip: None,
                at: None,
            }],
        };
        let mut s = InlayHintStore::new();
        s.set(InlayHintKey::new("10", "file:///a"), mk("s10"));
        s.set(InlayHintKey::new("9", "file:///a"), mk("s9"));
        s.set(InlayHintKey::new("2", "file:///b"), mk("s2"));

        // Lowest *numeric* server id wins ("9" < "10" numerically,
        // not lexicographically).
        assert_eq!(s.for_uri("file:///a").unwrap().hints[0].label, "s9");
        assert_eq!(s.for_uri("file:///b").unwrap().hints[0].label, "s2");
        assert!(s.for_uri("file:///nope").is_none());
    }

    /// E7c.3: a hint's anchor moves as a range end does --- shifted by
    /// an insert before it, carried past text typed exactly at it, kept
    /// by an edit after it --- and a logged URI is never stale.
    #[test]
    fn anchors_are_carried_across_edits() {
        use crate::semantic_tokens::DocumentEdit;
        let mut s = InlayHintStore::new();
        let key = InlayHintKey::new("1", "file:///a");
        let mut hint = InlayHint {
            line: 0,
            col: 5,
            label: ": i32".into(),
            kind: Some(InlayHintKind::Type),
            padding_left: false,
            padding_right: false,
            tooltip: None,
            at: Some(5),
        };
        s.note_logged("file:///a");
        s.set(
            key.clone(),
            InlayHintResponse {
                hints: vec![hint.clone()],
            },
        );
        let at = |s: &InlayHintStore| s.get(&key).unwrap().hints[0].at;
        s.translate_edit(
            "file:///a",
            DocumentEdit {
                start: 0,
                old_end: 0,
                inserted_len: 3,
            },
        );
        assert_eq!(at(&s), Some(8), "an insert before it shifts it");
        s.translate_edit(
            "file:///a",
            DocumentEdit {
                start: 8,
                old_end: 8,
                inserted_len: 2,
            },
        );
        assert_eq!(at(&s), Some(10), "text typed at it goes before it");
        s.translate_edit(
            "file:///a",
            DocumentEdit {
                start: 11,
                old_end: 12,
                inserted_len: 0,
            },
        );
        assert_eq!(at(&s), Some(10), "an edit after it leaves it");
        s.mark_stale("file:///a");
        assert!(!s.is_stale("file:///a"), "shifted, not hidden");
        hint.at = None;
        s.set(key.clone(), InlayHintResponse { hints: vec![hint] });
        s.translate_edit(
            "file:///a",
            DocumentEdit {
                start: 0,
                old_end: 0,
                inserted_len: 3,
            },
        );
        assert_eq!(at(&s), None, "no anchor, nothing to carry");
    }

    /// E7c fix 2: a delete ending at the anchor moves it and a delete
    /// starting at it leaves it; one that has it strictly inside drops
    /// the hint until the next answer, as a diagnostic about deleted
    /// text is dropped.
    #[test]
    fn a_hint_whose_anchor_is_inside_a_deleted_range_is_dropped() {
        use crate::semantic_tokens::DocumentEdit;
        let mut s = InlayHintStore::new();
        let key = InlayHintKey::new("1", "file:///a");
        let hint = InlayHint {
            line: 0,
            col: 10,
            label: ": i32".into(),
            kind: Some(InlayHintKind::Type),
            padding_left: false,
            padding_right: false,
            tooltip: None,
            at: Some(10),
        };
        s.note_logged("file:///a");
        s.set(key.clone(), InlayHintResponse { hints: vec![hint] });
        let at = |s: &InlayHintStore| s.get(&key).unwrap().hints.first().and_then(|h| h.at);
        let del = |s: &mut InlayHintStore, start: u64, old_end: u64| {
            s.translate_edit(
                "file:///a",
                DocumentEdit {
                    start,
                    old_end,
                    inserted_len: 0,
                },
            );
        };
        del(&mut s, 8, 10);
        assert_eq!(
            at(&s),
            Some(8),
            "the identifier before it deleted: kept, shifted"
        );
        del(&mut s, 8, 12);
        assert_eq!(at(&s), Some(8), "the text after it deleted: kept");
        del(&mut s, 4, 12);
        assert!(
            s.get(&key).unwrap().hints.is_empty(),
            "the line it sat on deleted: dropped"
        );
    }
}
