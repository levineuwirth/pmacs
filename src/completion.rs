// completion.rs --- T M4.7 LSP-backed completion: store + popup view.

//! Completion state and the [`CompletionView`] that renders it as a
//! popup.
//!
//! Per spec §M4.7: when the user types a completion trigger character
//! (or invokes completion explicitly), pmacs sends a
//! `textDocument/completion` request to the LSP. The response arrives
//! later --- the [`CompletionStore`] keeps the most-recent items per
//! server, and [`CompletionView`] renders them as a list popup.
//!
//! # Why a separate module
//!
//! Mirrors [`crate::diag`]: a shared store with a writer (the LSP
//! manager) and many readers (every active popup view). The
//! `Send` bound on `View` forces `Arc<Mutex<...>>`.
//!
//! # Trigger characters
//!
//! [`CompletionTriggers::should_fire`] inspects a character against
//! the trigger set negotiated during `initialize` (e.g. `.` for
//! method access, `::` for paths, `<` for generics). Identifier
//! characters are *not* triggers --- those are explicit-invocation
//! only --- so the editor decides whether to fire on identifier typing
//! based on its own UI policy.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use unicode_width::UnicodeWidthChar;

use crate::buffer::{Buffer, BufferId};
use crate::cell::{CellCoord, CellGrid, Color, Glyph, Style};
use crate::rope::Position;
use crate::view::{View, Viewport};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// LSP `CompletionItemKind`. Mirrors the LSP enum with the same
/// numeric values; `Text` is the safe fallback for unknown numbers
/// (the LSP spec says clients must accept extended kinds gracefully).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionItemKind {
    /// Plain text snippet.
    Text = 1,
    /// Method on an object.
    Method = 2,
    /// Free function.
    Function = 3,
    /// Constructor / factory.
    Constructor = 4,
    /// Field on a struct.
    Field = 5,
    /// Local or global variable.
    Variable = 6,
    /// Class / type.
    Class = 7,
    /// Interface / trait.
    Interface = 8,
    /// Module / namespace.
    Module = 9,
    /// Property accessor.
    Property = 10,
    /// Numeric / language unit.
    Unit = 11,
    /// Constant value literal.
    Value = 12,
    /// Enum.
    Enum = 13,
    /// Keyword.
    Keyword = 14,
    /// Snippet template.
    Snippet = 15,
    /// Color literal.
    Color = 16,
    /// File path.
    File = 17,
    /// Cross-reference.
    Reference = 18,
    /// Folder path.
    Folder = 19,
    /// Enum member.
    EnumMember = 20,
    /// Constant binding.
    Constant = 21,
    /// Struct.
    Struct = 22,
    /// Event.
    Event = 23,
    /// Operator.
    Operator = 24,
    /// Type parameter.
    TypeParameter = 25,
}

impl CompletionItemKind {
    /// Short single-letter glyph for the popup's left gutter.
    #[must_use]
    pub fn glyph(self) -> char {
        match self {
            Self::Method | Self::Function | Self::Constructor => 'f',
            Self::Field | Self::Property => 'p',
            Self::Variable | Self::Constant => 'v',
            Self::Class | Self::Struct => 'C',
            Self::Interface => 'I',
            Self::Module => 'M',
            Self::Enum | Self::EnumMember => 'E',
            Self::Keyword => 'k',
            Self::Snippet => 's',
            Self::TypeParameter => 't',
            Self::File | Self::Folder => '/',
            _ => '.',
        }
    }

    #[allow(
        clippy::match_same_arms,
        reason = "kept explicit code 1 arm for documentation; Text is also the safe fallback"
    )]
    fn from_lsp_value(v: Option<&Value>) -> Self {
        match v.and_then(Value::as_i64) {
            Some(1) => Self::Text,
            Some(2) => Self::Method,
            Some(3) => Self::Function,
            Some(4) => Self::Constructor,
            Some(5) => Self::Field,
            Some(6) => Self::Variable,
            Some(7) => Self::Class,
            Some(8) => Self::Interface,
            Some(9) => Self::Module,
            Some(10) => Self::Property,
            Some(11) => Self::Unit,
            Some(12) => Self::Value,
            Some(13) => Self::Enum,
            Some(14) => Self::Keyword,
            Some(15) => Self::Snippet,
            Some(16) => Self::Color,
            Some(17) => Self::File,
            Some(18) => Self::Reference,
            Some(19) => Self::Folder,
            Some(20) => Self::EnumMember,
            Some(21) => Self::Constant,
            Some(22) => Self::Struct,
            Some(23) => Self::Event,
            Some(24) => Self::Operator,
            Some(25) => Self::TypeParameter,
            _ => Self::Text,
        }
    }
}

