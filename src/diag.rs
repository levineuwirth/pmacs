// diag.rs --- T M4.6 LSP-backed diagnostics: store + view.

//! Diagnostic state and the [`DiagnosticView`] that renders it.
//!
//! Per spec §M4.6: a diagnostic is what an LSP reports for a buffer
//! ("syntax error here", "unused variable there"). The
//! [`DiagnosticStore`] holds the most recent set per URI, updated
//! when [`crate::lsp::LspManager`] handles a
//! `textDocument/publishDiagnostics` notification. The
//! [`DiagnosticView`] composes over a buffer's text view to underline
//! diagnostic ranges in severity-coloured wavy lines.
//!
//! # Why a separate module
//!
//! The store needs to be shared between the LSP layer (writer) and
//! every attached view (reader), and the view's `Send` bound forces
//! `Arc<Mutex<...>>`. Pulling this into its own module keeps `lsp.rs`
//! focused on the protocol and gives the store a clean home that
//! M4.7 (hover, completion, signature) and M4.8 (status surface) can
//! reuse.
//!
//! # Position model
//!
//! LSP positions are `(line, character)` where `character` is in
//! UTF-16 code units. For v0.1 we treat `character` as UTF-8 byte
//! columns within the line --- that's wrong for buffers with
//! non-BMP characters, but correct for the typical ASCII/BMP source
//! files that drive the M4.6 acceptance gate. M5 may add a proper
//! UTF-16 ↔ UTF-8 translation pass through the rope.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use unicode_width::UnicodeWidthChar;

use crate::buffer::Buffer;
use crate::cell::{CellCoord, CellGrid, Color, Style, UnderlineStyle};
use crate::overlay::merge_styles;
use crate::view::{View, Viewport};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// LSP diagnostic severity. `Error` is the most severe; `Hint` is
/// the least. Mirrors the LSP `DiagnosticSeverity` enum with the
/// same numeric values.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum DiagnosticSeverity {
    /// Compilation / parse error.
    Error = 1,
    /// Style or correctness warning.
    Warning = 2,
    /// Informational message.
    Information = 3,
    /// Hint (lowest severity).
    Hint = 4,
}

impl DiagnosticSeverity {
    /// Canonical short label for the modeline / status surface.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Information => "info",
            Self::Hint => "hint",
        }
    }

    /// One-letter gutter glyph.
    #[must_use]
    pub fn gutter_glyph(self) -> char {
        match self {
            Self::Error => 'E',
            Self::Warning => 'W',
            Self::Information => 'I',
            Self::Hint => 'H',
        }
    }

    fn from_lsp_value(v: Option<&Value>) -> Self {
        match v.and_then(Value::as_i64) {
            Some(1) => Self::Error,
            Some(2) => Self::Warning,
            Some(3) => Self::Information,
            // 4 hint, missing → also hint per the spec's "implementation
            // free to choose" clause; pmacs picks Hint as the floor
            // so absent severity isn't conflated with Error.
            _ => Self::Hint,
        }
    }
}

/// One diagnostic, parsed from `textDocument/publishDiagnostics`.
/// The `range` is half-open in LSP `(line, character)` coordinates.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    /// First affected position.
    pub start_line: u32,
    /// First affected column (UTF-16 code units per LSP; pmacs v0.1
    /// treats them as byte offsets within the line).
    pub start_col: u32,
    /// One-past-last affected position's line.
    pub end_line: u32,
    /// One-past-last affected position's column.
    pub end_col: u32,
    /// Severity bucket.
    pub severity: DiagnosticSeverity,
    /// Human-readable message.
    pub message: String,
    /// `source` field --- which linter / LSP produced this. Per the
    /// M4.6 acceptance, this must be visible.
    pub source: Option<String>,
    /// Optional `code` field (e.g. `"E0308"`, `"unused-imports"`).
    pub code: Option<String>,
}

