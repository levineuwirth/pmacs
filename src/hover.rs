// hover.rs --- T M4.7 LSP-backed hover documentation.

//! Hover state and the [`HoverView`] that renders it.
//!
//! Per spec §M4.7: the editor sends `textDocument/hover` on demand
//! (e.g. a key chord or mouse event). The reply carries documentation
//! for the symbol under the cursor; pmacs collapses LSP's
//! `MarkupContent` / `MarkedString[]` shapes into plain UTF-8 text, and
//! since E8 shows it in the popup at the caret ([`crate::lsp_popup`]),
//! drawn in the grid by [`HoverView`] and on the wire as
//! `InstanceMessage::Popup`.
//!
//! # Why a separate module
//!
//! Mirrors [`crate::diag`] and [`crate::completion`]: shared store,
//! many readers (every popup view), one writer (the LSP manager).

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

/// Parsed hover response. `contents` is the collapsed text body;
/// `range` is the buffer range the hover covers, if the server
/// reported one.
#[derive(Clone, Debug, Default)]
pub struct Hover {
    /// Documentation text. Lines are split on `\n`.
    pub contents: String,
    /// Optional source range `(start_line, start_col, end_line, end_col)`
    /// in LSP coordinates.
    pub range: Option<HoverRange>,
}

/// Source range a hover applies to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HoverRange {
    /// Start line.
    pub start_line: u32,
    /// Start column (LSP UTF-16 code units).
    pub start_col: u32,
    /// End line (one past last).
    pub end_line: u32,
    /// End column.
    pub end_col: u32,
}

impl Hover {
    /// Parse the LSP `Hover` JSON object. Accepts:
    /// * `{ "contents": MarkupContent, "range"?: Range }`
    /// * `{ "contents": MarkedString | MarkedString[] }`
    /// * `{ "contents": "string" }`
    /// * `null`
    ///
    /// Returns `None` for `null` (no hover information at this point).
    #[must_use]
    pub fn from_lsp_value(v: &Value) -> Option<Self> {
        if v.is_null() {
            return None;
        }
        let contents = collapse_contents(v.get("contents")?)?;
        let range = v.get("range").and_then(parse_hover_range);
        Some(Self { contents, range })
    }

    /// Number of visual lines in the hover's text. Always at least 1.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.contents.lines().count().max(1)
    }
}

fn collapse_contents(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_owned());
    }
    if let Some(arr) = v.as_array() {
        let mut out = String::new();
        let mut first = true;
        for item in arr {
            let s = collapse_contents(item)?;
            if !first {
                out.push('\n');
            }
            out.push_str(&s);
            first = false;
        }
        return Some(out);
    }
    if let Some(obj) = v.as_object()
        && let Some(s) = obj.get("value").and_then(Value::as_str)
    {
        return Some(s.to_owned());
    }
    None
}

fn parse_hover_range(v: &Value) -> Option<HoverRange> {
    let start = v.get("start")?;
    let end = v.get("end")?;
    Some(HoverRange {
        start_line: start.get("line")?.as_u64()? as u32,
        start_col: start.get("character")?.as_u64()? as u32,
        end_line: end.get("line")?.as_u64()? as u32,
        end_col: end.get("character")?.as_u64()? as u32,
    })
}

/// Per-server, per-uri hover state. A single hover at a time per key
/// (the previous one is replaced when a new request arrives).
#[derive(Default)]
pub struct HoverStore {
    by_key: HashMap<HoverKey, Hover>,
}

/// Key into [`HoverStore`].
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct HoverKey {
    /// LSP server id (decimal).
    pub server: String,
    /// Document URI.
    pub uri: String,
}

impl HoverKey {
    /// Construct a key.
    #[must_use]
    pub fn new(server: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            uri: uri.into(),
        }
    }
}

impl HoverStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the hover at `key`. Empty content drops the entry.
    pub fn set(&mut self, key: HoverKey, hover: Hover) {
        if hover.contents.is_empty() {
            self.by_key.remove(&key);
        } else {
            self.by_key.insert(key, hover);
        }
    }

    /// Drop the hover at `key`.
    pub fn clear(&mut self, key: &HoverKey) {
        self.by_key.remove(key);
    }

    /// Look up the hover at `key`.
    #[must_use]
    pub fn get(&self, key: &HoverKey) -> Option<&Hover> {
        self.by_key.get(key)
    }

    /// All keys currently in the store.
    pub fn keys(&self) -> impl Iterator<Item = &HoverKey> {
        self.by_key.keys()
    }
}