/// One completion candidate, parsed from `textDocument/completion`.
#[derive(Clone, Debug)]
pub struct CompletionItem {
    /// Display label (always present).
    pub label: String,
    /// Item kind.
    pub kind: CompletionItemKind,
    /// Optional one-line detail (e.g. a function signature).
    pub detail: Option<String>,
    /// Optional documentation; we collapse `MarkupContent` to its
    /// text body and store a plain string.
    pub documentation: Option<String>,
    /// Text inserted on accept; falls back to `label` if absent.
    pub insert_text: Option<String>,
    /// `sortText` LSP field; if absent the receiver may sort by `label`.
    pub sort_text: Option<String>,
    /// `filterText` LSP field; receiver may filter by typed prefix.
    pub filter_text: Option<String>,
}

impl CompletionItem {
    /// Parse a single LSP `CompletionItem` JSON object.
    #[must_use]
    pub fn from_lsp_value(v: &Value) -> Option<Self> {
        let label = v.get("label")?.as_str()?.to_owned();
        let kind = CompletionItemKind::from_lsp_value(v.get("kind"));
        let detail = v.get("detail").and_then(Value::as_str).map(str::to_owned);
        let documentation = v.get("documentation").and_then(extract_markup_text);
        let insert_text = v
            .get("insertText")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let sort_text = v.get("sortText").and_then(Value::as_str).map(str::to_owned);
        let filter_text = v
            .get("filterText")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Some(Self {
            label,
            kind,
            detail,
            documentation,
            insert_text,
            sort_text,
            filter_text,
        })
    }

    /// What gets typed into the buffer when the user accepts this
    /// item. Falls back to `label` per the LSP spec.
    #[must_use]
    pub fn effective_insert_text(&self) -> &str {
        self.insert_text.as_deref().unwrap_or(&self.label)
    }
}

/// Decode an LSP `MarkupContent` or legacy `MarkedString` into
/// plain text. Both forms surface as the same in pmacs (we don't
/// render markdown today).
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

/// Parsed `textDocument/completion` response. Either the
/// `CompletionList` shape or the bare `CompletionItem[]` shape.
#[derive(Clone, Debug, Default)]
pub struct CompletionResponse {
    /// All items in the response.
    pub items: Vec<CompletionItem>,
    /// `isIncomplete` from the response (false if the server returned
    /// a bare item list).
    pub is_incomplete: bool,
}

impl CompletionResponse {
    /// Parse the LSP response for a `textDocument/completion` request.
    /// Accepts both the `CompletionList` and `CompletionItem[]` shapes.
    /// Returns an empty response (`items` empty, `is_incomplete` false)
    /// for `null` or `Value::Null`.
    #[must_use]
    pub fn from_lsp_value(v: &Value) -> Self {
        if v.is_null() {
            return Self::default();
        }
        if let Some(arr) = v.as_array() {
            return Self {
                items: arr
                    .iter()
                    .filter_map(CompletionItem::from_lsp_value)
                    .collect(),
                is_incomplete: false,
            };
        }
        let is_incomplete = v
            .get("isIncomplete")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let items = v
            .get("items")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(CompletionItem::from_lsp_value)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Self {
            items,
            is_incomplete,
        }
    }
}

/// Per-server, per-uri completion state. Keyed by `(server_id, uri)`
/// because the same buffer might in principle be served by multiple
/// LSPs (e.g. a Python file with both pyright and ruff-lsp); each
/// keeps its own lane.
#[derive(Default)]
pub struct CompletionStore {
    by_key: HashMap<CompletionKey, CompletionResponse>,
    /// Currently-selected index per key. Maintained outside
    /// `CompletionResponse` so `set` can preserve / clamp it.
    selection: HashMap<CompletionKey, usize>,
}

