// hover.rs --- T M4.7 LSP-backed hover documentation.

//! Hover state and the [`HoverView`] that renders it.
//!
//! Per spec §M4.7: the editor sends `textDocument/hover` on demand
//! (e.g. a key chord or mouse event). The reply carries documentation
//! for the symbol under the cursor; pmacs collapses LSP's
//! `MarkupContent` / `MarkedString[]` shapes into plain UTF-8 text and
//! displays it in a popup view.
//!
//! # Why a separate module
//!
//! Mirrors [`crate::diag`] and [`crate::completion`]: shared store,
//! many readers (every popup view), one writer (the LSP manager).

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

/// Popup view that renders the most-recent hover for `key`. Owns
/// every cell inside its viewport.
pub struct HoverView {
    key: HoverKey,
    store: SharedHoverStore,
}

impl HoverView {
    /// Construct a hover view for `key` against `store`.
    #[must_use]
    pub fn new(key: HoverKey, store: SharedHoverStore) -> Self {
        Self { key, store }
    }

    /// The key this view is keyed under.
    #[must_use]
    pub fn key(&self) -> &HoverKey {
        &self.key
    }
}

impl View for HoverView {
    fn render(&mut self, _buf: &Buffer, viewport: Viewport, cells: &mut CellGrid<'_>) {
        let lines: Vec<String> = {
            let guard = self.store.lock().expect("hover store poisoned");
            guard
                .get(&self.key)
                .map(|h| h.contents.lines().map(str::to_owned).collect())
                .unwrap_or_default()
        };

        let max_rows = viewport.cell_size.rows;
        let max_cols = viewport.cell_size.cols;
        let origin = viewport.cell_origin;

        // Clear the popup region first (own every cell).
        for r in 0..max_rows {
            for c in 0..max_cols {
                *cells.at(CellCoord::new(origin.row + r, origin.col + c)) = Cell::default();
            }
        }
        if lines.is_empty() {
            return;
        }

        for row in 0..max_rows.min(lines.len() as u32) {
            let mut col: u32 = 0;
            for ch in lines[row as usize].chars() {
                if col >= max_cols {
                    break;
                }
                let width = char_display_width(ch);
                if width == 0 {
                    continue;
                }
                let cell = cells.at(CellCoord::new(origin.row + row, origin.col + col));
                cell.glyph = Glyph::Char(ch);
                cell.style = Style::default();
                cell.attachment = None;
                col += 1;
                if width == 2 && col < max_cols {
                    let cont = cells.at(CellCoord::new(origin.row + row, origin.col + col));
                    cont.glyph = Glyph::Continuation;
                    cont.style = Style::default();
                    cont.attachment = None;
                    col += 1;
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