impl Diagnostic {
    /// Parse a single diagnostic from an LSP JSON object. Returns
    /// `None` for objects that don't have the minimum required
    /// fields (range, message).
    #[must_use]
    pub fn from_lsp_value(v: &Value) -> Option<Self> {
        let range = v.get("range")?;
        let start = range.get("start")?;
        let end = range.get("end")?;
        let start_line = start.get("line")?.as_u64()? as u32;
        let start_col = start.get("character")?.as_u64()? as u32;
        let end_line = end.get("line")?.as_u64()? as u32;
        let end_col = end.get("character")?.as_u64()? as u32;
        let message = v.get("message")?.as_str()?.to_owned();
        let severity = DiagnosticSeverity::from_lsp_value(v.get("severity"));
        let source = v.get("source").and_then(|s| s.as_str()).map(str::to_owned);
        let code = v.get("code").and_then(|c| {
            c.as_str()
                .map(str::to_owned)
                .or_else(|| c.as_i64().map(|n| n.to_string()))
        });
        Some(Self {
            start_line,
            start_col,
            end_line,
            end_col,
            severity,
            message,
            source,
            code,
        })
    }

    /// Compare two diagnostics by start position (row first, then
    /// column). Used to keep `next/previous` navigation deterministic.
    fn compare_by_position(&self, other: &Self) -> std::cmp::Ordering {
        self.start_line
            .cmp(&other.start_line)
            .then(self.start_col.cmp(&other.start_col))
    }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Per-URI diagnostic set. Mutated by [`crate::lsp::LspManager`]
/// when handling `textDocument/publishDiagnostics`; read by
/// [`DiagnosticView`] on every render.
///
/// **T M11.8 — `stale_uris` tracking.** Per-URI flag set by
/// [`Self::mark_stale`] whenever the LSP layer ships a
/// `textDocument/didChange` for that URI; cleared by [`Self::set`]
/// when fresh diagnostics arrive. The semantic-frontend producer
/// reads [`Self::is_stale`] and suppresses decorations during the
/// window between an edit and clangd's republish, so a `pmacs-gpu`
/// (or any `semantic_render`) frontend never paints diagnostic
/// colors at byte positions that have since shifted under the
/// document. Closes the LSP-re-analysis-gap surface that bet #1
/// in the session-4 framing pass predicted.
#[derive(Default)]
pub struct DiagnosticStore {
    by_uri: HashMap<String, Vec<Diagnostic>>,
    /// URIs whose stored diagnostics are known to be out of date
    /// because a `textDocument/didChange` was issued after the last
    /// `publishDiagnostics` was absorbed. `Self::set` clears entries
    /// here on the assumption that a fresh `publishDiagnostics`
    /// corresponds to the latest sent version.
    stale_uris: std::collections::HashSet<String>,
}

impl DiagnosticStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the diagnostic set for `uri`. Sorts by start position
    /// so [`Self::next_after`] / [`Self::previous_before`] don't
    /// have to re-sort on each query.
    ///
    /// Also clears the URI's stale flag (T M11.8) — fresh
    /// diagnostics imply the LSP has caught up to the current
    /// document state.
    pub fn set(&mut self, uri: impl Into<String>, mut diags: Vec<Diagnostic>) {
        diags.sort_by(Diagnostic::compare_by_position);
        let uri = uri.into();
        self.stale_uris.remove(&uri);
        if diags.is_empty() {
            self.by_uri.remove(&uri);
        } else {
            self.by_uri.insert(uri, diags);
        }
    }

    /// Drop the diagnostics for `uri`. Also clears the stale flag —
    /// no entry to be stale about.
    pub fn clear(&mut self, uri: &str) {
        self.by_uri.remove(uri);
        self.stale_uris.remove(uri);
    }

    /// Mark `uri`'s stored diagnostics as stale (T M11.8). Called
    /// by the LSP layer on each `textDocument/didChange` so the
    /// `semantic_render` producer can suppress emission during the
    /// LSP-re-analysis gap. The next [`Self::set`] (or
    /// [`Self::clear`]) clears the flag.
    pub fn mark_stale(&mut self, uri: impl Into<String>) {
        self.stale_uris.insert(uri.into());
    }

    /// `true` iff the URI's stored diagnostics are stale (the
    /// document has been edited since the last `publishDiagnostics`
    /// absorption). T M11.8.
    #[must_use]
    pub fn is_stale(&self, uri: &str) -> bool {
        self.stale_uris.contains(uri)
    }

    /// All diagnostics for `uri`, in start-position order.
    /// Returns an empty slice if the URI has none.
    #[must_use]
    pub fn for_uri(&self, uri: &str) -> &[Diagnostic] {
        self.by_uri.get(uri).map_or(&[], Vec::as_slice)
    }