/// Composite key into the completion store. `server` is the
/// stringified [`crate::lsp::LspServerId`]; the URI matches the
/// document the completion was requested for.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct CompletionKey {
    /// LSP server id (decimal).
    pub server: String,
    /// Document URI the completion targets.
    pub uri: String,
}

impl CompletionKey {
    /// Construct a key.
    #[must_use]
    pub fn new(server: impl Into<String>, uri: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            uri: uri.into(),
        }
    }
}

impl CompletionStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the response at `key`. Resets the selection to 0
    /// (or none if there are no items).
    pub fn set(&mut self, key: CompletionKey, resp: CompletionResponse) {
        if resp.items.is_empty() {
            self.by_key.remove(&key);
            self.selection.remove(&key);
            return;
        }
        self.selection.insert(key.clone(), 0);
        self.by_key.insert(key, resp);
    }

    /// Drop the response at `key`.
    pub fn clear(&mut self, key: &CompletionKey) {
        self.by_key.remove(key);
        self.selection.remove(key);
    }

    /// Look up the response at `key`.
    #[must_use]
    pub fn get(&self, key: &CompletionKey) -> Option<&CompletionResponse> {
        self.by_key.get(key)
    }

    /// Items at `key`, or empty.
    #[must_use]
    pub fn items(&self, key: &CompletionKey) -> &[CompletionItem] {
        self.by_key.get(key).map_or(&[], |r| &r.items)
    }

    /// Currently selected index at `key`, or 0 if absent.
    #[must_use]
    pub fn selected(&self, key: &CompletionKey) -> usize {
        self.selection.get(key).copied().unwrap_or(0)
    }

    /// Move the selection. Saturates at `[0, items.len() - 1]`.
    pub fn select(&mut self, key: &CompletionKey, idx: usize) {
        let len = self.by_key.get(key).map_or(0, |r| r.items.len());
        if len == 0 {
            return;
        }
        let clamped = idx.min(len - 1);
        self.selection.insert(key.clone(), clamped);
    }

    /// Move selection by `delta`, wrapping at the ends.
    #[allow(
        clippy::cast_possible_wrap,
        reason = "selection indices are bounded by Vec::len() which fits in isize on every supported target"
    )]
    pub fn move_selection(&mut self, key: &CompletionKey, delta: isize) {
        let len = self.by_key.get(key).map_or(0, |r| r.items.len());
        if len == 0 {
            return;
        }
        let cur = self.selected(key) as isize;
        let len_i = len as isize;
        let mut next = (cur + delta) % len_i;
        if next < 0 {
            next += len_i;
        }
        self.selection.insert(key.clone(), next as usize);
    }

    /// True if the response at `key` was flagged `isIncomplete`.
    #[must_use]
    pub fn is_incomplete(&self, key: &CompletionKey) -> bool {
        self.by_key.get(key).is_some_and(|r| r.is_incomplete)
    }

    /// All keys currently in the store.
    pub fn keys(&self) -> impl Iterator<Item = &CompletionKey> {
        self.by_key.keys()
    }
}

/// Cheaply-cloneable shared handle.
pub type SharedCompletionStore = Arc<Mutex<CompletionStore>>;

/// Build a fresh shared store.
#[must_use]
pub fn make_shared_store() -> SharedCompletionStore {
    Arc::new(Mutex::new(CompletionStore::new()))
}

// ---------------------------------------------------------------------------
// Trigger machinery
// ---------------------------------------------------------------------------

/// Trigger character set negotiated with the LSP. Built from the
/// server's `completionProvider.triggerCharacters` capability.
#[derive(Clone, Debug, Default)]
pub struct CompletionTriggers {
    chars: Vec<char>,
}

impl CompletionTriggers {
    /// Empty trigger set --- only explicit invocation fires completion.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build from an LSP `completionProvider` capability object.
    /// Reads `completionProvider.triggerCharacters` (a JSON array of
    /// single-char strings). Unknown shapes yield an empty set.
    #[must_use]
    pub fn from_capabilities(caps: &Value) -> Self {
        let Some(arr) = caps
            .get("completionProvider")
            .and_then(|p| p.get("triggerCharacters"))
            .and_then(Value::as_array)
        else {
            return Self::empty();
        };
        let mut chars = Vec::with_capacity(arr.len());
        for v in arr {
            if let Some(s) = v.as_str()
                && let Some(ch) = s.chars().next()
            {
                chars.push(ch);
            }
        }
        Self { chars }
    }

