// highlight.rs --- T M4.3 syntax-highlight view + theme.

//! Syntax-highlight view (T M4.3).
//!
//! Glue between [`crate::syntax`] (parse trees + highlight queries)
//! and the [`crate::view::View`] composition stack
//! ([`crate::overlay`]). Reads the active [`Theme`] to translate
//! tree-sitter capture names into [`Style`] values, then writes
//! merged styles into the cells the base [`crate::text_view::TextView`]
//! has already painted.
//!
//! # Lifecycle
//!
//! 1. [`crate::syntax::SyntaxRegistry`] holds the shared
//!    [`ThemeHandle`] and per-language compiled queries.
//! 2. The Lua side ([`crate::lua_bindings`]) creates a
//!    [`SyntaxHighlightView`] when a buffer's grammar is detected
//!    and pushes it onto the active window's overlay stack.
//! 3. Every render: the view checks whether the buffer's
//!    [`crate::syntax::ParseViewHandle`] holds a different parse
//!    tree than last frame (compared by `Arc::ptr_eq`); if so it
//!    re-runs the highlight query, caches the result, and recomputes
//!    a per-line spans index. Render reads from the cached index
//!    to apply styles cell by cell.
//!
//! # Threading
//!
//! [`SyntaxHighlightView`] holds only `Arc<...>`-based state so it
//! satisfies the [`crate::view::View`]'s `Send` bound. In practice
//! everything runs main-thread; the `Send` bound exists because
//! `Box<dyn View>` is held by the buffer / window machinery.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use unicode_width::UnicodeWidthChar;

use crate::buffer::Buffer;
use crate::cell::{CellCoord, CellGrid, Color, Style, UnderlineStyle};
use crate::overlay::merge_styles;
use crate::syntax::{HighlightSpan, ParseTreeBundle, ParseViewHandle, compute_highlight_spans};
use crate::view::{View, Viewport};

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

/// Map from tree-sitter capture name to [`Style`] plus a fallback
/// `default_style`.
///
/// Capture names are dotted: `function.method`, `variable.parameter`,
/// `keyword.control.return`. [`Self::lookup`] tries the full name
/// first, then progressively shorter dot-separated prefixes, then
/// falls back to `default_style`. This matches conventional editor
/// theme behavior --- a theme can either be coarse (just `keyword`)
/// or fine-grained (`keyword.control.return`) and the lookup walks
/// the same hierarchy in both cases.
#[derive(Clone, Debug, Default)]
pub struct Theme {
    /// Direct map from capture name → style. T M4.3.
    pub by_capture: HashMap<String, Style>,
    /// Fallback style when no capture matches. Defaults to the
    /// terminal default colors (no override) so unhighlighted text
    /// looks identical to plain rendering.
    pub default_style: Style,
}

impl Theme {
    /// Empty theme: no captures match anything; the default style
    /// is the terminal default. Useful for tests that want to start
    /// from a clean slate.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// A small built-in dark theme that picks up the most common
    /// tree-sitter captures so opening a code file produces visible
    /// highlighting without a user theme. Uses 8/16-color indexed
    /// terminal colors for portability --- truecolor themes can be
    /// pushed in from Lua.
    ///
    /// Capture coverage is intentionally limited: keyword,
    /// function, type, string, comment, constant, number, operator,
    /// variable. Themes that want more granularity layer on top via
    /// [`Self::insert`] / Lua's `pmacs.theme.set`.
    #[must_use]
    pub fn default_dark() -> Self {
        let mut by_capture = HashMap::new();
        let bold = Style {
            bold: true,
            ..Style::default()
        };
        // 8-color indexed: 1 red, 2 green, 3 yellow, 4 blue,
        // 5 magenta, 6 cyan; bright variants 9..14.
        by_capture.insert(
            "keyword".to_owned(),
            Style {
                fg: Color::Indexed(5),
                ..bold
            },
        );
        by_capture.insert(
            "keyword.control".to_owned(),
            Style {
                fg: Color::Indexed(13),
                ..bold
            },
        );
        by_capture.insert(
            "function".to_owned(),
            Style {
                fg: Color::Indexed(4),
                ..Style::default()
            },
        );
        by_capture.insert(
            "function.method".to_owned(),
            Style {
                fg: Color::Indexed(12),
                ..Style::default()
            },
        );
        by_capture.insert(
            "type".to_owned(),
            Style {
                fg: Color::Indexed(3),
                ..Style::default()
            },
        );
        by_capture.insert(
            "type.builtin".to_owned(),
            Style {
                fg: Color::Indexed(11),
                ..Style::default()
            },
        );
        by_capture.insert(
            "string".to_owned(),
            Style {
                fg: Color::Indexed(2),
                ..Style::default()
            },
        );
        by_capture.insert(
            "comment".to_owned(),
            Style {
                fg: Color::Indexed(8),
                italic: true,
                ..Style::default()
            },
        );
        by_capture.insert(
            "constant".to_owned(),
            Style {
                fg: Color::Indexed(1),
                ..Style::default()
            },
        );
        by_capture.insert(
            "constant.builtin".to_owned(),
            Style {
                fg: Color::Indexed(9),
                ..Style::default()
            },
        );
        by_capture.insert(
            "number".to_owned(),
            Style {
                fg: Color::Indexed(1),
                ..Style::default()
            },
        );
        by_capture.insert(
            "operator".to_owned(),
            Style {
                fg: Color::Indexed(6),
                ..Style::default()
            },
        );
        by_capture.insert("variable".to_owned(), Style::default());
        by_capture.insert("punctuation".to_owned(), Style::default());
        Self {
            by_capture,
            default_style: Style::default(),
        }
    }

