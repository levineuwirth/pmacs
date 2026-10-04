// signature.rs --- T M4.7 LSP-backed signature help.

//! Signature-help state and the [`SignatureView`] that renders it.
//!
//! Per spec §M4.7: while the cursor is inside a function call, pmacs
//! sends `textDocument/signatureHelp` to find out what the call's
//! parameters are. The reply lists candidate signatures (overloads),
//! with optional pointers to which signature and parameter are
//! "active" given the current cursor position. Since E8 the active
//! signature shows in the popup at the caret ([`crate::lsp_popup`]),
//! drawn in the grid by [`SignatureView`] and on the wire as
//! `InstanceMessage::Popup`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pmacs_protocol::PopupKind;
use serde_json::Value;

use crate::buffer::Buffer;
use crate::cell::CellGrid;
use crate::highlight::ThemeHandle;
use crate::lsp_popup::{LspPopup, SharedLspPopup};
use crate::view::{View, Viewport};
use crate::window::WindowId;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// One signature parameter. `range` is a `[start, end]` byte slice
/// into the parent signature's `label` when the LSP gives offsets;
/// `text` is the full parameter label when the LSP gives a string.
/// Parsing collapses both onto a `(label, span)` pair where `span`
/// is `Some` iff offsets were provided.
#[derive(Clone, Debug)]
pub struct SignatureParameter {
    /// Parameter label, either the standalone string form or the
    /// substring of the parent signature that the offsets point at.
    pub label: String,
    /// Optional `[start, end]` byte offsets into the signature's
    /// `label`. Only populated when the LSP supplied them; the LSP
    /// `[number, number]` form is converted to `Some((start, end))`.
    pub span: Option<(u32, u32)>,
    /// Optional documentation.
    pub documentation: Option<String>,
}

/// One full signature (one overload). LSP allows multiple signatures
/// to share an `activeParameter`; pmacs preserves the per-signature
/// override if present.
#[derive(Clone, Debug)]
pub struct Signature {
    /// Full signature label (e.g. `fn echo(name: &str, count: usize)`).
    pub label: String,
    /// Optional documentation.
    pub documentation: Option<String>,
    /// Parameters in declaration order.
    pub parameters: Vec<SignatureParameter>,
    /// Per-signature `activeParameter` override; `None` falls back to
    /// the response-level `active_parameter`.
    pub active_parameter: Option<u32>,
}

/// Parsed `textDocument/signatureHelp` response.
#[derive(Clone, Debug, Default)]
pub struct SignatureHelp {
    /// Candidate signatures (overloads).
    pub signatures: Vec<Signature>,
    /// Index of the active signature; `0` if missing.
    pub active_signature: u32,
    /// Default active parameter, used when a signature doesn't carry
    /// its own `active_parameter`.
    pub active_parameter: Option<u32>,
}

impl SignatureHelp {
    /// Parse the LSP `SignatureHelp` JSON object. Returns an empty
    /// help (no signatures) for `null`.
    #[must_use]
    pub fn from_lsp_value(v: &Value) -> Self {
        if v.is_null() {
            return Self::default();
        }
        let signatures = v
            .get("signatures")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(parse_signature).collect::<Vec<_>>())
            .unwrap_or_default();
        let active_signature = v
            .get("activeSignature")
            .and_then(Value::as_u64)
            .map_or(0, |n| n as u32);
        let active_parameter = v
            .get("activeParameter")
            .and_then(Value::as_u64)
            .map(|n| n as u32);
        Self {
            signatures,
            active_signature,
            active_parameter,
        }
    }

    /// Currently active signature, or `None` if `signatures` is empty
    /// or the index is out of range.
    #[must_use]
    pub fn active(&self) -> Option<&Signature> {
        self.signatures.get(self.active_signature as usize)
    }

    /// Active parameter index for the active signature, falling back
    /// to the response-level default.
    #[must_use]
    pub fn active_parameter_index(&self) -> Option<u32> {
        self.active()
            .and_then(|s| s.active_parameter)
            .or(self.active_parameter)
    }
}

