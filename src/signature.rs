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
        Self::from_lsp_value_in(v, crate::lsp::PositionEncoding::Utf8)
    }

    /// [`Self::from_lsp_value`] for a server that negotiated `encoding`
    /// (E8.5): a parameter's `[start, end]` label offsets are converted
    /// here to bytes of the signature's label, so the span the popup
    /// marks is the parameter on a non-ASCII label too. The absorb path
    /// rewrites `Position`s to bytes; these offsets are not positions,
    /// so it never reached them.
    ///
    /// Which units they count is read from the offsets themselves (E8
    /// fix round 1). The spec says UTF-16, "as `Position` and `Range`
    /// does", and servers that negotiated UTF-8 take that both ways:
    /// rust-analyzer counts UTF-16 units, clangd its negotiated bytes.
    /// So the offsets are read in the negotiated encoding and in UTF-16,
    /// and the reading under which every parameter is delimited and
    /// balanced in the label ([`coherent`]) is the one kept. Under
    /// none, or under two that differ, no parameter carries a span and
    /// the popup marks nothing rather than text that may be wrong.
    #[must_use]
    pub fn from_lsp_value_in(v: &Value, encoding: crate::lsp::PositionEncoding) -> Self {
        if v.is_null() {
            return Self::default();
        }
        let signatures = v
            .get("signatures")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| parse_signature(s, encoding))
                    .collect::<Vec<_>>()
            })
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

fn parse_signature(v: &Value, encoding: crate::lsp::PositionEncoding) -> Option<Signature> {
    let label = v.get("label")?.as_str()?.to_owned();
    let documentation = v.get("documentation").and_then(extract_markup_text);
    let raw: Vec<RawParameter> = v
        .get("parameters")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(parse_parameter).collect())
        .unwrap_or_default();
    let offsets: Vec<(u32, u32)> = raw.iter().filter_map(|p| p.offsets).collect();
    let mut readings = vec![encoding];
    if encoding != crate::lsp::PositionEncoding::Utf16 {
        readings.push(crate::lsp::PositionEncoding::Utf16);
    }
    let mut whole = readings.into_iter().filter_map(|enc| {
        let spans = offsets
            .iter()
            .map(|&(s, e)| Some((label_byte(&label, s, enc)?, label_byte(&label, e, enc)?)))
            .collect::<Option<Vec<_>>>()?;
        coherent(&label, &spans).then_some(spans)
    });
    let spans = match (whole.next(), whole.next()) {
        (Some(one), Some(other)) if one != other => None,
        (one, _) => one,
    };
    let mut spans = spans.map(Vec::into_iter);
    let parameters = raw
        .into_iter()
        .map(|p| {
            let (label, span) = match (p.offsets, &mut spans) {
                (None, _) => (p.label, None),
                (Some(_), Some(spans)) => {
                    let (s, e) = spans.next().expect("one span per offset pair");
                    (label[s..e].to_owned(), Some((s as u32, e as u32)))
                }
                (Some(_), None) => (String::new(), None),
            };
            SignatureParameter {
                label,
                span,
                documentation: p.documentation,
            }
        })
        .collect();
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

/// A parameter as the server sent it: its label string, or its label's
/// `[start, end]` offsets in units not yet known.
struct RawParameter {
    label: String,
    offsets: Option<(u32, u32)>,
    documentation: Option<String>,
}

fn parse_parameter(v: &Value) -> Option<RawParameter> {
    let label_field = v.get("label")?;
    let documentation = v.get("documentation").and_then(extract_markup_text);
    if let Some(s) = label_field.as_str() {
        return Some(RawParameter {
            label: s.to_owned(),
            offsets: None,
            documentation,
        });
    }
    if let Some(arr) = label_field.as_array()
        && arr.len() == 2
    {
        let unit = |v: &Value| v.as_u64().and_then(|n| u32::try_from(n).ok());
        return Some(RawParameter {
            label: String::new(),
            offsets: Some((unit(&arr[0])?, unit(&arr[1])?)),
            documentation,
        });
    }
    None
}

/// The byte of `label` that `units` units of `enc` reach, or `None` when
/// they fall inside a character or past the end. Exact, unlike
/// [`crate::lsp::char_to_byte`], which snaps: a reading in the wrong
/// units must show as one.
fn label_byte(label: &str, units: u32, enc: crate::lsp::PositionEncoding) -> Option<usize> {
    let units = units as usize;
    match enc {
        crate::lsp::PositionEncoding::Utf8 => label.is_char_boundary(units).then_some(units),
        crate::lsp::PositionEncoding::Utf16 => {
            let mut counted = 0usize;
            for (byte, ch) in label.char_indices() {
                if counted == units {
                    return Some(byte);
                }
                if counted > units {
                    return None;
                }
                counted += ch.len_utf16();
            }
            (counted == units).then_some(label.len())
        }
    }
}