    /// Number of diagnostics across all URIs at each severity.
    /// `(error, warning, info, hint)`.
    #[must_use]
    pub fn totals(&self) -> (usize, usize, usize, usize) {
        let mut e = 0;
        let mut w = 0;
        let mut i = 0;
        let mut h = 0;
        for diags in self.by_uri.values() {
            for d in diags {
                match d.severity {
                    DiagnosticSeverity::Error => e += 1,
                    DiagnosticSeverity::Warning => w += 1,
                    DiagnosticSeverity::Information => i += 1,
                    DiagnosticSeverity::Hint => h += 1,
                }
            }
        }
        (e, w, i, h)
    }

    /// Per-URI count.
    #[must_use]
    pub fn count_for(&self, uri: &str) -> usize {
        self.by_uri.get(uri).map_or(0, Vec::len)
    }

    /// All known URIs that have at least one diagnostic.
    pub fn uris(&self) -> impl Iterator<Item = &str> {
        self.by_uri.keys().map(String::as_str)
    }

    /// First diagnostic in `uri` whose start is strictly past
    /// `(line, col)`. Returns `None` if no such diagnostic exists
    /// (caller may wrap to [`Self::first_for`]).
    #[must_use]
    pub fn next_after(&self, uri: &str, line: u32, col: u32) -> Option<&Diagnostic> {
        self.by_uri
            .get(uri)?
            .iter()
            .find(|d| d.start_line > line || (d.start_line == line && d.start_col > col))
    }

    /// Last diagnostic in `uri` whose start is strictly before
    /// `(line, col)`.
    #[must_use]
    pub fn previous_before(&self, uri: &str, line: u32, col: u32) -> Option<&Diagnostic> {
        self.by_uri
            .get(uri)?
            .iter()
            .rev()
            .find(|d| d.start_line < line || (d.start_line == line && d.start_col < col))
    }

    /// First diagnostic in `uri`, or `None` if none.
    #[must_use]
    pub fn first_for(&self, uri: &str) -> Option<&Diagnostic> {
        self.by_uri.get(uri)?.first()
    }

    /// Last diagnostic in `uri`, or `None` if none.
    #[must_use]
    pub fn last_for(&self, uri: &str) -> Option<&Diagnostic> {
        self.by_uri.get(uri)?.last()
    }
}

/// Cheaply-cloneable shared handle. Held by
/// [`crate::lsp::LspManager`] (writer) and every
/// [`DiagnosticView`] (reader).
pub type SharedDiagStore = Arc<Mutex<DiagnosticStore>>;