fn parse_signature(v: &Value) -> Option<Signature> {
    let label = v.get("label")?.as_str()?.to_owned();
    let documentation = v.get("documentation").and_then(extract_markup_text);
    let parameters = v
        .get("parameters")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|p| parse_parameter(p, &label))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let active_parameter = v
        .get("activeParameter")
        .and_then(Value::as_u64)
        .map(|n| n as u32);
    Some(Signature {
        label,
        documentation,
        parameters,
        active_parameter,
    })
}

fn parse_parameter(v: &Value, parent_label: &str) -> Option<SignatureParameter> {
    let label_field = v.get("label")?;
    let documentation = v.get("documentation").and_then(extract_markup_text);
    if let Some(s) = label_field.as_str() {
        return Some(SignatureParameter {
            label: s.to_owned(),
            span: None,
            documentation,
        });
    }
    if let Some(arr) = label_field.as_array()
        && arr.len() == 2
    {
        let start = arr[0].as_u64()? as u32;
        let end = arr[1].as_u64()? as u32;
        let s = parent_label
            .get(start as usize..end as usize)
            .unwrap_or("")
            .to_owned();
        return Some(SignatureParameter {
            label: s,
            span: Some((start, end)),
            documentation,
        });
    }
    None
}

fn extract_markup_text(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_owned());
    }
    if let Some(obj) = v.as_object()
        && let Some(s) = obj.get("value").and_then(Value::as_str)
    {
        return Some(s.to_owned());
    }
    None
}

/// Per-server, per-uri signature-help state.
#[derive(Default)]
pub struct SignatureStore {
    by_key: HashMap<SignatureKey, SignatureHelp>,
}

/// Key into [`SignatureStore`].
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct SignatureKey {
    /// LSP server id (decimal).
    pub server: String,
    /// Document URI.
    pub uri: String,
}

impl SignatureKey {
    /// Construct a key.
    #[must_use]
    pub fn new(server: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            uri: uri.into(),
        }
    }
}

impl SignatureStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the help at `key`. Empty `signatures` drops the entry.
    pub fn set(&mut self, key: SignatureKey, help: SignatureHelp) {
        if help.signatures.is_empty() {
            self.by_key.remove(&key);
        } else {
            self.by_key.insert(key, help);
        }
    }

    /// Drop the help at `key`.
    pub fn clear(&mut self, key: &SignatureKey) {
        self.by_key.remove(key);
    }

    /// Look up the help at `key`.
    #[must_use]
    pub fn get(&self, key: &SignatureKey) -> Option<&SignatureHelp> {
        self.by_key.get(key)
    }

    /// All keys currently in the store.
    pub fn keys(&self) -> impl Iterator<Item = &SignatureKey> {
        self.by_key.keys()
    }
}

/// Cheaply-cloneable shared handle.
pub type SharedSignatureStore = Arc<Mutex<SignatureStore>>;

