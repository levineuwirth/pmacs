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
use crate::cell::{CellCoord, CellGrid, Color, Glyph, Style, UnderlineStyle};
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

    /// Canonical severity color (T M4.6 / protocol v6): the
    /// `underline_color` of this severity's squiggle, the column-0
    /// marker background, and the minimap mark all share it. Indexed
    /// 1/3/6/8 (red / yellow / cyan / gray) for 8/16-color terminal
    /// portability.
    #[must_use]
    pub fn underline_color(self) -> Color {
        match self {
            Self::Error => Color::Indexed(1),
            Self::Warning => Color::Indexed(3),
            Self::Information => Color::Indexed(6),
            Self::Hint => Color::Indexed(8),
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
    /// Per-URI severity totals, maintained alongside `by_uri` so
    /// frame-time consumers do not rescan every diagnostic at render
    /// cadence. Entries exist exactly when `by_uri` entries do.
    severity_counts: HashMap<String, (u32, u32, u32, u32)>,
    /// URIs whose stored diagnostics are known to be out of date
    /// because a `textDocument/didChange` was issued after the last
    /// `publishDiagnostics` was absorbed. `Self::set` clears entries
    /// here on the assumption that a fresh `publishDiagnostics`
    /// corresponds to the latest sent version.
    stale_uris: std::collections::HashSet<String>,
    /// Per-URI change counter, bumped on every [`Self::set`] /
    /// [`Self::clear`]. Diagnostics arrive without a CRDT generation
    /// bump, so generation-keyed caches (the `FileStyleSummary`
    /// producer's) additionally key on this to know a republish
    /// happened (T M4.6 GPU parity).
    epochs: HashMap<String, u64>,
}

fn count_severities(diags: &[Diagnostic]) -> (u32, u32, u32, u32) {
    let mut counts = (0u32, 0u32, 0u32, 0u32);
    for diagnostic in diags {
        let slot = match diagnostic.severity {
            DiagnosticSeverity::Error => &mut counts.0,
            DiagnosticSeverity::Warning => &mut counts.1,
            DiagnosticSeverity::Information => &mut counts.2,
            DiagnosticSeverity::Hint => &mut counts.3,
        };
        *slot = slot.saturating_add(1);
    }
    counts
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
        let counts = count_severities(&diags);
        self.stale_uris.remove(&uri);
        *self.epochs.entry(uri.clone()).or_insert(0) += 1;
        if diags.is_empty() {
            self.by_uri.remove(&uri);
            self.severity_counts.remove(&uri);
        } else {
            self.severity_counts.insert(uri.clone(), counts);
            self.by_uri.insert(uri, diags);
        }
    }

    /// Drop the diagnostics for `uri`. Also clears the stale flag —
    /// no entry to be stale about.
    pub fn clear(&mut self, uri: &str) {
        self.by_uri.remove(uri);
        self.severity_counts.remove(uri);
        self.stale_uris.remove(uri);
        *self.epochs.entry(uri.to_owned()).or_insert(0) += 1;
    }

    /// Monotonic per-URI change counter: how many times `set` /
    /// `clear` ran for this URI. `0` for a URI never written.
    /// Consumers cache against this to detect republishes that no
    /// CRDT generation bump announces.
    #[must_use]
    pub fn epoch_for(&self, uri: &str) -> u64 {
        self.epochs.get(uri).copied().unwrap_or(0)
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
        for &(errors, warnings, information, hints) in self.severity_counts.values() {
            e += errors as usize;
            w += warnings as usize;
            i += information as usize;
            h += hints as usize;
        }
        (e, w, i, h)
    }

    /// Per-URI totals `(error, warning, information, hint)`.
    ///
    /// The tuple is computed once by [`Self::set`] and deliberately
    /// survives [`Self::mark_stale`]: staleness invalidates byte
    /// positions, while the last published counts remain valid as a
    /// frozen status summary until the next publication.
    #[must_use]
    pub fn severity_counts_for(&self, uri: &str) -> (u32, u32, u32, u32) {
        self.severity_counts.get(uri).copied().unwrap_or_default()
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

/// The RESOLVED severity color (themes arc Q#TH5): the `ui.diag.*`
/// face's `fg` when a face is set with a concrete color, else the
/// built-in [`DiagnosticSeverity::underline_color`]. The diag family
/// carries a special `Default` policy — `Default` fg means the
/// built-in severity color, never "plain" — because the color doubles
/// as the *presence* encoding in the minimap summary
/// (`FileStyleSummary.underline_color`, where `Default` reads as "no
/// mark"), so a plain severity color is unrepresentable and
/// `ui.diag.error = {}` degrades to the built-in on every surface.
#[must_use]
pub fn severity_color(
    theme: Option<&crate::highlight::Theme>,
    severity: DiagnosticSeverity,
) -> Color {
    let name = match severity {
        DiagnosticSeverity::Error => "ui.diag.error",
        DiagnosticSeverity::Warning => "ui.diag.warning",
        DiagnosticSeverity::Information => "ui.diag.info",
        DiagnosticSeverity::Hint => "ui.diag.hint",
    };
    match theme.and_then(|t| t.face(name)) {
        Some(f) if f.fg != Color::Default => f.fg,
        _ => severity.underline_color(),
    }
}

/// Style applied to bytes covered by a diagnostic: severity-shaped
/// underline (wavy for error/warning, single for info, dotted for
/// hint) colored via `underline_color` (not `fg`) so the squiggle
/// reads its severity color while the syntax view's text color
/// survives underneath (T M4.6, protocol v6). `color` is the
/// resolved severity color ([`severity_color`]).
fn style_for(severity: DiagnosticSeverity, color: Color) -> Style {
    let underline = match severity {
        DiagnosticSeverity::Error | DiagnosticSeverity::Warning => UnderlineStyle::Curly,
        DiagnosticSeverity::Information => UnderlineStyle::Single,
        DiagnosticSeverity::Hint => UnderlineStyle::Dotted,
    };
    Style {
        underline,
        underline_color: color,
        ..Style::default()
    }
}

/// Style of the column-0 line marker — the TUI's gutter sign
/// (T M4.6). The TUI reserves no gutter column, so the sign is a
/// severity-colored *background* on the line's first cell: the
/// glyph and its syntax color survive (the view contract is
/// style-only), and zero-width diagnostics — invisible to the
/// underline pass — still get a visible artifact. `color` is the
/// resolved severity color ([`severity_color`]).
fn marker_style_for(color: Color) -> Style {
    Style {
        bg: color,
        ..Style::default()
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
    /// Shared theme for the `ui.diag.*` face resolution (themes arc
    /// Q#TH9; the `SyntaxHighlightView` precedent). `None` — a bare
    /// test construction — paints the built-in severity colors.
    theme: Option<crate::highlight::ThemeHandle>,
}

impl DiagnosticView {
    /// Construct a diagnostic view for `uri` against `store`,
    /// resolving severity colors through `theme` when given.
    #[must_use]
    pub fn new(
        uri: impl Into<String>,
        store: SharedDiagStore,
        theme: Option<crate::highlight::ThemeHandle>,
    ) -> Self {
        Self {
            uri: uri.into(),
            store,
            theme,
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

        // Column-0 line markers (gutter signs, T M4.6): most severe
        // diagnostic per visible row wins. `Ord` on the severity enum
        // follows LSP numbering, so "most severe" is the minimum.
        let mut line_markers: std::collections::HashMap<u32, DiagnosticSeverity> =
            std::collections::HashMap::new();

        // One theme clone per render (themes arc Q#TH9, the
        // SyntaxHighlightView discipline) for the ui.diag.* faces.
        let theme = self
            .theme
            .as_ref()
            .map(|t| t.lock().expect("theme mutex poisoned").clone());

        for diag in &diags {
            let style = style_for(diag.severity, severity_color(theme.as_ref(), diag.severity));
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
                // Record the line marker before any byte-range work:
                // zero-width ranges (`byte_end <= byte_start` below)
                // skip the underline but still mark the line.
                line_markers
                    .entry(row_offset)
                    .and_modify(|s| *s = (*s).min(diag.severity))
                    .or_insert(diag.severity);
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
                let (start_col, end_col) =
                    underline_cols_for_line(line_bytes, byte_start, byte_end);
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

        // Paint the per-line severity markers last so one is visible even
        // when an underline span also touches the same cell.
        paint_line_markers(
            cells,
            cell_origin,
            viewport.gutter_w,
            max_cols,
            &line_markers,
            theme.as_ref(),
        );
    }
}

/// Paint one severity marker per diagnostic line (UX gutter sub-arc 2).
///
/// When the window reserves a gutter (`gutter_w > 0`), draw the severity
/// *sign glyph* in the gutter's leading column (`cell_origin.col -
/// gutter_w`, i.e. window column 0), colored by severity. Without a gutter,
/// fall back to the legacy column-0 *background* marker on the line's first
/// text cell — the "fake gutter" that predates a real gutter column.
fn paint_line_markers(
    cells: &mut CellGrid<'_>,
    cell_origin: CellCoord,
    gutter_w: u32,
    max_cols: u32,
    line_markers: &std::collections::HashMap<u32, DiagnosticSeverity>,
    theme: Option<&crate::highlight::Theme>,
) {
    for (&row_offset, &severity) in line_markers {
        let row = cell_origin.row + row_offset;
        let color = severity_color(theme, severity);
        if gutter_w > 0 {
            let cell = cells.at(CellCoord::new(
                row,
                cell_origin.col.saturating_sub(gutter_w),
            ));
            cell.glyph = Glyph::Char(severity.gutter_glyph());
            cell.style = Style {
                fg: color,
                ..Style::default()
            };
        } else if max_cols > 0 {
            let cell = cells.at(CellCoord::new(row, cell_origin.col));
            cell.style = merge_styles(cell.style, marker_style_for(color));
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers (mirror highlight.rs; kept private here to avoid
// cross-module coupling on internal helpers)
// ---------------------------------------------------------------------------

pub(crate) fn compute_line_offsets(source: &[u8]) -> Vec<u32> {
    let mut out = Vec::with_capacity(source.len() / 32 + 1);
    out.push(0);
    for (i, b) in source.iter().enumerate() {
        if *b == b'\n' {
            out.push(i as u32 + 1);
        }
    }
    out
}

pub(crate) fn line_at_offset(line_offsets: &[u32], offset: u32) -> u32 {
    match line_offsets.binary_search(&offset) {
        Ok(i) => i as u32,
        Err(i) => i.saturating_sub(1) as u32,
    }
}

/// Resolve the display-column span to underline for one line of a
/// diagnostic. Zero-width ranges — the shape parsers use for
/// "expected COMMA"-style errors anchored one past the last token
/// (rust-analyzer reports a missing comma as `col 12 → col 12` at
/// end of line, and the caller's `.min(line_byte_len)` clamps
/// collapse any past-EOL anchor the same way) — get a single-cell
/// span at the anchor: one past EOL is a blank cell inside the
/// window, and a squiggled space is exactly how other editors
/// surface it.
fn underline_cols_for_line(line_bytes: &[u8], byte_start: u32, byte_end: u32) -> (u32, u32) {
    if byte_end <= byte_start {
        let (anchor, _) =
            byte_range_to_display_cols(line_bytes, byte_start as usize, byte_start as usize);
        (anchor, anchor + 1)
    } else {
        byte_range_to_display_cols(line_bytes, byte_start as usize, byte_end as usize)
    }
}

pub(crate) fn byte_range_to_display_cols(
    line_bytes: &[u8],
    byte_start: usize,
    byte_end: usize,
) -> (u32, u32) {
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
    fn cached_severity_counts_replace_clear_and_survive_staleness() {
        let mut store = DiagnosticStore::new();
        assert_eq!(store.severity_counts_for("file:///a"), (0, 0, 0, 0));

        store.set(
            "file:///a",
            vec![
                diag(0, DiagnosticSeverity::Error, "1"),
                diag(1, DiagnosticSeverity::Error, "2"),
                diag(2, DiagnosticSeverity::Warning, "3"),
                diag(3, DiagnosticSeverity::Information, "4"),
                diag(4, DiagnosticSeverity::Hint, "5"),
            ],
        );
        assert_eq!(store.severity_counts_for("file:///a"), (2, 1, 1, 1));

        store.mark_stale("file:///a");
        assert_eq!(
            store.severity_counts_for("file:///a"),
            (2, 1, 1, 1),
            "staleness freezes the last published counts"
        );

        store.set(
            "file:///a",
            vec![diag(0, DiagnosticSeverity::Warning, "replacement")],
        );
        assert_eq!(store.severity_counts_for("file:///a"), (0, 1, 0, 0));

        store.clear("file:///a");
        assert_eq!(store.severity_counts_for("file:///a"), (0, 0, 0, 0));
    }

    #[test]
    fn empty_set_clears_uri() {
        let mut s = DiagnosticStore::new();
        s.set("a", vec![diag(0, DiagnosticSeverity::Error, "x")]);
        s.set("a", Vec::new());
        assert!(s.for_uri("a").is_empty());
        assert_eq!(s.severity_counts_for("a"), (0, 0, 0, 0));
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
    fn severity_styles_color_the_underline_not_the_text() {
        // T M4.6: each severity gets a distinct underline color via
        // `underline_color` (SGR 58); `fg` stays Default so the
        // syntax view's text color survives the merge.
        let mut seen = Vec::new();
        for s in [
            DiagnosticSeverity::Error,
            DiagnosticSeverity::Warning,
            DiagnosticSeverity::Information,
            DiagnosticSeverity::Hint,
        ] {
            let style = style_for(s, severity_color(None, s));
            assert_eq!(style.fg, Color::Default, "{s:?} must not set fg");
            assert_ne!(
                style.underline_color,
                Color::Default,
                "{s:?} must color its underline"
            );
            assert!(
                !seen.contains(&style.underline_color),
                "{s:?} reuses another severity's underline color"
            );
            seen.push(style.underline_color);
        }
    }

    #[test]
    fn epoch_bumps_on_set_and_clear_per_uri() {
        let mut store = DiagnosticStore::new();
        assert_eq!(store.epoch_for("file:///a"), 0);
        store.set("file:///a", vec![diag(0, DiagnosticSeverity::Error, "x")]);
        assert_eq!(store.epoch_for("file:///a"), 1);
        // An empty set (server reports clean) still counts — the
        // consumer must refresh to drop its marks.
        store.set("file:///a", vec![]);
        assert_eq!(store.epoch_for("file:///a"), 2);
        store.clear("file:///a");
        assert_eq!(store.epoch_for("file:///a"), 3);
        // mark_stale is not a content change; other URIs are isolated.
        store.mark_stale("file:///a");
        assert_eq!(store.epoch_for("file:///a"), 3);
        assert_eq!(store.epoch_for("file:///b"), 0);
    }

    #[test]
    fn view_advertises_diagnostic_kind() {
        // `pmacs.window._overlay_kinds()` introspection (task #23 wire-up,
        // mirroring "syntax-highlight" / LspStyleView) relies on this.
        let store = make_shared_store();
        let view = DiagnosticView::new("file:///a", store, None);
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

        let mut view = DiagnosticView::new("file:///a", store, None);
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
                gutter_w: 0,
            },
            &mut grid,
        );

        assert_eq!(
            grid.get(CellCoord::new(0, 0)).style.underline,
            UnderlineStyle::None,
            "stale diagnostics must not underline shifted TUI bytes"
        );
    }

    #[test]
    fn column_zero_marker_shows_most_severe_diagnostic_per_line() {
        use crate::cell::{Cell, CellSize, Glyph, UnderlineStyle};

        let store = make_shared_store();
        {
            let mut guard = store.lock().expect("diag store");
            // Line 0: a Hint and an Error overlap — the marker must
            // show the Error. Line 1: a zero-width Warning (start ==
            // end), invisible to the underline pass but still marked.
            guard.set(
                "file:///a",
                vec![
                    diag(0, DiagnosticSeverity::Hint, "h"),
                    diag(0, DiagnosticSeverity::Error, "e"),
                    Diagnostic {
                        start_line: 1,
                        start_col: 2,
                        end_line: 1,
                        end_col: 2,
                        severity: DiagnosticSeverity::Warning,
                        message: "w".to_owned(),
                        source: None,
                        code: None,
                    },
                ],
            );
        }

        let mut buf = Buffer::new(crate::buffer::BufferId::next(), "test.c");
        buf.apply_edit(crate::buffer::EditOp::Insert {
            pos: 0,
            bytes: b"hello\nworld\nclean\n",
        })
        .expect("seed buffer");

        let mut view = DiagnosticView::new("file:///a", store, None);
        let mut backing = vec![Cell::default(); 30];
        // Pre-paint glyphs at column 0 to pin the style-only contract.
        backing[0].glyph = Glyph::Char('h');
        let mut grid = CellGrid {
            cells: &mut backing,
            stride: 10,
            size: CellSize::new(3, 10),
        };
        view.render(
            &buf,
            Viewport {
                buffer_start: 0,
                buffer_end: buf.len(),
                cell_origin: CellCoord::new(0, 0),
                cell_size: CellSize::new(3, 10),
                gutter_w: 0,
            },
            &mut grid,
        );

        // Line 0: red (error) marker wins over the hint's gray; the
        // glyph survives untouched.
        assert_eq!(grid.get(CellCoord::new(0, 0)).style.bg, Color::Indexed(1));
        assert_eq!(grid.get(CellCoord::new(0, 0)).glyph, Glyph::Char('h'));
        // Line 1: zero-width warning gets a marker and a single-cell
        // squiggle at its anchor column — and only there.
        assert_eq!(grid.get(CellCoord::new(1, 0)).style.bg, Color::Indexed(3));
        assert_eq!(
            grid.get(CellCoord::new(1, 2)).style.underline,
            UnderlineStyle::Curly
        );
        assert_eq!(
            grid.get(CellCoord::new(1, 3)).style.underline,
            UnderlineStyle::None
        );
        // Line 2: clean — no marker.
        assert_eq!(grid.get(CellCoord::new(2, 0)).style.bg, Color::Default);
    }

    #[test]
    fn gutter_sign_replaces_the_column_marker_when_a_gutter_is_reserved() {
        use crate::cell::{Cell, CellSize, Glyph, UnderlineStyle};

        let store = make_shared_store();
        {
            let mut guard = store.lock().expect("diag store");
            guard.set(
                "file:///a",
                vec![
                    // Line 0: Hint + Error overlap → the sign shows Error.
                    diag(0, DiagnosticSeverity::Hint, "h"),
                    diag(0, DiagnosticSeverity::Error, "e"),
                    // Line 1: zero-width Warning (invisible to underline).
                    Diagnostic {
                        start_line: 1,
                        start_col: 2,
                        end_line: 1,
                        end_col: 2,
                        severity: DiagnosticSeverity::Warning,
                        message: "w".to_owned(),
                        source: None,
                        code: None,
                    },
                ],
            );
        }

        let mut buf = Buffer::new(crate::buffer::BufferId::next(), "test.c");
        buf.apply_edit(crate::buffer::EditOp::Insert {
            pos: 0,
            bytes: b"hello\nworld\nclean\n",
        })
        .expect("seed buffer");

        // A 2-cell gutter: text is shifted to column 2, signs land at
        // window column 0 (`cell_origin.col - gutter_w`).
        let mut view = DiagnosticView::new("file:///a", store, None);
        let mut backing = vec![Cell::default(); 30];
        let mut grid = CellGrid {
            cells: &mut backing,
            stride: 10,
            size: CellSize::new(3, 10),
        };
        view.render(
            &buf,
            Viewport {
                buffer_start: 0,
                buffer_end: buf.len(),
                cell_origin: CellCoord::new(0, 2),
                cell_size: CellSize::new(3, 8),
                gutter_w: 2,
            },
            &mut grid,
        );

        // Line 0: Error sign glyph 'E' in red at the gutter's leading col.
        assert_eq!(grid.get(CellCoord::new(0, 0)).glyph, Glyph::Char('E'));
        assert_eq!(grid.get(CellCoord::new(0, 0)).style.fg, Color::Indexed(1));
        // Line 1: Warning sign 'W' in yellow.
        assert_eq!(grid.get(CellCoord::new(1, 0)).glyph, Glyph::Char('W'));
        assert_eq!(grid.get(CellCoord::new(1, 0)).style.fg, Color::Indexed(3));
        // The legacy background marker on the first *text* cell is NOT
        // painted when a gutter carries the sign instead.
        assert_eq!(grid.get(CellCoord::new(0, 2)).style.bg, Color::Default);
        // The squiggle still lands in the shifted text area (col 2 + 2).
        assert_eq!(
            grid.get(CellCoord::new(1, 4)).style.underline,
            UnderlineStyle::Curly
        );
        // Line 2: clean — no sign glyph.
        assert_eq!(grid.get(CellCoord::new(2, 0)).glyph, Glyph::Char(' '));
    }

    #[test]
    fn end_of_line_zero_width_error_squiggles_the_cell_past_eol() {
        // The missing-comma shape: rust-analyzer anchors "expected
        // COMMA" as a zero-width range one past the line's last
        // character (`b: 2` → col 12..12 on a 12-byte line). The
        // squiggle must land on the blank cell just past EOL, not
        // vanish in the empty-range clamp.
        use crate::cell::{Cell, CellSize, UnderlineStyle};

        let store = make_shared_store();
        store.lock().expect("diag store").set(
            "file:///a",
            vec![Diagnostic {
                start_line: 0,
                start_col: 5, // one past "hello" (5 bytes)
                end_line: 0,
                end_col: 5,
                severity: DiagnosticSeverity::Error,
                message: "expected COMMA".to_owned(),
                source: None,
                code: None,
            }],
        );

        let mut buf = Buffer::new(crate::buffer::BufferId::next(), "test.rs");
        buf.apply_edit(crate::buffer::EditOp::Insert {
            pos: 0,
            bytes: b"hello\nworld\n",
        })
        .expect("seed buffer");

        let mut view = DiagnosticView::new("file:///a", store, None);
        let mut backing = vec![Cell::default(); 20];
        let mut grid = CellGrid {
            cells: &mut backing,
            stride: 10,
            size: CellSize::new(2, 10),
        };
        view.render(
            &buf,
            Viewport {
                buffer_start: 0,
                buffer_end: buf.len(),
                cell_origin: CellCoord::new(0, 0),
                cell_size: CellSize::new(2, 10),
                gutter_w: 0,
            },
            &mut grid,
        );

        // Single-cell red squiggle on the blank cell past "hello".
        let cell = grid.get(CellCoord::new(0, 5));
        assert_eq!(cell.style.underline, UnderlineStyle::Curly);
        assert_eq!(cell.style.underline_color, Color::Indexed(1));
        // Nothing under the word itself or beyond the anchor.
        assert_eq!(
            grid.get(CellCoord::new(0, 4)).style.underline,
            UnderlineStyle::None
        );
        assert_eq!(
            grid.get(CellCoord::new(0, 6)).style.underline,
            UnderlineStyle::None
        );
    }
}
