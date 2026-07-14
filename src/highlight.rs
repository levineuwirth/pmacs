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
use crate::lsp::SharedLspManager;
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
    /// Direct map from capture name → style. T M4.3. Names matching
    /// [`is_face_name`] (`ui` / `ui.*`) are UI faces (themes arc
    /// Q#TH2), reserved by convention — no tree-sitter capture or LSP
    /// token type uses them.
    pub by_capture: HashMap<String, Style>,
    /// Fallback style when no capture matches. Defaults to the
    /// terminal default colors (no override) so unhighlighted text
    /// looks identical to plain rendering.
    pub default_style: Style,
    /// Monotonic syntax-mutation counter (themes arc Q#TH6). Bumped
    /// by every successful Lua mutation that commits a non-face key
    /// (or touches `default_style`); keys the `StyleGate` and the
    /// minimap summary so a mid-session recolor re-ships spans.
    /// INVARIANT: only ever incremented — a wholesale `set` must
    /// replace `by_capture`, never the whole `Theme`, or consecutive
    /// mutations share an epoch and become invisible to every gate.
    pub syntax_epoch: u64,
    /// Monotonic face-mutation counter (themes arc Q#TH6). Bumped by
    /// every successful Lua mutation that commits a face key
    /// ([`is_face_name`]); keys the `ThemeFacts` producer and the
    /// minimap summary (`ui.diag.*` feeds its marks). Same
    /// increment-only invariant as `syntax_epoch`.
    pub face_epoch: u64,
}