/// Build a fresh shared store.
#[must_use]
pub fn make_shared_store() -> SharedSignatureStore {
    Arc::new(Mutex::new(SignatureStore::new()))
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// [`View::kind`] of [`SignatureView`], which dedupes it per window.
pub const SIGNATURE_POPUP_KIND: &str = "signature-popup";

/// The grid's signature popup (E8): a self-suppressing overlay on the
/// window a signature was asked in, reading the core's one popup
/// ([`crate::lsp_popup`]) and drawing only while that popup is a
/// signature for this window and the buffer it shows, its active
/// parameter marked. Attached by [`crate::editor_core::EditorCore`] when
/// a signature first opens there, and persistent after.
pub struct SignatureView {
    popup: SharedLspPopup,
    window_id: WindowId,
    /// The theme its `ui.popup` faces resolve through; `None` in a bare
    /// core, which paints the grid's own defaults.
    #[allow(dead_code, reason = "read by the grid painter (E8.4)")]
    theme: Option<ThemeHandle>,
}

impl SignatureView {
    /// An overlay for `window_id` reading `popup`.
    #[must_use]
    pub fn new(popup: SharedLspPopup, window_id: WindowId, theme: Option<ThemeHandle>) -> Self {
        Self {
            popup,
            window_id,
            theme,
        }
    }

    /// The popup this overlay draws now, if any.
    fn shown(&self, buf: &Buffer) -> Option<LspPopup> {
        self.popup
            .lock()
            .expect("lsp popup poisoned")
            .as_ref()
            .filter(|p| {
                p.kind == PopupKind::Signature
                    && p.window_id == self.window_id
                    && p.buffer_id == buf.id()
            })
            .cloned()
    }
}

impl View for SignatureView {
    fn kind(&self) -> &'static str {
        SIGNATURE_POPUP_KIND
    }

    fn render(&mut self, buf: &Buffer, _viewport: Viewport<'_>, _cells: &mut CellGrid<'_>) {
        // The grid's painter arrives with E8.4.
        let _ = self.shown(buf);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn from_lsp_value_parses_signature_help() {
        let v = json!({
            "signatures": [
                {
                    "label": "fn echo(name: &str, count: usize) -> String",
                    "documentation": "Echoes name count times.",
                    "parameters": [
                        { "label": "name: &str" },
                        { "label": "count: usize" }
                    ],
                    "activeParameter": 1
                }
            ],
            "activeSignature": 0,
            "activeParameter": 1
        });
        let h = SignatureHelp::from_lsp_value(&v);
        assert_eq!(h.signatures.len(), 1);
        assert_eq!(h.active_signature, 0);
        assert_eq!(h.active_parameter, Some(1));
        let sig = h.active().unwrap();
        assert_eq!(sig.parameters.len(), 2);
        assert_eq!(sig.parameters[1].label, "count: usize");
        assert_eq!(h.active_parameter_index(), Some(1));
    }

    #[test]
    fn parameter_label_with_offsets_extracts_substring() {
        let label = "fn f(a: i32, b: i32)";
        let v = json!({
            "signatures": [
                {
                    "label": label,
                    "parameters": [
                        { "label": [5, 11] },   // "a: i32"
                        { "label": [13, 19] }   // "b: i32"
                    ]
                }
            ],
            "activeSignature": 0,
            "activeParameter": 0
        });
        let h = SignatureHelp::from_lsp_value(&v);
        let sig = h.active().unwrap();
        assert_eq!(sig.parameters[0].label, "a: i32");
        assert_eq!(sig.parameters[0].span, Some((5, 11)));
        assert_eq!(sig.parameters[1].label, "b: i32");
    }

    #[test]
    fn from_lsp_value_handles_null() {
        let h = SignatureHelp::from_lsp_value(&Value::Null);
        assert!(h.signatures.is_empty());
        assert!(h.active().is_none());
        assert!(h.active_parameter_index().is_none());
    }

    #[test]
    fn signature_active_parameter_overrides_top_level() {
        let v = json!({
            "signatures": [
                {
                    "label": "fn x(a, b)",
                    "parameters": [{"label": "a"}, {"label": "b"}],
                    "activeParameter": 0
                }
            ],
            "activeSignature": 0,
            "activeParameter": 1
        });
        let h = SignatureHelp::from_lsp_value(&v);
        // Per-signature override wins.
        assert_eq!(h.active_parameter_index(), Some(0));
    }

    #[test]
    fn store_set_get_clear() {
        let mut s = SignatureStore::new();
        let key = SignatureKey::new("1", "file:///a");
        let h = SignatureHelp::from_lsp_value(&json!({
            "signatures": [{ "label": "fn x()", "parameters": [] }],
            "activeSignature": 0
        }));
        s.set(key.clone(), h);
        assert!(s.get(&key).is_some());
        s.clear(&key);
        assert!(s.get(&key).is_none());
    }

    #[test]
    fn empty_signatures_drops_entry() {
        let mut s = SignatureStore::new();
        let key = SignatureKey::new("1", "file:///a");
        s.set(
            key.clone(),
            SignatureHelp::from_lsp_value(&json!({"signatures": [], "activeSignature": 0})),
        );
        assert!(s.get(&key).is_none());
    }
}
