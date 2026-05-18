// signature.rs --- T M4.7 LSP-backed signature help.

//! Signature-help state and the [`SignatureView`] that renders it.
//!
//! Per spec §M4.7: while the cursor is inside a function call, pmacs
//! sends `textDocument/signatureHelp` to find out what the call's
//! parameters are. The reply lists candidate signatures (overloads),
//! with optional pointers to which signature and parameter are
//! "active" given the current cursor position. The
//! [`SignatureView`] renders the active signature with the active
//! parameter highlighted.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use unicode_width::UnicodeWidthChar;

use crate::buffer::Buffer;
use crate::cell::{Cell, CellCoord, CellGrid, Glyph, Style};
use crate::view::{View, Viewport};

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

/// Style for the active parameter --- bold + underline so the
/// emphasis carries on monochrome and palette terminals alike.
fn active_parameter_style() -> Style {
    Style {
        bold: true,
        underline: crate::cell::UnderlineStyle::Single,
        ..Style::default()
    }
}

/// Popup view that renders the active signature, highlighting the
/// active parameter.
pub struct SignatureView {
    key: SignatureKey,
    store: SharedSignatureStore,
}

impl SignatureView {
    /// Construct a signature view for `key` against `store`.
    #[must_use]
    pub fn new(key: SignatureKey, store: SharedSignatureStore) -> Self {
        Self { key, store }
    }

    /// The key this view is keyed under.
    #[must_use]
    pub fn key(&self) -> &SignatureKey {
        &self.key
    }
}

/// Snapshot of just the bits the renderer needs --- avoids holding
/// the lock across the render loop.
struct SignatureSnapshot {
    label: String,
    /// Optional `(start, end)` byte span of the active parameter
    /// inside `label`.
    active_span: Option<(u32, u32)>,
    /// Active parameter label (used when there's no span).
    active_label: Option<String>,
    /// Optional documentation line for the active signature.
    documentation: Option<String>,
}

impl SignatureView {
    fn snapshot(&self) -> Option<SignatureSnapshot> {
        let guard = self.store.lock().expect("signature store poisoned");
        let help = guard.get(&self.key)?;
        let sig = help.active()?;
        let active_idx = help.active_parameter_index();
        let (active_span, active_label) = match active_idx {
            Some(i) => match sig.parameters.get(i as usize) {
                Some(p) => (p.span, Some(p.label.clone())),
                None => (None, None),
            },
            None => (None, None),
        };
        Some(SignatureSnapshot {
            label: sig.label.clone(),
            active_span,
            active_label,
            documentation: sig.documentation.clone(),
        })
    }
}

impl View for SignatureView {
    fn render(&mut self, _buf: &Buffer, viewport: Viewport, cells: &mut CellGrid<'_>) {
        let snap = self.snapshot();
        let max_rows = viewport.cell_size.rows;
        let max_cols = viewport.cell_size.cols;
        let origin = viewport.cell_origin;

        for r in 0..max_rows {
            for c in 0..max_cols {
                *cells.at(CellCoord::new(origin.row + r, origin.col + c)) = Cell::default();
            }
        }
        let Some(snap) = snap else {
            return;
        };

        // Resolve the active span. Prefer offsets when present;
        // otherwise locate the active parameter's substring in the
        // label.
        let active_byte_range: Option<(usize, usize)> = match (snap.active_span, &snap.active_label)
        {
            (Some((s, e)), _) => Some((s as usize, e as usize)),
            (None, Some(lbl)) => snap.label.find(lbl.as_str()).map(|i| (i, i + lbl.len())),
            (None, None) => None,
        };

        // Row 0: signature label, with the active parameter range
        // styled. Row 1+: optional documentation, one line each
        // (truncated at viewport).
        if max_rows > 0 {
            let label_bytes = snap.label.as_bytes();
            let mut col: u32 = 0;
            let mut byte_idx: usize = 0;
            for ch in snap.label.chars() {
                if col >= max_cols {
                    break;
                }
                let width = char_display_width(ch);
                if width == 0 {
                    byte_idx += ch.len_utf8();
                    continue;
                }
                let in_active =
                    active_byte_range.is_some_and(|(s, e)| byte_idx >= s && byte_idx < e);
                let style = if in_active {
                    active_parameter_style()
                } else {
                    Style::default()
                };
                let cell = cells.at(CellCoord::new(origin.row, origin.col + col));
                cell.glyph = Glyph::Char(ch);
                cell.style = style;
                cell.attachment = None;
                col += 1;
                if width == 2 && col < max_cols {
                    let cont = cells.at(CellCoord::new(origin.row, origin.col + col));
                    cont.glyph = Glyph::Continuation;
                    cont.style = style;
                    cont.attachment = None;
                    col += 1;
                }
                byte_idx += ch.len_utf8();
                let _ = label_bytes; // silence unused-field on minor refactors
            }
        }

        if max_rows > 1
            && let Some(doc) = snap.documentation.as_deref()
        {
            for (i, line) in doc.lines().enumerate() {
                let row_idx = i as u32 + 1;
                if row_idx >= max_rows {
                    break;
                }
                let mut col: u32 = 0;
                for ch in line.chars() {
                    if col >= max_cols {
                        break;
                    }
                    let width = char_display_width(ch);
                    if width == 0 {
                        continue;
                    }
                    let cell = cells.at(CellCoord::new(origin.row + row_idx, origin.col + col));
                    cell.glyph = Glyph::Char(ch);
                    cell.style = Style::default();
                    cell.attachment = None;
                    col += 1;
                    if width == 2 && col < max_cols {
                        let cont = cells.at(CellCoord::new(origin.row + row_idx, origin.col + col));
                        cont.glyph = Glyph::Continuation;
                        cont.style = Style::default();
                        cont.attachment = None;
                        col += 1;
                    }
                }
            }
        }
    }
}

fn char_display_width(ch: char) -> u32 {
    UnicodeWidthChar::width(ch).unwrap_or(0) as u32
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