/// Whether `spans` (bytes of `label`, in the parameters' order) each mark
/// what a parameter is: non-empty, in order and apart, starting and
/// ending on no whitespace, opened (past whitespace) by the label's start
/// or one of `( , < [ { |` and closed by its end or one of `, ) > ] } |`,
/// with its own brackets balanced. Offsets read in the wrong units shift
/// by the bytes and units the characters before them differ by; a run
/// that is merely whole (E8 fix round 1's test) is reached that way by
/// ordinary non-ASCII code, `u8, a` or `名前` alone, and a delimited,
/// balanced one is not (review 2's Medium 1).
fn coherent(label: &str, spans: &[(usize, usize)]) -> bool {
    let opened = |at: usize| {
        label[..at]
            .trim_end()
            .chars()
            .next_back()
            .is_none_or(|c| "(,<[{|".contains(c))
    };
    let closed = |at: usize| {
        label[at..]
            .trim_start()
            .chars()
            .next()
            .is_none_or(|c| ",)>]}|".contains(c))
    };
    let mut end_of_last = 0usize;
    spans.iter().all(|&(s, e)| {
        let ok = s < e
            && s >= end_of_last
            && !label[s..e].starts_with(char::is_whitespace)
            && !label[s..e].ends_with(char::is_whitespace)
            && opened(s)
            && closed(e)
            && balanced(&label[s..e]);
        end_of_last = e;
        ok
    })
}