/// Cheaply-cloneable shared handle.
pub type SharedHoverStore = Arc<Mutex<HoverStore>>;

/// Build a fresh shared store.
#[must_use]
pub fn make_shared_store() -> SharedHoverStore {
    Arc::new(Mutex::new(HoverStore::new()))
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// [`View::kind`] of [`HoverView`], which dedupes it per window.
pub const HOVER_POPUP_KIND: &str = "hover-popup";

/// The grid's hover popup (E8): a self-suppressing overlay on the window
/// a hover was asked in, reading the core's one popup
/// ([`crate::lsp_popup`]) and drawing only while that popup is a hover
/// for this window and the buffer it shows. Attached by
/// [`crate::editor_core::EditorCore`] when a hover first opens there,
/// and persistent after, like the completion popup's overlay.
pub struct HoverView {
    popup: SharedLspPopup,
    window_id: WindowId,
    /// The theme its `ui.popup` faces resolve through; `None` in a bare
    /// core, which paints the grid's own defaults.
    theme: Option<ThemeHandle>,
}

impl HoverView {
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
                p.kind == PopupKind::Hover
                    && p.window_id == self.window_id
                    && p.buffer_id == buf.id()
            })
            .cloned()
    }
}

impl View for HoverView {
    fn kind(&self) -> &'static str {
        HOVER_POPUP_KIND
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
    fn from_lsp_value_parses_markup_content() {
        let v = json!({
            "contents": { "kind": "markdown", "value": "**bold** doc" },
            "range": {
                "start": { "line": 1, "character": 4 },
                "end":   { "line": 1, "character": 9 }
            }
        });
        let h = Hover::from_lsp_value(&v).unwrap();
        assert_eq!(h.contents, "**bold** doc");
        assert_eq!(
            h.range.unwrap(),
            HoverRange {
                start_line: 1,
                start_col: 4,
                end_line: 1,
                end_col: 9
            }
        );
    }

    #[test]
    fn from_lsp_value_parses_string_contents() {
        let v = json!({ "contents": "plain doc" });
        let h = Hover::from_lsp_value(&v).unwrap();
        assert_eq!(h.contents, "plain doc");
        assert!(h.range.is_none());
    }

    #[test]
    fn from_lsp_value_joins_marked_string_array() {
        let v = json!({
            "contents": [
                { "language": "rust", "value": "fn x()" },
                "Free text"
            ]
        });
        let h = Hover::from_lsp_value(&v).unwrap();
        assert_eq!(h.contents, "fn x()\nFree text");
    }

    #[test]
    fn from_lsp_value_returns_none_for_null() {
        assert!(Hover::from_lsp_value(&Value::Null).is_none());
    }

    #[test]
    fn store_set_get_clear() {
        let mut s = HoverStore::new();
        let key = HoverKey::new("1", "file:///a");
        s.set(
            key.clone(),
            Hover {
                contents: "hi".into(),
                range: None,
            },
        );
        assert_eq!(s.get(&key).unwrap().contents, "hi");
        s.clear(&key);
        assert!(s.get(&key).is_none());
    }

    #[test]
    fn empty_contents_drops_entry() {
        let mut s = HoverStore::new();
        let key = HoverKey::new("1", "file:///a");
        s.set(
            key.clone(),
            Hover {
                contents: "ok".into(),
                range: None,
            },
        );
        s.set(
            key.clone(),
            Hover {
                contents: String::new(),
                range: None,
            },
        );
        assert!(s.get(&key).is_none());
    }

    #[test]
    fn line_count_floor_is_one() {
        assert_eq!(
            Hover {
                contents: String::new(),
                range: None
            }
            .line_count(),
            1
        );
        assert_eq!(
            Hover {
                contents: "a\nb\nc".into(),
                range: None
            }
            .line_count(),
            3
        );
    }
}