    /// Resolve a capture name to a [`Style`]. Tries the full name,
    /// then strips one `.`-separated segment at a time, then falls
    /// back to `default_style`.
    #[must_use]
    pub fn lookup(&self, capture_name: &str) -> Style {
        let mut name = capture_name;
        loop {
            if let Some(s) = self.by_capture.get(name) {
                return *s;
            }
            match name.rfind('.') {
                Some(idx) => name = &name[..idx],
                None => return self.default_style,
            }
        }
    }

    /// Set the style for one capture name, replacing any prior entry.
    pub fn insert(&mut self, capture_name: impl Into<String>, style: Style) {
        self.by_capture.insert(capture_name.into(), style);
    }

    /// Forget every capture-style entry. The `default_style` is
    /// preserved.
    pub fn clear(&mut self) {
        self.by_capture.clear();
    }
}

/// Cheaply-cloneable shared theme handle. Held by the
/// [`crate::syntax::SyntaxRegistry`] and by every attached
/// [`SyntaxHighlightView`]; a Lua-driven theme edit is observable
/// to all attached views on the next render.
pub type ThemeHandle = Arc<Mutex<Theme>>;

// ---------------------------------------------------------------------------
// SyntaxHighlightView
// ---------------------------------------------------------------------------

/// Cached per-bundle highlight state. Keyed by the underlying
/// `Arc<ParseTreeBundle>`'s identity (compared via `Arc::ptr_eq`),
/// so a freshly-installed bundle invalidates the cache.
struct HighlightCache {
    /// The bundle the cache was built against. `None` until the
    /// first render observes a settled bundle.
    bundle: Option<Arc<ParseTreeBundle>>,
    /// Compiled spans, sorted wider-first per
    /// [`compute_highlight_spans`].
    spans: Vec<HighlightSpan>,
    /// Per-row first-byte offsets into `bundle.source`. `Vec<u32>`
    /// because pmacs files cap at 4 GiB.
    line_offsets: Vec<u32>,
    /// Capture names indexed by `HighlightSpan::capture_index`.
    /// Populated alongside `spans` so the render path doesn't need
    /// to keep a reference into the [`tree_sitter::Query`].
    capture_names: Arc<[String]>,
}

impl HighlightCache {
    fn empty() -> Self {
        Self {
            bundle: None,
            spans: Vec::new(),
            line_offsets: Vec::new(),
            capture_names: Arc::from(Vec::<String>::new().into_boxed_slice()),
        }
    }
}

/// Tab-stop width in display columns (must match
/// [`crate::text_view`]; both views write into the same cell grid).
const TAB_WIDTH: u32 = 8;

/// View that renders syntax highlighting from a tree-sitter parse
/// tree. Composes over [`crate::text_view::TextView`] per the M2.9
/// view-composition contract --- it never writes glyphs, only merges
/// styles into cells the base view has already painted.
pub struct SyntaxHighlightView {
    parse: ParseViewHandle,
    query: Arc<tree_sitter::Query>,
    theme: ThemeHandle,
    cache: HighlightCache,
}

impl SyntaxHighlightView {
    /// Construct a highlight view over `parse` using `query` and
    /// `theme`. The initial cache is empty; the first render with
    /// a settled bundle populates it.
    #[must_use]
    pub fn new(parse: ParseViewHandle, query: Arc<tree_sitter::Query>, theme: ThemeHandle) -> Self {
        Self {
            parse,
            query,
            theme,
            cache: HighlightCache::empty(),
        }
    }

    /// Test helper: number of cached highlight spans.
    #[must_use]
    pub fn cached_span_count(&self) -> usize {
        self.cache.spans.len()
    }