    /// True if `ch` is in the trigger set.
    #[must_use]
    pub fn should_fire(&self, ch: char) -> bool {
        self.chars.contains(&ch)
    }

    /// All trigger characters.
    #[must_use]
    pub fn chars(&self) -> &[char] {
        &self.chars
    }
}

// ---------------------------------------------------------------------------
// In-buffer completion popup session (Arc 1a, Q#C2)
// ---------------------------------------------------------------------------

/// One row of the in-buffer completion popup: a projection of a
/// [`crate::completion_framework::CompletionCandidate`] carrying only
/// what rendering and the accept path need. `insert_text` is already
/// resolved (label fallback applied) so accept never re-derives it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PopupCandidate {
    /// Display label.
    pub label: String,
    /// Item kind (drives the glyph column).
    pub kind: CompletionItemKind,
    /// Optional one-line detail rendered after the label.
    pub detail: Option<String>,
    /// Text that replaces `[anchor .. cursor]` on accept.
    pub insert_text: String,
}

/// Live state of the in-buffer completion popup (Q#C2). Frontend-
/// agnostic, mirroring [`crate::menu::MenuState`]: the Lua driver
/// publishes into it, the TUI [`CompletionView`] overlay renders from
/// it, the dispatcher's completion shadow navigates/accepts against
/// it, and (phase 2) the semantic producer ships it to the GPU.
///
/// Unlike the menu's cell anchor, `anchor` is a **byte offset** (the
/// prefix start) --- each frontend maps byte → screen position itself,
/// so the instance never learns a pixel.
pub struct CompletionPopupState {
    /// Buffer the popup targets. The session closes the moment the
    /// active buffer differs (Q#C3 validation).
    pub buffer_id: BufferId,
    /// Byte offset where the typed prefix starts. For a
    /// trigger-character session (e.g. right after `.`) the prefix is
    /// empty and `anchor` equals the cursor.
    pub anchor: Position,
    /// The prefix as of the last publish (refresh keeps it current).
    pub prefix: String,
    /// Candidates, best-first. The driver has already scored, dropped
    /// non-matches, and capped.
    pub candidates: Vec<PopupCandidate>,
    /// Highlighted row index into `candidates`.
    pub selected: usize,
    /// Full candidate count before any cap the driver applied.
    pub total: usize,
}

impl CompletionPopupState {
    /// Build a session. Returns `None` when `candidates` is empty ---
    /// an empty popup never opens (the driver enforces this too; this
    /// is the belt to its suspenders).
    #[must_use]
    pub fn new(
        buffer_id: BufferId,
        anchor: Position,
        prefix: String,
        candidates: Vec<PopupCandidate>,
        total: usize,
    ) -> Option<Self> {
        if candidates.is_empty() {
            return None;
        }
        Some(Self {
            buffer_id,
            anchor,
            prefix,
            candidates,
            selected: 0,
            total,
        })
    }

    /// Move the highlight by `delta`, wrapping at the ends.
    #[allow(
        clippy::cast_possible_wrap,
        reason = "candidate indices are bounded by Vec::len() which fits in isize on every supported target"
    )]
    pub fn step(&mut self, delta: isize) {
        let len = self.candidates.len() as isize;
        if len == 0 {
            return;
        }
        let mut next = (self.selected as isize + delta) % len;
        if next < 0 {
            next += len;
        }
        self.selected = next as usize;
    }

    /// The highlighted candidate.
    #[must_use]
    pub fn selected_candidate(&self) -> Option<&PopupCandidate> {
        self.candidates.get(self.selected)
    }
}

/// Shared handle to the open popup (`None` when closed). Held by
/// [`crate::editor_core::EditorCore`] and read by [`CompletionView`],
/// the completion twin of [`crate::menu::SharedMenu`].
pub type SharedCompletionPopup = Arc<Mutex<Option<CompletionPopupState>>>;