/// Themes arc Q#TH2: the face predicate. A theme key names a UI face
/// iff it is exactly `ui` (the deliberate inheritance catch-all —
/// [`Theme::face`]'s walk terminal) or starts with `ui.`. Shared by
/// the namespace reservation, the mutation-counter classification,
/// and the `ThemeFacts` producer's key filter.
#[must_use]
pub fn is_face_name(name: &str) -> bool {
    name == "ui" || name.starts_with("ui.")
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
    /// tree-sitter captures **and** the LSP semantic-token type names
    /// clangd / rust-analyzer / gopls actually emit, so opening a code
    /// file produces visible highlighting without a user theme. Uses
    /// 8/16-color indexed terminal colors for portability — truecolor
    /// themes can be pushed in from Lua.
    ///
    /// Themes that want more granularity layer on top via
    /// [`Self::insert`] / Lua's `pmacs.theme.set`. The dotted-prefix
    /// [`Self::lookup`] means a modifier-refined name like
    /// `function.defaultLibrary` falls back to `function` if not
    /// defined — safe to query unconditionally.
    #[must_use]
    pub fn default_dark() -> Self {
        // Indexed terminal palette: 1 red, 2 green, 3 yellow, 4 blue,
        // 5 magenta, 6 cyan, 8 bright black/gray; bright variants 9..14.
        let fg = |c: u8| Style {
            fg: Color::Indexed(c),
            ..Style::default()
        };
        let fg_bold = |c: u8| Style {
            fg: Color::Indexed(c),
            bold: true,
            ..Style::default()
        };
        let fg_italic = |c: u8| Style {
            fg: Color::Indexed(c),
            italic: true,
            ..Style::default()
        };
        let italic_only = Style {
            italic: true,
            ..Style::default()
        };

        // (capture name, style). LSP semantic-token type names are the
        // unprefixed entries (`macro`, `namespace`, `parameter`, …);
        // tree-sitter captures are the dotted ones (`keyword.control`,
        // `function.method`, `type.builtin`, `constant.builtin`).
        let entries: &[(&str, Style)] = &[
            ("keyword", fg_bold(5)),
            ("keyword.control", fg_bold(13)),
            ("function", fg(4)),
            ("function.method", fg(12)),
            ("type", fg(3)),
            ("type.builtin", fg(11)),
            ("string", fg(2)),
            ("comment", fg_italic(8)),
            ("constant", fg(1)),
            ("constant.builtin", fg(9)),
            ("number", fg(1)),
            ("operator", fg(6)),
            ("variable", Style::default()),
            ("punctuation", Style::default()),
            // LSP additions (no tree-sitter overlap).
            ("macro", fg_bold(13)),
            ("namespace", fg(11)),
            ("parameter", italic_only),
            ("property", fg(6)),
            ("class", fg(3)),
            ("struct", fg(3)),
            ("enum", fg(3)),
            ("interface", fg(3)),
            ("enumMember", fg(1)),
            ("modifier", fg_bold(5)),
            ("decorator", fg(13)),
            ("regexp", fg(2)),
            ("typeParameter", fg_italic(11)),
        ];
        let by_capture = entries
            .iter()
            .map(|(name, style)| ((*name).to_owned(), *style))
            .collect();
        Self {
            by_capture,
            default_style: Style::default(),
            syntax_epoch: 0,
            face_epoch: 0,
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

    /// Resolve a UI face name to its style, or `None` when unset
    /// (themes arc Q#TH4). Same dotted-prefix walk as [`Self::lookup`]
    /// — so `ui.search.match.active` falls back to `ui.search.match`,
    /// the `ui.diag.*` children to `ui.diag`, and everything to the
    /// bare-`ui` catch-all — but the walk returns `None` instead of
    /// falling back to `default_style`: an unset face must leave the
    /// paint site's hardcoded default untouched, and a user's
    /// `pmacs.theme.default` (a *syntax* fallback) must never bleed
    /// into chrome. An exact entry stops the walk, so an explicitly
    /// empty child (e.g. `ui.diag.error = {}`) blocks inheritance
    /// from a themed parent. Callers pass full face names only.
    #[must_use]
    pub fn face(&self, name: &str) -> Option<Style> {
        debug_assert!(is_face_name(name), "face() takes ui/ui.* names");
        let mut name = name;
        loop {
            if let Some(s) = self.by_capture.get(name) {
                return Some(*s);
            }
            match name.rfind('.') {
                Some(idx) => name = &name[..idx],
                None => return None,
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
// LspStyleView
// ---------------------------------------------------------------------------

/// View that paints LSP semantic-token styling into cells, layered
/// **alongside** [`SyntaxHighlightView`] when both exist for a
/// buffer. The grid TUI's `View::render` composition stack means
/// tree-sitter paints first (lexical: keywords, strings, operators,
/// the things a parser identifies from the source's *shape*) and
/// `LspStyleView` paints after (semantic: function vs variable,
/// macro vs constant, the things a language server identifies from
/// program *meaning*). Their styles compose through
/// [`crate::overlay::merge_styles`], so the final cell carries both
/// authorities' contributions — the `VSCode` / `Zed` "`TextMate` +
/// LSP semantic tokens" model, on a terminal grid.
///
/// For languages with no bundled tree-sitter grammar (Python, Go,
/// languages added later via LSP only), `LspStyleView` is the *sole*
/// styling source and still works — the merge with an absent
/// tree-sitter overlay is identity.
///
/// `M_B3` (this) dropped the earlier policy-A exclusivity gate in
/// `lsp.lua` that restricted attachment to grammar-less buffers. The
/// previous gate left grammar-backed languages (Rust, C, C++) without
/// LSP semantic refinement; dual-authority delivers richer coloring
/// without conflict.
///
/// Holds shared handles only (`SharedLspManager` + `ThemeHandle`), so
/// it survives buffer renames and LSP server restarts: every `render`
/// re-derives the buffer's URI from `buf.file_path()` and re-queries
/// the per-server encoding + legend via
/// `LspManager::semantic_style_context`. If any of those are absent
/// the render is a silent no-op — never spam, never panic.
pub struct LspStyleView {
    lsp: SharedLspManager,
    theme: ThemeHandle,
}

impl LspStyleView {
    /// Construct a view that paints LSP semantic-token styling using
    /// `lsp` as the source of truth and `theme` as the capture-name →
    /// style map. The view shares both handles; updates from elsewhere
    /// (a new LSP response, a theme edit) are observable on the next
    /// render.
    #[must_use]
    pub fn new(lsp: SharedLspManager, theme: ThemeHandle) -> Self {
        Self { lsp, theme }
    }
}

impl View for LspStyleView {
    fn kind(&self) -> &'static str {
        "lsp-style"
    }

    fn render(&mut self, buf: &Buffer, viewport: Viewport, cells: &mut CellGrid<'_>) {
        let Some(path) = buf.file_path() else {
            return; // No path ⇒ no URI ⇒ nothing to look up.
        };
        let uri = crate::lsp::path_to_file_uri(path);

        // Pull encoding + legend, and clone the token vec so we can
        // drop both the manager borrow and the store lock before
        // walking the viewport (cheap: tokens are small structs).
        let (ctx, tokens) = {
            let mgr = self.lsp.borrow();
            let Some(ctx) = mgr.semantic_style_context(&uri) else {
                return; // No server has tokens for this URI.
            };
            let store = mgr.semantic_token_store();
            let guard = store.lock().expect("semantic-token store mutex poisoned");
            if guard.is_stale(&uri) {
                return;
            }
            let Some((_, resp)) = guard.for_uri(&uri) else {
                return;
            };
            (ctx, resp.tokens.clone())
        };
        if tokens.is_empty() {
            return;
        }
        let theme = self.theme.lock().expect("theme mutex poisoned").clone();

        // Buffer source bytes (cheap rope slice, mirrors
        // `semantic_render::buffer_source_bytes`).
        let source = {
            let len = buf.len();
            let mut bytes = vec![0u8; len as usize];
            if !bytes.is_empty() {
                buf.snapshot_rope().slice(0, len, &mut bytes);
            }
            bytes
        };
        let line_offsets = compute_line_offsets(&source);
        if line_offsets.is_empty() {
            return;
        }

        let start_line = line_at_offset(&line_offsets, viewport.buffer_start as u32);
        let max_rows = viewport.cell_size.rows;
        let max_cols = viewport.cell_size.cols;
        let cell_origin = viewport.cell_origin;
        let total_lines = line_offsets.len() as u32;

        for row_offset in 0..max_rows {
            let line_idx = start_line + row_offset;
            if line_idx >= total_lines {
                break;
            }
            let line_start = line_offsets[line_idx as usize];
            let line_end = line_offsets
                .get(line_idx as usize + 1)
                .copied()
                .unwrap_or(source.len() as u32);
            let line_end_no_nl = if line_end > line_start
                && source.get(line_end as usize - 1).copied() == Some(b'\n')
            {
                line_end - 1
            } else {
                line_end
            };
            let line_bytes = &source[line_start as usize..line_end_no_nl as usize];
            let Ok(line_str) = std::str::from_utf8(line_bytes) else {
                continue; // Non-UTF-8 line ⇒ skip encoding conversion.
            };

            // O(tokens × visible_lines) — fine for the typical
            // semantic-token set; if it ever becomes the bottleneck,
            // pre-bucket tokens by line at the top of `render`.
            for t in tokens.iter().filter(|t| t.line == line_idx) {
                let Some(legend) = ctx.legend.as_ref() else {
                    continue; // No legend ⇒ cannot name a style.
                };
                let Some(name) = legend.type_name(t.token_type) else {
                    continue; // Unknown type index.
                };
                // Build the lookup name as `<type>.<first-modifier>`
                // when modifiers are set, else just `<type>`. The
                // theme's dotted-prefix `lookup` walks back to the
                // base if a more specific entry isn't defined, so
                // adding a modifier suffix is a strict refinement —
                // never worse than the unmodified lookup. Allocation
                // is skipped in the no-modifier case (the common one)
                // via `Cow::Borrowed`.
                let mods = legend.modifier_names(t.token_modifiers);
                let lookup_name: std::borrow::Cow<'_, str> = match mods.first() {
                    Some(m) => std::borrow::Cow::Owned(format!("{name}.{m}")),
                    None => std::borrow::Cow::Borrowed(name),
                };
                let style = theme.lookup(&lookup_name);
                if is_default_style(style) {
                    continue;
                }
                let start_b = crate::lsp::char_to_byte(line_str, t.start, ctx.encoding);
                let end_char = t.start.saturating_add(t.length);
                let end_b = crate::lsp::char_to_byte(line_str, end_char, ctx.encoding);
                if end_b <= start_b {
                    continue;
                }
                let (start_col, end_col) = byte_range_to_display_cols(line_bytes, start_b, end_b);
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
    fn face_returns_none_when_unset_never_default_style() {
        // Q#TH4: an unset face leaves the paint site's hardcoded
        // default untouched — even a loud user default_style (a
        // SYNTAX fallback) must not bleed into chrome.
        let mut t = Theme::empty();
        t.default_style = Style {
            bold: true,
            ..Style::default()
        };
        assert_eq!(t.face("ui.modeline"), None);
        assert_eq!(t.face("ui"), None);
        // lookup, by contrast, resolves through to default_style.
        assert!(t.lookup("ui.modeline").bold);
    }

    #[test]
    fn face_walks_dotted_prefixes_to_the_ui_catch_all() {
        let mut t = Theme::empty();
        t.insert(
            "ui",
            Style {
                italic: true,
                ..Style::default()
            },
        );
        t.insert(
            "ui.search.match",
            Style {
                bold: true,
                ..Style::default()
            },
        );
        // Exact match.
        assert!(t.face("ui.search.match").expect("set").bold);
        // One-segment fallback: active inherits from ui.search.match.
        assert!(t.face("ui.search.match.active").expect("inherit").bold);
        // Everything else falls to the bare-ui catch-all.
        assert!(t.face("ui.modeline").expect("catch-all").italic);
        assert!(t.face("ui.diag.error").expect("catch-all").italic);
    }

    #[test]
    fn face_exact_empty_child_blocks_parent_inheritance() {
        // Q#TH5 (round 3 finding 4): with a themed ui.diag parent, an
        // explicitly empty ui.diag.error child stops the walk at the
        // exact entry — errors reset to the built-in (the Default fg
        // policy applies at the consumer) while siblings inherit.
        let mut t = Theme::empty();
        t.insert(
            "ui.diag",
            Style {
                fg: Color::Indexed(93),
                ..Style::default()
            },
        );
        t.insert("ui.diag.error", Style::default());
        assert_eq!(t.face("ui.diag.error"), Some(Style::default()));
        assert_eq!(
            t.face("ui.diag.warning").expect("inherits").fg,
            Color::Indexed(93)
        );
    }

    #[test]
    fn face_predicate_accepts_ui_root_and_prefix_only() {
        // Q#TH2: exactly `ui` or `ui.`-prefixed — nothing else. A
        // name like `uix` must classify as syntax, not face.
        assert!(is_face_name("ui"));
        assert!(is_face_name("ui.modeline"));
        assert!(is_face_name("ui.search.match.active"));
        assert!(!is_face_name("uix"));
        assert!(!is_face_name("u"));
        assert!(!is_face_name("keyword"));
        assert!(!is_face_name("gui.modeline"));
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
    fn default_dark_covers_lsp_token_types() {
        // The LSP semantic-token type names clangd / rust-analyzer /
        // gopls actually emit must resolve to a non-default style in
        // the built-in theme — otherwise `LspStyleView` drops them as
        // "default-styled, nothing to render". Regression guard so a
        // future theme refactor doesn't silently lose coverage.
        let t = Theme::default_dark();
        for name in [
            "macro",
            "namespace",
            "parameter",
            "property",
            "class",
            "struct",
            "enum",
            "interface",
            "enumMember",
            "modifier",
            "decorator",
            "regexp",
            "typeParameter",
        ] {
            assert!(
                !is_default_style(t.lookup(name)),
                "default_dark must color LSP token type `{name}`"
            );
        }
    }

    #[test]
    fn is_default_style_round_trips() {
        assert!(is_default_style(Style::default()));
        assert!(!is_default_style(Style {
            bold: true,
            ..Style::default()
        }));
    }

    // --- M_B1: LspStyleView render ---

    #[test]
    fn lsp_style_view_paints_cells_from_semantic_tokens() {
        use crate::cell::{Cell, CellSize};
        use crate::editor::EditorState;
        use crate::lsp::PositionEncoding;
        use crate::semantic_tokens::{SemanticToken, SemanticTokenKey, SemanticTokensResponse};

        let state = EditorState::new();
        let buffer_id = state.core.borrow().active_window().buffer_id;

        // Seed the buffer: one line, `.cpp` path. The test renders
        // `LspStyleView` directly (no `SyntaxHighlightView` attached
        // alongside in this fixture), so the asserted cells reflect
        // the LSP authority alone — M_B3's dual-authority composition
        // is exercised end-to-end by Lua's `attach_buffer`, not here.
        {
            let mut core = state.core.borrow_mut();
            core.registry
                .clone()
                .borrow_mut()
                .get_mut(buffer_id)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"int main\n",
                })
                .expect("seed");
            core.set_buffer_path(buffer_id, Some(std::path::PathBuf::from("/tmp/x.cpp")));
        }
        // Theme face for the legend's lone type name.
        state.syntax_registry.theme().lock().expect("theme").insert(
            "kw",
            Style {
                bold: true,
                ..Style::default()
            },
        );
        // Register an Initialized fake LSP client (no process) with a
        // legend that maps token_type 0 → "kw", and UTF-16 encoding.
        let sid = state
            .lsp_manager
            .borrow_mut()
            .insert_initialized_test_client(
                serde_json::json!({
                    "semanticTokensProvider": {
                        "legend": { "tokenTypes": ["kw"], "tokenModifiers": [] }
                    }
                }),
                PositionEncoding::Utf16,
            );
        // Seed a single token at line 0, cols [0, 3) — covers "int".
        // (Buffer's normalized path drives the URI; encode it the same
        // way the producer does so the store lookup matches.)
        let active_path = state
            .core
            .borrow()
            .active_buffer_path()
            .expect("path set above");
        let uri = crate::lsp::path_to_file_uri(&active_path);
        {
            let mgr = state.lsp_manager.borrow();
            let store = mgr.semantic_token_store();
            store.lock().expect("store").set(
                SemanticTokenKey::new(sid.raw().to_string(), uri),
                SemanticTokensResponse {
                    tokens: vec![SemanticToken {
                        line: 0,
                        start: 0,
                        length: 3,
                        token_type: 0,
                        token_modifiers: 0,
                    }],
                    result_id: None,
                    raw: Vec::new(),
                },
            );
        }
        // Render into a small grid.
        let mut view = LspStyleView::new(state.lsp_manager.clone(), state.syntax_registry.theme());
        let mut backing: Vec<Cell> = vec![Cell::default(); 20];
        let mut grid = CellGrid {
            cells: &mut backing,
            stride: 20,
            size: CellSize::new(1, 20),
        };
        let viewport = Viewport {
            buffer_start: 0,
            buffer_end: u64::MAX,
            cell_origin: CellCoord::new(0, 0),
            cell_size: CellSize::new(1, 20),
            gutter_w: 0,
        };
        let registry = state.core.borrow().registry.clone();
        let reg = registry.borrow();
        let buf = reg.get(buffer_id).expect("buffer");
        view.render(buf, viewport, &mut grid);

        // Cells [0,3) painted bold (the seeded theme face); cell 3
        // (the space after "int") unchanged.
        for col in 0..3 {
            assert!(
                grid.get(CellCoord::new(0, col)).style.bold,
                "col {col} should be styled by the LSP token"
            );
        }
        assert!(
            !grid.get(CellCoord::new(0, 3)).style.bold,
            "col 3 (space) is outside the token range"
        );
    }

    #[test]
    fn lsp_style_view_suppresses_stale_semantic_tokens() {
        use crate::cell::{Cell, CellSize};
        use crate::editor::EditorState;
        use crate::lsp::PositionEncoding;
        use crate::semantic_tokens::{SemanticToken, SemanticTokenKey, SemanticTokensResponse};

        let state = EditorState::new();
        let buffer_id = state.core.borrow().active_window().buffer_id;
        {
            let mut core = state.core.borrow_mut();
            core.registry
                .clone()
                .borrow_mut()
                .get_mut(buffer_id)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"foo\n",
                })
                .expect("seed");
            core.set_buffer_path(buffer_id, Some(std::path::PathBuf::from("/tmp/x.c")));
        }
        state.syntax_registry.theme().lock().expect("theme").insert(
            "function",
            Style {
                bold: true,
                ..Style::default()
            },
        );
        let sid = state
            .lsp_manager
            .borrow_mut()
            .insert_initialized_test_client(
                serde_json::json!({
                    "semanticTokensProvider": {
                        "legend": { "tokenTypes": ["function"], "tokenModifiers": [] }
                    }
                }),
                PositionEncoding::Utf16,
            );
        let active_path = state.core.borrow().active_buffer_path().expect("path set");
        let uri = crate::lsp::path_to_file_uri(&active_path);
        {
            let mgr = state.lsp_manager.borrow();
            let store = mgr.semantic_token_store();
            let mut guard = store.lock().expect("store");
            guard.set(
                SemanticTokenKey::new(sid.raw().to_string(), uri.clone()),
                SemanticTokensResponse {
                    tokens: vec![SemanticToken {
                        line: 0,
                        start: 0,
                        length: 3,
                        token_type: 0,
                        token_modifiers: 0,
                    }],
                    result_id: None,
                    raw: Vec::new(),
                },
            );
            guard.mark_stale(uri);
        }

        let mut view = LspStyleView::new(state.lsp_manager.clone(), state.syntax_registry.theme());
        let mut backing: Vec<Cell> = vec![Cell::default(); 20];
        let mut grid = CellGrid {
            cells: &mut backing,
            stride: 20,
            size: CellSize::new(1, 20),
        };
        let viewport = Viewport {
            buffer_start: 0,
            buffer_end: u64::MAX,
            cell_origin: CellCoord::new(0, 0),
            cell_size: CellSize::new(1, 20),
            gutter_w: 0,
        };
        let registry = state.core.borrow().registry.clone();
        let reg = registry.borrow();
        let buf = reg.get(buffer_id).expect("buffer");
        view.render(buf, viewport, &mut grid);

        assert!(
            !grid.get(CellCoord::new(0, 0)).style.bold,
            "stale semantic tokens must not paint over current syntax/text"
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)] // Scripted fixture; the headline
    // test next door is similarly
    // shaped. If a third lands, extract
    // a shared helper.
    fn lsp_style_view_uses_modifier_in_capture_lookup() {
        use crate::cell::{Cell, CellSize};
        use crate::editor::EditorState;
        use crate::lsp::PositionEncoding;
        use crate::semantic_tokens::{SemanticToken, SemanticTokenKey, SemanticTokensResponse};

        let state = EditorState::new();
        let buffer_id = state.core.borrow().active_window().buffer_id;
        {
            let mut core = state.core.borrow_mut();
            core.registry
                .clone()
                .borrow_mut()
                .get_mut(buffer_id)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"foo bar\n",
                })
                .expect("seed");
            core.set_buffer_path(buffer_id, Some(std::path::PathBuf::from("/tmp/x.cpp")));
        }
        // Theme: base `function` is bold; the modifier-refined
        // `function.defaultLibrary` is italic (overrides bold).
        {
            let theme_handle = state.syntax_registry.theme();
            let mut theme = theme_handle.lock().expect("theme");
            theme.insert(
                "function",
                Style {
                    bold: true,
                    ..Style::default()
                },
            );
            theme.insert(
                "function.defaultLibrary",
                Style {
                    italic: true,
                    ..Style::default()
                },
            );
        }
        // Legend: tokenTypes=["function"], tokenModifiers=["defaultLibrary"].
        // Token 0: type=0, modifiers=0 (no bit set) ⇒ falls back to `function` (bold).
        // Token 1: type=0, modifiers=1 (bit 0 ⇒ defaultLibrary) ⇒ matches
        // `function.defaultLibrary` (italic, NOT bold).
        let sid = state
            .lsp_manager
            .borrow_mut()
            .insert_initialized_test_client(
                serde_json::json!({
                    "semanticTokensProvider": {
                        "legend": {
                            "tokenTypes": ["function"],
                            "tokenModifiers": ["defaultLibrary"]
                        }
                    }
                }),
                PositionEncoding::Utf16,
            );
        let active_path = state.core.borrow().active_buffer_path().expect("path set");
        let uri = crate::lsp::path_to_file_uri(&active_path);
        {
            let mgr = state.lsp_manager.borrow();
            let store = mgr.semantic_token_store();
            store.lock().expect("store").set(
                SemanticTokenKey::new(sid.raw().to_string(), uri),
                SemanticTokensResponse {
                    tokens: vec![
                        SemanticToken {
                            line: 0,
                            start: 0,
                            length: 3,
                            token_type: 0,
                            token_modifiers: 0,
                        },
                        SemanticToken {
                            line: 0,
                            start: 4,
                            length: 3,
                            token_type: 0,
                            token_modifiers: 1,
                        },
                    ],
                    result_id: None,
                    raw: Vec::new(),
                },
            );
        }
        let mut view = LspStyleView::new(state.lsp_manager.clone(), state.syntax_registry.theme());
        let mut backing: Vec<Cell> = vec![Cell::default(); 20];
        let mut grid = CellGrid {
            cells: &mut backing,
            stride: 20,
            size: CellSize::new(1, 20),
        };
        let viewport = Viewport {
            buffer_start: 0,
            buffer_end: u64::MAX,
            cell_origin: CellCoord::new(0, 0),
            cell_size: CellSize::new(1, 20),
            gutter_w: 0,
        };
        let registry = state.core.borrow().registry.clone();
        let reg = registry.borrow();
        let buf = reg.get(buffer_id).expect("buffer");
        view.render(buf, viewport, &mut grid);

        // "foo" (cols 0..3) ⇒ base `function` ⇒ bold.
        let c = grid.get(CellCoord::new(0, 0));
        assert!(c.style.bold, "unmodified token uses base `function`");
        assert!(!c.style.italic);
        // "bar" (cols 4..7) ⇒ `function.defaultLibrary` ⇒ italic, no bold.
        let c = grid.get(CellCoord::new(0, 4));
        assert!(
            c.style.italic,
            "modifier-refined token uses `function.defaultLibrary`"
        );
        assert!(!c.style.bold);
    }
}