/// Whether every bracket in `run` closes the one it opened, in order and
/// all of them: `(`, `[`, `{` and `<`, an arrow's `>` (`->`, `=>`)
/// closing none.
fn balanced(run: &str) -> bool {
    let mut open = Vec::new();
    let mut last = None;
    for c in run.chars() {
        match c {
            '(' | '[' | '{' | '<' => open.push(c),
            '>' if matches!(last, Some('-' | '=')) => {}
            ')' | ']' | '}' | '>' => {
                let opener = match c {
                    ')' => '(',
                    ']' => '[',
                    '}' => '{',
                    _ => '<',
                };
                if open.pop() != Some(opener) {
                    return false;
                }
            }
            _ => {}
        }
        last = Some(c);
    }
    open.is_empty()
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

    fn render(&mut self, buf: &Buffer, viewport: Viewport<'_>, cells: &mut CellGrid<'_>) {
        if let Some(popup) = self.shown(buf) {
            crate::lsp_popup::paint_grid_popup_into(
                &self.popup,
                buf,
                viewport,
                cells,
                &popup,
                self.theme.as_ref(),
            );
        }
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

    #[test]
    fn utf16_label_offsets_become_byte_offsets_of_the_label() {
        // "fn größe(höhe: u8, b: u8)": `höhe: u8` is UTF-16 units 9..17
        // (ö and ß are one unit each) and bytes 11..20.
        let label = "fn größe(höhe: u8, b: u8)";
        let v = json!({
            "signatures": [{
                "label": label,
                "parameters": [{ "label": [9, 17] }, { "label": [19, 24] }],
            }],
            "activeParameter": 0,
        });
        let h = SignatureHelp::from_lsp_value_in(&v, crate::lsp::PositionEncoding::Utf16);
        let p = &h.signatures[0].parameters[0];
        assert_eq!(p.label, "höhe: u8");
        assert_eq!(p.span, Some((11, 20)));
        assert_eq!(&label[11..20], "höhe: u8");
        // A server that negotiated UTF-8 and counts these offsets in
        // UTF-16 anyway (rust-analyzer does): read as bytes they cut
        // `größe` and `u8`, so the UTF-16 reading is the one kept.
        let h = SignatureHelp::from_lsp_value(&v);
        let p = &h.signatures[0].parameters;
        assert_eq!(
            (p[0].label.as_str(), p[0].span),
            ("höhe: u8", Some((11, 20)))
        );
        assert_eq!((p[1].label.as_str(), p[1].span), ("b: u8", Some((22, 27))));
    }

    /// E8 fix round 1: clangd negotiates UTF-8 and counts its label
    /// offsets in bytes, so the negotiated reading is kept; read as
    /// UTF-16 the same offsets would cut `int` and `höhe`.
    #[test]
    fn utf8_label_offsets_from_a_server_that_counts_bytes_are_kept() {
        let label = "größe(int höhe, int b) -> int";
        let v = json!({
            "signatures": [{
                "label": label,
                "parameters": [{ "label": [8, 17] }, { "label": [19, 24] }],
            }],
            "activeParameter": 1,
        });
        let h = SignatureHelp::from_lsp_value_in(&v, crate::lsp::PositionEncoding::Utf8);
        let p = &h.signatures[0].parameters;
        assert_eq!(
            (p[0].label.as_str(), p[0].span),
            ("int höhe", Some((8, 17)))
        );
        assert_eq!((p[1].label.as_str(), p[1].span), ("int b", Some((19, 24))));
    }

    /// Offsets no reading makes whole carry no span: the popup marks
    /// nothing rather than the wrong text.
    #[test]
    fn label_offsets_no_reading_makes_whole_mark_nothing() {
        let v = json!({
            "signatures": [{
                "label": "fn f(ab: u8, cd: u8)",
                "parameters": [{ "label": [6, 9] }, { "label": [14, 18] }],
            }],
            "activeParameter": 1,
        });
        let h = SignatureHelp::from_lsp_value_in(&v, crate::lsp::PositionEncoding::Utf8);
        let p = &h.signatures[0].parameters;
        assert_eq!(p.len(), 2, "each parameter is kept, with its documentation");
        assert!(p.iter().all(|p| p.span.is_none() && p.label.is_empty()));
    }

    /// The marked text of each parameter of `v`'s first signature, read
    /// for a server that negotiated UTF-8.
    fn marks_utf8(v: &Value) -> Vec<Option<String>> {
        let h = SignatureHelp::from_lsp_value_in(v, crate::lsp::PositionEncoding::Utf8);
        let label = &h.signatures[0].label;
        h.signatures[0]
            .parameters
            .iter()
            .map(|p| {
                p.span
                    .map(|(s, e)| label[s as usize..e as usize].to_owned())
            })
            .collect()
    }

    /// E8 fix round 2 (review 2's Medium 1): two readings under which
    /// every parameter is delimited and balanced, and which differ, mark
    /// nothing. `struct S<名é, T, U>(T, U)`'s UTF-16 offsets (19..20,
    /// 22..23) read as bytes are the generic `U` and the field `T`, each
    /// opened by a `,` or `(` and closed by a `>` or `,`; read as UTF-16
    /// they are the fields. Either may be the server's, so neither is
    /// kept. (The generic `名é` names no field, so this is not Rust that
    /// compiles; the generated sweep reached no such label.)
    #[test]
    fn two_readings_that_differ_mark_nothing() {
        let v = json!({
            "signatures": [{
                "label": "struct S<名é, T, U>(T, U)",
                "parameters": [{ "label": [19, 20] }, { "label": [22, 23] }],
            }],
            "activeParameter": 1,
        });
        assert_eq!(marks_utf8(&v), [None, None]);
    }

    /// E8 fix round 2: a reading whose span leaves a bracket open is not
    /// a parameter. `fn move_to(現在地: (i32, i32))` from rust-analyzer
    /// (UTF-16 offsets 11..26 on a UTF-8 server): read as bytes they are
    /// `現在地: (i32`, opened by the `(` and closed by the tuple's `,`,
    /// whole but unbalanced; refused, the UTF-16 reading is the only one
    /// left and is marked.
    #[test]
    fn a_reading_that_leaves_a_bracket_open_is_not_a_parameter() {
        let v = json!({
            "signatures": [{
                "label": "fn move_to(現在地: (i32, i32))",
                "parameters": [{ "label": [11, 26] }],
            }],
            "activeParameter": 0,
        });
        assert_eq!(marks_utf8(&v), [Some("現在地: (i32, i32)".to_owned())]);
    }

    /// The brackets a parameter's run must close, an arrow's `>` closing
    /// none.
    #[test]
    fn balanced_counts_brackets_and_not_arrows() {
        for run in [
            "f: impl Fn(u8) -> u8",
            "cb: (x: number) => void",
            "m: HashMap<K, Vec<(u8, u8)>>",
            "xs: [u8; 4]",
            "d: dict = {}",
        ] {
            assert!(balanced(run), "{run:?}");
        }
        for run in ["a: (i32", "Vec<T", "u8)", "x: [u8; 4", "(a]"] {
            assert!(!balanced(run), "{run:?}");
        }
    }
}