/// A fresh, closed shared popup.
#[must_use]
pub fn make_shared_popup() -> SharedCompletionPopup {
    Arc::new(Mutex::new(None))
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Default popup width in cells when nothing better is available.
/// Wide enough for "method-name : detail" without wrapping the typical
/// rust-analyzer reply.
const DEFAULT_POPUP_WIDTH: u32 = 40;

/// Rows the popup shows at once; when more candidates are live the
/// visible slice windows around the selection (mirroring the
/// minibuffer dropdown's `MB_VISIBLE` cap).
const POPUP_MAX_ROWS: u32 = 10;

/// Minimum popup width in cells (glyph column + a readable label).
const POPUP_MIN_WIDTH: u32 = 12;

/// Tab-stop width in display columns, matching [`crate::diag`] /
/// [`crate::text_view`].
const TAB_WIDTH: u32 = 8;

/// Style for the currently-selected row (reverse video so it pops on
/// any base palette).
fn selected_style() -> Style {
    Style {
        reverse: true,
        ..Style::default()
    }
}

/// Style for the kind-glyph column (dim foreground).
fn kind_style() -> Style {
    Style {
        fg: Color::Indexed(8),
        bg: Color::Indexed(236),
        ..Style::default()
    }
}

/// Popup background (non-selected rows) --- the same dim fill as the
/// context menu, so the popup reads as a floating surface over the
/// buffer text it occludes.
fn popup_style() -> Style {
    Style {
        fg: Color::Indexed(252),
        bg: Color::Indexed(236),
        ..Style::default()
    }
}

/// The visible slice of `n` candidates windowed around `selected`:
/// returns `(start, len)`. Mirrors the minibuffer dropdown's centered
/// window so the highlight stays in view as the user cycles.
#[must_use]
pub(crate) fn popup_window(n: usize, selected: usize, max: usize) -> (usize, usize) {
    if n <= max {
        return (0, n);
    }
    let half = max / 2;
    let start = selected.saturating_sub(half).min(n - max);
    (start, max)
}

/// Self-positioning popup overlay for the in-buffer completion session
/// (Q#C4). Persistent on the active window once attached (deduped by
/// [`View::kind`]); renders nothing while the popup is closed or the
/// window shows a different buffer, mirroring [`crate::menu::MenuView`]'s
/// self-suppressing model. Owns every cell inside the popup rectangle.
///
/// Placement: the row *below* the anchor's screen row, with as many
/// rows as fit; when nothing fits below, it flips *above* the anchor.
/// The left edge sits at the anchor's display column, shifted left when
/// the popup would overflow the window's right edge.
pub struct CompletionView {
    popup: SharedCompletionPopup,
}

impl CompletionView {
    /// Build a view reading `popup`.
    #[must_use]
    pub fn new(popup: SharedCompletionPopup) -> Self {
        Self { popup }
    }
}

/// Display column of `byte_end` within `line_bytes` (tab-aware,
/// UTF-8-aware). The completion twin of the diagnostic underline's
/// column resolution.
fn display_col_for_byte(line_bytes: &[u8], byte_end: u32) -> u32 {
    let end = (byte_end as usize).min(line_bytes.len());
    let text = String::from_utf8_lossy(&line_bytes[..end]);
    let mut col = 0u32;
    for ch in text.chars() {
        if ch == '\t' {
            col += TAB_WIDTH - (col % TAB_WIDTH);
        } else {
            col += char_display_width(ch);
        }
    }
    col
}

/// Resolved popup rectangle, in window-relative cells.
struct PopupRect {
    /// First popup row, relative to the viewport's top.
    top: u32,
    /// Left edge, relative to the viewport's left.
    left: u32,
    /// Popup width in cells.
    width: u32,
    /// Rows actually shown (≤ the windowed candidate count).
    shown: u32,
}

/// Map the popup's byte anchor to a clamped on-screen rectangle:
/// below the anchor row when at least one row fits, flipped above
/// otherwise; left edge at the anchor's display column, shifted back
/// from the right margin. `None` when the anchor is scrolled out of
/// the viewport or nothing fits.
fn resolve_popup_rect(
    buf: &Buffer,
    viewport: Viewport,
    anchor: Position,
    rows: &[PopupCandidate],
) -> Option<PopupRect> {
    // Anchor byte → (screen row, display col), the diag-view walk.
    let source: Vec<u8> = {
        let mut bytes = vec![0u8; buf.len() as usize];
        if !bytes.is_empty() {
            buf.snapshot_rope().slice(0, buf.len(), &mut bytes);
        }
        bytes
    };
    let anchor = (anchor as usize).min(source.len()) as u32;
    let line_offsets = crate::diag::compute_line_offsets(&source);
    let start_line = crate::diag::line_at_offset(&line_offsets, viewport.buffer_start as u32);
    let anchor_line = crate::diag::line_at_offset(&line_offsets, anchor);
    if anchor_line < start_line {
        return None; // anchor scrolled above the viewport
    }
    let anchor_row = anchor_line - start_line;
    let max_rows = viewport.cell_size.rows;
    let max_cols = viewport.cell_size.cols;
    if anchor_row >= max_rows || max_cols == 0 {
        return None; // anchor scrolled below the viewport
    }
    let line_start = line_offsets[anchor_line as usize];
    let line_end = line_offsets
        .get(anchor_line as usize + 1)
        .copied()
        .unwrap_or(source.len() as u32);
    let line_bytes = &source[line_start as usize..line_end as usize];
    let anchor_col = display_col_for_byte(line_bytes, anchor - line_start);

    // Vertical placement: below the anchor row when at least one row
    // fits, else flipped above.
    let want_rows = rows.len() as u32;
    let below = max_rows - anchor_row - 1;
    let (top, shown) = if below > 0 {
        (anchor_row + 1, want_rows.min(below))
    } else {
        let shown = want_rows.min(anchor_row);
        (anchor_row - shown, shown)
    };
    if shown == 0 {
        return None;
    }

    // Width: glyph column + widest visible "label  detail", clamped to
    // the window; left edge shifts back from the right margin.
    let widest = rows
        .iter()
        .map(|c| {
            let detail = c.detail.as_deref().map_or(0, |d| d.chars().count() + 2);
            (c.label.chars().count() + detail) as u32
        })
        .max()
        .unwrap_or(0);
    let width = (widest + 3)
        .clamp(POPUP_MIN_WIDTH, DEFAULT_POPUP_WIDTH)
        .min(max_cols);
    let left = anchor_col.min(max_cols - width);
    Some(PopupRect {
        top,
        left,
        width,
        shown,
    })
}

/// Paint one popup row (background fill, kind glyph, label + detail)
/// at absolute row `r`, columns `[abs_left .. abs_left + width)`.
fn paint_popup_row(
    cells: &mut CellGrid<'_>,
    item: &PopupCandidate,
    r: u32,
    abs_left: u32,
    width: u32,
    selected: bool,
) {
    let row_style = if selected {
        selected_style()
    } else {
        popup_style()
    };
    // Paint the whole row's background first so the selected row's
    // reverse video covers the trailing whitespace.
    for c in 0..width {
        let cell = cells.at(CellCoord::new(r, abs_left + c));
        cell.glyph = Glyph::Char(' ');
        cell.style = row_style;
        cell.attachment = None;
    }
    // Column 0: kind glyph.
    let kind_cell = cells.at(CellCoord::new(r, abs_left));
    kind_cell.glyph = Glyph::Char(item.kind.glyph());
    kind_cell.style = if selected {
        selected_style()
    } else {
        kind_style()
    };
    // Columns 2..: label, optionally followed by the detail.
    let mut text = String::with_capacity(item.label.len() + 4);
    text.push_str(&item.label);
    if let Some(detail) = item.detail.as_deref() {
        text.push_str("  ");
        text.push_str(detail);
    }
    let mut col: u32 = 2;
    for ch in text.chars() {
        if col >= width {
            break;
        }
        let cw = char_display_width(ch);
        if cw == 0 {
            continue;
        }
        let cell = cells.at(CellCoord::new(r, abs_left + col));
        cell.glyph = Glyph::Char(ch);
        cell.style = row_style;
        cell.attachment = None;
        col += 1;
        if cw == 2 && col < width {
            let cont = cells.at(CellCoord::new(r, abs_left + col));
            cont.glyph = Glyph::Continuation;
            cont.style = row_style;
            cont.attachment = None;
            col += 1;
        }
    }
}

impl View for CompletionView {
    fn kind(&self) -> &'static str {
        "completion-popup"
    }

    fn render(&mut self, buf: &Buffer, viewport: Viewport, cells: &mut CellGrid<'_>) {
        // Snapshot under the lock, then drop it before touching the rope.
        let (anchor, rows_data, selected_in_window): (Position, Vec<PopupCandidate>, usize) = {
            let guard = self.popup.lock().expect("completion popup poisoned");
            let Some(popup) = guard.as_ref() else {
                return;
            };
            if popup.buffer_id != buf.id() {
                return; // this window shows a different buffer
            }
            let (start, len) = popup_window(
                popup.candidates.len(),
                popup.selected,
                POPUP_MAX_ROWS as usize,
            );
            (
                popup.anchor,
                popup.candidates[start..start + len].to_vec(),
                popup.selected - start,
            )
        };
        if rows_data.is_empty() {
            return;
        }
        let Some(rect) = resolve_popup_rect(buf, viewport, anchor, &rows_data) else {
            return;
        };
        let origin = viewport.cell_origin;
        for (i, item) in rows_data.iter().take(rect.shown as usize).enumerate() {
            paint_popup_row(
                cells,
                item,
                origin.row + rect.top + i as u32,
                origin.col + rect.left,
                rect.width,
                i == selected_in_window,
            );
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
    fn from_lsp_value_parses_completion_list() {
        let v = json!({
            "isIncomplete": true,
            "items": [
                { "label": "println", "kind": 3, "detail": "macro println!", "insertText": "println!" },
                { "label": "print", "kind": 3 }
            ]
        });
        let r = CompletionResponse::from_lsp_value(&v);
        assert!(r.is_incomplete);
        assert_eq!(r.items.len(), 2);
        assert_eq!(r.items[0].label, "println");
        assert_eq!(r.items[0].kind, CompletionItemKind::Function);
        assert_eq!(r.items[0].detail.as_deref(), Some("macro println!"));
        assert_eq!(r.items[0].effective_insert_text(), "println!");
        // Falls back to label.
        assert_eq!(r.items[1].effective_insert_text(), "print");
    }

    #[test]
    fn from_lsp_value_parses_bare_array() {
        let v = json!([
            { "label": "a", "kind": 6 },
            { "label": "b" } // missing kind → Text
        ]);
        let r = CompletionResponse::from_lsp_value(&v);
        assert!(!r.is_incomplete);
        assert_eq!(r.items.len(), 2);
        assert_eq!(r.items[0].kind, CompletionItemKind::Variable);
        assert_eq!(r.items[1].kind, CompletionItemKind::Text);
    }

    #[test]
    fn from_lsp_value_collapses_markup_documentation() {
        let v = json!({
            "label": "x",
            "documentation": { "kind": "markdown", "value": "**bold** doc" }
        });
        let item = CompletionItem::from_lsp_value(&v).unwrap();
        assert_eq!(item.documentation.as_deref(), Some("**bold** doc"));
        // String form also works.
        let v2 = json!({ "label": "y", "documentation": "plain" });
        assert_eq!(
            CompletionItem::from_lsp_value(&v2)
                .unwrap()
                .documentation
                .as_deref(),
            Some("plain")
        );
    }

    #[test]
    fn store_set_clears_when_empty() {
        let mut s = CompletionStore::new();
        let key = CompletionKey::new("1", "file:///a");
        s.set(
            key.clone(),
            CompletionResponse {
                items: vec![CompletionItem::from_lsp_value(&json!({"label":"a"})).unwrap()],
                is_incomplete: false,
            },
        );
        assert_eq!(s.items(&key).len(), 1);
        s.set(key.clone(), CompletionResponse::default());
        assert!(s.items(&key).is_empty());
        assert!(!s.is_incomplete(&key));
    }

    #[test]
    fn store_selection_wraps() {
        let mut s = CompletionStore::new();
        let key = CompletionKey::new("1", "file:///a");
        s.set(
            key.clone(),
            CompletionResponse {
                items: (0..3)
                    .map(|i| {
                        CompletionItem::from_lsp_value(&json!({"label": format!("x{i}")})).unwrap()
                    })
                    .collect(),
                is_incomplete: false,
            },
        );
        assert_eq!(s.selected(&key), 0);
        s.move_selection(&key, 1);
        assert_eq!(s.selected(&key), 1);
        s.move_selection(&key, 5); // 1 + 5 = 6, mod 3 = 0
        assert_eq!(s.selected(&key), 0);
        s.move_selection(&key, -1); // -1 mod 3 = 2
        assert_eq!(s.selected(&key), 2);
    }

    #[test]
    fn store_select_clamps() {
        let mut s = CompletionStore::new();
        let key = CompletionKey::new("1", "file:///a");
        s.set(
            key.clone(),
            CompletionResponse {
                items: vec![
                    CompletionItem::from_lsp_value(&json!({"label":"a"})).unwrap(),
                    CompletionItem::from_lsp_value(&json!({"label":"b"})).unwrap(),
                ],
                is_incomplete: false,
            },
        );
        s.select(&key, 99);
        assert_eq!(s.selected(&key), 1);
    }

    #[test]
    fn triggers_from_capabilities() {
        let caps = json!({
            "completionProvider": { "triggerCharacters": [".", "::", ">"] }
        });
        let t = CompletionTriggers::from_capabilities(&caps);
        assert!(t.should_fire('.'));
        // `::` is recorded as ':' (the first char) per the LSP spec
        // wording --- triggers are single chars.
        assert!(t.should_fire(':'));
        assert!(t.should_fire('>'));
        assert!(!t.should_fire('a'));
    }

    #[test]
    fn triggers_empty_for_capabilities_without_completion_provider() {
        let caps = json!({ "hoverProvider": true });
        let t = CompletionTriggers::from_capabilities(&caps);
        assert!(t.chars().is_empty());
        assert!(!t.should_fire('.'));
    }

    #[test]
    fn item_kind_glyph_is_stable() {
        assert_eq!(CompletionItemKind::Function.glyph(), 'f');
        assert_eq!(CompletionItemKind::Field.glyph(), 'p');
        assert_eq!(CompletionItemKind::Variable.glyph(), 'v');
        assert_eq!(CompletionItemKind::Class.glyph(), 'C');
        assert_eq!(CompletionItemKind::Keyword.glyph(), 'k');
        assert_eq!(CompletionItemKind::Text.glyph(), '.');
    }

    #[test]
    fn null_response_is_empty() {
        let r = CompletionResponse::from_lsp_value(&Value::Null);
        assert!(r.items.is_empty());
        assert!(!r.is_incomplete);
    }

    // ---- popup session (Arc 1a, Q#C2) ---------------------------------------

    fn cand(label: &str) -> PopupCandidate {
        PopupCandidate {
            label: label.to_owned(),
            kind: CompletionItemKind::Text,
            detail: None,
            insert_text: label.to_owned(),
        }
    }

    #[test]
    fn popup_state_refuses_empty_candidates() {
        assert!(
            CompletionPopupState::new(BufferId::from_raw(1), 0, String::new(), vec![], 0).is_none()
        );
    }

    #[test]
    fn popup_state_step_wraps_both_directions() {
        let mut p = CompletionPopupState::new(
            BufferId::from_raw(1),
            0,
            "ab".into(),
            vec![cand("a"), cand("b"), cand("c")],
            3,
        )
        .unwrap();
        assert_eq!(p.selected, 0);
        p.step(1);
        assert_eq!(p.selected, 1);
        p.step(4); // 1 + 4 = 5, mod 3 = 2
        assert_eq!(p.selected, 2);
        p.step(1); // wraps to the top
        assert_eq!(p.selected, 0);
        p.step(-1); // wraps to the bottom
        assert_eq!(p.selected, 2);
        assert_eq!(p.selected_candidate().unwrap().label, "c");
    }

    #[test]
    fn popup_window_keeps_selection_visible() {
        // Fits: identity window.
        assert_eq!(popup_window(3, 0, 10), (0, 3));
        // Overflow: centers on the selection…
        assert_eq!(popup_window(30, 15, 10), (10, 10));
        // …clamps at the top…
        assert_eq!(popup_window(30, 0, 10), (0, 10));
        assert_eq!(popup_window(30, 2, 10), (0, 10));
        // …and at the bottom.
        assert_eq!(popup_window(30, 29, 10), (20, 10));
        // The selected index always falls inside the window.
        for sel in 0..30 {
            let (start, len) = popup_window(30, sel, 10);
            assert!(sel >= start && sel < start + len, "sel {sel} escaped");
        }
    }
}