/// Build a fresh shared store.
#[must_use]
pub fn make_shared_store() -> SharedDiagStore {
    Arc::new(Mutex::new(DiagnosticStore::new()))
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Tab-stop width in display columns, matching
/// [`crate::text_view`] and [`crate::highlight`].
const TAB_WIDTH: u32 = 8;

/// Style applied to bytes covered by an `Error` diagnostic. Wavy
/// underline in red so it composes with whatever the syntax view
/// painted on top.
fn error_style() -> Style {
    Style {
        underline: UnderlineStyle::Curly,
        // Red. Indexed 1 is portable across 8/16-color terminals.
        fg: Color::Default,
        bg: Color::Default,
        ..Style::default()
    }
}

/// Style applied to bytes covered by a `Warning` diagnostic.
fn warning_style() -> Style {
    Style {
        underline: UnderlineStyle::Curly,
        ..Style::default()
    }
}

/// Style applied to bytes covered by an `Information` diagnostic.
fn info_style() -> Style {
    Style {
        underline: UnderlineStyle::Single,
        ..Style::default()
    }
}

/// Style applied to bytes covered by a `Hint` diagnostic.
fn hint_style() -> Style {
    Style {
        underline: UnderlineStyle::Dotted,
        ..Style::default()
    }
}

fn style_for(severity: DiagnosticSeverity) -> Style {
    match severity {
        DiagnosticSeverity::Error => error_style(),
        DiagnosticSeverity::Warning => warning_style(),
        DiagnosticSeverity::Information => info_style(),
        DiagnosticSeverity::Hint => hint_style(),
    }
}

/// View that consumes the shared diagnostic store and underlines
/// affected ranges. Composes over [`crate::text_view::TextView`] per
/// the M2 view-composition contract --- never writes glyphs, only
/// merges underline styles into cells the base view has already
/// painted.
pub struct DiagnosticView {
    /// URI this view's diagnostics are keyed under. Set once at
    /// construction; M5 may add re-rooting if a buffer is renamed.
    uri: String,
    /// Shared store; mutated by the LSP manager, read by this view
    /// on every render.
    store: SharedDiagStore,
}

impl DiagnosticView {
    /// Construct a diagnostic view for `uri` against `store`.
    #[must_use]
    pub fn new(uri: impl Into<String>, store: SharedDiagStore) -> Self {
        Self {
            uri: uri.into(),
            store,
        }
    }

    /// The URI this view is keyed under.
    #[must_use]
    pub fn uri(&self) -> &str {
        &self.uri
    }
}

impl View for DiagnosticView {
    fn kind(&self) -> &'static str {
        "diagnostic"
    }

    fn render(&mut self, buf: &Buffer, viewport: Viewport, cells: &mut CellGrid<'_>) {
        // Snapshot the diagnostics under the lock and drop it
        // immediately so we don't hold the lock through rendering
        // (rendering touches the rope, which could in principle
        // share the lock if the LSP layer ever re-enters --- it
        // doesn't today, but the discipline is cheap).
        let diags: Vec<Diagnostic> = {
            let guard = self.store.lock().expect("diag store mutex poisoned");
            if guard.is_stale(&self.uri) {
                return;
            }
            guard.for_uri(&self.uri).to_vec()
        };
        if diags.is_empty() {
            return;
        }

        // Compute a snapshot of the buffer's source so we can
        // translate (line, column) → display column with tab/UTF-8
        // accounting. The rope keeps this cheap (refcount bump on
        // the root). M5 may switch to streaming row reads.
        let source: Vec<u8> = {
            let mut bytes = vec![0u8; buf.len() as usize];
            if !bytes.is_empty() {
                buf.snapshot_rope().slice(0, buf.len(), &mut bytes);
            }
            bytes
        };
        let line_offsets = compute_line_offsets(&source);
        let total_lines = line_offsets.len() as u32;
        let start_line_buf = line_at_offset(&line_offsets, viewport.buffer_start as u32);

        let max_rows = viewport.cell_size.rows;
        let max_cols = viewport.cell_size.cols;
        let cell_origin = viewport.cell_origin;

        for diag in &diags {
            let style = style_for(diag.severity);
            // Apply to each line the diagnostic touches. LSP ranges
            // are half-open at the end position; if end_col == 0
            // the diagnostic stops at the start of `end_line` so
            // we don't paint that line.
            let last_line = if diag.end_col == 0 && diag.end_line > diag.start_line {
                diag.end_line - 1
            } else {
                diag.end_line
            };
            for line in diag.start_line..=last_line {
                if line >= total_lines {
                    break;
                }
                if line < start_line_buf {
                    continue;
                }
                let row_offset = line - start_line_buf;
                if row_offset >= max_rows {
                    break;
                }
                let line_start = line_offsets[line as usize];
                let line_end = line_offsets
                    .get(line as usize + 1)
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
                let line_byte_len = line_bytes.len() as u32;

                // Resolve (start_col, end_col) within this line.
                let byte_start = if line == diag.start_line {
                    diag.start_col.min(line_byte_len)
                } else {
                    0
                };
                let byte_end = if line == diag.end_line {
                    diag.end_col.min(line_byte_len)
                } else {
                    line_byte_len
                };
                if byte_end <= byte_start {
                    // Empty range on a line --- still flag the
                    // gutter character (the cell at column 0). For
                    // a multi-line diagnostic with end_col=0, this
                    // path is hit on the first / last line; the
                    // visible column already moved on.
                    continue;
                }
                let (start_col, end_col) =
                    byte_range_to_display_cols(line_bytes, byte_start as usize, byte_end as usize);
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
// Shared helpers (mirror highlight.rs; kept private here to avoid
// cross-module coupling on internal helpers)
// ---------------------------------------------------------------------------

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

fn line_at_offset(line_offsets: &[u32], offset: u32) -> u32 {
    match line_offsets.binary_search(&offset) {
        Ok(i) => i as u32,
        Err(i) => i.saturating_sub(1) as u32,
    }
}

fn byte_range_to_display_cols(line_bytes: &[u8], byte_start: usize, byte_end: usize) -> (u32, u32) {
    let bs = byte_start.min(line_bytes.len());
    let be = byte_end.min(line_bytes.len());
    let display_to = |upto: usize| -> u32 {
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn diag(line: u32, sev: DiagnosticSeverity, msg: &str) -> Diagnostic {
        Diagnostic {
            start_line: line,
            start_col: 0,
            end_line: line,
            end_col: 5,
            severity: sev,
            message: msg.to_owned(),
            source: Some("test".to_owned()),
            code: None,
        }
    }

    #[test]
    fn from_lsp_value_parses_minimal_diagnostic() {
        let v = json!({
            "range": {
                "start": { "line": 1, "character": 4 },
                "end":   { "line": 1, "character": 9 },
            },
            "severity": 1,
            "message": "expected `;`",
            "source": "rust-analyzer",
            "code": "E0308"
        });
        let d = Diagnostic::from_lsp_value(&v).expect("parse");
        assert_eq!(d.start_line, 1);
        assert_eq!(d.start_col, 4);
        assert_eq!(d.end_line, 1);
        assert_eq!(d.end_col, 9);
        assert_eq!(d.severity, DiagnosticSeverity::Error);
        assert_eq!(d.message, "expected `;`");
        assert_eq!(d.source.as_deref(), Some("rust-analyzer"));
        assert_eq!(d.code.as_deref(), Some("E0308"));
    }

    #[test]
    fn from_lsp_value_returns_none_when_required_fields_missing() {
        // Missing range.
        assert!(Diagnostic::from_lsp_value(&json!({"message": "x"})).is_none());
        // Missing message.
        assert!(
            Diagnostic::from_lsp_value(&json!({
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } }
            }))
            .is_none()
        );
    }

    #[test]
    fn severity_falls_back_to_hint_for_unknown_or_missing() {
        let v = json!({
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } },
            "message": "x"
        });
        assert_eq!(
            Diagnostic::from_lsp_value(&v).unwrap().severity,
            DiagnosticSeverity::Hint
        );
    }

    #[test]
    fn store_set_replaces_and_sorts() {
        let mut s = DiagnosticStore::new();
        s.set(
            "file:///a",
            vec![
                diag(5, DiagnosticSeverity::Error, "x"),
                diag(2, DiagnosticSeverity::Warning, "y"),
            ],
        );
        let out = s.for_uri("file:///a");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].start_line, 2, "should sort by line");
        assert_eq!(out[1].start_line, 5);
    }

    #[test]
    fn store_clear_removes_uri() {
        let mut s = DiagnosticStore::new();
        s.set(
            "file:///a",
            vec![diag(0, DiagnosticSeverity::Error, "boom")],
        );
        s.clear("file:///a");
        assert!(s.for_uri("file:///a").is_empty());
    }

    #[test]
    fn next_after_finds_strictly_following_diagnostic() {
        let mut s = DiagnosticStore::new();
        s.set(
            "file:///a",
            vec![
                diag(2, DiagnosticSeverity::Warning, "y"),
                diag(5, DiagnosticSeverity::Error, "x"),
                diag(7, DiagnosticSeverity::Hint, "h"),
            ],
        );
        // From line 0: first one.
        assert_eq!(s.next_after("file:///a", 0, 0).unwrap().start_line, 2);
        // From line 5 col 0: skip the one at (5, 0).
        assert_eq!(s.next_after("file:///a", 5, 0).unwrap().start_line, 7);
        // Past the last: None.
        assert!(s.next_after("file:///a", 99, 0).is_none());
    }

    #[test]
    fn previous_before_finds_strictly_preceding_diagnostic() {
        let mut s = DiagnosticStore::new();
        s.set(
            "file:///a",
            vec![
                diag(2, DiagnosticSeverity::Warning, "y"),
                diag(5, DiagnosticSeverity::Error, "x"),
            ],
        );
        assert_eq!(s.previous_before("file:///a", 99, 0).unwrap().start_line, 5);
        assert_eq!(s.previous_before("file:///a", 5, 0).unwrap().start_line, 2);
        assert!(s.previous_before("file:///a", 0, 0).is_none());
    }

    #[test]
    fn totals_count_per_severity() {
        let mut store = DiagnosticStore::new();
        store.set(
            "a",
            vec![
                diag(0, DiagnosticSeverity::Error, "1"),
                diag(1, DiagnosticSeverity::Error, "2"),
                diag(2, DiagnosticSeverity::Warning, "3"),
            ],
        );
        store.set(
            "b",
            vec![
                diag(0, DiagnosticSeverity::Information, "4"),
                diag(1, DiagnosticSeverity::Hint, "5"),
            ],
        );
        let totals = store.totals();
        assert_eq!(totals, (2, 1, 1, 1));
    }

    #[test]
    fn empty_set_clears_uri() {
        let mut s = DiagnosticStore::new();
        s.set("a", vec![diag(0, DiagnosticSeverity::Error, "x")]);
        s.set("a", Vec::new());
        assert!(s.for_uri("a").is_empty());
        assert_eq!(s.uris().count(), 0);
    }

    // ---- T M11.8 stale-flag ----

    #[test]
    fn stale_flag_default_false() {
        let s = DiagnosticStore::new();
        assert!(!s.is_stale("file:///a"));
    }

    #[test]
    fn mark_stale_sets_flag() {
        let mut s = DiagnosticStore::new();
        s.mark_stale("file:///a");
        assert!(s.is_stale("file:///a"));
        assert!(!s.is_stale("file:///b"), "stale flag is per-URI");
    }

    #[test]
    fn set_clears_stale_flag() {
        let mut s = DiagnosticStore::new();
        s.mark_stale("file:///a");
        assert!(s.is_stale("file:///a"));
        s.set("file:///a", vec![diag(0, DiagnosticSeverity::Error, "x")]);
        assert!(!s.is_stale("file:///a"));
    }

    #[test]
    fn empty_set_clears_stale_flag_too() {
        // Empty diags after an edit means "LSP has caught up and
        // there are no issues" — clear stale to allow rendering an
        // empty decoration set.
        let mut s = DiagnosticStore::new();
        s.mark_stale("file:///a");
        s.set("file:///a", Vec::new());
        assert!(!s.is_stale("file:///a"));
    }

    #[test]
    fn clear_drops_stale_flag() {
        let mut s = DiagnosticStore::new();
        s.set("file:///a", vec![diag(0, DiagnosticSeverity::Error, "x")]);
        s.mark_stale("file:///a");
        s.clear("file:///a");
        assert!(!s.is_stale("file:///a"));
    }

    #[test]
    fn severity_label_and_glyph_are_stable() {
        for (s, lbl, gl) in [
            (DiagnosticSeverity::Error, "error", 'E'),
            (DiagnosticSeverity::Warning, "warning", 'W'),
            (DiagnosticSeverity::Information, "info", 'I'),
            (DiagnosticSeverity::Hint, "hint", 'H'),
        ] {
            assert_eq!(s.label(), lbl);
            assert_eq!(s.gutter_glyph(), gl);
        }
    }

    #[test]
    fn view_advertises_diagnostic_kind() {
        // `pmacs.window._overlay_kinds()` introspection (task #23 wire-up,
        // mirroring "syntax-highlight" / LspStyleView) relies on this.
        let store = make_shared_store();
        let view = DiagnosticView::new("file:///a", store);
        assert_eq!(view.kind(), "diagnostic");
    }

    #[test]
    fn diagnostic_view_suppresses_stale_store_entries() {
        use crate::cell::{Cell, CellSize, UnderlineStyle};

        let store = make_shared_store();
        {
            let mut guard = store.lock().expect("diag store");
            guard.set(
                "file:///a",
                vec![diag(0, DiagnosticSeverity::Error, "stale")],
            );
            guard.mark_stale("file:///a");
        }

        let mut buf = Buffer::new(crate::buffer::BufferId::next(), "test.c");
        buf.apply_edit(crate::buffer::EditOp::Insert {
            pos: 0,
            bytes: b"hello\n",
        })
        .expect("seed buffer");

        let mut view = DiagnosticView::new("file:///a", store);
        let mut backing = vec![Cell::default(); 10];
        let mut grid = CellGrid {
            cells: &mut backing,
            stride: 10,
            size: CellSize::new(1, 10),
        };
        view.render(
            &buf,
            Viewport {
                buffer_start: 0,
                buffer_end: buf.len(),
                cell_origin: CellCoord::new(0, 0),
                cell_size: CellSize::new(1, 10),
            },
            &mut grid,
        );

        assert_eq!(
            grid.get(CellCoord::new(0, 0)).style.underline,
            UnderlineStyle::None,
            "stale diagnostics must not underline shifted TUI bytes"
        );
    }
}