    /// Refresh `self.cache` if the parse view's current bundle
    /// differs from the cached one. No-op when the bundle pointer
    /// is unchanged --- the steady-state cost between parses.
    fn refresh_cache_if_stale(&mut self) {
        let Some(bundle) = self.parse.current() else {
            return;
        };
        let stale = self
            .cache
            .bundle
            .as_ref()
            .is_none_or(|prev| !Arc::ptr_eq(prev, &bundle));
        if !stale {
            return;
        }
        let spans = compute_highlight_spans(&self.query, &bundle);
        let line_offsets = compute_line_offsets(bundle.source.as_ref());
        let capture_names: Arc<[String]> = self
            .query
            .capture_names()
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        self.cache = HighlightCache {
            bundle: Some(bundle),
            spans,
            line_offsets,
            capture_names,
        };
    }

    /// Look up the style for span `s`, consulting the active theme.
    fn style_for(&self, theme: &Theme, s: HighlightSpan) -> Style {
        let idx = s.capture_index as usize;
        let Some(name) = self.cache.capture_names.get(idx) else {
            return theme.default_style;
        };
        theme.lookup(name)
    }
}

impl View for SyntaxHighlightView {
    fn kind(&self) -> &'static str {
        "syntax-highlight"
    }

    fn render(&mut self, _buf: &Buffer, viewport: Viewport, cells: &mut CellGrid<'_>) {
        self.refresh_cache_if_stale();
        let Some(bundle) = self.cache.bundle.clone() else {
            return;
        };
        if self.cache.spans.is_empty() || self.cache.line_offsets.is_empty() {
            return;
        }
        let source: &[u8] = bundle.source.as_ref();
        let theme = self.theme.lock().expect("theme mutex poisoned").clone();

        let start_line = line_at_offset(&self.cache.line_offsets, viewport.buffer_start as u32);
        let max_rows = viewport.cell_size.rows;
        let max_cols = viewport.cell_size.cols;
        let cell_origin = viewport.cell_origin;
        let total_lines = self.cache.line_offsets.len() as u32;

        for row_offset in 0..max_rows {
            let line_idx = start_line + row_offset;
            if line_idx >= total_lines {
                break;
            }
            let line_start = self.cache.line_offsets[line_idx as usize];
            let line_end = self
                .cache
                .line_offsets
                .get(line_idx as usize + 1)
                .copied()
                .unwrap_or(source.len() as u32);
            // Trim a single trailing newline if any --- the text
            // view doesn't paint it as a glyph either.
            let line_end_no_nl = if line_end > line_start
                && source.get(line_end as usize - 1).copied() == Some(b'\n')
            {
                line_end - 1
            } else {
                line_end
            };
            let line_bytes = &source[line_start as usize..line_end_no_nl as usize];

            // Spans whose start lies on this line. Wider-first
            // ordering means parents apply before children.
            // Spans that *cross* lines apply on every line they
            // touch --- this loop only filters by start, then
            // intersects with the line range below.
            for span in self
                .cache
                .spans
                .iter()
                .filter(|s| s.start_byte < line_end_no_nl && s.end_byte > line_start)
                .copied()
            {
                let style = self.style_for(&theme, span);
                if style == Style::default() {
                    // Nothing to merge --- skip the per-cell loop.
                    continue;
                }
                let s_start = span.start_byte.max(line_start);
                let s_end = span.end_byte.min(line_end_no_nl);
                let byte_col_start = (s_start - line_start) as usize;
                let byte_col_end = (s_end - line_start) as usize;
                let (start_col, end_col) =
                    byte_range_to_display_cols(line_bytes, byte_col_start, byte_col_end);
                if end_col <= start_col {
                    continue;
                }
                let cell_row = cell_origin.row + row_offset;
                let clamped_start = start_col.min(max_cols);
                let clamped_end = end_col.min(max_cols);
                for col in clamped_start..clamped_end {
                    let cell = cells.at(CellCoord::new(cell_row, cell_origin.col + col));
                    cell.style = merge_styles(cell.style, style);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Byte → row/col helpers
// ---------------------------------------------------------------------------

/// Build a sorted vector of line-start byte offsets for `source`.
/// `out[0] == 0` always; `out.len() == number of lines`. A trailing
/// newline produces one extra empty line, matching
/// [`crate::text_view::TextView`]'s convention.
fn compute_line_offsets(source: &[u8]) -> Vec<u32> {
    let mut out = Vec::with_capacity(source.len() / 32 + 1);
    out.push(0);
    for (i, b) in source.iter().enumerate() {
        if *b == b'\n' {
            out.push(i as u32 + 1);
        }
    }
    out
}

/// Index of the line containing byte `offset`.
fn line_at_offset(line_offsets: &[u32], offset: u32) -> u32 {
    match line_offsets.binary_search(&offset) {
        Ok(i) => i as u32,
        Err(i) => i.saturating_sub(1) as u32,
    }
}

/// Convert a half-open byte-column range `[byte_start, byte_end)`
/// inside `line_bytes` to a display-column range. UTF-8 aware; tabs
/// expand to the next [`TAB_WIDTH`]-aligned column. Bytes that don't
/// form complete codepoints (because the byte range falls inside a
/// multi-byte char) are skipped, matching
/// [`crate::text_view::TextView::pos_to_display`]'s conservative
/// rounding.
fn byte_range_to_display_cols(line_bytes: &[u8], byte_start: usize, byte_end: usize) -> (u32, u32) {
    let bs = byte_start.min(line_bytes.len());
    let be = byte_end.min(line_bytes.len());
    let display_to = |upto: usize| -> u32 {
        // Drop trailing bytes that don't form complete codepoints.
        let mut take = upto.min(line_bytes.len());
        while take > 0 && std::str::from_utf8(&line_bytes[..take]).is_err() {
            take -= 1;
        }
        let s = std::str::from_utf8(&line_bytes[..take]).unwrap_or("");
        let mut col: u32 = 0;
        for ch in s.chars() {
            col += char_display_width(ch, col);
        }
        col
    };
    (display_to(bs), display_to(be))
}

fn char_display_width(ch: char, current_col: u32) -> u32 {
    if ch == '\t' {
        TAB_WIDTH - (current_col % TAB_WIDTH)
    } else {
        UnicodeWidthChar::width(ch).unwrap_or(0) as u32
    }
}

// ---------------------------------------------------------------------------
// Style equality helper
// ---------------------------------------------------------------------------

/// True iff `style` is the zero/default style. The style-merge
/// short-circuits in [`SyntaxHighlightView::render`] when this
/// holds, since merging a default style is a no-op.
#[must_use]
pub fn is_default_style(style: Style) -> bool {
    style.fg == Color::Default
        && style.bg == Color::Default
        && !style.bold
        && !style.italic
        && style.underline == UnderlineStyle::None
        && !style.reverse
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_lookup_walks_dotted_prefixes() {
        let mut t = Theme::empty();
        t.insert(
            "function",
            Style {
                bold: true,
                ..Style::default()
            },
        );
        // Exact match.
        assert!(t.lookup("function").bold);
        // One-segment fallback: function.method falls back to function.
        assert!(t.lookup("function.method").bold);
        // No match --- returns default.
        assert!(!t.lookup("variable").bold);
    }

    #[test]
    fn theme_lookup_specific_overrides_general() {
        let mut t = Theme::empty();
        t.insert(
            "function",
            Style {
                bold: true,
                ..Style::default()
            },
        );
        t.insert(
            "function.method",
            Style {
                italic: true,
                ..Style::default()
            },
        );
        // function.method matches its own entry, not the function fallback.
        let s = t.lookup("function.method");
        assert!(s.italic);
        assert!(!s.bold);
    }

    #[test]
    fn line_offsets_basic() {
        let src = b"a\nbb\nccc";
        let off = compute_line_offsets(src);
        assert_eq!(off, vec![0, 2, 5]);
        assert_eq!(line_at_offset(&off, 0), 0);
        assert_eq!(line_at_offset(&off, 1), 0);
        assert_eq!(line_at_offset(&off, 2), 1);
        assert_eq!(line_at_offset(&off, 5), 2);
        assert_eq!(line_at_offset(&off, 7), 2);
    }

    #[test]
    fn line_offsets_trailing_newline_creates_empty_line() {
        let src = b"a\nb\n";
        let off = compute_line_offsets(src);
        // Three lines: "a", "b", "".
        assert_eq!(off, vec![0, 2, 4]);
    }

    #[test]
    fn byte_range_display_cols_ascii_round_trips() {
        let line = b"hello world";
        // "hello" → cols 0..5
        assert_eq!(byte_range_to_display_cols(line, 0, 5), (0, 5));
        // "world" → cols 6..11
        assert_eq!(byte_range_to_display_cols(line, 6, 11), (6, 11));
    }

    #[test]
    fn byte_range_display_cols_tabs_expand() {
        let line = b"\tx";
        // The full line: tab (0..8) + 'x' (8..9). Byte cols 0..2
        // map to display cols 0..9.
        assert_eq!(byte_range_to_display_cols(line, 0, 2), (0, 9));
        // Just the tab.
        assert_eq!(byte_range_to_display_cols(line, 0, 1), (0, 8));
    }

    #[test]
    fn byte_range_display_cols_clamps_past_end() {
        let line = b"hi";
        assert_eq!(byte_range_to_display_cols(line, 0, 999), (0, 2));
    }

    #[test]
    fn is_default_style_round_trips() {
        assert!(is_default_style(Style::default()));
        assert!(!is_default_style(Style {
            bold: true,
            ..Style::default()
        }));
    }
}
