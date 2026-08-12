//! pmacs-gpu — GPU/GUI frontend for pmacs.
//!
//! User-facing invocation is strict:
//!
//! - `pmacs-gpu --attach <unix-socket-path>` directly attaches to an
//!   already-running daemon and never starts or replaces it.
//! - The root `pmacs --gpu` broker invokes a hidden managed mode that connects
//!   first, starts the supplied daemon only for an absent/refused socket, and
//!   creates the window only after protocol and capability negotiation.
//! - Headless probe modes exercise the same direct and managed production
//!   connectors for acceptance without requiring a display.
//!
//! An attached frontend imports the daemon's `BufferSnapshot` into a local
//! loro replica, sends a `Viewport` back to request scoped styling, and
//! consumes the `StyleSpans` stream. Live `CrdtOp` updates apply to the
//! replica; subsequent `StyleSpans` frames re-style it.
//!
//! See `docs/pmacs-gpu-design.md` for the arc framing. Phase A's
//! adversarial-verification framing applies from session 4 forward;
//! findings classified per rule (iii) at surface-time.
//!
//! The bundled font is `JetBrains` Mono Regular, distributed under
//! the SIL Open Font License 1.1 (see `fonts/OFL.txt`).

mod attach;
mod math_layout;
mod math_parse;
mod terminal;

use std::collections::HashMap;
use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use glyphon::cosmic_text::{Affinity, Cursor, Scroll, Wrap};
use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, fontdb,
};
use loro::{ContainerTrait, ExportMode};
use pmacs_protocol::{
    AdornmentContent, AdornmentPlacement, BufferId, ByteRange, CellCoord, CellSize,
    CompletionPopupRow, CrdtOp, Decoration, DecorationKind, DecorationSegment, FrontendId,
    InlineAdornment, InstanceMessage, InstanceSignal, Key as ProtocolKey, LineNumberMode,
    MAX_STATUSLINE_FACE_BYTES, MAX_STATUSLINE_PROVIDERS, MAX_STATUSLINE_SEGMENT_BYTES,
    MAX_STATUSLINE_TOTAL_TEXT_BYTES, MenuPromptRow, MinibufferRow, Modifiers,
    MouseButton as ProtocolMouseButton, MouseKind as ProtocolMouseKind, PointerKind,
    SelectionSnapshot, StatuslineSegment, StyleSegment, StyleSpan, TAB_STOP_COLUMNS, TerminalFrame,
    UnderlineStyle,
    cell::{Color as CellColor, Style as CellStyle},
    is_builtin_pair_char, is_modeline_face_name,
    panel::{PANEL_MIN_VERSION, PanelFrame, PanelFramePayload},
};
use unicode_width::UnicodeWidthChar;
use wgpu::MultisampleState;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::attach::{AttachClient, AttachEvent, InitialTargetPaths};
use crate::terminal::{TerminalPaintPlan, TerminalPalette};

/// Bundled font (SIL Open Font License 1.1 — see `fonts/OFL.txt`).
const JETBRAINS_MONO: &[u8] = include_bytes!("../fonts/JetBrainsMono-Regular.ttf");

#[cfg(test)]
const TEST_MONO_TWO: &[u8] = include_bytes!("../fonts/test/PmacsTestMonoTwo-Regular.ttf");
#[cfg(test)]
const TEST_PROPORTIONAL: &[u8] = include_bytes!("../fonts/test/PmacsTestProportional-Regular.ttf");
#[cfg(test)]
const TEST_FAMILY_REGULAR: &[u8] = include_bytes!("../fonts/test/PmacsTestFamily-Regular.ttf");
#[cfg(test)]
const TEST_FAMILY_BOLD: &[u8] = include_bytes!("../fonts/test/PmacsTestFamily-Bold.ttf");
#[cfg(test)]
const TEST_FONT_SOURCES: &[&[u8]] = &[
    TEST_MONO_TWO,
    TEST_PROPORTIONAL,
    TEST_FAMILY_REGULAR,
    TEST_FAMILY_BOLD,
];

/// Extra font sources a headless `State` assembles with. Test builds add
/// the fixture families the font-preference tests rely on; the Vterm
/// Stage 3 attach probe, which is a release-mode binary, gets none.
fn headless_extra_font_sources() -> &'static [&'static [u8]] {
    #[cfg(test)]
    {
        TEST_FONT_SOURCES
    }
    #[cfg(not(test))]
    {
        &[]
    }
}

/// The default font family — the query `family: None` (and every
/// rejected requested family) resolves through (framing Q#F6). The
/// bundle guarantees the query is never empty; a monospaced
/// system-installed face of the same family may legitimately win by
/// insertion order (availability, not face identity).
const DEFAULT_FONT_FAMILY: &str = "JetBrains Mono";

/// Derived per-preference metrics (framing Q#F6). One knob — the
/// preference size — scales every surface by `size / 16.0`; the
/// unset default (`scale == 1.0`, `advance_ratio == 1.0`)
/// reproduces today's `BASE_*` constants bit-for-bit, so never-set
/// renders byte-identically. `advance_ratio` is the measured
/// selected/default NORMAL-face advance ratio (the fixed-ASCII
/// probe): the empty-document gutter fallback and the menu hit
/// width follow the resolved family without JetBrains-only drift.
#[derive(Clone, Copy)]
struct FontMetrics {
    scale: f32,
    advance_ratio: f32,
}

impl Default for FontMetrics {
    fn default() -> Self {
        Self {
            scale: 1.0,
            advance_ratio: 1.0,
        }
    }
}

impl FontMetrics {
    fn code_font_size(self) -> f32 {
        BASE_CODE_FONT_SIZE * self.scale
    }
    fn code_line_height(self) -> f32 {
        BASE_CODE_LINE_HEIGHT * self.scale
    }
    fn gutter_advance_fallback(self) -> f32 {
        BASE_GUTTER_MONO_ADVANCE_FALLBACK * self.scale * self.advance_ratio
    }
    fn status_band_height(self) -> f32 {
        BASE_STATUS_BAND_HEIGHT * self.scale
    }
    /// The panel divider strip's thickness (Stage 2 framing §5.3).
    ///
    /// Scaled like the status band because it is row chrome, not a fixed
    /// surface inset. The whole strip is both painted and hit-tested, so
    /// paint geometry and drag geometry cannot drift apart.
    fn divider_height(self) -> f32 {
        BASE_DIVIDER_HEIGHT * self.scale
    }
    fn status_font_size(self) -> f32 {
        BASE_STATUS_FONT_SIZE * self.scale
    }
    fn status_line_height(self) -> f32 {
        BASE_STATUS_LINE_HEIGHT * self.scale
    }
    fn menu_row_height(self) -> f32 {
        BASE_MENU_ROW_HEIGHT * self.scale
    }
    fn menu_font_size(self) -> f32 {
        BASE_MENU_FONT_SIZE * self.scale
    }
    fn menu_line_height(self) -> f32 {
        BASE_MENU_LINE_HEIGHT * self.scale
    }
    fn menu_char_w(self) -> f32 {
        BASE_MENU_CHAR_W * self.scale * self.advance_ratio
    }
    fn mb_drop_row_height(self) -> f32 {
        BASE_MB_DROP_ROW_HEIGHT * self.scale
    }
    fn mb_drop_font_size(self) -> f32 {
        BASE_MB_DROP_FONT_SIZE * self.scale
    }
    fn mb_drop_line_height(self) -> f32 {
        BASE_MB_DROP_LINE_HEIGHT * self.scale
    }
}

/// What sanitized assembly retained (framing Q#F6): the default
/// family name and the bundled face's ID, both asserted present and
/// monospaced at assembly time. The rejected-family fallback and the
/// `family: None` default both resolve through
/// `Family::Name(&default_family)` against the sanitized db, so the
/// fallback is total and cannot recurse into a proportional
/// collision.
struct FontDefaults {
    default_family: String,
    bundled_id: fontdb::ID,
    /// IDs loaded through the optional assembly input. Production
    /// passes no extras; headless tests retain fixture IDs so they can
    /// assert cosmic-text classified the final database, not a
    /// post-construction mutation.
    #[cfg(test)]
    extra_ids: Vec<fontdb::ID>,
}

/// Remove every NON-monospace face that advertises `default_family`
/// (framing Q#F6, round 3 finding 4): fontdb returns the first
/// surviving equally-good candidate in insertion order, so a
/// closer-weight proportional system face could otherwise win
/// bold/italic queries even when the normal query selected a valid
/// monospaced face. Parameterized by family + bundled ID so tests
/// run the PRODUCTION path with an unreserved fixture family. The
/// bundled face is monospaced and survives by construction.
fn sanitize_font_database(db: &mut fontdb::Database, default_family: &str, bundled_id: fontdb::ID) {
    let doomed: Vec<fontdb::ID> = db
        .faces()
        .filter(|f| {
            !f.monospaced
                && f.id != bundled_id
                && f.families.iter().any(|(name, _)| name == default_family)
        })
        .map(|f| f.id)
        .collect();
    for id in doomed {
        db.remove_face(id);
    }
}

/// Sanitized, current-order font database + `FontSystem` (framing
/// Q#F6): system fonts FIRST (what `FontSystem::new()` does today),
/// the bundled bytes second — retaining the bundled `fontdb::ID` —
/// then the collision filter, then cosmic-text's current
/// generic-family defaults, and only then the `FontSystem`
/// construction, so its internal monospace-ID set includes every
/// surviving monospaced face (the bundle included). The locale is
/// resolved exactly as cosmic-text does (`sys_locale`, `"en-US"`
/// fallback).
fn build_font_system(extra_sources: &[&'static [u8]]) -> (FontSystem, FontDefaults) {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let bundled_ids =
        db.load_font_source(fontdb::Source::Binary(std::sync::Arc::new(JETBRAINS_MONO)));
    let bundled_id = *bundled_ids
        .first()
        .expect("bundled JetBrains Mono contains one face");
    // Inline-math slice (Q#MS7): the math glyphs draw through
    // cosmic-text, so the same bytes the layout engine measures must
    // resolve as a family here — the F8b pin. Proportional, so the
    // same-family monospace filter below cannot touch it.
    db.load_font_source(fontdb::Source::Binary(std::sync::Arc::new(
        math_layout::LATIN_MODERN_MATH,
    )));
    let extra_ids: Vec<fontdb::ID> = extra_sources
        .iter()
        .flat_map(|bytes| db.load_font_source(fontdb::Source::Binary(std::sync::Arc::new(*bytes))))
        .collect();
    sanitize_font_database(&mut db, DEFAULT_FONT_FAMILY, bundled_id);
    db.set_monospace_family("Noto Sans Mono");
    db.set_sans_serif_family("Open Sans");
    db.set_serif_family("DejaVu Serif");
    let locale = sys_locale::get_locale().unwrap_or_else(|| String::from("en-US"));
    let font_system = FontSystem::new_with_locale_and_db(locale, db);
    #[cfg(not(test))]
    drop(extra_ids);
    let defaults = FontDefaults {
        default_family: DEFAULT_FONT_FAMILY.to_owned(),
        bundled_id,
        #[cfg(test)]
        extra_ids,
    };
    // Assembly-time assertions (framing Q#F6): the default query and
    // the bundled face are present and monospaced — the total
    // fallback depends on both.
    debug_assert!(
        font_system
            .db()
            .face(defaults.bundled_id)
            .is_some_and(|f| f.monospaced),
        "the bundled face must survive sanitization and be monospaced"
    );
    debug_assert!(
        font_system.is_monospace(defaults.bundled_id),
        "cosmic-text must register the bundled face as monospace"
    );
    debug_assert!(
        query_normal_face(font_system.db(), &defaults.default_family)
            .and_then(|id| font_system.db().face(id))
            .is_some_and(|f| f.monospaced),
        "the default family query must resolve to a monospaced face"
    );
    (font_system, defaults)
}

/// The normal-style query for `family` — the same `fontdb::Query`
/// implied by the base `Attrs` installed on all seven buffers
/// (normal weight, normal style, normal stretch).
fn query_normal_face(db: &fontdb::Database, family: &str) -> Option<fontdb::ID> {
    db.query(&fontdb::Query {
        families: &[fontdb::Family::Name(family)],
        weight: fontdb::Weight::NORMAL,
        stretch: fontdb::Stretch::Normal,
        style: fontdb::Style::Normal,
    })
}

/// The family name the bundled math font resolves to in fontdb; the
/// draw pass pins `Attrs` to it so drawn advances come from the same
/// face layout measured (framing F8b).
const MATH_FONT_FAMILY: &str = "Latin Modern Math";

/// The Q#MS10 fit budget at the given code metrics, derived from the
/// bundled code face's baseline placement (the framing's pinned rule).
/// A custom `set_font` family shifts the painted baseline slightly; v0
/// accepts that — boxes draw against the real shaped baseline, so only
/// the fit margin is approximate.
fn math_code_budget(fm: FontMetrics) -> (f32, f32) {
    ttf_parser::Face::parse(JETBRAINS_MONO, 0).map_or((0.0, 0.0), |face| {
        math_layout::line_box_budget(&face, fm.code_font_size(), fm.code_line_height())
    })
}

/// The fixed ASCII advance probe (framing Q#F6). The measurement uses
/// its total shaped width divided by this logical cell count; it does
/// not assume one glyph per digit because a valid monospace face may
/// substitute multi-cell digit ligatures.
const ADVANCE_PROBE: &str = "0123456789";

/// Wire-validation bounds for `FontFacts::size_centi_px` — 6.0..=72.0
/// logical px in integer hundredths (framing Q#F6, fail closed): 0
/// would panic `Buffer::set_metrics`, and the GPU re-checks on
/// arrival because this is deserialized protocol input — the
/// daemon-side Lua range check is a UX courtesy, not a trust
/// boundary.
const FONT_SIZE_CENTI_PX_RANGE: std::ops::RangeInclusive<u32> = 600..=7200;

/// Measure `family`'s normal-face cell advance at `metrics` by
/// shaping [`ADVANCE_PROBE`] in a scratch buffer — independent of
/// document contents, so the measurement is deterministic and the
/// NORMAL face is authoritative even when the first code glyph is
/// bold/italic. Total run width divided by logical cells survives
/// ligature substitution; `None` when the family shapes no width.
fn probe_mono_advance(font_system: &mut FontSystem, family: &str, metrics: Metrics) -> Option<f32> {
    let mut probe = Buffer::new(font_system, metrics);
    probe.set_size(font_system, None, None);
    probe.set_text(
        font_system,
        ADVANCE_PROBE,
        &Attrs::new().family(Family::Name(family)),
        Shaping::Advanced,
        None,
    );
    probe.shape_until_scroll(font_system, false);
    let total_width: f32 = probe.layout_runs().map(|run| run.line_w).sum();
    let cells = ADVANCE_PROBE.chars().count() as f32;
    (total_width > 0.0 && cells > 0.0).then_some(total_width / cells)
}

/// Initial window size in logical pixels.
const INITIAL_WIDTH: u32 = 800;
const INITIAL_HEIGHT: u32 = 200;

/// Color the surface clears to before text renders.
const BG: wgpu::Color = wgpu::Color {
    r: 0.05,
    g: 0.05,
    b: 0.07,
    a: 1.0,
};

const TEXT_LEFT: f32 = 16.0;
const TEXT_TOP: f32 = 16.0;

/// Stroke thickness for terminal straight-underline forms, in pixels.
const TERMINAL_UNDERLINE_PX: f32 = 1.0;

/// Fallback terminal selection wash when no `ui.selection` face is set.
const TERMINAL_SELECTION_RGBA: [f32; 4] = [0.35, 0.45, 0.75, 0.35];

/// The terminal cursor block. Translucent so the glyph beneath stays
/// readable — a terminal cursor sits ON a character, unlike the
/// document caret, which sits between two.
const TERMINAL_CURSOR_RGBA: [f32; 4] = [0.85, 0.85, 0.9, 0.55];
/// Caret bar width in px, and its color (bright, near-opaque — drawn
/// over the text so it reads as the active insertion point). Session
/// B1.
const CARET_WIDTH: f32 = 2.0;
const CARET_COLOR: [f32; 4] = [0.90, 0.90, 0.96, 0.90];

/// Math ink (Q#MS6): the plain code text color, as glyph color for
/// the mini-buffers and quad rgba for the fraction rule. Colour-by-
/// context is the parent arc's deferred Q#IM2.
const MATH_INK_COLOR: Color = Color::rgb(230, 230, 235);
const MATH_INK_RGBA: [f32; 4] = [230.0 / 255.0, 230.0 / 255.0, 235.0 / 255.0, 1.0];

/// Line-height factor for a math glyph's mini-buffer: roomy enough
/// that a lone glyph's ascender/descender never clips against the
/// buffer's own line box. Positioning ignores it — the `TextArea` top
/// is set from the mini-buffer's SHAPED `line_y`, so the glyph's
/// baseline lands exactly where layout put it.
const MATH_GLYPH_LINE_FACTOR: f32 = 2.0;
/// Extra source lines shaped beyond the visible window so a 1-line
/// scroll doesn't always re-slice and the bottom partial line renders
/// (Q#S3). Kept small — overscan is wasted shaping.
const SCROLL_OVERSCAN: usize = 2;
const TEXT_RIGHT_GAP: f32 = 10.0;
const MINIMAP_WIDTH: f32 = 48.0;
const MINIMAP_RIGHT: f32 = 12.0;
const MINIMAP_TOP: f32 = 12.0;
const MINIMAP_BOTTOM: f32 = 12.0;
const MINIMAP_MIN_SURFACE_WIDTH: u32 = 180;
const MINIMAP_MIN_THUMB_HEIGHT: f32 = 18.0;
const MINIMAP_H_PAD: f32 = 3.0;
const MINIMAP_CODE_COLS: f32 = 100.0;
const MINIMAP_MIN_STROKE_WIDTH: f32 = 1.5;
const MINIMAP_MAX_LINE_STROKE_HEIGHT: f32 = 2.0;
const BASE_CODE_LINE_HEIGHT: f32 = 22.0;
/// Font size of the code buffer (and the line-number gutter, so their
/// line heights match and rows align).
const BASE_CODE_FONT_SIZE: f32 = 16.0;
/// Gap in px between the line-number gutter digits and the code
/// (UX gutter arc, GPU side of sub-arc 1 — mirrors the TUI gutter).
const GUTTER_GAP_PX: f32 = 10.0;
/// Fallback monospace advance in px when no shaped glyph is available to
/// measure (0.6 em at the 16px code font).
const BASE_GUTTER_MONO_ADVANCE_FALLBACK: f32 = 9.6;
/// Diagnostic gutter sign (UX gutter sub-arc 2): a thin severity-colored
/// bar hugging the gutter's left edge, left of the line numbers — the GPU
/// analogue of the TUI's leading-column sign glyph. `X` is its left inset,
/// `W` its width; it spans the full line height.
const GUTTER_SIGN_X: f32 = 4.0;
const GUTTER_SIGN_W: f32 = 4.0;
/// Minimum text-area width (px) the gutter must leave. If reserving the
/// gutter would crowd the text below this, the gutter is dropped for the
/// frame — the GPU mirror of the TUI's too-narrow-window disable, so a
/// narrow window or a very large file can never force `left >= right`.
const MIN_TEXT_WIDTH_PX: f32 = 48.0;
const MINIMAP_BG: [f32; 4] = [0.075, 0.075, 0.105, 0.92];
const MINIMAP_DEFAULT_LINE: [f32; 4] = [0.23, 0.23, 0.29, 0.82];
const MINIMAP_THUMB_FILL: [f32; 4] = [0.82, 0.82, 0.92, 0.18];
const MINIMAP_THUMB_BORDER: [f32; 4] = [0.86, 0.86, 0.96, 0.7];
/// Q#M7 — dragging within this many pixels of the text area's top or
/// bottom edge auto-scrolls toward the pointer.
const EDGE_SCROLL_BAND: f32 = 24.0;
/// Q#M7 — one line per tick while edge-scrolling.
const EDGE_SCROLL_TICK: std::time::Duration = std::time::Duration::from_millis(35);
/// Q#M6 (bet #2) — after a far jump (no shaped line reused), hold
/// the redraw this long so the daemon's restyle usually lands before
/// the first visible frame: the styled frame replaces the unstyled
/// flash. Short enough to read as instantaneous when styling never
/// arrives (plain-text buffers).
const JUMP_STYLE_HOLD: std::time::Duration = std::time::Duration::from_millis(25);
/// Status band (Q#S2): one-line strip reserved at the surface
/// bottom — buffer name + modified star on the left, diagnostics /
/// cursor / scroll readout on the right.
const BASE_STATUS_BAND_HEIGHT: f32 = 26.0;
/// Panel divider (Stage 2 framing §5.3, decided open item): the rule
/// between the document and an installed panel band, at scale 1.0.
///
/// A 1-2 px rule is adequate decoration but too fragile as a drag target;
/// 4 px still reads as a rule while giving the pointer something to grab.
const BASE_DIVIDER_HEIGHT: f32 = 4.0;
/// Fallback fill for the divider strip when no `ui.divider` face is set.
const DIVIDER_RGBA: [f32; 4] = [0.28, 0.28, 0.36, 1.0];
const STATUS_BAND_BG: [f32; 4] = [0.105, 0.105, 0.145, 1.0];
const STATUS_TEXT_PAD: f32 = 10.0;
const BASE_STATUS_FONT_SIZE: f32 = 13.0;
const BASE_STATUS_LINE_HEIGHT: f32 = 18.0;
// Context menu popup (Q#CM1). One row per item/separator; width tracks
// the widest label (estimated from a fixed per-char advance, which the
// code font's monospacing makes good enough for hit-testing + the bg
// quad to agree).
const BASE_MENU_ROW_HEIGHT: f32 = 22.0;
const BASE_MENU_FONT_SIZE: f32 = 14.0;
const BASE_MENU_LINE_HEIGHT: f32 = 22.0;
const MENU_PAD_X: f32 = 12.0;
const BASE_MENU_CHAR_W: f32 = 8.4;
const MENU_MIN_WIDTH: f32 = 140.0;
const MENU_MAX_WIDTH: f32 = 380.0;
const MENU_BG: [f32; 4] = [0.16, 0.16, 0.20, 0.98];
const MENU_SELECTED_BG: [f32; 4] = [0.20, 0.40, 0.66, 1.0];
const MENU_SEPARATOR_BG: [f32; 4] = [0.30, 0.30, 0.36, 1.0];

// Minibuffer completion dropdown (Q#MB1). A vertical list anchored just
// above the bottom band, best match at the top; reuses the menu popup's
// colors. Width tracks the widest candidate (measured from the shaped
// buffer).
const BASE_MB_DROP_ROW_HEIGHT: f32 = 20.0;
const BASE_MB_DROP_FONT_SIZE: f32 = 13.0;
const BASE_MB_DROP_LINE_HEIGHT: f32 = 20.0;
const MB_DROP_PAD_X: f32 = 10.0;
const MB_DROP_MIN_WIDTH: f32 = 160.0;
const MB_DROP_MAX_WIDTH: f32 = 480.0;

/// Visible slice of the completion dropdown given `n` shaped candidates,
/// the `selected` index, and `band_top` pixels available above the status
/// band (audit F-007). Returns `(first, count)` — `count` clamped to the
/// rows that actually fit (so the box never renders above `y = 0`) and
/// `first` scrolled to keep `selected` on screen. `None` when nothing can
/// show: no candidates, or the window is too short for even one row. When
/// the whole list fits this is `(0, n)`, identical to the pre-clamp
/// behavior — the common path is unchanged.
fn mb_dropdown_window(
    n: usize,
    selected: usize,
    band_top: f32,
    fm: FontMetrics,
) -> Option<(usize, usize)> {
    if n == 0 {
        return None;
    }
    let max_rows = (band_top / fm.mb_drop_row_height()).floor() as usize;
    if max_rows == 0 {
        return None;
    }
    let count = n.min(max_rows);
    let sel = selected.min(n - 1);
    // Anchor `sel` at the window's bottom edge when it would otherwise be
    // below the fold, then clamp so we never scroll past the last row.
    let first = sel.saturating_sub(count - 1).min(n - count);
    Some((first, count))
}
const QUAD_SHADER: &str = r"
struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) pos: vec2<f32>,
    @location(1) color: vec4<f32>,
) -> VertexOut {
    var out: VertexOut;
    out.pos = vec4<f32>(pos, 0.0, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return in.color;
}
";

const QUAD_VERTEX_STRIDE: wgpu::BufferAddress = 24;
const QUAD_VERTEX_ATTRS: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];

/// Diagnostic squiggle shader (Q#W1). The vertex carries, beyond NDC
/// position and color, a `uv`: `uv.x` is the absolute screen-space
/// pixel x (so the wave's phase is continuous across separately
/// emitted glyph-run rects), `uv.y` is the signed pixel offset from
/// the band's vertical centerline. The fragment draws an
/// anti-aliased sine: alpha falls off with distance to the curve via
/// `fwidth`/`smoothstep` (both core WGSL — no MSAA or feature flag).
const SQUIGGLE_SHADER: &str = r"
struct VertexOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
) -> VertexOut {
    var out: VertexOut;
    out.pos = vec4<f32>(pos, 0.0, 1.0);
    out.uv = uv;
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let wavelength = 6.0;   // px per full sine period
    let amplitude = 1.4;    // px peak from centerline
    let thickness = 1.0;    // px stroke half-width
    let two_pi = 6.2831853;
    let wave = amplitude * sin(in.uv.x * (two_pi / wavelength));
    let dist = abs(in.uv.y - wave);
    let aa = fwidth(dist);
    let alpha = 1.0 - smoothstep(thickness - aa, thickness + aa, dist);
    return vec4<f32>(in.color.rgb, in.color.a * alpha);
}
";

const SQUIGGLE_VERTEX_STRIDE: wgpu::BufferAddress = 32;
const SQUIGGLE_VERTEX_ATTRS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];

const CONNECTING_TEXT: &str = "(connecting...)";

/// Container id the daemon uses on its loro `LoroDoc` for the
/// buffer's text. Must match `pmacs::crdt::CrdtState`'s container
/// name (`"body"`).
const LORO_TEXT_CONTAINER: &str = "body";

/// Custom events delivered to the winit event loop. The reader thread
/// in `attach.rs` forwards each decoded `InstanceMessage` through the
/// `EventLoopProxy<AppEvent>` it was handed by `connect()`; the main
/// thread dispatches them in `user_event` below.
#[derive(Debug)]
pub enum AppEvent {
    /// A message or disconnect notification from the attach reader
    /// thread.
    Attach(AttachEvent),
}

/// CLI mode derived from argv.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    /// Print CLI help without initializing winit or wgpu.
    Help,
    /// Print package and protocol versions without initializing winit or wgpu.
    Version,
    /// `pmacs-gpu --attach <socket>`: strict direct attach to an existing daemon.
    Attach { socket: PathBuf },
    /// Hidden root-broker entry: connect or start the supplied daemon before
    /// creating the window.
    ManagedAttach {
        socket: PathBuf,
        daemon_executable: PathBuf,
        initial_target: Option<InitialTargetPaths>,
    },
    /// `pmacs-gpu --headless-probe <socket> <report>`: attach through
    /// the real client, render real frames offscreen, and write a
    /// machine-readable report.
    ///
    /// This exists for the Vterm Stage 3 acceptance, which must exercise
    /// a real daemon, a real PTY, and real wgpu rendering in ONE path.
    /// It drives the same `attach` handshake, the same
    /// `apply_attach_message`, and the same `render_to_view` the windowed
    /// mode does — only winit is absent, because CI has no display.
    HeadlessProbe { socket: PathBuf, report: PathBuf },
    /// Hidden display-less acceptance seam for managed daemon lifecycle.
    HeadlessManagedProbe {
        socket: PathBuf,
        report: PathBuf,
        daemon_executable: PathBuf,
        initial_target: Option<InitialTargetPaths>,
    },
}

/// Number of decimal digits in `n` (for `n >= 1`); allocation-free. Sizes
/// the line-number gutter (UX gutter arc). Mirrors the TUI's
/// `pmacs::window::decimal_digits` — kept local since pmacs-gpu doesn't
/// depend on the `pmacs` crate.
fn decimal_digits(mut n: usize) -> u32 {
    let mut d = 1u32;
    while n >= 10 {
        n /= 10;
        d += 1;
    }
    d
}

fn main() {
    env_logger::init();
    let mode = match parse_args(&std::env::args_os().skip(1).collect::<Vec<_>>()) {
        Ok(mode) => mode,
        Err(error) => {
            eprintln!("pmacs-gpu: {error}\n\n{GPU_USAGE}");
            std::process::exit(2);
        }
    };
    match &mode {
        Mode::Help => {
            println!("{GPU_USAGE}");
            return;
        }
        Mode::Version => {
            println!(
                "pmacs-gpu {} (protocol v{})",
                env!("CARGO_PKG_VERSION"),
                pmacs_protocol::PROTOCOL_VERSION
            );
            return;
        }
        Mode::HeadlessProbe { socket, report } => {
            std::process::exit(run_headless_probe(socket, report));
        }
        Mode::HeadlessManagedProbe {
            socket,
            report,
            daemon_executable,
            initial_target,
        } => {
            std::process::exit(run_headless_managed_probe(
                socket,
                report,
                daemon_executable,
                initial_target.clone(),
            ));
        }
        Mode::Attach { .. } | Mode::ManagedAttach { .. } => {}
    }

    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .expect("create winit event loop");
    let proxy = event_loop.create_proxy();
    let (attach_client, pending_events) = if let Mode::ManagedAttach {
        socket,
        daemon_executable,
        initial_target,
    } = &mode
    {
        match attach::connect_managed_with_target(
            socket,
            daemon_executable,
            initial_target.clone(),
            proxy.clone(),
        ) {
            Ok(mut managed) => {
                let pending = managed
                    .client
                    .take_initial_message()
                    .map(|message| vec![AppEvent::Attach(AttachEvent::Message(Box::new(message)))])
                    .unwrap_or_default();
                (Some(managed.client), pending)
            }
            Err(error) => {
                eprintln!("pmacs-gpu: managed attach failed: {error}");
                std::process::exit(1);
            }
        }
    } else {
        (None, Vec::new())
    };
    let mut app = App {
        mode,
        proxy: Some(proxy),
        state: None,
        attach_client,
        pending_events,
        modifiers: winit::keyboard::ModifiersState::empty(),
    };
    event_loop
        .run_app(&mut app)
        .expect("winit event loop run_app");
}

/// Drive a real attach session headlessly and write a probe report.
///
/// The Vterm Stage 3 acceptance needs one path that exercises a real
/// daemon, a real PTY child, and real wgpu rendering together — a
/// decoded-message fixture would prove none of the three fit. This is
/// that path minus winit, which CI has no display for.
///
/// The report is one `key=value` line per fact so the acceptance test
/// asserts on named observations rather than parsing prose. Exit code 0
/// means the report was written; anything else means the probe could not
/// run and the acceptance fails loudly rather than reading a stale file.
#[allow(
    clippy::too_many_lines,
    reason = "one linear attach-render-observe probe session"
)]
fn run_headless_probe(socket: &Path, report: &Path) -> i32 {
    use std::fmt::Write as _;
    use std::sync::mpsc;

    let Some(mut state) = State::new_headless(900, 600, "(connecting...)") else {
        eprintln!("pmacs-gpu probe: no wgpu adapter available");
        return 3;
    };

    let (tx, rx) = mpsc::channel::<AttachEvent>();
    let client = match attach::connect_with_sink(socket, move |event| tx.send(event).is_ok()) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("pmacs-gpu probe: attach failed: {error}");
            return 4;
        }
    };
    state.set_frontend_id(client.frontend_id());
    // The panel wire is part of the real client, so the probe arms it exactly
    // as the winit path does. Leaving it out is why nothing could exercise a
    // panel-hosted terminal: with no declaration the daemon has no columns,
    // and with no columns no panel is ever presentable.
    state.set_panel_wire(client.session_protocol_version());

    let mut facts = ProbeFacts {
        session_protocol_version: client.session_protocol_version(),
        baseline_protocol_version: client.baseline_protocol_version(),
        ..ProbeFacts::default()
    };
    if let Some((geometry_epoch, total)) = state.next_geometry_declaration(GeometryTrigger::Surface)
        && client
            .send_frontend_cell_geometry(geometry_epoch, total)
            .is_ok()
    {
        facts.panel_declarations += 1;
    }

    // Ask the daemon to open the acceptance terminal in THIS frontend's
    // window. Going through a real key press is the point: the daemon's
    // `terminal.open` targets the invoking frontend, so this is what
    // puts the attached window on a terminal buffer.
    if let Some(chord) = std::env::var_os("PMACS_GPU_PROBE_OPEN_KEY")
        .and_then(|value| value.into_string().ok())
        .and_then(|value| value.chars().next())
    {
        let _ = client.send_key(ProtocolKey::Char(chord), Modifiers::CTRL | Modifiers::ALT);
    }

    // Quiet-observation mode. `PMACS_GPU_PROBE_OBSERVE_MS` makes the probe
    // send NO input and request NO resize, and observe for exactly that long
    // instead of stopping at its usual condition.
    //
    // This exists because the ordinary probe cannot see a frame storm: it
    // stops as soon as it has watched a resize land, so a session emitting a
    // frame every tick and one emitting three in total both satisfy it. A
    // fixed window over a child that produces no output turns "how many
    // frames did the daemon send?" into a number worth asserting on.
    let observe_window = std::env::var("PMACS_GPU_PROBE_OBSERVE_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(std::time::Duration::from_millis);
    // Normal probes stop only after their fixture-specific evidence arrives.
    // A producer fixture names the text it must paint; an input fixture uses
    // the latched echo observation. Keeping that choice outside this generic
    // runner prevents one fixture's breadcrumb from forcing another fixture
    // to sit on the 20-second safety deadline.
    let expected_frame_text = std::env::var("PMACS_GPU_PROBE_EXPECT_TEXT")
        .ok()
        .filter(|value| !value.is_empty());
    // A panel fixture's evidence is text in the BAND, not in a full-window
    // terminal. Naming it separately keeps one fixture's breadcrumb from
    // satisfying another's loop exit — the leak that made a Vterm probe pass
    // on its safety deadline.
    let expected_panel_text = std::env::var("PMACS_GPU_PROBE_EXPECT_PANEL_TEXT")
        .ok()
        .filter(|value| !value.is_empty());
    let quiet = observe_window.is_some();
    let deadline = std::time::Instant::now()
        + observe_window.unwrap_or_else(|| std::time::Duration::from_secs(20));
    let mut sent_input = false;
    let mut sent_resize = false;
    let mut completion_observed = false;
    while std::time::Instant::now() < deadline {
        let Ok(event) = rx.recv_timeout(std::time::Duration::from_millis(200)) else {
            continue;
        };
        match event {
            AttachEvent::Disconnected(reason) => {
                facts.disconnect = Some(reason);
                break;
            }
            AttachEvent::Message(msg) => {
                let is_snapshot = matches!(*msg, InstanceMessage::BufferSnapshot { .. });
                if let InstanceMessage::TerminalFrame(frame) = msg.as_ref() {
                    facts.frames += 1;
                    facts.last_frame_cols = frame.size.cols;
                    facts.last_frame_rows = frame.size.rows;
                    facts.last_frame_text = frame_probe_text(frame);
                    facts.last_title.clone_from(&frame.title);
                }
                match msg.as_ref() {
                    InstanceMessage::PanelFrame(
                        pmacs_protocol::panel::PanelFramePayload::Present(frame),
                    ) => {
                        facts.panel_frames += 1;
                        facts.panel_rows = frame.size.rows;
                        facts.panel_cols = frame.size.cols;
                        facts.panel_focused = frame.focused;
                        facts.panel_frame_text = grid_probe_text(&frame.cells);
                        if let Some(expected) = expected_panel_text.as_deref()
                            && facts.panel_frame_text.contains(expected)
                        {
                            facts.panel_text_observed = true;
                        }
                    }
                    InstanceMessage::PanelFrame(
                        pmacs_protocol::panel::PanelFramePayload::Absent,
                    ) => facts.panel_absent_observed = true,
                    _ => {}
                }
                state.apply_attach_message(*msg);
                if is_snapshot {
                    // The dual declaration: a byte viewport for a
                    // document, a cell size for a terminal. The daemon
                    // keeps whichever matches.
                    if let Some(buffer_id) = state.current_buffer_id {
                        let (start, end) = state.view_range;
                        let _ = client.send_viewport(
                            buffer_id,
                            pmacs_protocol::ByteRange { start, end },
                            0,
                        );
                    }
                    if let Some((buffer_id, size)) = state.terminal_declaration_if_changed()
                        && client.send_terminal_resize(buffer_id, size).is_ok()
                    {
                        facts.declarations += 1;
                        state.note_terminal_declaration_sent(buffer_id, size);
                    }
                }
                if state.terminal.is_some() {
                    facts.entered_terminal_mode = true;
                    // Render a REAL frame through the real composition
                    // path, then record that it composited.
                    let pixels = state.render_offscreen();
                    let first = pixels.first().copied().unwrap_or_default();
                    if pixels.iter().any(|&b| b != first) {
                        facts.rendered_nonuniform_frames += 1;
                    }
                    if facts.last_frame_text.contains(PROBE_INPUT_CHAR) {
                        facts.input_echo_observed = true;
                    }
                    if !quiet && !sent_input && facts.frames >= 1 {
                        sent_input = true;
                        // Real child input over the real wire.
                        let _ =
                            client.send_key(ProtocolKey::Char(PROBE_INPUT_CHAR), Modifiers::NONE);
                        let _ = client.send_key(ProtocolKey::Enter, Modifiers::NONE);
                    }
                    if !quiet && !sent_resize && facts.frames >= 2 {
                        sent_resize = true;
                        state.resize(700, 500);
                        if let Some((buffer_id, size)) = state.terminal_declaration_if_changed()
                            && client.send_terminal_resize(buffer_id, size).is_ok()
                        {
                            facts.declarations += 1;
                            facts.resized_cols = size.cols;
                            facts.resized_rows = size.rows;
                            state.note_terminal_declaration_sent(buffer_id, size);
                        }
                    }
                    if facts.resized_cols > 0 && facts.last_frame_cols == facts.resized_cols {
                        facts.observed_resized_frame = true;
                    }
                }
                // A panel fixture's document window is NOT a terminal — the
                // terminal lives in the band — so the arm above never fires
                // and its resize/composite evidence never arrives. The band
                // gets the same treatment against its own observations.
                if state.panel.presented().is_some() {
                    let pixels = state.render_offscreen();
                    let first = pixels.first().copied().unwrap_or_default();
                    if pixels.iter().any(|&b| b != first) {
                        facts.rendered_nonuniform_frames += 1;
                    }
                    if !quiet && !sent_input && facts.panel_frames >= 1 {
                        sent_input = true;
                        let _ =
                            client.send_key(ProtocolKey::Char(PROBE_INPUT_CHAR), Modifiers::NONE);
                        let _ = client.send_key(ProtocolKey::Enter, Modifiers::NONE);
                    }
                    if !quiet && !sent_resize && facts.panel_frames >= 2 {
                        sent_resize = true;
                        state.resize(700, 500);
                        if let Some((geometry_epoch, total)) =
                            state.next_geometry_declaration(GeometryTrigger::Surface)
                            && client
                                .send_frontend_cell_geometry(geometry_epoch, total)
                                .is_ok()
                        {
                            facts.panel_declarations += 1;
                            facts.panel_resized_cols = total.cols;
                        }
                    }
                    if facts.panel_resized_cols > 0 && facts.panel_cols == facts.panel_resized_cols
                    {
                        facts.panel_observed_resized_frame = true;
                    }
                }
                let fixture_evidence_observed = expected_frame_text.as_deref().map_or_else(
                    || facts.input_echo_observed,
                    |expected| facts.last_frame_text.contains(expected),
                );
                // Do not exit merely because resize/composition happened
                // first: that races the fixture's required PTY evidence and
                // produces a self-contradictory "successful" probe report
                // whose later acceptance assertion must reject it.
                if expected_panel_text.is_some() {
                    // The panel fixture's completion, stated in its own terms.
                    // `panel_text_observed` alone is not enough: it would let a
                    // pass happen before the band ever composited or the resize
                    // round-tripped, and this acceptance is exactly about the
                    // band being real.
                    if !quiet
                        && facts.panel_text_observed
                        && facts.panel_observed_resized_frame
                        && facts.rendered_nonuniform_frames >= 2
                    {
                        completion_observed = true;
                        break;
                    }
                } else if !quiet
                    && facts.observed_resized_frame
                    && facts.rendered_nonuniform_frames >= 2
                    && fixture_evidence_observed
                {
                    completion_observed = true;
                    break;
                }
            }
        }
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "session_protocol_version={}",
        facts.session_protocol_version
    );
    let _ = writeln!(
        out,
        "baseline_protocol_version={}",
        facts.baseline_protocol_version
    );
    let _ = writeln!(out, "panel_declarations={}", facts.panel_declarations);
    let _ = writeln!(out, "panel_frames={}", facts.panel_frames);
    let _ = writeln!(out, "panel_resized_cols={}", facts.panel_resized_cols);
    let _ = writeln!(
        out,
        "panel_observed_resized_frame={}",
        facts.panel_observed_resized_frame
    );
    let _ = writeln!(out, "panel_rows={}", facts.panel_rows);
    let _ = writeln!(out, "panel_cols={}", facts.panel_cols);
    let _ = writeln!(out, "panel_focused={}", facts.panel_focused);
    let _ = writeln!(out, "panel_text_observed={}", facts.panel_text_observed);
    let _ = writeln!(out, "panel_absent_observed={}", facts.panel_absent_observed);
    let _ = writeln!(
        out,
        "panel_frame_text_hex={}",
        hex_bytes(facts.panel_frame_text.as_bytes())
    );
    let _ = writeln!(out, "declarations={}", facts.declarations);
    let _ = writeln!(out, "frames={}", facts.frames);
    let _ = writeln!(
        out,
        "rendered_nonuniform_frames={}",
        facts.rendered_nonuniform_frames
    );
    let _ = writeln!(out, "entered_terminal_mode={}", facts.entered_terminal_mode);
    let _ = writeln!(
        out,
        "observed_resized_frame={}",
        facts.observed_resized_frame
    );
    let _ = writeln!(out, "last_frame_rows={}", facts.last_frame_rows);
    let _ = writeln!(out, "last_frame_cols={}", facts.last_frame_cols);
    let _ = writeln!(out, "resized_rows={}", facts.resized_rows);
    let _ = writeln!(out, "resized_cols={}", facts.resized_cols);
    let _ = writeln!(out, "last_title={}", facts.last_title.unwrap_or_default());
    let _ = writeln!(out, "last_frame_text={}", facts.last_frame_text);
    let _ = writeln!(out, "input_echo_observed={}", facts.input_echo_observed);
    let _ = writeln!(out, "completion_observed={completion_observed}");
    let _ = writeln!(out, "disconnect={}", facts.disconnect.unwrap_or_default());
    if let Err(error) = std::fs::write(report, out) {
        eprintln!(
            "pmacs-gpu probe: writing {} failed: {error}",
            report.display()
        );
        return 5;
    }
    0
}

/// Exercise the real managed connector without creating a display.
///
/// After the first real `BufferSnapshot`, the probe writes `phase=ready` and
/// holds the session open until stdin reaches EOF. Lifecycle observations
/// refresh the report while held; EOF writes `phase=complete`.
#[allow(
    clippy::too_many_lines,
    reason = "one linear managed-connect and lifecycle observation probe"
)]
fn run_headless_managed_probe(
    socket: &Path,
    report: &Path,
    daemon_executable: &Path,
    initial_target: Option<InitialTargetPaths>,
) -> i32 {
    use std::io::Read as _;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    let (connector_tx, event_rx) = mpsc::channel::<AttachEvent>();
    let managed = match attach::connect_managed_with_target_and_sink(
        socket,
        daemon_executable,
        initial_target,
        move |event| connector_tx.send(event).is_ok(),
    ) {
        Ok(managed) => managed,
        Err(error) => {
            let contents = format!("phase=error\nerror={error}\n");
            let _ = write_probe_report(report, &contents);
            eprintln!("pmacs-gpu managed probe: attach failed: {error}");
            return 4;
        }
    };
    let mut client = managed.client;
    let initial_message = client.take_initial_message();
    let initial_target_ready = matches!(
        initial_message.as_ref(),
        Some(InstanceMessage::BufferSnapshot { .. })
    );
    let mut buffer_facts = ManagedProbeBufferFacts::default();
    if let Some(message) = initial_message.as_ref()
        && let Err(error) = buffer_facts.observe(message)
    {
        let contents = format!("phase=error\nerror={error}\n");
        let _ = write_probe_report(report, &contents);
        eprintln!("pmacs-gpu managed probe: {error}");
        return 7;
    }
    let daemon = managed.daemon;
    let protocol = client.session_protocol_version();
    let baseline = client.baseline_protocol_version();

    let (stdin_tx, stdin_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("pmacs-gpu managed probe stdin".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            let _ = std::io::stdin().read_to_end(&mut bytes);
            let _ = stdin_tx.send(());
        })
        .expect("spawn managed probe stdin reader");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut ready = initial_target_ready;
    let mut stdin_closed = false;
    let mut disconnect = String::new();
    let mut last_reaped = false;
    let mut last_wait_result = None;
    let mut last_disconnect = String::new();
    if ready
        && let Err(error) = write_managed_probe_report(
            report,
            "ready",
            protocol,
            baseline,
            &daemon,
            &buffer_facts,
            &disconnect,
        )
    {
        eprintln!(
            "pmacs-gpu managed probe: writing {} failed: {error}",
            report.display()
        );
        return 5;
    }
    loop {
        if stdin_rx.try_recv().is_ok() {
            stdin_closed = true;
        }
        match event_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(AttachEvent::Message(message)) => {
                let is_snapshot = matches!(*message, InstanceMessage::BufferSnapshot { .. });
                if let Err(error) = buffer_facts.observe(&message) {
                    let contents = format!("phase=error\nerror={error}\n");
                    let _ = write_probe_report(report, &contents);
                    eprintln!("pmacs-gpu managed probe: {error}");
                    return 7;
                }
                if is_snapshot {
                    ready = true;
                    if let Err(error) = write_managed_probe_report(
                        report,
                        "ready",
                        protocol,
                        baseline,
                        &daemon,
                        &buffer_facts,
                        &disconnect,
                    ) {
                        eprintln!(
                            "pmacs-gpu managed probe: writing {} failed: {error}",
                            report.display()
                        );
                        return 5;
                    }
                }
            }
            Ok(AttachEvent::Disconnected(reason)) => disconnect = reason,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if disconnect.is_empty() {
                    "attach event channel closed".clone_into(&mut disconnect);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        let reaped = daemon.daemon_reaped();
        let wait_result = daemon.daemon_wait_result();
        if ready
            && (reaped != last_reaped
                || wait_result != last_wait_result
                || disconnect != last_disconnect)
        {
            if let Err(error) = write_managed_probe_report(
                report,
                "ready",
                protocol,
                baseline,
                &daemon,
                &buffer_facts,
                &disconnect,
            ) {
                eprintln!(
                    "pmacs-gpu managed probe: writing {} failed: {error}",
                    report.display()
                );
                return 5;
            }
            last_reaped = reaped;
            last_wait_result = wait_result;
            last_disconnect.clone_from(&disconnect);
        }

        if ready && stdin_closed {
            if let Err(error) = write_managed_probe_report(
                report,
                "complete",
                protocol,
                baseline,
                &daemon,
                &buffer_facts,
                &disconnect,
            ) {
                eprintln!(
                    "pmacs-gpu managed probe: writing {} failed: {error}",
                    report.display()
                );
                return 5;
            }
            return 0;
        }
        if !ready && Instant::now() >= deadline {
            let contents = format!(
                "phase=error\nerror=timed out waiting for BufferSnapshot\ndisconnect={disconnect}\n"
            );
            let _ = write_probe_report(report, &contents);
            eprintln!("pmacs-gpu managed probe: timed out waiting for BufferSnapshot");
            return 6;
        }
    }
}

#[derive(Default)]
struct ManagedProbeBufferFacts {
    snapshots: u32,
    last_snapshot_text: String,
}

impl ManagedProbeBufferFacts {
    fn observe(&mut self, message: &InstanceMessage) -> Result<(), String> {
        let InstanceMessage::BufferSnapshot { crdt_snapshot, .. } = message else {
            return Ok(());
        };
        let doc = loro::LoroDoc::new();
        doc.import(crdt_snapshot)
            .map_err(|error| format!("BufferSnapshot import failed: {error:?}"))?;
        self.snapshots += 1;
        self.last_snapshot_text = doc.get_text(LORO_TEXT_CONTAINER).to_string();
        Ok(())
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn write_managed_probe_report(
    report: &Path,
    phase: &str,
    protocol: u32,
    baseline: u32,
    daemon: &attach::ManagedDaemonFacts,
    buffer_facts: &ManagedProbeBufferFacts,
    disconnect: &str,
) -> std::io::Result<()> {
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = writeln!(out, "phase={phase}");
    let _ = writeln!(out, "session_protocol_version={protocol}");
    let _ = writeln!(out, "baseline_protocol_version={baseline}");
    let _ = writeln!(out, "buffer_snapshot=true");
    let _ = writeln!(out, "buffer_snapshots={}", buffer_facts.snapshots);
    let _ = writeln!(
        out,
        "last_snapshot_hex={}",
        hex_bytes(buffer_facts.last_snapshot_text.as_bytes())
    );
    let _ = writeln!(out, "spawned_daemon={}", daemon.spawned_daemon());
    let _ = writeln!(
        out,
        "daemon_pid={}",
        daemon.daemon_pid().unwrap_or_default()
    );
    let _ = writeln!(out, "daemon_reaped={}", daemon.daemon_reaped());
    let _ = writeln!(
        out,
        "daemon_wait_result={}",
        daemon.daemon_wait_result().unwrap_or_default()
    );
    let _ = writeln!(out, "disconnect={disconnect}");
    write_probe_report(report, &out)
}

fn write_probe_report(report: &Path, contents: &str) -> std::io::Result<()> {
    let mut temporary = report.as_os_str().to_os_string();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, contents)?;
    std::fs::rename(temporary, report)
}

/// Named observations the headless probe reports back to the acceptance.
#[allow(
    clippy::struct_excessive_bools,
    reason = "a flat report of independently-latched observations, not a state machine"
)]
#[derive(Default)]
struct ProbeFacts {
    /// The version the SESSION negotiated (this frontend's counter-offer).
    session_protocol_version: u32,
    /// The compatibility baseline the daemon advertised in `Hello`.
    ///
    /// Reported beside the negotiated version rather than instead of it:
    /// Stage 2B-3's whole activation claim is that these two DIFFER — the
    /// daemon still advertises a version every shipped frontend accepts
    /// while this session speaks the newer wire — and a report carrying
    /// only one of them cannot express that.
    baseline_protocol_version: u32,
    /// How many `Present` panel frames the daemon shipped.
    panel_frames: u32,
    /// The last `Present` band's grid, and whether it owned focus.
    panel_rows: u32,
    panel_cols: u32,
    panel_focused: bool,
    /// The band's own text, so an acceptance can prove the PTY child's output
    /// landed IN THE PANEL rather than in a full-window document terminal.
    panel_frame_text: String,
    /// Latched across frames: a reflow can push the breadcrumb off the last
    /// band, so "it arrived" and "it is still on the final band" are
    /// different questions and only the first is what the fixture means.
    panel_text_observed: bool,
    /// Whether an authoritative `Absent` was seen.
    panel_absent_observed: bool,
    /// The panel geometry declarations this probe sent.
    panel_declarations: u32,
    /// The band's grid after the probe's resize, and whether a band at that
    /// exact width was actually observed afterwards.
    panel_resized_cols: u32,
    panel_observed_resized_frame: bool,
    declarations: u32,
    frames: u32,
    rendered_nonuniform_frames: u32,
    entered_terminal_mode: bool,
    observed_resized_frame: bool,
    last_frame_rows: u32,
    last_frame_cols: u32,
    resized_rows: u32,
    resized_cols: u32,
    last_title: Option<String>,
    last_frame_text: String,
    /// Whether any frame carried the probe's own typed character back.
    ///
    /// Latched ACROSS frames, not read off the final one: a later geometry
    /// change reflows the screen, so "the echo arrived" and "the echo is
    /// still on the last frame" are different questions and only the first
    /// one is about input reaching the child.
    input_echo_observed: bool,
    disconnect: Option<String>,
}

/// The character the probe types into the child. Distinct from anything the
/// acceptance children print themselves, so its appearance is unambiguous.
const PROBE_INPUT_CHAR: char = 'x';

/// One-line printable text of a terminal frame, for probe reporting.
fn frame_probe_text(frame: &TerminalFrame) -> String {
    grid_probe_text(&frame.cells)
}

/// The same flattening for any wire cell grid, so a panel frame and a
/// terminal frame are read the same way rather than by two near-copies.
fn grid_probe_text(cells: &[pmacs_protocol::Cell]) -> String {
    let mut text = String::new();
    for cell in cells {
        match &cell.glyph {
            pmacs_protocol::Glyph::Char(ch) => text.push(*ch),
            pmacs_protocol::Glyph::Cluster(bytes) => {
                text.push_str(&String::from_utf8_lossy(bytes));
            }
            pmacs_protocol::Glyph::Continuation => {}
        }
    }
    text.retain(|ch| !ch.is_control());
    text
}

const GPU_USAGE: &str = "\
pmacs-gpu — GPU frontend for pmacs

NORMAL STARTUP:
  pmacs --gpu [--socket NAME|PATH] [FILE]   start/reuse a daemon and open FILE

ADVANCED DIRECT ATTACH:
  pmacs-gpu --attach <socket>               attach to an existing daemon only

OPTIONS:
  pmacs-gpu --help                          print this help
  pmacs-gpu --version                       print package and protocol versions";

/// Strict parser for direct, managed, and headless GPU entry points.
#[allow(
    clippy::too_many_lines,
    reason = "one exact-arity parser keeps private GPU entry points visibly fail-closed"
)]
fn parse_args(args: &[OsString]) -> Result<Mode, String> {
    fn option_like(value: &OsString) -> bool {
        value.as_os_str().as_bytes().starts_with(b"-")
    }

    fn reject_option_like(command: &str, operands: &[&OsString]) -> Result<(), String> {
        if let Some(operand) = operands.iter().find(|operand| option_like(operand)) {
            return Err(format!(
                "{command} received option-like path operand {}; prefix it with ./ if it is a path",
                operand.to_string_lossy()
            ));
        }
        Ok(())
    }

    fn target(cwd: &OsString, path: &OsString) -> InitialTargetPaths {
        InitialTargetPaths {
            cwd: PathBuf::from(cwd),
            path: PathBuf::from(path),
        }
    }

    if let Some(flag) = args.first()
        && option_like(flag)
        && flag.to_str().is_none()
    {
        return Err("option names must be valid UTF-8".to_owned());
    }

    match args {
        [flag] if flag == "--help" || flag == "-h" => Ok(Mode::Help),
        [flag] if flag == "--version" || flag == "-V" => Ok(Mode::Version),
        [flag, socket] if flag == "--attach" => {
            reject_option_like("--attach", &[socket])?;
            Ok(Mode::Attach {
                socket: PathBuf::from(socket),
            })
        }
        [flag, socket, daemon_executable] if flag == "--managed-attach" => {
            reject_option_like("--managed-attach", &[socket, daemon_executable])?;
            Ok(Mode::ManagedAttach {
                socket: PathBuf::from(socket),
                daemon_executable: PathBuf::from(daemon_executable),
                initial_target: None,
            })
        }
        [flag, socket, daemon_executable, marker, cwd, path]
            if flag == "--managed-attach" && marker == "--initial-target" =>
        {
            reject_option_like("--managed-attach", &[socket, daemon_executable, cwd])?;
            Ok(Mode::ManagedAttach {
                socket: PathBuf::from(socket),
                daemon_executable: PathBuf::from(daemon_executable),
                initial_target: Some(target(cwd, path)),
            })
        }
        [flag, socket, report] if flag == "--headless-probe" => {
            reject_option_like("--headless-probe", &[socket, report])?;
            Ok(Mode::HeadlessProbe {
                socket: PathBuf::from(socket),
                report: PathBuf::from(report),
            })
        }
        [flag, socket, report, daemon_executable] if flag == "--headless-managed-probe" => {
            reject_option_like(
                "--headless-managed-probe",
                &[socket, report, daemon_executable],
            )?;
            Ok(Mode::HeadlessManagedProbe {
                socket: PathBuf::from(socket),
                report: PathBuf::from(report),
                daemon_executable: PathBuf::from(daemon_executable),
                initial_target: None,
            })
        }
        [flag, socket, report, daemon_executable, marker, cwd, path]
            if flag == "--headless-managed-probe" && marker == "--initial-target" =>
        {
            reject_option_like(
                "--headless-managed-probe",
                &[socket, report, daemon_executable, cwd],
            )?;
            Ok(Mode::HeadlessManagedProbe {
                socket: PathBuf::from(socket),
                report: PathBuf::from(report),
                daemon_executable: PathBuf::from(daemon_executable),
                initial_target: Some(target(cwd, path)),
            })
        }
        [] => Err(
            "managed startup is provided by `pmacs --gpu`; direct use requires --attach <socket>"
                .to_owned(),
        ),
        [flag, ..] if flag == "--help" || flag == "-h" || flag == "--version" || flag == "-V" => {
            Err(format!(
                "{} does not accept operands",
                flag.to_string_lossy()
            ))
        }
        [flag, ..]
            if flag == "--attach"
                || flag == "--managed-attach"
                || flag == "--headless-probe"
                || flag == "--headless-managed-probe" =>
        {
            Err(format!(
                "{} received the wrong number of operands",
                flag.to_string_lossy()
            ))
        }
        [other, ..] => Err(format!(
            "unrecognized argument: {}",
            other.to_string_lossy()
        )),
    }
}

/// Top-level application handler. `state` is `Option` because winit
/// 0.30 builds the window in `resumed()`, not at `main()` start;
/// `attach_client` is held so the write half of the Unix stream
/// stays alive for as long as the window does.
struct App {
    mode: Mode,
    /// The event-loop proxy is taken in `resumed()` and handed to the
    /// reader thread. `Option` only because it can't be cloned out of
    /// a non-Option in a borrow.
    proxy: Option<winit::event_loop::EventLoopProxy<AppEvent>>,
    state: Option<State>,
    /// User events received before winit creates `state`. Managed attach
    /// starts its reader before `run_app`, so the initial snapshot may arrive
    /// before `resumed` on backends with a different callback order.
    pending_events: Vec<AppEvent>,
    /// Held both for stream lifetime and for the main loop's
    /// `send_viewport` / `send_key` write-back path.
    attach_client: Option<AttachClient>,
    /// Latest modifier state from winit (`ModifiersChanged`). winit
    /// delivers modifiers separately from key presses, so we track the
    /// current set and apply it when a key is sent (session B1).
    modifiers: winit::keyboard::ModifiersState,
}

fn defer_app_event(
    state_ready: bool,
    pending: &mut Vec<AppEvent>,
    event: AppEvent,
) -> Option<AppEvent> {
    if state_ready {
        Some(event)
    } else {
        pending.push(event);
        None
    }
}

type LoroTextDeltaBatches = Arc<Mutex<Vec<Vec<loro::TextDelta>>>>;

/// All resources owned by one running pmacs-gpu instance.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent render/input state flags, not a config bitset"
)]
struct State {
    // `None` in the headless render-test path (F-014): a windowless State
    // that renders to an offscreen texture instead of a surface.
    window: Option<Arc<Window>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: Option<wgpu::Surface<'static>>,
    config: wgpu::SurfaceConfiguration,
    font_system: FontSystem,
    /// What sanitized assembly retained (Q#F6): the default family
    /// and the bundled face ID — the total-fallback anchors for
    /// every font resolution.
    font_defaults: FontDefaults,
    /// Derived metrics for the current preference (Q#F6); default
    /// reproduces the BASE_* constants exactly.
    fm: FontMetrics,
    /// The family every shaped attrs run selects (Q#F6): the
    /// sanitized default at assembly; replaced only by a
    /// four-style-monospace-validated resolution in
    /// `apply_font_facts` — rejected requests fall back HERE, so
    /// the accessor is total.
    resolved_family: String,
    /// The resolved family's measured normal-face advance at the
    /// current code metrics (the [`ADVANCE_PROBE`] result, Q#F6).
    /// Authoritative for gutter geometry once a `FontFacts` has been
    /// applied; `None` until then, falling back to today's
    /// first-shaped-glyph sampling.
    measured_mono_advance: Option<f32>,
    /// The retained normalized code-buffer scroll (framing Q#F6):
    /// slice-local `line == 0` always holds, this is the `vertical`
    /// pixel residual within the top source line's visual runs, and
    /// `horizontal` stays 0 (glyphon 0.11 never applies it). Nonzero
    /// only after caret-following crossed into a wrapped run;
    /// explicit wheel/minimap jumps clear it, `BufferSnapshot`
    /// resets it (buffer-scoped view state), and every full reshape
    /// reapplies it instead of installing `Scroll::default`.
    code_scroll_residual: f32,
    /// Horizontal scroll offset of the code area, in **pixels** (Stage
    /// 5, framing Q#G1).
    ///
    /// Pixels rather than columns because the GPU's other viewport
    /// state already is (`code_scroll_residual` above), and a pixel
    /// offset composes with the clip rectangle without per-frame
    /// rounding. Column parity with the TUI is still exact: the code
    /// font is monospace by contract
    /// (`family_is_monospace_everywhere`), so `columns × advance` is a
    /// definition rather than an approximation.
    ///
    /// **Local viewport state, never sent.** Same category as
    /// `scroll_top`: no wire message and no protocol bump (§1.2).
    ///
    /// Reset to 0 on BOTH the wrap transition and a buffer snapshot
    /// (Q#G2) — inertness alone would let a stale offset reappear.
    code_scroll_left: f32,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    quad_renderer: QuadRenderer,
    squiggle_renderer: SquiggleRenderer,
    buffer: Buffer,
    /// What the buffer is currently shaped to. Held so we can detect
    /// no-op updates and skip the re-shape.
    current_text: String,
    /// Buffer-absolute byte offset for each source line in
    /// `current_text`. Updated with text changes and reused by
    /// reshape/scroll logic so those paths do not rescan the whole
    /// file on every semantic frame.
    current_line_starts: Vec<u64>,
    /// Buffer-absolute Unicode scalar offset for each source line in
    /// `current_text`. Loro's text event deltas use Unicode offsets
    /// on native builds, so this lets the CRDT hot path convert a
    /// retain/delete position to bytes by scanning only one source
    /// line instead of the whole prefix.
    current_line_char_starts: Vec<u64>,
    /// Code-shape data used to give the minimap horizontal structure
    /// even though `FileStyleSummary` carries only one dominant style
    /// per line. Summary replacement rebuilds the table; accepted text
    /// edits update the affected line immediately (or rebuild after
    /// structural/batched edits).
    current_line_shapes: Vec<MinimapLineShape>,
    /// Local CRDT replica seeded by `BufferSnapshot`. `None` in
    /// hello-world mode or before the first snapshot arrives in
    /// attach mode.
    loro_doc: Option<loro::LoroDoc>,
    /// Pending text diff batches captured from the local Loro replica.
    /// `CrdtOp` imports fire the subscription synchronously; the GPU
    /// drains these deltas and patches `current_text` incrementally
    /// instead of materializing the whole Loro text after each edit.
    loro_text_delta_batches: Option<LoroTextDeltaBatches>,
    /// Kept alive for as long as `loro_doc` is active. Dropping it
    /// unsubscribes before the next buffer snapshot replaces the doc.
    loro_text_subscription: Option<loro::Subscription>,
    /// Buffer the current rope text + spans interpret. Set when a
    /// `BufferSnapshot` arrives; used as the routing key for
    /// `StyleSpans` updates (drop those for other buffers).
    current_buffer_id: Option<BufferId>,
    /// Sorted-by-`range.start` styling spans for `current_buffer_id`.
    /// Replaced wholesale on `StyleSpans { full: true, .. }`; merged
    /// per the M11.4 dirty-segment rule on `full: false` (segments'
    /// ranges authoritatively replace styling within them; spans
    /// straddling a dirty edge get clipped to outside the dirty
    /// range).
    current_spans: Vec<StyleSpan>,
    /// Sorted-by-`range.start` decorations for `current_buffer_id`.
    /// Same M11.4 dirty-merge semantics as `current_spans`: `Decorations
    /// { full: true, .. }` replaces; `full: false` clips/replaces per
    /// segment range.
    ///
    /// Composition with `current_spans` in `reshape`: a decoration's
    /// color override beats the span's `style.fg` for the bytes it
    /// covers (semantic signal — a diagnostic — outranks syntactic
    /// signal). Decoration kinds whose visual is a background
    /// (`Selection`, `SearchMatch`, `SearchMatchActive`, `CurrentLine`)
    /// are not rendered in session 5; see the session-5 design note
    /// for the deferred quad-pipeline finding.
    current_decorations: Vec<Decoration>,
    /// Inline virtual text for `current_buffer_id` (session 6).
    /// Producer-side Phase A currently emits LSP inlay hints as
    /// `AtOffset` text adornments only. The GUI stores the whole scoped
    /// set and projects it into the shaped rich text without inserting
    /// bytes into `current_text`; source byte ranges for style spans and
    /// decorations therefore remain source-relative.
    current_adornments: Vec<InlineAdornment>,
    /// Whole-file per-line dominant styles for the minimap (session 7).
    /// The daemon emits this summary on first frame and after CRDT
    /// generation changes. We keep the latest summary until a newer one
    /// arrives, matching the ownership rule used by style spans,
    /// decorations, and inline adornments.
    current_summary: Option<FileStyleSummaryState>,
    /// Peer presence (session 9.3), keyed by source frontend id. Each
    /// entry is one *other* attached frontend's cursor + selection,
    /// delivered via `InstanceMessage::PresenceUpdate`. A read-only
    /// mirror has no cursor of its own (no input path), so its own
    /// `Selection` / `CurrentLine` decorations are inert; the editing
    /// peer's presence is what the user actually watches. The quad-
    /// background path renders `Selection` / `CurrentLine` washes from
    /// these entries rather than from `current_decorations`. Sender
    /// exclusion at the daemon means our own id never appears here.
    peer_presences: HashMap<FrontendId, PeerPresence>,
    /// This frontend's own cursor (session B1), from the daemon's
    /// `CursorByte`. pmacs-gpu sends `Key` events; the daemon moves the
    /// authoritative window cursor and reports it back here (Q#B3), so
    /// the caret follows whatever the daemon decided — including motion
    /// from commands this frontend never interprets locally. `None`
    /// until the first `CursorByte`.
    own_cursor: Option<OwnCursor>,
    /// Top visible *source line* (0-based). Scroll is line-based
    /// (Q#S1). `reshape` shapes only the lines from here through the
    /// visible window; `view_range` records the byte span actually fed
    /// to cosmic-text so caret/wash byte offsets can be rebased onto
    /// it.
    scroll_top: usize,
    /// Whole-file byte range `[vstart, vend)` of the slice the
    /// cosmic-text `buffer` currently holds (session S1). Everything
    /// the buffer renders is in slice coordinates (`file_byte -
    /// vstart`); this is the rebasing origin for the caret and the
    /// background washes.
    view_range: (u64, u64),
    /// Last `[vstart, vend)` declared to the daemon via a `Viewport`
    /// event. Re-declared only when it changes (scroll, edit that
    /// shifts visible bytes, buffer switch) so the producer scopes
    /// `StyleSpans` to what's on screen without per-frame churn (Q#S5).
    last_viewport_sent: Option<(u64, u64)>,
    /// Frontend id assigned by the daemon. Needed for locally-authored
    /// optimistic CRDT ops, whose Loro peer id must match the
    /// authenticated frontend id the daemon sees on the socket.
    local_frontend_id: Option<FrontendId>,
    /// Daemon-side key dispatcher state. Plain printable chars are
    /// optimistically applied only while this is true; when false,
    /// keys round-trip so minibuffer and prefix commands keep their
    /// daemon-owned semantics.
    dispatch_idle: bool,
    /// OS clipboard handle (Q#CM6), created lazily on first cut / copy /
    /// paste. `None` until first use or when the platform clipboard is
    /// unavailable (headless / unsupported compositor) --- clipboard ops
    /// then degrade to no-ops rather than crashing.
    clipboard: Option<arboard::Clipboard>,
    /// Whether `own_cursor` is still an authoritative position for
    /// local optimistic insertion. Round-tripped keys can move the
    /// daemon cursor in ways the GPU does not predict, so they mark
    /// this false until the next `CursorByte`.
    cursor_fresh: bool,
    /// Furthest locally-predicted cursor after optimistic inserts that
    /// the daemon has not yet confirmed. `CursorByte` frames already
    /// in flight can arrive after local typing; accepting one below
    /// this floor would rewind subsequent optimistic inserts and
    /// scramble their order.
    optimistic_cursor_floor: Option<OwnCursor>,
    /// Round-trip keys typed while optimistic inserts are still
    /// awaiting confirmation. Sending a backward-moving key before
    /// the floor is acknowledged would make its legitimate cursor
    /// result indistinguishable from an older in-flight frame.
    deferred_round_trip_keys: Vec<(ProtocolKey, Modifiers)>,
    /// When the current `optimistic_cursor_floor` was armed. If the
    /// daemon never confirms the prediction (op dropped by
    /// validation, a peer racing our window cursor), an unbounded
    /// floor would wedge deferred round-trip keys forever; after
    /// [`FLOOR_CONFIRM_TIMEOUT`] the floor releases, `cursor_fresh`
    /// drops, and the next `CursorByte` resynchronizes.
    optimistic_floor_set_at: Option<std::time::Instant>,
    /// Optimistic local edits not yet known to be reflected in
    /// incoming producer frames. Each entry pairs the version scalar
    /// of this replica's doc *after* the edit applied (computed by
    /// [`loro_version_scalar`], the same per-peer counter sum the
    /// daemon stamps into `StyleSpans` / `Decorations` `generation`)
    /// with the projection edit itself. On frame arrival, entries at
    /// or below the frame's generation are pruned and the frame's
    /// byte ranges are translated through the remainder — otherwise a
    /// frame computed before an in-flight keystroke repaints the
    /// viewport's colors a few bytes left of the text (the typing
    /// "color shimmer"). Cleared whenever the cache is rebuilt
    /// wholesale (snapshot / full-materialization fallback).
    ///
    /// Caveat (accepted): scalars from *divergent* replicas are not
    /// causally comparable, so a peer edit racing our unconfirmed
    /// ops can mis-prune by one frame; the next generation-keyed
    /// full resync self-corrects.
    unconfirmed_edits: Vec<(u64, TextProjectionEdit)>,
    /// Q#M2 — projected→source hit map for the currently shaped
    /// slice. Rebuilt by every `reshape` from the same chunks that
    /// feed glyphon; source offsets are slice-relative (pair with
    /// `view_range.0`).
    current_hit_runs: Vec<ProjectedRun>,
    /// Line-start byte offsets of the *projected* text (cosmic-text
    /// reports hits as line index + byte-within-line).
    projected_line_starts: Vec<u64>,
    /// Last reported pointer position, in window pixels.
    pointer_pos: Option<(f64, f64)>,
    /// Primary button is held after a Down inside the text area.
    pointer_drag_active: bool,
    /// Hit byte of the last Pointer event sent — Drag coalescing:
    /// pixel-rate motion only ships when the hit byte changes.
    last_pointer_sent_byte: Option<u64>,
    /// `(when, byte, chain_count)` of the last primary Down, for
    /// frontend-side multi-click detection (same-hit within the
    /// interval): count 1 = single, 2 = the double already fired,
    /// so the next same-hit press is a triple (Q#M4).
    last_pointer_down: Option<(std::time::Instant, u64, u8)>,
    /// A press began inside the minimap band (Q#M6): subsequent
    /// `CursorMoved` scrubs the viewport instead of dragging a
    /// selection, until release. Never sends `Pointer` events —
    /// the viewport is frontend-owned.
    minimap_scrub_active: bool,
    /// Q#M7 — `Some(±1)` while a drag sits in the top/bottom edge
    /// band; `about_to_wait` ticks the viewport one line toward the
    /// pointer per [`EDGE_SCROLL_TICK`] and re-runs the drag
    /// hit-test (the mouse may be stationary — `CursorMoved` alone
    /// would stall the selection).
    edge_scroll_dir: Option<i64>,
    /// When the last edge-scroll tick fired.
    edge_scroll_last: Option<std::time::Instant>,
    /// Q#M6 (bet #2) — a far jump rebuilt every visible line from
    /// spans that can't cover the new region; the redraw is held
    /// until restyle arrival (which clears this) or this deadline,
    /// whichever is first, so the unstyled frame usually never
    /// shows. `about_to_wait` enforces the deadline.
    styled_redraw_deadline: Option<std::time::Instant>,
    /// Q#R2 — the per-line surgery path skips rebuilding the pointer
    /// hit map (clicks are rare next to keystrokes); this marks it
    /// stale so `hit_test_source_byte` rebuilds on demand from the
    /// same shared chunk function.
    hit_map_dirty: bool,
    /// Per-shaped-line chunk cache: `line_chunk_cache[i]` is the
    /// chunk set `buffer.lines[i]` was built from. Lets incoming
    /// frames re-shape ONLY lines whose styling actually changed, and
    /// lets scroll reuse retained lines wholesale.
    line_chunk_cache: Vec<Vec<RichChunk>>,
    /// Per-shaped-line math suppression state, in lockstep with
    /// `line_chunk_cache`: the detected spans with the Q#MS5 gate bit
    /// each was built under (the line-reuse predicate's third input
    /// beside content and styling), and the placed boxes the draw
    /// pass paints into the reserved spacer rectangles.
    line_math_cache: Vec<MathLineState>,
    /// The MATH layout engine over the bundled font, or `None` when
    /// the font failed to yield math metrics at startup — a hard
    /// error in the math path only (Q#MS7): spans render as source
    /// and the editor keeps running.
    math_engine: Option<math_layout::MathLayout<'static>>,
    /// `(above_baseline, below_baseline)` budget a math box must fit
    /// (Q#MS10), derived by `math_layout::line_box_budget` from the
    /// CODE font's baseline placement. Recomputed when metrics change.
    math_budget: (f32, f32),
    /// Absolute source-line index of `buffer.lines[0]`.
    shaped_top: usize,
    bg_vertex_buffer: ReusableVertexBuffer,
    squiggle_vertex_buffer: ReusableVertexBuffer,
    caret_vertex_buffer: ReusableVertexBuffer,
    minimap_vertex_buffer: ReusableVertexBuffer,
    /// Q#S2/Q#SL10 — the status band's shaped right rich text.
    status_buffer: Buffer,
    /// Rich runs currently installed in the right status buffer.
    /// `None` is the invalidation sentinel; an empty vector is valid.
    status_runs: Option<Vec<(String, Color)>>,
    /// The independently left-aligned status buffer.
    status_left_buffer: Buffer,
    /// Rich runs currently installed in the left status buffer.
    status_left_runs: Option<Vec<(String, Color)>>,
    /// Latest atomically validated custom statusline replacement.
    statusline_segments: Option<StatuslineSegmentsLocal>,
    /// Q#S1 — the wire-authoritative status facts (protocol v8).
    status_facts: Option<StatusFactsLocal>,
    /// Q#SR5 — the live incremental-search prompt (protocol v9), or
    /// `None` when no search is running. While `Some`, the status
    /// band's left side shows `I-search: <query> (n/m)` in place of
    /// the buffer name; the matches highlight via `SearchMatch`
    /// decorations.
    search_prompt: Option<SearchPromptLocal>,
    /// Q#MB1 — the live minibuffer (protocol v12), or `None` when
    /// closed. The prompt+input render in the bottom band; the
    /// candidates (when present) render as a dropdown above it.
    minibuffer: Option<MinibufferLocal>,
    /// Q#CM1 — the live context menu (protocol v11), or `None` when
    /// closed. The rows + highlight come from `MenuPrompt`; the popup
    /// draws at the pixel of the right-click.
    menu: Option<MenuLocal>,
    /// Pixel of the most recent right-click, remembered so the
    /// `MenuPrompt` that follows can anchor the popup there.
    menu_anchor_px: (f64, f64),
    /// Shaped label text for the open menu (Q#CM1), one line per row.
    menu_buffer: Buffer,
    /// Dedicated text renderer for the menu, so its glyphs draw in a
    /// layer *over* the buffer text + caret (a popup), not interleaved
    /// with them in the main text pass.
    menu_text_renderer: TextRenderer,
    /// Popup background / highlight / separator quads (Q#CM1).
    menu_bg_vertex_buffer: ReusableVertexBuffer,
    /// Shaped candidate text for the minibuffer dropdown (Q#MB1), one
    /// line per candidate.
    mb_buffer: Buffer,
    /// Dedicated text renderer for the minibuffer dropdown (its own
    /// layer over the buffer, like the menu's).
    mb_text_renderer: TextRenderer,
    /// Dedicated text renderer for inline-math glyphs (Q#MS6): each
    /// `MathItem::Glyph` draws from its own mini-buffer positioned at
    /// layout's exact x/baseline, so an accumulated shaping advance
    /// can never move a glyph off its measured origin — the same
    /// per-run argument the terminal renderer made.
    math_text_renderer: TextRenderer,
    /// Minibuffer dropdown background + selection quads (Q#MB1).
    mb_bg_vertex_buffer: ReusableVertexBuffer,
    /// Arc 1a Q#C5 — the live in-buffer completion popup (protocol
    /// v15), or `None` when closed.
    completion: Option<CompletionLocal>,
    /// Shaped row text for the completion dropdown, one line per
    /// candidate ("glyph label  detail").
    completion_buffer: Buffer,
    /// Dedicated text renderer for the completion dropdown (its own
    /// layer over the buffer, like the menu's / minibuffer's).
    completion_text_renderer: TextRenderer,
    /// Completion dropdown background + selection quads.
    completion_bg_vertex_buffer: ReusableVertexBuffer,
    /// Minimap vertex bytes cached by [`MinimapCacheKey`] —
    /// rebuilding rescanned every line shape per frame.
    minimap_cache: Option<(MinimapCacheKey, Vec<u8>)>,
    /// Line-number gutter mode (UX gutter arc, GPU side). Shipped by the
    /// daemon over `InstanceMessage::LineNumbers` (protocol v14). `Off` ⇒
    /// zero coordinate change: `gutter_width_px()` is 0 and every shift site
    /// is a no-op. Relative/Hybrid are rendered locally against the GPU's
    /// own cursor line.
    line_numbers: LineNumberMode,
    /// Shaped right-aligned line numbers, one per visible code line — its
    /// own text layer over the code, aligned row-for-row (same line height).
    gutter_buffer: Buffer,
    /// Dedicated renderer for the gutter number layer (like the menu / mb).
    gutter_text_renderer: TextRenderer,
    /// The daemon-resolved UI face table (themes arc Q#TH7, protocol
    /// v16). Exact-name lookup only — inheritance is resolved
    /// daemon-side, so the frontend never walks. Complete replacement
    /// per `ThemeFacts`; a face absent from the map means "use the
    /// site's hardcoded default". Applied per draw through the
    /// `face_fg_or` / `face_wash_or` / `modeline_face_colors` /
    /// `diag_face_rgba` resolvers (Q#TH5 mask + `Default` mapping).
    faces: HashMap<String, CellStyle>,
    /// Vterm Stage 3 — the installed terminal frame and its derived
    /// paint plan, or `None` in document mode. The two-state machine is
    /// explicit: `BufferSnapshot` always leaves terminal mode, a valid
    /// matching `TerminalFrame` always enters it.
    terminal: Option<TerminalLocal>,
    /// Bottom-panel arc Stage 2B-3: this frontend's panel band — the
    /// retained frame, its geometry declaration, the divider drag, and the
    /// exhaustion latch. Present on every `State`, inert until a session
    /// negotiates the panel wire.
    panel: PanelBand,
    /// One shaped buffer per planned panel run.
    panel_text_buffers: Vec<Buffer>,
    /// Whether the negotiated session carries the panel wire at all.
    ///
    /// Keyed on the NEGOTIATED version, never the `Hello` baseline.
    panel_wire: bool,
    /// Set when a font/scale transaction has invalidated the panel's
    /// geometry declaration, so the caller that owns the client knows to
    /// re-declare under a `Metrics` trigger.
    ///
    /// A flag rather than a direct send because `apply_message` cannot
    /// reach the attach client, and because the distinction it carries —
    /// `Metrics` versus `Surface` — is exactly what stops an identical
    /// `CellSize` from being deduped away.
    panel_metrics_changed: bool,
    /// The terminal geometry last declared to the daemon, with the
    /// buffer it described. Suppresses an unchanged re-declaration and
    /// forces a fresh one after a buffer switch.
    last_terminal_size_sent: Option<(BufferId, CellSize)>,
    /// Whether an invalid terminal frame has already been reported.
    /// Bounds the log while a bad producer keeps sending.
    terminal_frame_error_latched: bool,
    /// Last terminal cell a motion or drag was reported at. Pixel-rate
    /// motion inside ONE cell is not new information for the daemon —
    /// the document drag path dedupes by hit byte for the same reason.
    /// Cleared on press, release, and every exit from terminal mode, so
    /// a gesture that returns to the same cell still reports.
    last_terminal_pointer_cell: Option<CellCoord>,
    /// One shaped buffer per planned text run. Rebuilt only when the
    /// plan changes, never per frame.
    terminal_text_buffers: Vec<Buffer>,
    /// Dedicated renderer for terminal glyphs, so they draw in their own
    /// layer with terminal clipping rather than through the document
    /// text pass.
    terminal_text_renderer: TextRenderer,
    /// The band's own glyph layer.
    panel_text_renderer: TextRenderer,
}

/// Vterm Stage 3 — the GPU's terminal mode.
struct TerminalLocal {
    /// The terminal identity buffer this frame describes.
    buffer_id: BufferId,
    /// The last valid frame. Retained verbatim so an identical
    /// re-send can be recognized and skipped without a rebuild.
    frame: TerminalFrame,
    /// Cell-space paint data derived from `frame`.
    plan: TerminalPaintPlan,
}

/// Why frame geometry is being (re-)declared (Q#BP2S1).
///
/// The two arms differ in exactly one way — whether an *identical*
/// [`CellSize`] still advances the epoch — and that difference is the whole
/// reason the epoch is frontend-owned. Collapsing them into one call site
/// reintroduces the bug option 1 was chosen to avoid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GeometryTrigger {
    /// A surface resize, or the first declaration after attach. An
    /// identical cell total means nothing the daemon can act on changed,
    /// so it is not re-declared.
    Surface,
    /// A font family, size, or scale change. The cell total may be
    /// **identical** while the pixels behind it are not, which is exactly
    /// what daemon-side value dedup cannot see — so this always advances.
    Metrics,
}

/// One `PanelResizeRows` request a drag has decided to make.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PanelResizeRequest {
    geometry_epoch: u64,
    panel_epoch: u64,
    rows: u32,
}

/// Which surface a pointer pixel belongs to (Q#BP16).
///
/// **One authority for "does the band claim this pixel", consulted by every
/// pointer handler.** Four handlers route gestures — motion, left
/// press/release, right press, and wheel — and the band has to be consulted
/// first in all four. When each handler decided for itself, three of them
/// simply did not ask, so right-click and wheel fell through to the document
/// underneath and a held left button was reported as a hover. A single
/// classifier makes forgetting the band impossible to do quietly, and makes
/// the routing testable without a window or a daemon.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PointerSurface {
    /// The divider strip: a drag handle, never a cell gesture.
    PanelDivider,
    /// A cell inside the band.
    PanelCell(CellCoord),
    /// The band's fractional right-edge remainder, or anywhere else in the
    /// band that maps to no cell. Distinct from `Elsewhere` because the band
    /// still owns the pixel — it just emits no `PanelPointer`.
    PanelBackground,
    /// Not the band: the document, the terminal, the minimap, or the chrome.
    Elsewhere,
}

/// A live divider drag (Q#BP15a, parent acceptance 47).
#[derive(Clone, Copy, Debug)]
struct PanelDrag {
    /// Presentation the gesture started against. A drag that outlives its
    /// panel is dropped rather than applied to the successor.
    panel_epoch: u64,
    /// Geometry declaration the gesture is measured against.
    geometry_epoch: u64,
    /// Rows the panel had when the drag started.
    start_rows: u32,
    /// Pointer y where the drag started, in surface pixels.
    start_y: f32,
    /// Last row count actually sent, so a drag that re-crosses the same
    /// row boundary does not re-send it.
    sent_rows: u32,
}

/// The GPU frontend's half of the bottom panel (Q#BP15, Q#BP15a, Q#BP16).
#[derive(Default)]
struct PanelBand {
    /// The last **valid** frame received, retained until an authoritative
    /// `Absent`.
    ///
    /// Silence is not absence: the daemon must send `Absent` explicitly on
    /// close *and* on hide, and until it does, this is what paints. An
    /// invalid frame is rejected whole and leaves this untouched.
    frame: Option<PanelFrame>,
    /// This frontend's monotonic geometry declaration id. `0` means never
    /// declared, which the wire rejects.
    geometry_epoch: u64,
    /// The cell total behind `geometry_epoch`, for the `Surface` dedup.
    declared: Option<CellSize>,
    /// The advance the declaration was computed with, retained so painting
    /// and hit-testing resolve cells with the **same** number the daemon's
    /// column count was derived from.
    ///
    /// Caching it is not an optimization. The declaration needs
    /// `&mut FontSystem` to shape its probe, so a `&self` painter cannot
    /// re-derive it and would reach for `mono_advance` — which is
    /// document-dependent. The declared grid, the painted grid, and the
    /// hit-tested grid would then be three different grids, and a test that
    /// asserted only the declaration would not see it. One value behind the
    /// declaration makes all three agree *by construction*.
    declared_advance: Option<f32>,
    /// Terminal exhaustion latch (framing §3.1).
    ///
    /// Once set: no further declaration is sent, and no retained frame
    /// paints or hit-tests **however well its epoch still matches**. The
    /// latch is what stops an old `Present` from resurrecting a band under
    /// geometry this frontend has disowned; only a fresh session clears
    /// it, because only a fresh session builds a fresh `PanelBand`.
    exhausted: bool,
    /// Live divider drag. One pointer, one gesture.
    drag: Option<PanelDrag>,
    /// Whether a left-button gesture that started INSIDE the band is still
    /// held. Mirrors `pointer_drag_active` for the document: without it a
    /// motion event cannot tell a hover from a drag, so `Drag(Left)` is never
    /// emitted and panel selection cannot work at all.
    pointer_held: bool,
    /// Last cell a panel pointer gesture reported, so sub-cell motion does
    /// not become pixel-rate wire traffic. Reset on every press and release,
    /// because the first drag after a press must reach the daemon even at the
    /// cell the press landed on.
    last_pointer_cell: Option<CellCoord>,
    /// Whether the pointer is currently over the divider strip, which
    /// decides the `RowResize` cursor icon.
    hover_divider: bool,
    /// Cell-space paint data derived from `frame`, rebuilt on receipt so
    /// the render path never re-derives it per frame.
    plan: Option<TerminalPaintPlan>,
}

impl PanelBand {
    /// **The** single derivation of "is there a band on screen right now".
    ///
    /// Every consumer goes through this: the band inset the three
    /// boundaries are computed from, the painter, the hit-tester, and the
    /// drag. 2B-2's review found that two derivations of one panel
    /// predicate is precisely how the renderer and the durable state come
    /// to disagree, so there is exactly one here too.
    ///
    /// Three conditions, closing three different holes:
    ///
    /// * a retained valid frame exists (silence retains, `Absent` clears);
    /// * its `geometry_epoch` matches the current declaration — after a
    ///   new declaration is sent, an older retained frame neither paints
    ///   nor accepts input until a matching `Present` arrives (parent 41);
    /// * the exhaustion latch is clear.
    fn presented(&self) -> Option<&PanelFrame> {
        if self.exhausted {
            return None;
        }
        self.frame
            .as_ref()
            .filter(|frame| frame.geometry_epoch == self.geometry_epoch)
    }
}

/// Kind-glyph column for a completion row: the LSP
/// `CompletionItemKind` numeric code → the single-char glyph the TUI
/// popup uses (`crate::completion::CompletionItemKind::glyph`'s
/// mapping, replicated — the GPU crate doesn't depend on `pmacs`).
/// Unknown codes fall back to the plain-text dot, per the LSP
/// "accept extended kinds gracefully" contract.
fn completion_kind_glyph(kind: u8) -> char {
    match kind {
        2..=4 => 'f',   // method / function / constructor
        5 | 10 => 'p',  // field / property
        6 | 21 => 'v',  // variable / constant
        7 | 22 => 'C',  // class / struct
        8 => 'I',       // interface
        9 => 'M',       // module
        13 | 20 => 'E', // enum / enum member
        14 => 'k',      // keyword
        15 => 's',      // snippet
        25 => 't',      // type parameter
        17 | 19 => '/', // file / folder
        _ => '.',
    }
}

/// Latest validated custom statusline replacement (Q#SL7/Q#SL10).
#[derive(Clone, Debug, PartialEq, Eq)]
struct StatuslineSegmentsLocal {
    buffer_id: BufferId,
    left: Vec<StatuslineSegment>,
    right: Vec<StatuslineSegment>,
}
/// Validate the complete untrusted statusline payload before any state
/// changes. Numeric and namespace policy lives only in pmacs-protocol.
fn validate_statusline_segments(
    left: &[StatuslineSegment],
    right: &[StatuslineSegment],
) -> Result<(), &'static str> {
    let count = left
        .len()
        .checked_add(right.len())
        .ok_or("segment count overflow")?;
    if count > MAX_STATUSLINE_PROVIDERS {
        return Err("too many segments");
    }

    let mut total_text_bytes = 0usize;
    for segment in left.iter().chain(right) {
        if segment.text.is_empty() {
            return Err("empty segment text");
        }
        if segment.text.len() > MAX_STATUSLINE_SEGMENT_BYTES {
            return Err("segment text too long");
        }
        if segment.text.chars().any(char::is_control) {
            return Err("segment text contains a control character");
        }
        total_text_bytes = total_text_bytes
            .checked_add(segment.text.len())
            .ok_or("total text length overflow")?;
        if total_text_bytes > MAX_STATUSLINE_TOTAL_TEXT_BYTES {
            return Err("total segment text too long");
        }
        if segment.face.len() > MAX_STATUSLINE_FACE_BYTES {
            return Err("segment face too long");
        }
        if segment.face.chars().any(char::is_control) {
            return Err("segment face contains a control character");
        }
        if !is_modeline_face_name(&segment.face) {
            return Err("segment face is outside ui.modeline");
        }
    }
    Ok(())
}

/// The wire-authoritative status facts (Q#S1, protocol v8; `message`
/// since v15), mirrored from `InstanceMessage::StatusFacts`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct StatusFactsLocal {
    buffer_id: BufferId,
    name: String,
    modified: bool,
    diag_errors: u32,
    diag_warnings: u32,
    /// The core's transient status message (`pmacs.editor.set_status`
    /// — "12 references", LSP errors, ...), or `None` when clear.
    message: Option<String>,
}

/// The live incremental-search prompt (Q#SR5/Q#RX6, protocol v10),
/// mirrored from a `SearchPrompt` message whose `query` was `Some`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SearchPromptLocal {
    buffer_id: BufferId,
    query: String,
    active: Option<u32>,
    total: u32,
    regex: bool,
    invalid: bool,
}

/// The live minibuffer (Q#MB1, protocol v12), mirrored from whichever
/// minibuffer variant this session's negotiated version carries, when
/// its `prompt` was `Some`. The prompt+input draw in the bottom band
/// with a caret; `rows` (a windowed slice) feed the dropdown.
///
/// **One local shape for two wire variants.** A `>= 23` daemon sends
/// `MinibufferPromptRows` with per-row details; a `12..=22` daemon sends
/// the frozen `MinibufferPrompt` with bare strings, which land here as
/// rows whose `detail` is `None`. Both are live: this binary offers its
/// own `PROTOCOL_VERSION` only when the daemon advertises the current
/// baseline, and echoes an older baseline verbatim — so an older daemon
/// still negotiates an older session, and the legacy arm is reachable
/// rather than dead code.
#[derive(Clone, Debug, PartialEq)]
struct MinibufferLocal {
    prompt: String,
    input: String,
    cursor: u32,
    rows: Vec<MinibufferRow>,
    selected: Option<u32>,
    total: u32,
}

/// The live in-buffer completion popup (Arc 1a Q#C5, protocol v15),
/// mirrored from a `CompletionPopup` whose `anchor` was `Some`. The
/// dropdown anchors at the glyph rect of `anchor` (a byte offset —
/// the caret mapping reused), one row per candidate; navigation and
/// accept round-trip into the daemon's completion shadow.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CompletionLocal {
    /// Buffer the popup targets. Rendering and the key gates check
    /// this against `current_buffer_id` so a popup can never act
    /// against a buffer it wasn't opened in (buffer switches also
    /// clear the whole mirror at the `BufferSnapshot` arm; this is
    /// the belt to that suspender).
    buffer_id: BufferId,
    /// Byte offset of the prefix start.
    anchor: u64,
    /// Bytes of typed prefix at `anchor` (reserved for a bolded-
    /// prefix refinement; unused by the first render).
    #[allow(
        dead_code,
        reason = "shipped on the wire for the bolded-prefix refinement"
    )]
    prefix_len: u32,
    /// Windowed candidate rows (label / kind / detail), best-first.
    rows: Vec<CompletionPopupRow>,
    /// Highlighted row within `rows`.
    selected: Option<u32>,
    /// Total candidate count (reserved for an "i/total" hint).
    #[allow(
        dead_code,
        reason = "shipped on the wire for the i/total hint refinement"
    )]
    total: u32,
}

/// The live context menu (Q#CM1, protocol v11), mirrored from a
/// `MenuPrompt` with non-empty rows. The popup draws at `anchor_px`
/// (the right-click pixel, remembered locally — the daemon never sees
/// pixels).
#[derive(Clone, Debug, PartialEq)]
struct MenuLocal {
    rows: Vec<MenuPromptRow>,
    active: Option<u32>,
    anchor_px: (f64, f64),
}

/// pmacs-gpu's own cursor position, mirrored from `CursorByte`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OwnCursor {
    buffer_id: BufferId,
    byte: u64,
}

/// One peer frontend's cursor + selection in a buffer, from
/// `InstanceMessage::PresenceUpdate`. Byte offsets are in the buffer's
/// coordinate space; the renderer maps them to glyph rectangles via
/// the local layout and clamps to text length, so a presence that
/// briefly lags an edit can never index out.
#[derive(Clone, Copy, Debug)]
struct PeerPresence {
    buffer_id: BufferId,
    cursor: u64,
    selection: Option<SelectionSnapshot>,
}

struct QuadRenderer {
    pipeline: wgpu::RenderPipeline,
}

#[derive(Clone, Debug)]
struct FileStyleSummaryState {
    generation: u64,
    lines: Vec<CellStyle>,
}

impl App {
    /// Ship a Pointer event if the daemon speaks protocol v5+ — the
    /// Q#M1 frontend-side gate (an older instance cannot decode the
    /// variant and would drop the connection).
    fn send_pointer(&self, buffer_id: BufferId, byte: u64, kind: PointerKind, mods: Modifiers) {
        let Some(client) = self.attach_client.as_ref() else {
            return;
        };
        if client.session_protocol_version() < 5 {
            return;
        }
        // TripleDown is a v7 variant; a pre-v7 instance would
        // hard-error decoding it. Downgrade to a plain Down — the
        // exact behavior the third click had before v7 (the chain
        // restarting).
        let kind = if kind == PointerKind::TripleDown && client.session_protocol_version() < 7 {
            PointerKind::Down
        } else {
            kind
        };
        // Context (right-click, Q#CM1) is a v11 variant; a pre-v11
        // instance can't open a menu, so drop the gesture rather than
        // sending an undecodable variant.
        if kind == PointerKind::Context && client.session_protocol_version() < 11 {
            return;
        }
        if let Err(e) = client.send_pointer(buffer_id, byte, kind, mods) {
            eprintln!("pmacs-gpu: send_pointer failed: {e}");
        }
    }

    /// Ship a terminal-cell gesture if the daemon speaks v19+.
    ///
    /// The terminal twin of [`Self::send_pointer`]: same frontend-side
    /// version gate, cells instead of source bytes.
    fn send_terminal_pointer(
        &self,
        buffer_id: BufferId,
        coord: CellCoord,
        kind: ProtocolMouseKind,
        mods: Modifiers,
    ) {
        let Some(client) = self.attach_client.as_ref() else {
            return;
        };
        if client.session_protocol_version() < 19 {
            return;
        }
        if let Err(e) = client.send_terminal_pointer(buffer_id, coord, kind, mods) {
            eprintln!("pmacs-gpu: send_terminal_pointer failed: {e}");
        }
    }

    /// Declare the current terminal cell geometry when it has changed.
    ///
    /// Called after every applied message and after any real geometry
    /// change. An unchanged size sends nothing, so a redraw storm
    /// produces no wire traffic; a changed one sends exactly once.
    fn flush_terminal_declaration(&mut self) {
        let Some(client) = self.attach_client.as_ref() else {
            return;
        };
        if client.session_protocol_version() < 19 {
            return;
        }
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let Some((buffer_id, size)) = state.terminal_declaration_if_changed() else {
            return;
        };
        match client.send_terminal_resize(buffer_id, size) {
            Ok(()) => state.note_terminal_declaration_sent(buffer_id, size),
            Err(e) => eprintln!("pmacs-gpu: send_terminal_resize failed: {e}"),
        }
    }

    /// Ship a `FrontendCellGeometry` if this trigger calls for one
    /// (Q#BP15a).
    ///
    /// The decision is `State`'s; only the send is here, because only this
    /// side owns the attach client. Gated on the NEGOTIATED session
    /// version — the `Hello` baseline stays at the compatibility floor
    /// permanently, so gating on it would leave the band dark forever.
    fn flush_panel_geometry(&mut self, trigger: GeometryTrigger) {
        let Some(client) = self.attach_client.as_ref() else {
            return;
        };
        if client.session_protocol_version() < PANEL_MIN_VERSION {
            return;
        }
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let Some((geometry_epoch, total)) = state.next_geometry_declaration(trigger) else {
            return;
        };
        if let Err(e) = client.send_frontend_cell_geometry(geometry_epoch, total) {
            eprintln!("pmacs-gpu: send_frontend_cell_geometry failed: {e}");
        }
    }

    /// The panel cell a pixel is over, if the band can take a gesture at all.
    fn panel_pointer_hit(&self, x: f64, y: f64) -> Option<(u64, u64, BufferId, CellCoord)> {
        let client = self.attach_client.as_ref()?;
        if client.session_protocol_version() < PANEL_MIN_VERSION {
            return None;
        }
        let state = self.state.as_ref()?;
        let frame = state.panel.presented()?;
        let coord = state.panel_hit_test(x as f32, y as f32)?;
        Some((
            frame.geometry_epoch,
            frame.panel_epoch,
            frame.buffer_id,
            coord,
        ))
    }

    /// Ship one panel gesture at `(x, y)`, reporting whether the band claimed
    /// it.
    fn send_panel_pointer_at(
        &mut self,
        x: f64,
        y: f64,
        kind: ProtocolMouseKind,
        mods: Modifiers,
    ) -> bool {
        let Some((geometry_epoch, panel_epoch, buffer_id, coord)) = self.panel_pointer_hit(x, y)
        else {
            return false;
        };
        let Some(client) = self.attach_client.as_ref() else {
            return false;
        };
        if let Err(e) =
            client.send_panel_pointer(geometry_epoch, panel_epoch, buffer_id, coord, kind, mods)
        {
            eprintln!("pmacs-gpu: send_panel_pointer failed: {e}");
        }
        true
    }

    /// Ship a panel gesture at an explicitly chosen cell.
    ///
    /// Used for a release, whose cell may be the last one reported rather than
    /// the one under the pointer: a panel selection drag routinely ends past
    /// the band's edge, and dropping that release leaves the daemon holding a
    /// button down forever. The terminal path drops such a release; the panel
    /// must not.
    fn send_panel_pointer_at_cell(
        &mut self,
        coord: Option<CellCoord>,
        kind: ProtocolMouseKind,
        mods: Modifiers,
    ) -> bool {
        let Some(client) = self.attach_client.as_ref() else {
            return false;
        };
        if client.session_protocol_version() < PANEL_MIN_VERSION {
            return false;
        }
        let Some(state) = self.state.as_ref() else {
            return false;
        };
        let Some(frame) = state.panel.presented() else {
            return false;
        };
        let Some(coord) = coord else {
            return false;
        };
        if let Err(e) = client.send_panel_pointer(
            frame.geometry_epoch,
            frame.panel_epoch,
            frame.buffer_id,
            coord,
            kind,
            mods,
        ) {
            eprintln!("pmacs-gpu: send_panel_pointer failed: {e}");
        }
        true
    }

    /// Advance a live divider drag, sending `PanelResizeRows` only when the
    /// requested row count actually changes.
    ///
    /// Row counts, never pixels: the daemon clamps by `window.min-height`,
    /// so the frontend's job is to name the rows the pointer is asking for,
    /// not to enforce the floor itself.
    fn advance_panel_drag(&mut self, y: f64) {
        let Some(client) = self.attach_client.as_ref() else {
            return;
        };
        if client.session_protocol_version() < PANEL_MIN_VERSION {
            return;
        }
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let Some(request) = state.panel_drag_request(y as f32) else {
            return;
        };
        match client.send_panel_resize_rows(
            request.geometry_epoch,
            request.panel_epoch,
            request.rows,
        ) {
            Ok(()) => state.note_panel_drag_sent(request.rows),
            Err(e) => eprintln!("pmacs-gpu: send_panel_resize_rows failed: {e}"),
        }
    }

    /// Resolve a pixel to a terminal cell, or `None` when this window is
    /// not in terminal mode or the pixel is outside the grid.
    ///
    /// The status band and the padding past the last whole column are
    /// deliberately not terminal hits: a gesture there belongs to the
    /// chrome, not the child.
    fn terminal_pointer_hit(&self, x: f64, y: f64) -> Option<(BufferId, CellCoord)> {
        let state = self.state.as_ref()?;
        let terminal = state.terminal.as_ref()?;
        let coord = crate::terminal::hit_test_cell(
            x as f32,
            y as f32,
            (TEXT_LEFT, TEXT_TOP),
            state.mono_advance(),
            state.fm.code_line_height(),
            terminal.plan.size,
        )?;
        Some((terminal.buffer_id, coord))
    }

    /// Ship a [`pmacs_protocol::FrontendEvent::MenuPointer`] if the
    /// daemon speaks v11+ (Q#CM1). Navigates the open menu the daemon
    /// owns; pixels stay local, only the resolved row index crosses.
    fn send_menu_pointer(&self, index: Option<u32>, invoke: bool) {
        let Some(client) = self.attach_client.as_ref() else {
            return;
        };
        if client.session_protocol_version() < 11 {
            return;
        }
        if let Err(e) = client.send_menu_pointer(index, invoke) {
            eprintln!("pmacs-gpu: send_menu_pointer failed: {e}");
        }
    }

    fn dispatch_app_event(&mut self, event: AppEvent) {
        let state = self
            .state
            .as_mut()
            .expect("app events dispatch only after state initialization");
        match event {
            AppEvent::Attach(AttachEvent::Message(msg)) => {
                let debug_apply = debug_apply();
                let apply_start = debug_apply.then(std::time::Instant::now);
                let label = debug_apply.then(|| instance_message_label(msg.as_ref()));
                let follow_up = state.apply_attach_message(*msg);
                if let (Some(start), Some(label)) = (apply_start, label) {
                    eprintln!(
                        "pmacs-gpu apply: {label}={}us",
                        std::time::Instant::now().duration_since(start).as_micros()
                    );
                }
                // If the message triggered a follow-up Viewport
                // (currently: every BufferSnapshot does), emit it back
                // to the daemon. The daemon's `SemanticRenderState`
                // produces no styling until a viewport is declared.
                if let Some(ViewportSend {
                    buffer_id,
                    visible,
                    generation,
                }) = follow_up
                    && let Some(client) = self.attach_client.as_ref()
                    && let Err(e) = client.send_viewport(buffer_id, visible, generation)
                {
                    eprintln!("pmacs-gpu: send Viewport failed: {e}");
                }
                // Vterm Stage 3 — the dual declaration. After every
                // snapshot the frontend re-declares BOTH its byte
                // viewport (above) and its terminal cell size, because
                // an empty terminal identity snapshot does not announce
                // itself as a terminal. The daemon keeps whichever one
                // matches the buffer's kind, which is what breaks the
                // otherwise circular "need a frame to know to ask for
                // one" dependency.
                self.flush_terminal_declaration();
                // Q#BP2S1 — a font or scale transaction arrives as a
                // message, so its geometry re-declaration is flushed here.
                // `Metrics` rather than `Surface` on purpose: the cell total
                // may be IDENTICAL while the pixels behind it are not, and
                // the `Surface` dedup would drop exactly that case.
                if self
                    .state
                    .as_mut()
                    .is_some_and(State::take_panel_metrics_changed)
                {
                    self.flush_panel_geometry(GeometryTrigger::Metrics);
                }
                let Some(state) = self.state.as_mut() else {
                    return;
                };
                state.release_timed_out_floor();
                let ready_keys = state.take_ready_round_trip_keys();
                if let Some(client) = self.attach_client.as_ref() {
                    for (key, mods) in ready_keys {
                        if debug_input() {
                            eprintln!("pmacs-gpu flush_key: {key:?} mods={mods:?}");
                        }
                        if let Err(e) = client.send_key(key, mods) {
                            eprintln!("pmacs-gpu: flush send_key failed: {e}");
                        }
                    }
                }
            }
            AppEvent::Attach(AttachEvent::Disconnected(reason)) => {
                eprintln!("pmacs-gpu: daemon disconnected ({reason})");
                state.on_daemon_disconnected("(daemon disconnected)");
            }
        }
    }

    /// Perform [`LifecycleRoute::Resize`]. `width`/`height` arrive
    /// already clamped away from zero by the router.
    fn apply_resize(&mut self, width: u32, height: u32) {
        let vp = self
            .state
            .as_mut()
            .and_then(|state| state.resize(width, height));
        if let Some(vp) = vp
            && let Some(client) = self.attach_client.as_ref()
            && let Err(e) = client.send_viewport(vp.buffer_id, vp.visible, vp.generation)
        {
            eprintln!("pmacs-gpu: resize send_viewport failed: {e}");
        }
        // Vterm Stage 3 — a resize is a real geometry change, so
        // the cell grid is re-derived and declared here. The
        // daemon resizes the shared PTY only if this frontend is
        // the durable controller.
        self.flush_terminal_declaration();
        // Q#BP15a — and so is the panel's whole-frame capacity. An
        // identical cell total is not re-declared: nothing the
        // daemon can act on changed.
        self.flush_panel_geometry(GeometryTrigger::Surface);
    }

    /// Perform [`PointerRoute::Moved`]. `x`/`y` are the physical
    /// pointer position winit reported.
    #[allow(clippy::too_many_lines)] // one linear gesture pipeline; splitting hides the order.
    fn apply_cursor_moved(&mut self, x: f64, y: f64) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        state.pointer_pos = Some((x, y));
        // Q#CM1 — while the menu is open, motion only moves the
        // highlight; send a hover when the item under the pointer
        // changes from the daemon's current active row.
        if state.menu.is_some() {
            let hit = state.menu_hit(x, y);
            let active = state.menu.as_ref().and_then(|m| m.active);
            if let Some((row, true)) = hit
                && active != Some(row)
            {
                self.send_menu_pointer(Some(row), false);
            }
            return;
        }
        // Bottom panel Stage 2B-3 — the band is consumed BEFORE
        // the terminal and document paths. It sits below
        // `document_text_bottom`, so a band pixel cannot hit the
        // document grid, but ordering it first is what makes that a
        // stated rule rather than a consequence of the arithmetic.
        if state.panel.drag.is_some() {
            self.advance_panel_drag(y);
            return;
        }
        let surface = state.classify_pointer_surface(x as f32, y as f32);
        if state.set_panel_divider_hover(surface == PointerSurface::PanelDivider) {
            state.apply_panel_cursor_icon();
        }
        match surface {
            PointerSurface::PanelDivider | PointerSurface::PanelBackground => return,
            PointerSurface::PanelCell(coord) => {
                // A held left button makes this a `Drag(Left)`, not a
                // `Move`. That distinction is the whole of panel
                // selection: `Move` never focuses or claims, while
                // every non-`Move` gesture activates the panel first,
                // so reporting a drag as a hover makes a selection
                // drag silently do nothing.
                let kind = state.panel_motion_kind();
                if state.panel_motion_is_new(coord) {
                    let mods = translate_mods(self.modifiers);
                    self.send_panel_pointer_at(x, y, kind, mods);
                }
                return;
            }
            PointerSurface::Elsewhere => {
                if state.panel.pointer_held {
                    // A drag that wandered out of the band keeps
                    // belonging to the band until the button comes up.
                    return;
                }
            }
        }
        // Vterm Stage 3 — inside the terminal clip, motion is a
        // terminal gesture. Consumed before minimap scrubbing
        // and document hit testing: terminal mode paints no
        // minimap and has no source bytes to resolve.
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if state.terminal.is_some() {
            let dragging = state.pointer_drag_active;
            if let Some((buffer_id, coord)) = self.terminal_pointer_hit(x, y) {
                // Sub-cell motion resolves to the same cell and
                // carries nothing new. Report only on a cell
                // change, matching the document drag path's
                // hit-byte dedupe — otherwise pixel-rate motion
                // becomes pixel-rate wire traffic, and every one
                // of those is a daemon-side gesture.
                let state = self.state.as_mut().expect("checked above");
                if state.terminal_motion_is_new(coord) {
                    let mods = translate_mods(self.modifiers);
                    let kind = if dragging {
                        ProtocolMouseKind::Drag(ProtocolMouseButton::Left)
                    } else {
                        ProtocolMouseKind::Move
                    };
                    self.send_terminal_pointer(buffer_id, coord, kind, mods);
                }
            }
            return;
        }
        if state.minimap_scrub_active {
            // Scrubbing (Q#M6): the press began on the
            // minimap; motion keeps jumping, even if the
            // pointer wanders out of the band.
            let vp = state.minimap_jump_to(y);
            if let Some(vp) = vp
                && let Some(client) = self.attach_client.as_ref()
                && let Err(e) = client.send_viewport(vp.buffer_id, vp.visible, vp.generation)
            {
                eprintln!("pmacs-gpu: minimap scrub send_viewport failed: {e}");
            }
            return;
        }
        if !state.pointer_drag_active {
            return;
        }
        // Q#M7 — arm/disarm edge auto-scroll from the drag's
        // vertical position; `about_to_wait` runs the ticks.
        state.edge_scroll_dir =
            edge_scroll_direction(y as f32, state.config.height, state.fm, state.band_inset());
        // Drag coalescing (predicted finding #4): pixel-rate
        // motion only ships when the hit byte changes.
        let Some(byte) = state.hit_test_source_byte(x, y) else {
            return;
        };
        if state.last_pointer_sent_byte == Some(byte) {
            return;
        }
        state.last_pointer_sent_byte = Some(byte);
        state.note_pointer_round_trip();
        let buffer_id = state.current_buffer_id;
        let mods = translate_mods(self.modifiers);
        if let Some(buffer_id) = buffer_id {
            self.send_pointer(buffer_id, byte, PointerKind::Drag, mods);
        }
    }

    /// Perform [`PointerRoute::Left`], press or release.
    #[allow(clippy::too_many_lines)] // one linear gesture pipeline; splitting hides the order.
    fn apply_left_button(&mut self, button_state: ElementState) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let Some((x, y)) = state.pointer_pos else {
            return;
        };
        // Q#CM1 — while the menu is open the left button drives
        // it: a press invokes the row under the pointer (or
        // dismisses on a click outside); a release is swallowed.
        if state.menu.is_some() {
            if button_state == ElementState::Pressed {
                let action = match state.menu_hit(x, y) {
                    Some((row, true)) => Some((Some(row), true)),
                    Some((_, false)) => None,   // separator — ignore
                    None => Some((None, true)), // outside — dismiss
                };
                if let Some((index, invoke)) = action {
                    self.send_menu_pointer(index, invoke);
                }
            }
            return;
        }
        let mods = translate_mods(self.modifiers);
        // Bottom panel Stage 2B-3 — the divider strip and the band
        // claim the gesture before either document path sees it.
        let panel_surface = state.classify_pointer_surface(x as f32, y as f32);
        match button_state {
            ElementState::Pressed => {
                if panel_surface == PointerSurface::PanelDivider
                    && state.begin_panel_drag(x as f32, y as f32)
                {
                    return;
                }
                // Arm the gesture BEFORE sending, and only when the
                // press actually landed on a cell: arming on a miss
                // would make a later in-band motion send a `Drag` with
                // no preceding `Down`, and not arming at all means
                // `Drag(Left)` is never emitted and panel selection
                // cannot work at all.
                if let PointerSurface::PanelCell(_) = panel_surface {
                    let state = self.state.as_mut().expect("checked above");
                    state.set_panel_pointer_held(true);
                    self.send_panel_pointer_at(
                        x,
                        y,
                        ProtocolMouseKind::Down(ProtocolMouseButton::Left),
                        mods,
                    );
                    return;
                }
                if state.panel.pointer_held {
                    let state = self.state.as_mut().expect("checked above");
                    state.set_panel_pointer_held(false);
                }
            }
            ElementState::Released => {
                if state.end_panel_drag() {
                    return;
                }
                if state.panel.pointer_held {
                    let cell = state.panel_release_cell(x as f32, y as f32);
                    self.send_panel_pointer_at_cell(
                        cell,
                        ProtocolMouseKind::Up(ProtocolMouseButton::Left),
                        mods,
                    );
                    if let Some(state) = self.state.as_mut() {
                        state.set_panel_pointer_held(false);
                    }
                    return;
                }
            }
        }
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if state.terminal.is_some() {
            let hit = self.terminal_pointer_hit(x, y);
            let state = self.state.as_mut().expect("checked above");
            let kind = match button_state {
                ElementState::Pressed => {
                    // A press that MISSES the grid (the status
                    // band, the trailing padding) starts no
                    // drag: arming the flag there would make a
                    // later in-grid motion send a `Drag` with no
                    // preceding `Down`.
                    state.pointer_drag_active = hit.is_some();
                    ProtocolMouseKind::Down(ProtocolMouseButton::Left)
                }
                ElementState::Released => {
                    // A release always ends the drag, including
                    // one that wandered outside the grid.
                    state.pointer_drag_active = false;
                    ProtocolMouseKind::Up(ProtocolMouseButton::Left)
                }
            };
            // A press or release always reports, and it re-arms
            // the motion dedupe: the first drag after a press
            // must reach the daemon even at the cell the press
            // landed on.
            state.last_terminal_pointer_cell = None;
            if let Some((buffer_id, coord)) = hit {
                self.send_terminal_pointer(buffer_id, coord, kind, mods);
            }
            return;
        }
        match button_state {
            ElementState::Pressed => {
                if state.in_minimap_band(x, y) {
                    // Q#M6 — consumed before text hit-testing;
                    // never a Pointer event.
                    state.minimap_scrub_active = true;
                    let vp = state.minimap_jump_to(y);
                    if let Some(vp) = vp
                        && let Some(client) = self.attach_client.as_ref()
                        && let Err(e) =
                            client.send_viewport(vp.buffer_id, vp.visible, vp.generation)
                    {
                        eprintln!("pmacs-gpu: minimap jump send_viewport failed: {e}");
                    }
                    return;
                }
                let Some(byte) = state.hit_test_source_byte(x, y) else {
                    return;
                };
                let kind = state.classify_pointer_down(byte, mods.contains(Modifiers::SHIFT));
                state.pointer_drag_active = true;
                state.last_pointer_sent_byte = Some(byte);
                state.note_pointer_round_trip();
                if let Some(buffer_id) = state.current_buffer_id {
                    if debug_input() {
                        eprintln!("pmacs-gpu pointer: {kind:?} byte={byte}");
                    }
                    self.send_pointer(buffer_id, byte, kind, mods);
                }
            }
            ElementState::Released => {
                if state.minimap_scrub_active {
                    state.minimap_scrub_active = false;
                    return;
                }
                if !state.pointer_drag_active {
                    return;
                }
                state.pointer_drag_active = false;
                state.edge_scroll_dir = None;
                state.edge_scroll_last = None;
                let byte = state
                    .hit_test_source_byte(x, y)
                    .or(state.last_pointer_sent_byte);
                let buffer_id = state.current_buffer_id;
                if let (Some(byte), Some(buffer_id)) = (byte, buffer_id) {
                    self.send_pointer(buffer_id, byte, PointerKind::Up, mods);
                }
            }
        }
    }

    /// Perform [`PointerRoute::RightPress`].
    ///
    /// Q#CM1 — right-click opens the context menu at the hit byte
    /// (or dismisses an open one). The anchor pixel is remembered
    /// so the popup the daemon sends back draws at the click.
    #[allow(clippy::too_many_lines)] // one linear gesture pipeline; splitting hides the order.
    fn apply_right_press(&mut self) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let Some((x, y)) = state.pointer_pos else {
            return;
        };
        if state.menu.is_some() {
            self.send_menu_pointer(None, true);
            return;
        }
        // Bottom panel Stage 2B-3 — a right-click in the band is a
        // panel gesture, claimed before the terminal and document
        // paths. The daemon decides between child mouse reporting and
        // the editor context menu, so the anchor is remembered here
        // exactly as for a document click; without this the band's
        // context actions are unreachable and the click is applied to
        // the document underneath instead.
        if let PointerSurface::PanelCell(_) = state.classify_pointer_surface(x as f32, y as f32) {
            if let Some(state) = self.state.as_mut() {
                state.menu_anchor_px = (x, y);
            }
            let mods = translate_mods(self.modifiers);
            self.send_panel_pointer_at(
                x,
                y,
                ProtocolMouseKind::Down(ProtocolMouseButton::Right),
                mods,
            );
            return;
        }
        let Some(state) = self.state.as_mut() else {
            return;
        };
        // Vterm Stage 3 — a right-click in the terminal clip is
        // a terminal gesture; the daemon decides between child
        // reporting and the editor context menu, so the anchor
        // is remembered here exactly as for a document click.
        if state.terminal.is_some() {
            state.menu_anchor_px = (x, y);
            if let Some((buffer_id, coord)) = self.terminal_pointer_hit(x, y) {
                let mods = translate_mods(self.modifiers);
                self.send_terminal_pointer(
                    buffer_id,
                    coord,
                    ProtocolMouseKind::Down(ProtocolMouseButton::Right),
                    mods,
                );
            }
            return;
        }
        let Some(byte) = state.hit_test_source_byte(x, y) else {
            return;
        };
        state.menu_anchor_px = (x, y);
        let buffer_id = state.current_buffer_id;
        let mods = translate_mods(self.modifiers);
        if let Some(buffer_id) = buffer_id {
            self.send_pointer(buffer_id, byte, PointerKind::Context, mods);
        }
    }

    /// Perform [`PointerRoute::Wheel`].
    #[allow(clippy::too_many_lines)] // one linear gesture pipeline; splitting hides the order.
    fn apply_wheel(&mut self, delta: MouseScrollDelta) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        // Wheel scroll is local-only: the GPU owns the
        // viewport. Positive winit y = scroll up = smaller
        // scroll_top.
        let lines = match delta {
            winit::event::MouseScrollDelta::LineDelta(_, y) => {
                (-y * WHEEL_LINES_PER_TICK).round() as i64
            }
            winit::event::MouseScrollDelta::PixelDelta(p) => {
                (-(p.y as f32) / state.fm.code_line_height()).round() as i64
            }
        };
        if lines == 0 {
            return;
        }
        // Bottom panel Stage 2B-3 — a wheel tick over the band scrolls
        // the PANEL's window, which is daemon-side state, so it
        // crosses the wire instead of moving this frontend's local
        // document `scroll_top`. Falling through would scroll the
        // document while the pointer is inside the panel.
        if let Some((x, y)) = state.pointer_pos
            && matches!(
                state.classify_pointer_surface(x as f32, y as f32),
                PointerSurface::PanelCell(_)
            )
        {
            let mods = translate_mods(self.modifiers);
            let kind = if lines < 0 {
                ProtocolMouseKind::ScrollUp
            } else {
                ProtocolMouseKind::ScrollDown
            };
            self.send_panel_pointer_at(x, y, kind, mods);
            return;
        }
        let Some(state) = self.state.as_mut() else {
            return;
        };
        // Vterm Stage 3 — the terminal's scrollback belongs to
        // the daemon-side view, not to this frontend's local
        // scroll, so a wheel tick crosses the wire as a
        // terminal gesture instead of moving `scroll_top`.
        if state.terminal.is_some() {
            if let Some((x, y)) = state.pointer_pos
                && let Some((buffer_id, coord)) = self.terminal_pointer_hit(x, y)
            {
                let mods = translate_mods(self.modifiers);
                let kind = if lines < 0 {
                    ProtocolMouseKind::ScrollUp
                } else {
                    ProtocolMouseKind::ScrollDown
                };
                self.send_terminal_pointer(buffer_id, coord, kind, mods);
            }
            return;
        }
        let vp = state.scroll_by_lines(lines);
        if let Some(vp) = vp
            && let Some(client) = self.attach_client.as_ref()
            && let Err(e) = client.send_viewport(vp.buffer_id, vp.visible, vp.generation)
        {
            eprintln!("pmacs-gpu: wheel send_viewport failed: {e}");
        }
    }

    /// Perform [`LifecycleRoute::Redraw`]. A no-op before `resumed` has
    /// built the surface.
    fn apply_redraw(&mut self) {
        if let Some(state) = self.state.as_mut() {
            state.render();
        }
    }

    /// Perform [`KeyAction::Press`]. The router has already
    /// discarded key-ups, so `key` is always a press.
    #[allow(clippy::too_many_lines)] // one linear key pipeline; splitting hides the order.
    fn apply_keyboard(&mut self, key: &KeyEvent) -> EventOutcome {
        // While the daemon is intercepting keystrokes — an active
        // incremental search (Q#SR5), or a minibuffer / pending
        // prefix — every key belongs to its handler, not the
        // buffer. The GUI round-trips them all and never
        // optimistic-applies (that would edit the document
        // mid-search).
        let intercept = self
            .state
            .as_ref()
            .is_some_and(State::daemon_intercepts_keys);

        // Arc 1a Q#C6 — the completion popup is NON-modal, so it
        // never flips the intercept gate (typing stays
        // optimistic; the daemon's after-edit refresh re-ships
        // the popup). Only the keys whose *default GPU handling
        // is wrong under a popup* need this flag: Esc (below,
        // else it's the local quit) and RET/TAB (the optimistic
        // gate further down, else they'd insert instead of
        // accept). C-n/C-p/C-g already round-trip as command
        // chords, Up/Down as forwarded motion keys — the daemon's
        // completion shadow handles all of them.
        let completion_open = self
            .state
            .as_ref()
            .is_some_and(State::completion_open_for_current_buffer);

        // Escape cancels an active intercept (e.g. a running
        // search) or dismisses the completion popup; otherwise it
        // stays the local quit.
        if matches!(key.logical_key, Key::Named(NamedKey::Escape)) {
            if intercept || completion_open {
                if let Some(client) = self.attach_client.as_ref()
                    && let Err(e) = client.send_key(ProtocolKey::Escape, Modifiers::NONE)
                {
                    eprintln!("pmacs-gpu: send Escape (cancel) failed: {e}");
                }
            } else {
                // Q#S1-1 / A4 — the local quit, unchanged here and
                // deleted by Stage 1a: an idle Escape must reach the
                // daemon. `window_event` performs the exit; this is
                // the only reason a body needs an outcome at all.
                return EventOutcome::Exit;
            }
            return EventOutcome::Continue;
        }

        let Some((pkey, mut pmods)) = translate_key(&key.logical_key, self.modifiers) else {
            return EventOutcome::Continue;
        };

        // AltGr / international text (audit F-004). winit reports
        // the text a keypress produces; when a keypress yields
        // printable text *while both Ctrl and Alt* are held — the
        // AltGr signature on Windows (LCtrl+RAlt) — it's text
        // input, not a command chord. Strip those modifiers (keep
        // Shift) so it inserts (through the plain-text path, or the
        // daemon's SelfInsert while a prompt is open) instead of
        // being routed to the keymap. Alt alone is left intact so
        // macOS Option-as-Meta still reaches the keymap; on layouts
        // where AltGr isn't Ctrl+Alt this is a no-op.
        if matches!(pkey, ProtocolKey::Char(_)) && is_layout_text(key.text.as_deref(), pmods) {
            pmods = if pmods.contains(Modifiers::SHIFT) {
                Modifiers::SHIFT
            } else {
                Modifiers::NONE
            };
        }

        // Ctrl-V — OS paste (Q#CM6). Read the system clipboard
        // locally via arboard and ship it as a `Paste` event; the
        // daemon inserts it. Handled before binding `client` so
        // the `&mut self` clipboard read doesn't conflict with the
        // client borrow. Skipped while intercepting (the daemon's
        // active handler owns the key then). The daemon keymap's
        // C-y yanks the in-app slot instead.
        if !intercept && pkey == ProtocolKey::Char('v') && pmods == Modifiers::CTRL {
            let bytes = self.state.as_mut().and_then(State::read_os_clipboard);
            if let Some(bytes) = bytes
                && let Some(client) = self.attach_client.as_ref()
                && let Err(e) = client.send_paste(bytes)
            {
                eprintln!("pmacs-gpu: send_paste failed: {e}");
            }
            return EventOutcome::Continue;
        }

        let Some(client) = self.attach_client.as_ref() else {
            return EventOutcome::Continue;
        };

        // Intercept path: round-trip every key into the daemon's
        // active handler (search query / step / accept / cancel).
        if intercept {
            if let Some(state) = self.state.as_mut() {
                state.mark_cursor_stale_after_round_trip();
            }
            if debug_input() {
                eprintln!("pmacs-gpu send_key (intercepted): {pkey:?} mods={pmods:?}");
            }
            if let Err(e) = client.send_key(pkey, pmods) {
                eprintln!("pmacs-gpu: send_key (intercepted) failed: {e}");
            }
            return EventOutcome::Continue;
        }

        // Idle: forward any command chord (Char/Enter/Tab with
        // Ctrl or Alt) to the daemon (Q#GC1). These drive the
        // keymap — `C-a`, `M-f`, `C-x C-s`, isearch/clipboard/M-x,
        // … — the same path the TUI forwards everything through.
        // The GUI no longer withholds them (the minibuffer / prompt
        // flows they open now render, Q#MB1). Once a forwarded
        // chord opens a prompt or enters a prefix, `dispatch_idle`
        // flips false and the intercept gate round-trips the rest —
        // no optimistic local flip, so a chord that changes no
        // daemon state can never wedge the gate. (Ctrl-V / OS paste
        // is handled locally above and never reaches here.)
        if is_command_chord(pkey, pmods) {
            if let Some(state) = self.state.as_mut() {
                state.mark_cursor_stale_after_round_trip();
            }
            if let Err(e) = client.send_key(pkey, pmods) {
                eprintln!("pmacs-gpu: send_key (command chord) failed: {e}");
            }
            return EventOutcome::Continue;
        }

        // Session B2 forwards cursor motion + plain text editing
        // (Char / Backspace / Enter / Delete / Tab). Command chords
        // are handled above; Meta/Super-only chords fall through
        // here and are withheld, leaving OS/WM shortcuts (Cmd-Q,
        // Cmd-C) to the platform.
        if !should_forward_key(pkey, pmods) {
            return EventOutcome::Continue;
        }

        // Arc 1a Q#C6 — with the popup open, RET and TAB mean
        // "accept", not "insert \n / \t": skip the optimistic
        // path so they round-trip into the daemon's
        // dispatch_completion_key. Everything else stays
        // optimistic.
        let completion_takes_key =
            completion_open && matches!(pkey, ProtocolKey::Enter | ProtocolKey::Tab);

        if !completion_takes_key
            && let Some(op) = self.state.as_mut().and_then(|state| {
                state
                    .optimistic_crdt_insert(pkey, pmods)
                    .or_else(|| state.optimistic_crdt_delete(pkey, pmods))
            })
        {
            if debug_input() {
                eprintln!(
                    "pmacs-gpu send_crdt: key={pkey:?} buf={:?} bytes={}B",
                    op.buffer_id,
                    op.op.bytes.len()
                );
            }
            if let Err(e) = client.send_crdt_op(op.buffer_id, op.op) {
                eprintln!("pmacs-gpu: send_crdt_op failed: {e}");
            }
            // An optimistic edit near the viewport edge can
            // scroll (a wrap-inducing insert, a Backspace
            // above the top); re-declare the scoped viewport
            // so the producer styles the newly visible lines.
            if let Some(vp) = op.viewport
                && let Err(e) = client.send_viewport(vp.buffer_id, vp.visible, vp.generation)
            {
                eprintln!("pmacs-gpu: send Viewport failed: {e}");
            }
            return EventOutcome::Continue;
        }
        if let Some(state) = self.state.as_mut() {
            if state.defer_round_trip_key_if_needed(pkey, pmods) {
                if debug_input() {
                    eprintln!(
                        "pmacs-gpu defer_key: {pkey:?} mods={pmods:?} \
                         pending optimistic cursor"
                    );
                }
                return EventOutcome::Continue;
            }
            state.mark_cursor_stale_after_round_trip();
        }
        if debug_input() {
            eprintln!("pmacs-gpu send_key: {pkey:?} mods={pmods:?}");
        }
        if let Err(e) = client.send_key(pkey, pmods) {
            eprintln!("pmacs-gpu: send_key failed: {e}");
        }
        EventOutcome::Continue
    }
}

/// GUI Stage 1-pre — the input seam.
///
/// `window_event` receives a `WindowEvent` and, until this seam existed,
/// decided *and performed* everything in one 655-line match. Nothing
/// below it could be witnessed without a display: `ActiveEventLoop` is
/// non-constructible outside a live event loop, and the arms that matter
/// reach a GPU surface or a socket.
///
/// The seam splits the two halves. **Deciding** is [`route_event`] — a
/// free function over `&WindowEvent` alone, so a headless test can drive
/// it for **every family whose event winit lets a test construct**,
/// which is all of them except keyboard; see [`route_keyboard`] for that
/// exception and how far it reaches. **Performing** stays on `App`, in
/// the `apply_*` methods the router's variants name.
///
/// The decision is what the route *is*, not merely which family claims
/// it: `Exit` is the local exit effect, `Resize` carries the clamped
/// surface extent, `Modifiers` carries the state mutation. **Two arms —
/// `CloseRequested` and `RedrawRequested` — send nothing outbound at
/// all**, so a harness recording only protocol traffic would leave them
/// invisible; that is why a route names its local effect and the harness
/// records routes.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Route<'a> {
    /// The lifecycle family — see [`route_lifecycle`].
    Lifecycle(LifecycleRoute),
    /// The keyboard family — see [`route_keyboard`].
    Keyboard {
        action: KeyAction,
        key: &'a KeyEvent,
    },
    /// The pointer family — see [`route_pointer`].
    Pointer(PointerRoute),
    /// No family claims this event: pmacs ignores it.
    Unrouted,
}

/// What the event loop must do once a family's body has run. Only the
/// keyboard family produces anything but `Continue` today: an idle
/// Escape is a local quit. Returning the decision rather than taking an
/// `&ActiveEventLoop` is what keeps every body reachable from a test —
/// the crate's two `event_loop.exit()` call sites, this one and
/// `LifecycleRoute::Exit`, both sit in `window_event` and nowhere else.
///
/// Stage 1a's A4 deletes that branch (an idle Escape must reach the
/// daemon and never exit), at which point this type has one variant and
/// should go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventOutcome {
    Continue,
    Exit,
}

/// The lifecycle family: events about the **window itself** — closing,
/// resizing, repainting — rather than about a gesture aimed into the
/// document. `ModifiersChanged` is grouped here as the one exception,
/// and it is named as one: it is a bare state mutation with no gesture
/// of its own and no body to extract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleRoute {
    /// `CloseRequested` — leave the event loop. Q#S1-1: a native close
    /// detaches this frontend, it does not shut the daemon down.
    Exit,
    /// `ModifiersChanged` — the new modifier state, already unwrapped
    /// from winit's `Modifiers` wrapper.
    Modifiers(winit::keyboard::ModifiersState),
    /// `Resized` — the new surface extent, **already clamped away from
    /// zero**. A minimize delivers `0×0`, and wgpu rejects a zero-extent
    /// surface configuration, so the clamp is a rule rather than
    /// defensive padding; deciding it here is what makes it witnessable
    /// without a surface.
    Resize { width: u32, height: u32 },
    /// `RedrawRequested` — paint a frame. Nothing goes to the daemon,
    /// which is why the harness records local effects rather than
    /// outbound traffic.
    Redraw,
}

/// The keyboard family's whole decision. `Release` is a route rather
/// than an absence: the family **claims** a key-up and drops it, which
/// is a different fact from no family claiming the event, and
/// conflating the two would hide the drop the moment a later slice
/// wants key-up semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyAction {
    /// A key-down — the only keyboard state pmacs acts on.
    Press,
    /// A key-up. Claimed and deliberately discarded.
    Release,
}

/// Decide what `window_event` should do with an event, from the event
/// alone. See [`Route`].
fn route_event(event: &WindowEvent) -> Route<'_> {
    if let Some(lifecycle) = route_lifecycle(event) {
        return Route::Lifecycle(lifecycle);
    }
    if let Some((action, key)) = route_keyboard(event) {
        return Route::Keyboard { action, key };
    }
    if let Some(pointer) = route_pointer(event) {
        return Route::Pointer(pointer);
    }
    Route::Unrouted
}

/// The lifecycle family's decision. `None` means some other family owns
/// the event.
fn route_lifecycle(event: &WindowEvent) -> Option<LifecycleRoute> {
    match event {
        WindowEvent::CloseRequested => Some(LifecycleRoute::Exit),
        WindowEvent::ModifiersChanged(mods) => Some(LifecycleRoute::Modifiers(mods.state())),
        WindowEvent::Resized(size) => Some(LifecycleRoute::Resize {
            width: size.width.max(1),
            height: size.height.max(1),
        }),
        WindowEvent::RedrawRequested => Some(LifecycleRoute::Redraw),
        _ => None,
    }
}

/// The keyboard family's claim. `None` means some other family owns the
/// event.
///
/// **A SECOND ACCEPTED STRUCTURAL EXCEPTION, alongside P3.** This
/// function's own pattern arm is unwitnessable: `KeyEvent` carries a
/// `pub(crate) platform_specific` field, so **no `WindowEvent::KeyboardInput`
/// can be constructed outside winit** and no headless test can feed one.
/// The limitation is winit's and not this seam's — it is why the arm is
/// kept to a pattern and a call, with the family's only real decision
/// factored into [`route_key_action`], which takes an `ElementState` and
/// is tested directly. The pointer families have no such problem:
/// `DeviceId::dummy()` is provided by winit for exactly this purpose,
/// and their events are constructible.
fn route_keyboard(event: &WindowEvent) -> Option<(KeyAction, &KeyEvent)> {
    match event {
        WindowEvent::KeyboardInput { event: key, .. } => Some((route_key_action(key.state), key)),
        _ => None,
    }
}

/// Press or release — see [`KeyAction`].
fn route_key_action(state: ElementState) -> KeyAction {
    match state {
        ElementState::Pressed => KeyAction::Press,
        ElementState::Released => KeyAction::Release,
    }
}

/// The pointer family — Session M-2 pointer input, see
/// `docs/pmacs-gpu-mouse-framing.md`.
///
/// The button discrimination used to live in the shape of two
/// overlapping `MouseInput` match arms — left in **either** state, right
/// in the **pressed** state only, everything else falling through the
/// wildcard several hundred lines away. Naming the cases makes the
/// asymmetry a decision instead of an artefact of arm order.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PointerRoute {
    /// `CursorMoved`, at the physical position winit reported.
    Moved { x: f64, y: f64 },
    /// `MouseInput` on the left button, in **either** state: a press
    /// starts a selection or a scrub, a release ends it.
    Left(ElementState),
    /// `MouseInput` **pressing** the right button. The context menu
    /// opens on the press, so the matching release is deliberately
    /// nothing — see `UnusedButton`.
    RightPress,
    /// A `MouseInput` this frontend has no semantics for: the right
    /// button's release, and every middle / back / forward / other
    /// button in either state. **Claimed by the pointer family and
    /// dropped**, exactly as it behaved when it fell through to the
    /// wildcard. Stage 1b's B4 gives the middle button a meaning
    /// (PRIMARY-selection paste on Linux) and lands here.
    UnusedButton,
    /// `MouseWheel`. The delta is carried raw: converting it to lines
    /// needs the code line height, which is `State`'s to know.
    Wheel(MouseScrollDelta),
}

/// The pointer family's decision. `None` means some other family owns
/// the event.
fn route_pointer(event: &WindowEvent) -> Option<PointerRoute> {
    match event {
        WindowEvent::CursorMoved { position, .. } => Some(PointerRoute::Moved {
            x: position.x,
            y: position.y,
        }),
        WindowEvent::MouseInput { state, button, .. } => Some(match (button, state) {
            (MouseButton::Left, _) => PointerRoute::Left(*state),
            (MouseButton::Right, ElementState::Pressed) => PointerRoute::RightPress,
            _ => PointerRoute::UnusedButton,
        }),
        WindowEvent::MouseWheel { delta, .. } => Some(PointerRoute::Wheel(*delta)),
        _ => None,
    }
}

/// GUI Stage 1-pre — the headless routing harness (P2).
///
/// It feeds `WindowEvent`s through the production [`route_event`] and
/// records what each one routed to. The transcript is of **routes**,
/// deliberately, not of outbound protocol traffic: `CloseRequested`
/// exits and `RedrawRequested` repaints, and neither sends the daemon
/// anything, so a transcript of daemon traffic alone cannot tell a
/// handled arm from a dropped one. A route names its local effect —
/// exit, resize extent, modifier mutation — which is what makes those
/// arms observable here at all.
///
/// P3 is the stated exception: that `window_event` *calls* the router
/// rather than deciding for itself is a code-review invariant, not a
/// tested one. `ActiveEventLoop` cannot be constructed outside a live
/// event loop, so the real callback is unreachable from a test. The
/// mutation evidence below covers every router arm and **not** the
/// delegation.
#[cfg(test)]
#[derive(Debug, Default)]
struct RoutingHarness<'a> {
    transcript: Vec<Route<'a>>,
}

#[cfg(test)]
impl<'a> RoutingHarness<'a> {
    fn feed(&mut self, event: &'a WindowEvent) -> Route<'a> {
        let route = route_event(event);
        self.transcript.push(route);
        route
    }

    fn transcript(&self) -> &[Route<'a>] {
        &self.transcript
    }
}

#[cfg(test)]
mod input_routing_tests {
    use super::*;
    use winit::dpi::{PhysicalPosition, PhysicalSize};
    use winit::event::{DeviceId, TouchPhase};
    use winit::keyboard::ModifiersState;

    fn modifiers_changed(state: ModifiersState) -> WindowEvent {
        WindowEvent::ModifiersChanged(state.into())
    }

    /// The per-variant rows below drive [`route_event`] directly and the
    /// transcript row drives the harness. That split is deliberate: it
    /// leaves the transcript row as P2's sole owner, so a harness that
    /// stopped recording an effect fails exactly one row instead of
    /// every row.
    ///
    /// Events are bound to locals rather than passed as temporaries
    /// because a `Route` borrows the event it came from — the keyboard
    /// variant carries a `&KeyEvent`.
    fn route_one(event: &WindowEvent) -> Route<'_> {
        route_event(event)
    }

    /// P1 — `CloseRequested`. It sends the daemon nothing and its whole
    /// effect is local, so this row exists only because the route names
    /// the effect.
    #[test]
    fn close_requested_routes_to_exit() {
        let event = WindowEvent::CloseRequested;
        assert_eq!(route_one(&event), Route::Lifecycle(LifecycleRoute::Exit));
    }

    /// P1 — `ModifiersChanged` carries the unwrapped state, which is the
    /// mutation `window_event` performs.
    #[test]
    fn modifiers_changed_routes_the_new_state() {
        let mods = ModifiersState::CONTROL | ModifiersState::ALT;
        let held = modifiers_changed(mods);
        assert_eq!(
            route_one(&held),
            Route::Lifecycle(LifecycleRoute::Modifiers(mods))
        );
        // An empty state is a real transition (every modifier released),
        // not an absent one.
        let released = modifiers_changed(ModifiersState::empty());
        assert_eq!(
            route_one(&released),
            Route::Lifecycle(LifecycleRoute::Modifiers(ModifiersState::empty()))
        );
    }

    /// P1 — `Resized` carries the extent through unchanged when it is
    /// already non-zero.
    #[test]
    fn resized_routes_the_new_extent() {
        let event = WindowEvent::Resized(PhysicalSize::new(1280, 720));
        assert_eq!(
            route_one(&event),
            Route::Lifecycle(LifecycleRoute::Resize {
                width: 1280,
                height: 720,
            })
        );
    }

    /// A minimize delivers `0×0` and wgpu rejects a zero-extent surface
    /// configuration. The clamp is per axis, so an extent that collapses
    /// on one axis only still keeps the other.
    #[test]
    fn a_zero_extent_resize_clamps_per_axis() {
        let collapsed = WindowEvent::Resized(PhysicalSize::new(0, 0));
        assert_eq!(
            route_one(&collapsed),
            Route::Lifecycle(LifecycleRoute::Resize {
                width: 1,
                height: 1,
            })
        );
        let no_width = WindowEvent::Resized(PhysicalSize::new(0, 720));
        assert_eq!(
            route_one(&no_width),
            Route::Lifecycle(LifecycleRoute::Resize {
                width: 1,
                height: 720,
            })
        );
        let no_height = WindowEvent::Resized(PhysicalSize::new(1280, 0));
        assert_eq!(
            route_one(&no_height),
            Route::Lifecycle(LifecycleRoute::Resize {
                width: 1280,
                height: 1,
            })
        );
    }

    /// P1 — `RedrawRequested`. The second arm that sends the daemon
    /// nothing, and the reason the harness cannot be a transcript of
    /// outbound traffic.
    #[test]
    fn redraw_requested_routes_to_redraw() {
        let event = WindowEvent::RedrawRequested;
        assert_eq!(route_one(&event), Route::Lifecycle(LifecycleRoute::Redraw));
    }

    /// P1, keyboard — the family's whole decision.
    ///
    /// It is driven through [`route_key_action`] rather than the
    /// harness, and that is the accepted exception documented on
    /// [`route_keyboard`]: `KeyEvent` has a `pub(crate)` field, so no
    /// `WindowEvent::KeyboardInput` exists that a test can build. What
    /// remains unwitnessed is one pattern arm with no logic in it; the
    /// decision itself is here.
    #[test]
    fn a_press_is_acted_on_and_a_release_is_discarded() {
        assert_eq!(route_key_action(ElementState::Pressed), KeyAction::Press);
        assert_eq!(route_key_action(ElementState::Released), KeyAction::Release);
    }

    fn mouse_input(state: ElementState, button: MouseButton) -> WindowEvent {
        WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button,
        }
    }

    /// P1 — `CursorMoved` carries the position through.
    #[test]
    fn cursor_moved_routes_the_position() {
        let event = WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(12.5, 34.25),
        };
        assert_eq!(
            route_one(&event),
            Route::Pointer(PointerRoute::Moved { x: 12.5, y: 34.25 })
        );
    }

    /// P1 — the left button routes in **both** states, because a press
    /// starts a gesture and a release ends it. The state is carried, not
    /// discarded: routing a release as a press would leave a drag armed
    /// forever.
    #[test]
    fn the_left_button_routes_in_either_state() {
        for state in [ElementState::Pressed, ElementState::Released] {
            let event = mouse_input(state, MouseButton::Left);
            assert_eq!(
                route_one(&event),
                Route::Pointer(PointerRoute::Left(state)),
                "left {state:?}"
            );
        }
    }

    /// P1 — the right button is **asymmetric with the left, on purpose**:
    /// the context menu opens on the press and the release means nothing.
    /// This asymmetry used to be implicit in the order and shape of two
    /// overlapping match arms.
    #[test]
    fn the_right_button_routes_only_on_press() {
        let pressed = mouse_input(ElementState::Pressed, MouseButton::Right);
        assert_eq!(
            route_one(&pressed),
            Route::Pointer(PointerRoute::RightPress)
        );
        let released = mouse_input(ElementState::Released, MouseButton::Right);
        assert_eq!(
            route_one(&released),
            Route::Pointer(PointerRoute::UnusedButton)
        );
    }

    /// A button the frontend has no semantics for is **claimed by the
    /// pointer family and dropped**, not left unrouted — the same
    /// distinction the keyboard family draws for a key-up. Stage 1b's B4
    /// gives the middle button a meaning and lands on this route.
    #[test]
    fn a_button_without_semantics_is_claimed_and_dropped() {
        for button in [
            MouseButton::Middle,
            MouseButton::Back,
            MouseButton::Forward,
            MouseButton::Other(9),
        ] {
            for state in [ElementState::Pressed, ElementState::Released] {
                let event = mouse_input(state, button);
                assert_eq!(
                    route_one(&event),
                    Route::Pointer(PointerRoute::UnusedButton),
                    "{button:?} {state:?}"
                );
            }
        }
    }

    /// P1 — the wheel delta is carried **raw**. Converting it to lines
    /// needs the code line height, which only `State` knows, so the
    /// router must not try.
    #[test]
    fn the_wheel_carries_its_delta_unconverted() {
        let lines = WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::LineDelta(0.0, -3.0),
            phase: TouchPhase::Moved,
        };
        assert_eq!(
            route_one(&lines),
            Route::Pointer(PointerRoute::Wheel(MouseScrollDelta::LineDelta(0.0, -3.0)))
        );
        let pixels = WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 17.5)),
            phase: TouchPhase::Moved,
        };
        assert_eq!(
            route_one(&pixels),
            Route::Pointer(PointerRoute::Wheel(MouseScrollDelta::PixelDelta(
                PhysicalPosition::new(0.0, 17.5)
            )))
        );
    }

    /// An event no family claims. `Occluded` is chosen because pmacs has
    /// never handled it and — unlike a mouse button it has no semantics
    /// for — no family claims it at all.
    #[test]
    fn an_unclaimed_event_is_unrouted() {
        let event = WindowEvent::Occluded(true);
        assert_eq!(route_one(&event), Route::Unrouted);
    }

    /// P2 — the harness records a transcript, and the transcript
    /// distinguishes every routed effect from the others and from an
    /// unclaimed event. **Two of these rows produce no outbound traffic
    /// whatsoever** (`Redraw`, `Exit`), which is the property that rules
    /// out a harness built on protocol traffic alone.
    #[test]
    fn the_harness_records_each_local_effect_in_order() {
        let events = [
            WindowEvent::Resized(PhysicalSize::new(800, 600)),
            modifiers_changed(ModifiersState::SHIFT),
            WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(4.0, 8.0),
            },
            mouse_input(ElementState::Pressed, MouseButton::Left),
            mouse_input(ElementState::Pressed, MouseButton::Middle),
            WindowEvent::RedrawRequested,
            WindowEvent::Occluded(false),
            WindowEvent::CloseRequested,
        ];
        let mut harness = RoutingHarness::default();
        for event in &events {
            harness.feed(event);
        }
        assert_eq!(
            harness.transcript(),
            &[
                Route::Lifecycle(LifecycleRoute::Resize {
                    width: 800,
                    height: 600,
                }),
                Route::Lifecycle(LifecycleRoute::Modifiers(ModifiersState::SHIFT)),
                Route::Pointer(PointerRoute::Moved { x: 4.0, y: 8.0 }),
                Route::Pointer(PointerRoute::Left(ElementState::Pressed)),
                Route::Pointer(PointerRoute::UnusedButton),
                Route::Lifecycle(LifecycleRoute::Redraw),
                Route::Unrouted,
                Route::Lifecycle(LifecycleRoute::Exit),
            ]
        );
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        self.state = Some(State::new(event_loop, CONNECTING_TEXT));
        if let Some(client) = self.attach_client.as_ref()
            && let Some(state) = self.state.as_mut()
        {
            state.set_frontend_id(client.frontend_id());
            state.set_panel_wire(client.session_protocol_version());
        }
        // Q#BP15a: the first declaration rides the first surface this
        // frontend actually has. The daemon needs columns before it can
        // paint a first panel frame, so this is sent WITHOUT a side window
        // — gating it on panel presence would deadlock the first open.
        self.flush_panel_geometry(GeometryTrigger::Surface);

        // In attach mode, kick off the connection now that the event
        // loop is running and a proxy is available. Failure logs and
        // leaves the window showing its `(connecting...)` placeholder
        // — better UX than killing the window during dev.
        if let Mode::Attach { socket } = self.mode.clone() {
            let proxy = self.proxy.take().expect("proxy taken twice");
            match attach::connect(&socket, proxy) {
                Ok(client) => {
                    if let Some(state) = self.state.as_mut() {
                        state.set_frontend_id(client.frontend_id());
                        state.set_panel_wire(client.session_protocol_version());
                    }
                    self.attach_client = Some(client);
                    self.flush_panel_geometry(GeometryTrigger::Surface);
                }
                Err(e) => {
                    eprintln!("pmacs-gpu: attach failed: {e}");
                    if let Some(state) = self.state.as_mut() {
                        // Render a concise, actionable line in the window
                        // itself (F-003) — e.g. a non-CRDT daemon — rather
                        // than a generic "see stderr" the user won't read.
                        state.set_text(&e.window_status());
                    }
                }
            }
        }
        for event in std::mem::take(&mut self.pending_events) {
            self.dispatch_app_event(event);
        }
    }

    /// P3 — the thin call-through. Every decision belongs to
    /// [`route_event`] and every effect to an `apply_*` method; what is
    /// left here is `event_loop.exit()`, which exists nowhere else in
    /// the crate and is the one thing a headless test cannot reach.
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match route_event(&event) {
            Route::Lifecycle(LifecycleRoute::Exit) => event_loop.exit(),
            Route::Lifecycle(LifecycleRoute::Modifiers(mods)) => self.modifiers = mods,
            Route::Lifecycle(LifecycleRoute::Resize { width, height }) => {
                self.apply_resize(width, height);
            }
            Route::Lifecycle(LifecycleRoute::Redraw) => self.apply_redraw(),
            Route::Keyboard {
                action: KeyAction::Press,
                key,
            } => {
                if self.apply_keyboard(key) == EventOutcome::Exit {
                    event_loop.exit();
                }
            }
            Route::Pointer(PointerRoute::Moved { x, y }) => self.apply_cursor_moved(x, y),
            Route::Pointer(PointerRoute::Left(button_state)) => {
                self.apply_left_button(button_state);
            }
            Route::Pointer(PointerRoute::RightPress) => self.apply_right_press(),
            Route::Pointer(PointerRoute::Wheel(delta)) => self.apply_wheel(delta),
            // Three different facts, merged only because all three are
            // today nothing to do: a key-up the keyboard family claimed
            // and dropped, a button the pointer family has no semantics
            // for, and an event no family claims at all.
            Route::Keyboard {
                action: KeyAction::Release,
                ..
            }
            | Route::Pointer(PointerRoute::UnusedButton)
            | Route::Unrouted => {}
        }
    }

    /// The deadline pump. Two timed concerns share it, both armed
    /// rarely:
    ///
    ///   * Q#M7 — the edge auto-scroll tick, while a drag sits in
    ///     the top/bottom edge band. Each due tick scrolls one line
    ///     toward the pointer and re-runs the drag hit-test at the
    ///     *current* pointer position, so the selection keeps
    ///     growing while the mouse is stationary past the edge.
    ///   * Q#M6 (bet #2) — the post-jump styled-redraw hold: if the
    ///     daemon's restyle hasn't landed by the deadline, draw the
    ///     unstyled frame anyway (responsiveness floor).
    ///
    /// With neither armed the loop stays in plain `Wait`.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let now = std::time::Instant::now();
        let mut next_wake: Option<std::time::Instant> = None;

        // Q#M6 — held post-jump frame.
        if let Some(deadline) = state.styled_redraw_deadline {
            if now >= deadline {
                state.styled_redraw_deadline = None;
                state.request_redraw();
            } else {
                next_wake = Some(deadline);
            }
        }

        // Q#M7 — edge auto-scroll.
        let mut drag_resend: Option<(BufferId, u64)> = None;
        if state.pointer_drag_active
            && let Some(dir) = state.edge_scroll_dir
        {
            let due = state
                .edge_scroll_last
                .is_none_or(|at| now.duration_since(at) >= EDGE_SCROLL_TICK);
            if due {
                state.edge_scroll_last = Some(now);
                let vp = state.scroll_by_lines(dir);
                if let Some(vp) = vp
                    && let Some(client) = self.attach_client.as_ref()
                    && let Err(e) = client.send_viewport(vp.buffer_id, vp.visible, vp.generation)
                {
                    eprintln!("pmacs-gpu: edge-scroll send_viewport failed: {e}");
                }
                let state = self.state.as_mut().expect("checked above");
                if let Some((x, y)) = state.pointer_pos
                    && let Some(byte) = state.hit_test_source_byte(x, y)
                    && state.last_pointer_sent_byte != Some(byte)
                {
                    state.last_pointer_sent_byte = Some(byte);
                    state.note_pointer_round_trip();
                    if let Some(buffer_id) = state.current_buffer_id {
                        drag_resend = Some((buffer_id, byte));
                    }
                }
            }
            let last = self
                .state
                .as_ref()
                .and_then(|s| s.edge_scroll_last)
                .unwrap_or(now);
            let tick_wake = last + EDGE_SCROLL_TICK;
            next_wake = Some(next_wake.map_or(tick_wake, |w| w.min(tick_wake)));
        }
        if let Some((buffer_id, byte)) = drag_resend {
            let mods = translate_mods(self.modifiers);
            self.send_pointer(buffer_id, byte, PointerKind::Drag, mods);
        }

        event_loop.set_control_flow(match next_wake {
            Some(at) => winit::event_loop::ControlFlow::WaitUntil(at),
            None => winit::event_loop::ControlFlow::Wait,
        });
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        if let Some(event) = defer_app_event(self.state.is_some(), &mut self.pending_events, event)
        {
            self.dispatch_app_event(event);
        }
    }
}

/// Follow-up event the main loop fires back to the daemon after
/// processing a message. Right now only Viewport (post-snapshot);
/// later sessions extend this enum.
#[derive(Debug, Clone, Copy)]
struct ViewportSend {
    buffer_id: BufferId,
    visible: ByteRange,
    generation: u64,
}

#[derive(Debug)]
struct CrdtOpSend {
    buffer_id: BufferId,
    op: CrdtOp,
    /// A scoped-viewport re-declaration when the optimistic insert
    /// scrolled the view (Enter on the bottom visible line). Sent
    /// after the op so the producer styles the newly visible range.
    viewport: Option<ViewportSend>,
}

/// How long an unconfirmed optimistic-cursor prediction may gate
/// `CursorByte` acceptance and defer round-trip keys before the
/// escape hatch releases it. Generous against a busy daemon tick;
/// tiny against a human noticing wedged keys.
const FLOOR_CONFIRM_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

/// Frontend-side double-click interval (Q#M1: the daemon cannot see
/// pixels, so the frontend decides what a double-click is). Matches
/// the TUI's `DOUBLE_CLICK_MAX_DELAY`.
const DOUBLE_CLICK_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);

/// Wheel lines scrolled per `MouseScrollDelta::LineDelta` unit.
const WHEEL_LINES_PER_TICK: f32 = 3.0;

/// Byte range an optimistic Backspace/Delete removes at `cursor`, or
/// `None` when it can't be predicted locally: buffer edge (the
/// daemon's behavior is a no-op there anyway), a modifier variant
/// (C-BS word-delete and friends are separate bindings), or a stale
/// mid-codepoint cursor. The range is exactly one codepoint, matching
/// `buffer.delete-backward` / `buffer.delete-forward`'s no-region
/// behavior; region deletes are excluded upstream by the selection
/// gate (they round-trip into `delete_region`).
fn optimistic_delete_range(
    text: &str,
    cursor: usize,
    key: ProtocolKey,
    mods: Modifiers,
) -> Option<(usize, usize)> {
    if !mods.is_empty() {
        return None;
    }
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    match key {
        ProtocolKey::Backspace => {
            let (start, _) = text[..cursor].char_indices().next_back()?;
            Some((start, cursor))
        }
        ProtocolKey::Delete => {
            let ch = text[cursor..].chars().next()?;
            Some((cursor, cursor + ch.len_utf8()))
        }
        _ => None,
    }
}

/// The literal text `key` inserts when handled optimistically, or
/// `None` for keys that must round-trip through the daemon.
///
/// `Tab` qualifies alongside printable chars because its default
/// binding (`buffer.tab`) reduces to a plain `insert_char(9)` —
/// byte-identical to a self-insert, so the local application cannot
/// diverge from what the daemon will do with the same op. `Enter`
/// does NOT: since Q#AI1 (docs/auto-indent-framing.md) RET binds
/// `edit.newline-and-indent`, whose inserted text depends on the
/// current line's indentation — and round-tripping is also what makes
/// RET rebindings (e.g. the buffer list's visit binding) reachable
/// from this frontend at all. Two caveats are the caller's job:
/// `optimistic_crdt_insert` round-trips when an own-window selection
/// is active (the daemon commands consume the region first — CUA
/// type-over — which a raw op can't), and modified variants (`C-TAB`,
/// …) return `None` here: a keymap may bind them to anything.
fn optimistic_insert_text(key: ProtocolKey, mods: Modifiers, chbuf: &mut [u8; 4]) -> Option<&str> {
    if !is_plain_text_modifiers(mods) {
        return None;
    }
    match key {
        // Auto-pairing Q#AP1: the built-in pair charset always
        // round-trips so the typed opener and the pairing hook's
        // closer land as adjacent daemon-peer undo units, and
        // dispatch-path CUA type-over / skip-over-close apply. An
        // optimistic pair char would put the opener on this
        // frontend's peer with the closer on the daemon's — uncleanly
        // undoable from either side.
        ProtocolKey::Char(ch) if !ch.is_control() && !is_builtin_pair_char(ch) => {
            Some(ch.encode_utf8(chbuf))
        }
        ProtocolKey::Tab if mods.is_empty() => Some("\t"),
        _ => None,
    }
}

/// A vertex buffer reused across frames: rewritten in place while the
/// data fits, reallocated (with slack) when it grows. `render()`
/// previously allocated fresh wgpu buffers for the background / caret
/// / minimap quads on every frame.
/// `(summary generation, surface width, surface height, scroll_top)`
/// — everything the minimap quads depend on.
type MinimapCacheKey = (u64, u32, u32, usize);

struct ReusableVertexBuffer {
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
}

impl ReusableVertexBuffer {
    const fn new() -> Self {
        Self {
            buffer: None,
            capacity: 0,
        }
    }

    /// Upload `bytes`, reusing the existing allocation when possible.
    /// Returns the buffer to bind, or `None` for empty input.
    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &str,
        bytes: &[u8],
    ) -> Option<&wgpu::Buffer> {
        if bytes.is_empty() {
            return None;
        }
        let len = bytes.len() as u64;
        if self.buffer.is_none() || self.capacity < len {
            // Grow with slack so steady selection/minimap churn
            // settles into one allocation.
            let capacity = len.next_power_of_two();
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.capacity = capacity;
        }
        let buffer = self.buffer.as_ref().expect("just ensured");
        queue.write_buffer(buffer, 0, bytes);
        Some(buffer)
    }
}

impl QuadRenderer {
    fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pmacs-gpu quad shader"),
            source: wgpu::ShaderSource::Wgsl(QUAD_SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pmacs-gpu quad pipeline layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pmacs-gpu quad pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: QUAD_VERTEX_STRIDE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &QUAD_VERTEX_ATTRS,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline }
    }

    fn render<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        vertex_buffer: &'pass wgpu::Buffer,
        vertex_count: u32,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.draw(0..vertex_count, 0..1);
    }
}

/// Diagnostic-squiggle pipeline (Q#W1). Parallel to [`QuadRenderer`]
/// but with the [`SQUIGGLE_SHADER`] / [`SQUIGGLE_VERTEX_ATTRS`] vertex
/// format that carries the per-fragment `uv` the sine fragment shader
/// needs.
struct SquiggleRenderer {
    pipeline: wgpu::RenderPipeline,
}

impl SquiggleRenderer {
    fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pmacs-gpu squiggle shader"),
            source: wgpu::ShaderSource::Wgsl(SQUIGGLE_SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pmacs-gpu squiggle pipeline layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pmacs-gpu squiggle pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: SQUIGGLE_VERTEX_STRIDE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &SQUIGGLE_VERTEX_ATTRS,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline }
    }

    fn render<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        vertex_buffer: &'pass wgpu::Buffer,
        vertex_count: u32,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.draw(0..vertex_count, 0..1);
    }
}

impl State {
    #[allow(clippy::too_many_lines)] // linear GPU/font/surface setup; splitting would obscure ordering.
    fn new(event_loop: &ActiveEventLoop, initial_text: &str) -> Self {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("pmacs-gpu")
                        .with_inner_size(winit::dpi::LogicalSize::new(
                            f64::from(INITIAL_WIDTH),
                            f64::from(INITIAL_HEIGHT),
                        )),
                )
                .expect("create window"),
        );

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("request_adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("pmacs-gpu device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            ..wgpu::DeviceDescriptor::default()
        }))
        .expect("request_device");

        let inner_size = window.inner_size();
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(surface_caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: inner_size.width.max(1),
            height: inner_size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        Self::assemble(
            Some(window),
            Some(surface),
            device,
            queue,
            config,
            initial_text,
            &[],
        )
    }

    /// Build a windowless `State` that renders to an offscreen texture, for
    /// the headless render tests (F-014) and the Vterm Stage 3 headless
    /// attach probe. Returns `None` when no GPU adapter is available (a
    /// dev box with no working Vulkan, or CI without lavapipe), so the
    /// caller skips rather than fails.
    fn new_headless(width: u32, height: u32, initial_text: &str) -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("pmacs-gpu headless device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            ..wgpu::DeviceDescriptor::default()
        }))
        .ok()?;
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
        };
        Some(Self::assemble(
            None,
            None,
            device,
            queue,
            config,
            initial_text,
            // Font fixtures exist only in test builds; the release-mode
            // attach probe runs on the bundled face alone.
            headless_extra_font_sources(),
        ))
    }

    /// Build the window-agnostic half of a `State` — font system, glyph
    /// atlas, the three text renderers, quad/squiggle pipelines, and every
    /// render-input field — given an already-created device/queue and the
    /// target `format`. Shared by the windowed `new` and headless
    /// `new_headless` (F-014).
    #[allow(clippy::too_many_lines)] // one large struct literal.
    fn assemble(
        window: Option<Arc<Window>>,
        surface: Option<wgpu::Surface<'static>>,
        device: wgpu::Device,
        queue: wgpu::Queue,
        config: wgpu::SurfaceConfiguration,
        initial_text: &str,
        extra_font_sources: &[&'static [u8]],
    ) -> Self {
        // Pipelines, atlas, and any offscreen texture must all share the
        // render-target format; `config.format` is the single source.
        let format = config.format;
        // Q#F6: construction always starts at the default metrics;
        // a preference arriving later re-metrics via apply_font_facts.
        let fm = FontMetrics::default();
        // Q#F6: sanitized current-order assembly — system fonts, the
        // bundled face (ID retained), the same-family collision
        // filter, generic defaults, THEN FontSystem construction so
        // its monospace-ID set sees the final database.
        let (mut font_system, font_defaults) = build_font_system(extra_font_sources);
        // Inline-math slice (Q#MS7): the layout engine over the bundled
        // Latin Modern Math. Failure is surfaced once and disables only
        // the math path — spans keep rendering as source.
        let math_engine = match math_layout::MathLayout::new(math_layout::LATIN_MODERN_MATH) {
            Ok(engine) => Some(engine),
            Err(e) => {
                eprintln!("pmacs-gpu: bundled math font unusable ({e:?}); inline math disabled");
                None
            }
        };
        let math_budget = math_code_budget(fm);
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let mut viewport = Viewport::new(&device, &cache);
        viewport.update(
            &queue,
            Resolution {
                width: config.width,
                height: config.height,
            },
        );
        let mut atlas = TextAtlas::new(&device, &queue, &cache, format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        // Q#CM1 — a second renderer so the menu draws as a top layer.
        let menu_text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        // Q#MB1 — a third renderer for the minibuffer dropdown layer.
        let mb_text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        // Inline-math slice (Q#MS6) — a renderer for the math glyph layer.
        let math_text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        // Arc 1a Q#C5 — a renderer for the completion dropdown layer.
        let completion_text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        // UX gutter — a renderer for the line-number layer.
        let gutter_text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        // Vterm Stage 3 — terminal glyphs draw in their own layer with
        // their own clip; interleaving them with document text would
        // subject them to the document's gutter offset and wrapping.
        let terminal_text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        // Bottom panel Stage 2B-3 — the band's glyphs get their own layer
        // and their own clip for the same reason: they answer to the
        // daemon's panel grid, not to the document's gutter or wrapping.
        let panel_text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        let quad_renderer = QuadRenderer::new(&device, format);
        let squiggle_renderer = SquiggleRenderer::new(&device, format);

        // Smaller font in attach mode (file contents tend to be more
        // than one line); larger only fits "hello, pmacs"-shaped
        // strings. Picked metrics that look reasonable for code at
        // 800px wide.
        let mut buffer = Buffer::new(
            &mut font_system,
            Metrics::new(fm.code_font_size(), fm.code_line_height()),
        );
        // Declare the document's wrap mode instead of inheriting
        // cosmic-text's constructor default (`Wrap::WordOrGlyph`).
        //
        // `Glyph`, not `None`: `ui.line-wrap` defaults to `wrap`, so a
        // frontend that has not yet been told anything — or is talking
        // to a pre-v22 daemon that never will be — should already be in
        // the default mode. What changes versus the inherited default is
        // only the break rule, word to character, which is the
        // cross-frontend parity this stage buys.
        //
        // Declaring it is load-bearing rather than tidy: without it the
        // document runs on `WordOrGlyph` until some message happens to
        // change it, so a frontend talking to a pre-v22 daemon word
        // wraps forever while the grid renderer character wraps.
        buffer.set_wrap(&mut font_system, Wrap::Glyph);
        buffer.set_size(
            &mut font_system,
            Some(config.width as f32),
            Some(config.height as f32),
        );
        let mut status_buffer = Buffer::new(
            &mut font_system,
            Metrics::new(fm.status_font_size(), fm.status_line_height()),
        );
        status_buffer.set_size(
            &mut font_system,
            Some(config.width as f32),
            Some(fm.status_band_height()),
        );
        status_buffer.set_wrap(&mut font_system, Wrap::None);
        let mut status_left_buffer = Buffer::new(
            &mut font_system,
            Metrics::new(fm.status_font_size(), fm.status_line_height()),
        );
        status_left_buffer.set_size(
            &mut font_system,
            Some(config.width as f32),
            Some(fm.status_band_height()),
        );
        status_left_buffer.set_wrap(&mut font_system, Wrap::None);
        let mut menu_buffer = Buffer::new(
            &mut font_system,
            Metrics::new(fm.menu_font_size(), fm.menu_line_height()),
        );
        menu_buffer.set_size(
            &mut font_system,
            Some(MENU_MAX_WIDTH),
            Some(config.height as f32),
        );
        // Rows stay rows (framing Q#F6): the three row-oriented popup
        // buffers never wrap — their protocols, row windows, selection
        // quads, and hit tests all assign exactly one row height per
        // source line, and a label wrapping after a font change would
        // paint on a second visual row that hit-tests as the following
        // item. The pixel bounds keep owning horizontal clipping.
        menu_buffer.set_wrap(&mut font_system, Wrap::None);
        let mut mb_buffer = Buffer::new(
            &mut font_system,
            Metrics::new(fm.mb_drop_font_size(), fm.mb_drop_line_height()),
        );
        mb_buffer.set_size(
            &mut font_system,
            Some(MB_DROP_MAX_WIDTH),
            Some(config.height as f32),
        );
        mb_buffer.set_wrap(&mut font_system, Wrap::None);
        // Completion dropdown buffer (Arc 1a): the minibuffer
        // dropdown's metrics, its own layer.
        let mut completion_buffer = Buffer::new(
            &mut font_system,
            Metrics::new(fm.mb_drop_font_size(), fm.mb_drop_line_height()),
        );
        completion_buffer.set_size(
            &mut font_system,
            Some(MB_DROP_MAX_WIDTH),
            Some(config.height as f32),
        );
        completion_buffer.set_wrap(&mut font_system, Wrap::None);
        // Line-number gutter buffer (UX gutter arc): same font size + line
        // height as the code buffer so its rows align one-for-one.
        let mut gutter_buffer = Buffer::new(
            &mut font_system,
            Metrics::new(fm.code_font_size(), fm.code_line_height()),
        );
        gutter_buffer.set_size(
            &mut font_system,
            Some(config.width as f32),
            Some(config.height as f32),
        );
        let (current_line_starts, current_line_char_starts) = line_offset_tables(initial_text);

        let mut state = Self {
            window,
            device,
            queue,
            surface,
            config,
            font_system,
            font_defaults,
            fm: FontMetrics::default(),
            resolved_family: DEFAULT_FONT_FAMILY.to_owned(),
            measured_mono_advance: None,
            code_scroll_residual: 0.0,
            code_scroll_left: 0.0,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            quad_renderer,
            squiggle_renderer,
            buffer,
            current_text: initial_text.to_owned(),
            current_line_starts,
            current_line_char_starts,
            current_line_shapes: minimap_line_shapes(initial_text),
            loro_doc: None,
            loro_text_delta_batches: None,
            loro_text_subscription: None,
            current_buffer_id: None,
            current_spans: Vec::new(),
            current_decorations: Vec::new(),
            current_adornments: Vec::new(),
            current_summary: None,
            peer_presences: HashMap::new(),
            own_cursor: None,
            scroll_top: 0,
            view_range: (0, 0),
            last_viewport_sent: None,
            local_frontend_id: None,
            dispatch_idle: false,
            clipboard: None,
            cursor_fresh: false,
            optimistic_cursor_floor: None,
            deferred_round_trip_keys: Vec::new(),
            optimistic_floor_set_at: None,
            unconfirmed_edits: Vec::new(),
            current_hit_runs: Vec::new(),
            projected_line_starts: vec![0],
            pointer_pos: None,
            pointer_drag_active: false,
            last_pointer_sent_byte: None,
            last_pointer_down: None,
            minimap_scrub_active: false,
            edge_scroll_dir: None,
            edge_scroll_last: None,
            styled_redraw_deadline: None,
            hit_map_dirty: false,
            line_chunk_cache: Vec::new(),
            line_math_cache: Vec::new(),
            math_engine,
            math_budget,
            shaped_top: 0,
            bg_vertex_buffer: ReusableVertexBuffer::new(),
            squiggle_vertex_buffer: ReusableVertexBuffer::new(),
            caret_vertex_buffer: ReusableVertexBuffer::new(),
            minimap_vertex_buffer: ReusableVertexBuffer::new(),
            status_buffer,
            status_runs: None,
            status_left_buffer,
            status_left_runs: None,
            statusline_segments: None,
            status_facts: None,
            search_prompt: None,
            minibuffer: None,
            menu: None,
            menu_anchor_px: (0.0, 0.0),
            menu_buffer,
            menu_text_renderer,
            menu_bg_vertex_buffer: ReusableVertexBuffer::new(),
            mb_buffer,
            mb_text_renderer,
            math_text_renderer,
            mb_bg_vertex_buffer: ReusableVertexBuffer::new(),
            completion: None,
            completion_buffer,
            completion_text_renderer,
            completion_bg_vertex_buffer: ReusableVertexBuffer::new(),
            minimap_cache: None,
            line_numbers: LineNumberMode::Off,
            faces: HashMap::new(),
            gutter_buffer,
            gutter_text_renderer,
            terminal: None,
            panel: PanelBand::default(),
            panel_text_buffers: Vec::new(),
            panel_wire: false,
            panel_metrics_changed: false,
            last_terminal_size_sent: None,
            terminal_frame_error_latched: false,
            last_terminal_pointer_cell: None,
            terminal_text_buffers: Vec::new(),
            terminal_text_renderer,
            panel_text_renderer,
        };
        // Real drawable dimensions from construction (framing Q#F6):
        // wrapping and `shape_until_cursor` must use the same clip the
        // painter uses, and a v16 daemon never sends the FontFacts
        // that would sync them later.
        state.sync_buffer_dimensions();
        // Shape the initial text through the shared chunk path so the
        // `buffer.lines` ↔ `line_chunk_cache` invariant holds from
        // construction — the caret/anchor projection (framing Q#F6)
        // inverts the per-line chunk cache, and a directly-set buffer
        // would leave it empty.
        state.reshape();
        state
    }

    fn set_frontend_id(&mut self, frontend_id: FrontendId) {
        self.local_frontend_id = Some(frontend_id);
        if let Some(doc) = self.loro_doc.as_ref()
            && let Err(e) = doc.set_peer_id(frontend_id.0)
        {
            eprintln!("pmacs-gpu: failed to set local Loro peer id: {e:?}");
        }
    }

    /// `true` while the daemon is intercepting keystrokes — an active
    /// incremental search (Q#SR5), surfaced by a live `SearchPrompt`, or
    /// the daemon reporting non-idle (`DispatchIdle { idle: false }` for
    /// a minibuffer / pending prefix). In this state the GUI round-trips
    /// every key to the daemon's handler instead of optimistically
    /// applying it to the buffer.
    fn daemon_intercepts_keys(&self) -> bool {
        // Q#CM1 / Q#MB1 — an open menu or minibuffer shadows the keymap
        // like search: every key round-trips so the daemon's
        // `dispatch_menu_key` / minibuffer handler drives it.
        self.search_prompt.is_some()
            || self.menu.is_some()
            || self.minibuffer.is_some()
            || !self.dispatch_idle
    }

    /// Shared eligibility gates for the optimistic edit paths
    /// (insert + delete). `None` ⇒ the key must round-trip:
    /// - dispatcher busy (minibuffer/prefix flows own the keys), or
    ///   the cursor isn't authoritative;
    /// - CUA region semantics: an own-window selection means typing
    ///   replaces and Backspace/Delete consume the region — those
    ///   semantics live in the daemon's region-aware commands, which
    ///   a raw `CrdtOp` bypasses. (Our own selection arrives as a
    ///   `Selection` decoration; peer selections live in
    ///   `peer_presences` and don't gate.)
    /// - bookkeeping: no frontend id / cursor / matching buffer /
    ///   replica doc, or the peer id can't be set.
    fn optimistic_edit_eligible(&self) -> Option<(OwnCursor, u64)> {
        if !self.dispatch_idle || !self.cursor_fresh {
            return None;
        }
        if self
            .current_decorations
            .iter()
            .any(|d| d.kind == DecorationKind::Selection)
        {
            return None;
        }
        let frontend_id = self.local_frontend_id?;
        let own = self.own_cursor?;
        if self.current_buffer_id != Some(own.buffer_id) {
            return None;
        }
        let doc = self.loro_doc.as_ref()?;
        let peer_id = frontend_id.0;
        if doc.peer_id() != peer_id
            && let Err(e) = doc.set_peer_id(peer_id)
        {
            eprintln!("pmacs-gpu: failed to set optimistic Loro peer id: {e:?}");
            return None;
        }
        Some((own, peer_id))
    }

    fn optimistic_crdt_insert(&mut self, key: ProtocolKey, mods: Modifiers) -> Option<CrdtOpSend> {
        let mut chbuf = [0u8; 4];
        let insert = optimistic_insert_text(key, mods, &mut chbuf)?;
        let (own, peer_id) = self.optimistic_edit_eligible()?;
        let cursor = usize::try_from(own.byte).ok()?;
        if cursor > self.current_text.len() || !self.current_text.is_char_boundary(cursor) {
            return None;
        }
        let doc = self.loro_doc.as_ref()?;
        let delta_batches = self.loro_text_delta_batches.clone()?;
        clear_loro_text_delta_batches(&delta_batches);
        let before = doc.oplog_vv();
        if let Err(e) = doc
            .get_text(LORO_TEXT_CONTAINER)
            .insert_utf8(cursor, insert)
        {
            eprintln!("pmacs-gpu: optimistic insert failed: {e:?}");
            return None;
        }
        let bytes = doc
            .export(ExportMode::updates(&before))
            .expect("export local optimistic Loro update");
        let drained = drain_loro_text_delta_batches(&delta_batches);
        let predicted = OwnCursor {
            buffer_id: own.buffer_id,
            byte: own.byte.saturating_add(insert.len() as u64),
        };
        Some(self.finish_optimistic_edit(&drained, predicted, peer_id, bytes))
    }

    /// Optimistic single-codepoint Backspace / Delete. Mirrors the
    /// insert path: the daemon's `buffer.delete-backward/-forward`
    /// no-region behavior is exactly "delete one codepoint", so the
    /// local application cannot diverge; region deletes are excluded
    /// by the selection gate (they round-trip into `delete_region`),
    /// and modified variants (C-BS word delete, …) round-trip via
    /// `optimistic_delete_range` returning `None`. The daemon applies
    /// the op through its single-delete CRDT hot path.
    fn optimistic_crdt_delete(&mut self, key: ProtocolKey, mods: Modifiers) -> Option<CrdtOpSend> {
        if !matches!(key, ProtocolKey::Backspace | ProtocolKey::Delete) {
            return None;
        }
        let (own, peer_id) = self.optimistic_edit_eligible()?;
        let cursor = usize::try_from(own.byte).ok()?;
        let (start, end) = optimistic_delete_range(&self.current_text, cursor, key, mods)?;
        let doc = self.loro_doc.as_ref()?;
        let delta_batches = self.loro_text_delta_batches.clone()?;
        clear_loro_text_delta_batches(&delta_batches);
        let before = doc.oplog_vv();
        if let Err(e) = doc
            .get_text(LORO_TEXT_CONTAINER)
            .delete_utf8(start, end - start)
        {
            eprintln!("pmacs-gpu: optimistic delete failed: {e:?}");
            return None;
        }
        let bytes = doc
            .export(ExportMode::updates(&before))
            .expect("export local optimistic Loro update");
        let drained = drain_loro_text_delta_batches(&delta_batches);
        let predicted = OwnCursor {
            buffer_id: own.buffer_id,
            byte: start as u64,
        };
        Some(self.finish_optimistic_edit(&drained, predicted, peer_id, bytes))
    }

    /// Common tail of the optimistic edit paths: patch the local text
    /// from the drained Loro deltas (journaling them for
    /// incoming-frame translation), predict the cursor + arm the
    /// confirmation floor, follow the caret, and package the wire op.
    fn finish_optimistic_edit(
        &mut self,
        drained: &[Vec<loro::TextDelta>],
        predicted: OwnCursor,
        peer_id: u64,
        bytes: Vec<u8>,
    ) -> CrdtOpSend {
        if drained.is_empty() {
            let text = self
                .loro_doc
                .as_ref()
                .map(|doc| doc.get_text(LORO_TEXT_CONTAINER).to_string());
            if let Some(text) = text {
                self.set_text(&text);
            }
            // Cache rebuilt wholesale — there are no translated
            // anchors left for frame translation to protect.
            self.unconfirmed_edits.clear();
        } else {
            match self.apply_loro_text_delta_batches(drained) {
                Ok(edits) => {
                    // Journal this keystroke so producer frames the
                    // daemon computed before integrating it can be
                    // translated on arrival (see `unconfirmed_edits`).
                    // The scalar is read *after* the local edit, so
                    // any frame stamped at or beyond it includes us.
                    let scalar = self.loro_doc.as_ref().map_or(0, loro_version_scalar);
                    self.unconfirmed_edits
                        .extend(edits.into_iter().map(|e| (scalar, e)));
                }
                Err(reason) => {
                    eprintln!(
                        "pmacs-gpu: optimistic text update failed ({reason}); falling back to \
                         full materialization"
                    );
                    let text = self
                        .loro_doc
                        .as_ref()
                        .map(|doc| doc.get_text(LORO_TEXT_CONTAINER).to_string());
                    if let Some(text) = text {
                        self.set_text(&text);
                    }
                    self.unconfirmed_edits.clear();
                }
            }
        }
        self.own_cursor = Some(predicted);
        self.optimistic_cursor_floor = Some(predicted);
        self.optimistic_floor_set_at = Some(std::time::Instant::now());
        // Follow the caret NOW rather than when the daemon's
        // `CursorByte` confirms — an optimistic edit on the bottom
        // visible line that wraps (or a Backspace pulling the caret
        // above the top) moves it outside the slice, and waiting a
        // round trip to scroll reads as a hitch. Visual-run aware
        // (framing Q#F6): the confirming identical `CursorByte` has
        // `moved == false`, so deferring wrapped-run repair here
        // would leave a newly wrapped caret off-screen indefinitely.
        self.ensure_caret_painted();
        // Q#MS5/F4: the text applied above re-chunked under the OLD
        // caret; the effective caret only just moved. Suppression keys
        // on `own_cursor`, so re-run the compare — without this, a
        // typed char that lands the caret against a span boundary
        // renders one keystroke stale.
        self.refresh_math_suppression();
        let viewport = self.viewport_send_if_changed(predicted.buffer_id);
        CrdtOpSend {
            buffer_id: predicted.buffer_id,
            op: CrdtOp { peer_id, bytes },
            viewport,
        }
    }

    fn mark_cursor_stale_after_round_trip(&mut self) {
        self.cursor_fresh = false;
    }

    fn apply_loro_text_delta_batches(
        &mut self,
        delta_batches: &[Vec<loro::TextDelta>],
    ) -> Result<Vec<TextProjectionEdit>, &'static str> {
        let caret_was_painted = self.caret_painted_in_code_clip();
        let line_count_before = self.current_line_starts.len();
        let edits = apply_loro_text_delta_batches(
            &mut self.current_text,
            &mut self.current_line_starts,
            &mut self.current_line_char_starts,
            delta_batches,
        )?;
        if edits.is_empty() {
            return Ok(edits);
        }
        self.refresh_minimap_shapes_after_edits(&edits, line_count_before);
        self.translate_cached_anchors(&edits);
        // A newline edit can cross a gutter digit boundary (9 -> 10,
        // 99 -> 100). Synchronize the painter-derived code width
        // before any reshape so cosmic-text wraps at the final clip.
        let geometry_changed = self.sync_buffer_dimensions();
        // Q#R1 — the keystroke case (one edit, no line-structure
        // change) re-shapes only the affected BufferLine; everything
        // else falls back to the full slice reshape.
        let single_line_edit = edits.len() == 1
            && self.current_line_starts.len() == line_count_before
            && !self.current_text
                [edits[0].start as usize..(edits[0].start + edits[0].inserted_len) as usize]
                .contains('\n');
        if geometry_changed || !(single_line_edit && self.try_reshape_line(edits[0])) {
            self.reshape();
        }
        if geometry_changed && caret_was_painted {
            self.ensure_caret_painted();
        }
        Ok(edits)
    }

    fn translate_cached_anchors(&mut self, edits: &[TextProjectionEdit]) {
        for edit in edits {
            translate_style_spans(&mut self.current_spans, *edit);
            translate_decorations(&mut self.current_decorations, *edit);
            translate_inline_adornments(&mut self.current_adornments, *edit);
        }
    }

    /// Keep minimap horizontal geometry in lock-step with accepted text
    /// edits instead of waiting for the next debounced style summary.
    /// The common one-line edit updates one cached shape; line-structure
    /// or batched edits rebuild the table because their intermediate
    /// coordinates need not describe the final line partition.
    fn refresh_minimap_shapes_after_edits(
        &mut self,
        edits: &[TextProjectionEdit],
        line_count_before: usize,
    ) {
        if edits.len() == 1
            && self.current_line_starts.len() == line_count_before
            && self.current_line_shapes.len() == self.current_line_starts.len()
        {
            let line = self
                .current_line_starts
                .partition_point(|&start| start <= edits[0].start)
                .saturating_sub(1);
            let start = self.current_line_starts[line] as usize;
            let end = self
                .current_line_starts
                .get(line + 1)
                .map_or(self.current_text.len(), |next| *next as usize - 1);
            self.current_line_shapes[line] = minimap_line_shape(&self.current_text[start..end]);
        } else {
            self.current_line_shapes = minimap_line_shapes(&self.current_text);
        }
        self.minimap_cache = None;
    }

    /// Drop journal entries already reflected in a producer frame
    /// stamped `generation` — see the `unconfirmed_edits` field docs.
    fn prune_unconfirmed_edits(&mut self, generation: u64) {
        self.unconfirmed_edits
            .retain(|(scalar, _)| *scalar > generation);
    }

    fn optimistic_floor_timed_out(&self) -> bool {
        self.optimistic_floor_set_at
            .is_some_and(|armed| armed.elapsed() >= FLOOR_CONFIRM_TIMEOUT)
    }

    /// Escape hatch: release a floor the daemon never confirmed so
    /// deferred round-trip keys can't wedge forever. Dropping
    /// `cursor_fresh` falls the GPU back to round-trip mode until the
    /// next `CursorByte` resynchronizes the cursor.
    fn release_timed_out_floor(&mut self) {
        if self.optimistic_cursor_floor.is_some() && self.optimistic_floor_timed_out() {
            eprintln!(
                "pmacs-gpu: optimistic cursor unconfirmed after {FLOOR_CONFIRM_TIMEOUT:?}; \
                 falling back to round-trip input"
            );
            self.optimistic_cursor_floor = None;
            self.optimistic_floor_set_at = None;
            self.cursor_fresh = false;
        }
    }

    fn defer_round_trip_key_if_needed(&mut self, key: ProtocolKey, mods: Modifiers) -> bool {
        if self.optimistic_cursor_floor.is_none() && self.deferred_round_trip_keys.is_empty() {
            return false;
        }
        self.cursor_fresh = false;
        self.deferred_round_trip_keys.push((key, mods));
        true
    }

    fn take_ready_round_trip_keys(&mut self) -> Vec<(ProtocolKey, Modifiers)> {
        if self.optimistic_cursor_floor.is_some() || self.deferred_round_trip_keys.is_empty() {
            return Vec::new();
        }
        self.cursor_fresh = false;
        std::mem::take(&mut self.deferred_round_trip_keys)
    }

    /// Replace the rendered text with `text` and request a redraw.
    /// Returns `false` when `text` is byte-identical to the current
    /// rendering (avoids the re-shape cost when an unchanged buffer
    /// ticks).
    ///
    /// Replaces the rope text and routes through `reshape` so the
    /// rich-text rendering uses the current spans, decorations, and
    /// inline adornments. When called from the `CrdtOp` path (text
    /// shifted under existing source anchors) those anchors are
    /// momentarily stale relative to the new byte positions —
    /// `reshape` clamps via `range.end.min(text_len)` so rendering is
    /// safe, but visual styling may be off until the daemon's next
    /// semantic frame catches up. A real artifact; classified as a
    /// known Phase A limitation rather than a bug.
    fn set_text(&mut self, text: &str) -> bool {
        if self.current_text == text {
            return false;
        }
        let caret_was_painted = self.caret_painted_in_code_clip();
        self.current_text.clear();
        self.current_text.push_str(text);
        let (line_starts, line_char_starts) = line_offset_tables(text);
        self.current_line_starts = line_starts;
        self.current_line_char_starts = line_char_starts;
        self.current_line_shapes = minimap_line_shapes(text);
        self.minimap_cache = None;
        let geometry_changed = self.sync_buffer_dimensions();
        self.reshape();
        if geometry_changed && caret_was_painted {
            self.ensure_caret_painted();
        }
        true
    }

    /// Apply one `InstanceMessage`; return a follow-up
    /// `ViewportSend` if the message requires the main loop to fire
    /// one back at the daemon.
    ///
    /// Session 4 introduced four variants; session 5 adds
    /// `Decorations`:
    /// - `BufferSnapshot` — bootstrap a fresh `LoroDoc`, extract text,
    ///   request the daemon scope styling to the new buffer (return a
    ///   Viewport send-back).
    /// - `CrdtOp` — apply incremental updates to the doc; text
    ///   patched from Loro's diff event.
    /// - `StyleSpans` — replace or merge per the M11.4 dirty-segment
    ///   rule; reshape the rich-text rendering.
    /// - `Decorations` — same M11.4 shape as `StyleSpans` but for the
    ///   `DecorationKind` set (diagnostics, selection, current line,
    ///   search match). Session 5 renders diagnostic kinds as fg color
    ///   overrides; background-kind decorations are accumulated but
    ///   not painted (see session 5's deferred quad-pipeline finding).
    /// - `InlineAdornments` — replace the scoped virtual-text set and
    ///   reshape the display projection. Session 6 consumes `AtOffset`
    ///   text adornments (LSP inlay hints); other placements/content
    ///   remain explicitly deferred.
    /// - `FileStyleSummary` — replace the whole-file minimap summary.
    ///   Session 7 renders it as a right-side per-line style overview
    ///   plus a visible-window affordance.
    /// - `Goodbye` — surfaced via the reader thread's clean-EOF path,
    ///   not handled here.
    ///
    /// The grid variants (`CellDelta`, `Cursor`, `CursorByte`) are
    /// ignored — pmacs-gpu lays out locally and tracks the cursor via
    /// `PresenceUpdate` (session 9.3). Remaining semantic variants land
    /// in subsequent Phase A sessions.
    /// Lazily-created OS clipboard handle (Q#CM6). Returns `None` if the
    /// platform clipboard can't be opened, so callers degrade to no-ops.
    fn os_clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(c) => self.clipboard = Some(c),
                Err(e) => {
                    eprintln!("pmacs-gpu: OS clipboard unavailable: {e}");
                    return None;
                }
            }
        }
        self.clipboard.as_mut()
    }

    /// Read the OS clipboard as bytes (for Ctrl-V → `Paste`). `None` on
    /// any failure (empty / non-text / unavailable).
    fn read_os_clipboard(&mut self) -> Option<Vec<u8>> {
        match self.os_clipboard()?.get_text() {
            Ok(s) => Some(s.into_bytes()),
            Err(e) => {
                eprintln!("pmacs-gpu: clipboard read failed: {e}");
                None
            }
        }
    }

    /// Write bytes to the OS clipboard (for an inbound
    /// `Signal::Clipboard` after a daemon copy/cut). Lossy UTF-8; the
    /// daemon only ever sends valid document text.
    fn write_os_clipboard(&mut self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes).into_owned();
        if let Some(c) = self.os_clipboard()
            && let Err(e) = c.set_text(text)
        {
            eprintln!("pmacs-gpu: clipboard write failed: {e}");
        }
    }

    #[allow(clippy::too_many_lines)] // per-variant match dispatcher; one arm per InstanceMessage.
    fn apply_attach_message(&mut self, msg: InstanceMessage) -> Option<ViewportSend> {
        match msg {
            InstanceMessage::BufferSnapshot {
                buffer_id,
                crdt_snapshot,
            } => {
                let doc = loro::LoroDoc::new();
                if let Some(frontend_id) = self.local_frontend_id
                    && let Err(e) = doc.set_peer_id(frontend_id.0)
                {
                    eprintln!("pmacs-gpu: failed to set snapshot Loro peer id: {e:?}");
                }
                if let Err(e) = doc.import(&crdt_snapshot) {
                    eprintln!("pmacs-gpu: BufferSnapshot import failed: {e:?}");
                    return None;
                }
                let text = doc.get_text(LORO_TEXT_CONTAINER).to_string();
                let (text_delta_batches, text_subscription) = subscribe_loro_text(&doc);
                self.loro_text_subscription = None;
                self.loro_text_delta_batches = None;
                self.loro_doc = Some(doc);
                self.loro_text_delta_batches = Some(text_delta_batches);
                self.loro_text_subscription = Some(text_subscription);
                self.current_buffer_id = Some(buffer_id);
                // New buffer ⇒ drop any prior styling/decorations;
                // the next StyleSpans / Decorations frame for this
                // buffer is authoritative.
                self.current_spans.clear();
                self.current_decorations.clear();
                self.current_adornments.clear();
                self.current_summary = None;
                // Peer cursors and our own cursor are anchored in the
                // prior buffer's coordinate space; drop them so a stale
                // offset can't paint against the new rope before the
                // next PresenceUpdate / CursorByte arrives.
                self.peer_presences.clear();
                self.own_cursor = None;
                // The completion popup too (Arc 1a): its anchor is a
                // byte in the prior buffer, and the producer never
                // ships a close for a viewport that no longer exists
                // (first-sight of the new buffer stays silent) — a
                // retained popup would render against the new rope AND
                // keep hijacking Esc/RET/TAB. The daemon-side session
                // was already invalidated by the switch.
                self.completion = None;
                // PR #120 round 3 finding 1 — the remaining
                // buffer-scoped facts, same reasoning: search and menu
                // popups anchor in the prior buffer AND gate key and
                // pointer interception (`daemon_intercepts_keys`, the
                // pointer arms), and the new buffer's first CLOSED
                // state is suppressed daemon-side, so no close message
                // ever comes — a retained popup would hijack input
                // forever. The status band's name/counts describe the
                // buffer we just left; the producer's reset contract
                // re-ships the new buffer's facts on its first frame.
                // The minibuffer deliberately survives: it is one
                // global core instance, matching the producer's
                // surviving `last_minibuffer` baseline.
                self.search_prompt = None;
                self.menu = None;
                self.status_facts = None;
                self.statusline_segments = None;
                self.status_runs = None;
                self.status_left_runs = None;
                self.cursor_fresh = false;
                self.optimistic_cursor_floor = None;
                self.optimistic_floor_set_at = None;
                self.deferred_round_trip_keys.clear();
                self.unconfirmed_edits.clear();
                // New buffer ⇒ back to the top, and force a viewport
                // re-declaration for the new buffer's scoped range.
                // The caret-follow residual is buffer-scoped view
                // state and resets with it; the global font
                // preference/metrics survive (framing Q#F6).
                self.scroll_top = 0;
                self.code_scroll_residual = 0.0;
                // Stage 5 (Q#G2): the horizontal offset is viewport
                // state tied to the document being shown, so it resets
                // with the other two. Without this a buffer switch
                // inherits the PREVIOUS document's leftward viewport
                // and renders the new buffer scrolled sideways until a
                // cursor motion repairs it — a symptom nothing about
                // the new buffer explains.
                self.code_scroll_left = 0.0;
                self.last_viewport_sent = None;
                // Vterm Stage 3 — a snapshot ALWAYS leaves terminal
                // mode, including a terminal→terminal switch. The prior
                // frame describes another session's screen, and the
                // daemon has already dropped its own baseline, so the
                // next valid frame is authoritative for whatever this
                // buffer turns out to be.
                self.exit_terminal_mode();
                if !self.set_text(&text) {
                    // Even byte-identical A -> B snapshots can clear a
                    // prior minimap, so the new buffer's shaping clip
                    // must still be synchronized before rebuilding.
                    self.sync_buffer_dimensions();
                    self.reshape();
                }
                self.viewport_send_if_changed(buffer_id)
            }
            InstanceMessage::CrdtOp { buffer_id, op } => {
                if self.current_buffer_id != Some(buffer_id) {
                    // Edit op for a different buffer than we currently
                    // render. Ignore for now (multi-buffer is a future
                    // session); when buffer-switching lands we'll
                    // index ops by buffer.
                    return None;
                }
                let Some(doc) = self.loro_doc.as_ref() else {
                    // Mid-attach race: ops before snapshot. The
                    // snapshot will have the ops baked in.
                    return None;
                };
                let delta_batches = self.loro_text_delta_batches.clone();
                if let Some(delta_batches) = delta_batches.as_ref() {
                    clear_loro_text_delta_batches(delta_batches);
                }
                let import_status = match doc.import(&op.bytes) {
                    Ok(status) => status,
                    Err(e) => {
                        eprintln!("pmacs-gpu: CrdtOp import failed: {e:?}");
                        return None;
                    }
                };
                // NOTE: `current_spans` / `current_decorations` index
                // into the *pre-edit* byte positions. The producer's
                // next render frame (in pmacs core, post-T M11.7
                // generation-transition fix) ships `full=true`
                // styling for buffers whose generation advanced, so
                // the next message replaces the stale items
                // wholesale via `replace_style_spans` /
                // `replace_decorations`. The single-frame gap
                // between CrdtOp arrival and that next frame paints
                // styling at stale byte positions — the session-4
                // documented "one-frame stale" artifact. A previous
                // attempt to fix it by clearing both vectors here
                // (`49785c4`) was reverted because the producer's
                // *incremental* updates ship dirty-range spans only,
                // and an emptied cache loses the non-dirty viewport
                // styling entirely.
                //
                // InlineAdornments use whole-set suppression rather
                // than dirty segments, so the same ownership rule
                // applies here: keep the last set until the producer
                // sends a replacement. Session 8 closed the stale
                // inlay case producer-side: `didChange` marks the
                // inlay store stale, and the producer sends one empty
                // replacement to clear cached virtual text until a
                // fresh `textDocument/inlayHint` response arrives.
                let delta_batches = delta_batches
                    .as_ref()
                    .map(drain_loro_text_delta_batches)
                    .unwrap_or_default();
                if delta_batches.is_empty() {
                    if !import_status.success.is_empty() {
                        let text = self
                            .loro_doc
                            .as_ref()
                            .map(|doc| doc.get_text(LORO_TEXT_CONTAINER).to_string());
                        if let Some(text) = text {
                            self.set_text(&text);
                        }
                        self.unconfirmed_edits.clear();
                    }
                } else {
                    match self.apply_loro_text_delta_batches(&delta_batches) {
                        Ok(edits) => {
                            // A daemon-originated edit shifts the text
                            // under any still-unconfirmed optimistic
                            // edits. Rebase the journal's anchors so
                            // frames that include this edit (but not
                            // ours) translate correctly. Entries are
                            // inserts (start == old_end) or
                            // single-codepoint deletes; both rebase by
                            // position translation, clamped so a range
                            // can't invert.
                            for incoming in &edits {
                                for (_, pending) in &mut self.unconfirmed_edits {
                                    pending.start =
                                        translate_byte_position(pending.start, *incoming);
                                    pending.old_end =
                                        translate_byte_position(pending.old_end, *incoming)
                                            .max(pending.start);
                                }
                            }
                        }
                        Err(reason) => {
                            eprintln!(
                                "pmacs-gpu: incremental CRDT text update failed ({reason}); \
                                 falling back to full materialization"
                            );
                            let text = self
                                .loro_doc
                                .as_ref()
                                .map(|doc| doc.get_text(LORO_TEXT_CONTAINER).to_string());
                            if let Some(text) = text {
                                self.set_text(&text);
                            }
                            self.unconfirmed_edits.clear();
                        }
                    }
                }
                // Local typing usually shifts only the viewport's end
                // byte while the top visible source line stays fixed.
                // The declared range includes overscan, and the daemon's
                // generation bump already forces a full style resync, so
                // re-declaring on every byte is mostly write amplification.
                // Re-declare here only if the viewport origin moved (for
                // example because an edit before `scroll_top` shifted the
                // top line); scroll/resize/snapshot still send exact ranges.
                self.viewport_send_if_origin_changed(buffer_id)
            }
            InstanceMessage::StyleSpans {
                buffer_id,
                generation,
                full,
                segments,
            } => {
                if self.current_buffer_id != Some(buffer_id) {
                    return None;
                }
                // The producer computed this frame against the daemon
                // text at `generation` (its CRDT version scalar). Any
                // optimistic local inserts the daemon hadn't integrated
                // yet shift the frame's byte ranges; translate them so
                // the repaint doesn't flash every color after the
                // cursor a few bytes left of its glyphs for one frame
                // (the typing shimmer).
                self.prune_unconfirmed_edits(generation);
                let segments = translate_style_segments(segments, &self.unconfirmed_edits);
                if full {
                    self.replace_style_spans(segments);
                } else {
                    self.merge_style_spans(segments);
                }
                // Re-shape only lines whose styling actually changed
                // — a parse-settle frame after a burst usually
                // recolors a line or two, and a scroll-triggered
                // resync only the newly exposed ones.
                self.refresh_changed_lines();
                None
            }
            InstanceMessage::Decorations {
                buffer_id,
                generation,
                full,
                segments,
            } => {
                if self.current_buffer_id != Some(buffer_id) {
                    return None;
                }
                // Same staleness translation as the StyleSpans arm.
                self.prune_unconfirmed_edits(generation);
                let segments = translate_decoration_segments(segments, &self.unconfirmed_edits);
                if full {
                    self.replace_decorations(segments);
                } else {
                    self.merge_decorations(segments);
                }
                // Every decoration kind is now a quad (backgrounds
                // for Selection/CurrentLine, underline bars for the
                // diagnostics — the fg-recolor path retired with T
                // M4.6 parity), and quads rebuild cheaply per frame
                // in `render()`. No decoration change needs a
                // reshape, so none triggers one — diagnostic
                // publishes no longer pay set_rich_text +
                // shape_until_scroll... with ONE exception since the
                // inline-math slice: a Selection endpoint is a Q#MS11
                // suppression gate, so a selection change re-runs the
                // per-line compare when the slice could hold math
                // (no-op for every other decoration kind and for
                // math-free text).
                self.refresh_math_suppression();
                self.request_redraw();
                None
            }
            InstanceMessage::InlineAdornments { buffer_id, items } => {
                if self.current_buffer_id != Some(buffer_id) {
                    return None;
                }
                self.current_adornments = items;
                self.current_adornments.sort_by_key(|a| a.at);
                self.refresh_changed_lines();
                None
            }
            InstanceMessage::FileStyleSummary {
                buffer_id,
                generation,
                lines,
            } => self.apply_file_style_summary(buffer_id, generation, lines),
            // Q#S1 (protocol v8; `message` since v15) — the
            // wire-authoritative half of the status band: name,
            // modified, whole-file diag counts, and the transient
            // status message (LSP command summaries).
            InstanceMessage::StatusFacts {
                buffer_id,
                name,
                modified,
                diag_errors,
                diag_warnings,
                message,
            } => {
                self.status_facts = Some(StatusFactsLocal {
                    buffer_id,
                    name,
                    modified,
                    diag_errors,
                    diag_warnings,
                    message,
                });
                self.request_redraw();
                None
            }
            // UX gutter (protocol v14): the daemon owns the per-window
            // line-number mode; apply it to our local gutter state and
            // repaint on change.
            InstanceMessage::LineNumbers { mode, .. } => {
                if self.line_numbers != mode {
                    let caret_was_painted = self.caret_painted_in_code_clip();
                    self.line_numbers = mode;
                    return self.reflow_dynamic_code_geometry(caret_was_painted);
                }
                None
            }
            // Themes Q#TH7 (protocol v16): the daemon-resolved UI face
            // table — complete replacement each send. The status-band
            // shaping cache MUST be invalidated here (Q#TH8): the
            // E:/W: counter colors are baked into glyphon rich-text
            // attributes at compose time and `refresh_status_line`
            // skips re-shaping while the composed strings are
            // unchanged, so a diag-face change with constant counts
            // would keep stale colors indefinitely without this.
            InstanceMessage::ThemeFacts { faces } => {
                self.faces = faces.into_iter().map(|f| (f.name, f.style)).collect();
                self.status_runs = None;
                self.status_left_runs = None;
                self.request_redraw();
                None
            }
            // Q#SR5 / Q#RX6 — the live isearch prompt (protocol v10).
            // `query: None` clears the band (search ended); `Some` shows
            // `[Regex] I-search: <query> (n/m)` on the band's left side.
            // The matches themselves arrive as SearchMatch decorations
            // and the keys round-trip via the intercept gate, so this
            // handler only drives the prompt text.
            InstanceMessage::SearchPrompt {
                buffer_id,
                query,
                active,
                total,
                regex,
                invalid,
            } => {
                self.search_prompt = query.map(|q| SearchPromptLocal {
                    buffer_id,
                    query: q,
                    active,
                    total,
                    regex,
                    invalid,
                });
                self.request_redraw();
                None
            }
            // Session 9.3 — peer presence. The editing frontend's
            // cursor + selection drive the `CurrentLine` / `Selection`
            // washes for this read-only mirror (finding QB1). Store
            // per source frontend; a redraw recomputes the background
            // rects from `peer_presences`. We never receive our own
            // (daemon sender exclusion).
            InstanceMessage::PresenceUpdate {
                frontend_id,
                buffer_id,
                cursor,
                selection,
            } => {
                // Run with `PMACS_GPU_DEBUG_PRESENCE=1` to confirm peer
                // presence is arriving and routed to the right buffer.
                // A `buf != current` line means the peer is on a buffer
                // this mirror isn't displaying (no wash expected); no
                // line at all means the message isn't reaching us.
                if debug_presence() {
                    eprintln!(
                        "pmacs-gpu presence: fid={frontend_id:?} buf={buffer_id:?} \
                         current={:?} cursor={cursor} sel={selection:?}",
                        self.current_buffer_id
                    );
                }
                self.peer_presences.insert(
                    frontend_id,
                    PeerPresence {
                        buffer_id,
                        cursor,
                        selection,
                    },
                );
                self.request_redraw();
                None
            }
            // Session B1 — our own cursor. The daemon emits this per
            // tick for the replica; the caret + own-window decorations
            // follow it. Only meaningful once we send Key events that
            // move it.
            InstanceMessage::CursorByte {
                buffer_id,
                byte_pos,
            } => {
                if debug_input() {
                    eprintln!(
                        "pmacs-gpu cursor: buf={buffer_id:?} byte={byte_pos} \
                         current={:?} match={}",
                        self.current_buffer_id,
                        self.current_buffer_id == Some(buffer_id)
                    );
                }
                if let Some(floor) = self.optimistic_cursor_floor {
                    // With deletes in the optimistic set the predicted
                    // cursor is no longer monotonic, so only the EXACT
                    // predicted byte (or a cursor for another buffer)
                    // confirms; any other value is an in-flight frame
                    // from before our unconfirmed edits. The timeout
                    // hatch accepts daemon truth if confirmation never
                    // comes (op dropped, peer raced our cursor).
                    let confirmed = floor.buffer_id != buffer_id || byte_pos == floor.byte;
                    if confirmed || self.optimistic_floor_timed_out() {
                        self.optimistic_cursor_floor = None;
                        self.optimistic_floor_set_at = None;
                    } else {
                        if debug_input() {
                            eprintln!(
                                "pmacs-gpu cursor: ignored stale in-flight position \
                                 buf={buffer_id:?} byte={byte_pos} predicted={}",
                                floor.byte
                            );
                        }
                        return None;
                    }
                }
                let arrived = OwnCursor {
                    buffer_id,
                    byte: byte_pos,
                };
                let moved = self.own_cursor != Some(arrived);
                self.own_cursor = Some(arrived);
                self.cursor_fresh = self.current_buffer_id == Some(buffer_id);
                // Session S1 — keep the caret on screen (Q#S2). When the
                // cursor leaves the visible slice (arrows past an edge,
                // PageUp/Down), scroll to follow it, re-shape the new
                // slice, and re-declare the scoped Viewport so the
                // producer ships spans for what's now visible.
                //
                // Only when the cursor MOVED. The daemon attaches a
                // CursorByte to every frame it produces — including
                // the frames our own Viewport sends trigger — so an
                // unconditional follow snapped the viewport back to
                // a stationary cursor on every minimap jump / scrub
                // (and on any wheel scroll past the cursor's screen):
                // jump → Viewport → frame + re-announced CursorByte →
                // snap, in a loop. Scrolling away from a cursor that
                // isn't moving is the user's prerogative.
                //
                // The follow is visual-run aware (framing Q#F6): the
                // shared helper closes the old source-line-only hole
                // where a caret on a wrapped continuation run below
                // the band never scrolled into view.
                if moved {
                    self.ensure_caret_painted();
                    // Q#MS5: the caret is a suppression input now. A
                    // move that crosses a math-span boundary must
                    // re-chunk the affected lines even when no scroll
                    // or edit follows — the follow above only reshapes
                    // on scroll, and a retained line under the stale
                    // suppression state is the #120 edge.
                    self.refresh_math_suppression();
                    if let Some(vp) = self.viewport_send_if_changed(buffer_id) {
                        return Some(vp);
                    }
                }
                self.request_redraw();
                None
            }
            InstanceMessage::DispatchIdle { idle } => {
                self.dispatch_idle = idle;
                None
            }
            // Q#CM6 — a daemon copy/cut published the region; write it to
            // the OS clipboard via arboard so other apps can paste it.
            InstanceMessage::Signal(InstanceSignal::Clipboard(bytes)) => {
                self.write_os_clipboard(&bytes);
                None
            }
            // Q#CM1 — the context menu's rows + highlight. Empty rows
            // close it; otherwise anchor the popup at the remembered
            // right-click pixel.
            InstanceMessage::MenuPrompt { rows, active, .. } => {
                self.menu = if rows.is_empty() {
                    None
                } else {
                    Some(MenuLocal {
                        rows,
                        active,
                        anchor_px: self.menu_anchor_px,
                    })
                };
                self.request_redraw();
                None
            }
            // Q#MB1 — the minibuffer prompt/input/candidates. `prompt:
            // None` closes it. This is the FROZEN legacy variant, which
            // only a `12..=22` daemon sends; its candidates carry no
            // detail, so they become rows with `detail: None` and render
            // exactly as they did before v23.
            InstanceMessage::MinibufferPrompt {
                prompt,
                input,
                cursor,
                candidates,
                selected,
                total,
            } => {
                self.minibuffer = prompt.map(|prompt| MinibufferLocal {
                    prompt,
                    input,
                    cursor,
                    rows: candidates
                        .into_iter()
                        .map(|label| MinibufferRow {
                            label,
                            detail: None,
                        })
                        .collect(),
                    selected,
                    total,
                });
                self.request_redraw();
                None
            }
            // Discovery Stage 2 — the v23 rows form of the same surface,
            // carrying an optional per-row detail (a command's
            // description). `prompt: None` closes it, and the close
            // arrives in THIS family because the daemon picks the family
            // per peer: a rows session closed by a legacy clear would
            // leave the dropdown on screen forever.
            InstanceMessage::MinibufferPromptRows {
                prompt,
                input,
                cursor,
                rows,
                selected,
                total,
            } => {
                self.minibuffer = prompt.map(|prompt| MinibufferLocal {
                    prompt,
                    input,
                    cursor,
                    rows,
                    selected,
                    total,
                });
                self.request_redraw();
                None
            }
            // Arc 1a Q#C5/Q#C6 — the in-buffer completion dropdown.
            // A close (`anchor: None`) always applies — the daemon may
            // ship it carrying a buffer this window just switched away
            // from, and dropping it would wedge a stale popup. An OPEN
            // for a buffer this window isn't showing is dropped (the
            // CrdtOp rule).
            InstanceMessage::CompletionPopup {
                buffer_id,
                anchor,
                prefix_len,
                rows,
                selected,
                total,
            } => {
                let Some(anchor) = anchor else {
                    self.completion = None;
                    self.request_redraw();
                    return None;
                };
                if self.current_buffer_id != Some(buffer_id) {
                    return None;
                }
                self.completion = Some(CompletionLocal {
                    buffer_id,
                    anchor,
                    prefix_len,
                    rows,
                    selected,
                    total,
                });
                self.request_redraw();
                None
            }
            // Arc 4 stage 3 (Q#SL7/Q#SL10) — validate the entire
            // untrusted replacement before changing either side.
            InstanceMessage::StatuslineSegments {
                buffer_id,
                left,
                right,
            } => {
                if let Err(reason) = validate_statusline_segments(&left, &right) {
                    eprintln!("pmacs-gpu: ignoring invalid StatuslineSegments: {reason}");
                    return None;
                }
                let next = StatuslineSegmentsLocal {
                    buffer_id,
                    left,
                    right,
                };
                if self.statusline_segments.as_ref() != Some(&next) {
                    self.statusline_segments = Some(next);
                    self.status_runs = None;
                    self.status_left_runs = None;
                    self.request_redraw();
                }
                None
            }
            // Arc 4 stage 2 (framing Q#F6/Q#F7) — the global font
            // preference. Authoritative per attachment: `(None, None)`
            // is a real reset to the sanitized defaults, never
            // inferred from silence. The rebuild may change the
            // visible slice (new wrapping/metrics), so re-declare the
            // viewport from the final normalized origin.
            InstanceMessage::FontFacts {
                family,
                size_centi_px,
            } => {
                self.apply_font_facts(family.as_deref(), size_centi_px);
                // Q#F6 + Vterm Stage 3: new metrics mean a new cell
                // grid. The terminal shape/geometry caches are dropped
                // here and stay dropped until an authoritative frame at
                // the matching size arrives, so nothing paints at the
                // old advance under the new font.
                self.invalidate_terminal_shaping();
                // Q#BP2S1 / A2B-2: the panel's shaped runs were measured at
                // the old advance, and the CELL total may be identical while
                // the pixels behind it are not. The reshape is here; the new
                // declaration is the caller's, because only it can send.
                self.rebuild_panel_text_buffers();
                self.panel_metrics_changed = true;
                self.current_buffer_id
                    .and_then(|bid| self.viewport_send_if_changed(bid))
            }
            InstanceMessage::TerminalFrame(frame) => {
                self.apply_terminal_frame(frame);
                None
            }
            InstanceMessage::LineWrapFacts { buffer_id, wrap } => {
                self.apply_line_wrap(buffer_id, wrap);
                None
            }
            InstanceMessage::PanelFrame(payload) => {
                // The band changes the DOCUMENT's pixel height, so a panel
                // that appears or disappears has to reshape the document
                // buffers as well as request a repaint. Skipping the
                // reshape leaves the code layer sized to the old boundary
                // and the last lines painting under the band.
                if self.apply_panel_payload(payload) {
                    self.sync_buffer_dimensions();
                    self.request_redraw();
                }
                None
            }
            _ => None,
        }
    }

    /// Install a decoded terminal frame, or reject it whole.
    ///
    /// Rejection is total by design: a partially applied frame would mix
    /// cells from two screens. An invalid frame therefore keeps the
    /// previous valid one, requests no redraw, and reports one latched
    /// diagnostic instead of painting something the daemon never
    /// authorized.
    fn apply_terminal_frame(&mut self, frame: TerminalFrame) {
        if self.current_buffer_id != Some(frame.buffer_id) {
            // A frame for a buffer this window is no longer showing.
            // The daemon clears its baseline on every snapshot, so the
            // authoritative frame for the buffer we DO show is already
            // on its way.
            return;
        }
        if let Err(error) = frame.validate() {
            if !self.terminal_frame_error_latched {
                self.terminal_frame_error_latched = true;
                eprintln!("pmacs-gpu: rejecting invalid TerminalFrame: {error}");
            }
            return;
        }
        self.terminal_frame_error_latched = false;
        if self
            .terminal
            .as_ref()
            .is_some_and(|terminal| terminal.frame == frame)
        {
            // A duplicate valid frame does no work at all: no plan
            // rebuild, no reshape, no redraw.
            return;
        }
        let plan = TerminalPaintPlan::build(&frame, Self::terminal_palette());
        let buffer_id = frame.buffer_id;
        self.terminal = Some(TerminalLocal {
            buffer_id,
            frame,
            plan,
        });
        self.rebuild_terminal_text_buffers();
        self.request_redraw();
    }

    /// The frontend defaults `Color::Default` resolves against.
    fn terminal_palette() -> TerminalPalette {
        let fg = plain_text_color();
        TerminalPalette {
            default_fg: [fg.r(), fg.g(), fg.b()],
            default_bg: [
                (WINDOW_BG_RGBA[0] * 255.0) as u8,
                (WINDOW_BG_RGBA[1] * 255.0) as u8,
                (WINDOW_BG_RGBA[2] * 255.0) as u8,
            ],
        }
    }

    /// Show a disconnect notice, leaving terminal mode first.
    ///
    /// Terminal mode prepares NO document code layer, and the terminal
    /// glyph layer keeps painting the last frame it was given. Setting
    /// the text without leaving terminal mode therefore writes into a
    /// layer nothing draws, and the user is left looking at a frozen,
    /// live-looking terminal that silently ignores input — with GPU
    /// auto-reconnect a named deferral, until relaunch. The F-008
    /// "make the teardown visible" contract applies to terminal mode
    /// too.
    fn on_daemon_disconnected(&mut self, notice: &str) {
        self.exit_terminal_mode();
        // A retained band is the daemon's projection of a window that is no
        // longer being updated. Leaving it on screen beside a disconnect
        // notice is the same "frozen, live-looking surface" the terminal arm
        // above exists to prevent.
        self.exit_panel_band();
        if !self.set_text(notice) {
            // Byte-identical text still needs a repaint: the frame that
            // is on screen is the terminal's, not this notice.
            self.reshape();
        }
        self.request_redraw();
    }

    /// Leave terminal mode and drop every terminal-only cache.
    fn exit_terminal_mode(&mut self) {
        self.terminal = None;
        self.terminal_text_buffers.clear();
        self.terminal_frame_error_latched = false;
        self.last_terminal_size_sent = None;
        self.last_terminal_pointer_cell = None;
    }

    /// Drop the band and every cache behind it.
    ///
    /// The geometry declaration goes too: the next session is a new session,
    /// its epochs start from scratch, and a retained declaration would
    /// describe a peer that is gone.
    fn exit_panel_band(&mut self) {
        self.panel = PanelBand::default();
        self.panel_text_buffers.clear();
    }

    /// Drop shaping and geometry caches without leaving terminal mode.
    ///
    /// Used when the font changes: the installed frame is still the
    /// child's authoritative screen, but every cached shape was measured
    /// at the old advance.
    fn invalidate_terminal_shaping(&mut self) {
        if self.terminal.is_some() {
            self.rebuild_terminal_text_buffers();
        }
        self.last_terminal_size_sent = None;
    }

    /// Reshape one cosmic-text buffer per planned run.
    ///
    /// One buffer per RUN, not per row: a row-wide buffer would let a
    /// wide or cluster glyph's shaped advance decide where the following
    /// column starts, and terminal columns belong to the child.
    fn rebuild_terminal_text_buffers(&mut self) {
        let Some(terminal) = self.terminal.as_ref() else {
            self.terminal_text_buffers.clear();
            return;
        };
        let metrics = Metrics::new(self.fm.code_font_size(), self.fm.code_line_height());
        let advance = self.mono_advance();
        let family = self.resolved_family.clone();
        let runs: Vec<_> = terminal
            .plan
            .runs
            .iter()
            .map(|run| {
                (
                    run.text.clone(),
                    run.cells as f32 * advance,
                    run.bold,
                    run.italic,
                )
            })
            .collect();
        let mut buffers = Vec::with_capacity(runs.len());
        for (text, width, bold, italic) in runs {
            let mut buffer = Buffer::new(&mut self.font_system, metrics);
            // No wrapping: a run occupies exactly the cells the child
            // gave it, and overflow is a clip, never a second row.
            buffer.set_wrap(&mut self.font_system, Wrap::None);
            buffer.set_size(
                &mut self.font_system,
                Some(width.max(1.0)),
                Some(metrics.line_height),
            );
            let attrs = Attrs::new()
                .family(Family::Name(&family))
                .weight(if bold {
                    glyphon::cosmic_text::Weight::BOLD
                } else {
                    glyphon::cosmic_text::Weight::NORMAL
                })
                .style(if italic {
                    glyphon::cosmic_text::Style::Italic
                } else {
                    glyphon::cosmic_text::Style::Normal
                });
            buffer.set_text(
                &mut self.font_system,
                &text,
                &attrs,
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(&mut self.font_system, false);
            buffers.push(buffer);
        }
        self.terminal_text_buffers = buffers;
    }

    /// The terminal content rectangle's pixel origin.
    ///
    /// Deliberately not `text_left()`: terminal mode draws no document
    /// gutter, so the grid starts at the plain text inset.
    fn terminal_origin() -> (f32, f32) {
        (TEXT_LEFT, TEXT_TOP)
    }

    /// The cell grid this window's drawable rectangle admits, or `None`
    /// when it cannot fit one whole cell.
    fn terminal_cell_viewport(&self) -> Option<CellSize> {
        let (origin_x, origin_y) = Self::terminal_origin();
        let width = self.config.width as f32 - origin_x;
        let height =
            document_text_bottom(self.config.height, self.fm, self.band_inset()) - origin_y;
        crate::terminal::cell_viewport(
            width,
            height,
            self.mono_advance(),
            self.fm.code_line_height(),
        )
    }

    // -----------------------------------------------------------------
    // Bottom panel band (Stage 2B-3)
    // -----------------------------------------------------------------

    /// Whether this session negotiated the panel wire.
    ///
    /// Set once from the negotiated session version, never from the
    /// `Hello` baseline: the baseline stays at the compatibility floor
    /// forever, so reading it here would leave the band permanently dark.
    fn set_panel_wire(&mut self, session_protocol_version: u32) {
        self.panel_wire = session_protocol_version >= PANEL_MIN_VERSION;
    }

    /// The band inset the document boundary is computed from.
    ///
    /// Routed through [`PanelBand::presented`] rather than re-deriving
    /// "is a panel visible" here, because a second derivation of that
    /// predicate is how the renderer and the retained state drift apart.
    fn band_inset(&self) -> PanelBandInset {
        self.panel
            .presented()
            .map_or(PanelBandInset::ABSENT, |frame| {
                PanelBandInset::installed(frame.size.rows, self.fm)
            })
    }

    /// The panel band's content rectangle in surface pixels:
    /// `(x, y, width, height)`, cells only — the divider sits above `y`.
    fn panel_content_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let frame = self.panel.presented()?;
        let band = PanelBandInset::installed(frame.size.rows, self.fm);
        let cells_px = band.px() - self.fm.divider_height();
        if cells_px <= 0.0 {
            return None;
        }
        let top =
            document_text_bottom(self.config.height, self.fm, band) + self.fm.divider_height();
        // Origin x = 0 and the FULL surface width, matching the declaration.
        // Any fractional right-edge remainder past the last whole column is
        // band background: it maps to no cell and emits no `PanelPointer`,
        // which `hit_test_cell`'s column bound already enforces.
        Some((0.0, top, self.config.width as f32, cells_px))
    }

    /// The divider strip: paint geometry AND hit geometry, one rect.
    ///
    /// Deliberately the same value for both. The framing decided a 4 px
    /// strip precisely so the pointer has a usable target, and deriving
    /// the hover band separately from the painted rule is how the two come
    /// to disagree by a pixel that the user can see but not grab.
    fn panel_divider_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let frame = self.panel.presented()?;
        let band = PanelBandInset::installed(frame.size.rows, self.fm);
        Some((
            0.0,
            document_text_bottom(self.config.height, self.fm, band),
            self.config.width as f32,
            self.fm.divider_height(),
        ))
    }

    /// The stable normal-face advance the geometry declaration uses
    /// (framing §5.3, A2B-3).
    ///
    /// **Never [`Self::mono_advance`].** That falls back to the first
    /// shaped glyph of the *document* buffer when no `FontFacts` probe has
    /// been applied, which would make the panel's column count
    /// document-dependent: two frontends with identical metrics showing
    /// different files would derive different `total.cols`, and the same
    /// frontend's panel width would change when its first glyph did.
    ///
    /// `None` when the family shapes no width — the caller declares zero
    /// usable geometry rather than reaching for a document sample.
    fn panel_probe_advance(&mut self) -> Option<f32> {
        let metrics = Metrics::new(self.fm.code_font_size(), self.fm.code_line_height());
        let family = self.resolved_family.clone();
        probe_mono_advance(&mut self.font_system, &family, metrics)
    }

    /// This surface's whole-cell capacity as the daemon's layout model
    /// sees it (Q#BP15a's pixel→cell conversion).
    ///
    /// Zero-sized on any degenerate input, which is the fail-closed arm
    /// parent 41 requires: the daemon treats zero columns as
    /// non-presentable and the panel hides, rather than a non-finite
    /// metric producing an absurd row count and an oversized allocation.
    fn declared_cell_total(&mut self) -> (CellSize, Option<f32>) {
        let Some(advance) = self.panel_probe_advance() else {
            return (CellSize::new(0, 0), None);
        };
        let height = (geometry_capacity_bottom(self.config.height, self.fm) - TEXT_TOP).max(0.0);
        // **Full surface width from x = 0.** The panel grid is not inset by
        // the document's `TEXT_LEFT` or gutter — those are document padding,
        // and the band is a separate surface spanning the frame (parent
        // framing Q#BP15a: "`total.cols` describes the full-width panel grid
        // beginning at x=0; document `TEXT_LEFT`/gutter padding is
        // unrelated"). Deducting `TEXT_LEFT` here under-declares columns and
        // leaves a strip the daemon never fills.
        let width = self.config.width as f32;
        let total = crate::terminal::panel_cell_capacity(
            width,
            height,
            advance,
            self.fm.code_line_height(),
        )
        .unwrap_or_else(|| CellSize::new(0, 0));
        (total, Some(advance))
    }

    /// The advance every panel cell computation must use: the one behind the
    /// current declaration.
    ///
    /// `None` before a declaration exists — which is also when
    /// [`PanelBand::presented`] is `None`, so no painter or hit test can be
    /// reached without it.
    fn panel_cell_advance(&self) -> Option<f32> {
        self.panel
            .declared_advance
            .filter(|advance| advance.is_finite() && *advance > 0.0)
    }

    /// Advance the geometry declaration if this trigger calls for one, and
    /// return what the caller must send.
    ///
    /// The decision lives here and the *send* lives at the seam that owns
    /// the attach client, so the whole state machine — dedup, exhaustion,
    /// the latch — is reachable without a daemon.
    fn next_geometry_declaration(&mut self, trigger: GeometryTrigger) -> Option<(u64, CellSize)> {
        if !self.panel_wire || self.panel.exhausted {
            return None;
        }
        let (total, advance) = self.declared_cell_total();
        if trigger == GeometryTrigger::Surface
            && self.panel.geometry_epoch != 0
            && self.panel.declared == Some(total)
        {
            return None;
        }
        let Some(next) = self.panel.geometry_epoch.checked_add(1) else {
            // Fail closed, and LATCH. Retaining the last declaration is not
            // fail-closed: if the surface then resizes, the daemon would
            // keep painting a panel sized to a frame that no longer exists.
            // Dropping the retained frame alone is not enough either — an
            // old `Present` whose epoch still matched would resurrect a
            // band under geometry this frontend has disowned.
            self.panel.exhausted = true;
            self.panel.frame = None;
            self.panel.plan = None;
            self.panel.drag = None;
            self.panel.hover_divider = false;
            self.panel.declared_advance = None;
            return None;
        };
        self.panel.geometry_epoch = next;
        self.panel.declared = Some(total);
        self.panel.declared_advance = advance;
        Some((next, total))
    }

    /// Apply an inbound `PanelFrame` payload.
    ///
    /// Returns `true` when the band's appearance changed, so the caller
    /// can request a redraw without guessing.
    ///
    /// Validation is atomic: a rejected frame leaves the retained one
    /// exactly as it was, because `PanelFrame::validate` is pure and runs
    /// before any state is touched.
    fn apply_panel_payload(&mut self, payload: PanelFramePayload) -> bool {
        if self.panel.exhausted {
            // A latched session has disowned its geometry for good, so a
            // payload answering it describes nothing this frontend can
            // present. Retaining the frame anyway would leave `presented()`
            // as the only thing standing between a disowned declaration and
            // a painted band, and would spend a reshape on every arriving
            // frame for the rest of the session.
            return false;
        }
        match payload {
            PanelFramePayload::Absent => {
                // Authoritative removal, and always safe. Note this does
                // NOT clear the geometry declaration: the frontend's frame
                // capacity is unchanged by a panel closing, and discarding
                // it would force a needless re-declaration before the next
                // open.
                let had = self.panel.presented().is_some();
                self.panel.frame = None;
                self.panel.plan = None;
                self.panel.drag = None;
                self.panel.hover_divider = false;
                self.panel.pointer_held = false;
                self.panel.last_pointer_cell = None;
                had
            }
            PanelFramePayload::Present(frame) => {
                if let Err(error) = frame.validate() {
                    eprintln!("pmacs-gpu: rejecting invalid panel frame: {error}");
                    return false;
                }
                if self.panel.frame.as_ref() == Some(&frame) {
                    // A duplicate does no work — not even a reshape.
                    return false;
                }
                let plan = TerminalPaintPlan::build_grid(
                    frame.size,
                    &frame.cells,
                    frame.cursor,
                    Self::terminal_palette(),
                );
                self.panel.frame = Some(frame);
                self.panel.plan = Some(plan);
                self.rebuild_panel_text_buffers();
                true
            }
        }
    }

    /// Reshape one cosmic-text buffer per planned panel run.
    ///
    /// One buffer per RUN for the same reason terminal mode does it: a
    /// row-wide buffer would let a wide or cluster glyph's shaped advance
    /// decide where the next column starts, and panel columns belong to
    /// the daemon's grid, not to the shaper.
    fn rebuild_panel_text_buffers(&mut self) {
        let Some(plan) = self.panel.plan.as_ref() else {
            self.panel_text_buffers.clear();
            return;
        };
        let metrics = Metrics::new(self.fm.code_font_size(), self.fm.code_line_height());
        // The declaration's advance, never the document's: a run shaped to a
        // different cell width than the daemon counted columns with drifts
        // one column further off across the row.
        let Some(advance) = self.panel_cell_advance() else {
            self.panel_text_buffers.clear();
            return;
        };
        let family = self.resolved_family.clone();
        let runs: Vec<(String, f32, bool, bool)> = plan
            .runs
            .iter()
            .map(|run| {
                (
                    run.text.clone(),
                    run.cells as f32 * advance,
                    run.bold,
                    run.italic,
                )
            })
            .collect();
        let mut buffers = Vec::with_capacity(runs.len());
        for (text, width, bold, italic) in runs {
            let mut buffer = Buffer::new(&mut self.font_system, metrics);
            buffer.set_wrap(&mut self.font_system, Wrap::None);
            buffer.set_size(
                &mut self.font_system,
                Some(width.max(1.0)),
                Some(metrics.line_height),
            );
            let attrs = Attrs::new()
                .family(Family::Name(&family))
                .weight(if bold {
                    glyphon::cosmic_text::Weight::BOLD
                } else {
                    glyphon::cosmic_text::Weight::NORMAL
                })
                .style(if italic {
                    glyphon::cosmic_text::Style::Italic
                } else {
                    glyphon::cosmic_text::Style::Normal
                });
            buffer.set_text(
                &mut self.font_system,
                &text,
                &attrs,
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(&mut self.font_system, false);
            buffers.push(buffer);
        }
        self.panel_text_buffers = buffers;
    }

    /// Pixel rectangle of a cell run inside the panel band.
    fn panel_run_rect(&self, run: crate::terminal::CellRun) -> Option<(f32, f32, f32, f32)> {
        let (ox, oy, _, _) = self.panel_content_rect()?;
        let advance = self.panel_cell_advance()?;
        let line = self.fm.code_line_height();
        Some((
            ox + run.start_col as f32 * advance,
            oy + run.row as f32 * line,
            (run.end_col - run.start_col) as f32 * advance,
            line,
        ))
    }

    /// The band's quad batch: the divider strip, cell backgrounds, and the
    /// panel caret, drawn under the band's glyphs.
    #[allow(
        clippy::too_many_lines,
        reason = "one band's complete quad batch: divider, cell backgrounds, every straight underline form, caret"
    )]
    fn panel_quad_vertex_bytes(&self) -> Vec<u8> {
        let mut rects = Vec::new();
        if let Some((x, y, w, h)) = self.panel_divider_rect() {
            rects.push(MinimapRect {
                x,
                y,
                w,
                h,
                color: self.face_wash_or("ui.divider", DIVIDER_RGBA),
            });
        }
        if let Some(plan) = self.panel.plan.as_ref()
            && self.panel.presented().is_some()
        {
            let window_bg = Self::terminal_palette().default_bg;
            for bg in &plan.backgrounds {
                if bg.color == window_bg {
                    continue;
                }
                if let Some((x, y, w, h)) = self.panel_run_rect(bg.run) {
                    rects.push(MinimapRect {
                        x,
                        y,
                        w,
                        h,
                        color: rgb_to_quad(bg.color, 1.0),
                    });
                }
            }
            // Only a FOCUSED panel paints its caret. The producer includes
            // `cursor` for a passive panel too — it is the window's real
            // point, and the daemon does not suppress it — so painting it
            // unconditionally puts a second insertion caret on screen and
            // makes focus ownership visually ambiguous. `focused` is exactly
            // the presentation bit Q#BP14b reserves for this.
            // Straight underline forms as fixed-cell quads, exactly as the
            // terminal path does; curly rides the squiggle pipeline below,
            // which owns the sine wave. Dropping these silently loses every
            // diagnostic and styled-terminal underline inside the band.
            for underline in &plan.underlines {
                if underline.style == UnderlineStyle::Curly {
                    continue;
                }
                let Some((x, y, w, h)) = self.panel_run_rect(underline.run) else {
                    continue;
                };
                let color = rgb_to_quad(underline.color, 1.0);
                let thickness = TERMINAL_UNDERLINE_PX;
                let baseline = y + h - thickness * 2.0;
                match underline.style {
                    UnderlineStyle::Double => {
                        rects.push(MinimapRect {
                            x,
                            y: baseline,
                            w,
                            h: thickness,
                            color,
                        });
                        rects.push(MinimapRect {
                            x,
                            y: baseline + thickness * 2.0,
                            w,
                            h: thickness,
                            color,
                        });
                    }
                    UnderlineStyle::Dotted | UnderlineStyle::Dashed => {
                        let period = if underline.style == UnderlineStyle::Dotted {
                            TERMINAL_UNDERLINE_PX * 3.0
                        } else {
                            TERMINAL_UNDERLINE_PX * 8.0
                        };
                        let duty = if underline.style == UnderlineStyle::Dotted {
                            0.5
                        } else {
                            0.625
                        };
                        let mut dash_x = x;
                        while dash_x < x + w {
                            let dash_w = (period * duty).min(x + w - dash_x);
                            rects.push(MinimapRect {
                                x: dash_x,
                                y: baseline,
                                w: dash_w,
                                h: thickness,
                                color,
                            });
                            dash_x += period;
                        }
                    }
                    UnderlineStyle::Single => rects.push(MinimapRect {
                        x,
                        y: baseline,
                        w,
                        h: thickness,
                        color,
                    }),
                    UnderlineStyle::Curly | UnderlineStyle::None => {}
                }
            }
            if let Some(cursor) = plan.cursor
                && self.panel.presented().is_some_and(|frame| frame.focused)
                && let Some((x, y, w, h)) = self.panel_run_rect(cursor)
            {
                rects.push(MinimapRect {
                    x,
                    y,
                    w,
                    h,
                    color: TERMINAL_CURSOR_RGBA,
                });
            }
        }
        if rects.is_empty() {
            return Vec::new();
        }
        rects_to_vertex_bytes(&rects, self.config.width, self.config.height)
    }

    /// Curly underlines inside the band, on the squiggle pipeline — the same
    /// split the terminal path makes, because the sine wave belongs to that
    /// pipeline and a quad cannot express it.
    fn panel_squiggle_vertex_bytes(&self) -> Vec<u8> {
        let Some(plan) = self.panel.plan.as_ref() else {
            return Vec::new();
        };
        if self.panel.presented().is_none() {
            return Vec::new();
        }
        let rects: Vec<MinimapRect> = plan
            .underlines
            .iter()
            .filter(|underline| underline.style == UnderlineStyle::Curly)
            .filter_map(|underline| {
                let (x, y, w, h) = self.panel_run_rect(underline.run)?;
                Some(MinimapRect {
                    x,
                    y: y + h - DIAG_SQUIGGLE_PX,
                    w,
                    h: DIAG_SQUIGGLE_PX,
                    color: rgb_to_quad(underline.color, 1.0),
                })
            })
            .collect();
        squiggles_to_vertex_bytes(&rects, self.config.width, self.config.height)
    }

    /// Which panel cell a surface pixel is over, if any (Q#BP16).
    ///
    /// Returns `None` outside the band's content rect, which is what keeps
    /// a document gesture from being reported as a panel one.
    fn panel_hit_test(&self, x: f32, y: f32) -> Option<CellCoord> {
        let frame = self.panel.presented()?;
        let (ox, oy, w, h) = self.panel_content_rect()?;
        if x < ox || x >= ox + w || y < oy || y >= oy + h {
            return None;
        }
        crate::terminal::hit_test_cell(
            x,
            y,
            (ox, oy),
            self.panel_cell_advance()?,
            self.fm.code_line_height(),
            frame.size,
        )
    }

    /// Classify a pointer pixel against the band.
    ///
    /// The divider is tested before the cells because it sits directly above
    /// them and a gesture on the strip is a resize, not a selection.
    fn classify_pointer_surface(&self, x: f32, y: f32) -> PointerSurface {
        if self.panel_divider_contains(x, y) {
            return PointerSurface::PanelDivider;
        }
        let Some((ox, oy, w, h)) = self.panel_content_rect() else {
            return PointerSurface::Elsewhere;
        };
        if x < ox || x >= ox + w || y < oy || y >= oy + h {
            return PointerSurface::Elsewhere;
        }
        match self.panel_hit_test(x, y) {
            Some(coord) => PointerSurface::PanelCell(coord),
            None => PointerSurface::PanelBackground,
        }
    }

    /// The gesture kind a panel motion carries: a held left button makes it a
    /// drag, and that distinction is the whole of panel selection — `Move`
    /// never focuses or claims, while every non-`Move` gesture activates the
    /// panel first.
    fn panel_motion_kind(&self) -> ProtocolMouseKind {
        if self.panel.pointer_held {
            ProtocolMouseKind::Drag(ProtocolMouseButton::Left)
        } else {
            ProtocolMouseKind::Move
        }
    }

    /// The cell a release belongs to: the one under the pointer while it is
    /// still in the band, else the last cell the gesture reported.
    ///
    /// A panel selection drag routinely ends past the band's edge, and
    /// dropping that release leaves the daemon holding a button down forever.
    fn panel_release_cell(&self, x: f32, y: f32) -> Option<CellCoord> {
        match self.classify_pointer_surface(x, y) {
            PointerSurface::PanelCell(coord) => Some(coord),
            _ => self.panel.last_pointer_cell,
        }
    }

    /// Whether a panel motion at `coord` carries anything new, and latch it.
    ///
    /// Sub-cell motion resolves to the same cell and says nothing the daemon
    /// can act on. Without this, pixel-rate motion becomes pixel-rate wire
    /// traffic and every one of those is a daemon-side gesture — the same
    /// reason the terminal path dedupes.
    fn panel_motion_is_new(&mut self, coord: CellCoord) -> bool {
        if self.panel.last_pointer_cell == Some(coord) {
            return false;
        }
        self.panel.last_pointer_cell = Some(coord);
        true
    }

    /// Arm or disarm the panel's left-button gesture, re-arming the motion
    /// dedupe either way.
    fn set_panel_pointer_held(&mut self, held: bool) {
        self.panel.pointer_held = held;
        self.panel.last_pointer_cell = None;
    }

    /// Begin a divider drag at surface pixel `y`, if the pointer is on the
    /// strip. Returns whether a drag was started.
    fn begin_panel_drag(&mut self, x: f32, y: f32) -> bool {
        if !self.panel_divider_contains(x, y) {
            return false;
        }
        let Some(frame) = self.panel.presented() else {
            return false;
        };
        self.panel.drag = Some(PanelDrag {
            panel_epoch: frame.panel_epoch,
            geometry_epoch: frame.geometry_epoch,
            start_rows: frame.size.rows,
            start_y: y,
            sent_rows: frame.size.rows,
        });
        true
    }

    /// End any live divider drag.
    fn end_panel_drag(&mut self) -> bool {
        self.panel.drag.take().is_some()
    }

    /// The row request a live drag at pixel `y` implies, or `None` when
    /// there is no drag, no presented panel, or the request is unchanged.
    ///
    /// The drag's own epochs are checked against the panel that is on
    /// screen NOW: a gesture that outlives its presentation is dropped
    /// rather than applied to the successor, which is the same rule the
    /// daemon enforces on receipt. Both sides check because neither may
    /// depend on the other having done it.
    fn panel_drag_request(&mut self, y: f32) -> Option<PanelResizeRequest> {
        let drag = self.panel.drag?;
        let frame = self.panel.presented()?;
        if frame.panel_epoch != drag.panel_epoch || frame.geometry_epoch != drag.geometry_epoch {
            self.panel.drag = None;
            return None;
        }
        let line = self.fm.code_line_height();
        if !line.is_finite() || line <= 0.0 {
            return None;
        }
        // Dragging the divider UP grows the panel, so a negative pixel
        // delta is a positive row delta.
        let delta_rows = ((drag.start_y - y) / line).round();
        if !delta_rows.is_finite() {
            return None;
        }
        let rows = (i64::from(drag.start_rows) + delta_rows as i64).max(1);
        let rows = u32::try_from(rows).unwrap_or(u32::MAX);
        if rows == drag.sent_rows {
            return None;
        }
        Some(PanelResizeRequest {
            geometry_epoch: drag.geometry_epoch,
            panel_epoch: drag.panel_epoch,
            rows,
        })
    }

    /// Record that a row request was actually sent, so re-crossing the same
    /// row boundary does not re-send it.
    fn note_panel_drag_sent(&mut self, rows: u32) {
        if let Some(drag) = self.panel.drag.as_mut() {
            drag.sent_rows = rows;
        }
    }

    /// Apply the divider hover cursor icon to the real window.
    ///
    /// `RowResize` while the pointer is on the strip, the default arrow
    /// otherwise. Driven from the same `hover_divider` bit the hit test
    /// sets, so the icon cannot advertise a drag target the press would
    /// miss.
    fn apply_panel_cursor_icon(&self) {
        if let Some(window) = &self.window {
            window.set_cursor(if self.panel.hover_divider {
                winit::window::CursorIcon::RowResize
            } else {
                winit::window::CursorIcon::Default
            });
        }
    }

    /// Consume the "a font/scale change invalidated the declaration" flag.
    fn take_panel_metrics_changed(&mut self) -> bool {
        std::mem::take(&mut self.panel_metrics_changed)
    }

    /// Update divider hover, reporting whether the cursor icon must change.
    fn set_panel_divider_hover(&mut self, hovering: bool) -> bool {
        if self.panel.hover_divider == hovering {
            return false;
        }
        self.panel.hover_divider = hovering;
        true
    }

    /// Whether a surface pixel is on the divider strip — the exact rect
    /// that gets painted.
    fn panel_divider_contains(&self, x: f32, y: f32) -> bool {
        self.panel_divider_rect()
            .is_some_and(|(rx, ry, rw, rh)| x >= rx && x < rx + rw && y >= ry && y < ry + rh)
    }

    /// Pixel rectangle of a cell run in the terminal grid.
    fn terminal_run_rect(&self, run: crate::terminal::CellRun) -> (f32, f32, f32, f32) {
        let (ox, oy) = Self::terminal_origin();
        let advance = self.mono_advance();
        let line = self.fm.code_line_height();
        (
            ox + run.start_col as f32 * advance,
            oy + run.row as f32 * line,
            (run.end_col - run.start_col) as f32 * advance,
            line,
        )
    }

    /// Backgrounds, straight underlines, the selection wash, and the
    /// terminal clip, as one quad batch drawn under the glyphs.
    ///
    /// Runs whose resolved background equals the window clear color are
    /// dropped: the clear already painted them, and emitting a
    /// full-screen quad per frame for the common case is pure waste.
    fn terminal_quad_vertex_bytes(&self) -> Vec<u8> {
        let Some(terminal) = self.terminal.as_ref() else {
            return Vec::new();
        };
        let window_bg = Self::terminal_palette().default_bg;
        let mut rects = Vec::new();
        for bg in &terminal.plan.backgrounds {
            if bg.color == window_bg {
                continue;
            }
            let (x, y, w, h) = self.terminal_run_rect(bg.run);
            rects.push(MinimapRect {
                x,
                y,
                w,
                h,
                color: rgb_to_quad(bg.color, 1.0),
            });
        }
        // Straight underline forms as fixed-cell quads; curly rides the
        // squiggle pipeline, which owns the sine wave.
        for underline in &terminal.plan.underlines {
            if underline.style == UnderlineStyle::Curly {
                continue;
            }
            let (x, y, w, h) = self.terminal_run_rect(underline.run);
            let color = rgb_to_quad(underline.color, 1.0);
            let thickness = TERMINAL_UNDERLINE_PX;
            let baseline = y + h - thickness * 2.0;
            match underline.style {
                UnderlineStyle::Double => {
                    rects.push(MinimapRect {
                        x,
                        y: baseline,
                        w,
                        h: thickness,
                        color,
                    });
                    rects.push(MinimapRect {
                        x,
                        y: baseline + thickness * 2.0,
                        w,
                        h: thickness,
                        color,
                    });
                }
                UnderlineStyle::Dotted | UnderlineStyle::Dashed => {
                    // Dotted and dashed differ only in duty cycle; both
                    // are stepped along the run so a one-cell run still
                    // shows at least one mark.
                    let period = if underline.style == UnderlineStyle::Dotted {
                        TERMINAL_UNDERLINE_PX * 3.0
                    } else {
                        TERMINAL_UNDERLINE_PX * 8.0
                    };
                    let duty = if underline.style == UnderlineStyle::Dotted {
                        0.5
                    } else {
                        0.625
                    };
                    let mut at = x;
                    while at < x + w {
                        let seg = (period * duty).min(x + w - at);
                        rects.push(MinimapRect {
                            x: at,
                            y: baseline,
                            w: seg,
                            h: thickness,
                            color,
                        });
                        at += period;
                    }
                }
                _ => rects.push(MinimapRect {
                    x,
                    y: baseline,
                    w,
                    h: thickness,
                    color,
                }),
            }
        }
        // Terminal selection is the editor's, not the child's: it draws
        // as a separate wash through the existing `ui.selection` site
        // and never rewrites a cell's own style.
        let selection_color = self.face_wash_or("ui.selection", TERMINAL_SELECTION_RGBA);
        for run in &terminal.plan.selection {
            let (x, y, w, h) = self.terminal_run_rect(*run);
            rects.push(MinimapRect {
                x,
                y,
                w,
                h,
                color: selection_color,
            });
        }
        rects_to_vertex_bytes(&rects, self.config.width, self.config.height)
    }

    /// Curly terminal underlines through the existing squiggle pipeline.
    fn terminal_squiggle_vertex_bytes(&self) -> Vec<u8> {
        let Some(terminal) = self.terminal.as_ref() else {
            return Vec::new();
        };
        let rects: Vec<MinimapRect> = terminal
            .plan
            .underlines
            .iter()
            .filter(|underline| underline.style == UnderlineStyle::Curly)
            .map(|underline| {
                let (x, y, w, h) = self.terminal_run_rect(underline.run);
                MinimapRect {
                    x,
                    y: y + h - DIAG_SQUIGGLE_PX,
                    w,
                    h: DIAG_SQUIGGLE_PX,
                    color: rgb_to_quad(underline.color, 1.0),
                }
            })
            .collect();
        squiggles_to_vertex_bytes(&rects, self.config.width, self.config.height)
    }

    /// The child cursor's quad, painted through the caret primitive so
    /// it lands over the glyph it sits on.
    fn terminal_cursor_vertex_bytes(&self) -> Vec<u8> {
        let Some(terminal) = self.terminal.as_ref() else {
            return Vec::new();
        };
        let Some(cursor) = terminal.plan.cursor else {
            return Vec::new();
        };
        let (x, y, w, h) = self.terminal_run_rect(cursor);
        rects_to_vertex_bytes(
            &[MinimapRect {
                x,
                y,
                w,
                h,
                color: TERMINAL_CURSOR_RGBA,
            }],
            self.config.width,
            self.config.height,
        )
    }

    /// A geometry declaration for the current buffer if it
    /// changed, else `None`.
    ///
    /// Called after a snapshot and after any real geometry change
    /// (window resize, scale, font). An equal size is silent, so a
    /// redraw storm produces no wire traffic.
    fn terminal_declaration_if_changed(&mut self) -> Option<(BufferId, CellSize)> {
        let buffer_id = self.current_buffer_id?;
        let size = self.terminal_cell_viewport()?;
        if self.last_terminal_size_sent == Some((buffer_id, size)) {
            return None;
        }
        Some((buffer_id, size))
    }

    /// Whether terminal motion at `coord` is new information.
    ///
    /// Records the cell as reported either way, so a caller that skips
    /// the send still advances the memo. Press and release reset it
    /// (`last_terminal_pointer_cell = None`), which is what lets the
    /// first drag after a press reach the daemon even at the cell the
    /// press landed on.
    fn terminal_motion_is_new(&mut self, coord: CellCoord) -> bool {
        let changed = self.last_terminal_pointer_cell != Some(coord);
        self.last_terminal_pointer_cell = Some(coord);
        changed
    }

    /// Record a declaration the caller actually put on the wire.
    ///
    /// Separate from [`Self::terminal_declaration_if_changed`] so a
    /// FAILED send does not leave a size believed-declared: the daemon
    /// would still hold the old geometry while this frontend suppressed
    /// every retry as unchanged.
    fn note_terminal_declaration_sent(&mut self, buffer_id: BufferId, size: CellSize) {
        self.last_terminal_size_sent = Some((buffer_id, size));
    }

    /// True while the completion popup is open **for the buffer this
    /// window currently shows** — the predicate the key gates (Esc,
    /// RET/TAB) and the render path share, so a stale mirror can
    /// never act against a foreign buffer.
    fn completion_open_for_current_buffer(&self) -> bool {
        self.completion
            .as_ref()
            .is_some_and(|c| Some(c.buffer_id) == self.current_buffer_id)
    }

    /// A `ViewportSend` for the current `view_range` if it differs from
    /// the last one declared, else `None` (Q#S5 coalescing). `generation`
    /// is 0 — the producer's full-resync triggers on the visible-range
    /// change and on the CRDT generation bump, not this field.
    fn viewport_send_if_changed(&mut self, buffer_id: BufferId) -> Option<ViewportSend> {
        if self.last_viewport_sent == Some(self.view_range) {
            return None;
        }
        self.last_viewport_sent = Some(self.view_range);
        let (start, end) = self.view_range;
        Some(ViewportSend {
            buffer_id,
            visible: ByteRange { start, end },
            generation: 0,
        })
    }

    /// Edit-path variant of [`Self::viewport_send_if_changed`]. For
    /// ordinary insertion/deletion inside the visible slice, only the
    /// end byte moves; sending that on every `CrdtOp` doubles the
    /// frontend-to-daemon write traffic while the producer already has
    /// a CRDT generation transition to trigger a full viewport resync.
    /// If the start byte moves, the top visible line itself shifted, so
    /// the daemon needs a fresh declaration.
    fn viewport_send_if_origin_changed(&mut self, buffer_id: BufferId) -> Option<ViewportSend> {
        let Some((last_start, last_end)) = self.last_viewport_sent else {
            return self.viewport_send_if_changed(buffer_id);
        };
        if last_start != self.view_range.0 {
            return self.viewport_send_if_changed(buffer_id);
        }
        // End drift: typing grows the slice end while the declared
        // end stays put, and the daemon clips styling to the declared
        // range. Long unbroken typing would eat through the bottom
        // overscan and the deepest lines would lose styling — once
        // the drift exceeds half the overscan (in lines), re-declare.
        let starts = &self.current_line_starts;
        let declared_line = starts.partition_point(|&s| s <= last_end);
        let current_line = starts.partition_point(|&s| s <= self.view_range.1);
        if current_line.abs_diff(declared_line) * 2 > SCROLL_OVERSCAN {
            return self.viewport_send_if_changed(buffer_id);
        }
        None
    }

    /// Adjust `scroll_top` so the own cursor's source line is within the
    /// visible window (Q#S2). Returns whether `scroll_top` changed (in
    /// which case the caller re-shapes + re-declares the viewport).
    fn scroll_to_cursor(&mut self) -> bool {
        let Some(own) = self.own_cursor else {
            return false;
        };
        if self.current_buffer_id != Some(own.buffer_id) {
            return false;
        }
        let line_starts = &self.current_line_starts;
        let cursor = own.byte.min(self.current_text.len() as u64);
        // Cursor's source line = largest i with line_starts[i] <= cursor.
        let cursor_line = line_starts
            .partition_point(|&s| s <= cursor)
            .saturating_sub(1);
        let visible =
            estimated_visible_lines(self.config.height, self.fm, self.band_inset()).max(1);
        let old = self.scroll_top;
        if cursor_line < self.scroll_top {
            self.scroll_top = cursor_line;
        } else if cursor_line >= self.scroll_top + visible {
            self.scroll_top = cursor_line + 1 - visible;
        }
        self.scroll_top != old
    }

    /// Bring the own caret's VISUAL run into the drawable code window
    /// (framing Q#F6) — the shared follow helper for the `CursorByte`
    /// arm, the optimistic edit completion, and the font/resize
    /// transactions. The coarse source-line [`Self::scroll_to_cursor`]
    /// runs first (the byte may be outside the shaped slice), then
    /// cosmic-text's `Buffer::shape_until_cursor` follows wrapped
    /// layout runs vertically. Its `Scroll.horizontal` result is
    /// discarded — glyphon 0.11 never applies horizontal scroll when
    /// placing glyphs, so retaining it would make state claim a
    /// scroll the painter never displays. Ends by re-normalizing the
    /// scroll; callers declare the viewport only after that final
    /// source origin is stable.
    fn ensure_caret_painted(&mut self) {
        let Some(own) = self.own_cursor else {
            return;
        };
        if self.current_buffer_id != Some(own.buffer_id) {
            return;
        }
        if self.scroll_to_cursor() {
            // Pure scroll: retained lines keep their shape caches;
            // only newly exposed lines shape.
            self.rebuild_lines_reusing_scroll();
        }
        let byte = own.byte.min(self.current_text.len() as u64);
        if let Some(cursor) = self.code_byte_to_layout_cursor(byte) {
            self.buffer
                .shape_until_cursor(&mut self.font_system, cursor, false);
        }
        self.normalize_code_scroll();
        self.horizontal_follow(byte);
        self.request_redraw();
    }

    /// Move `code_scroll_left` so the caret's column is on screen
    /// (Stage 5, framing Q#G2 — automatic only).
    ///
    /// The horizontal mirror of `scroll_to_cursor`, and deliberately
    /// the same shape as the TUI's `horizontal_follow`: scroll only far
    /// enough to bring the caret back inside, so a caret already
    /// visible never moves the view. With no explicit scroll commands,
    /// every horizontal viewport move originates here.
    ///
    /// Runs AFTER `normalize_code_scroll` because it reads the caret's
    /// laid-out x, which the vertical normalization can change.
    ///
    /// **The decision is `pmacs_protocol::scroll::follow_left`**, the
    /// same function `src/editor.rs` calls, so the two frontends cannot
    /// choose different edges for the same cursor. That rule is stated
    /// in columns; this is where the conversion happens, and Q#G3 is
    /// what makes it exact — a non-monospace code font is rejected
    /// before it can reach layout, so every advance is the same width
    /// and `px / advance` is a column count rather than an estimate.
    ///
    /// The result is re-multiplied rather than kept in pixels, which
    /// **snaps the offset to the column grid**. That is the point: it
    /// is what makes "the same first visible character in both
    /// frontends" true rather than approximately true.
    fn horizontal_follow(&mut self, byte: u64) {
        // A wrapped buffer has nothing past the right edge; the offset
        // is pinned to 0 by `apply_line_wrap` and must stay there.
        if self.buffer.wrap() != Wrap::None {
            self.code_scroll_left = 0.0;
            return;
        }
        let Some((code_x, _top, _h)) = self.code_byte_px(byte) else {
            return;
        };
        let advance = self.mono_advance();
        let width = self.text_bounds_right() as f32 - self.text_left();
        if advance <= 0.0 || width <= 0.0 {
            return;
        }
        // `floor` for the width — a half-visible trailing column is not
        // a column you can read — and `round` for the two positions,
        // which are exact multiples of the advance up to f32
        // accumulation error.
        let cols = (width / advance).floor().max(0.0) as u32;
        let cursor_col = (code_x / advance).round().max(0.0) as u32;
        let left_col = (self.code_scroll_left / advance).round().max(0.0) as u32;
        let next = pmacs_protocol::scroll::follow_left(left_col, cursor_col, cols);
        self.code_scroll_left = next as f32 * advance;
    }

    /// Monospace glyph advance in px, used to size the line-number
    /// gutter (UX gutter arc). Once a `FontFacts` has been applied the
    /// measured NORMAL-face probe advance is authoritative (framing
    /// Q#F6) — different monospaced faces need not share an advance,
    /// so sampling an arbitrary (possibly bold/italic) code glyph
    /// could skew the gutter. Before any probe: today's behavior —
    /// the first shaped glyph, else the ratio-scaled constant.
    fn mono_advance(&self) -> f32 {
        if let Some(advance) = self.measured_mono_advance {
            return advance;
        }
        self.buffer
            .layout_runs()
            .flat_map(|run| run.glyphs.iter())
            .next()
            .map_or(self.fm.gutter_advance_fallback(), |g| g.w)
    }

    /// Width in px the line-number gutter reserves on the left, or 0 when
    /// disabled (UX gutter arc, Q#UX3): `digits * advance + gap`. Mirrors
    /// the TUI's `Window::gutter_width`; the unit here is pixels.
    fn gutter_width_px(&self) -> f32 {
        if !self.line_numbers.is_on() {
            return 0.0;
        }
        let lines = self.current_line_starts.len().max(1);
        let want = decimal_digits(lines) as f32 * self.mono_advance() + GUTTER_GAP_PX;
        // Fit guard (mirrors the TUI's too-narrow disable): never reserve so
        // much gutter that the text area collapses. `text_bounds_right` is
        // the text clip edge (minimap-aware); if the gutter would leave less
        // than `MIN_TEXT_WIDTH_PX` past `TEXT_LEFT`, drop it this frame
        // rather than shift `text_left` to or past the clip and render into
        // a degenerate `left >= right` rectangle.
        let avail = self.text_bounds_right() as f32 - TEXT_LEFT;
        if want + MIN_TEXT_WIDTH_PX > avail {
            0.0
        } else {
            want
        }
    }

    /// The code's left origin in px: `TEXT_LEFT` plus the gutter. Every
    /// byte→pixel x site adds this instead of the bare `TEXT_LEFT` (Q#UX2),
    /// and the pixel→byte hit-test subtracts it.
    fn text_left(&self) -> f32 {
        TEXT_LEFT + self.gutter_width_px()
    }

    /// **The** screen↔code transform (Stage 5, framing §1.1).
    ///
    /// A code-relative x — what `code_byte_px`, the decoration geometry
    /// and the math/completion origins all produce — becomes a screen x
    /// here and nowhere else. Written once because the alternative is
    /// five call sites that can disagree, and a disagreement between
    /// the caret and the glyphs it sits among is invisible until
    /// somebody scrolls.
    fn code_x_to_screen(&self, code_x: f32) -> f32 {
        self.text_left() - self.code_scroll_left + code_x
    }

    /// The exact inverse, for hit testing.
    fn screen_x_to_code(&self, screen_x: f32) -> f32 {
        screen_x - self.text_left() + self.code_scroll_left
    }

    /// **The** code clip rectangle's left edge (Stage 5, framing §1.1).
    ///
    /// glyphon honors `TextBounds`, so the document `TextArea` clips
    /// itself. **The manual quad and squiggle renderers do not** —
    /// nothing stopped them painting into the gutter, and nothing
    /// needed to, because before this stage no code-relative x could be
    /// negative. Every code-relative painter must now intersect with
    /// this.
    fn code_clip_left(&self) -> f32 {
        if self.line_numbers.is_on() {
            self.text_left().floor()
        } else {
            0.0
        }
    }

    /// Crop a code-relative rect `[x, x + w)` in SCREEN coordinates to
    /// the code clip's left edge, returning the surviving `(x, w)`.
    /// `None` when the rect lies wholly inside the gutter.
    ///
    /// **Cropping, not dropping**, because these rects are washes: a
    /// selection or search band running in from off the left edge must
    /// still paint the part that IS on screen. Dropping it whole is the
    /// exact boundary defect the TUI painter had before Stage 4.
    fn crop_to_code_clip_left(&self, screen_x: f32, w: f32) -> Option<(f32, f32)> {
        let left = self.code_clip_left();
        let right = screen_x + w;
        (right > left).then(|| {
            let x = screen_x.max(left);
            (x, right - x)
        })
    }

    /// Whether a code-relative rect `[x, x + w)` in SCREEN coordinates
    /// survives the code clip's left edge — the same boundary as
    /// [`Self::crop_to_code_clip_left`], for the callers (the caret)
    /// that want a yes/no rather than a cropped rect. Delegating keeps
    /// one rule: a caret the crop would discard is never painted.
    fn survives_code_clip_left(&self, screen_x: f32, w: f32) -> bool {
        self.crop_to_code_clip_left(screen_x, w).is_some()
    }

    /// The GPU's own cursor's 0-based buffer line, or `0` when there's no
    /// own cursor in the displayed buffer (relative/hybrid then count from
    /// the top — a rare transient). Derived from the whole-buffer line
    /// table, so it's independent of the shaped slice.
    fn cursor_line(&self) -> usize {
        let byte = match self.own_cursor.as_ref() {
            Some(c) if Some(c.buffer_id) == self.current_buffer_id => c.byte,
            _ => 0,
        };
        self.current_line_starts
            .partition_point(|&start| start <= byte)
            .saturating_sub(1)
    }

    /// Reshape the gutter buffer to the right-aligned line numbers for the
    /// currently-shaped code lines (UX gutter arc). The projection mirrors
    /// the code layout's VISUAL runs (framing Q#F6): each source line's
    /// number rides its first run and wrapped continuation runs get blank
    /// gutter rows, then the code buffer's normalized vertical scroll is
    /// applied verbatim — both buffers share the line height, so rows stay
    /// aligned when wrapping or a caret-follow residual is active. No-op
    /// when the gutter is off.
    fn refresh_gutter_buffer(&mut self) {
        use std::fmt::Write as _;
        if !self.line_numbers.is_on() {
            return;
        }
        let digits = decimal_digits(self.current_line_starts.len().max(1)) as usize;
        let first = self.shaped_top;
        // Relative/Hybrid measure distance from the cursor's buffer line;
        // Absolute ignores it. Rebuilt every render, so it tracks the cursor.
        let cursor_line = self.cursor_line();
        let mode = self.line_numbers;
        let n = self.buffer.lines.len();
        let mut text = String::new();
        for i in 0..n {
            if i > 0 {
                text.push('\n');
            }
            let num = mode.number_for(first + i, cursor_line).unwrap_or(0);
            let _ = write!(text, "{num:>digits$}");
            // Continuation blanks: one empty row per wrapped run past
            // the first. Unlaid lines (below the drawable window) count
            // as one row; their gutter rows are equally invisible.
            let runs = self
                .buffer
                .line_layout(&mut self.font_system, i)
                .map_or(1, |layout| layout.len().max(1));
            for _ in 1..runs {
                text.push('\n');
            }
        }
        let family = self.resolved_family.clone();
        self.gutter_buffer.set_text(
            &mut self.font_system,
            &text,
            &Attrs::new().family(Family::Name(&family)),
            Shaping::Advanced,
            None,
        );
        self.gutter_buffer
            .set_scroll(Scroll::new(0, self.code_scroll_residual, 0.0));
        self.gutter_buffer
            .shape_until_scroll(&mut self.font_system, false);
    }

    /// Resolve a window-pixel position to an **absolute source byte**
    /// (Q#M2): pixel → cosmic-text hit (shaped line + byte within
    /// line) → projected byte → run map → slice byte → + `vstart`.
    /// `None` when no buffer is attached or the position is outside
    /// anything hit-testable.
    /// Text-relative x for hit testing, classifying the gutter band first
    /// (UX gutter, Q#UX6). A click left of the text origin (`raw_x < 0`,
    /// i.e. inside the gutter) is not a text hit — it clamps to `0.0`, the
    /// line start, rather than feeding glyphon a negative x (undefined).
    /// Mirrors the TUI's saturate-to-column-0 affordance and is the stable
    /// seam a future gutter marker would branch on instead of relying on
    /// glyphon's negative-x edge behavior.
    fn gutter_aware_rel_x(&self, x: f64) -> f32 {
        // Stage 5: the EXACT inverse of `code_x_to_screen`, so a click
        // lands on the glyph under the pointer at any offset. The
        // gutter clamp stays in SCREEN space and is applied first — a
        // click in the gutter band means "the first visible column",
        // which after scrolling is the offset, not column 0.
        if self.line_numbers.is_on() && (x as f32) < self.text_left() {
            return self.code_scroll_left;
        }
        self.screen_x_to_code(x as f32)
    }

    /// The shaped slice's math substitutions, read back from the
    /// per-line chunk caches and rebased to slice-relative offsets —
    /// the spacer text and suppressed range are both in the cached
    /// `MathBox` chunks, so the hit map reproduces the shaped state
    /// exactly instead of re-planning under a possibly-newer caret.
    fn cached_math_subs_for_slice(&self, vstart: u64, vend: u64) -> Vec<MathSubstitution> {
        let mut subs = Vec::new();
        if self.math_engine.is_none() {
            return subs;
        }
        let (top, ranges) = self.slice_line_ranges(vstart, vend);
        if top != self.shaped_top || ranges.len() != self.line_chunk_cache.len() {
            // The caches describe a different slice; a math-blind map
            // (boxes unclickable at worst) beats a wrong one.
            return subs;
        }
        for (i, &(ls, _)) in ranges.iter().enumerate() {
            let base = ls - vstart;
            for chunk in &self.line_chunk_cache[i] {
                if let ChunkSource::MathBox { start, end } = chunk.source {
                    subs.push(MathSubstitution {
                        span: math_parse::MathSpan {
                            start: (base + start) as usize,
                            end: (base + end) as usize,
                        },
                        spacer: chunk.text.clone(),
                        boxed: math_layout::MathBox {
                            width: 0.0,
                            ascent: 0.0,
                            descent: 0.0,
                            items: Vec::new(),
                        },
                    });
                }
            }
        }
        subs
    }

    fn hit_test_source_byte(&mut self, x: f64, y: f64) -> Option<u64> {
        self.current_buffer_id?;
        if self.hit_map_dirty {
            // Q#R2 — a per-line reshape deferred this; rebuild from
            // the same chunk source the shaped buffer was built from.
            let (vstart, vend) = self.view_range;
            // B1': the hit map must see the SAME math suppressions the
            // shaped lines carry, so the slice-wide substitution list
            // is read back from the per-line caches (never recomputed
            // — a caret that moved since the last reshape must not
            // make the map disagree with the glyphs), rebased from
            // line-relative to slice-relative offsets.
            let subs = self.cached_math_subs_for_slice(vstart, vend);
            let rich = clipped_chunks_for_range(
                &self.current_text,
                &self.current_spans,
                &self.current_adornments,
                vstart,
                vend,
                &subs,
            );
            let (hit_runs, projected_line_starts) = build_hit_runs(&rich);
            self.current_hit_runs = hit_runs;
            self.projected_line_starts = projected_line_starts;
            self.hit_map_dirty = false;
        }
        let rel_x = self.gutter_aware_rel_x(x);
        let rel_y = y as f32 - TEXT_TOP;
        let cursor = self.buffer.hit(rel_x, rel_y)?;
        let line_start = *self.projected_line_starts.get(cursor.line)?;
        let projected = line_start + cursor.index as u64;
        let slice_byte = projected_to_source(&self.current_hit_runs, projected)?;
        let (vstart, vend) = self.view_range;
        Some((vstart + slice_byte).min(vend))
    }

    /// Wheel scroll (local-only — the GPU owns the viewport; no wire
    /// event exists or is needed). Positive `delta` scrolls down.
    fn scroll_by_lines(&mut self, delta: i64) -> Option<ViewportSend> {
        let max_top = self.current_line_starts.len().saturating_sub(1);
        let new_top = self
            .scroll_top
            .saturating_add_signed(delta as isize)
            .min(max_top);
        // An explicit jump owns the viewport wholesale: it also clears
        // the caret-follow residual (framing Q#F6), so a retained
        // sub-line offset can't pin the top row. A wheel-up at the top
        // clamp edge still scrolls when its only remaining motion IS
        // the residual; a wheel-down pinned at the bottom clamp keeps
        // the residual (nothing below to reveal).
        let residual_scrolls_up = delta < 0 && self.code_scroll_residual != 0.0;
        if new_top == self.scroll_top && !residual_scrolls_up {
            return None;
        }
        self.code_scroll_residual = 0.0;
        self.scroll_top = new_top;
        self.rebuild_lines_reusing_scroll();
        self.current_buffer_id
            .and_then(|bid| self.viewport_send_if_changed(bid))
    }

    /// True when the pixel position lies inside the minimap band
    /// (Q#M6). Presses here are consumed locally and never become
    /// `Pointer` events.
    fn in_minimap_band(&self, x: f64, y: f64) -> bool {
        minimap_band_contains(
            x as f32,
            y as f32,
            self.config.width,
            self.config.height,
            self.fm,
            self.band_inset(),
        )
    }

    /// Popup width in pixels (Q#CM1) — widest label estimated from a
    /// fixed per-char advance, padded, clamped. Used by both hit-testing
    /// and the bg quad so they line up.
    fn menu_width_px(menu: &MenuLocal, fm: FontMetrics) -> f32 {
        let max_chars = menu
            .rows
            .iter()
            .map(|r| r.label.chars().count())
            .max()
            .unwrap_or(0);
        (max_chars as f32 * fm.menu_char_w() + 2.0 * MENU_PAD_X)
            .clamp(MENU_MIN_WIDTH, MENU_MAX_WIDTH)
    }

    /// Hit-test a pixel against the open popup (Q#CM1). Returns
    /// `(row_index, is_item)` when inside the popup rectangle, or `None`
    /// when outside (or no menu open).
    fn menu_hit(&self, x: f64, y: f64) -> Option<(u32, bool)> {
        let menu = self.menu.as_ref()?;
        let (ax, ay) = menu.anchor_px;
        let w = f64::from(Self::menu_width_px(menu, self.fm));
        let h = menu.rows.len() as f64 * f64::from(self.fm.menu_row_height());
        if x < ax || x >= ax + w || y < ay || y >= ay + h {
            return None;
        }
        let row = (((y - ay) / f64::from(self.fm.menu_row_height())).floor() as usize)
            .min(menu.rows.len() - 1);
        Some((row as u32, !menu.rows[row].separator))
    }

    /// Center the viewport on the source line the minimap pixel `y`
    /// maps to — the inverse of the painter's linear line→y
    /// interpolation. Reuses [`Self::scroll_by_lines`] for the
    /// clamp / rebuild / viewport-send plumbing.
    fn minimap_jump_to(&mut self, y: f64) -> Option<ViewportSend> {
        let target = minimap_y_to_line(
            y as f32,
            self.config.height,
            self.current_line_starts.len(),
            self.fm,
            self.band_inset(),
        )?;
        let centered = target.saturating_sub(
            estimated_visible_lines(self.config.height, self.fm, self.band_inset()) / 2,
        );
        let delta = i64::try_from(centered).unwrap_or(i64::MAX)
            - i64::try_from(self.scroll_top).unwrap_or(i64::MAX);
        self.scroll_by_lines(delta)
    }

    /// Q#R1 — per-line incremental reshape for a single-line text
    /// edit: rebuild ONE `BufferLine` instead of re-shaping the whole
    /// visible slice. Returns `false` when the edit needs the full
    /// `reshape` (slice origin moved, edited line outside the shaped
    /// slice, exotic paragraph separators that the full path would
    /// have split on). The caller has already established the edit is
    /// single-line (line count unchanged, no `\n` inserted).
    fn try_reshape_line(&mut self, edit: TextProjectionEdit) -> bool {
        let (vstart, vend) = self.visible_byte_range();
        if vstart != self.view_range.0 {
            // The slice origin moved (edit before the viewport): the
            // whole slice shifts; surgery can't help.
            return false;
        }
        if edit.start >= vend {
            // Entirely past the visible slice: no shaped line's
            // content changes; offsets are clip-rebased per frame.
            self.view_range = (vstart, vend);
            self.hit_map_dirty = true;
            self.request_redraw();
            return true;
        }
        let line_idx = self
            .current_line_starts
            .partition_point(|&s| s <= edit.start)
            .saturating_sub(1);
        let line_start = self.current_line_starts[line_idx];
        if line_start < vstart {
            return false;
        }
        let next_start = self.current_line_starts.get(line_idx + 1).copied();
        let content_end = next_start
            .map_or(self.current_text.len() as u64, |n| n.saturating_sub(1))
            .min(vend);
        let Some(shaped_idx) = line_idx.checked_sub(self.shaped_top) else {
            return false;
        };
        if shaped_idx >= self.buffer.lines.len()
            || shaped_idx >= self.line_chunk_cache.len()
            || shaped_idx >= self.line_math_cache.len()
        {
            // E.g. typing on the phantom empty line after a trailing
            // newline — no BufferLine exists for it; full reshape
            // handles those shapes correctly.
            return false;
        }
        let (chunks, math) = self.chunks_for_line(line_start, content_end);
        self.buffer.lines[shaped_idx] = line_from_chunks(&chunks, &self.resolved_family);
        self.line_chunk_cache[shaped_idx] = chunks;
        self.line_math_cache[shaped_idx] = math;
        self.view_range = (vstart, vend);
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        // The edited line's wrap count can shrink under a retained
        // residual, advancing the slice-local scroll (framing Q#F6);
        // its rebuilds re-derive `view_range` themselves.
        self.normalize_code_scroll();
        self.hit_map_dirty = true;
        self.request_redraw();
        true
    }

    /// `(top, [(line_start, content_end)])` for the slice
    /// `[vstart, vend)`: one entry per shaped line, content excluding
    /// the `\n`. A line starting exactly at `vend` (incl. the phantom
    /// line after a trailing `\n`) is not shaped — matching the line
    /// splitting `set_rich_text` used to do.
    fn slice_line_ranges(&self, vstart: u64, vend: u64) -> (usize, Vec<(u64, u64)>) {
        let starts = &self.current_line_starts;
        let n = starts.len();
        let top = self.scroll_top.min(n.saturating_sub(1));
        let mut ranges = Vec::new();
        let mut idx = top;
        while idx < n {
            let ls = starts[idx];
            if ls >= vend {
                break;
            }
            let ce = starts
                .get(idx + 1)
                .map_or(self.current_text.len() as u64, |&next| next - 1)
                .min(vend);
            ranges.push((ls, ce));
            idx += 1;
        }
        if ranges.is_empty() {
            ranges.push((vstart, vstart));
        }
        (top, ranges)
    }

    fn chunks_for_line(
        &self,
        line_start: u64,
        content_end: u64,
    ) -> (Vec<RichChunk>, MathLineState) {
        let (subs, mut state) = self.math_plan_for_line(line_start, content_end);
        let chunks = clipped_chunks_for_range(
            &self.current_text,
            &self.current_spans,
            &self.current_adornments,
            line_start,
            content_end,
            &subs,
        );
        state.placed = placed_math_boxes(&chunks, &subs);
        (chunks, state)
    }

    /// The line's math suppression plan (Q#MS3/Q#MS4). Detection runs
    /// here — the chunk-build path, never the edit path — and the gate
    /// reads the EFFECTIVE caret (`own_cursor`, which optimistic edits
    /// predict forward) plus the own-selection endpoints, so
    /// suppression cannot flap during an unconfirmed edit (framing
    /// Q#MS5/F4). Any failure — parse, layout, fit, degenerate spacer —
    /// leaves that span as source (Q#MS8).
    fn math_plan_for_line(
        &self,
        line_start: u64,
        content_end: u64,
    ) -> (Vec<MathSubstitution>, MathLineState) {
        let mut subs = Vec::new();
        let mut state = MathLineState::default();
        let Some(engine) = self.math_engine.as_ref() else {
            return (subs, state);
        };
        let Some(line) = self
            .current_text
            .get(line_start as usize..content_end as usize)
        else {
            return (subs, state);
        };
        if !line.as_bytes().contains(&b'$') {
            return (subs, state);
        }
        let spans = math_parse::detect_math_spans(line);
        if spans.is_empty() {
            return (subs, state);
        }
        let gates = self.math_gate_positions(line_start, content_end);
        let advance = self.mono_advance();
        for span in spans {
            let gated = gates
                .iter()
                .any(|&p| span.start as u64 <= p && p <= span.end as u64);
            state.gates.push((span, gated));
            if gated {
                continue;
            }
            let Ok(node) = math_parse::parse(&line[span.interior()]) else {
                continue;
            };
            let Ok(boxed) = engine.layout(&node, self.fm.code_font_size()) else {
                continue;
            };
            let Some(fitted) =
                math_layout::fit_to_line(&boxed, self.math_budget.0, self.math_budget.1)
            else {
                continue;
            };
            let spacer = math_layout::spacer_for_width(fitted.width, advance);
            if spacer.is_empty() {
                continue;
            }
            subs.push(MathSubstitution {
                span,
                spacer,
                boxed: fitted,
            });
        }
        (subs, state)
    }

    /// Line-relative byte positions whose presence inside a span
    /// unsuppresses it: the effective caret and both own-selection
    /// endpoints (Q#MS5 generalised by Q#MS11 — "you are addressing
    /// this text" and "you see this text" are the same condition).
    fn math_gate_positions(&self, line_start: u64, content_end: u64) -> Vec<u64> {
        let mut gates = Vec::new();
        let mut push = |byte: u64| {
            if byte >= line_start && byte <= content_end {
                gates.push(byte - line_start);
            }
        };
        if let Some(own) = self.own_cursor
            && Some(own.buffer_id) == self.current_buffer_id
        {
            push(own.byte);
        }
        for d in &self.current_decorations {
            if d.kind == DecorationKind::Selection {
                push(d.range.start);
                push(d.range.end);
            }
        }
        gates
    }

    /// Whether a retained line's cached gate bits match the current
    /// effective caret/selection — the suppression input to the
    /// line-reuse predicate beside content and styling (Q#MS5; a
    /// retained line shaped under the opposite suppression state is
    /// the #120 stale-mirror failure). Content is unchanged on every
    /// reuse path, so the cached span set is authoritative and only
    /// the gate bits can differ.
    fn math_gates_match(&self, line_start: u64, content_end: u64, state: &MathLineState) -> bool {
        if state.gates.is_empty() {
            return true;
        }
        let gates = self.math_gate_positions(line_start, content_end);
        state.gates.iter().all(|&(span, was_gated)| {
            let now = gates
                .iter()
                .any(|&p| span.start as u64 <= p && p <= span.end as u64);
            now == was_gated
        })
    }

    /// Caret or selection motion can flip a span's Q#MS5 gate without
    /// any content change, which the frame-driven refresh paths never
    /// see; re-run the per-line chunk compare when the visible slice
    /// could hold math at all. Cheap when it cannot (one `$` scan).
    fn refresh_math_suppression(&mut self) {
        if self.math_engine.is_none() || self.terminal.is_some() {
            return;
        }
        let (vstart, vend) = self.view_range;
        let Some(slice) = self.current_text.get(vstart as usize..vend as usize) else {
            return;
        };
        if slice.as_bytes().contains(&b'$') {
            self.refresh_changed_lines();
        }
    }

    /// Rebuild the shaped slice, reusing any retained line whose
    /// absolute index was already shaped (pure scroll: content and
    /// styling unchanged for retained lines, their shape caches
    /// survive — only newly exposed lines pay shaping). Falls back to
    /// building everything when nothing overlaps. Every builder keeps
    /// `line_chunk_cache` current, so reuse is always sound here.
    fn rebuild_lines_reusing_scroll(&mut self) {
        let (vstart, vend) = self.visible_byte_range();
        self.view_range = (vstart, vend);
        let (new_top, ranges) = self.slice_line_ranges(vstart, vend);
        let old_top = self.shaped_top;
        let mut old_lines: Vec<Option<glyphon::cosmic_text::BufferLine>> =
            std::mem::take(&mut self.buffer.lines)
                .into_iter()
                .map(Some)
                .collect();
        let mut old_cache: Vec<Option<Vec<RichChunk>>> = std::mem::take(&mut self.line_chunk_cache)
            .into_iter()
            .map(Some)
            .collect();
        let mut old_math: Vec<Option<MathLineState>> = std::mem::take(&mut self.line_math_cache)
            .into_iter()
            .map(Some)
            .collect();
        let mut lines = Vec::with_capacity(ranges.len());
        let mut cache = Vec::with_capacity(ranges.len());
        let mut math = Vec::with_capacity(ranges.len());
        let mut any_reused = false;
        for (i, &(ls, ce)) in ranges.iter().enumerate() {
            let abs = new_top + i;
            let reused = abs.checked_sub(old_top).and_then(|j| {
                if j < old_lines.len() && j < old_cache.len() && j < old_math.len() {
                    // Reuse is sound only when suppression state is a
                    // third invariant beside content and styling
                    // (Q#MS5): a caret-follow scroll lands here with a
                    // caret that may have crossed a span boundary, and
                    // a retained line shaped under the opposite
                    // suppression state is the #120 stale-mirror
                    // failure (framing acceptance 11).
                    if !self.math_gates_match(ls, ce, old_math[j].as_ref()?) {
                        return None;
                    }
                    Some((
                        old_lines[j].take()?,
                        old_cache[j].take()?,
                        old_math[j].take()?,
                    ))
                } else {
                    None
                }
            });
            if let Some((line, chunks, m)) = reused {
                any_reused = true;
                lines.push(line);
                cache.push(chunks);
                math.push(m);
            } else {
                let (chunks, m) = self.chunks_for_line(ls, ce);
                lines.push(line_from_chunks(&chunks, &self.resolved_family));
                cache.push(chunks);
                math.push(m);
            }
        }
        self.buffer.lines = lines;
        self.line_chunk_cache = cache;
        self.line_math_cache = math;
        self.shaped_top = new_top;
        self.buffer
            .set_scroll(Scroll::new(0, self.code_scroll_residual, 0.0));
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        self.normalize_code_scroll();
        self.hit_map_dirty = true;
        if any_reused || self.line_chunk_cache.is_empty() {
            self.styled_redraw_deadline = None;
            self.request_redraw();
        } else {
            // Far jump (Q#M6, bet #2): every line rebuilt, and the
            // span set covers the *old* viewport — drawing now would
            // flash unstyled text. Hold the redraw until the restyle
            // lands (`refresh_changed_lines` clears this) or the
            // deadline fires in `about_to_wait`.
            self.styled_redraw_deadline = Some(std::time::Instant::now() + JUMP_STYLE_HOLD);
        }
    }

    /// Re-shape ONLY lines whose chunk set changed — the incoming
    /// frame path (`StyleSpans` / fg `Decorations` / `InlineAdornments`).
    /// A parse-settle frame after a typing burst usually recolors a
    /// line or two; re-shaping the whole slice for it was a full
    /// keystroke-cost stall.
    fn refresh_changed_lines(&mut self) {
        let (vstart, vend) = self.visible_byte_range();
        let (top, ranges) = self.slice_line_ranges(vstart, vend);
        if (vstart, vend) != self.view_range
            || top != self.shaped_top
            || ranges.len() != self.line_chunk_cache.len()
            || ranges.len() != self.line_math_cache.len()
            || ranges.len() != self.buffer.lines.len()
        {
            self.reshape();
            return;
        }
        let mut any = false;
        for (i, &(ls, ce)) in ranges.iter().enumerate() {
            let (chunks, m) = self.chunks_for_line(ls, ce);
            if chunks != self.line_chunk_cache[i] {
                self.buffer.lines[i] = line_from_chunks(&chunks, &self.resolved_family);
                self.line_chunk_cache[i] = chunks;
                any = true;
            }
            // Always current, even when the chunks are unchanged: an
            // unsuppressible span's gate bit can flip with no chunk
            // difference, and a stale bit would defeat the reuse
            // comparison later.
            self.line_math_cache[i] = m;
        }
        if any {
            self.buffer.shape_until_scroll(&mut self.font_system, false);
            // Adornment chunks can change a line's wrap count under a
            // retained residual (framing Q#F6).
            self.normalize_code_scroll();
            self.hit_map_dirty = true;
        }
        // Fresh styling reached the slice — release any held
        // post-jump frame (Q#M6, bet #2).
        self.styled_redraw_deadline = None;
        self.request_redraw();
    }

    // -----------------------------------------------------------------
    // Themes (Q#TH5/Q#TH7/Q#TH8): UI-face resolution. Faces arrive
    // daemon-resolved over `ThemeFacts`; lookups are exact-name. A set
    // face owns its surface within its stage-1 mask, and a `Default`
    // component inside the mask maps to the frontend's PLAIN rendering
    // — the buffer-text default fg / the window-background bg — never
    // the old chrome constant. An UNSET face keeps the site constant.
    // -----------------------------------------------------------------

    /// fg resolution for an {fg}-mask site: face set → its fg
    /// (`Default` ↦ the plain text color); unset → `fallback`
    /// (today's site constant).
    fn face_fg_or(&self, name: &str, fallback: Color) -> Color {
        match self.faces.get(name) {
            Some(f) => cell_color_to_glyphon(f.fg).unwrap_or_else(plain_text_color),
            None => fallback,
        }
    }

    /// Wash resolution for a {bg}-mask face: face set → its bg RGB
    /// carrying the site's current alpha (`Default` bg ↦ no wash — a
    /// fully transparent quad); unset → `fallback` (today's wash
    /// constant, alpha included).
    fn face_wash_or(&self, name: &str, fallback: [f32; 4]) -> [f32; 4] {
        match self.faces.get(name) {
            Some(f) => match cell_color_to_glyphon(f.bg) {
                Some(c) => glyphon_to_rgba(c, fallback[3]),
                None => [0.0, 0.0, 0.0, 0.0],
            },
            None => fallback,
        }
    }

    /// `Some((band quad rgba, band text color))` when `ui.modeline`
    /// is set — mask {fg, bg, reverse}: `Default` bg ↦ the window
    /// background (an untinted band), `Default` fg ↦ the plain text
    /// color, `reverse` swaps the two after mapping. `None` when
    /// unset: each band site keeps its own constant.
    fn modeline_face_colors(&self) -> Option<([f32; 4], Color)> {
        let f = self.faces.get("ui.modeline")?;
        let text = cell_color_to_glyphon(f.fg).unwrap_or_else(plain_text_color);
        let quad = match cell_color_to_glyphon(f.bg) {
            Some(c) => glyphon_to_rgba(c, 1.0),
            None => WINDOW_BG_RGBA,
        };
        Some(if f.reverse {
            (glyphon_to_rgba(text, 1.0), rgba_to_glyphon(quad))
        } else {
            (quad, text)
        })
    }

    /// Diag-family TEXT color (Q#TH5 policy): the `ui.diag.*` face's
    /// fg when set with a concrete color, else `fallback` (the
    /// built-in severity constant). Unlike [`Self::face_fg_or`], a
    /// set face's `Default` fg maps to the BUILT-IN color, never
    /// plain — the severity color doubles as the minimap presence
    /// encoding, so a plain severity is unrepresentable.
    fn diag_face_fg_or(&self, name: &str, fallback: Color) -> Color {
        self.faces
            .get(name)
            .and_then(|f| cell_color_to_glyphon(f.fg))
            .unwrap_or(fallback)
    }

    /// Diag-family quad color — [`Self::diag_face_fg_or`]'s rgba
    /// twin, keyed by decoration kind.
    fn diag_face_rgba(&self, kind: DecorationKind, fallback: [f32; 4]) -> [f32; 4] {
        let name = match kind {
            DecorationKind::DiagnosticError => "ui.diag.error",
            DecorationKind::DiagnosticWarning => "ui.diag.warning",
            DecorationKind::DiagnosticInfo => "ui.diag.info",
            DecorationKind::DiagnosticHint => "ui.diag.hint",
            _ => return fallback,
        };
        match self
            .faces
            .get(name)
            .and_then(|f| cell_color_to_glyphon(f.fg))
        {
            Some(c) => glyphon_to_rgba(c, 1.0),
            None => fallback,
        }
    }

    /// The OWN-window wash color for a background decoration kind:
    /// the local selection and search washes resolve their faces;
    /// peer rects (`collect_peer_rects`) deliberately keep the
    /// constants — peer theming rides the deferred peer-cursor
    /// palette arc (Q#TH5, round 2 finding 9).
    fn own_wash_color(&self, kind: DecorationKind) -> Option<[f32; 4]> {
        let fallback = decoration_kind_to_bg_color(kind)?;
        let name = match kind {
            DecorationKind::Selection => "ui.selection",
            DecorationKind::SearchMatch => "ui.search.match",
            DecorationKind::SearchMatchActive => "ui.search.match.active",
            _ => return Some(fallback),
        };
        Some(self.face_wash_or(name, fallback))
    }

    /// The band's left-segment text color, mirroring the content
    /// precedence in [`Self::compose_status_left_runs`].
    fn status_left_color(&self) -> Color {
        let fallback = Color::rgb(200, 200, 210);
        if self.minibuffer.is_some()
            || self
                .search_prompt
                .as_ref()
                .is_some_and(|s| Some(s.buffer_id) == self.current_buffer_id)
        {
            return self.face_fg_or("ui.minibuffer", fallback);
        }
        let has_message = self
            .status_facts
            .as_ref()
            .filter(|f| Some(f.buffer_id) == self.current_buffer_id)
            .is_some_and(|f| f.message.is_some());
        if has_message {
            return self.face_fg_or("ui.statusline", fallback);
        }
        self.modeline_face_colors()
            .map_or(fallback, |(_, text)| text)
    }

    fn status_right_base_color(&self) -> Color {
        self.modeline_face_colors()
            .map_or(Color::rgb(168, 168, 180), |(_, text)| text)
    }

    /// Resolve an exact custom face against `ThemeFacts`. The producer
    /// already normalizes custom entries to an {fg}-only style; absent
    /// entries, `ui.modeline`, and defensive `Default` all select the
    /// effective base modeline color.
    fn status_segment_color(&self, face: &str, base: Color) -> Color {
        if face == "ui.modeline" {
            return base;
        }
        self.faces
            .get(face)
            .and_then(|style| cell_color_to_glyphon(style.fg))
            .unwrap_or(base)
    }

    fn current_statusline_segments(&self) -> Option<&StatuslineSegmentsLocal> {
        self.statusline_segments
            .as_ref()
            .filter(|segments| Some(segments.buffer_id) == self.current_buffer_id)
    }

    /// Compose the protected right group. Custom providers precede the
    /// legacy diagnostic/cursor/scroll suffix. Custom boundaries are one
    /// base-colored space; the built-in suffix retains its exact two-space
    /// separators.
    /// `&mut self` because the wrapped branch asks cosmic-text where two
    /// bytes actually landed, and laying a line out shapes it. That is
    /// the point rather than a wart: the alternative is a per-frame
    /// cached `(first_visible, last_visible)` pair, which is a value
    /// maintained beside the layout and free to disagree with it —
    /// the same shape as the `code_wrap` shadow field this lane already
    /// removed once.
    fn compose_status_runs(&mut self) -> Vec<(String, Color)> {
        use std::fmt::Write as _;

        let base = self.status_right_base_color();
        let mut runs = Vec::new();
        if let Some(custom) = self.current_statusline_segments() {
            for segment in &custom.right {
                if !runs.is_empty() {
                    runs.push((" ".to_owned(), base));
                }
                runs.push((
                    segment.text.clone(),
                    self.status_segment_color(&segment.face, base),
                ));
            }
        }

        let mut builtins = Vec::new();
        if let Some(facts) = self
            .status_facts
            .as_ref()
            .filter(|facts| Some(facts.buffer_id) == self.current_buffer_id)
        {
            if facts.diag_errors > 0 {
                builtins.push((
                    format!("E:{}", facts.diag_errors),
                    self.diag_face_fg_or("ui.diag.error", Color::rgb(241, 76, 76)),
                ));
            }
            if facts.diag_warnings > 0 {
                builtins.push((
                    format!("W:{}", facts.diag_warnings),
                    self.diag_face_fg_or("ui.diag.warning", Color::rgb(245, 245, 67)),
                ));
            }
        }

        let mut readout = String::new();
        let mut cursor_row = self.scroll_top;
        if let Some(own) = self.own_cursor
            && self.current_buffer_id == Some(own.buffer_id)
        {
            let byte = floor_char_boundary(
                &self.current_text,
                (own.byte as usize).min(self.current_text.len()),
            );
            let line = self
                .current_line_starts
                .partition_point(|&start| start as usize <= byte)
                .saturating_sub(1);
            cursor_row = line;
            let line_start = self.current_line_starts.get(line).copied().unwrap_or(0) as usize;
            let col = self
                .current_text
                .get(line_start..byte)
                .map_or(0, |text| text.chars().count());
            let _ = write!(readout, "L{}:C{}", line + 1, col + 1);
            readout.push_str("  ");
        }
        if self.buffer.wrap() == Wrap::None {
            readout.push_str(&format_scroll_indicator(
                self.scroll_top,
                estimated_visible_lines(self.config.height, self.fm, self.band_inset()),
                self.current_line_starts.len(),
                cursor_row,
            ));
        } else {
            // Wrapping makes the line-space formatter wrong, not merely
            // imprecise: it compares `visible` (VISUAL rows that fit)
            // against `total_lines` (SOURCE lines), so a document whose
            // lines each wrap to three rows reports `Bot` from a third
            // of the way down. This is not new in this lane — the GPU
            // has always wrapped — but the lane is where it became
            // nameable, because `ui.line-wrap` is now what decides which
            // formula applies.
            //
            // The percentage comes from bytes because there is no row
            // total to take it from: only the viewport slice is shaped,
            // so rows below it were never laid out and counting them
            // arithmetically would disagree with the breaks cosmic-text
            // actually chose. Same rule as the TUI, from
            // `pmacs_protocol::scroll`.
            let byte_len = self.current_text.len() as u64;
            let byte_pos = self
                .own_cursor
                .filter(|own| self.current_buffer_id == Some(own.buffer_id))
                .map_or_else(
                    // No cursor of ours in this buffer: the viewport's
                    // own top is the honest position, matching the
                    // `cursor_row = self.scroll_top` fallback above.
                    || {
                        self.current_line_starts
                            .get(self.scroll_top)
                            .copied()
                            .unwrap_or(0)
                    },
                    |own| own.byte.min(byte_len),
                );
            let first_visible = self.code_byte_painted(0);
            let last_visible = self.code_byte_painted(byte_len);
            readout.push_str(&render_scroll_position(pmacs_protocol::scroll::classify(
                first_visible,
                last_visible,
                byte_pos,
                byte_len,
            )));
        }
        builtins.push((readout, base));

        if !runs.is_empty() {
            runs.push((" ".to_owned(), base));
        }
        for (index, builtin) in builtins.into_iter().enumerate() {
            if index > 0 {
                runs.push(("  ".to_owned(), base));
            }
            runs.push(builtin);
        }
        runs
    }

    /// Compose the left group. Minibuffer, isearch, and transient
    /// messages suppress custom left segments; ordinary buffer identity
    /// starts at the leading edge but may be fully clipped by the right group.
    fn compose_status_left_runs(&self) -> Vec<(String, Color)> {
        if let Some(minibuffer) = self.minibuffer.as_ref() {
            return vec![(
                format!("{}{}", minibuffer.prompt, minibuffer.input),
                self.status_left_color(),
            )];
        }
        if let Some(search) = self
            .search_prompt
            .as_ref()
            .filter(|search| Some(search.buffer_id) == self.current_buffer_id)
        {
            let label = if search.regex {
                "Regex I-search: "
            } else {
                "I-search: "
            };
            let count = if search.query.is_empty() {
                String::new()
            } else if search.invalid {
                " [invalid]".to_owned()
            } else if search.total == 0 {
                " [no match]".to_owned()
            } else {
                format!(
                    " ({}/{})",
                    search.active.map_or(0, |active| active + 1),
                    search.total
                )
            };
            return vec![(
                format!("{label}{}{count}", search.query),
                self.status_left_color(),
            )];
        }
        if let Some(message) = self
            .status_facts
            .as_ref()
            .filter(|facts| Some(facts.buffer_id) == self.current_buffer_id)
            .and_then(|facts| facts.message.as_deref())
        {
            return vec![(message.to_owned(), self.status_left_color())];
        }

        let base = self.status_left_color();
        let identity = match self
            .status_facts
            .as_ref()
            .filter(|facts| Some(facts.buffer_id) == self.current_buffer_id)
        {
            Some(facts) if facts.modified => format!("{} ●", facts.name),
            Some(facts) => facts.name.clone(),
            None => String::new(),
        };
        let mut runs = Vec::new();
        if !identity.is_empty() {
            runs.push((identity, base));
        }
        if let Some(custom) = self.current_statusline_segments() {
            for segment in &custom.left {
                if !runs.is_empty() {
                    runs.push((" ".to_owned(), base));
                }
                runs.push((
                    segment.text.clone(),
                    self.status_segment_color(&segment.face, base),
                ));
            }
        }
        runs
    }

    /// Re-shape only when the complete ordered rich-run key changes.
    /// Cache advancement follows successful installation and shaping.
    fn refresh_status_line(&mut self) {
        let right = self.compose_status_runs();
        let left = self.compose_status_left_runs();
        let family = self.resolved_family.clone();
        let default_attrs = Attrs::new().family(Family::Name(&family));

        if self.status_runs.as_ref() != Some(&right) {
            let rich = right
                .iter()
                .map(|(text, color)| (text.as_str(), default_attrs.clone().color(*color)));
            self.status_buffer.set_rich_text(
                &mut self.font_system,
                rich,
                &default_attrs,
                Shaping::Advanced,
                None,
            );
            self.status_buffer
                .shape_until_scroll(&mut self.font_system, false);
            self.status_runs = Some(right);
        }
        if self.status_left_runs.as_ref() != Some(&left) {
            let rich = left
                .iter()
                .map(|(text, color)| (text.as_str(), default_attrs.clone().color(*color)));
            self.status_left_buffer.set_rich_text(
                &mut self.font_system,
                rich,
                &default_attrs,
                Shaping::Advanced,
                None,
            );
            self.status_left_buffer
                .shape_until_scroll(&mut self.font_system, false);
            self.status_left_runs = Some(left);
        }
    }

    /// The status band's background quad (Q#S2): a full-width strip
    /// under the band text.
    fn status_band_vertex_bytes(&self) -> Vec<u8> {
        // Themes Q#TH5: a set ui.modeline face owns the band surface.
        let color = self
            .modeline_face_colors()
            .map_or(STATUS_BAND_BG, |(quad, _)| quad);
        let rect = MinimapRect {
            x: 0.0,
            y: status_band_top(self.config.height, self.fm),
            w: self.config.width as f32,
            h: self.fm.status_band_height(),
            color,
        };
        rects_to_vertex_bytes(&[rect], self.config.width, self.config.height)
    }

    /// Re-shape the menu label text from `self.menu` (Q#CM1), one line
    /// per row (separators are blank lines so rows stay aligned with the
    /// bg quads). A no-op string when the menu is closed.
    fn refresh_menu_buffer(&mut self) {
        let text = self.menu.as_ref().map_or_else(String::new, |menu| {
            menu.rows
                .iter()
                .map(|r| if r.separator { "" } else { r.label.as_str() })
                .collect::<Vec<_>>()
                .join("\n")
        });
        let family = self.resolved_family.clone();
        self.menu_buffer.set_text(
            &mut self.font_system,
            &text,
            &Attrs::new().family(Family::Name(&family)),
            Shaping::Advanced,
            None,
        );
        self.menu_buffer
            .shape_until_scroll(&mut self.font_system, false);
    }

    /// Popup background, active-row highlight, and separator quads
    /// (Q#CM1). Empty when the menu is closed.
    fn menu_vertex_bytes(&self) -> Vec<u8> {
        let Some(menu) = self.menu.as_ref() else {
            return Vec::new();
        };
        let ax = menu.anchor_px.0 as f32;
        let ay = menu.anchor_px.1 as f32;
        let w = Self::menu_width_px(menu, self.fm);
        let mut rects = vec![MinimapRect {
            x: ax,
            y: ay,
            w,
            h: menu.rows.len() as f32 * self.fm.menu_row_height(),
            color: MENU_BG,
        }];
        for (i, row) in menu.rows.iter().enumerate() {
            let ry = ay + i as f32 * self.fm.menu_row_height();
            if row.separator {
                rects.push(MinimapRect {
                    x: ax + MENU_PAD_X,
                    y: ry + self.fm.menu_row_height() / 2.0 - 0.5,
                    w: w - 2.0 * MENU_PAD_X,
                    h: 1.0,
                    color: MENU_SEPARATOR_BG,
                });
            } else if menu.active == Some(i as u32) {
                rects.push(MinimapRect {
                    x: ax,
                    y: ry,
                    w,
                    h: self.fm.menu_row_height(),
                    color: MENU_SELECTED_BG,
                });
            }
        }
        rects_to_vertex_bytes(&rects, self.config.width, self.config.height)
    }

    /// Re-shape the minibuffer dropdown candidates (Q#MB1), one line per
    /// candidate, best match first. Empty when there are no candidates.
    ///
    /// Discovery Stage 2: a row with a `detail` renders `label  detail`,
    /// the same two-space form the completion dropdown already uses. A
    /// row without one renders the bare label, so a file-path or
    /// buffer-name prompt looks exactly as it did before v23.
    fn refresh_mb_buffer(&mut self) {
        let text = self.minibuffer.as_ref().map_or_else(String::new, |mb| {
            mb.rows
                .iter()
                .map(|row| match row.detail.as_deref() {
                    Some(detail) => format!("{}  {detail}", row.label),
                    None => row.label.clone(),
                })
                .collect::<Vec<_>>()
                .join("\n")
        });
        let family = self.resolved_family.clone();
        self.mb_buffer.set_text(
            &mut self.font_system,
            &text,
            &Attrs::new().family(Family::Name(&family)),
            Shaping::Advanced,
            None,
        );
        self.mb_buffer
            .shape_until_scroll(&mut self.font_system, false);
    }

    /// The visible slice `(first, count)` of the dropdown candidates
    /// (audit F-007), clamped to the rows that fit above the band and
    /// scrolled to keep the selection on screen. `None` when closed,
    /// candidate-free, or too short for a row. See [`mb_dropdown_window`].
    fn mb_visible_window(&self) -> Option<(usize, usize)> {
        let mb = self.minibuffer.as_ref()?;
        let band_top = status_band_top(self.config.height, self.fm);
        mb_dropdown_window(
            mb.rows.len(),
            mb.selected.map_or(0, |s| s as usize),
            band_top,
            self.fm,
        )
    }

    /// Dropdown geometry `(left, top_y, width)` when the minibuffer has
    /// candidates: a list anchored just above the bottom band, growing
    /// upward, as wide as the widest candidate (clamped). `None` when
    /// closed or candidate-free. The height is the *visible* row count
    /// (F-007), so `top_y` never goes above the window top.
    /// `refresh_mb_buffer` must have run so the width measurement is
    /// current.
    fn mb_dropdown_rect(&self) -> Option<(f32, f32, f32)> {
        let (_first, count) = self.mb_visible_window()?;
        let widest = self
            .mb_buffer
            .layout_runs()
            .map(|r| r.line_w)
            .fold(0.0_f32, f32::max);
        let width = (widest + 2.0 * MB_DROP_PAD_X).clamp(MB_DROP_MIN_WIDTH, MB_DROP_MAX_WIDTH);
        let band_top = status_band_top(self.config.height, self.fm);
        let top_y = band_top - count as f32 * self.fm.mb_drop_row_height();
        Some((STATUS_TEXT_PAD, top_y, width))
    }

    /// Minibuffer dropdown background + selection-highlight quads (Q#MB1).
    /// Empty when closed / candidate-free.
    fn mb_dropdown_vertex_bytes(&self) -> Vec<u8> {
        let Some(mb) = self.minibuffer.as_ref() else {
            return Vec::new();
        };
        let Some((first, count)) = self.mb_visible_window() else {
            return Vec::new();
        };
        let Some((x, top_y, width)) = self.mb_dropdown_rect() else {
            return Vec::new();
        };
        let mut rects = vec![MinimapRect {
            x,
            y: top_y,
            w: width,
            h: count as f32 * self.fm.mb_drop_row_height(),
            color: MENU_BG,
        }];
        // Highlight the selection at its row *within the visible window*;
        // by construction it always falls inside [first, first + count).
        if let Some(sel) = mb.selected.map(|s| s as usize)
            && sel >= first
            && sel < first + count
        {
            rects.push(MinimapRect {
                x,
                y: top_y + (sel - first) as f32 * self.fm.mb_drop_row_height(),
                w: width,
                h: self.fm.mb_drop_row_height(),
                color: MENU_SELECTED_BG,
            });
        }
        rects_to_vertex_bytes(&rects, self.config.width, self.config.height)
    }

    /// Re-shape the completion dropdown rows (Arc 1a Q#C5), one line
    /// per candidate: kind glyph, label, then the dimmable detail.
    /// Empty when the popup is closed.
    fn refresh_completion_buffer(&mut self) {
        let text = self.completion.as_ref().map_or_else(String::new, |comp| {
            comp.rows
                .iter()
                .map(|row| {
                    let glyph = completion_kind_glyph(row.kind);
                    match row.detail.as_deref() {
                        Some(detail) => format!("{glyph} {}  {detail}", row.label),
                        None => format!("{glyph} {}", row.label),
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        });
        let family = self.resolved_family.clone();
        self.completion_buffer.set_text(
            &mut self.font_system,
            &text,
            &Attrs::new().family(Family::Name(&family)),
            Shaping::Advanced,
            None,
        );
        self.completion_buffer
            .shape_until_scroll(&mut self.font_system, false);
    }

    /// The pixel position of the completion popup's byte anchor:
    /// `(x, line_top_y, line_height)` of the glyph the anchor sits
    /// before — the caret mapping (`caret_rect`) reused for a second
    /// byte. `None` when the popup is closed or the anchor is
    /// scrolled out of the visible slice (the popup then simply
    /// doesn't draw this frame; scrolling back restores it).
    fn completion_anchor_px(&mut self) -> Option<(f32, f32, f32)> {
        if !self.completion_open_for_current_buffer() {
            return None; // never paint against a foreign buffer's rope
        }
        let anchor = self.completion.as_ref()?.anchor;
        let (vstart, vend) = self.view_range;
        if vend <= vstart || anchor < vstart || anchor > vend {
            return None;
        }
        // The caret mapping, visual-run aware (framing Q#F6): the
        // anchor's run, not its source line's first run. Off the
        // drawable window (a wrapped run below the band, or above a
        // caret-follow residual) counts as scrolled out.
        let (x, top, line_height) = self.code_byte_px(anchor)?;
        let y = TEXT_TOP + top;
        let bottom = document_text_bottom(self.config.height, self.fm, self.band_inset());
        if y >= bottom || y + line_height <= TEXT_TOP {
            return None;
        }
        // Stage 5: an anchor scrolled off the LEFT is out of view the
        // same way one below the band is. Returning `None` HIDES the
        // popup — it does not close it. The daemon owns completion
        // state and its key handling; closure is `CompletionPopup {
        // anchor: None }`, which is the daemon's to send. Scrolling
        // back brings the popup straight back, and a viewport lane must
        // not quietly redefine when a completion ends.
        //
        // **A POINT predicate, not `survives_code_clip_left`.** An
        // anchor is a position between glyphs — it has no horizontal
        // extent of its own, and the popup it places is drawn to its
        // RIGHT. The first version of this passed `line_height` as the
        // width, which is a vertical dimension standing in for a
        // horizontal one: an anchor up to a line-height left of the
        // gutter then "survived", and `completion_dropdown_rect` has no
        // left clamp (it bounds `ax` against the right margin only), so
        // that x reached the popup's left edge and painted over the
        // line numbers.
        //
        // That absent clamp stays absent deliberately. This predicate
        // is what guarantees `ax >= code_clip_left()`, and a second
        // clamp downstream would be a duplicate of the same rule — the
        // failure mode this stage's whole shared-transform design
        // exists to avoid. `completion_dropdown_rect` is witnessed
        // against it instead.
        let screen_x = self.code_x_to_screen(x);
        if screen_x < self.code_clip_left() {
            return None;
        }
        Some((screen_x, y, line_height))
    }

    /// Layout of the completion dropdown: `(first_row, row_count,
    /// left_x, top_y)`. Anchored on the row *below* the anchor's line
    /// (growing downward toward the status band); flips above when
    /// nothing fits below — the TUI overlay's placement rule. The
    /// visible slice windows around the selection so it stays on
    /// screen when fewer rows fit than the wire shipped (the F-007
    /// discipline).
    fn completion_dropdown_layout(&mut self) -> Option<(usize, usize, f32, f32)> {
        let comp = self.completion.as_ref()?;
        let n = comp.rows.len();
        if n == 0 {
            return None;
        }
        let sel = comp.selected.map_or(0, |s| s as usize);
        let (ax, line_top, line_h) = self.completion_anchor_px()?;
        let band_top = document_text_bottom(self.config.height, self.fm, self.band_inset());
        let below_px = band_top - (line_top + line_h);
        let above_px = line_top - TEXT_TOP;
        let max_below = (below_px / self.fm.mb_drop_row_height()).floor() as usize;
        let max_above = (above_px / self.fm.mb_drop_row_height()).floor() as usize;
        let (avail, below) = if max_below >= 1 {
            (max_below, true)
        } else {
            (max_above, false)
        };
        if avail == 0 {
            return None;
        }
        let count = n.min(avail);
        let first = if n <= count {
            0
        } else {
            sel.saturating_sub(count / 2).min(n - count)
        };
        let top_y = if below {
            line_top + line_h
        } else {
            line_top - count as f32 * self.fm.mb_drop_row_height()
        };
        Some((first, count, ax, top_y))
    }

    /// Dropdown geometry `(left, top_y, width)`: as wide as the widest
    /// row (clamped, the minibuffer bounds), left edge at the anchor
    /// column shifted back from the window's right margin.
    /// `refresh_completion_buffer` must have run so the width
    /// measurement is current.
    fn completion_dropdown_rect(&mut self) -> Option<(f32, f32, f32)> {
        let (_first, _count, ax, top_y) = self.completion_dropdown_layout()?;
        let widest = self
            .completion_buffer
            .layout_runs()
            .map(|r| r.line_w)
            .fold(0.0_f32, f32::max);
        let width = (widest + 2.0 * MB_DROP_PAD_X).clamp(MB_DROP_MIN_WIDTH, MB_DROP_MAX_WIDTH);
        let left = ax.min((self.config.width as f32 - width).max(0.0));
        Some((left, top_y, width))
    }

    /// Completion dropdown background + selection-highlight quads.
    /// Empty when closed or the anchor is off-screen.
    fn completion_dropdown_vertex_bytes(&mut self) -> Vec<u8> {
        let Some(selected) = self.completion.as_ref().map(|c| c.selected) else {
            return Vec::new();
        };
        let Some((first, count, _ax, _ty)) = self.completion_dropdown_layout() else {
            return Vec::new();
        };
        let Some((x, top_y, width)) = self.completion_dropdown_rect() else {
            return Vec::new();
        };
        let mut rects = vec![MinimapRect {
            x,
            y: top_y,
            w: width,
            h: count as f32 * self.fm.mb_drop_row_height(),
            color: MENU_BG,
        }];
        if let Some(sel) = selected.map(|s| s as usize)
            && sel >= first
            && sel < first + count
        {
            rects.push(MinimapRect {
                x,
                y: top_y + (sel - first) as f32 * self.fm.mb_drop_row_height(),
                w: width,
                h: self.fm.mb_drop_row_height(),
                color: MENU_SELECTED_BG,
            });
        }
        rects_to_vertex_bytes(&rects, self.config.width, self.config.height)
    }

    /// Bookkeeping for an outgoing Pointer event: it supersedes any
    /// unconfirmed optimistic-cursor prediction (the daemon's answer
    /// will be the click position, not the typing prediction), and
    /// the cursor is not authoritative again until that `CursorByte`
    /// lands.
    fn note_pointer_round_trip(&mut self) {
        self.cursor_fresh = false;
        self.optimistic_cursor_floor = None;
        self.optimistic_floor_set_at = None;
    }

    /// Frontend-side multi-click detection: a second Down at the
    /// same hit byte within the interval upgrades to `DoubleDown`,
    /// a third to `TripleDown` (Q#M4); a fourth restarts the chain.
    fn classify_pointer_down(&mut self, byte: u64, shift: bool) -> PointerKind {
        if shift {
            // Shift-click extends the selection (Q#M5); it neither
            // advances nor inherits the multi-click chain — two
            // Shift-clicks must not become a word select.
            self.last_pointer_down = None;
            return PointerKind::Down;
        }
        let now = std::time::Instant::now();
        let prior_chain = self
            .last_pointer_down
            .take()
            .and_then(|(at, prev, count)| {
                (prev == byte && now.duration_since(at) <= DOUBLE_CLICK_WINDOW).then_some(count)
            })
            .unwrap_or(0);
        match prior_chain {
            0 => {
                self.last_pointer_down = Some((now, byte, 1));
                PointerKind::Down
            }
            1 => {
                self.last_pointer_down = Some((now, byte, 2));
                PointerKind::DoubleDown
            }
            _ => {
                // Chain consumed: a fourth click starts over.
                PointerKind::TripleDown
            }
        }
    }

    fn apply_file_style_summary(
        &mut self,
        buffer_id: BufferId,
        generation: u64,
        lines: Vec<CellStyle>,
    ) -> Option<ViewportSend> {
        if self.current_buffer_id != Some(buffer_id) {
            return None;
        }
        if self
            .current_summary
            .as_ref()
            .is_some_and(|summary| generation < summary.generation)
        {
            return None;
        }
        let caret_was_painted = self.caret_painted_in_code_clip();
        self.current_line_shapes = minimap_line_shapes(&self.current_text);
        self.current_summary = Some(FileStyleSummaryState { generation, lines });
        // PR #120 round 1 finding 1: a newly accepted summary can
        // arrive at an UNCHANGED generation (theme recolor,
        // diagnostic republish) and the minimap cache keys only on
        // (generation, dims, scroll) — without an explicit drop the
        // stale vertices survive until an edit, resize, or scroll.
        // The daemon payload-suppresses identical summaries, so every
        // summary accepted here is genuinely new and the invalidation
        // is precise.
        self.minimap_cache = None;
        self.reflow_dynamic_code_geometry(caret_was_painted)
    }

    /// `full = true` path: discard prior styling, take the segments'
    /// spans as authoritative for the declared viewport.
    fn replace_style_spans(&mut self, segments: Vec<StyleSegment>) {
        self.current_spans = spans_from_segments(segments);
    }

    /// `full = false` path: each segment's `range` authoritatively
    /// replaces styling within it. Spans fully inside any dirty range
    /// drop; spans straddling a dirty edge get clipped to outside the
    /// range; the new spans are appended; finally everything sorts.
    ///
    /// This is exactly the surface bet #1 from the framing pass
    /// predicted ("dirty-segment edges at viewport boundaries —
    /// headless-test-blind-spot probe"). Per-byte adversarial
    /// behavior here lives in the user-side validation, not in unit
    /// tests — that's the design-doc framing's whole point.
    fn merge_style_spans(&mut self, segments: Vec<StyleSegment>) {
        for seg in &segments {
            let dirty = seg.range;
            let mut kept = Vec::with_capacity(self.current_spans.len());
            for sp in self.current_spans.drain(..) {
                if sp.range.end <= dirty.start || sp.range.start >= dirty.end {
                    // Outside the dirty range entirely — keep as-is.
                    kept.push(sp);
                } else if sp.range.start < dirty.start && sp.range.end > dirty.end {
                    // Straddles both edges: split into two clipped halves.
                    kept.push(StyleSpan {
                        range: ByteRange {
                            start: sp.range.start,
                            end: dirty.start,
                        },
                        style: sp.style,
                    });
                    kept.push(StyleSpan {
                        range: ByteRange {
                            start: dirty.end,
                            end: sp.range.end,
                        },
                        style: sp.style,
                    });
                } else if sp.range.start < dirty.start {
                    // Straddles the left edge only — clip to the left.
                    kept.push(StyleSpan {
                        range: ByteRange {
                            start: sp.range.start,
                            end: dirty.start,
                        },
                        style: sp.style,
                    });
                } else if sp.range.end > dirty.end {
                    // Straddles the right edge only — clip to the right.
                    kept.push(StyleSpan {
                        range: ByteRange {
                            start: dirty.end,
                            end: sp.range.end,
                        },
                        style: sp.style,
                    });
                }
                // else: fully inside the dirty range ⇒ drop.
            }
            self.current_spans = kept;
        }
        for seg in segments {
            self.current_spans.extend(seg.spans);
        }
        self.current_spans.sort_by_key(|s| s.range.start);
    }

    /// `Decorations { full: true, .. }` path — exactly the
    /// `replace_style_spans` shape for decorations. The wire structure
    /// is intentionally symmetric (`DecorationSegment` ↔ `StyleSegment`).
    fn replace_decorations(&mut self, segments: Vec<DecorationSegment>) {
        self.current_decorations.clear();
        for seg in segments {
            self.current_decorations.extend(seg.decorations);
        }
        self.current_decorations.sort_by_key(|d| d.range.start);
    }

    /// `Decorations { full: false, .. }` path — M11.4 dirty-merge for
    /// decorations. Structurally identical to [`Self::merge_style_spans`]
    /// — same edge-clip/drop/split logic, same trailing append +
    /// re-sort.
    ///
    /// **Recorded session-5 finding (rule iii, deferred):** this
    /// duplication of the M11.4 merge algorithm across two
    /// `(range, T)`-shaped types invites a generic
    /// `merge_dirty_segments<T: HasRange>` helper. The refactor is
    /// minor in lines but touches a load-bearing invariant; deferring
    /// until at least a third instance arrives (e.g. peer-cursor
    /// decorations from `PresenceUpdate`) so the abstraction is
    /// inducted from three points rather than two.
    fn merge_decorations(&mut self, segments: Vec<DecorationSegment>) {
        for seg in &segments {
            let dirty = seg.range;
            let mut kept = Vec::with_capacity(self.current_decorations.len());
            for d in self.current_decorations.drain(..) {
                if d.range.end <= dirty.start || d.range.start >= dirty.end {
                    kept.push(d);
                } else if d.range.start < dirty.start && d.range.end > dirty.end {
                    kept.push(Decoration {
                        range: ByteRange {
                            start: d.range.start,
                            end: dirty.start,
                        },
                        kind: d.kind,
                    });
                    kept.push(Decoration {
                        range: ByteRange {
                            start: dirty.end,
                            end: d.range.end,
                        },
                        kind: d.kind,
                    });
                } else if d.range.start < dirty.start {
                    kept.push(Decoration {
                        range: ByteRange {
                            start: d.range.start,
                            end: dirty.start,
                        },
                        kind: d.kind,
                    });
                } else if d.range.end > dirty.end {
                    kept.push(Decoration {
                        range: ByteRange {
                            start: dirty.end,
                            end: d.range.end,
                        },
                        kind: d.kind,
                    });
                }
            }
            self.current_decorations = kept;
        }
        for seg in segments {
            self.current_decorations.extend(seg.decorations);
        }
        self.current_decorations.sort_by_key(|d| d.range.start);
    }

    /// Re-build the cosmic-text Buffer from `current_text` +
    /// `current_spans` + `current_decorations` +
    /// `current_adornments`. Source styling/decorations remain
    /// byte-indexed into `current_text`; adornments contribute extra
    /// rich-text chunks at their anchors without mutating the source
    /// string. That display projection is the central session-6
    /// invariant: virtual text must not shift the source-byte ranges
    /// used by `StyleSpans` / `Decorations`.
    ///
    /// Complexity is O(B × (S + D)) per reshape where B is the boundary
    /// count and S+D is spans+decorations. For viewport-scoped data
    /// this is bounded by visible bytes. A sweep-line refactor with
    /// active-set pointers is the obvious upgrade if reshape cost
    /// surfaces in profile data — recorded but not done in session 5.
    /// Whole-file byte range `[vstart, vend)` of the source lines that
    /// should be shaped: from `scroll_top` through the visible window
    /// plus a small overscan (Q#S1/S3). Both ends fall on line
    /// boundaries (cosmic-text splits `BufferLine`s on `\n`, so a
    /// mid-line slice would corrupt the first/last line).
    fn visible_byte_range(&self) -> (u64, u64) {
        let line_starts = &self.current_line_starts;
        let n = line_starts.len();
        let top = self.scroll_top.min(n.saturating_sub(1));
        let span = estimated_visible_lines(self.config.height, self.fm, self.band_inset()).max(1)
            + SCROLL_OVERSCAN;
        let vstart = line_starts[top];
        let bottom = top.saturating_add(span).min(n);
        let vend = if bottom < n {
            line_starts[bottom]
        } else {
            self.current_text.len() as u64
        };
        (vstart, vend)
    }

    /// Rebuild the shaped slice from `scroll_top` and reapply the
    /// retained normalized scroll (framing Q#F6) — the raw builder
    /// shared by [`Self::reshape`] and the [`Self::normalize_code_scroll`]
    /// fold loop. Callers that end a "final code shape" must run the
    /// normalizer after so the slice-local `line == 0` invariant is
    /// re-established.
    fn rebuild_code_slice(&mut self) {
        // Session S1 — shape only the visible byte slice. Feeding the
        // whole rope to `set_rich_text` (a BufferLine per source line)
        // made large-file editing O(file) per keystroke; cosmic-text
        // touches only `current_text[vstart..vend]` now. Spans /
        // decorations / adornments arrive in whole-file coordinates and
        // are clipped + rebased onto the slice (subtract `vstart`).
        let (vstart, vend) = self.visible_byte_range();
        self.view_range = (vstart, vend);
        let (top, ranges) = self.slice_line_ranges(vstart, vend);
        let mut lines = Vec::with_capacity(ranges.len());
        let mut cache = Vec::with_capacity(ranges.len());
        let mut math = Vec::with_capacity(ranges.len());
        for &(ls, ce) in &ranges {
            let (chunks, m) = self.chunks_for_line(ls, ce);
            lines.push(line_from_chunks(&chunks, &self.resolved_family));
            cache.push(chunks);
            math.push(m);
        }
        self.buffer.lines = lines;
        self.line_chunk_cache = cache;
        self.line_math_cache = math;
        self.shaped_top = top;
        self.buffer
            .set_scroll(Scroll::new(0, self.code_scroll_residual, 0.0));
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        // The pointer hit map rebuilds lazily from the same caches
        // (Q#R2) — clicks are rare next to keystrokes/frames.
        self.hit_map_dirty = true;
    }

    /// Re-establish the normalized code-scroll invariant after a
    /// final code shape (framing Q#F6): cosmic-text advances the
    /// slice-local `scroll.line` when new wrapping/metrics push the
    /// retained vertical residual across source lines. Fold that
    /// delta into the whole-file `scroll_top`, retain the residual,
    /// rebuild from the new source origin, and repeat until
    /// `line == 0`. Every iteration strictly advances the clamped
    /// source origin; a non-advancing or past-EOF fold clamps to the
    /// last source line with a default scroll instead of looping.
    /// `scroll.horizontal` is discarded throughout — glyphon 0.11
    /// never applies it when placing glyphs, so retaining it would
    /// claim a scroll the painter never displays.
    fn normalize_code_scroll(&mut self) {
        loop {
            let scroll = self.buffer.scroll();
            if scroll.horizontal != 0.0 {
                self.buffer
                    .set_scroll(Scroll::new(scroll.line, scroll.vertical, 0.0));
            }
            if scroll.line == 0 {
                self.code_scroll_residual = scroll.vertical.max(0.0);
                return;
            }
            let last_line = self.current_line_starts.len().saturating_sub(1);
            let new_top = self.shaped_top + scroll.line;
            if new_top > self.shaped_top && new_top <= last_line {
                self.scroll_top = new_top;
                self.code_scroll_residual = scroll.vertical;
                self.rebuild_code_slice();
            } else {
                self.scroll_top = last_line;
                self.code_scroll_residual = 0.0;
                self.rebuild_code_slice();
                return;
            }
        }
    }

    /// Honor a wrap mode for `buffer_id` (protocol v22).
    ///
    /// The document buffer has never set a wrap mode, so it has been
    /// running on cosmic-text's constructor default,
    /// `Wrap::WordOrGlyph` — word wrap that nobody chose. This makes the
    /// mode explicit in both directions and settles it on
    /// **`Wrap::Glyph`**: character wrap is what the grid renderer can
    /// implement identically without pulling UAX #14 into it, and it is
    /// what Emacs does by default. GUI users lose word wrap; that is a
    /// deliberate, documented trade for the two frontends agreeing.
    ///
    /// Changing wrap reflows the whole document, exactly like a font
    /// change, so the retained scroll anchor is repaired through
    /// `normalize_code_scroll` rather than left pointing at a row that
    /// no longer exists.
    fn apply_line_wrap(&mut self, buffer_id: BufferId, wrap: bool) {
        // Only the buffer on screen can be reflowed; the mode is
        // buffer-local, so a message for anything else is not ours to
        // apply. The daemon resends on buffer switch precisely so this
        // stays correct rather than needing a per-buffer cache here.
        if self.current_buffer_id != Some(buffer_id) {
            return;
        }
        let want = if wrap { Wrap::Glyph } else { Wrap::None };
        // Compare against the BUFFER, not a shadow field. A cached copy
        // can disagree with what cosmic-text actually holds — and when
        // it does, the short-circuit turns a real mode change into a
        // silent no-op. Reading the authority cannot drift from it.
        if self.buffer.wrap() == want {
            return;
        }
        self.buffer.set_wrap(&mut self.font_system, want);
        // Stage 5 (Q#G2): RESET, not merely ignore. A wrapped buffer has
        // nothing past the right edge, so an offset is meaningless here
        // — but leaving it parked would surface a stale viewport the
        // instant the buffer toggled back to `truncate`, before any
        // cursor motion. The TUI's `horizontal_follow` zeroes it on the
        // same branch for the same reason.
        self.code_scroll_left = 0.0;
        self.reshape();
    }

    fn reshape(&mut self) {
        self.rebuild_code_slice();
        self.normalize_code_scroll();
        // Full restyle: release any held post-jump frame (Q#M6).
        self.styled_redraw_deadline = None;
        self.request_redraw();
    }

    /// Ask the window to repaint. A no-op headless (no window), where the
    /// render tests drive `render_offscreen` directly (F-014).
    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// One dimension helper for ALL seven buffers (framing Q#F6):
    /// metrics and the current REAL drawable dimensions change
    /// atomically via `set_metrics_and_size` — `set_metrics` alone
    /// deliberately preserves old dimensions, and the old `resize()`
    /// only touched four of the seven buffers (the "numbers stop at
    /// 10" class of skew). Code and gutter get the drawable code
    /// clip's height (and code its clip width, so wrapping and
    /// `shape_until_cursor` use the same clip the painter uses); the
    /// status pair gets the derived band; the row popups get the
    /// surface height — their protocols window rows themselves.
    /// `set_metrics_and_size` no-ops when nothing changed, so calling
    /// this eagerly is cheap. Returns whether the code buffer's
    /// metrics or drawable dimensions changed and therefore require
    /// reshaping before its next frame.
    fn sync_buffer_dimensions(&mut self) -> bool {
        let fm = self.fm;
        let width = self.config.width as f32;
        let height = self.config.height as f32;
        let code_metrics = Metrics::new(fm.code_font_size(), fm.code_line_height());
        let code_width = (self.text_bounds_right() as f32 - self.text_left()).max(0.0);
        let code_height =
            (document_text_bottom(self.config.height, fm, self.band_inset()) - TEXT_TOP).max(0.0);
        let code_layout_changed = self.buffer.metrics() != code_metrics
            || self.buffer.size() != (Some(code_width), Some(code_height));
        self.buffer.set_metrics_and_size(
            &mut self.font_system,
            code_metrics,
            Some(code_width),
            Some(code_height),
        );
        self.gutter_buffer.set_metrics_and_size(
            &mut self.font_system,
            code_metrics,
            Some(width),
            Some(code_height),
        );
        let status_metrics = Metrics::new(fm.status_font_size(), fm.status_line_height());
        self.status_buffer.set_metrics_and_size(
            &mut self.font_system,
            status_metrics,
            Some(width),
            Some(fm.status_band_height()),
        );
        self.status_left_buffer.set_metrics_and_size(
            &mut self.font_system,
            status_metrics,
            Some(width),
            Some(fm.status_band_height()),
        );
        self.menu_buffer.set_metrics_and_size(
            &mut self.font_system,
            Metrics::new(fm.menu_font_size(), fm.menu_line_height()),
            Some(MENU_MAX_WIDTH),
            Some(height),
        );
        let drop_metrics = Metrics::new(fm.mb_drop_font_size(), fm.mb_drop_line_height());
        self.mb_buffer.set_metrics_and_size(
            &mut self.font_system,
            drop_metrics,
            Some(MB_DROP_MAX_WIDTH),
            Some(height),
        );
        self.completion_buffer.set_metrics_and_size(
            &mut self.font_system,
            drop_metrics,
            Some(MB_DROP_MAX_WIDTH),
            Some(height),
        );
        code_layout_changed
    }

    /// Reflow after a dynamic painter-geometry input changes: gutter
    /// mode/digit width or minimap presence. The caller captures
    /// `caret_was_painted` against the old geometry before mutating
    /// that input. Resize the buffer first, shape once at the final
    /// clip, normalize the visual residual, and re-follow only a caret
    /// that was actually painted. A viewport is returned only when
    /// that settling changed the source range.
    fn reflow_dynamic_code_geometry(&mut self, caret_was_painted: bool) -> Option<ViewportSend> {
        if self.sync_buffer_dimensions() {
            self.reshape();
            if caret_was_painted {
                self.ensure_caret_painted();
            }
        } else {
            self.request_redraw();
        }
        self.current_buffer_id
            .and_then(|buffer_id| self.viewport_send_if_changed(buffer_id))
    }

    /// Whether every face the shipped attribute set can select for
    /// `family` is monospaced (framing Q#F6): the four queries
    /// reachable through the base `Attrs` — normal, bold, italic, and
    /// bold-italic, all at normal stretch. fontdb's style matching
    /// would otherwise let a closer-weight proportional sibling win
    /// bold/italic text even when the normal query resolved a valid
    /// monospaced face, silently under-sizing gutter and menu
    /// advance geometry.
    fn family_is_monospace_everywhere(&self, family: &str) -> bool {
        let db = self.font_system.db();
        [
            (fontdb::Weight::NORMAL, fontdb::Style::Normal),
            (fontdb::Weight::BOLD, fontdb::Style::Normal),
            (fontdb::Weight::NORMAL, fontdb::Style::Italic),
            (fontdb::Weight::BOLD, fontdb::Style::Italic),
        ]
        .into_iter()
        .all(|(weight, style)| {
            db.query(&fontdb::Query {
                families: &[fontdb::Family::Name(family)],
                weight,
                stretch: fontdb::Stretch::Normal,
                style,
            })
            .and_then(|id| db.face(id))
            .is_some_and(|face| face.monospaced)
        })
    }

    /// Apply a `FontFacts` preference wholesale — framing Q#F6's one
    /// transaction. Fail-closed wire validation first (this is
    /// deserialized protocol input; the daemon-side Lua check is a UX
    /// courtesy, not a trust boundary), then: record the actual
    /// painted-caret decision, resolve the family (four-style
    /// monospace gate, total fallback to the sanitized default),
    /// derive metrics + the measured advance, re-metric/re-size all
    /// seven buffers, drop the string-equality status caches, reshape
    /// at the retained scroll, settle the drawable width, re-follow a
    /// formerly painted caret (or only re-normalize an intentionally
    /// caret-free viewport), and invalidate dependent layout. No
    /// frame is submitted between the passes, and no atlas action is
    /// needed — the per-frame `atlas.trim()` clears `glyphs_in_use`,
    /// making old-font glyphs eligible for later LRU-style eviction
    /// under allocation pressure.
    fn apply_font_facts(&mut self, family: Option<&str>, size_centi_px: Option<u32>) {
        if let Some(size) = size_centi_px
            && !FONT_SIZE_CENTI_PX_RANGE.contains(&size)
        {
            // 0 would panic `Buffer::set_metrics`; huge values produce
            // pathological metrics/allocations. Reject the WHOLE
            // message: current state kept, nothing re-shaped.
            eprintln!(
                "pmacs-gpu: ignoring FontFacts with out-of-range size {size} \
                 (allowed {}..={} hundredths of a logical px)",
                FONT_SIZE_CENTI_PX_RANGE.start(),
                FONT_SIZE_CENTI_PX_RANGE.end(),
            );
            return;
        }
        let caret_was_painted = self.caret_painted_in_code_clip();
        let resolved = match family {
            None => self.font_defaults.default_family.clone(),
            Some(requested) if self.family_is_monospace_everywhere(requested) => {
                requested.to_owned()
            }
            Some(requested) => {
                // Deterministic frontend fallback, never round-tripped
                // back — the daemon never learns resolution outcomes.
                eprintln!(
                    "pmacs-gpu: font family {requested:?} is unavailable or not \
                     monospaced across its normal/bold/italic/bold-italic \
                     queries; falling back to {:?}",
                    self.font_defaults.default_family
                );
                self.font_defaults.default_family.clone()
            }
        };
        #[allow(clippy::cast_precision_loss)] // size <= 7200 is exact in f32
        let scale = size_centi_px.map_or(1.0, |size| size as f32 / 100.0 / BASE_CODE_FONT_SIZE);
        // The measure pass (framing Q#F6): a fixed ASCII probe in the
        // resolved family at the new code metrics, independent of
        // document contents. The NORMAL-face advance becomes
        // authoritative for gutter geometry, and the selected/default
        // ratio scales the const-based fallbacks — the default family
        // is ratio 1 by construction, so never-set/reset stays
        // byte-identical.
        let code_metrics = Metrics::new(BASE_CODE_FONT_SIZE * scale, BASE_CODE_LINE_HEIGHT * scale);
        let selected_advance = probe_mono_advance(&mut self.font_system, &resolved, code_metrics);
        let resolved_is_default = resolved == self.font_defaults.default_family;
        let advance_ratio = if resolved_is_default {
            1.0
        } else {
            let default_advance = probe_mono_advance(
                &mut self.font_system,
                &self.font_defaults.default_family,
                code_metrics,
            );
            match (selected_advance, default_advance) {
                (Some(selected), Some(default)) if selected > 0.0 && default > 0.0 => {
                    selected / default
                }
                _ => 1.0,
            }
        };
        self.resolved_family = resolved;
        self.fm = FontMetrics {
            scale,
            advance_ratio,
        };
        // The Q#MS10 fit budget follows the code metrics; the reshape
        // below rebuilds every line's math plan against it.
        self.math_budget = math_code_budget(self.fm);
        // The default family already has an exact, pre-preference
        // geometry path: a shaped glyph when present, otherwise the
        // ratio-scaled baseline constant. Keep using it so resetting
        // to `(None, None)` is bit-identical to never-set; averaging a
        // ten-cell f32 run can differ by one ulp. Alternate families
        // need the measured normal-face advance because their cell
        // width is not encoded in the baseline.
        self.measured_mono_advance = if resolved_is_default {
            None
        } else {
            selected_advance
        };
        // Every row-oriented surface stays one row across the metric
        // transaction, including the two status buffers (Q#SL10).
        self.status_buffer
            .set_wrap(&mut self.font_system, Wrap::None);
        self.status_left_buffer
            .set_wrap(&mut self.font_system, Wrap::None);
        self.menu_buffer.set_wrap(&mut self.font_system, Wrap::None);
        self.mb_buffer.set_wrap(&mut self.font_system, Wrap::None);
        self.completion_buffer
            .set_wrap(&mut self.font_system, Wrap::None);
        // Metrics + current dimensions atomically on all seven.
        self.sync_buffer_dimensions();
        // Colors and family are attrs embedded in the status buffers.
        // `None` forces the next frame to install and shape rich runs.
        self.status_runs = None;
        self.status_left_runs = None;
        // Attrs-bearing reshape at the retained scroll (reshape
        // normalizes it against the FINAL family/metrics/dims).
        self.reshape();
        // Settle the drawable code width: `text_left` depends on the
        // measured advance via the gutter, so re-derive and reshape
        // once more if it moved. No frame is submitted between passes.
        let width_before = self.buffer.size().0;
        self.sync_buffer_dimensions();
        if self.buffer.size().0 != width_before {
            self.reshape();
        }
        if caret_was_painted {
            self.ensure_caret_painted();
        } else {
            // Preserve the user's scroll — a font change must never
            // turn an overscan-only caret into a snap-back.
            self.normalize_code_scroll();
        }
        // Dependent layout: the minimap vertex cache keys on
        // (generation, size, scroll_top) and would miss a pure
        // metrics change; hit runs rebuild lazily (reshape dirtied
        // them).
        self.minimap_cache = None;
        self.request_redraw();
    }

    fn resize(&mut self, width: u32, height: u32) -> Option<ViewportSend> {
        // The same painted-before policy as the font transaction
        // (framing Q#F6), decided against the OLD geometry: narrowing
        // must not strand a stationary caret in a new wrap, and
        // widening an intentionally caret-free viewport only
        // normalizes its retained scroll.
        let caret_was_painted = self.caret_painted_in_code_clip();
        self.config.width = width;
        self.config.height = height;
        if let Some(surface) = &self.surface {
            surface.configure(&self.device, &self.config);
        }
        self.viewport
            .update(&self.queue, Resolution { width, height });
        // All seven buffers through the shared dimension helper.
        self.sync_buffer_dimensions();
        // A taller/shorter window changes the visible line count, so the
        // slice + scoped viewport change (session S1).
        self.reshape();
        if caret_was_painted {
            self.ensure_caret_painted();
        }
        self.request_redraw();
        self.current_buffer_id
            .and_then(|bid| self.viewport_send_if_changed(bid))
    }

    /// Acquire the surface's current texture and render into it — the live
    /// windowed path. Composition lives in `render_to_view`, shared with
    /// the headless offscreen path (`render_offscreen`, F-014).
    fn render(&mut self) {
        let frame = {
            let Some(surface) = self.surface.as_ref() else {
                return;
            };
            match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                    surface.configure(&self.device, &self.config);
                    return;
                }
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    return;
                }
                wgpu::CurrentSurfaceTexture::Validation => {
                    eprintln!("surface acquisition raised a validation error");
                    return;
                }
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.render_to_view(&view);
        frame.present();
    }

    /// Render one frame to an offscreen texture and read it back as packed
    /// RGBA8 (`width * height * 4` bytes, row padding removed). The entry
    /// point for the headless render harness (F-014) and for the Vterm
    /// Stage 3 attach probe, which needs real composited pixels rather
    /// than a claim that it rendered.
    fn render_offscreen(&mut self) -> Vec<u8> {
        let width = self.config.width;
        let height = self.config.height;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pmacs-gpu offscreen target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.render_to_view(&view);

        // Copy into a mappable buffer, honoring the 256-byte per-row
        // alignment `copy_texture_to_buffer` requires.
        let unpadded_bytes_per_row = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pmacs-gpu readback"),
            size: u64::from(padded_bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pmacs-gpu readback encoder"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll readback");
        rx.recv().expect("map channel").expect("map readback");

        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((unpadded_bytes_per_row * height) as usize);
        for row in 0..height {
            let start = (row * padded_bytes_per_row) as usize;
            pixels.extend_from_slice(&mapped[start..start + unpadded_bytes_per_row as usize]);
        }
        drop(mapped);
        readback.unmap();
        pixels
    }

    /// Compose and submit one frame into `view`. Window-agnostic — shared
    /// by the live surface path (`render`) and the headless offscreen path
    /// (`render_offscreen`, F-014). No surface acquire, no `present`.
    #[allow(clippy::too_many_lines)] // linear per-frame GPU sequence + optional timing.
    fn render_to_view(&mut self, view: &wgpu::TextureView) {
        let frame_start = debug_frame().then(std::time::Instant::now);
        self.refresh_status_line();
        self.refresh_menu_buffer();
        // UX gutter: reshape the line-number layer to the current scroll
        // (no-op when the gutter is off).
        self.refresh_gutter_buffer();
        // Q#CM1 — the context-menu popup quads (bg / highlight /
        // separators), drawn as a top layer after everything else.
        let menu_vertices = self.menu_vertex_bytes();
        let menu_vertex_count = (menu_vertices.len() / QUAD_VERTEX_STRIDE as usize) as u32;
        let menu_bg_buffer = self
            .menu_bg_vertex_buffer
            .upload(
                &self.device,
                &self.queue,
                "pmacs-gpu context menu",
                &menu_vertices,
            )
            .cloned();
        // Q#MB1 — the minibuffer dropdown quads (bg + selection), a top
        // layer above the band. `refresh_mb_buffer` first so the width
        // measurement in `mb_dropdown_vertex_bytes` is current.
        self.refresh_mb_buffer();
        let mb_vertices = self.mb_dropdown_vertex_bytes();
        let mb_vertex_count = (mb_vertices.len() / QUAD_VERTEX_STRIDE as usize) as u32;
        let mb_bg_buffer = self
            .mb_bg_vertex_buffer
            .upload(
                &self.device,
                &self.queue,
                "pmacs-gpu minibuffer dropdown",
                &mb_vertices,
            )
            .cloned();
        // Arc 1a Q#C5 — the completion dropdown quads (bg + selection),
        // a layer over the code anchored at the popup's byte anchor.
        // `refresh_completion_buffer` first so the width measurement in
        // `completion_dropdown_vertex_bytes` is current.
        self.refresh_completion_buffer();
        let completion_vertices = self.completion_dropdown_vertex_bytes();
        let completion_vertex_count =
            (completion_vertices.len() / QUAD_VERTEX_STRIDE as usize) as u32;
        let completion_bg_buffer = self
            .completion_bg_vertex_buffer
            .upload(
                &self.device,
                &self.queue,
                "pmacs-gpu completion dropdown",
                &completion_vertices,
            )
            .cloned();
        // Vterm Stage 3 — terminal mode replaces every document paint
        // batch. Document decoration washes, squiggles, caret, minimap,
        // and gutter describe a rope this window is not showing; the
        // status band and the popup layers above it stay, because they
        // are buffer-independent chrome the daemon still drives.
        let terminal_mode = self.terminal.is_some();
        // Inline-math ink (Q#MS6): glyph mini-buffers plus fraction-rule
        // quads. The rules ride the bg quad batch AFTER the decoration
        // washes, so a selection/search wash under a rendered box never
        // paints over its fraction bar; the glyphs get their own layer
        // in the code z-slot below.
        let (math_buffers, math_rules) = if terminal_mode {
            (Vec::new(), Vec::new())
        } else {
            self.build_math_paint()
        };
        // The band's strip rides the bg quad batch so it draws under
        // the band text (text renders after the first quad draw).
        let mut bg_vertices = if terminal_mode {
            self.terminal_quad_vertex_bytes()
        } else {
            self.decoration_background_vertex_bytes()
        };
        bg_vertices.extend(rects_to_vertex_bytes(
            &math_rules,
            self.config.width,
            self.config.height,
        ));
        bg_vertices.extend(self.status_band_vertex_bytes());
        // Bottom panel Stage 2B-3 — the divider strip, the band's cell
        // backgrounds, and the panel caret. Added in BOTH modes: the
        // document may itself be a terminal while a panel is open, so
        // gating this on `terminal_mode` would make the band vanish
        // exactly when it is hosting the output the user asked for.
        bg_vertices.extend(self.panel_quad_vertex_bytes());
        let bg_vertex_count = (bg_vertices.len() / QUAD_VERTEX_STRIDE as usize) as u32;
        let bg_buffer = self
            .bg_vertex_buffer
            .upload(
                &self.device,
                &self.queue,
                "pmacs-gpu decoration backgrounds",
                &bg_vertices,
            )
            .cloned();
        // Diagnostic squiggles (Q#W1): own pipeline + buffer, drawn
        // between the wash quads and the text (under the glyphs, the
        // z-slot the straight bar held).
        let mut squiggle_vertices = if terminal_mode {
            self.terminal_squiggle_vertex_bytes()
        } else {
            self.squiggle_vertex_bytes()
        };
        // The band's curly underlines, in BOTH modes for the same reason its
        // quads are: the document may itself be a terminal while a panel is
        // open.
        squiggle_vertices.extend(self.panel_squiggle_vertex_bytes());
        let squiggle_vertex_count =
            (squiggle_vertices.len() / SQUIGGLE_VERTEX_STRIDE as usize) as u32;
        let squiggle_buffer = self
            .squiggle_vertex_buffer
            .upload(
                &self.device,
                &self.queue,
                "pmacs-gpu diagnostic squiggles",
                &squiggle_vertices,
            )
            .cloned();
        let caret_vertices = if terminal_mode {
            self.terminal_cursor_vertex_bytes()
        } else {
            self.caret_vertex_bytes()
        };
        let caret_vertex_count = (caret_vertices.len() / QUAD_VERTEX_STRIDE as usize) as u32;
        let caret_buffer = self
            .caret_vertex_buffer
            .upload(
                &self.device,
                &self.queue,
                "pmacs-gpu caret",
                &caret_vertices,
            )
            .cloned();
        let after_bg = debug_frame().then(std::time::Instant::now);
        // Minimap quads depend only on (summary, size, scroll); cache
        // the vertex bytes instead of rescanning every line shape per
        // frame.
        let minimap_key = (
            self.current_summary.as_ref().map_or(0, |s| s.generation),
            self.config.width,
            self.config.height,
            self.scroll_top,
        );
        if self
            .minimap_cache
            .as_ref()
            .is_none_or(|(key, _)| *key != minimap_key)
        {
            self.minimap_cache = Some((minimap_key, self.minimap_vertex_bytes()));
        }
        let empty_minimap: Vec<u8> = Vec::new();
        let minimap_vertices = if terminal_mode {
            &empty_minimap
        } else {
            &self.minimap_cache.as_ref().expect("just filled").1
        };
        let minimap_vertex_count = (minimap_vertices.len() / QUAD_VERTEX_STRIDE as usize) as u32;
        let minimap_buffer = self
            .minimap_vertex_buffer
            .upload(
                &self.device,
                &self.queue,
                "pmacs-gpu minimap vertices",
                minimap_vertices,
            )
            .cloned();
        let after_minimap = debug_frame().then(std::time::Instant::now);
        let text_bounds_right = self.text_bounds_right();

        // Right-align from the true full shaped width. An over-wide
        // custom prefix may put this origin left of the surface; bounds
        // clip it while the protected suffix remains pinned.
        let status_width = self
            .status_buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max);
        let status_left = self.config.width as f32 - STATUS_TEXT_PAD - status_width;
        let status_top = status_band_top(self.config.height, self.fm)
            + (self.fm.status_band_height() - self.fm.status_line_height()) / 2.0;
        // UX gutter: the code's left origin (past the gutter) and the
        // main-text clip-left. Computed here as locals — calling `self.*`
        // inside the `prepare` args would conflict with its `&mut` borrows.
        let text_left = self.text_left();
        // Hoisted for the same borrow reason as the colors below: the
        // terminal areas are built inside a `&mut self.*` argument list.
        let mono_advance = self.mono_advance();
        let gutter_clip_left = if self.line_numbers.is_on() {
            text_left.floor() as i32
        } else {
            0
        };
        // Themes Q#TH5/Q#TH9: resolve the face-driven colors before
        // the prepare call — its `&mut self.*` field borrows preclude
        // method calls on `self` inside the argument list.
        let readout_color = self
            .modeline_face_colors()
            .map_or(Color::rgb(168, 168, 180), |(_, text)| text);
        let left_color = self.status_left_color();
        let gutter_color = self.face_fg_or("ui.gutter", Color::rgb(120, 120, 135));
        // Vterm Stage 3 — the document code layer is dropped entirely
        // in terminal mode; terminal glyphs draw from their own
        // per-run layer below, positioned at cell origins.
        let code_areas: Vec<TextArea> = if terminal_mode {
            Vec::new()
        } else {
            vec![TextArea {
                buffer: &self.buffer,
                // Stage 5: the whole of the glyph-side mechanism.
                // glyphon clips to `bounds`, whose `left` stays at the
                // gutter, so shifting the origin scrolls the text and
                // the gutter keeps its own pixels.
                left: text_left - self.code_scroll_left,
                top: TEXT_TOP,
                scale: 1.0,
                bounds: TextBounds {
                    left: gutter_clip_left,
                    top: 0,
                    right: text_bounds_right,
                    // Clip at the status band (Q#S3): a final
                    // partially-visible line must not bleed
                    // into the band.
                    bottom: document_text_bottom(self.config.height, self.fm, self.band_inset())
                        .round() as i32,
                },
                default_color: Color::rgb(230, 230, 235),
                custom_glyphs: &[],
            }]
        };
        self.text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                code_areas.into_iter().chain([
                    TextArea {
                        buffer: &self.status_buffer,
                        left: status_left,
                        top: status_top,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: 0,
                            top: status_band_top(self.config.height, self.fm).round() as i32,
                            right: self.config.width.cast_signed(),
                            bottom: self.config.height.cast_signed(),
                        },
                        // Themes Q#TH5: a set ui.modeline face colors
                        // the readout too (its fg after the reverse
                        // swap); unset keeps the dimmer gray.
                        default_color: readout_color,
                        custom_glyphs: &[],
                    },
                    TextArea {
                        buffer: &self.status_left_buffer,
                        left: STATUS_TEXT_PAD,
                        top: status_top,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: 0,
                            top: status_band_top(self.config.height, self.fm).round() as i32,
                            // Stop at the right group's actual origin.
                            right: status_left.max(0.0).round() as i32,
                            bottom: self.config.height.cast_signed(),
                        },
                        // Themes Q#TH3: the left segment's face follows
                        // its CONTENT class (minibuffer/isearch →
                        // ui.minibuffer; message → ui.statusline; name
                        // → ui.modeline).
                        default_color: left_color,
                        custom_glyphs: &[],
                    },
                ]),
                &mut self.swash_cache,
            )
            .expect("text_renderer prepare");

        // Inline-math glyph layer (Q#MS6): one TextArea per glyph,
        // clipped by the same code-area bounds as the code layer.
        let math_areas: Vec<TextArea> = math_buffers
            .iter()
            .map(|(buf, left, top)| TextArea {
                buffer: buf,
                left: *left,
                top: *top,
                scale: 1.0,
                bounds: TextBounds {
                    left: gutter_clip_left,
                    top: 0,
                    right: text_bounds_right,
                    bottom: document_text_bottom(self.config.height, self.fm, self.band_inset())
                        .round() as i32,
                },
                default_color: MATH_INK_COLOR,
                custom_glyphs: &[],
            })
            .collect();
        self.math_text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                math_areas,
                &mut self.swash_cache,
            )
            .expect("math text_renderer prepare");

        // UX gutter: prepare the line-number layer in the reserved left
        // strip (empty when off → renders nothing). Same `top` + line
        // height as the code, so numbers align row-for-row.
        let gutter_areas: Vec<TextArea> = if self.line_numbers.is_on() && !terminal_mode {
            vec![TextArea {
                buffer: &self.gutter_buffer,
                left: TEXT_LEFT,
                top: TEXT_TOP,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: gutter_clip_left,
                    bottom: document_text_bottom(self.config.height, self.fm, self.band_inset())
                        .round() as i32,
                },
                // Themes Q#TH5: ui.gutter's {fg} mask colors the digits.
                default_color: gutter_color,
                custom_glyphs: &[],
            }]
        } else {
            Vec::new()
        };
        self.gutter_text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                gutter_areas,
                &mut self.swash_cache,
            )
            .expect("gutter_text_renderer prepare");

        // Q#CM1 — prepare the menu glyphs in their own layer (empty when
        // closed, so the renderer draws nothing).
        let menu_areas: Vec<TextArea> = self
            .menu
            .as_ref()
            .map(|menu| {
                let ax = menu.anchor_px.0 as f32;
                let ay = menu.anchor_px.1 as f32;
                TextArea {
                    buffer: &self.menu_buffer,
                    left: ax + MENU_PAD_X,
                    top: ay + 2.0,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: ax as i32,
                        top: ay as i32,
                        right: (ax + Self::menu_width_px(menu, self.fm)).round() as i32,
                        bottom: (ay + menu.rows.len() as f32 * self.fm.menu_row_height()).round()
                            as i32,
                    },
                    default_color: Color::rgb(232, 232, 238),
                    custom_glyphs: &[],
                }
            })
            .into_iter()
            .collect();
        self.menu_text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                menu_areas,
                &mut self.swash_cache,
            )
            .expect("menu text_renderer prepare");

        // Q#MB1 — prepare the minibuffer dropdown glyphs in their layer.
        // The buffer is shaped with *all* candidates; F-007 scrolls it up
        // by `first` rows so line `first` lands at `top_y`, and the
        // existing `bounds.top`/`bottom` clip the rows scrolled out of the
        // visible window (no per-resize re-shape needed).
        // Hoisted for the same borrow reason as the band colors above.
        let candidate_color = self.face_fg_or("ui.minibuffer.candidate", Color::rgb(232, 232, 238));
        let mb_areas: Vec<TextArea> = self
            .mb_visible_window()
            .zip(self.mb_dropdown_rect())
            .map(|((first, _count), (x, top_y, width))| TextArea {
                buffer: &self.mb_buffer,
                left: x + MB_DROP_PAD_X,
                top: top_y - first as f32 * self.fm.mb_drop_row_height(),
                scale: 1.0,
                bounds: TextBounds {
                    left: x as i32,
                    top: top_y as i32,
                    right: (x + width).round() as i32,
                    bottom: status_band_top(self.config.height, self.fm).round() as i32,
                },
                // Themes Q#TH5 (round 3 finding 1): the candidate
                // glyph layer is ui.minibuffer.candidate's GPU site;
                // the popup bg/selection quads stay chrome constants.
                default_color: candidate_color,
                custom_glyphs: &[],
            })
            .into_iter()
            .collect();
        self.mb_text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                mb_areas,
                &mut self.swash_cache,
            )
            .expect("minibuffer text_renderer prepare");

        // Arc 1a Q#C5 — prepare the completion dropdown glyphs in their
        // layer. The buffer is shaped with *all* wire rows; the layout
        // scrolls it up by `first` rows so row `first` lands at `top_y`,
        // and `bounds` clips the rows outside the visible window (the
        // minibuffer dropdown's F-007 shape).
        let completion_layout = self
            .completion_dropdown_layout()
            .zip(self.completion_dropdown_rect());
        let completion_areas: Vec<TextArea> = completion_layout
            .map(|((first, count, _ax, _ty), (x, top_y, width))| TextArea {
                buffer: &self.completion_buffer,
                left: x + MB_DROP_PAD_X,
                top: top_y - first as f32 * self.fm.mb_drop_row_height(),
                scale: 1.0,
                bounds: TextBounds {
                    left: x as i32,
                    top: top_y as i32,
                    right: (x + width).round() as i32,
                    bottom: (top_y + count as f32 * self.fm.mb_drop_row_height()).round() as i32,
                },
                default_color: Color::rgb(232, 232, 238),
                custom_glyphs: &[],
            })
            .into_iter()
            .collect();
        self.completion_text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                completion_areas,
                &mut self.swash_cache,
            )
            .expect("completion text_renderer prepare");

        // Vterm Stage 3 — one TextArea per planned run, each pinned to
        // its own cell origin and clipped to its declared footprint.
        // Per-run areas are the point: a row-wide area would let one
        // wide glyph's shaped advance shift every column after it.
        // Hoisted out of the closure below: the band inset borrows `self`
        // immutably, and the closure already holds one.
        let document_clip_bottom =
            document_text_bottom(self.config.height, self.fm, self.band_inset()).round() as i32;
        let terminal_areas: Vec<TextArea> = self
            .terminal
            .as_ref()
            .map(|terminal| {
                let (ox, oy) = (TEXT_LEFT, TEXT_TOP);
                let advance = mono_advance;
                let line = self.fm.code_line_height();
                let clip_bottom = document_clip_bottom;
                let clip_right = self.config.width.cast_signed();
                terminal
                    .plan
                    .runs
                    .iter()
                    .zip(self.terminal_text_buffers.iter())
                    .map(|(run, buffer)| {
                        let left = ox + run.col as f32 * advance;
                        let top = oy + run.row as f32 * line;
                        let right = (left + run.cells as f32 * advance).round() as i32;
                        TextArea {
                            buffer,
                            left,
                            top,
                            scale: 1.0,
                            bounds: TextBounds {
                                left: left.floor() as i32,
                                top: top.floor().max(0.0) as i32,
                                right: right.min(clip_right),
                                bottom: (top + line).round().min(clip_bottom as f32) as i32,
                            },
                            default_color: Color::rgb(230, 230, 235),
                            custom_glyphs: &[],
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.terminal_text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                terminal_areas,
                &mut self.swash_cache,
            )
            .expect("terminal text_renderer prepare");

        // Bottom panel Stage 2B-3 — one `TextArea` per planned panel run,
        // pinned to its own cell origin inside the band and clipped to the
        // band's content rect. The clip is the band's, never the
        // document's: a panel run must not be cut off at
        // `document_text_bottom`, which is the boundary directly above it.
        let panel_areas: Vec<TextArea> = match (self.panel_content_rect(), self.panel.plan.as_ref())
        {
            (Some((ox, oy, bw, bh)), Some(plan))
                if self.panel.presented().is_some() && self.panel_cell_advance().is_some() =>
            {
                let advance = self
                    .panel_cell_advance()
                    .expect("checked by the match guard");
                let line = self.fm.code_line_height();
                let clip_top = oy.round() as i32;
                let clip_right = (ox + bw).round() as i32;
                let clip_bottom = (oy + bh).round() as i32;
                plan.runs
                    .iter()
                    .zip(self.panel_text_buffers.iter())
                    .map(|(run, buffer)| {
                        let left = ox + run.col as f32 * advance;
                        let top = oy + run.row as f32 * line;
                        // Per-run bounds, intersected with the band. The
                        // run's own footprint is the inner clip — that is
                        // what stops one wide glyph's shaped advance from
                        // bleeding into the next column — and the band rect
                        // is the outer one.
                        let right = (left + run.cells as f32 * advance).round() as i32;
                        TextArea {
                            buffer,
                            left,
                            top,
                            scale: 1.0,
                            bounds: TextBounds {
                                left: left.floor().max(0.0) as i32,
                                top: top.floor().max(clip_top as f32) as i32,
                                right: right.min(clip_right),
                                bottom: (top + line).round().min(clip_bottom as f32) as i32,
                            },
                            default_color: rgb_to_glyphon(run.color),
                            custom_glyphs: &[],
                        }
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        self.panel_text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                panel_areas,
                &mut self.swash_cache,
            )
            .expect("panel text_renderer prepare");

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pmacs-gpu frame encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pmacs-gpu pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(BG),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            // Q#2 stance (α): single render pass, three draws. Quad
            // backgrounds first (Selection today; CurrentLine in 9.2)
            // so their translucent fills sit under the glyphs; text
            // second so source/inlay color shows on top; minimap last
            // so it draws over the right-margin text region.
            if let Some(vertex_buffer) = bg_buffer.as_ref() {
                self.quad_renderer
                    .render(&mut pass, vertex_buffer, bg_vertex_count);
            }
            // Diagnostic squiggles under the glyphs (Q#W1).
            if let Some(vertex_buffer) = squiggle_buffer.as_ref() {
                self.squiggle_renderer
                    .render(&mut pass, vertex_buffer, squiggle_vertex_count);
            }
            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("text_renderer render");
            // Inline-math glyphs share the code layer's z-slot: over
            // the washes and rule quads, under the caret and popups.
            self.math_text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("math text_renderer render");
            // Vterm Stage 3 — terminal glyphs sit in the code layer's
            // z-slot: over the cell backgrounds and underlines, under
            // the cursor block.
            self.terminal_text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("terminal text_renderer render");
            // The band's glyphs, over its own cell backgrounds and the
            // divider strip.
            self.panel_text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("panel text_renderer render");
            // UX gutter: line numbers in the reserved left strip (empty
            // layer when off).
            self.gutter_text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("gutter_text_renderer render");
            // Caret over the text so the insertion point reads on top
            // of the glyph it sits before (session B1).
            if let Some(vertex_buffer) = caret_buffer.as_ref() {
                self.quad_renderer
                    .render(&mut pass, vertex_buffer, caret_vertex_count);
            }
            if let Some(vertex_buffer) = minimap_buffer.as_ref() {
                self.quad_renderer
                    .render(&mut pass, vertex_buffer, minimap_vertex_count);
            }
            // Q#MB1 — the minibuffer dropdown draws above the band: bg +
            // selection quads, then its candidate glyphs on top.
            if let Some(vertex_buffer) = mb_bg_buffer.as_ref() {
                self.quad_renderer
                    .render(&mut pass, vertex_buffer, mb_vertex_count);
            }
            self.mb_text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("minibuffer text_renderer render");
            // Arc 1a Q#C5 — the completion dropdown floats over the
            // code at its byte anchor: bg + selection quads, then its
            // row glyphs on top (under the context menu, which stays
            // the topmost surface).
            if let Some(vertex_buffer) = completion_bg_buffer.as_ref() {
                self.quad_renderer
                    .render(&mut pass, vertex_buffer, completion_vertex_count);
            }
            self.completion_text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("completion text_renderer render");
            // Q#CM1 — the context menu draws last: its bg/highlight quads
            // occlude everything beneath, then its glyphs on top.
            if let Some(vertex_buffer) = menu_bg_buffer.as_ref() {
                self.quad_renderer
                    .render(&mut pass, vertex_buffer, menu_vertex_count);
            }
            self.menu_text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("menu text_renderer render");
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        self.atlas.trim();

        if let (Some(start), Some(after_bg), Some(after_minimap)) =
            (frame_start, after_bg, after_minimap)
        {
            let end = std::time::Instant::now();
            let us = |a: std::time::Instant, b: std::time::Instant| b.duration_since(a).as_micros();
            eprintln!(
                "pmacs-gpu frame: bg={}us minimap={}us prepare+submit={}us total={}us peers={}",
                us(start, after_bg),
                us(after_bg, after_minimap),
                us(after_minimap, end),
                us(start, end),
                self.peer_presences.len(),
            );
        }
    }

    fn text_bounds_right(&self) -> i32 {
        if self.has_minimap() {
            minimap_left(self.config.width).map_or(self.config.width.cast_signed(), |left| {
                (left - TEXT_RIGHT_GAP).max(TEXT_LEFT + 1.0).round() as i32
            })
        } else {
            self.config.width.cast_signed()
        }
    }

    fn has_minimap(&self) -> bool {
        self.current_summary
            .as_ref()
            .is_some_and(|summary| !summary.lines.is_empty())
            && minimap_left(self.config.width).is_some()
    }

    fn minimap_vertex_bytes(&self) -> Vec<u8> {
        let Some(summary) = self.current_summary.as_ref() else {
            return Vec::new();
        };
        let visible_lines = estimated_visible_lines(self.config.height, self.fm, self.band_inset());
        let rects = minimap_rects(
            &summary.lines,
            &self.current_line_shapes,
            self.config.width,
            self.config.height,
            // The thumb tracks the live scroll position. (It was
            // hardcoded to 0 from the minimap's first session —
            // surfaced by Q#M6 validation, where jumping finally
            // made the frozen thumb obvious.)
            self.scroll_top,
            visible_lines,
            self.fm,
            self.band_inset(),
        );
        rects_to_vertex_bytes(&rects, self.config.width, self.config.height)
    }

    /// Vertex bytes for quad-pipeline background washes (drawn *under*
    /// the text). Two sources, both `Selection` / `CurrentLine`: this
    /// frontend's *own* window decorations from `current_decorations`
    /// (live again since session B1 reactivated the own cursor — Q#B4;
    /// QB1 had suppressed them while the mirror was read-only), and
    /// *peer* presence from `PresenceUpdate` (session 9.3). Both reuse
    /// the same `\n`-line offset table to rebase cosmic-text's
    /// line-relative glyph offsets (QB3). The caret is separate (drawn
    /// *over* text) — see [`Self::caret_vertex_bytes`].
    fn decoration_background_vertex_bytes(&self) -> Vec<u8> {
        let Some(buffer_id) = self.current_buffer_id else {
            return Vec::new();
        };
        let (vstart, vend) = self.view_range;
        if vend <= vstart {
            return Vec::new();
        }
        // Glyph offsets are relative to the *slice* the buffer holds
        // (session S1), so the line table is computed on the slice and
        // every whole-file byte range is clip-rebased onto it.
        let slice = &self.current_text[vstart as usize..vend as usize];
        let line_offsets = line_byte_offsets(slice);
        let mut rects = Vec::new();
        self.collect_own_decoration_rects(&mut rects, &line_offsets, vstart, vend);
        self.collect_peer_rects(buffer_id, &line_offsets, vstart, vend, &mut rects);
        self.collect_gutter_sign_rects(&mut rects, &line_offsets, vstart, vend);
        rects_to_vertex_bytes(&rects, self.config.width, self.config.height)
    }

    /// Per-visible-line diagnostic sign bars in the gutter (UX gutter
    /// sub-arc 2): one severity-colored bar at the gutter's left edge for
    /// each line carrying a diagnostic, most-severe winning. Only when the
    /// gutter is on, mirroring the TUI (signs ride the line-number gutter).
    /// The GPU analogue of the TUI's leading-column `E`/`W`/`I`/`H` glyph.
    fn collect_gutter_sign_rects(
        &self,
        rects: &mut Vec<MinimapRect>,
        line_offsets: &[u64],
        vstart: u64,
        vend: u64,
    ) {
        if !self.line_numbers.is_on() {
            return;
        }
        let slice_len = vend - vstart;
        for run in self.buffer.layout_runs() {
            let line_base = line_offsets.get(run.line_i).copied().unwrap_or(0);
            let line_end = line_offsets
                .get(run.line_i + 1)
                .copied()
                .unwrap_or(slice_len);
            let mut best: Option<(u8, [f32; 4])> = None;
            for d in &self.current_decorations {
                let Some(rank) = diagnostic_severity_rank(d.kind) else {
                    continue;
                };
                let Some((lo, hi)) = clip_rebase_range(d.range.start, d.range.end, vstart, vend)
                else {
                    continue;
                };
                if hi <= line_base || lo >= line_end {
                    continue; // decoration doesn't touch this line
                }
                if best.is_none_or(|(r, _)| rank < r)
                    && let Some(color) = decoration_kind_to_underline_color(d.kind)
                {
                    // Themes Q#TH5: gutter signs share the resolved
                    // severity color with the squiggles.
                    best = Some((rank, self.diag_face_rgba(d.kind, color)));
                }
            }
            if let Some((_, color)) = best {
                rects.push(MinimapRect {
                    x: GUTTER_SIGN_X,
                    y: TEXT_TOP + run.line_top,
                    w: GUTTER_SIGN_W,
                    h: run.line_height,
                    color,
                });
            }
        }
    }

    /// Own-window `Selection` washes from `current_decorations`. The
    /// caret already marks the own cursor, so the own *`CurrentLine`*
    /// wash is deliberately NOT rendered — a whole-line highlight on
    /// every cursor line reads as a persistent selection, which is not
    /// wanted as default editor behavior (revising Q#B4: the caret is
    /// the own-cursor indicator; the line wash isn't). Peer presence
    /// still shows other frontends' lines via `collect_peer_rects`.
    fn collect_own_decoration_rects(
        &self,
        rects: &mut Vec<MinimapRect>,
        line_offsets: &[u64],
        vstart: u64,
        vend: u64,
    ) {
        for d in &self.current_decorations {
            if d.kind == DecorationKind::CurrentLine {
                continue;
            }
            let Some((lo, hi)) = clip_rebase_range(d.range.start, d.range.end, vstart, vend) else {
                continue;
            };
            // Diagnostic underlines are squiggles now, drawn by their
            // own pipeline (`squiggle_vertex_bytes`); only the solid
            // washes belong in this quad batch. Own washes resolve
            // their theme faces (Q#TH5); peers keep the constants.
            if let Some(color) = self.own_wash_color(d.kind) {
                self.push_glyph_extent_rects(rects, line_offsets, lo, hi, color, None);
            }
        }
    }

    /// Vertex bytes for diagnostic squiggles (Q#W1), drawn by the
    /// dedicated [`SquiggleRenderer`] pipeline rather than the solid
    /// quad batch. Geometry is the same bottom-hugging glyph-extent
    /// band as the old straight bar — only the vertex *format* differs
    /// (carries the `uv` the sine fragment shader needs).
    fn squiggle_vertex_bytes(&self) -> Vec<u8> {
        if self.current_buffer_id.is_none() {
            return Vec::new();
        }
        let (vstart, vend) = self.view_range;
        if vend <= vstart {
            return Vec::new();
        }
        let slice = &self.current_text[vstart as usize..vend as usize];
        let line_offsets = line_byte_offsets(slice);
        let mut rects = Vec::new();
        for d in &self.current_decorations {
            let Some(color) = decoration_kind_to_underline_color(d.kind) else {
                continue;
            };
            // Themes Q#TH5: squiggles follow the ui.diag.* faces
            // (fg mask, Default ↦ the built-in severity constant).
            let color = self.diag_face_rgba(d.kind, color);
            if let Some((lo, hi)) = clip_rebase_range(d.range.start, d.range.end, vstart, vend) {
                self.push_glyph_extent_rects(
                    &mut rects,
                    &line_offsets,
                    lo,
                    hi,
                    color,
                    Some(DIAG_SQUIGGLE_PX),
                );
            }
        }
        squiggles_to_vertex_bytes(&rects, self.config.width, self.config.height)
    }

    /// Peer cursor-line + selection washes from `PresenceUpdate`
    /// (session 9.3). Single-peer mirrors reuse the `Selection` /
    /// `CurrentLine` colors; per-peer distinct colors are deferred.
    fn collect_peer_rects(
        &self,
        buffer_id: BufferId,
        line_offsets: &[u64],
        vstart: u64,
        vend: u64,
        rects: &mut Vec<MinimapRect>,
    ) {
        let text_len = self.current_text.len() as u64;
        for presence in self.peer_presences.values() {
            if presence.buffer_id != buffer_id {
                continue;
            }
            if let Some(color) = decoration_kind_to_bg_color(DecorationKind::CurrentLine) {
                let (lo, hi) = source_line_range(&self.current_text, presence.cursor);
                if let Some((lo, hi)) = clip_rebase_range(lo, hi, vstart, vend) {
                    self.push_glyph_extent_rects(rects, line_offsets, lo, hi, color, None);
                }
            }
            if let Some(sel) = presence.selection
                && let Some(color) = decoration_kind_to_bg_color(DecorationKind::Selection)
            {
                let lo = sel.anchor.min(sel.active).min(text_len);
                let hi = sel.anchor.max(sel.active).min(text_len);
                if let Some((lo, hi)) = clip_rebase_range(lo, hi, vstart, vend) {
                    self.push_glyph_extent_rects(rects, line_offsets, lo, hi, color, None);
                }
            }
        }
    }

    /// Vertex bytes for the caret quad, drawn *over* the text (B1).
    /// Empty when no own cursor is known, it's in another buffer, or
    /// its visual run is scrolled outside the drawable code window
    /// (the caret quad has no `TextBounds` clip of its own, so the
    /// drawable intersection gates it here).
    /// The frame's math ink (Q#MS6): one shaped mini-buffer per
    /// `MathItem::Glyph` — `(buffer, left, top)` in window pixels —
    /// plus the fraction-rule quads. Origins come from the REAL shaped
    /// spacer glyph and the run's REAL baseline, and each mini-buffer
    /// is positioned by the baseline cosmic-text actually produced for
    /// it, so measured and drawn geometry share one origin (F8b: the
    /// family is pinned to the bundled math font layout measured).
    fn build_math_paint(&mut self) -> (Vec<(Buffer, f32, f32)>, Vec<MinimapRect>) {
        let mut pending: Vec<(char, f32, f32, f32)> = Vec::new();
        let mut rules: Vec<MinimapRect> = Vec::new();
        if self.line_math_cache.iter().all(|m| m.placed.is_empty()) {
            return (Vec::new(), rules);
        }
        for run in self.buffer.layout_runs() {
            let Some(state) = self.line_math_cache.get(run.line_i) else {
                continue;
            };
            for placed in &state.placed {
                // The run holding the spacer's FIRST glyph anchors the
                // box. A spacer split by soft wrap draws whole at that
                // origin — the one-rectangle model Q#MS4 reserves.
                let Some(anchor) = run
                    .glyphs
                    .iter()
                    .find(|g| g.start as u64 == placed.projected_start)
                else {
                    continue;
                };
                // Stage 5: the box rides the offset with the spacer
                // glyph it is anchored to. The glyph mini-buffers are
                // clipped by their layer's `TextBounds` (the same
                // `gutter_clip_left` the code layer uses), so only the
                // rule quads below need the manual crop.
                let origin_x = self.code_x_to_screen(anchor.x);
                let baseline_px = TEXT_TOP + run.line_y;
                for item in &placed.boxed.items {
                    match *item {
                        math_layout::MathItem::Glyph {
                            ch,
                            x,
                            baseline,
                            size_px,
                        } => {
                            // Box space is y-up around the baseline;
                            // screen space is y-down.
                            pending.push((ch, origin_x + x, baseline_px - baseline, size_px));
                        }
                        math_layout::MathItem::Rule {
                            x,
                            y,
                            width,
                            thickness,
                        } => {
                            // The fraction bars are quads in the bg
                            // batch, so unlike the glyphs above they
                            // carry no scissor and must be cropped.
                            if let Some((sx, sw)) = self.crop_to_code_clip_left(origin_x + x, width)
                            {
                                rules.push(MinimapRect {
                                    x: sx,
                                    y: baseline_px - y - thickness / 2.0,
                                    w: sw,
                                    h: thickness,
                                    color: MATH_INK_RGBA,
                                });
                            }
                        }
                    }
                }
            }
        }
        let mut buffers = Vec::with_capacity(pending.len());
        for (ch, x, baseline_px, size_px) in pending {
            let mut buf = Buffer::new(
                &mut self.font_system,
                Metrics::new(size_px, size_px * MATH_GLYPH_LINE_FACTOR),
            );
            buf.set_size(&mut self.font_system, None, None);
            buf.set_text(
                &mut self.font_system,
                ch.encode_utf8(&mut [0u8; 4]),
                &Attrs::new().family(Family::Name(MATH_FONT_FAMILY)),
                Shaping::Advanced,
                None,
            );
            buf.shape_until_scroll(&mut self.font_system, false);
            let Some(line_y) = buf.layout_runs().next().map(|r| r.line_y) else {
                continue;
            };
            buffers.push((buf, x, baseline_px - line_y));
        }
        (buffers, rules)
    }

    fn caret_vertex_bytes(&mut self) -> Vec<u8> {
        // Q#MB1 — while the minibuffer is open the caret lives in the
        // band at the input cursor, not in the buffer.
        if self.minibuffer.is_some() {
            return self
                .minibuffer_caret_rect()
                .map(|r| rects_to_vertex_bytes(&[r], self.config.width, self.config.height))
                .unwrap_or_default();
        }
        let Some(rect) = self.code_caret_rect_in_clip() else {
            return Vec::new();
        };
        rects_to_vertex_bytes(&[rect], self.config.width, self.config.height)
    }

    /// The caret rectangle for an open minibuffer (Q#MB1): a thin bar in
    /// the bottom band at the input cursor. The band font is monospace,
    /// so the per-char advance is the shaped status-left width divided by
    /// its char count; the caret sits `prompt_chars + cursor` advances
    /// from the band's left pad.
    fn minibuffer_caret_rect(&self) -> Option<MinimapRect> {
        let mb = self.minibuffer.as_ref()?;
        let line_w = self
            .status_left_buffer
            .layout_runs()
            .map(|r| r.line_w)
            .fold(0.0_f32, f32::max);
        let chars = mb.prompt.chars().count() + mb.input.chars().count();
        let advance = if chars > 0 {
            line_w / chars as f32
        } else {
            0.0
        };
        let cursor_chars = mb.prompt.chars().count() as f32 + mb.cursor as f32;
        let status_top = status_band_top(self.config.height, self.fm)
            + (self.fm.status_band_height() - self.fm.status_line_height()) / 2.0;
        Some(MinimapRect {
            x: STATUS_TEXT_PAD + advance * cursor_chars,
            y: status_top,
            w: CARET_WIDTH,
            h: self.fm.status_line_height(),
            color: CARET_COLOR,
        })
    }

    /// Map an absolute source byte to `(slice line index, projected
    /// byte offset within that shaped line)` through the same reusable
    /// chunk mapping used by decoration geometry.
    /// Adornments retain left gravity, while a source tab's two byte
    /// boundaries map to the leading and trailing edges of all of its
    /// projected spaces.
    fn code_byte_to_projected(&self, byte: u64) -> Option<(usize, usize)> {
        let line_idx = self
            .current_line_starts
            .partition_point(|&s| s <= byte)
            .saturating_sub(1);
        let slice_i = line_idx.checked_sub(self.shaped_top)?;
        let chunks = self.line_chunk_cache.get(slice_i)?;
        let rel = byte - self.current_line_starts[line_idx];
        source_to_projected(chunks, rel).map(|projected| (slice_i, projected as usize))
    }

    /// Convert an absolute source byte to a cursor cosmic-text can
    /// represent without taking `Buffer::layout_cursor`'s line-start
    /// fallback. Exact glyph ends keep `Before` affinity so a wrap
    /// boundary belongs to the preceding visual run; exact starts use
    /// `After` when no end exists there. A codepoint boundary inside a
    /// shaped cluster (combining sequence or ligature) has no native
    /// layout cursor, so it snaps explicitly to that cluster's logical
    /// end. Geometry and caret-follow both consume this same cursor.
    fn code_byte_to_layout_cursor(&mut self, byte: u64) -> Option<Cursor> {
        let (slice_i, projected) = self.code_byte_to_projected(byte)?;
        let layout = self.buffer.line_layout(&mut self.font_system, slice_i)?;
        let mut exact_end = false;
        let mut exact_start = false;
        let mut containing_end: Option<usize> = None;
        let mut previous_end: Option<usize> = None;
        let mut next_start: Option<usize> = None;
        for glyph in layout.iter().flat_map(|line| &line.glyphs) {
            exact_end |= glyph.end == projected;
            exact_start |= glyph.start == projected;
            if glyph.start < projected && projected < glyph.end {
                containing_end = Some(containing_end.map_or(glyph.end, |end| end.min(glyph.end)));
            }
            if glyph.end <= projected {
                previous_end = Some(previous_end.map_or(glyph.end, |end| end.max(glyph.end)));
            }
            if glyph.start >= projected {
                next_start = Some(next_start.map_or(glyph.start, |start| start.min(glyph.start)));
            }
        }

        let (index, affinity) = if exact_end {
            (projected, Affinity::Before)
        } else if let Some(end) = containing_end {
            (end, Affinity::Before)
        } else if exact_start {
            (projected, Affinity::After)
        } else if let Some(end) = previous_end {
            // Zero-width/unshaped codepoints between clusters keep
            // left gravity rather than falling all the way to x=0.
            (end, Affinity::Before)
        } else if let Some(start) = next_start {
            (start, Affinity::After)
        } else {
            // Empty lines have no glyph boundary; cosmic-text's only
            // representable position is the line start.
            (0, Affinity::Before)
        };
        Some(Cursor::new_with_affinity(slice_i, index, affinity))
    }

    /// The caret geometry `(x, top, line_height)` for an absolute
    /// source `byte`, in code-area-local space (x excludes
    /// `text_left`, top excludes `TEXT_TOP`) — visual-run aware
    /// (framing Q#F6). Inverts the chunk projection, then uses
    /// cosmic-text's `layout_cursor` on the same cluster-normalized,
    /// explicit-affinity `Cursor` that `ensure_caret_painted` shapes
    /// toward, so a wrap boundary selects the same visual run and an
    /// interior cluster byte never triggers cosmic-text's line-start
    /// fallback. The vertical position accumulates the laid-out run
    /// heights above the selected run under the normalized scroll;
    /// callers decide visibility by intersecting with the drawable
    /// clip.
    fn code_byte_px(&mut self, byte: u64) -> Option<(f32, f32, f32)> {
        let cursor = self.code_byte_to_layout_cursor(byte)?;
        let slice_i = cursor.line;
        let lc = self.buffer.layout_cursor(&mut self.font_system, cursor)?;
        let scroll = self.buffer.scroll();
        if slice_i < scroll.line {
            return None;
        }
        let default_line_height = self.buffer.metrics().line_height;
        let mut top = -scroll.vertical;
        for line_i in scroll.line..slice_i {
            let layout = self.buffer.line_layout(&mut self.font_system, line_i)?;
            for layout_line in layout {
                top += layout_line.line_height_opt.unwrap_or(default_line_height);
            }
        }
        let layout = self.buffer.line_layout(&mut self.font_system, slice_i)?;
        for layout_line in layout.iter().take(lc.layout) {
            top += layout_line.line_height_opt.unwrap_or(default_line_height);
        }
        let run = layout.get(lc.layout)?;
        let line_height = run.line_height_opt.unwrap_or(default_line_height);
        let x = if lc.glyph < run.glyphs.len() {
            run.glyphs[lc.glyph].x
        } else {
            // Caret after the final glyph (line/run end).
            run.glyphs.last().map_or(0.0, |glyph| glyph.x + glyph.w)
        };
        Some((x, top, line_height))
    }

    /// The caret rectangle for the own cursor: a thin bar at the left
    /// edge of the glyph the cursor sits before (or the right edge of
    /// the last glyph at line end), on the cursor's VISUAL run —
    /// wrapped continuation runs included (framing Q#F6). `None` when
    /// the cursor is outside the shaped slice. Callers gate painting
    /// on the drawable code clip.
    fn caret_rect(&mut self) -> Option<MinimapRect> {
        let own = self.own_cursor?;
        if self.current_buffer_id != Some(own.buffer_id) {
            return None;
        }
        let (vstart, vend) = self.view_range;
        let cursor = own.byte;
        if cursor < vstart || cursor > vend {
            return None; // scrolled off-screen
        }
        let (x, top, line_height) = self.code_byte_px(cursor)?;
        Some(MinimapRect {
            // UX gutter: the caret sits in the code area, past the
            // gutter — and past the horizontal offset (Stage 5), which
            // is why this goes through the one transform.
            x: self.code_x_to_screen(x),
            y: TEXT_TOP + top,
            w: CARET_WIDTH,
            h: line_height,
            color: CARET_COLOR,
        })
    }

    /// [`Self::caret_rect`] intersected with the drawable code clip —
    /// the painter's own bounds, NOT `view_range`, so the two-line
    /// source overscan and wrapped runs clipped below the band don't
    /// count as painted (framing Q#F6).
    ///
    /// **The left edge is now tested too.** This comment used to say
    /// "right of the gutter isn't needed: the caret x can't precede
    /// `text_left`" — an invariant Stage 5 deletes. A comment asserting
    /// something a later stage falsifies is worse than silence, so it
    /// is rewritten rather than merely joined by a new condition.
    fn code_caret_rect_in_clip(&mut self) -> Option<MinimapRect> {
        let rect = self.caret_rect()?;
        let bottom = document_text_bottom(self.config.height, self.fm, self.band_inset());
        let right = self.text_bounds_right() as f32;
        (rect.y < bottom
            && rect.y + rect.h > TEXT_TOP
            && rect.x < right
            && self.survives_code_clip_left(rect.x, rect.w))
        .then_some(rect)
    }

    /// The pre-change follow decision (framing Q#F6): whether the own
    /// caret is ACTUALLY painted inside the drawable code area right
    /// now — and only while the minibuffer is closed (its caret lives
    /// in the band). Painted-before ⇒ the font/resize transaction
    /// re-follows with `ensure_caret_painted`; not painted ⇒ the
    /// user's scroll is preserved and never snapped back.
    fn caret_painted_in_code_clip(&mut self) -> bool {
        self.minibuffer.is_none() && self.code_caret_rect_in_clip().is_some()
    }

    /// Whether an arbitrary source `byte` is actually painted in the
    /// drawable code area — [`Self::code_caret_rect_in_clip`]'s test,
    /// generalized off the own cursor.
    ///
    /// The scroll indicator needs this and nothing weaker. Two cheaper
    /// predicates are available and both are wrong:
    ///
    /// - `view_range.0 == 0` / `view_range.1 == len` describe the
    ///   **shaped** span, which carries `SCROLL_OVERSCAN` source lines
    ///   past the window. A slice that merely reaches EOF says nothing
    ///   about whether EOF is on screen, so `Bot` would latch on early.
    ///   (This is exactly the guess that broke
    ///   `extreme_sizes_render_with_contained_popups` when it was tried.)
    /// - `scroll_top == 0` ignores `code_scroll_residual`, so scrolling
    ///   into the middle of a wrapped first line still claims `Top`.
    ///
    /// Asking where the byte lands answers both, because cosmic-text
    /// laid it out: wrapped continuation runs pushed below the band and
    /// overscan lines shaped past the bottom both fail the clip.
    fn code_byte_painted(&mut self, byte: u64) -> bool {
        let (vstart, vend) = self.view_range;
        if byte < vstart || byte > vend {
            return false;
        }
        // Deliberately no `vend <= vstart` rejection, though the caret
        // and completion-anchor paths both carry one. An empty range is
        // not the same as an empty layout: a file ending in a newline
        // has a final empty line, and a viewport parked on it shapes
        // one real row at `(len, len)`. Rejecting that reported
        // "neither end visible" — a percentage — at the exact moment
        // the user had scrolled to the bottom. `code_byte_px` already
        // returns `None` when nothing is shaped, which is the condition
        // that guard was reaching for.
        let Some((_x, top, line_height)) = self.code_byte_px(byte) else {
            return false;
        };
        let y = TEXT_TOP + top;
        let bottom = document_text_bottom(self.config.height, self.fm, self.band_inset());
        // Partial overlap counts as painted, the same rule the caret
        // clip uses. A first row half-scrolled under the top edge is
        // still legible, and a stricter test here would disagree with
        // the caret about the very same row.
        y < bottom && y + line_height > TEXT_TOP
    }

    /// Push one rect per visual line whose glyphs overlap the
    /// slice-relative source byte range `[lo, hi)`, spanning the
    /// matching projected glyphs' horizontal extent. Each source-line
    /// intersection is mapped through its cached chunks first, so a
    /// source tab covers every expanded space even when a soft wrap
    /// divides those spaces between visual runs.
    fn push_glyph_extent_rects(
        &self,
        rects: &mut Vec<MinimapRect>,
        line_offsets: &[u64],
        lo: u64,
        hi: u64,
        color: [f32; 4],
        bar_px: Option<f32>,
    ) {
        if hi <= lo {
            return;
        }
        for run in self.buffer.layout_runs() {
            let line_base = line_offsets.get(run.line_i).copied().unwrap_or(0);
            let line_end = line_offsets
                .get(run.line_i + 1)
                .copied()
                .unwrap_or(self.view_range.1 - self.view_range.0);
            let source_lo = lo.max(line_base);
            let source_hi = hi.min(line_end);
            if source_hi <= source_lo {
                continue;
            }
            let Some(chunks) = self.line_chunk_cache.get(run.line_i) else {
                continue;
            };
            let Some(projected_lo) = source_to_projected(chunks, source_lo - line_base) else {
                continue;
            };
            let Some(projected_hi) = source_to_projected(chunks, source_hi - line_base) else {
                continue;
            };
            // Q#MS11: a wash that INTERSECTS a suppressed span covers
            // the span's WHOLE reserved rectangle — the box has no
            // interior byte map, so a partial wash cannot be placed
            // honestly, and a match strictly inside the span would
            // otherwise produce a zero-width interval (both endpoints
            // collapse onto the box's left edge).
            let (projected_lo, projected_hi) = widen_over_math_chunks(
                chunks,
                (source_lo - line_base, source_hi - line_base),
                (projected_lo, projected_hi),
            );
            let mut min_x: Option<f32> = None;
            let mut max_x: Option<f32> = None;
            for glyph in run.glyphs {
                let g_start = glyph.start as u64;
                let g_end = glyph.end as u64;
                if g_end <= projected_lo || g_start >= projected_hi {
                    continue;
                }
                let x0 = glyph.x;
                let x1 = glyph.x + glyph.w;
                min_x = Some(min_x.map_or(x0, |v| v.min(x0)));
                max_x = Some(max_x.map_or(x1, |v| v.max(x1)));
            }
            if let (Some(x0), Some(x1)) = (min_x, max_x)
                && x1 > x0
            {
                // `bar_px`: a band hugging the bottom of the line box
                // instead of a full-height wash — the diagnostic
                // squiggle's geometry (T M4.6 parity; the squiggle
                // shape comes from the fragment shader, Q#W1).
                let (y, h) = match bar_px {
                    Some(bar) => (TEXT_TOP + run.line_top + run.line_height - bar, bar),
                    None => (TEXT_TOP + run.line_top, run.line_height),
                };
                // Stage 5: code-relative extents ride the offset, and
                // this quad batch has no scissor of its own — glyphon's
                // `TextBounds` covers the glyph layers only. So the
                // shared clip crops them at the gutter here.
                let Some((x, w)) = self.crop_to_code_clip_left(self.code_x_to_screen(x0), x1 - x0)
                else {
                    continue;
                };
                rects.push(MinimapRect { x, y, w, h, color });
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct MinimapRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: [f32; 4],
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct MinimapLineShape {
    indent_cols: usize,
    content_cols: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct RichChunk {
    text: String,
    color: Option<glyphon::Color>,
    /// Where this chunk's text came from — the seam the pointer
    /// hit-test walks back through (Q#M2).
    source: ChunkSource,
}

/// Origin of one [`RichChunk`] in the shaped (projected) text.
/// Offsets are slice-relative (the same space `projected_rich_chunks`
/// works in); the hit test rebases with the slice's `vstart`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChunkSource {
    /// Verbatim source text starting at this slice byte offset.
    Source { start: u64 },
    /// One source tab byte expanded into one or more projected spaces.
    SourceTab { start: u64 },
    /// Injected adornment text (inlay hint) anchored at this slice
    /// byte offset. Hits inside it snap to the anchor.
    Adornment { anchor: u64 },
    /// A suppressed inline-math span (Q#MS4). The chunk's text is SPACER
    /// spaces reserving the laid-out box's width — a `RichChunk`'s only
    /// width is its text, so there is no zero-glyph strut to reserve with
    /// (framing F2). `start`..`end` is the suppressed source range,
    /// delimiters included; hits inside snap to `start`, the same rule
    /// `Adornment` uses, because the box has no interior byte map.
    MathBox { start: u64, end: u64 },
}

/// One run of the projected→source hit map (Q#M2), built by
/// [`build_hit_runs`] from the same chunks `reshape` feeds glyphon —
/// so the map and the shaped buffer can never disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProjectedRun {
    /// Byte offset of this run in the shaped (projected) text.
    projected_start: u64,
    /// Run length in projected bytes.
    len: u64,
    source: ChunkSource,
}

/// One math span scheduled for suppression on a line: the detected
/// span (line-relative), the spacer text reserving its box's width
/// (Q#MS4), and the fitted box the draw pass paints.
#[derive(Clone, Debug, PartialEq)]
struct MathSubstitution {
    span: math_parse::MathSpan,
    spacer: String,
    boxed: math_layout::MathBox,
}

/// Per-line math state cached beside the line's chunks.
#[derive(Clone, Debug, PartialEq, Default)]
struct MathLineState {
    /// Every detected span with the Q#MS5/Q#MS11 gate bit it was
    /// built under (`true` = a caret/selection endpoint was inside,
    /// so the span rendered as source). The line-reuse predicate
    /// compares these against the CURRENT gate to refuse a retained
    /// line shaped under the opposite suppression state — the #120
    /// stale-line edge (framing acceptance 11).
    gates: Vec<(math_parse::MathSpan, bool)>,
    /// Suppressed spans' placements for the draw pass, in projected
    /// byte space within this shaped line.
    placed: Vec<PlacedMathBox>,
}

/// A suppressed span's box, addressed by its spacer's projected byte
/// range so the draw pass can find its pixel origin in the shaped
/// line's glyphs.
#[derive(Clone, Debug, PartialEq)]
struct PlacedMathBox {
    projected_start: u64,
    projected_len: u64,
    boxed: math_layout::MathBox,
}

#[cfg(test)]
mod math_chunk_tests {
    use super::*;

    fn spacer_chunk(start: u64, end: u64, spaces: usize) -> RichChunk {
        RichChunk {
            text: " ".repeat(spaces),
            color: None,
            source: ChunkSource::MathBox { start, end },
        }
    }

    /// Q#MS4: hits anywhere inside a suppressed span snap to its start, the
    /// same rule `Adornment` uses, because the box has no interior byte map.
    #[test]
    fn hits_inside_a_math_box_snap_to_the_span_start() {
        // `ab` + `$x^2$` suppressed to 3 spacer columns + `cd`
        let chunks = vec![
            RichChunk {
                text: "ab".to_owned(),
                color: None,
                source: ChunkSource::Source { start: 0 },
            },
            spacer_chunk(2, 7, 3),
            RichChunk {
                text: "cd".to_owned(),
                color: None,
                source: ChunkSource::Source { start: 7 },
            },
        ];
        let (runs, _) = build_hit_runs(&chunks);
        assert_eq!(projected_to_source(&runs, 0), Some(0));
        assert_eq!(projected_to_source(&runs, 1), Some(1));
        // Every boundary within the spacer maps to the span start (2).
        for projected in 2..=4 {
            assert_eq!(
                projected_to_source(&runs, projected),
                Some(2),
                "projected {projected} must snap to the span start"
            );
        }
        // Past the box, ordinary source mapping resumes.
        assert_eq!(projected_to_source(&runs, 5), Some(7));
        assert_eq!(projected_to_source(&runs, 6), Some(8));
    }

    /// The inverse direction: a caret anywhere in the suppressed range sits
    /// at the box's left edge, and text after it accounts for the full
    /// reserved width (acceptance 10's "shifts by the quantized difference").
    #[test]
    fn source_positions_inside_a_math_box_map_to_its_left_edge() {
        let chunks = vec![
            RichChunk {
                text: "ab".to_owned(),
                color: None,
                source: ChunkSource::Source { start: 0 },
            },
            spacer_chunk(2, 7, 3),
            RichChunk {
                text: "cd".to_owned(),
                color: None,
                source: ChunkSource::Source { start: 7 },
            },
        ];
        assert_eq!(source_to_projected(&chunks, 0), Some(0));
        assert_eq!(source_to_projected(&chunks, 2), Some(2));
        // Interior source bytes collapse onto the left edge. `end` (7) is
        // EXCLUSIVE and therefore NOT interior — an earlier revision of this
        // test asserted Some(2) for it and pinned the bug.
        assert_eq!(source_to_projected(&chunks, 4), Some(2));
        assert_eq!(source_to_projected(&chunks, 6), Some(2));
        assert_eq!(source_to_projected(&chunks, 7), Some(5), "end is exclusive");
        assert_eq!(source_to_projected(&chunks, 8), Some(6));
    }

    /// A box at the END of a line has no following chunk to catch the
    /// trailing boundary, which is exactly where the interior-snap rule used
    /// to send a click back to the span start.
    #[test]
    fn a_line_final_math_box_maps_its_trailing_boundary_after_the_span() {
        let chunks = vec![
            RichChunk {
                text: "ab".to_owned(),
                color: None,
                source: ChunkSource::Source { start: 0 },
            },
            spacer_chunk(2, 7, 3),
        ];
        let (runs, _) = build_hit_runs(&chunks);
        assert_eq!(projected_to_source(&runs, 2), Some(2), "interior snaps");
        assert_eq!(projected_to_source(&runs, 4), Some(2), "interior snaps");
        assert_eq!(
            projected_to_source(&runs, 5),
            Some(7),
            "the trailing boundary lands after the span, not on its start"
        );
    }

    /// A math chunk carries generated spacer text, so tab expansion must
    /// leave it alone rather than treating a space as a source tab.
    #[test]
    fn tab_expansion_preserves_a_math_chunk_untouched() {
        let chunks = vec![spacer_chunk(0, 5, 4)];
        let expanded = expand_chunk_tabs(chunks);
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].text, "    ");
        assert!(matches!(
            expanded[0].source,
            ChunkSource::MathBox { start: 0, end: 5 }
        ));
    }
}

/// Build the projected→source run map plus the projected text's line
/// start table (cosmic-text reports hits as line + byte-within-line).
fn build_hit_runs(chunks: &[RichChunk]) -> (Vec<ProjectedRun>, Vec<u64>) {
    let mut runs = Vec::with_capacity(chunks.len());
    let mut line_starts = vec![0u64];
    let mut projected = 0u64;
    for chunk in chunks {
        let len = chunk.text.len() as u64;
        runs.push(ProjectedRun {
            projected_start: projected,
            len,
            source: chunk.source,
        });
        for (i, b) in chunk.text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(projected + i as u64 + 1);
            }
        }
        projected += len;
    }
    (runs, line_starts)
}

/// Map a projected byte offset back to a slice-relative source byte.
/// A source tab's leading boundary maps before the byte; every
/// boundary inside its expanded spaces (including the trailing edge)
/// maps after it. Adornments snap to their left-gravity anchor.
fn projected_to_source(runs: &[ProjectedRun], projected: u64) -> Option<u64> {
    if runs.is_empty() {
        return None;
    }
    let idx = runs
        .partition_point(|r| r.projected_start <= projected)
        .saturating_sub(1);
    let run = runs[idx];
    let within = projected.saturating_sub(run.projected_start).min(run.len);
    match run.source {
        ChunkSource::Source { start } => Some(start + within),
        ChunkSource::SourceTab { start } => Some(start + u64::from(within > 0)),
        ChunkSource::Adornment { anchor } => Some(anchor),
        // Q#MS4: interior boundaries snap to the span start, since the box
        // has no interior byte map. The TRAILING boundary is not interior —
        // it maps after the span, matching `SourceTab`'s rule and keeping a
        // click past a line-final box off the span start.
        ChunkSource::MathBox { start, end } => Some(if within >= run.len { end } else { start }),
    }
}

/// Map a slice-relative source boundary into projected byte space.
/// This is the inverse boundary policy shared by caret placement and
/// horizontal decoration geometry. At an adornment anchor the earliest
/// projected boundary wins, preserving left gravity.
fn source_to_projected(chunks: &[RichChunk], source: u64) -> Option<u64> {
    let mut projected = 0u64;
    for chunk in chunks {
        let len = chunk.text.len() as u64;
        match chunk.source {
            ChunkSource::Source { start } => {
                if source <= start {
                    return Some(projected);
                }
                let end = start + len;
                if source <= end {
                    return Some(projected + source - start);
                }
            }
            ChunkSource::SourceTab { start } => {
                if source <= start {
                    return Some(projected);
                }
                if source <= start + 1 {
                    return Some(projected + len);
                }
            }
            ChunkSource::Adornment { anchor } => {
                if source <= anchor {
                    return Some(projected);
                }
            }
            ChunkSource::MathBox { end, .. } => {
                // `end` is EXCLUSIVE: it is the first byte AFTER the span, so
                // it must NOT claim the box's left edge. Letting it fall
                // through gives it the position past the reserved width —
                // which also keeps caret geometry continuous and stops a
                // search match starting at `end` from washing a box it does
                // not intersect (Q#MS11).
                if source < end {
                    return Some(projected);
                }
            }
        }
        projected += len;
    }
    (!chunks.is_empty()).then_some(projected)
}

fn minimap_left(surface_width: u32) -> Option<f32> {
    if surface_width < MINIMAP_MIN_SURFACE_WIDTH {
        return None;
    }
    let x = surface_width as f32 - MINIMAP_RIGHT - MINIMAP_WIDTH;
    (x > TEXT_LEFT + TEXT_RIGHT_GAP).then_some(x)
}

/// The pixel height an **installed** panel band takes off the document
/// text area: the panel's own rows plus the divider above them.
///
/// Zero whenever no `Present` panel is being painted, which is what makes
/// [`document_text_bottom`] reduce to the pre-panel boundary. It is a
/// newtype rather than a bare `f32` because three different boundaries
/// take a pixel height in this module and only one of them takes *this*
/// one; passing `status_band_height()` where this belongs would compile.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct PanelBandInset(f32);

impl PanelBandInset {
    /// The band that is not there.
    const ABSENT: Self = Self(0.0);

    /// The band an installed panel of `rows` rows occupies, divider
    /// included. Non-finite or negative inputs collapse to
    /// [`Self::ABSENT`] rather than propagating into a coordinate.
    fn installed(rows: u32, fm: FontMetrics) -> Self {
        let px = rows as f32 * fm.code_line_height() + fm.divider_height();
        if px.is_finite() && px > 0.0 {
            Self(px)
        } else {
            Self::ABSENT
        }
    }

    fn px(self) -> f32 {
        self.0
    }
}

/// Boundary 1 of 3 (Stage 2 framing §5.3): the top of the status band.
///
/// **This must not move when a panel band is installed.** The status
/// chrome stays pixel-identical at the physical window bottom; a band
/// grows upward from it. Every status-owned consumer reads this one — the
/// band background, both status text bounds, both `status_top`s, and all
/// of the minibuffer, which is global bufferless chrome anchored to the
/// band rather than to the document.
fn status_band_top(surface_height: u32, fm: FontMetrics) -> f32 {
    (surface_height as f32 - fm.status_band_height()).max(0.0)
}

/// Boundary 2 of 3: the bottom of the area this frontend may offer the
/// daemon as layout cells (Q#BP15a).
///
/// **The divider is subtracted even while the panel is absent**, and that
/// asymmetry against [`document_text_bottom`] is deliberate — it is what
/// breaks the first-open cycle. The daemon sizes a panel from the capacity
/// it was told about, so if the capacity ignored the divider, the first
/// panel it granted would not fit the space that actually appears once the
/// divider is painted alongside it. The document, meanwhile, loses no
/// pixels until a `Present` panel is really on screen.
fn geometry_capacity_bottom(surface_height: u32, fm: FontMetrics) -> f32 {
    (status_band_top(surface_height, fm) - fm.divider_height()).max(0.0)
}

/// Boundary 3 of 3: the bottom of the **document** text area.
///
/// Moves up by exactly `band.px()` when a panel is installed. Every
/// document-owned consumer reads this one: the code/math/gutter/terminal
/// clips, the code height, caret clipping, completion anchoring and
/// placement, the minimap, the visible-line estimate, the terminal cell
/// viewport, and edge scrolling.
fn document_text_bottom(surface_height: u32, fm: FontMetrics, band: PanelBandInset) -> f32 {
    (status_band_top(surface_height, fm) - band.px()).max(0.0)
}

/// The minimap's drawable height: the text area minus its own
/// top/bottom insets.
fn minimap_height(surface_height: u32, fm: FontMetrics, band: PanelBandInset) -> f32 {
    document_text_bottom(surface_height, fm, band) - MINIMAP_TOP - MINIMAP_BOTTOM
}

fn estimated_visible_lines(surface_height: u32, fm: FontMetrics, band: PanelBandInset) -> usize {
    ((document_text_bottom(surface_height, fm, band) - TEXT_TOP.max(0.0)) / fm.code_line_height())
        .ceil()
        .max(1.0) as usize
}

/// True when `(x, y)` lies inside the minimap band — the painter's
/// geometry (`minimap_left` × the `MINIMAP_TOP..bottom` column),
/// shared by the Q#M6 press hit-test.
fn minimap_band_contains(
    x: f32,
    y: f32,
    surface_width: u32,
    surface_height: u32,
    fm: FontMetrics,
    band: PanelBandInset,
) -> bool {
    let Some(left) = minimap_left(surface_width) else {
        return false;
    };
    let height = minimap_height(surface_height, fm, band);
    height > 0.0
        && x >= left
        && x < surface_width as f32 - MINIMAP_RIGHT
        && y >= MINIMAP_TOP
        && y < MINIMAP_TOP + height
}

/// Render a [`pmacs_protocol::scroll::ScrollPosition`] for the status
/// line. The classification is shared with the TUI; only this spelling
/// is local (framing §5d.6), so the two frontends cannot decide
/// differently but each stays free to present it.
fn render_scroll_position(pos: pmacs_protocol::scroll::ScrollPosition) -> String {
    use pmacs_protocol::scroll::ScrollPosition;
    match pos {
        ScrollPosition::All => "All".to_owned(),
        ScrollPosition::Top => "Top".to_owned(),
        ScrollPosition::Bot => "Bot".to_owned(),
        ScrollPosition::Percent(p) => format!("{p}%"),
    }
}

/// The TUI mode line's scroll readout, ported verbatim (Q#S1): "All"
/// when the buffer fits, "Top"/"Bot" at the extremes, else the cursor
/// row as a percentage of the file.
///
/// Only reached when wrapping is **off**. Under wrapping it is not
/// merely imprecise but wrong — it reckons in source lines while the
/// window holds visual rows — so that path goes through
/// [`render_scroll_position`] instead.
fn format_scroll_indicator(
    view_top: usize,
    visible: usize,
    total_lines: usize,
    cursor_row: usize,
) -> String {
    if total_lines <= 1 {
        return "All".to_string();
    }
    if visible > 0 {
        if visible >= total_lines {
            return "All".to_string();
        }
        if view_top == 0 {
            return "Top".to_string();
        }
        if view_top.saturating_add(visible) >= total_lines {
            return "Bot".to_string();
        }
    }
    let pct = (cursor_row + 1).saturating_mul(100) / total_lines;
    format!("{pct}%")
}

/// Q#M7 — which way (if any) a drag at pixel `y` should auto-scroll:
/// `-1` in the band hugging the text area's top, `+1` in the band at
/// the text area's bottom (above the status band), `None` in the
/// interior.
fn edge_scroll_direction(
    y: f32,
    surface_height: u32,
    fm: FontMetrics,
    band: PanelBandInset,
) -> Option<i64> {
    let bottom = document_text_bottom(surface_height, fm, band);
    if y < TEXT_TOP + EDGE_SCROLL_BAND {
        Some(-1)
    } else if band.px() > 0.0 && y >= bottom {
        // Moving the boundary is necessary but NOT sufficient: this arm has
        // no upper bound, so a pixel far below the text area still reads as
        // "further down the document". With a band installed that pixel is
        // on ANOTHER SURFACE, and letting it arm the document's auto-scroll
        // is the named symptom of leaving this consumer on the old bottom —
        // the document scrolls while the pointer is inside the panel.
        //
        // Gated on an installed band so a bandless surface keeps its exact
        // previous behavior, including over the status band.
        None
    } else if y > bottom - EDGE_SCROLL_BAND {
        Some(1)
    } else {
        None
    }
}

/// Map a minimap pixel `y` to a whole-file source line — the inverse
/// of the painter's `y = MINIMAP_TOP + line * height / total`
/// interpolation, clamped into the file. `None` for an empty file or
/// a degenerate surface.
fn minimap_y_to_line(
    y: f32,
    surface_height: u32,
    total_lines: usize,
    fm: FontMetrics,
    band: PanelBandInset,
) -> Option<usize> {
    if total_lines == 0 {
        return None;
    }
    let height = minimap_height(surface_height, fm, band);
    if height <= 0.0 {
        return None;
    }
    let frac = ((y - MINIMAP_TOP) / height).clamp(0.0, 1.0);
    Some(((frac * total_lines as f32) as usize).min(total_lines - 1))
}

#[allow(
    clippy::too_many_arguments,
    reason = "one painter's complete geometry inputs; the band inset is the eighth"
)]
fn minimap_rects(
    lines: &[CellStyle],
    shapes: &[MinimapLineShape],
    surface_width: u32,
    surface_height: u32,
    first_visible_line: usize,
    visible_lines: usize,
    fm: FontMetrics,
    band: PanelBandInset,
) -> Vec<MinimapRect> {
    let Some(x) = minimap_left(surface_width) else {
        return Vec::new();
    };
    if lines.is_empty() || minimap_height(surface_height, fm, band) <= 0.0 {
        return Vec::new();
    }
    let height = minimap_height(surface_height, fm, band);
    let pixel_rows = height.round().max(1.0) as usize;
    let mut rects = Vec::new();
    rects.push(MinimapRect {
        x,
        y: MINIMAP_TOP,
        w: MINIMAP_WIDTH,
        h: height,
        color: MINIMAP_BG,
    });

    if lines.len() <= pixel_rows {
        for (idx, style) in lines.iter().copied().enumerate() {
            let y0 = MINIMAP_TOP + idx as f32 * height / lines.len() as f32;
            let y1 = MINIMAP_TOP + (idx + 1) as f32 * height / lines.len() as f32;
            if let Some(shape) = shapes
                .get(idx)
                .copied()
                .filter(MinimapLineShape::has_content)
            {
                push_minimap_line_stroke(
                    &mut rects,
                    x,
                    y0,
                    (y1 - y0).clamp(1.0, MINIMAP_MAX_LINE_STROKE_HEIGHT),
                    minimap_style_color(style),
                    shape,
                );
            }
        }
    } else {
        for row in 0..pixel_rows {
            let line_start = row * lines.len() / pixel_rows;
            let line_end = ((row + 1) * lines.len())
                .div_ceil(pixel_rows)
                .min(lines.len());
            let y0 = MINIMAP_TOP + row as f32 * height / pixel_rows as f32;
            let y1 = MINIMAP_TOP + (row + 1) as f32 * height / pixel_rows as f32;
            if let Some(shape) = dominant_line_shape(shapes, line_start, line_end) {
                push_minimap_line_stroke(
                    &mut rects,
                    x,
                    y0,
                    (y1 - y0).max(1.0),
                    minimap_style_color(dominant_line_style(&lines[line_start..line_end])),
                    shape,
                );
            }
        }
    }

    push_minimap_thumb(
        &mut rects,
        x,
        height,
        lines.len(),
        first_visible_line,
        visible_lines,
    );
    rects
}

impl MinimapLineShape {
    fn has_content(&self) -> bool {
        self.content_cols > 0
    }
}

fn push_minimap_line_stroke(
    rects: &mut Vec<MinimapRect>,
    x: f32,
    y: f32,
    h: f32,
    color: [f32; 4],
    shape: MinimapLineShape,
) {
    if !shape.has_content() {
        return;
    }
    let available = (MINIMAP_WIDTH - MINIMAP_H_PAD * 2.0).max(MINIMAP_MIN_STROKE_WIDTH);
    let indent = (shape.indent_cols as f32 / MINIMAP_CODE_COLS * available)
        .min((available - MINIMAP_MIN_STROKE_WIDTH).max(0.0));
    let width = (shape.content_cols as f32 / MINIMAP_CODE_COLS * available).clamp(
        MINIMAP_MIN_STROKE_WIDTH,
        (available - indent).max(MINIMAP_MIN_STROKE_WIDTH),
    );
    rects.push(MinimapRect {
        x: x + MINIMAP_H_PAD + indent,
        y,
        w: width,
        h,
        color,
    });
}

fn push_minimap_thumb(
    rects: &mut Vec<MinimapRect>,
    x: f32,
    minimap_height: f32,
    line_count: usize,
    first_visible_line: usize,
    visible_lines: usize,
) {
    let start_line = first_visible_line.min(line_count);
    let end_line = start_line
        .saturating_add(visible_lines.max(1))
        .min(line_count);
    let mut y0 = MINIMAP_TOP + start_line as f32 * minimap_height / line_count as f32;
    let mut y1 = MINIMAP_TOP + end_line as f32 * minimap_height / line_count as f32;
    if y1 - y0 < MINIMAP_MIN_THUMB_HEIGHT {
        let mid = (y0 + y1) * 0.5;
        y0 = (mid - MINIMAP_MIN_THUMB_HEIGHT * 0.5).max(MINIMAP_TOP);
        y1 = (y0 + MINIMAP_MIN_THUMB_HEIGHT).min(MINIMAP_TOP + minimap_height);
        y0 = (y1 - MINIMAP_MIN_THUMB_HEIGHT).max(MINIMAP_TOP);
    }
    let h = (y1 - y0).max(1.0);
    rects.push(MinimapRect {
        x,
        y: y0,
        w: MINIMAP_WIDTH,
        h,
        color: MINIMAP_THUMB_FILL,
    });
    rects.push(MinimapRect {
        x,
        y: y0,
        w: 1.0,
        h,
        color: MINIMAP_THUMB_BORDER,
    });
    rects.push(MinimapRect {
        x: x + MINIMAP_WIDTH - 1.0,
        y: y0,
        w: 1.0,
        h,
        color: MINIMAP_THUMB_BORDER,
    });
}

fn dominant_line_style(lines: &[CellStyle]) -> CellStyle {
    if lines.is_empty() {
        return CellStyle::default();
    }
    let mut tally: Vec<(CellStyle, usize)> = Vec::new();
    for style in lines {
        if let Some((_, count)) = tally.iter_mut().find(|(candidate, _)| candidate == style) {
            *count += 1;
        } else {
            tally.push((*style, 1));
        }
    }
    tally
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map_or(CellStyle::default(), |(style, _)| style)
}

fn dominant_line_shape(
    shapes: &[MinimapLineShape],
    line_start: usize,
    line_end: usize,
) -> Option<MinimapLineShape> {
    let slice = shapes.get(line_start.min(shapes.len())..line_end.min(shapes.len()))?;
    let mut count = 0usize;
    let mut indent_sum = 0usize;
    let mut content_sum = 0usize;
    for shape in slice.iter().filter(|shape| shape.has_content()) {
        count += 1;
        indent_sum += shape.indent_cols;
        content_sum += shape.content_cols;
    }
    // `then`, NOT `then_some`: `bool::then_some` takes its argument by
    // value, so the struct literal --- and with it `indent_sum / count`
    // --- is evaluated before the guard is ever consulted. A slab of
    // all-blank source lines makes `count` zero and panics the frontend
    // on the division. `bool::then` defers the body into a closure, so
    // the zero case short-circuits to `None`.
    //
    // Clippy's `unnecessary_lazy_evaluations` lint pushes in exactly the
    // wrong direction here; it does not fire on a body that can panic,
    // but do not "simplify" this back.
    (count > 0).then(|| MinimapLineShape {
        indent_cols: indent_sum / count,
        content_cols: content_sum.div_ceil(count),
    })
}

fn minimap_line_shapes(text: &str) -> Vec<MinimapLineShape> {
    text.split('\n').map(minimap_line_shape).collect()
}

fn minimap_line_shape(line: &str) -> MinimapLineShape {
    let mut total_cols = 0usize;
    let mut indent_cols = 0usize;
    let mut in_indent = true;
    for ch in line.trim_end_matches('\r').chars() {
        let next_col = advance_minimap_col(total_cols, ch);
        if in_indent && (ch == ' ' || ch == '\t') {
            indent_cols = next_col;
        } else {
            in_indent = false;
        }
        total_cols = next_col;
    }
    MinimapLineShape {
        indent_cols,
        content_cols: total_cols.saturating_sub(indent_cols),
    }
}

fn advance_minimap_col(col: usize, ch: char) -> usize {
    if ch == '\t' {
        let tab_stop = TAB_STOP_COLUMNS as usize;
        col + tab_stop - col % tab_stop
    } else {
        col + UnicodeWidthChar::width(ch).unwrap_or(0)
    }
}

fn minimap_style_color(style: CellStyle) -> [f32; 4] {
    // A set underline_color is the producer's diagnostic mark for the
    // line (protocol v6, T M4.6 parity) — the minimap's gutter sign.
    // It outranks the syntax-dominant fg so error/warning lines read
    // at a glance.
    let color = match style.underline_color {
        CellColor::Default => style.fg,
        marked => marked,
    };
    match color {
        CellColor::Default => MINIMAP_DEFAULT_LINE,
        CellColor::Rgb(r, g, b) => rgb_to_minimap_color(r, g, b),
        CellColor::Indexed(idx) => {
            let c = indexed_to_glyphon(idx);
            rgb_to_minimap_color(c.r(), c.g(), c.b())
        }
    }
}

fn rgb_to_minimap_color(r: u8, g: u8, b: u8) -> [f32; 4] {
    [
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        0.9,
    ]
}

/// One-shot env flag: `PMACS_GPU_DEBUG_PRESENCE=1` logs each received
/// `PresenceUpdate`. Read once (the env lock is not free per call) and
/// cached for the process lifetime.
fn debug_presence() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("PMACS_GPU_DEBUG_PRESENCE").is_some())
}

/// One-shot env flag: `PMACS_GPU_DEBUG_FRAME=1` logs per-`render()`
/// sub-phase timings (background rects, minimap rects, glyph prepare,
/// total) so a perceived cursor-tracking slowdown can be localized to
/// a specific phase.
fn debug_frame() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("PMACS_GPU_DEBUG_FRAME").is_some())
}

/// One-shot env flag: `PMACS_GPU_DEBUG_APPLY=1` logs how long the
/// main thread spends applying each inbound daemon message. This
/// separates CRDT text patching, style replacement, and cursor updates
/// from the later `render()` timings.
fn debug_apply() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("PMACS_GPU_DEBUG_APPLY").is_some())
}

fn instance_message_label(msg: &InstanceMessage) -> &'static str {
    match msg {
        InstanceMessage::CellDelta { .. } => "CellDelta",
        InstanceMessage::LineWrapFacts { .. } => "LineWrapFacts",
        InstanceMessage::Cursor(_) => "Cursor",
        InstanceMessage::ModeLine(_) => "ModeLine",
        InstanceMessage::Signal(_) => "Signal",
        InstanceMessage::Goodbye(_) => "Goodbye",
        InstanceMessage::CrdtOp { .. } => "CrdtOp",
        InstanceMessage::PresenceUpdate { .. } => "PresenceUpdate",
        InstanceMessage::BufferSnapshot { .. } => "BufferSnapshot",
        InstanceMessage::CursorByte { .. } => "CursorByte",
        InstanceMessage::StyleSpans { .. } => "StyleSpans",
        InstanceMessage::Decorations { .. } => "Decorations",
        InstanceMessage::InlineAdornments { .. } => "InlineAdornments",
        InstanceMessage::FileStyleSummary { .. } => "FileStyleSummary",
        InstanceMessage::StatusFacts { .. } => "StatusFacts",
        InstanceMessage::SearchPrompt { .. } => "SearchPrompt",
        InstanceMessage::MenuPrompt { .. } => "MenuPrompt",
        InstanceMessage::MinibufferPrompt { .. } => "MinibufferPrompt",
        InstanceMessage::MinibufferPromptRows { .. } => "MinibufferPromptRows",
        InstanceMessage::BlockAdornments { .. } => "BlockAdornments",
        InstanceMessage::FoldState { .. } => "FoldState",
        InstanceMessage::ResourceOffer { .. } => "ResourceOffer",
        InstanceMessage::DispatchIdle { .. } => "DispatchIdle",
        InstanceMessage::LineNumbers { .. } => "LineNumbers",
        InstanceMessage::CompletionPopup { .. } => "CompletionPopup",
        InstanceMessage::ThemeFacts { .. } => "ThemeFacts",
        InstanceMessage::FontFacts { .. } => "FontFacts",
        InstanceMessage::StatuslineSegments { .. } => "StatuslineSegments",
        InstanceMessage::TerminalFrame(_) => "TerminalFrame",
        InstanceMessage::InitialTargetResult(_) => "InitialTargetResult",
        InstanceMessage::PanelFrame(_) => "PanelFrame",
    }
}

/// One-shot env flag: `PMACS_GPU_DEBUG_INPUT=1` logs the input path —
/// keys sent and `CursorByte` received (with the buffer it targets vs
/// the buffer being displayed). The buffer comparison is the B1
/// diagnostic: if `CursorByte` targets a different buffer than
/// `current`, the caret won't track (the displayed/edited buffers are
/// out of sync).
fn debug_input() -> bool {
    static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("PMACS_GPU_DEBUG_INPUT").is_some())
}

/// Translate a winit logical key + current modifier state into a
/// protocol `(Key, Modifiers)`. Returns `None` for keys the protocol
/// has no representation for (the daemon ignores `Key::Unknown`, so
/// there's no value in forwarding them). `translate_key` covers the
/// full editing set; session B1 gates the send on [`is_motion_key`].
fn translate_mods(mods: winit::keyboard::ModifiersState) -> Modifiers {
    let mut bits = 0u8;
    if mods.shift_key() {
        bits |= Modifiers::SHIFT.bits();
    }
    if mods.control_key() {
        bits |= Modifiers::CTRL.bits();
    }
    if mods.alt_key() {
        bits |= Modifiers::ALT.bits();
    }
    if mods.super_key() {
        bits |= Modifiers::META.bits();
    }
    Modifiers::from_bits_truncate(bits)
}

fn translate_key(
    logical: &Key,
    mods: winit::keyboard::ModifiersState,
) -> Option<(ProtocolKey, Modifiers)> {
    let pmods = translate_mods(mods);

    let pkey = match logical {
        Key::Named(named) => match named {
            NamedKey::ArrowLeft => ProtocolKey::Left,
            NamedKey::ArrowRight => ProtocolKey::Right,
            NamedKey::ArrowUp => ProtocolKey::Up,
            NamedKey::ArrowDown => ProtocolKey::Down,
            NamedKey::Home => ProtocolKey::Home,
            NamedKey::End => ProtocolKey::End,
            NamedKey::PageUp => ProtocolKey::PageUp,
            NamedKey::PageDown => ProtocolKey::PageDown,
            NamedKey::Backspace => ProtocolKey::Backspace,
            NamedKey::Enter => ProtocolKey::Enter,
            NamedKey::Delete => ProtocolKey::Delete,
            NamedKey::Insert => ProtocolKey::Insert,
            NamedKey::Tab => ProtocolKey::Tab,
            NamedKey::Space => ProtocolKey::Char(' '),
            _ => return None,
        },
        Key::Character(s) => ProtocolKey::Char(s.chars().next()?),
        _ => return None,
    };
    Some((pkey, pmods))
}

/// Cursor-motion keys — forwarded with any modifier set (e.g. `C-Left`
/// is word-motion, `S-Down` extends a selection; the daemon's keymap
/// decides).
fn is_motion_key(key: ProtocolKey) -> bool {
    matches!(
        key,
        ProtocolKey::Left
            | ProtocolKey::Right
            | ProtocolKey::Up
            | ProtocolKey::Down
            | ProtocolKey::Home
            | ProtocolKey::End
            | ProtocolKey::PageUp
            | ProtocolKey::PageDown
    )
}

/// Whether to forward a translated key to the daemon (session B2).
/// Motion keys go through with any modifiers (C-<left> is word
/// motion). Deletion keys do too: C-BS / C-DEL / M-BS are word-level
/// deletes in the default keymap — the same editing-command family as
/// chorded motion, and an unbound chord is a harmless no-op at the
/// daemon keymap. (Chorded deletes never apply optimistically:
/// `optimistic_delete_range` requires empty modifiers, so they always
/// round-trip into their bound commands.) The remaining text keys
/// (`Char` / `Enter` / `Tab`) go through only *without* a
/// Ctrl/Alt/Meta chord modifier: a bare key edits text, but those
/// chords drive commands and minibuffer flows the GUI can't render or
/// interact with yet (deferred to a later session). Shift is not a
/// chord modifier — `Shift`+a already arrives as `Char('A')`.
fn should_forward_key(key: ProtocolKey, mods: Modifiers) -> bool {
    if is_motion_key(key) {
        return true;
    }
    if matches!(key, ProtocolKey::Backspace | ProtocolKey::Delete) {
        return true;
    }
    if !is_plain_text_modifiers(mods) {
        return false;
    }
    matches!(
        key,
        ProtocolKey::Char(_) | ProtocolKey::Enter | ProtocolKey::Tab
    )
}

/// A command chord (Q#GC1): a `Char` / `Enter` / `Tab` with `Ctrl` or
/// `Alt` held. These drive the daemon keymap (motion like `C-a`,
/// commands like `M-f` / `C-x C-s`, isearch `C-s`, clipboard `M-w`,
/// `M-x`, …) and are forwarded to it, subsuming the old per-feature
/// allowlists. `Char + Ctrl/Alt` is the exact set `should_forward_key`
/// withholds; motion / `Backspace` / `Delete` keep their own path, and
/// `Meta`/`Super`-only chords (no `Ctrl`/`Alt`) are left to the OS.
/// `Ctrl-V` is intercepted for OS paste before this is reached.
fn is_command_chord(key: ProtocolKey, mods: Modifiers) -> bool {
    matches!(
        key,
        ProtocolKey::Char(_) | ProtocolKey::Enter | ProtocolKey::Tab
    ) && (mods.contains(Modifiers::CTRL) || mods.contains(Modifiers::ALT))
}

/// Whether a keypress is **`AltGr` layout text** (audit F-004): winit
/// produced printable `text` while **both `Ctrl` and `Alt`** are held.
/// `AltGr` is `Ctrl+Alt` on Windows (the OS synthesizes LCtrl+RAlt), and
/// on such layouts the produced character (`@`, `€`, `{`, …) would
/// otherwise be misclassified as a command chord; when this is true the
/// caller strips the command modifiers so it inserts.
///
/// The gate is deliberately `Ctrl+Alt`, **not** "any command modifier":
/// `Alt` alone is *not* `AltGr`. On macOS the `Option` key is reported as
/// `Alt` and produces printable text for most letters (`Option+x` → "≈"),
/// but Option-as-Meta is exactly how the GUI reaches `M-x` / `M-f` / … —
/// stripping `Alt`-alone would swallow every macOS Meta chord. Genuine
/// `AltGr` needs both modifiers, so requiring both leaves `Alt`-alone
/// (macOS `Option`, plain `Meta`) to forward as command chords. Returns
/// `false` for genuine command chords (no text, or a control char) and
/// for plain text (no command modifier — already handled).
fn is_layout_text(text: Option<&str>, mods: Modifiers) -> bool {
    mods.contains(Modifiers::CTRL)
        && mods.contains(Modifiers::ALT)
        && text.is_some_and(|t| !t.is_empty() && t.chars().all(|c| !c.is_control()))
}

fn is_plain_text_modifiers(mods: Modifiers) -> bool {
    !mods.contains(Modifiers::CTRL)
        && !mods.contains(Modifiers::ALT)
        && !mods.contains(Modifiers::META)
        && !mods.contains(Modifiers::HYPER)
}

/// Clip a whole-file byte range `[start, end)` to the visible slice
/// `[vstart, vend)` and rebase it into slice coordinates (subtract
/// `vstart`). Returns `None` when the range is disjoint from the slice.
/// The single rebasing primitive for session S1 — caret and washes
/// route through it (Q#S4).
fn clip_rebase_range(start: u64, end: u64, vstart: u64, vend: u64) -> Option<(u64, u64)> {
    let s = start.max(vstart);
    let e = end.min(vend);
    if e <= s {
        return None;
    }
    Some((s - vstart, e - vstart))
}

/// Sum of the doc's per-peer version-vector counters — the **same
/// formula** as the daemon's `CrdtState::version_scalar`, which is
/// what the producer stamps into `StyleSpans` / `Decorations`
/// `generation`. The sum is integration-order independent, so once
/// both replicas hold the same set of ops the scalars are equal;
/// that is what makes frame generations comparable against locally
/// computed values in `unconfirmed_edits`.
fn loro_version_scalar(doc: &loro::LoroDoc) -> u64 {
    doc.oplog_vv()
        .values()
        .map(|counter| u64::try_from(*counter).unwrap_or(0))
        .sum()
}

/// Translate one incoming `StyleSpans` frame's segments through the
/// optimistic edits the daemon had not yet integrated when it
/// computed the frame. Ranges that a (defensive) delete fully
/// removes drop out.
fn translate_style_segments(
    segments: Vec<StyleSegment>,
    edits: &[(u64, TextProjectionEdit)],
) -> Vec<StyleSegment> {
    if edits.is_empty() {
        return segments;
    }
    segments
        .into_iter()
        .filter_map(|seg| {
            let mut range = seg.range;
            let mut spans = seg.spans;
            for (_, edit) in edits {
                range = translate_byte_range(range, *edit)?;
                spans = spans
                    .into_iter()
                    .filter_map(|mut sp| {
                        sp.range = translate_byte_range(sp.range, *edit)?;
                        Some(sp)
                    })
                    .collect();
            }
            Some(StyleSegment { range, spans })
        })
        .collect()
}

/// `Decorations` twin of [`translate_style_segments`].
fn translate_decoration_segments(
    segments: Vec<DecorationSegment>,
    edits: &[(u64, TextProjectionEdit)],
) -> Vec<DecorationSegment> {
    if edits.is_empty() {
        return segments;
    }
    segments
        .into_iter()
        .filter_map(|seg| {
            let mut range = seg.range;
            let mut decorations = seg.decorations;
            for (_, edit) in edits {
                range = translate_byte_range(range, *edit)?;
                decorations = decorations
                    .into_iter()
                    .filter_map(|mut d| {
                        d.range = translate_byte_range(d.range, *edit)?;
                        Some(d)
                    })
                    .collect();
            }
            Some(DecorationSegment { range, decorations })
        })
        .collect()
}

fn subscribe_loro_text(doc: &loro::LoroDoc) -> (LoroTextDeltaBatches, loro::Subscription) {
    let text = doc.get_text(LORO_TEXT_CONTAINER);
    let delta_batches = Arc::new(Mutex::new(Vec::<Vec<loro::TextDelta>>::new()));
    let captured_batches = Arc::clone(&delta_batches);
    let subscription = doc.subscribe(
        &text.id(),
        Arc::new(move |event| {
            let mut guard = captured_batches
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for event in event.events {
                if let Some(delta) = event.diff.as_text()
                    && !delta.is_empty()
                {
                    guard.push(delta.clone());
                }
            }
        }),
    );
    (delta_batches, subscription)
}

fn clear_loro_text_delta_batches(delta_batches: &LoroTextDeltaBatches) {
    delta_batches
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

fn drain_loro_text_delta_batches(
    delta_batches: &LoroTextDeltaBatches,
) -> Vec<Vec<loro::TextDelta>> {
    let mut guard = delta_batches
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    std::mem::take(&mut *guard)
}

/// Largest char-boundary `<= index` (stable equivalent of the unstable
/// `str::floor_char_boundary`). Used to snap externally-supplied byte
/// offsets to valid slice points so a stale, mid-codepoint offset can't
/// panic a `text[..]` slice.
fn floor_char_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let mut i = index;
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Buffer-absolute byte offset of the start of each `\n`-delimited
/// line (index 0 = byte 0). Indexed by cosmic-text's
/// `LayoutRun::line_i` to rebase line-relative glyph offsets.
fn line_byte_offsets(text: &str) -> Vec<u64> {
    line_offset_tables(text).0
}

fn line_offset_tables(text: &str) -> (Vec<u64>, Vec<u64>) {
    let mut starts = vec![0u64];
    let mut char_starts = vec![0u64];
    let mut chars_seen = 0u64;
    for (byte, ch) in text.char_indices() {
        chars_seen += 1;
        if ch == '\n' {
            starts.push(byte as u64 + 1);
            char_starts.push(chars_seen);
        }
    }
    (starts, char_starts)
}

fn byte_offset_for_char_offset(
    text: &str,
    line_starts: &[u64],
    line_char_starts: &[u64],
    char_offset: usize,
) -> Option<usize> {
    if line_starts.len() != line_char_starts.len() {
        return None;
    }
    let line = line_char_starts
        .partition_point(|&start| start <= char_offset as u64)
        .saturating_sub(1);
    let byte_start = *line_starts.get(line)? as usize;
    let char_start = *line_char_starts.get(line)? as usize;
    let mut byte = byte_start;
    for _ in 0..char_offset.checked_sub(char_start)? {
        let ch = text.get(byte..)?.chars().next()?;
        byte += ch.len_utf8();
    }
    Some(byte)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TextProjectionEdit {
    start: u64,
    old_end: u64,
    inserted_len: u64,
}

fn apply_loro_text_delta_batches(
    text: &mut String,
    line_starts: &mut Vec<u64>,
    line_char_starts: &mut Vec<u64>,
    delta_batches: &[Vec<loro::TextDelta>],
) -> Result<Vec<TextProjectionEdit>, &'static str> {
    let mut edits = Vec::new();
    for delta in delta_batches {
        apply_loro_text_delta_batch(text, line_starts, line_char_starts, delta, &mut edits)?;
    }
    Ok(edits)
}

fn apply_loro_text_delta_batch(
    text: &mut String,
    line_starts: &mut Vec<u64>,
    line_char_starts: &mut Vec<u64>,
    delta: &[loro::TextDelta],
    edits: &mut Vec<TextProjectionEdit>,
) -> Result<(), &'static str> {
    let mut cursor_char = 0usize;
    for op in delta {
        match op {
            loro::TextDelta::Retain { retain, .. } => {
                cursor_char = cursor_char
                    .checked_add(*retain)
                    .ok_or("retain offset overflow")?;
            }
            loro::TextDelta::Insert { insert, .. } => {
                if insert.is_empty() {
                    continue;
                }
                let start_byte =
                    byte_offset_for_char_offset(text, line_starts, line_char_starts, cursor_char)
                        .ok_or("insert offset outside current text")?;
                replace_text_range_with_line_updates(
                    text,
                    line_starts,
                    line_char_starts,
                    start_byte,
                    start_byte,
                    cursor_char,
                    cursor_char,
                    insert,
                )?;
                edits.push(TextProjectionEdit {
                    start: start_byte as u64,
                    old_end: start_byte as u64,
                    inserted_len: insert.len() as u64,
                });
                cursor_char = cursor_char
                    .checked_add(insert.chars().count())
                    .ok_or("insert offset overflow")?;
            }
            loro::TextDelta::Delete { delete } => {
                if *delete == 0 {
                    continue;
                }
                let start_char = cursor_char;
                let end_char = cursor_char
                    .checked_add(*delete)
                    .ok_or("delete offset overflow")?;
                let start_byte =
                    byte_offset_for_char_offset(text, line_starts, line_char_starts, start_char)
                        .ok_or("delete start outside current text")?;
                let end_byte =
                    byte_offset_for_char_offset(text, line_starts, line_char_starts, end_char)
                        .ok_or("delete end outside current text")?;
                replace_text_range_with_line_updates(
                    text,
                    line_starts,
                    line_char_starts,
                    start_byte,
                    end_byte,
                    start_char,
                    end_char,
                    "",
                )?;
                edits.push(TextProjectionEdit {
                    start: start_byte as u64,
                    old_end: end_byte as u64,
                    inserted_len: 0,
                });
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn replace_text_range_with_line_updates(
    text: &mut String,
    line_starts: &mut Vec<u64>,
    line_char_starts: &mut Vec<u64>,
    start_byte: usize,
    end_byte: usize,
    start_char: usize,
    end_char: usize,
    insert: &str,
) -> Result<(), &'static str> {
    if line_starts.len() != line_char_starts.len() {
        return Err("line offset tables have different lengths");
    }
    if start_byte > end_byte || end_byte > text.len() {
        return Err("replacement byte range is outside current text");
    }
    if start_char > end_char {
        return Err("replacement char range is inverted");
    }
    if !text.is_char_boundary(start_byte) || !text.is_char_boundary(end_byte) {
        return Err("replacement byte range is not on char boundaries");
    }

    let start_line = line_starts
        .partition_point(|&start| start <= start_byte as u64)
        .saturating_sub(1);
    let remove_start = start_line + 1;
    let remove_end = line_starts.partition_point(|&start| start <= end_byte as u64);
    let (inserted_line_starts, inserted_line_char_starts) =
        inserted_line_offsets(insert, start_byte, start_char);
    let inserted_line_count = inserted_line_starts.len();
    let byte_delta = signed_usize_delta(insert.len(), end_byte - start_byte)?;
    let char_delta = signed_usize_delta(insert.chars().count(), end_char - start_char)?;

    text.replace_range(start_byte..end_byte, insert);
    line_starts.splice(remove_start..remove_end, inserted_line_starts);
    line_char_starts.splice(remove_start..remove_end, inserted_line_char_starts);
    let suffix_start = remove_start + inserted_line_count;
    for start in line_starts.iter_mut().skip(suffix_start) {
        shift_u64(start, byte_delta);
    }
    for start in line_char_starts.iter_mut().skip(suffix_start) {
        shift_u64(start, char_delta);
    }
    Ok(())
}

fn inserted_line_offsets(
    insert: &str,
    start_byte: usize,
    start_char: usize,
) -> (Vec<u64>, Vec<u64>) {
    let mut line_starts = Vec::new();
    let mut line_char_starts = Vec::new();
    let mut chars_seen = 0usize;
    for (rel_byte, ch) in insert.char_indices() {
        chars_seen += 1;
        if ch == '\n' {
            line_starts.push((start_byte + rel_byte + 1) as u64);
            line_char_starts.push((start_char + chars_seen) as u64);
        }
    }
    (line_starts, line_char_starts)
}

fn shift_u64(value: &mut u64, delta: i64) {
    if delta >= 0 {
        *value = value.saturating_add(delta as u64);
    } else {
        *value = value.saturating_sub(delta.unsigned_abs());
    }
}

fn signed_usize_delta(new_len: usize, old_len: usize) -> Result<i64, &'static str> {
    let new_len = i64::try_from(new_len).map_err(|_| "new length exceeds i64")?;
    let old_len = i64::try_from(old_len).map_err(|_| "old length exceeds i64")?;
    Ok(new_len - old_len)
}

fn translate_style_spans(spans: &mut Vec<StyleSpan>, edit: TextProjectionEdit) {
    let mut translated = Vec::with_capacity(spans.len());
    for mut span in spans.drain(..) {
        if let Some(range) = translate_byte_range(span.range, edit) {
            span.range = range;
            translated.push(span);
        }
    }
    *spans = translated;
}

fn translate_decorations(decorations: &mut Vec<Decoration>, edit: TextProjectionEdit) {
    let mut translated = Vec::with_capacity(decorations.len());
    for mut decoration in decorations.drain(..) {
        // Optimistic clear (diagnostics only): an edit that touches a
        // diagnostic's range invalidates it locally, so drop the
        // squiggle now instead of holding a stale wave over the text
        // you just changed until the LSP re-analyzes and republishes.
        // Scoped to the *touched* diagnostic — an error elsewhere
        // still translates and holds, so the no-blink benefit of the
        // producer's hold-while-stale survives. Non-diagnostic
        // decorations (selection / current-line) always translate.
        if decoration_kind_to_underline_color(decoration.kind).is_some()
            && edit_touches_range(decoration.range, edit)
        {
            continue;
        }
        if let Some(range) = translate_byte_range(decoration.range, edit) {
            decoration.range = range;
            translated.push(decoration);
        }
    }
    *decorations = translated;
}

/// True when `edit` touches `range` in the pre-edit coordinate space.
/// An insert (`old_end == start`) touches when its point lies within
/// `[start, end]` (inclusive — typing at either edge of an error
/// token counts); a delete/replace touches when its span overlaps.
fn edit_touches_range(range: ByteRange, edit: TextProjectionEdit) -> bool {
    edit.start <= range.end && range.start <= edit.old_end
}

fn translate_inline_adornments(adornments: &mut [InlineAdornment], edit: TextProjectionEdit) {
    for adornment in adornments {
        adornment.at = translate_byte_position(adornment.at, edit);
    }
}

fn translate_byte_range(range: ByteRange, edit: TextProjectionEdit) -> Option<ByteRange> {
    let start = translate_range_start(range.start, edit);
    let end = translate_range_end(range.end, edit);
    (start < end).then_some(ByteRange { start, end })
}

fn translate_range_start(pos: u64, edit: TextProjectionEdit) -> u64 {
    if edit.old_end == edit.start {
        if pos >= edit.start {
            pos.saturating_add(edit.inserted_len)
        } else {
            pos
        }
    } else if pos <= edit.start {
        pos
    } else if pos >= edit.old_end {
        shift_position(pos, edit)
    } else {
        edit.start
    }
}

fn translate_range_end(pos: u64, edit: TextProjectionEdit) -> u64 {
    if edit.old_end == edit.start {
        // `>=` (not `>`): a range ending exactly at a pure-insert
        // point *extends over* the inserted text. Typing at the end
        // of a token is the dominant editing case, and inheriting the
        // preceding span's color keeps the new char stably colored
        // instead of blinking default-white until the next parse
        // settles. (The start counterpart keeps `>=` shifting right,
        // so a following span never overlaps the extension.)
        if pos >= edit.start {
            pos.saturating_add(edit.inserted_len)
        } else {
            pos
        }
    } else if pos <= edit.start {
        pos
    } else if pos >= edit.old_end {
        shift_position(pos, edit)
    } else {
        edit.start.saturating_add(edit.inserted_len)
    }
}

fn translate_byte_position(pos: u64, edit: TextProjectionEdit) -> u64 {
    if edit.old_end == edit.start {
        if pos >= edit.start {
            pos.saturating_add(edit.inserted_len)
        } else {
            pos
        }
    } else if pos <= edit.start {
        pos
    } else if pos >= edit.old_end {
        shift_position(pos, edit)
    } else {
        edit.start.saturating_add(edit.inserted_len)
    }
}

fn shift_position(pos: u64, edit: TextProjectionEdit) -> u64 {
    let old_len = edit.old_end.saturating_sub(edit.start);
    if edit.inserted_len >= old_len {
        pos.saturating_add(edit.inserted_len - old_len)
    } else {
        pos.saturating_sub(old_len - edit.inserted_len)
    }
}

/// Byte range `[start, end)` of the source line containing `cursor`:
/// `start` is just after the previous `\n` (or 0), `end` is just after
/// the next `\n` (or text length). Mirrors the producer's
/// `current_line_range` so the rendered `CurrentLine` wash covers the
/// same bytes the producer would. `cursor` is clamped to the text
/// length so a peer presence that briefly lags an edit is safe.
fn source_line_range(text: &str, cursor: u64) -> (u64, u64) {
    let c = (cursor as usize).min(text.len());
    let start = text[..c].rfind('\n').map_or(0, |i| i + 1);
    let end = text[c..].find('\n').map_or(text.len(), |i| c + i + 1);
    (start as u64, end as u64)
}

fn rects_to_vertex_bytes(
    rects: &[MinimapRect],
    surface_width: u32,
    surface_height: u32,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(rects.len() * 6 * QUAD_VERTEX_STRIDE as usize);
    for rect in rects {
        push_rect_vertices(&mut bytes, *rect, surface_width, surface_height);
    }
    bytes
}

fn push_rect_vertices(bytes: &mut Vec<u8>, rect: MinimapRect, width: u32, height: u32) {
    if rect.w <= 0.0 || rect.h <= 0.0 || width == 0 || height == 0 {
        return;
    }
    let x0 = px_to_ndc_x(rect.x, width);
    let x1 = px_to_ndc_x(rect.x + rect.w, width);
    let y0 = px_to_ndc_y(rect.y, height);
    let y1 = px_to_ndc_y(rect.y + rect.h, height);
    push_quad_vertex(bytes, x0, y0, rect.color);
    push_quad_vertex(bytes, x1, y0, rect.color);
    push_quad_vertex(bytes, x1, y1, rect.color);
    push_quad_vertex(bytes, x0, y0, rect.color);
    push_quad_vertex(bytes, x1, y1, rect.color);
    push_quad_vertex(bytes, x0, y1, rect.color);
}

fn push_quad_vertex(bytes: &mut Vec<u8>, x: f32, y: f32, color: [f32; 4]) {
    for value in [x, y, color[0], color[1], color[2], color[3]] {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
}

/// Vertex bytes for the diagnostic-squiggle pipeline (Q#W1). Same six
/// vertices per rect as a quad, but each carries a `uv`: `uv.x` is the
/// absolute screen-space pixel x of the corner (continuous phase
/// across separately emitted rects), `uv.y` is the signed pixel offset
/// from the band's vertical centerline (`±h/2`). The fragment shader
/// turns that into the sine.
fn squiggles_to_vertex_bytes(rects: &[MinimapRect], width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(rects.len() * 6 * SQUIGGLE_VERTEX_STRIDE as usize);
    for rect in rects {
        if rect.w <= 0.0 || rect.h <= 0.0 || width == 0 || height == 0 {
            continue;
        }
        let x0 = px_to_ndc_x(rect.x, width);
        let x1 = px_to_ndc_x(rect.x + rect.w, width);
        let y0 = px_to_ndc_y(rect.y, height);
        let y1 = px_to_ndc_y(rect.y + rect.h, height);
        let (left, right) = (rect.x, rect.x + rect.w);
        let half = rect.h * 0.5;
        // Top edge → centerline-relative uv.y = -half; bottom → +half.
        push_squiggle_vertex(&mut bytes, x0, y0, left, -half, rect.color);
        push_squiggle_vertex(&mut bytes, x1, y0, right, -half, rect.color);
        push_squiggle_vertex(&mut bytes, x1, y1, right, half, rect.color);
        push_squiggle_vertex(&mut bytes, x0, y0, left, -half, rect.color);
        push_squiggle_vertex(&mut bytes, x1, y1, right, half, rect.color);
        push_squiggle_vertex(&mut bytes, x0, y1, left, half, rect.color);
    }
    bytes
}

fn push_squiggle_vertex(
    bytes: &mut Vec<u8>,
    x: f32,
    y: f32,
    uv_x: f32,
    uv_y: f32,
    color: [f32; 4],
) {
    for value in [x, y, uv_x, uv_y, color[0], color[1], color[2], color[3]] {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
}

fn px_to_ndc_x(x: f32, width: u32) -> f32 {
    x / width as f32 * 2.0 - 1.0
}

fn px_to_ndc_y(y: f32, height: u32) -> f32 {
    1.0 - y / height as f32 * 2.0
}

/// Build the rich-text chunks fed to glyphon. Source chunks come from
/// `text` and retain source-byte styling; inline adornments create
/// extra chunks at their anchors and therefore do not shift any source
/// span/decoration range.
/// Clip + rebase the whole-file styling caches onto the byte range
/// `[start, end)` and build its projected chunks (Q#R1: the ONE chunk
/// source both the full `reshape` and the per-line surgery derive
/// from, so the two paths cannot disagree about a line's content).
/// Adornment anchors use an inclusive end — an anchor exactly at
/// `end` (a line's `\n`, or the slice end) injects after the last
/// content byte, matching the full-walk boundary behavior.
/// Assemble one shaped line from its chunks: concatenated projected
/// text + an attrs span per colored chunk (mirroring `set_rich_text`'s
/// only-when-non-default rule). Every line gets `LineEnding::Lf` —
/// the separator byte itself never enters a line's text.
fn line_from_chunks(chunks: &[RichChunk], family: &str) -> glyphon::cosmic_text::BufferLine {
    let default_attrs = Attrs::new().family(Family::Name(family));
    let mut attrs_list = glyphon::cosmic_text::AttrsList::new(&default_attrs);
    let mut text = String::new();
    for chunk in chunks {
        let start = text.len();
        text.push_str(&chunk.text);
        if let Some(c) = chunk.color {
            attrs_list.add_span(start..text.len(), &default_attrs.clone().color(c));
        }
    }
    glyphon::cosmic_text::BufferLine::new(
        text,
        glyphon::cosmic_text::LineEnding::Lf,
        attrs_list,
        Shaping::Advanced,
    )
}

fn clipped_chunks_for_range(
    text: &str,
    spans: &[StyleSpan],
    adornments: &[InlineAdornment],
    start: u64,
    end: u64,
    math: &[MathSubstitution],
) -> Vec<RichChunk> {
    let range_text = &text[start as usize..end as usize];
    let spans: Vec<StyleSpan> = spans
        .iter()
        .filter_map(|sp| {
            clip_rebase_range(sp.range.start, sp.range.end, start, end).map(|(s, e)| StyleSpan {
                range: ByteRange { start: s, end: e },
                style: sp.style,
            })
        })
        .collect();
    let adornments: Vec<InlineAdornment> = adornments
        .iter()
        .filter(|a| a.at >= start && a.at <= end)
        .map(|a| {
            let mut a = a.clone();
            a.at -= start;
            a
        })
        .collect();
    projected_rich_chunks(range_text, &spans, &adornments, math)
}

fn projected_rich_chunks(
    text: &str,
    spans: &[StyleSpan],
    adornments: &[InlineAdornment],
    math: &[MathSubstitution],
) -> Vec<RichChunk> {
    let text_len = text.len() as u64;
    // Every boundary used to slice `text` must be snapped to a UTF-8
    // char boundary. Span / decoration / adornment offsets come from
    // the daemon for a possibly-earlier generation than the rope this
    // frame holds (the one-frame edit race), so a raw offset can land
    // inside a multi-byte char and panic the slice. Flooring to the
    // previous char boundary is safe: it only shifts a chunk edge left
    // to the start of the codepoint it fell inside.
    let snap = |b: u64| floor_char_boundary(text, b.min(text_len) as usize) as u64;
    let mut boundaries: Vec<u64> = vec![0, text_len];
    for sp in spans {
        boundaries.push(snap(sp.range.start));
        boundaries.push(snap(sp.range.end));
    }
    let mut renderable_adornments: Vec<(usize, u64, &InlineAdornment)> = adornments
        .iter()
        .enumerate()
        .filter_map(|(idx, a)| renderable_adornment_anchor(a, text_len).map(|at| (idx, at, a)))
        .collect();
    for (_, at, _) in &renderable_adornments {
        boundaries.push(snap(*at));
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    renderable_adornments.sort_by_key(|(idx, at, _)| (*at, *idx));

    let mut chunks = Vec::new();
    let mut adorn_idx = 0usize;
    for w in boundaries.windows(2) {
        let (a, b) = (w[0], w[1]);
        push_adornments_at(&mut chunks, &renderable_adornments, &mut adorn_idx, a);
        if a < b {
            chunks.push(RichChunk {
                text: text[a as usize..b as usize].to_owned(),
                color: source_color_at(a, spans),
                source: ChunkSource::Source { start: a },
            });
        }
    }
    push_adornments_at(
        &mut chunks,
        &renderable_adornments,
        &mut adorn_idx,
        text_len,
    );
    if chunks.is_empty() {
        chunks.push(RichChunk {
            text: String::new(),
            color: None,
            source: ChunkSource::Source { start: 0 },
        });
    }
    // Q#MS4: substitute suppressed math spans BEFORE tab expansion — a
    // literal tab inside a span vanishes with the span's source bytes,
    // and tabs outside keep their `SourceTab` provenance.
    let chunks = substitute_math_spans(chunks, math);
    expand_chunk_tabs(chunks)
}

/// Replace each suppressed span's source bytes with ONE spacer chunk
/// (Q#MS4). Spans are disjoint and ordered (detection scans left to
/// right); a span split across style-boundary chunks emits its spacer
/// at the first overlap and swallows the rest. Adornment chunks pass
/// through — they consume no source bytes — so a hint anchored inside
/// a suppressed span surfaces beside the box rather than vanishing.
fn substitute_math_spans(chunks: Vec<RichChunk>, subs: &[MathSubstitution]) -> Vec<RichChunk> {
    if subs.is_empty() {
        return chunks;
    }
    let mut out = Vec::with_capacity(chunks.len() + subs.len());
    let mut emitted = vec![false; subs.len()];
    for chunk in chunks {
        let ChunkSource::Source { start } = chunk.source else {
            out.push(chunk);
            continue;
        };
        let end = start + chunk.text.len() as u64;
        let mut pos = start;
        while pos < end {
            // The first span not entirely before `pos`.
            let idx = subs.partition_point(|s| s.span.end as u64 <= pos);
            let cut = match subs.get(idx) {
                Some(s) if (s.span.start as u64) <= pos => {
                    // Inside a suppressed span: emit its spacer once.
                    if !emitted[idx] {
                        emitted[idx] = true;
                        out.push(RichChunk {
                            text: s.spacer.clone(),
                            color: None,
                            source: ChunkSource::MathBox {
                                start: s.span.start as u64,
                                end: s.span.end as u64,
                            },
                        });
                    }
                    pos = (s.span.end as u64).min(end);
                    continue;
                }
                Some(s) => (s.span.start as u64).min(end),
                None => end,
            };
            // Verbatim source up to the next span (or chunk end). Span
            // boundaries sit on ASCII `$`, so the slice is char-safe.
            out.push(RichChunk {
                text: chunk.text[(pos - start) as usize..(cut - start) as usize].to_owned(),
                color: chunk.color,
                source: ChunkSource::Source { start: pos },
            });
            pos = cut;
        }
    }
    out
}

/// Q#MS11's intersection rule for wash geometry: widen a projected
/// interval to cover the FULL projected extent of every math chunk
/// whose suppressed source range intersects the wash's source range.
/// Non-intersecting boxes are untouched — the round-3 F1 fix keeps a
/// wash that merely starts at a span's exclusive `end` off the box.
fn widen_over_math_chunks(
    chunks: &[RichChunk],
    (source_lo, source_hi): (u64, u64),
    (mut projected_lo, mut projected_hi): (u64, u64),
) -> (u64, u64) {
    let mut projected = 0u64;
    for chunk in chunks {
        let len = chunk.text.len() as u64;
        if let ChunkSource::MathBox { start, end } = chunk.source
            && source_lo < end
            && start < source_hi
        {
            projected_lo = projected_lo.min(projected);
            projected_hi = projected_hi.max(projected + len);
        }
        projected += len;
    }
    (projected_lo, projected_hi)
}

/// Pair each post-expansion `MathBox` chunk with its fitted box, in
/// projected byte space — the draw pass's addressing.
fn placed_math_boxes(chunks: &[RichChunk], subs: &[MathSubstitution]) -> Vec<PlacedMathBox> {
    let mut placed = Vec::new();
    let mut projected = 0u64;
    for chunk in chunks {
        if let ChunkSource::MathBox { start, end } = chunk.source
            && let Some(sub) = subs
                .iter()
                .find(|s| s.span.start as u64 == start && s.span.end as u64 == end)
        {
            placed.push(PlacedMathBox {
                projected_start: projected,
                projected_len: chunk.text.len() as u64,
                boxed: sub.boxed.clone(),
            });
        }
        projected += chunk.text.len() as u64;
    }
    placed
}

/// Expand display tabs after source styling and adornment insertion.
/// Chunks without tabs are moved through unchanged. A chunk containing
/// tabs is split only at those bytes; every emitted space keeps the
/// original color, while source tabs gain explicit provenance.
fn expand_chunk_tabs(chunks: Vec<RichChunk>) -> Vec<RichChunk> {
    let mut expanded = Vec::with_capacity(chunks.len());
    let mut column = 0usize;
    for chunk in chunks {
        if !chunk.text.contains('\t') {
            advance_display_column(&mut column, &chunk.text);
            expanded.push(chunk);
            continue;
        }

        let RichChunk {
            text,
            color,
            source,
        } = chunk;
        let mut segment_start = 0usize;
        for (byte, ch) in text.char_indices() {
            if ch != '\t' {
                continue;
            }
            if segment_start < byte {
                let segment = &text[segment_start..byte];
                advance_display_column(&mut column, segment);
                expanded.push(RichChunk {
                    text: segment.to_owned(),
                    color,
                    source: offset_chunk_source(source, segment_start as u64),
                });
            }
            let tab_stop = TAB_STOP_COLUMNS as usize;
            let tab_width = tab_stop - column % tab_stop;
            expanded.push(RichChunk {
                text: " ".repeat(tab_width),
                color,
                source: match source {
                    ChunkSource::Source { start } => ChunkSource::SourceTab {
                        start: start + byte as u64,
                    },
                    ChunkSource::Adornment { anchor } => ChunkSource::Adornment { anchor },
                    ChunkSource::SourceTab { start } => ChunkSource::SourceTab { start },
                    // A suppressed math span's spacer text is generated, not
                    // source, so it holds no tab byte to expand.
                    ChunkSource::MathBox { start, end } => ChunkSource::MathBox { start, end },
                },
            });
            column += tab_width;
            segment_start = byte + 1;
        }
        if segment_start < text.len() {
            let segment = &text[segment_start..];
            advance_display_column(&mut column, segment);
            expanded.push(RichChunk {
                text: segment.to_owned(),
                color,
                source: offset_chunk_source(source, segment_start as u64),
            });
        }
    }
    expanded
}

fn offset_chunk_source(source: ChunkSource, byte_offset: u64) -> ChunkSource {
    match source {
        ChunkSource::Source { start } => ChunkSource::Source {
            start: start + byte_offset,
        },
        ChunkSource::SourceTab { start } => ChunkSource::SourceTab { start },
        ChunkSource::Adornment { anchor } => ChunkSource::Adornment { anchor },
        // The suppressed range is already in slice coordinates and is never
        // split, so a within-chunk offset does not move it.
        ChunkSource::MathBox { start, end } => ChunkSource::MathBox { start, end },
    }
}

fn advance_display_column(column: &mut usize, text: &str) {
    for ch in text.chars() {
        if ch == '\n' {
            *column = 0;
        } else {
            *column += UnicodeWidthChar::width(ch).unwrap_or(0);
        }
    }
}

fn renderable_adornment_anchor(adornment: &InlineAdornment, text_len: u64) -> Option<u64> {
    match (&adornment.placement, &adornment.content) {
        (AdornmentPlacement::AtOffset, AdornmentContent::Text { .. }) => {
            Some(adornment.at.min(text_len))
        }
        // Session 6 consumes the inlay-hint producer surface only.
        // Other placements and resource handles need layout/resource
        // policy, so silently ignore them until their sessions land.
        _ => None,
    }
}

fn push_adornments_at(
    chunks: &mut Vec<RichChunk>,
    adornments: &[(usize, u64, &InlineAdornment)],
    next: &mut usize,
    at: u64,
) {
    while let Some((_, anchor, adornment)) = adornments.get(*next).copied() {
        if anchor != at {
            break;
        }
        if let AdornmentContent::Text { text, style } = &adornment.content {
            chunks.push(RichChunk {
                text: text.clone(),
                color: Some(adornment_text_color(style.fg)),
                source: ChunkSource::Adornment { anchor },
            });
        }
        *next += 1;
    }
}

fn adornment_text_color(fg: CellColor) -> glyphon::Color {
    cell_color_to_glyphon(fg).unwrap_or_else(|| glyphon::Color::rgb(130, 130, 140))
}

/// The `StyleSpans { full: true }` transform `replace_style_spans` applies:
/// flatten every segment's spans and start-sort. Extracted as a free
/// function so it (and `source_color_at`) can be exercised without a live
/// `State` — the start-sort is the step that makes producer depth-order
/// irrelevant on the wire, which is why the daemon flattens to disjoint
/// spans and this consumer folds (framing Q#IJ6).
fn spans_from_segments(segments: Vec<StyleSegment>) -> Vec<StyleSpan> {
    let mut spans: Vec<StyleSpan> = Vec::new();
    for seg in segments {
        spans.extend(seg.spans);
    }
    spans.sort_by_key(|s| s.range.start);
    spans
}

fn source_color_at(byte: u64, spans: &[StyleSpan]) -> Option<glyphon::Color> {
    // Fold every covering span in order, matching the semantic-client
    // `effective_style_at` contract (last covering span with a non-default
    // fg wins) rather than returning the first. The daemon flattens
    // injection layers into disjoint spans (framing Q#IJ6), so usually at
    // most one covers a byte — but where spans do overlap (a styled
    // markdown parent under an injected child), the topmost color must win,
    // never the outermost. Spans arrive start-sorted, so a narrower nested
    // span (later start) folds after and overrides its enclosing span.
    let mut color = None;
    for sp in spans
        .iter()
        .filter(|sp| sp.range.start <= byte && byte < sp.range.end)
    {
        if let Some(c) = cell_color_to_glyphon(sp.style.fg) {
            color = Some(c);
        }
    }
    color
}

/// Convert a `pmacs-protocol::cell::Color` to a `glyphon::Color`.
/// Returns `None` for `Default` so the renderer falls back to the
/// `Attrs` default color (white-ish in our render) rather than
/// stomping with an arbitrary RGB.
///
/// `Indexed` uses the standard ANSI 16-color + 256-color cube
/// palette. The TUI interprets these via terminal-level color codes;
/// the GPU has no equivalent layer, so the palette mapping lives
/// here. Picked to roughly match `xterm-256color` defaults so
/// existing pmacs themes look consistent across both frontends.
fn cell_color_to_glyphon(c: CellColor) -> Option<glyphon::Color> {
    match c {
        CellColor::Default => None,
        CellColor::Rgb(r, g, b) => Some(glyphon::Color::rgb(r, g, b)),
        CellColor::Indexed(idx) => Some(indexed_to_glyphon(idx)),
    }
}

/// The GPU's "plain" text color — the buffer-text default at
/// `main.rs`'s primary `TextArea`. The Q#TH5 mapping target for a
/// set face's `Default` fg.
fn plain_text_color() -> glyphon::Color {
    glyphon::Color::rgb(230, 230, 235)
}

/// The window clear color as a quad rgba — the Q#TH5 mapping target
/// for a set face's `Default` bg (an untinted surface). Must equal
/// [`BG`].
const WINDOW_BG_RGBA: [f32; 4] = [0.05, 0.05, 0.07, 1.0];

/// Resolved terminal RGB → quad color, carrying `alpha`.
/// A resolved wire cell color as a glyphon text color.
fn rgb_to_glyphon(rgb: crate::terminal::Rgb) -> Color {
    Color::rgb(rgb[0], rgb[1], rgb[2])
}

fn rgb_to_quad(rgb: crate::terminal::Rgb, alpha: f32) -> [f32; 4] {
    [
        f32::from(rgb[0]) / 255.0,
        f32::from(rgb[1]) / 255.0,
        f32::from(rgb[2]) / 255.0,
        alpha,
    ]
}

/// glyphon (u8) → quad (f32) color, carrying `alpha`. Divides by 255
/// — the same space every existing float constant uses (e.g. the
/// error squiggle `[0.945, …]` is exactly `rgb(241, 76, 76) / 255`).
fn glyphon_to_rgba(c: glyphon::Color, alpha: f32) -> [f32; 4] {
    [
        f32::from(c.r()) / 255.0,
        f32::from(c.g()) / 255.0,
        f32::from(c.b()) / 255.0,
        alpha,
    ]
}

/// quad (f32) → glyphon (u8) color (alpha dropped) — the `reverse`
/// swap's other direction.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "inputs are 0.0..=1.0 quad colors by construction"
)]
fn rgba_to_glyphon(c: [f32; 4]) -> glyphon::Color {
    glyphon::Color::rgb(
        (c[0] * 255.0) as u8,
        (c[1] * 255.0) as u8,
        (c[2] * 255.0) as u8,
    )
}

/// Standard xterm-style 256-color palette: 16 base colors + 6×6×6
/// RGB cube (16..=231) + 24-step grayscale (232..=255). Values
/// pulled from the conventional xterm defaults; the 6×6×6 cube uses
/// the standard step values {0, 95, 135, 175, 215, 255}.
fn indexed_to_glyphon(idx: u8) -> glyphon::Color {
    const ANSI16: [(u8, u8, u8); 16] = [
        (0, 0, 0),       // 0 black
        (205, 49, 49),   // 1 red
        (13, 188, 121),  // 2 green
        (229, 229, 16),  // 3 yellow
        (36, 114, 200),  // 4 blue
        (188, 63, 188),  // 5 magenta
        (17, 168, 205),  // 6 cyan
        (229, 229, 229), // 7 white
        (102, 102, 102), // 8 bright black
        (241, 76, 76),   // 9 bright red
        (35, 209, 139),  // 10 bright green
        (245, 245, 67),  // 11 bright yellow
        (59, 142, 234),  // 12 bright blue
        (214, 112, 214), // 13 bright magenta
        (41, 184, 219),  // 14 bright cyan
        (255, 255, 255), // 15 bright white
    ];
    if idx < 16 {
        let (r, g, b) = ANSI16[idx as usize];
        return glyphon::Color::rgb(r, g, b);
    }
    if (16..=231).contains(&idx) {
        // 6×6×6 cube.
        const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        let i = idx - 16;
        let r = STEPS[(i / 36) as usize];
        let g = STEPS[((i / 6) % 6) as usize];
        let b = STEPS[(i % 6) as usize];
        return glyphon::Color::rgb(r, g, b);
    }
    // 232..=255: 24-step grayscale, evenly spaced 8..=238.
    let level = 8 + 10 * (idx - 232);
    glyphon::Color::rgb(level, level, level)
}

/// Height of the diagnostic squiggle band, in pixels (Q#W1). Taller
/// than the original straight bar (2px) so the sine wave fits: the
/// fragment shader's `amplitude + thickness` (~2.4px each side) must
/// sit inside half the band.
const DIAG_SQUIGGLE_PX: f32 = 6.0;

/// Map a [`DecorationKind`] to an underline color, or `None` for
/// kinds that don't underline.
///
/// Session 5 originally rendered diagnostics by *recoloring the text
/// foreground*, which clobbered the syntax color of the very token
/// the diagnostic points at — the same flaw the TUI fixed with
/// protocol v6's `underline_color` (T M4.6). The GPU's equivalent is
/// a wavy squiggle hugging the bottom of the glyph extent (Q#W1, was
/// a straight bar through PR #65); the text keeps its syntax color.
/// Same RGB palette the fg path used (red / yellow / light blue /
/// dim gray), so the window's severity language is unchanged.
fn decoration_kind_to_underline_color(kind: DecorationKind) -> Option<[f32; 4]> {
    match kind {
        // ANSI bright red — matches TUI diagnostic-error palette.
        DecorationKind::DiagnosticError => Some([0.945, 0.298, 0.298, 1.0]),
        // ANSI bright yellow.
        DecorationKind::DiagnosticWarning => Some([0.961, 0.961, 0.263, 1.0]),
        // ANSI bright blue.
        DecorationKind::DiagnosticInfo => Some([0.231, 0.557, 0.918, 1.0]),
        // ANSI bright black (dim gray — hints should be visible but
        // visually quietest of the diagnostic four).
        DecorationKind::DiagnosticHint => Some([0.4, 0.4, 0.4, 1.0]),
        // Background kinds wash the full line box instead.
        DecorationKind::Selection
        | DecorationKind::SearchMatch
        | DecorationKind::SearchMatchActive
        | DecorationKind::CurrentLine => None,
    }
}

/// Severity rank of a diagnostic decoration kind (UX gutter sub-arc 2):
/// `0` = most severe (`Error`) … `3` = least (`Hint`); `None` for
/// non-diagnostic kinds. Lets the gutter sign pick the most-severe
/// diagnostic touching a line (min rank wins), mirroring the TUI.
fn diagnostic_severity_rank(kind: DecorationKind) -> Option<u8> {
    match kind {
        DecorationKind::DiagnosticError => Some(0),
        DecorationKind::DiagnosticWarning => Some(1),
        DecorationKind::DiagnosticInfo => Some(2),
        DecorationKind::DiagnosticHint => Some(3),
        DecorationKind::Selection
        | DecorationKind::SearchMatch
        | DecorationKind::SearchMatchActive
        | DecorationKind::CurrentLine => None,
    }
}

/// Background-bearing companion to
/// [`decoration_kind_to_underline_color`]: maps each
/// background-needing `DecorationKind` to its quad-pipeline color as
/// an RGBA tuple in 0..=1 space. Returns `None` for underline-only
/// kinds (the four diagnostic severities) so the two helpers form a
/// total cover with no overlap.
///
/// Session 9.1 shipped `Selection`; session 9.2 adds `CurrentLine`.
/// `SearchMatch` / `SearchMatchActive` wait on a search feature in
/// pmacs core (Q#4 in `docs/pmacs-gpu-quad-backgrounds-framing.md`),
/// so they continue to return `None` here.
#[allow(clippy::match_same_arms)] // each `None` arm has a distinct rationale comment.
fn decoration_kind_to_bg_color(kind: DecorationKind) -> Option<[f32; 4]> {
    match kind {
        // Translucent blue, similar to the conventional editor
        // selection background. The 0.30 alpha lets the underlying
        // glyph color show through unmodified — text remains readable
        // because the text render pass runs after this one in the same
        // render pass (Q#2 stance α).
        DecorationKind::Selection => Some([0.31, 0.42, 0.82, 0.30]),
        // Blue-grey wash, quietest of the background kinds (it's always
        // on) but still visible. The first 9.2/9.3 value (alpha 0.08)
        // computed to ~10/255 above the dark clear color and was
        // swamped by glyphs on a text line — invisible in practice.
        // 0.22 keeps it subtle vs Selection's 0.30 while actually
        // reading as a current-line band.
        DecorationKind::CurrentLine => Some([0.55, 0.60, 0.75, 0.22]),
        // In-buffer search (Q#SR4): a translucent yellow wash under
        // every match, a stronger amber under the active one so it
        // stands out as you step through. Both let the glyph color
        // show through (text renders after this pass).
        DecorationKind::SearchMatch => Some([0.85, 0.78, 0.20, 0.30]),
        DecorationKind::SearchMatchActive => Some([0.95, 0.55, 0.12, 0.48]),
        // Underline-only — handled by
        // [`decoration_kind_to_underline_color`].
        DecorationKind::DiagnosticError
        | DecorationKind::DiagnosticWarning
        | DecorationKind::DiagnosticInfo
        | DecorationKind::DiagnosticHint => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pmacs_protocol::cell::Style;

    fn style_with_fg(fg: CellColor) -> Style {
        Style {
            fg,
            ..Style::default()
        }
    }

    fn color_close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.into_iter()
            .zip(b)
            .all(|(left, right)| (left - right).abs() < 0.001)
    }

    fn f32_at(bytes: &[u8], index: usize) -> f32 {
        let start = index * std::mem::size_of::<f32>();
        f32::from_ne_bytes(
            bytes[start..start + std::mem::size_of::<f32>()]
                .try_into()
                .expect("f32 bytes"),
        )
    }

    fn span(start: u64, end: u64, fg: CellColor) -> StyleSpan {
        StyleSpan {
            range: ByteRange { start, end },
            style: style_with_fg(fg),
        }
    }

    fn adornment(at: u64, placement: AdornmentPlacement, text: &str) -> InlineAdornment {
        InlineAdornment {
            at,
            placement,
            content: AdornmentContent::Text {
                text: text.to_owned(),
                style: Style::default(),
            },
        }
    }

    fn resource_adornment(at: u64, placement: AdornmentPlacement) -> InlineAdornment {
        InlineAdornment {
            at,
            placement,
            content: AdornmentContent::Resource { handle: 7 },
        }
    }

    fn chunk_texts(chunks: &[RichChunk]) -> Vec<&str> {
        chunks.iter().map(|chunk| chunk.text.as_str()).collect()
    }

    #[test]
    fn source_line_range_locates_enclosing_line() {
        // "abc\nde\nfgh": newlines at byte 3 and 6; len = 10.
        let text = "abc\nde\nfgh";
        // Cursor on line 0 → [0, 4) (includes the trailing \n).
        assert_eq!(source_line_range(text, 0), (0, 4));
        assert_eq!(source_line_range(text, 2), (0, 4));
        // Start of line 1 → [4, 7).
        assert_eq!(source_line_range(text, 4), (4, 7));
        assert_eq!(source_line_range(text, 5), (4, 7));
        // Last line has no trailing \n → [7, 10).
        assert_eq!(source_line_range(text, 8), (7, 10));
        // Cursor past end clamps to the last line, never indexes out.
        assert_eq!(source_line_range(text, 99), (7, 10));
    }

    #[test]
    fn translate_key_maps_motion_named_keys_and_chars() {
        use winit::keyboard::{Key as WKey, ModifiersState, NamedKey, SmolStr};

        let none = ModifiersState::empty();
        // Motion named keys translate and are gated as motion.
        for (named, expected) in [
            (NamedKey::ArrowLeft, ProtocolKey::Left),
            (NamedKey::ArrowRight, ProtocolKey::Right),
            (NamedKey::ArrowUp, ProtocolKey::Up),
            (NamedKey::ArrowDown, ProtocolKey::Down),
            (NamedKey::Home, ProtocolKey::Home),
            (NamedKey::End, ProtocolKey::End),
            (NamedKey::PageUp, ProtocolKey::PageUp),
            (NamedKey::PageDown, ProtocolKey::PageDown),
        ] {
            let (k, m) = translate_key(&WKey::Named(named), none).expect("named maps");
            assert_eq!(k, expected);
            assert!(m.is_empty());
            assert!(is_motion_key(k), "{expected:?} should gate as motion");
        }

        // A character key maps to Char but is NOT a motion key (B1
        // gates it out; B2 opens it).
        let (k, _) = translate_key(&WKey::Character(SmolStr::new("a")), none).expect("char maps");
        assert_eq!(k, ProtocolKey::Char('a'));
        assert!(!is_motion_key(k));

        // Editing named keys translate (for B2) but don't gate as motion.
        let (bk, _) = translate_key(&WKey::Named(NamedKey::Backspace), none).expect("bksp maps");
        assert_eq!(bk, ProtocolKey::Backspace);
        assert!(!is_motion_key(bk));

        let (space, _) = translate_key(&WKey::Named(NamedKey::Space), none).expect("space maps");
        assert_eq!(space, ProtocolKey::Char(' '));
        assert!(!is_motion_key(space));
    }

    #[test]
    fn should_forward_key_gates_editing_keys_and_excludes_chords() {
        let none = Modifiers::NONE;
        let ctrl = Modifiers::CTRL;
        let shift = Modifiers::SHIFT;

        // Plain text-editing keys forward.
        for key in [
            ProtocolKey::Char('a'),
            ProtocolKey::Char('A'),
            ProtocolKey::Backspace,
            ProtocolKey::Enter,
            ProtocolKey::Delete,
            ProtocolKey::Tab,
        ] {
            assert!(should_forward_key(key, none), "{key:?} should forward");
        }
        // Shift is not a chord modifier (Shift+a already arrives as 'A').
        assert!(should_forward_key(ProtocolKey::Char('A'), shift));

        // Ctrl/Alt/Meta + a non-motion key is a chord — withheld in B2.
        assert!(!should_forward_key(ProtocolKey::Char('x'), ctrl));
        assert!(!should_forward_key(ProtocolKey::Char('f'), Modifiers::ALT));
        assert!(!should_forward_key(
            ProtocolKey::Char('h'),
            Modifiers::HYPER
        ));

        // Motion keys forward regardless of modifiers (C-Left = word-left).
        assert!(should_forward_key(ProtocolKey::Left, ctrl));
        assert!(should_forward_key(ProtocolKey::Down, shift));
        assert!(should_forward_key(ProtocolKey::PageUp, none));

        // Deletion keys forward regardless of modifiers too — C-BS /
        // C-DEL / M-BS are word-level deletes in the default keymap,
        // the same editing-command family as chorded motion.
        assert!(should_forward_key(ProtocolKey::Backspace, ctrl));
        assert!(should_forward_key(ProtocolKey::Delete, ctrl));
        assert!(should_forward_key(ProtocolKey::Backspace, Modifiers::ALT));
    }

    #[test]
    fn command_chord_forwards_char_chords_with_ctrl_or_alt() {
        // Q#GC1 — any Char/Enter/Tab with Ctrl or Alt is a command chord,
        // forwarded to the daemon keymap. This subsumes the old allowlists
        // (isearch C-s/C-r/C-M-s, clipboard M-w/C-w/C-y, M-x) plus the
        // rest of the keymap (C-a, C-e, M-f, C-x, …).
        let ctrl_alt = Modifiers::CTRL | Modifiers::ALT;
        for (key, mods) in [
            (ProtocolKey::Char('s'), Modifiers::CTRL), // isearch
            (ProtocolKey::Char('s'), ctrl_alt),        // regex isearch
            (ProtocolKey::Char('w'), Modifiers::ALT),  // copy
            (ProtocolKey::Char('y'), Modifiers::CTRL), // yank
            (ProtocolKey::Char('x'), Modifiers::ALT),  // M-x
            (ProtocolKey::Char('x'), Modifiers::CTRL), // C-x prefix
            (ProtocolKey::Char('a'), Modifiers::CTRL), // line-start (was withheld)
            (ProtocolKey::Char('f'), Modifiers::ALT),  // forward-word
            (ProtocolKey::Enter, Modifiers::CTRL),
        ] {
            assert!(
                is_command_chord(key, mods),
                "{key:?}+{mods:?} is a command chord"
            );
            // The general forwarding is *why* these move; `should_forward_key`
            // still withholds them (they're caught before it).
            assert!(!should_forward_key(key, mods));
        }
        // Plain text, and Meta/Super-only chords, are not command chords.
        assert!(!is_command_chord(ProtocolKey::Char('a'), Modifiers::NONE));
        assert!(!is_command_chord(ProtocolKey::Char('c'), Modifiers::META));
        // Motion isn't routed here (it keeps its own defer-aware path).
        assert!(!is_command_chord(ProtocolKey::Left, Modifiers::CTRL));
    }

    #[test]
    fn is_layout_text_distinguishes_altgr_from_command_chords() {
        // Audit F-004 — AltGr (Ctrl+Alt on Windows) produces printable
        // text; that's text input, so the caller strips the modifiers.
        let ctrl_alt = Modifiers::CTRL | Modifiers::ALT;
        assert!(is_layout_text(Some("@"), ctrl_alt));
        assert!(is_layout_text(Some("{"), ctrl_alt));
        assert!(is_layout_text(Some("€"), ctrl_alt)); // AltGr+e on many layouts
        // Alt ALONE is not AltGr. On macOS the Option key is Alt and emits
        // printable text (Option+x → "≈"), but Option-as-Meta is how the
        // GUI reaches M-x / M-f — leave it intact so it forwards as a
        // command chord instead of self-inserting the symbol.
        assert!(!is_layout_text(Some("≈"), Modifiers::ALT)); // macOS Option+x → M-x
        assert!(!is_layout_text(Some("€"), Modifiers::ALT));
        // Genuine command chords produce no text (or a control char) —
        // not layout text, so they still route to the keymap.
        assert!(!is_layout_text(None, Modifiers::CTRL)); // C-a etc.
        assert!(!is_layout_text(Some("\u{1}"), Modifiers::CTRL)); // Ctrl+A control char
        assert!(!is_layout_text(Some(""), ctrl_alt)); // no text
        // Plain text has no command modifier, so it's already handled and
        // needs no stripping.
        assert!(!is_layout_text(Some("a"), Modifiers::NONE));
        assert!(!is_layout_text(Some("A"), Modifiers::SHIFT));
    }

    #[test]
    fn mb_dropdown_window_clamps_and_keeps_selection_visible() {
        // Row height is 20.0; a 1000px space fits any producer-capped list.
        assert_eq!(
            mb_dropdown_window(5, 2, 1000.0, FontMetrics::default()),
            Some((0, 5))
        );
        // The whole-fits path is identical to no clamp: (0, n).
        assert_eq!(
            mb_dropdown_window(10, 9, 1000.0, FontMetrics::default()),
            Some((0, 10))
        );

        // A 100px space fits 5 rows. Selection near the top anchors at 0.
        assert_eq!(
            mb_dropdown_window(10, 0, 100.0, FontMetrics::default()),
            Some((0, 5))
        );
        // Selection past the fold scrolls so it stays visible (bottom edge).
        assert_eq!(
            mb_dropdown_window(10, 9, 100.0, FontMetrics::default()),
            Some((5, 5))
        );
        let (first, count) = mb_dropdown_window(10, 7, 100.0, FontMetrics::default()).unwrap();
        assert!(
            first <= 7 && 7 < first + count,
            "sel 7 in [{first},{})",
            first + count
        );

        // Degenerate: too short for even one row ⇒ hide, never draw above 0.
        assert_eq!(
            mb_dropdown_window(10, 0, 10.0, FontMetrics::default()),
            None
        );
        // No candidates ⇒ nothing.
        assert_eq!(
            mb_dropdown_window(0, 0, 500.0, FontMetrics::default()),
            None
        );
        // An out-of-range selection is clamped, not panicked.
        assert_eq!(
            mb_dropdown_window(3, 99, 1000.0, FontMetrics::default()),
            Some((0, 3))
        );
    }

    #[test]
    fn translate_key_carries_modifiers() {
        use winit::keyboard::{Key as WKey, ModifiersState, NamedKey};

        let ctrl = ModifiersState::CONTROL;
        let (k, m) = translate_key(&WKey::Named(NamedKey::ArrowLeft), ctrl).expect("maps");
        assert_eq!(k, ProtocolKey::Left);
        assert!(m.contains(Modifiers::CTRL));
        assert!(!m.contains(Modifiers::SHIFT));
    }

    #[test]
    fn line_byte_offsets_indexes_each_logical_line() {
        // "abc\nde\nfgh": lines start at bytes 0, 4, 7. Indexed by
        // LayoutRun::line_i to rebase line-relative glyph offsets.
        assert_eq!(line_byte_offsets("abc\nde\nfgh"), vec![0, 4, 7]);
        // Trailing newline yields a final empty line at byte len.
        assert_eq!(line_byte_offsets("a\nb\n"), vec![0, 2, 4]);
        // No newline: one line at 0.
        assert_eq!(line_byte_offsets("abc"), vec![0]);
        assert_eq!(line_byte_offsets(""), vec![0]);
    }

    #[test]
    fn line_char_offsets_track_unicode_line_starts() {
        let text = "aé\n😀b\n";
        let (line_starts, line_char_starts) = line_offset_tables(text);
        assert_eq!(line_starts, vec![0, 4, 10]);
        assert_eq!(line_char_starts, vec![0, 3, 6]);
    }

    #[test]
    fn byte_offset_for_char_offset_scans_only_within_line() {
        let text = "aé\n😀b";
        let (line_starts, line_char_starts) = line_offset_tables(text);
        assert_eq!(
            byte_offset_for_char_offset(text, &line_starts, &line_char_starts, 0),
            Some(0)
        );
        assert_eq!(
            byte_offset_for_char_offset(text, &line_starts, &line_char_starts, 2),
            Some(3)
        );
        assert_eq!(
            byte_offset_for_char_offset(text, &line_starts, &line_char_starts, 3),
            Some(4)
        );
        assert_eq!(
            byte_offset_for_char_offset(text, &line_starts, &line_char_starts, 4),
            Some(8)
        );
    }

    #[test]
    fn loro_text_delta_batch_inserts_multibyte_text_and_updates_lines() {
        let mut text = "aé\nb".to_owned();
        let (mut line_starts, mut line_char_starts) = line_offset_tables(&text);
        let delta = vec![
            loro::TextDelta::Retain {
                retain: 3,
                attributes: None,
            },
            loro::TextDelta::Insert {
                insert: "😀\n".to_owned(),
                attributes: None,
            },
        ];

        let mut edits = Vec::new();
        apply_loro_text_delta_batch(
            &mut text,
            &mut line_starts,
            &mut line_char_starts,
            &delta,
            &mut edits,
        )
        .expect("delta applies");

        assert_eq!(text, "aé\n😀\nb");
        assert_eq!((line_starts, line_char_starts), line_offset_tables(&text));
        assert_eq!(
            edits,
            vec![TextProjectionEdit {
                start: 4,
                old_end: 4,
                inserted_len: "😀\n".len() as u64,
            }]
        );
    }

    #[test]
    fn loro_text_delta_batch_deletes_across_unicode_lines() {
        let mut text = "aé\n😀\nb".to_owned();
        let (mut line_starts, mut line_char_starts) = line_offset_tables(&text);
        let delta = vec![
            loro::TextDelta::Retain {
                retain: 1,
                attributes: None,
            },
            loro::TextDelta::Delete { delete: 3 },
        ];

        let mut edits = Vec::new();
        apply_loro_text_delta_batch(
            &mut text,
            &mut line_starts,
            &mut line_char_starts,
            &delta,
            &mut edits,
        )
        .expect("delta applies");

        assert_eq!(text, "a\nb");
        assert_eq!((line_starts, line_char_starts), line_offset_tables(&text));
        assert_eq!(
            edits,
            vec![TextProjectionEdit {
                start: 1,
                old_end: 8,
                inserted_len: 0,
            }]
        );
    }

    #[test]
    fn cached_style_ranges_translate_through_insertions() {
        let edit = TextProjectionEdit {
            start: 5,
            old_end: 5,
            inserted_len: 3,
        };

        assert_eq!(
            translate_byte_range(ByteRange { start: 10, end: 14 }, edit),
            Some(ByteRange { start: 13, end: 17 }),
            "ranges after the insert shift right"
        );
        assert_eq!(
            translate_byte_range(ByteRange { start: 2, end: 10 }, edit),
            Some(ByteRange { start: 2, end: 13 }),
            "ranges containing the insert expand"
        );
        assert_eq!(
            translate_byte_range(ByteRange { start: 2, end: 5 }, edit),
            Some(ByteRange { start: 2, end: 8 }),
            "ranges ending exactly at the insert boundary extend over the typed \
             text — typed chars inherit the preceding token's color until the \
             next authoritative frame"
        );
    }

    #[test]
    fn optimistic_insert_text_covers_plain_chars_and_tab_but_not_enter() {
        let mut buf = [0u8; 4];
        let none = Modifiers::NONE;
        let shift = Modifiers::SHIFT;
        let ctrl = Modifiers::CTRL;

        assert_eq!(
            optimistic_insert_text(ProtocolKey::Char('a'), none, &mut buf),
            Some("a")
        );
        assert_eq!(
            optimistic_insert_text(ProtocolKey::Char('É'), shift, &mut buf),
            Some("É"),
            "shifted printable chars stay optimistic (shift is how uppercase arrives)"
        );
        assert_eq!(
            optimistic_insert_text(ProtocolKey::Enter, none, &mut buf),
            None,
            "RET binds edit.newline-and-indent (Q#AI1): the inserted text depends \
             on the current line, so plain Enter must round-trip — this is also \
             what makes RET rebindings reachable from the GPU frontend"
        );
        assert_eq!(
            optimistic_insert_text(ProtocolKey::Tab, none, &mut buf),
            Some("\t"),
            "TAB is bound to buffer.tab = insert_char(9): identical to a self-insert"
        );

        // Modified Enter/Tab and chords round-trip — a keymap may bind
        // S-RET / C-TAB to anything.
        assert_eq!(
            optimistic_insert_text(ProtocolKey::Enter, shift, &mut buf),
            None
        );
        assert_eq!(
            optimistic_insert_text(ProtocolKey::Tab, ctrl, &mut buf),
            None
        );
        assert_eq!(
            optimistic_insert_text(ProtocolKey::Char('x'), ctrl, &mut buf),
            None
        );
        // Deletions and motion still round-trip.
        assert_eq!(
            optimistic_insert_text(ProtocolKey::Backspace, none, &mut buf),
            None
        );
        assert_eq!(
            optimistic_insert_text(ProtocolKey::Left, none, &mut buf),
            None
        );
    }

    #[test]
    fn optimistic_insert_text_round_trips_builtin_pair_chars() {
        // Auto-pairing Q#AP1: the nine built-in pair chars must reach
        // daemon dispatch so the opener and the pairing hook's closer
        // are adjacent daemon-peer undo units. Both modifier shapes
        // real keyboards produce are pinned: `[`/`]`/`'`/`` ` ``
        // arrive unshifted, `(`/`)`/`{`/`}`/`"` arrive with SHIFT — a
        // gate that only caught `Modifiers::NONE` would leak every
        // shifted pair char back onto the optimistic path.
        let mut buf = [0u8; 4];
        for c in pmacs_protocol::BUILTIN_PAIR_CHARS {
            assert_eq!(
                optimistic_insert_text(ProtocolKey::Char(c), Modifiers::NONE, &mut buf),
                None,
                "unshifted {c:?} must round-trip"
            );
            assert_eq!(
                optimistic_insert_text(ProtocolKey::Char(c), Modifiers::SHIFT, &mut buf),
                None,
                "shifted {c:?} must round-trip"
            );
        }
    }

    /// Q#R1 parity invariant: the per-line surgery's chunk source
    /// (`clipped_chunks_for_range` over one line's content range)
    /// must agree byte-for-byte — text AND color — with the full
    /// slice walk split at line boundaries. Pinned here so the
    /// surgically rebuilt `BufferLine` can't drift from what a full
    /// `set_rich_text` would have produced.
    #[test]
    fn per_line_chunks_match_the_full_walk() {
        // (byte, color) stream, with `\n` bytes dropped — the full
        // walk keeps them inside source chunks; per-line walks
        // exclude them (cosmic strips the separator per line).
        fn flat(chunks: &[RichChunk]) -> Vec<(u8, Option<u32>)> {
            chunks
                .iter()
                .flat_map(|c| {
                    let color = c.color.map(|col| col.0);
                    c.text
                        .bytes()
                        .filter(|&b| b != b'\n')
                        .map(move |b| (b, color))
                        .collect::<Vec<_>>()
                })
                .collect()
        }
        // Two content lines + trailing newline. A span crossing the
        // line break, plus two inlay hints: one mid-line-1, one
        // anchored EXACTLY at line 0's newline (the predicted-finding
        // #1 boundary case — it must belong to line 0, before the \n).
        let text = "alpha BETA\ngamma delta\n";
        let spans = vec![StyleSpan {
            range: ByteRange { start: 6, end: 16 },
            style: CellStyle {
                fg: CellColor::Indexed(2),
                ..CellStyle::default()
            },
        }];
        let hint = |at: u64, label: &str| InlineAdornment {
            at,
            placement: AdornmentPlacement::AtOffset,
            content: AdornmentContent::Text {
                text: label.to_owned(),
                style: CellStyle::default(),
            },
        };
        let adornments = vec![hint(10, "<eol>"), hint(17, ": T ")];

        let full = flat(&clipped_chunks_for_range(
            text,
            &spans,
            &adornments,
            0,
            text.len() as u64,
            &[],
        ));

        // Line ranges as the surgery computes them: content excludes
        // the newline; the phantom line after the trailing `\n` is
        // empty.
        let mut per_line = Vec::new();
        for (start, content_end) in [(0u64, 10u64), (11, 22), (23, 23)] {
            per_line.extend(flat(&clipped_chunks_for_range(
                text,
                &spans,
                &adornments,
                start,
                content_end,
                &[],
            )));
        }

        assert_eq!(
            per_line, full,
            "per-line chunk walks must reproduce the full walk exactly \
             (text and colors, newlines excluded)"
        );

        // The boundary hint landed on line 0 (before its newline), not
        // line 1.
        let line0 = clipped_chunks_for_range(text, &spans, &adornments, 0, 10, &[]);
        assert!(
            line0.iter().any(|c| c.text == "<eol>"),
            "newline-anchored hint belongs to the line it terminates"
        );
        let line1 = clipped_chunks_for_range(text, &spans, &adornments, 11, 22, &[]);
        assert!(
            line1.iter().all(|c| c.text != "<eol>"),
            "newline-anchored hint must not duplicate onto the next line"
        );
        assert!(
            line1.iter().any(|c| c.text == ": T "),
            "mid-line hint renders on its own line"
        );
    }

    #[test]
    fn hit_runs_map_projected_bytes_back_to_source() {
        // Source slice "ab\ncd" with an inlay hint ": i32 " anchored
        // at byte 2 (end of "ab"): projected text = "ab: i32 \ncd".
        let chunks = vec![
            RichChunk {
                text: "ab".into(),
                color: None,
                source: ChunkSource::Source { start: 0 },
            },
            RichChunk {
                text: ": i32 ".into(),
                color: None,
                source: ChunkSource::Adornment { anchor: 2 },
            },
            RichChunk {
                text: "\ncd".into(),
                color: None,
                source: ChunkSource::Source { start: 2 },
            },
        ];
        let (runs, line_starts) = build_hit_runs(&chunks);

        assert_eq!(
            line_starts,
            vec![0, 9],
            "projected line table counts the newline at projected byte 8"
        );

        // Hits inside source runs map linearly.
        assert_eq!(projected_to_source(&runs, 0), Some(0));
        assert_eq!(projected_to_source(&runs, 1), Some(1));
        assert_eq!(
            projected_to_source(&runs, 9),
            Some(3),
            "projected 'c' (byte 9) maps to source byte 3"
        );
        // Hits inside the adornment snap to its anchor.
        for projected in 2..8 {
            assert_eq!(
                projected_to_source(&runs, projected),
                Some(2),
                "adornment hit at projected {projected} snaps to the anchor"
            );
        }
        // Past-the-end hits clamp into the last run.
        assert_eq!(projected_to_source(&runs, 999), Some(5));
        // Empty map: nothing to hit.
        assert_eq!(projected_to_source(&[], 0), None);
    }

    #[test]
    fn tab_projection_uses_shared_stops_and_unicode_columns() {
        let projected = |text: &str| {
            projected_rich_chunks(text, &[], &[], &[])
                .into_iter()
                .map(|chunk| chunk.text)
                .collect::<String>()
        };

        assert_eq!(projected("\t"), "        ", "column 0 advances to 8");
        assert_eq!(projected("1234567\t"), "1234567 ", "column 7 advances to 8");
        assert_eq!(
            projected("12345678\t"),
            "12345678        ",
            "column 8 advances to 16"
        );
        assert_eq!(
            projected("界\t\n\u{301}\t"),
            "界      \n\u{301}        ",
            "wide scalars count as two, zero-width scalars as zero, and newline resets"
        );
    }

    #[test]
    fn tab_projection_preserves_source_and_adornment_provenance_and_style() {
        let red = CellColor::Rgb(255, 0, 0);
        let chunks = projected_rich_chunks(
            "1234567\tX",
            &[span(7, 8, red)],
            &[adornment(0, AdornmentPlacement::AtOffset, "\t")],
            &[],
        );
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.text.as_str())
                .collect::<String>(),
            "        1234567 X",
            "the adornment tab participates in the same logical column stream"
        );
        let source_tab = chunks
            .iter()
            .find(|chunk| matches!(chunk.source, ChunkSource::SourceTab { start: 7 }))
            .expect("source tab has a first-class projected run");
        assert_eq!(source_tab.text, " ");
        assert_eq!(source_tab.color, cell_color_to_glyphon(red));
        assert!(
            chunks.iter().any(
                |chunk| matches!(chunk.source, ChunkSource::Adornment { anchor: 0 })
                    && chunk.text == "        "
            ),
            "adornment tabs expand without pretending to be source bytes"
        );
    }

    #[test]
    fn tab_projection_moves_chunks_without_tabs_unchanged() {
        let text = String::from("wide 界 and plain");
        let allocation = text.as_ptr();
        let chunks = expand_chunk_tabs(vec![RichChunk {
            text,
            color: None,
            source: ChunkSource::Source { start: 0 },
        }]);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text.as_ptr(), allocation);
    }

    #[test]
    fn source_tab_projection_boundaries_are_bidirectional() {
        let chunks = projected_rich_chunks("\tX", &[], &[], &[]);
        let (runs, _) = build_hit_runs(&chunks);

        assert_eq!(source_to_projected(&chunks, 0), Some(0));
        assert_eq!(source_to_projected(&chunks, 1), Some(8));
        assert_eq!(source_to_projected(&chunks, 2), Some(9));
        assert_eq!(projected_to_source(&runs, 0), Some(0));
        for projected in 1..=8 {
            assert_eq!(
                projected_to_source(&runs, projected),
                Some(1),
                "projected boundary {projected} inside the tab maps after its source byte"
            );
        }
        assert_eq!(projected_to_source(&runs, 9), Some(2));
    }

    #[test]
    fn adornment_tab_keeps_left_gravity_in_source_mapping() {
        let chunks = projected_rich_chunks(
            "X",
            &[],
            &[adornment(0, AdornmentPlacement::AtOffset, "\t")],
            &[],
        );
        let (runs, _) = build_hit_runs(&chunks);

        assert_eq!(source_to_projected(&chunks, 0), Some(0));
        assert_eq!(source_to_projected(&chunks, 1), Some(9));
        for projected in 0..8 {
            assert_eq!(projected_to_source(&runs, projected), Some(0));
        }
    }

    #[test]
    fn optimistic_delete_range_covers_single_codepoints_only() {
        let none = Modifiers::NONE;
        let text = "aé😀b";

        // Backspace deletes the codepoint before the cursor, whatever
        // its width: 'é' is 2 bytes, '😀' is 4.
        assert_eq!(
            optimistic_delete_range(text, 3, ProtocolKey::Backspace, none),
            Some((1, 3)),
            "backspace before the cursor crosses the full 'é'"
        );
        assert_eq!(
            optimistic_delete_range(text, 7, ProtocolKey::Backspace, none),
            Some((3, 7)),
            "backspace crosses the full '😀'"
        );
        // Delete removes the codepoint at the cursor.
        assert_eq!(
            optimistic_delete_range(text, 1, ProtocolKey::Delete, none),
            Some((1, 3))
        );
        assert_eq!(
            optimistic_delete_range(text, 7, ProtocolKey::Delete, none),
            Some((7, 8))
        );

        // Buffer edges: nothing to delete ⇒ round-trip (daemon no-op).
        assert_eq!(
            optimistic_delete_range(text, 0, ProtocolKey::Backspace, none),
            None
        );
        assert_eq!(
            optimistic_delete_range(text, text.len(), ProtocolKey::Delete, none),
            None
        );
        // Mid-codepoint (stale) cursor ⇒ round-trip, never a panic.
        assert_eq!(
            optimistic_delete_range(text, 2, ProtocolKey::Backspace, none),
            None
        );
        // Modified variants are separate bindings (C-BS word delete).
        assert_eq!(
            optimistic_delete_range(text, 3, ProtocolKey::Backspace, Modifiers::CTRL),
            None
        );
        // Non-delete keys are not this helper's business.
        assert_eq!(
            optimistic_delete_range(text, 3, ProtocolKey::Char('x'), none),
            None
        );
    }

    #[test]
    fn incoming_frames_translate_through_unconfirmed_edits() {
        // A frame computed at daemon generation G arrives while one
        // local optimistic insert (scalar G+1: 3 bytes at byte 5) is
        // still unconfirmed: the frame's ranges must shift through it.
        let unconfirmed = vec![(
            11u64,
            TextProjectionEdit {
                start: 5,
                old_end: 5,
                inserted_len: 3,
            },
        )];
        let segments = vec![StyleSegment {
            range: ByteRange { start: 0, end: 20 },
            spans: vec![
                StyleSpan {
                    range: ByteRange { start: 2, end: 4 },
                    style: CellStyle::default(),
                },
                StyleSpan {
                    range: ByteRange { start: 10, end: 14 },
                    style: CellStyle::default(),
                },
            ],
        }];

        let translated = translate_style_segments(segments, &unconfirmed);
        assert_eq!(translated.len(), 1);
        assert_eq!(
            translated[0].range,
            ByteRange { start: 0, end: 23 },
            "segment range expands over the unconfirmed insert"
        );
        assert_eq!(
            translated[0].spans[0].range,
            ByteRange { start: 2, end: 4 },
            "spans before the insert are untouched"
        );
        assert_eq!(
            translated[0].spans[1].range,
            ByteRange { start: 13, end: 17 },
            "spans after the insert shift right by its length"
        );

        // With no unconfirmed edits the frame passes through as-is.
        let untouched = translate_style_segments(
            vec![StyleSegment {
                range: ByteRange { start: 0, end: 20 },
                spans: Vec::new(),
            }],
            &[],
        );
        assert_eq!(untouched[0].range, ByteRange { start: 0, end: 20 });
    }

    #[test]
    fn cached_style_ranges_translate_through_deletions() {
        let edit = TextProjectionEdit {
            start: 5,
            old_end: 9,
            inserted_len: 0,
        };

        assert_eq!(
            translate_byte_range(ByteRange { start: 12, end: 16 }, edit),
            Some(ByteRange { start: 8, end: 12 }),
            "ranges after the deletion shift left"
        );
        assert_eq!(
            translate_byte_range(ByteRange { start: 3, end: 12 }, edit),
            Some(ByteRange { start: 3, end: 8 }),
            "ranges spanning the deletion shrink"
        );
        assert_eq!(
            translate_byte_range(ByteRange { start: 6, end: 8 }, edit),
            None,
            "ranges fully removed by the deletion drop"
        );
    }

    #[test]
    fn editing_a_diagnostic_clears_it_optimistically_but_holds_untouched_ones() {
        let diag = |start, end| Decoration {
            range: ByteRange { start, end },
            kind: DecorationKind::DiagnosticError,
        };
        // Insert the missing char right at the 1-byte widened anchor of
        // an end-of-line "expected COMMA": the squiggle clears now.
        let mut decos = vec![diag(11, 12), diag(40, 50)];
        translate_decorations(
            &mut decos,
            TextProjectionEdit {
                start: 11,
                old_end: 11,
                inserted_len: 1,
            },
        );
        // The touched diagnostic is gone; the far one is kept (shifted
        // right by the insert) — hold-while-stale still applies to it.
        assert_eq!(decos.len(), 1, "only the touched diagnostic clears");
        assert_eq!(decos[0].range, ByteRange { start: 41, end: 51 });

        // A non-diagnostic decoration over the edited region is never
        // dropped — selection / current-line translate as before.
        let mut sel = vec![Decoration {
            range: ByteRange { start: 8, end: 14 },
            kind: DecorationKind::Selection,
        }];
        translate_decorations(
            &mut sel,
            TextProjectionEdit {
                start: 10,
                old_end: 10,
                inserted_len: 2,
            },
        );
        assert_eq!(sel.len(), 1, "selection survives an edit in its range");
    }

    #[test]
    fn source_line_range_handles_empty_and_leading_newline() {
        assert_eq!(source_line_range("", 0), (0, 0));
        // "\nx": cursor 0 is on the empty first line [0, 1).
        assert_eq!(source_line_range("\nx", 0), (0, 1));
        // cursor 1 is on line 1 → [1, 2).
        assert_eq!(source_line_range("\nx", 1), (1, 2));
    }

    #[test]
    fn bg_color_helper_covers_selection_current_line_and_search() {
        // Sessions 9.1 + 9.2: Selection and CurrentLine paint.
        assert!(decoration_kind_to_bg_color(DecorationKind::Selection).is_some());
        assert!(decoration_kind_to_bg_color(DecorationKind::CurrentLine).is_some());

        // In-buffer search (Q#SR4): both match kinds wash a bg.
        assert!(decoration_kind_to_bg_color(DecorationKind::SearchMatch).is_some());
        assert!(decoration_kind_to_bg_color(DecorationKind::SearchMatchActive).is_some());

        // Underline-only kinds belong to the underline helper (T M4.6
        // parity: squiggle bars, not text recoloring).
        for kind in [
            DecorationKind::DiagnosticError,
            DecorationKind::DiagnosticWarning,
            DecorationKind::DiagnosticInfo,
            DecorationKind::DiagnosticHint,
        ] {
            assert!(decoration_kind_to_bg_color(kind).is_none());
            assert!(decoration_kind_to_underline_color(kind).is_some());
        }
    }

    #[test]
    fn underline_and_bg_helpers_are_disjoint_total_cover() {
        // Every DecorationKind is renderable by exactly one helper.
        // Adding a new kind without updating one of the helpers should
        // fail this assertion.
        for kind in [
            DecorationKind::Selection,
            DecorationKind::SearchMatch,
            DecorationKind::SearchMatchActive,
            DecorationKind::CurrentLine,
            DecorationKind::DiagnosticError,
            DecorationKind::DiagnosticWarning,
            DecorationKind::DiagnosticInfo,
            DecorationKind::DiagnosticHint,
        ] {
            let ul = decoration_kind_to_underline_color(kind).is_some();
            let bg = decoration_kind_to_bg_color(kind).is_some();
            assert!(
                ul ^ bg,
                "{kind:?}: underline={ul} bg={bg} — should be exactly one"
            );
        }
    }

    #[test]
    fn projected_rich_chunks_tolerates_mid_codepoint_boundaries() {
        // Stale span offsets (from a prior generation) can land inside a
        // multi-byte char after an edit. "ab→cd": '→' is the 3 bytes
        // [2,5); a span ending at byte 3 is mid-codepoint and must not
        // panic the slice — it floors to the char start.
        let text = "ab→cd";
        let chunks = projected_rich_chunks(
            text,
            &[
                span(0, 3, CellColor::Indexed(1)),
                span(4, 9, CellColor::Indexed(2)),
            ],
            &[],
            &[],
        );
        let rendered: String = chunks.iter().map(|chunk| chunk.text.as_str()).collect();
        assert_eq!(rendered, text, "chunks must reassemble the original text");
    }

    #[test]
    fn clip_rebase_range_clips_to_slice_and_subtracts_vstart() {
        // Visible slice is whole-file bytes [10, 20).
        assert_eq!(clip_rebase_range(12, 18, 10, 20), Some((2, 8))); // inside
        assert_eq!(clip_rebase_range(5, 15, 10, 20), Some((0, 5))); // clipped left
        assert_eq!(clip_rebase_range(15, 25, 10, 20), Some((5, 10))); // clipped right
        assert_eq!(clip_rebase_range(10, 20, 10, 20), Some((0, 10))); // exact
        assert_eq!(clip_rebase_range(0, 8, 10, 20), None); // entirely before
        assert_eq!(clip_rebase_range(20, 30, 10, 20), None); // entirely after
        assert_eq!(clip_rebase_range(14, 14, 10, 20), None); // empty range
        // vstart 0 is the unscrolled identity case.
        assert_eq!(clip_rebase_range(3, 7, 0, 100), Some((3, 7)));
    }

    #[test]
    fn floor_char_boundary_snaps_into_multibyte_char() {
        let text = "ab→cd"; // '→' = bytes [2,5)
        assert_eq!(floor_char_boundary(text, 0), 0);
        assert_eq!(floor_char_boundary(text, 2), 2);
        assert_eq!(floor_char_boundary(text, 3), 2); // inside '→' → floor to 2
        assert_eq!(floor_char_boundary(text, 4), 2);
        assert_eq!(floor_char_boundary(text, 5), 5);
        assert_eq!(floor_char_boundary(text, 99), text.len());
    }

    #[test]
    fn projected_rich_chunks_inserts_at_offset_without_source_bytes() {
        let chunks = projected_rich_chunks(
            "abcd",
            &[],
            &[adornment(2, AdornmentPlacement::AtOffset, "X")],
            &[],
        );

        assert_eq!(chunk_texts(&chunks), vec!["ab", "X", "cd"]);
        let rendered: String = chunks.iter().map(|chunk| chunk.text.as_str()).collect();
        assert_eq!(rendered, "abXcd");
    }

    #[test]
    fn inline_adornment_does_not_shift_source_style_ranges() {
        let chunks = projected_rich_chunks(
            "abcd",
            &[span(2, 4, CellColor::Indexed(1))],
            &[adornment(2, AdornmentPlacement::AtOffset, "X")],
            &[],
        );

        assert_eq!(chunk_texts(&chunks), vec!["ab", "X", "cd"]);
        assert!(chunks[0].color.is_none());
        assert!(
            chunks[1].color.is_some(),
            "default-styled virtual text should render as muted adornment text"
        );
        assert!(
            chunks[2].color.is_some(),
            "source styling must still begin at source byte 2"
        );
    }

    #[test]
    fn unsupported_adornment_placements_are_ignored_for_session_6() {
        let chunks = projected_rich_chunks(
            "abcd",
            &[],
            &[
                adornment(0, AdornmentPlacement::BeforeLine, "before"),
                adornment(4, AdornmentPlacement::EndOfLine, "end"),
                resource_adornment(2, AdornmentPlacement::AtOffset),
            ],
            &[],
        );

        assert_eq!(chunk_texts(&chunks), vec!["abcd"]);
    }

    #[test]
    fn adornment_anchor_past_end_clamps_to_end() {
        let chunks = projected_rich_chunks(
            "abcd",
            &[],
            &[adornment(99, AdornmentPlacement::AtOffset, "X")],
            &[],
        );

        assert_eq!(chunk_texts(&chunks), vec!["abcd", "X"]);
    }

    #[test]
    #[allow(clippy::too_many_lines, reason = "one table of band and mapping cases")]
    fn minimap_band_and_inverse_line_mapping() {
        // 800×600 surface: band x = [800-12-48, 800-12) = [740, 788).
        // The status band reserves 26px (Q#S3), so the text area
        // ends at 574 and the minimap column is y = [12, 562)
        // (height 550).
        assert!(minimap_band_contains(
            750.0,
            100.0,
            800,
            600,
            FontMetrics::default(),
            PanelBandInset::ABSENT,
        ));
        assert!(
            !minimap_band_contains(
                739.0,
                100.0,
                800,
                600,
                FontMetrics::default(),
                PanelBandInset::ABSENT
            ),
            "left of band"
        );
        assert!(
            !minimap_band_contains(
                788.0,
                100.0,
                800,
                600,
                FontMetrics::default(),
                PanelBandInset::ABSENT
            ),
            "right of band"
        );
        assert!(
            !minimap_band_contains(
                750.0,
                5.0,
                800,
                600,
                FontMetrics::default(),
                PanelBandInset::ABSENT
            ),
            "above band"
        );
        assert!(
            !minimap_band_contains(
                750.0,
                563.0,
                800,
                600,
                FontMetrics::default(),
                PanelBandInset::ABSENT
            ),
            "below band (status strip)"
        );
        // Too-narrow surfaces have no minimap at all.
        assert!(!minimap_band_contains(
            100.0,
            100.0,
            150,
            600,
            FontMetrics::default(),
            PanelBandInset::ABSENT,
        ));

        // Inverse mapping: height = 550; 100 lines. Top → line 0,
        // bottom → last line, midpoint → ~half.
        assert_eq!(
            minimap_y_to_line(
                12.0,
                600,
                100,
                FontMetrics::default(),
                PanelBandInset::ABSENT
            ),
            Some(0)
        );
        assert_eq!(
            minimap_y_to_line(
                561.9,
                600,
                100,
                FontMetrics::default(),
                PanelBandInset::ABSENT
            ),
            Some(99)
        );
        assert_eq!(
            minimap_y_to_line(
                12.0 + 275.0,
                600,
                100,
                FontMetrics::default(),
                PanelBandInset::ABSENT
            ),
            Some(50)
        );
        // Out-of-band y clamps rather than panics (scrubbing wanders).
        assert_eq!(
            minimap_y_to_line(
                0.0,
                600,
                100,
                FontMetrics::default(),
                PanelBandInset::ABSENT
            ),
            Some(0)
        );
        assert_eq!(
            minimap_y_to_line(
                9999.0,
                600,
                100,
                FontMetrics::default(),
                PanelBandInset::ABSENT
            ),
            Some(99)
        );
        assert_eq!(
            minimap_y_to_line(
                100.0,
                600,
                0,
                FontMetrics::default(),
                PanelBandInset::ABSENT
            ),
            None,
            "empty file"
        );
    }

    #[test]
    fn edge_scroll_direction_bands() {
        // 600px surface: up-band y < 16 + 24 = 40; the text area
        // ends at 574 (status band, Q#S3), so the down-band is
        // y > 574 - 24 = 550.
        assert_eq!(
            edge_scroll_direction(10.0, 600, FontMetrics::default(), PanelBandInset::ABSENT),
            Some(-1)
        );
        assert_eq!(
            edge_scroll_direction(39.9, 600, FontMetrics::default(), PanelBandInset::ABSENT),
            Some(-1)
        );
        assert_eq!(
            edge_scroll_direction(40.0, 600, FontMetrics::default(), PanelBandInset::ABSENT),
            None,
            "interior"
        );
        assert_eq!(
            edge_scroll_direction(300.0, 600, FontMetrics::default(), PanelBandInset::ABSENT),
            None
        );
        assert_eq!(
            edge_scroll_direction(550.0, 600, FontMetrics::default(), PanelBandInset::ABSENT),
            None,
            "band edge exclusive"
        );
        assert_eq!(
            edge_scroll_direction(551.0, 600, FontMetrics::default(), PanelBandInset::ABSENT),
            Some(1)
        );
    }

    #[test]
    fn scroll_indicator_matches_tui_formula() {
        // Verbatim port of the TUI's format (Q#S1) — both frontends
        // must read the same.
        assert_eq!(format_scroll_indicator(0, 10, 1, 0), "All");
        assert_eq!(format_scroll_indicator(0, 50, 30, 10), "All");
        assert_eq!(format_scroll_indicator(0, 10, 100, 5), "Top");
        assert_eq!(format_scroll_indicator(90, 10, 100, 95), "Bot");
        assert_eq!(format_scroll_indicator(40, 10, 100, 49), "50%");
    }

    #[test]
    fn minimap_rects_project_line_styles_as_right_side_bands() {
        let red = style_with_fg(CellColor::Rgb(255, 0, 0));
        let blue = style_with_fg(CellColor::Rgb(0, 0, 255));
        let shapes = minimap_line_shapes("alpha\nbeta\ngamma\ndelta");
        let rects = minimap_rects(
            &[red, red, blue, blue],
            &shapes,
            240,
            80,
            0,
            2,
            FontMetrics::default(),
            PanelBandInset::ABSENT,
        );

        assert!(
            rects
                .iter()
                .any(|r| color_close(r.color, rgb_to_minimap_color(255, 0, 0))),
            "red line summary band should render"
        );
        assert!(
            rects
                .iter()
                .any(|r| color_close(r.color, rgb_to_minimap_color(0, 0, 255))),
            "blue line summary band should render"
        );
        assert!(
            rects
                .iter()
                .any(|r| color_close(r.color, MINIMAP_THUMB_FILL)),
            "visible-window affordance should render"
        );
    }

    #[test]
    fn minimap_rects_bucket_large_files_to_pixel_rows() {
        let red = style_with_fg(CellColor::Rgb(255, 0, 0));
        let blue = style_with_fg(CellColor::Rgb(0, 0, 255));
        let lines: Vec<_> = (0..10_000)
            .map(|idx| if idx % 2 == 0 { red } else { blue })
            .collect();
        let shapes = vec![
            MinimapLineShape {
                indent_cols: 0,
                content_cols: 40,
            };
            lines.len()
        ];

        let rects = minimap_rects(
            &lines,
            &shapes,
            240,
            120,
            0,
            30,
            FontMetrics::default(),
            PanelBandInset::ABSENT,
        );

        let pixel_rows = (120.0 - MINIMAP_TOP - MINIMAP_BOTTOM).round() as usize;
        assert!(
            rects.len() <= pixel_rows + 4,
            "minimap must bucket by visible rows, not emit per source line"
        );
    }

    #[test]
    fn minimap_downsampling_survives_a_slab_of_blank_lines() {
        // Regression: `dominant_line_shape` counted only lines with
        // content, then built its average with `then_some` --- which
        // evaluates its argument eagerly, so `indent_sum / count`
        // divided by zero whenever a downsampled pixel row covered
        // nothing but blank lines. Reachable on any long file with a
        // run of blank lines, which is precisely when the bucketing
        // branch runs at all.
        let red = style_with_fg(CellColor::Rgb(255, 0, 0));
        let lines = vec![red; 10_000];
        // Every line blank: `minimap_line_shape("")` yields
        // `content_cols == 0`, so `has_content()` is false throughout
        // and every bucket counts zero contentful lines.
        let shapes = vec![
            MinimapLineShape {
                indent_cols: 0,
                content_cols: 0,
            };
            lines.len()
        ];

        let rects = minimap_rects(
            &lines,
            &shapes,
            240,
            120,
            0,
            30,
            FontMetrics::default(),
            PanelBandInset::ABSENT,
        );

        // The strokes are all suppressed (no content to draw), but the
        // thumb still paints --- the point is that this returns at all.
        assert!(
            rects.len() <= 8,
            "blank slabs must emit no line strokes, got {}",
            rects.len()
        );
    }

    #[test]
    fn minimap_downsampling_averages_only_contentful_lines() {
        // Guards the other half: a bucket that mixes blank and
        // contentful lines must average over the contentful ones only,
        // so the fix cannot regress into `count = slice.len()`.
        let blank = MinimapLineShape {
            indent_cols: 0,
            content_cols: 0,
        };
        let solid = MinimapLineShape {
            indent_cols: 4,
            content_cols: 20,
        };
        let shapes = [blank, solid, solid, blank];

        let shape = dominant_line_shape(&shapes, 0, 4).expect("bucket has contentful lines");

        assert_eq!(shape.indent_cols, 4, "blank lines must not dilute indent");
        assert_eq!(shape.content_cols, 20, "blank lines must not dilute length");
    }

    #[test]
    fn minimap_dominant_line_shape_is_none_for_an_empty_bucket() {
        let shape = dominant_line_shape(&[], 0, 0);
        assert!(shape.is_none(), "an empty bucket has no shape");
    }

    #[test]
    fn minimap_hidden_when_surface_is_too_narrow() {
        let lines = [style_with_fg(CellColor::Rgb(255, 0, 0))];
        let shapes = [MinimapLineShape {
            indent_cols: 0,
            content_cols: 10,
        }];

        assert!(
            minimap_rects(
                &lines,
                &shapes,
                120,
                120,
                0,
                1,
                FontMetrics::default(),
                PanelBandInset::ABSENT
            )
            .is_empty()
        );
    }

    #[test]
    fn minimap_rects_use_line_shape_for_indent_and_length() {
        let red = style_with_fg(CellColor::Rgb(255, 0, 0));
        let shapes = [
            MinimapLineShape {
                indent_cols: 0,
                content_cols: 80,
            },
            MinimapLineShape {
                indent_cols: 24,
                content_cols: 12,
            },
        ];

        let rects = minimap_rects(
            &[red, red],
            &shapes,
            240,
            80,
            0,
            2,
            FontMetrics::default(),
            PanelBandInset::ABSENT,
        );
        let strokes: Vec<_> = rects
            .iter()
            .filter(|r| color_close(r.color, rgb_to_minimap_color(255, 0, 0)))
            .collect();

        assert_eq!(strokes.len(), 2);
        assert!(
            strokes[1].x > strokes[0].x,
            "indented source line should shift right in the minimap"
        );
        assert!(
            strokes[1].w < strokes[0].w,
            "shorter source line should draw a shorter minimap stroke"
        );
    }

    #[test]
    fn minimap_line_shapes_preserve_trailing_empty_line() {
        let shapes = minimap_line_shapes("a\n");

        assert_eq!(
            shapes,
            vec![
                MinimapLineShape {
                    indent_cols: 0,
                    content_cols: 1,
                },
                MinimapLineShape::default(),
            ]
        );
    }

    #[test]
    fn minimap_columns_match_code_tab_and_unicode_widths() {
        assert_eq!(
            minimap_line_shapes("\tX\n1234567\tX\n界\u{301}\tX"),
            vec![
                MinimapLineShape {
                    indent_cols: 8,
                    content_cols: 1,
                },
                MinimapLineShape {
                    indent_cols: 0,
                    content_cols: 9,
                },
                MinimapLineShape {
                    indent_cols: 0,
                    content_cols: 9,
                },
            ]
        );
    }

    #[test]
    fn minimap_rects_encode_six_vertices_per_quad() {
        let rect = MinimapRect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
            color: rgb_to_minimap_color(255, 0, 0),
        };

        let bytes = rects_to_vertex_bytes(&[rect], 100, 100);

        assert_eq!(bytes.len(), 6 * QUAD_VERTEX_STRIDE as usize);
        assert!((f32_at(&bytes, 0) + 1.0).abs() < 0.001);
        assert!((f32_at(&bytes, 1) - 1.0).abs() < 0.001);
        assert!((f32_at(&bytes, 2) - 1.0).abs() < 0.001);
    }

    #[test]
    fn squiggle_vertices_carry_centerline_relative_uv() {
        // Q#W1: uv.x is the absolute screen pixel x (so phase is
        // continuous across rects); uv.y is signed px from the band
        // centerline (±h/2). Layout per vertex: [ndc_x, ndc_y, uv_x,
        // uv_y, r, g, b, a] — 8 floats.
        let rect = MinimapRect {
            x: 10.0,
            y: 20.0,
            w: 30.0,
            h: 6.0,
            color: [0.9, 0.1, 0.2, 1.0],
        };
        let bytes = squiggles_to_vertex_bytes(&[rect], 100, 100);
        assert_eq!(bytes.len(), 6 * SQUIGGLE_VERTEX_STRIDE as usize);

        // Vertex 0 = top-left: uv = (left x, -h/2).
        assert!((f32_at(&bytes, 2) - 10.0).abs() < 0.001, "uv.x left");
        assert!((f32_at(&bytes, 3) + 3.0).abs() < 0.001, "uv.y top = -h/2");
        // Color rides every vertex.
        assert!((f32_at(&bytes, 4) - 0.9).abs() < 0.001);
        assert!((f32_at(&bytes, 7) - 1.0).abs() < 0.001);
        // Vertex 2 = bottom-right (8 floats in): uv = (right x, +h/2).
        assert!((f32_at(&bytes, 16 + 2) - 40.0).abs() < 0.001, "uv.x right");
        assert!((f32_at(&bytes, 16 + 3) - 3.0).abs() < 0.001, "uv.y bottom");
    }

    #[test]
    fn squiggles_skip_degenerate_rects() {
        let zero_w = MinimapRect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 6.0,
            color: [1.0, 0.0, 0.0, 1.0],
        };
        assert!(squiggles_to_vertex_bytes(&[zero_w], 100, 100).is_empty());
        assert!(squiggles_to_vertex_bytes(&[zero_w], 0, 100).is_empty());
    }

    // --- Headless render harness (F-014) ---------------------------------
    //
    // These render a real frame through the actual `render_to_view`
    // composition path to an offscreen texture and read the pixels back.
    // They skip (not fail) when no wgpu adapter is available — a dev box
    // without working Vulkan, or CI without lavapipe.

    /// Detail-free minibuffer rows from bare labels — what a `12..=22`
    /// daemon's frozen `MinibufferPrompt` lands as.
    fn detailless_rows<I, S>(labels: I) -> Vec<MinibufferRow>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        labels
            .into_iter()
            .map(|label| MinibufferRow {
                label: label.into(),
                detail: None,
            })
            .collect()
    }

    /// Build a headless `State`, or return `None` and log when there's no
    /// adapter so the caller can skip. When `PMACS_REQUIRE_GPU` is set
    /// (CI, where lavapipe is installed) a missing adapter is a hard
    /// failure instead — so a broken software-rasterizer setup can't pass
    /// as a silently-skipped green (F-014).
    fn headless_or_skip(width: u32, height: u32, text: &str) -> Option<State> {
        let state = State::new_headless(width, height, text);
        if state.is_none() {
            assert!(
                std::env::var_os("PMACS_REQUIRE_GPU").is_none(),
                "PMACS_REQUIRE_GPU is set but no wgpu adapter was available"
            );
            eprintln!("skipping headless render test: no wgpu adapter available");
        }
        state
    }

    #[test]
    fn headless_render_produces_a_full_nonblank_frame() {
        let Some(mut state) = headless_or_skip(320, 240, "fn main() {}") else {
            return;
        };
        let px = state.render_offscreen();
        assert_eq!(px.len(), 320 * 240 * 4, "packed RGBA8 of the whole frame");
        // A real frame varies (text ink over the background). A single
        // uniform value would mean nothing composited.
        let first = px[0];
        assert!(
            px.iter().any(|&b| b != first),
            "frame is a single uniform value — nothing appears to have rendered"
        );
    }

    // --- Inline-math slice acceptance (framing §5, criteria 5–11/14/15) --
    //
    // Everything here renders through the REAL `render_to_view`
    // composition to offscreen pixels: a layout engine wired to
    // nothing cannot pass these (the framing's central vacuity worry).

    /// The first suppressed span's spacer chunk in the shaped slice:
    /// `(span_start, span_end, spacer_text)`, line-relative.
    fn math_chunk(state: &State) -> Option<(u64, u64, String)> {
        state.line_chunk_cache.iter().flatten().find_map(|c| {
            if let ChunkSource::MathBox { start, end } = c.source {
                Some((start, end, c.text.clone()))
            } else {
                None
            }
        })
    }

    /// Pixel x-extents of the first placed box and of its full spacer:
    /// `(box_left, box_right, spacer_right)`.
    fn box_pixels(state: &State) -> Option<(f32, f32, f32)> {
        let placed = state
            .line_math_cache
            .iter()
            .flat_map(|m| &m.placed)
            .next()?;
        let lo = placed.projected_start;
        let hi = lo + placed.projected_len;
        for run in state.buffer.layout_runs() {
            let Some(anchor) = run.glyphs.iter().find(|g| g.start as u64 == lo) else {
                continue;
            };
            let left = state.text_left() + anchor.x;
            let spacer_right = run
                .glyphs
                .iter()
                .filter(|g| (g.start as u64) >= lo && (g.start as u64) < hi)
                .map(|g| g.x + g.w)
                .fold(anchor.x, f32::max);
            return Some((
                left,
                left + placed.boxed.width,
                state.text_left() + spacer_right,
            ));
        }
        None
    }

    /// Whether the pixel at `(x, y)` differs from the frame's own
    /// background sample by more than anti-aliasing noise. Relative to
    /// a sampled corner pixel, so the sRGB target encoding is
    /// irrelevant.
    fn differs_from_bg(px: &[u8], width: u32, bg: [u8; 3], x: u32, y: u32) -> bool {
        let i = ((y * width + x) * 4) as usize;
        px[i].abs_diff(bg[0]) > 25
            || px[i + 1].abs_diff(bg[1]) > 25
            || px[i + 2].abs_diff(bg[2]) > 25
    }

    fn bg_sample(px: &[u8], width: u32) -> [u8; 3] {
        let i = ((8 * width + 8) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    }

    fn region(px: &[u8], width: u32, x0: u32, x1: u32, y0: u32, y1: u32) -> Vec<u8> {
        let mut out = Vec::new();
        for y in y0..y1 {
            out.extend_from_slice(
                &px[((y * width + x0) * 4) as usize..((y * width + x1) * 4) as usize],
            );
        }
        out
    }

    /// Acceptance 5 + 10 (first half): a suppressed span draws INK the
    /// same projected text without math does not, while every pixel
    /// left of the box is identical — so the difference is the drawn
    /// box, not a reflow artifact.
    #[test]
    fn math_box_renders_ink_where_a_plain_spacer_renders_none() {
        let Some(mut state) = headless_or_skip(480, 120, "before $x^2$ after") else {
            return;
        };
        // No caret anywhere: every gate is empty, the span suppresses.
        let with_math = state.render_offscreen();
        let (_, _, spacer) = math_chunk(&state).expect("the span must suppress");
        let (bl, br, _) = box_pixels(&state).expect("the box is placed");
        // A buffer holding the LITERAL projected text — spacer spaces
        // in place of the math — shapes the identical glyph stream
        // with an empty box region.
        let plain_text = format!("before {spacer} after");
        let Some(mut plain_state) = headless_or_skip(480, 120, &plain_text) else {
            return;
        };
        let plain = plain_state.render_offscreen();
        let bg = bg_sample(&with_math, 480);
        let (y0, y1) = (TEXT_TOP as u32, (TEXT_TOP + BASE_CODE_LINE_HEIGHT) as u32);
        let box_cols = (bl.floor() as u32).saturating_sub(1)..(br.ceil() as u32 + 1);
        let math_ink = (y0..y1)
            .flat_map(|y| box_cols.clone().map(move |x| (x, y)))
            .filter(|&(x, y)| differs_from_bg(&with_math, 480, bg, x, y))
            .count();
        let plain_ink = (y0..y1)
            .flat_map(|y| box_cols.clone().map(move |x| (x, y)))
            .filter(|&(x, y)| differs_from_bg(&plain, 480, bg, x, y))
            .count();
        assert!(
            math_ink > 4,
            "the box region must hold drawn math ink ({math_ink} px)"
        );
        assert_eq!(
            plain_ink, 0,
            "the spacer-only control renders nothing there"
        );
        // Text BEFORE the span occupies identical pixels (acceptance 10).
        let cut = bl.floor() as u32 - 1;
        assert_eq!(
            region(&with_math, 480, 0, cut, y0, y1),
            region(&plain, 480, 0, cut, y0, y1),
            "pixels left of the box must not move"
        );
    }

    /// Acceptance 6: the fraction bar is a drawn rule — a contiguous
    /// horizontal ink run spanning most of the box, with operand ink
    /// above and below it.
    #[test]
    fn a_fraction_draws_rule_pixels_between_its_operand_rows() {
        let Some(mut state) = headless_or_skip(480, 120, r"x $\frac{ab}{cd}$ y") else {
            return;
        };
        let px = state.render_offscreen();
        let (bl, br, _) = box_pixels(&state).expect("fraction renders");
        let bg = bg_sample(&px, 480);
        let cols = bl.floor() as u32..br.ceil() as u32;
        let band = TEXT_TOP as u32..(TEXT_TOP + BASE_CODE_LINE_HEIGHT) as u32;
        let run_len = |y: u32| {
            let mut best = 0u32;
            let mut cur = 0u32;
            for x in cols.clone() {
                if differs_from_bg(&px, 480, bg, x, y) {
                    cur += 1;
                    best = best.max(cur);
                } else {
                    cur = 0;
                }
            }
            best
        };
        let width = br - bl;
        let rule_row = band
            .clone()
            .find(|&y| f64::from(run_len(y)) >= f64::from(width) * 0.8)
            .expect("some row must carry the full-width rule run");
        let ink_in_rows = |rows: std::ops::Range<u32>| {
            rows.flat_map(|y| cols.clone().map(move |x| (x, y)))
                .filter(|&(x, y)| differs_from_bg(&px, 480, bg, x, y))
                .count()
        };
        assert!(
            ink_in_rows(band.start..rule_row.saturating_sub(1)) > 0,
            "numerator ink above the rule"
        );
        assert!(
            ink_in_rows(rule_row + 2..band.end) > 0,
            "denominator ink below the rule"
        );
    }

    /// Acceptance 7 + 9 + 15: with the caret inside the span — driven
    /// through the REAL `CursorByte` arm, which owns the suppression
    /// refresh — the frame is pixel-identical to math being disabled
    /// wholesale; moving the caret out re-renders the math.
    #[test]
    fn caret_inside_a_span_shows_source_exactly_as_if_math_were_disabled() {
        let Some(mut state) = headless_or_skip(480, 120, "before $x^2$ after") else {
            return;
        };
        let buf = BufferId::next();
        state.current_buffer_id = Some(buf);
        state.apply_attach_message(InstanceMessage::CursorByte {
            buffer_id: buf,
            byte_pos: 9,
        });
        let caret_inside = state.render_offscreen();
        let engine = state.math_engine.take();
        state.reshape();
        let disabled = state.render_offscreen();
        assert_eq!(
            caret_inside, disabled,
            "caret-inside must render the raw source, exactly"
        );
        // Direction two: engine back, caret out — math returns.
        state.math_engine = engine;
        state.apply_attach_message(InstanceMessage::CursorByte {
            buffer_id: buf,
            byte_pos: 0,
        });
        let caret_outside = state.render_offscreen();
        assert!(
            math_chunk(&state).is_some(),
            "the span suppresses again once the caret leaves"
        );
        assert_ne!(caret_outside, disabled, "the math box is drawn again");
    }

    /// Acceptance 9 + 15 (+ the round-3 F3 uncoverable-glyph path) on
    /// pixels: every failure mode renders exactly as if math were
    /// disabled — no panic, no half-rendered box.
    #[test]
    fn failures_and_display_math_render_as_source() {
        for text in [r"$\frac{a$ x", r"$\unknown{}$ x", "$$x$$", "$x日$ y"] {
            let Some(mut state) = headless_or_skip(480, 120, text) else {
                return;
            };
            let with_engine = state.render_offscreen();
            assert!(
                math_chunk(&state).is_none(),
                "{text:?} must not suppress anything"
            );
            state.math_engine = None;
            state.reshape();
            let disabled = state.render_offscreen();
            assert_eq!(with_engine, disabled, "{text:?} must render as source");
        }
    }

    /// Acceptance 8: clicks inside the rendered box land on the span's
    /// start byte; the surrounding text's mapping is unchanged.
    #[test]
    fn a_click_inside_a_rendered_box_lands_on_the_span_start() {
        let Some(mut state) = headless_or_skip(480, 120, "before $x^2$ after") else {
            return;
        };
        state.current_buffer_id = Some(BufferId::next());
        let _ = state.render_offscreen();
        let (bl, _, spacer_right) = box_pixels(&state).expect("box placed");
        let y = f64::from(TEXT_TOP + 5.0);
        assert_eq!(
            state.hit_test_source_byte(f64::from(bl + 2.0), y),
            Some(7),
            "a click just inside the box snaps to the span start"
        );
        assert_eq!(
            state.hit_test_source_byte(f64::from(f32::midpoint(bl, spacer_right)), y),
            Some(7),
            "mid-box clicks snap to the span start"
        );
        assert_eq!(
            state.hit_test_source_byte(f64::from(state.text_left() + 1.0), y),
            Some(0),
            "text before the box maps as ordinary source"
        );
        // Just past the reserved width, BEFORE the following space
        // glyph's midpoint: cosmic-text rounds a click to the nearest
        // caret boundary, so a mid-glyph click legitimately rounds
        // forward — the boundary under test is the box's trailing
        // edge, which the round-3 F1 fix maps after the span.
        assert_eq!(
            state.hit_test_source_byte(f64::from(spacer_right + 1.0), y),
            Some(12),
            "the first click past the reserved width lands after the span"
        );
    }

    /// Acceptance 11's scroll-reuse bite: a line retained by the
    /// scroll path while the caret's gate state changed must be
    /// rebuilt, not reused — with suppression missing from the reuse
    /// predicate, the stale source-state line survives and this fails.
    #[test]
    fn scroll_reuse_refuses_a_line_shaped_under_a_stale_caret_gate() {
        let Some(mut state) = headless_or_skip(480, 160, "$x^2$\nsecond line\nthird line") else {
            return;
        };
        let buf = BufferId::next();
        state.current_buffer_id = Some(buf);
        state.apply_attach_message(InstanceMessage::CursorByte {
            buffer_id: buf,
            byte_pos: 2,
        });
        assert!(
            math_chunk(&state).is_none(),
            "caret inside: the span renders as source"
        );
        let source_frame = state.render_offscreen();
        // The caret leaves the span by a path that reaches the scroll
        // rebuild WITHOUT any refresh in between: reuse must refuse
        // the retained line because its gate bit is stale.
        state.own_cursor = Some(OwnCursor {
            buffer_id: buf,
            byte: 8,
        });
        state.rebuild_lines_reusing_scroll();
        assert!(
            math_chunk(&state).is_some(),
            "the stale-gated line must be rebuilt, not reused"
        );
        let rebuilt_frame = state.render_offscreen();
        assert_ne!(source_frame, rebuilt_frame, "the math box is drawn");
    }

    /// Acceptance 10 (second half): suppression toggling reflows ONLY
    /// the affected line, and the text after the span shifts by
    /// exactly the quantized projection difference.
    #[test]
    fn suppression_reflow_is_confined_to_the_affected_line() {
        let Some(mut state) = headless_or_skip(480, 160, "before $x^2$ after\nsecond line") else {
            return;
        };
        let buf = BufferId::next();
        state.current_buffer_id = Some(buf);
        let _ = state.render_offscreen();
        let (_, _, spacer) = math_chunk(&state).expect("suppressed");
        let spacer_len = spacer.len() as u64;
        // Projected position of the 'a' in "after" (byte 13) while the
        // box is rendered...
        let suppressed_projected = state.code_byte_to_projected(13).expect("maps").1 as u64;
        let suppressed_frame = state.render_offscreen();
        // ...and with the caret inside (raw source).
        state.apply_attach_message(InstanceMessage::CursorByte {
            buffer_id: buf,
            byte_pos: 9,
        });
        assert!(math_chunk(&state).is_none());
        let raw_projected = state.code_byte_to_projected(13).expect("maps").1 as u64;
        let raw_frame = state.render_offscreen();
        assert_eq!(
            suppressed_projected,
            raw_projected + spacer_len - 5,
            "text after the span shifts by exactly the quantized difference \
             (spacer {spacer_len} vs 5 source bytes)"
        );
        // The second line's pixel band is untouched by the toggle.
        let (y0, y1) = (
            (TEXT_TOP + BASE_CODE_LINE_HEIGHT) as u32,
            (TEXT_TOP + 2.0 * BASE_CODE_LINE_HEIGHT) as u32,
        );
        assert_eq!(
            region(&suppressed_frame, 480, 0, 480, y0, y1),
            region(&raw_frame, 480, 0, 480, y0, y1),
            "no other line moves"
        );
    }

    /// Acceptance 14: a selection ENDPOINT inside a span unsuppresses
    /// it (pixel-identical to math-disabled with the same selection);
    /// a selection ENCLOSING the span leaves it rendered and washes
    /// the whole reserved rectangle (Q#MS11's intersection rule).
    #[test]
    fn selection_endpoints_gate_and_enclosing_selections_wash_the_box() {
        let Some(mut state) = headless_or_skip(480, 120, "before $x^2$ after") else {
            return;
        };
        state.current_buffer_id = Some(BufferId::next());
        // (a) endpoint at byte 9, inside the span.
        state.current_decorations = vec![Decoration {
            range: ByteRange { start: 9, end: 14 },
            kind: DecorationKind::Selection,
        }];
        state.reshape();
        assert!(
            math_chunk(&state).is_none(),
            "an endpoint inside the span unsuppresses it"
        );
        let gated = state.render_offscreen();
        let engine = state.math_engine.take();
        state.reshape();
        let disabled = state.render_offscreen();
        assert_eq!(gated, disabled, "gated == raw source with the same wash");
        // (b) both endpoints outside: the span stays rendered and the
        // wash covers the whole reserved rectangle.
        state.math_engine = engine;
        state.current_decorations = vec![Decoration {
            range: ByteRange { start: 0, end: 18 },
            kind: DecorationKind::Selection,
        }];
        state.reshape();
        assert!(
            math_chunk(&state).is_some(),
            "an enclosing selection leaves the span rendered"
        );
        let washed = state.render_offscreen();
        state.current_decorations.clear();
        state.reshape();
        let unwashed = state.render_offscreen();
        let (bl, br, _) = box_pixels(&state).expect("box placed");
        let (y0, y1) = (TEXT_TOP as u32, (TEXT_TOP + BASE_CODE_LINE_HEIGHT) as u32);
        assert_ne!(
            region(&washed, 480, bl as u32, br.ceil() as u32, y0, y1),
            region(&unwashed, 480, bl as u32, br.ceil() as u32, y0, y1),
            "the wash tints the box's reserved rectangle"
        );
    }

    #[test]
    fn completion_popup_is_scoped_to_its_buffer() {
        // Buffer-switch regression (PR #93 validation finding 1): a
        // retained popup mirror must be inert — no key gating, no
        // anchor mapping — the moment `current_buffer_id` differs
        // from the popup's buffer.
        let Some(mut state) = headless_or_skip(320, 240, "hello_world he") else {
            return;
        };
        let own = BufferId::next();
        let other = BufferId::next();
        state.current_buffer_id = Some(own);
        // Headless states never declare a viewport; anchor mapping
        // reads `view_range`, so pin it to the whole text.
        state.view_range = (0, state.current_text.len() as u64);
        state.completion = Some(CompletionLocal {
            buffer_id: own,
            anchor: 12,
            prefix_len: 2,
            rows: vec![CompletionPopupRow {
                label: "hello_world".into(),
                kind: 3,
                detail: None,
            }],
            selected: Some(0),
            total: 1,
        });
        assert!(state.completion_open_for_current_buffer());
        assert!(
            state.completion_anchor_px().is_some(),
            "the popup anchors in its own buffer"
        );
        // The window switches buffers; the mirror is stale.
        state.current_buffer_id = Some(other);
        assert!(
            !state.completion_open_for_current_buffer(),
            "a foreign-buffer popup must not gate keys"
        );
        assert!(
            state.completion_anchor_px().is_none(),
            "a foreign-buffer popup must not paint"
        );
    }

    #[test]
    fn headless_text_changes_the_rendered_frame() {
        let Some(mut empty) = headless_or_skip(320, 240, "") else {
            return;
        };
        let empty_px = empty.render_offscreen();
        let mut with_text =
            State::new_headless(320, 240, "hello pmacs").expect("adapter was just available");
        let text_px = with_text.render_offscreen();
        assert_eq!(empty_px.len(), text_px.len());
        let differing = empty_px
            .iter()
            .zip(&text_px)
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            differing > 200,
            "text should paint visible ink (only {differing} bytes differ from the empty frame)"
        );
    }

    #[test]
    fn headless_line_number_gutter_changes_the_frame() {
        // UX gutter: enabling line numbers must add ink on the left and
        // shift the code right — the rendered frame must differ.
        let Some(mut off) = headless_or_skip(400, 300, "alpha\nbeta\ngamma\ndelta\n") else {
            return;
        };
        let off_px = off.render_offscreen();
        let mut on = State::new_headless(400, 300, "alpha\nbeta\ngamma\ndelta\n")
            .expect("adapter was just available");
        on.line_numbers = LineNumberMode::Absolute;
        let on_px = on.render_offscreen();
        assert_eq!(off_px.len(), on_px.len());
        let differing = off_px.iter().zip(&on_px).filter(|(a, b)| a != b).count();
        assert!(
            differing > 200,
            "the gutter should add ink + shift the text (only {differing} bytes differ)"
        );
    }

    #[test]
    fn headless_diagnostic_gutter_sign_changes_the_frame() {
        // UX gutter sub-arc 2: with the gutter on, a diagnostic on a line
        // must add a severity-colored sign bar in the gutter — the frame
        // must differ from the same gutter with no diagnostics.
        let text = "alpha\nbeta\ngamma\n";
        let Some(mut plain) = headless_or_skip(400, 300, text) else {
            return;
        };
        plain.line_numbers = LineNumberMode::Absolute;
        plain.current_buffer_id = Some(BufferId::next());
        plain.view_range = (0, text.len() as u64);
        let plain_px = plain.render_offscreen();

        let mut with_diag =
            State::new_headless(400, 300, text).expect("adapter was just available");
        with_diag.line_numbers = LineNumberMode::Absolute;
        with_diag.current_buffer_id = Some(BufferId::next());
        with_diag.view_range = (0, text.len() as u64);
        with_diag.current_decorations.push(Decoration {
            range: ByteRange { start: 0, end: 5 }, // "alpha"
            kind: DecorationKind::DiagnosticError,
        });
        let diag_px = with_diag.render_offscreen();

        assert_eq!(plain_px.len(), diag_px.len());
        let differing = plain_px
            .iter()
            .zip(&diag_px)
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            differing > 20,
            "the diagnostic sign bar should add ink ({differing} bytes differ)"
        );
    }

    #[test]
    fn headless_relative_mode_renders_differently_from_absolute() {
        // Sub-arc 3: with the cursor on line 2, relative numbering
        // (2,1,0,1,2) must differ from absolute (1,2,3,4,5) — proving the
        // GPU renders the mode against its own cursor line.
        let text = "alpha\nbeta\ngamma\ndelta\nepsilon\n";
        let bid = BufferId::next();
        let cursor = OwnCursor {
            buffer_id: bid,
            byte: 13, // inside "gamma" (buffer line 2)
        };
        let Some(mut abs) = headless_or_skip(400, 300, text) else {
            return;
        };
        abs.current_buffer_id = Some(bid);
        abs.view_range = (0, text.len() as u64);
        abs.own_cursor = Some(cursor);
        abs.line_numbers = LineNumberMode::Absolute;
        let abs_px = abs.render_offscreen();

        let mut rel = State::new_headless(400, 300, text).expect("adapter was just available");
        rel.current_buffer_id = Some(bid);
        rel.view_range = (0, text.len() as u64);
        rel.own_cursor = Some(cursor);
        rel.line_numbers = LineNumberMode::Relative;
        let rel_px = rel.render_offscreen();

        assert_eq!(abs_px.len(), rel_px.len());
        let differing = abs_px.iter().zip(&rel_px).filter(|(a, b)| a != b).count();
        assert!(
            differing > 20,
            "relative numbering must differ from absolute ({differing} bytes differ)"
        );
    }

    #[test]
    fn gutter_aware_rel_x_clamps_the_gutter_band() {
        // F1: a click in the gutter band (left of the text origin) clamps
        // to the line start (rel_x 0), never a negative x into glyphon.
        let text = "alpha\nbeta\ngamma\n";
        let Some(mut s) = headless_or_skip(400, 300, text) else {
            return;
        };
        s.line_numbers = LineNumberMode::Absolute;
        let text_left = f64::from(s.text_left());
        assert!(text_left > f64::from(TEXT_LEFT), "the gutter is present");

        // Inside the gutter band and at the exact origin → clamped to 0.
        assert!(s.gutter_aware_rel_x(text_left - 4.0).abs() < f32::EPSILON);
        assert!(s.gutter_aware_rel_x(text_left).abs() < f32::EPSILON);
        // Well into the text → a positive text-relative x.
        assert!(s.gutter_aware_rel_x(text_left + 40.0) > 0.0);

        // With the gutter off there's no band, so a left-of-origin x passes
        // through negative (the pre-gutter behavior is unchanged).
        s.line_numbers = LineNumberMode::Off;
        assert!(s.gutter_aware_rel_x(f64::from(TEXT_LEFT) - 4.0) < 0.0);
    }

    #[test]
    fn narrow_window_drops_the_gutter() {
        // F2: a window too narrow to fit the gutter + a minimum text area
        // drops the gutter for the frame (no `left >= right`), mirroring the
        // TUI. A wide window keeps it.
        let text = "l1\nl2\nl3\n";
        let Some(mut narrow) = headless_or_skip(60, 200, text) else {
            return;
        };
        narrow.line_numbers = LineNumberMode::Absolute;
        assert!(
            narrow.gutter_width_px() < f32::EPSILON,
            "a 60px window can't fit gutter + min text → gutter dropped"
        );
        assert!(
            (narrow.text_left() - TEXT_LEFT).abs() < f32::EPSILON,
            "text origin unshifted when the gutter is dropped"
        );

        let mut wide = State::new_headless(800, 200, text).expect("adapter was just available");
        wide.line_numbers = LineNumberMode::Absolute;
        assert!(
            wide.gutter_width_px() > 0.0,
            "a wide window keeps the gutter"
        );
    }

    #[test]
    fn line_number_messages_reflow_the_code_width_both_directions() {
        let Some(mut state) = headless_or_skip(320, 240, "one\ntwo\nthree\n") else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        let plain_width = state.buffer.size().0.expect("bounded code width");

        let _ = state.apply_attach_message(InstanceMessage::LineNumbers {
            buffer_id: bid,
            mode: LineNumberMode::Absolute,
        });
        let gutter_width = state.gutter_width_px();
        assert!(gutter_width > 0.0, "precondition: the gutter fits");
        assert!(
            (state.buffer.size().0.expect("bounded code width") - (plain_width - gutter_width))
                .abs()
                < 0.01,
            "enabling line numbers must reshape at the narrower painter clip"
        );

        let _ = state.apply_attach_message(InstanceMessage::LineNumbers {
            buffer_id: bid,
            mode: LineNumberMode::Off,
        });
        assert!(
            (state.buffer.size().0.expect("bounded code width") - plain_width).abs() < 0.01,
            "disabling line numbers restores the original shaping width"
        );
    }

    #[test]
    fn incremental_line_count_digit_transitions_reflow_the_code_width() {
        let nine_lines = (0..9).map(|_| "x").collect::<Vec<_>>().join("\n");
        let Some(mut state) = headless_or_skip(320, 240, &nine_lines) else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        let _ = state.apply_attach_message(InstanceMessage::LineNumbers {
            buffer_id: bid,
            mode: LineNumberMode::Absolute,
        });
        let one_digit_width = state.buffer.size().0.expect("bounded code width");

        let insert = vec![
            loro::TextDelta::Retain {
                retain: nine_lines.chars().count(),
                attributes: None,
            },
            loro::TextDelta::Insert {
                insert: "\nx".to_owned(),
                attributes: None,
            },
        ];
        state
            .apply_loro_text_delta_batches(&[insert])
            .expect("9 -> 10 line insertion applies");
        assert_eq!(state.current_line_starts.len(), 10);
        let two_digit_width = state.buffer.size().0.expect("bounded code width");
        assert!(
            two_digit_width < one_digit_width,
            "the second gutter digit must narrow the code buffer immediately"
        );

        let delete = vec![
            loro::TextDelta::Retain {
                retain: nine_lines.chars().count(),
                attributes: None,
            },
            loro::TextDelta::Delete { delete: 2 },
        ];
        state
            .apply_loro_text_delta_batches(&[delete])
            .expect("10 -> 9 line deletion applies");
        assert_eq!(state.current_line_starts.len(), 9);
        assert!(
            (state.buffer.size().0.expect("bounded code width") - one_digit_width).abs() < 0.01,
            "crossing back to one digit restores the wider shaping clip"
        );
    }

    #[test]
    fn minimap_presence_reflows_and_refollows_a_painted_caret() {
        let text = "x".repeat(250);
        let Some(mut state) = headless_or_skip(320, 240, &text) else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: text.len() as u64,
        });
        state.ensure_caret_painted();
        assert!(
            state.caret_painted_in_code_clip(),
            "precondition: the end caret paints without a minimap"
        );
        let plain_width = state.buffer.size().0.expect("bounded code width");

        let _ = state.apply_attach_message(InstanceMessage::FileStyleSummary {
            buffer_id: bid,
            generation: 1,
            lines: vec![CellStyle::default()],
        });
        let minimap_width = state.buffer.size().0.expect("bounded code width");
        assert!(
            minimap_width < plain_width,
            "a nonempty summary reserves the minimap before reshaping"
        );
        assert!(
            state.code_scroll_residual > 0.0 && state.caret_painted_in_code_clip(),
            "a previously painted caret is followed into its newly wrapped run"
        );

        let _ = state.apply_attach_message(InstanceMessage::FileStyleSummary {
            buffer_id: bid,
            generation: 2,
            lines: Vec::new(),
        });
        assert!(
            (state.buffer.size().0.expect("bounded code width") - plain_width).abs() < 0.01,
            "an empty summary removes the minimap and restores the code width"
        );
    }

    #[test]
    fn byte_identical_snapshot_clears_minimap_geometry_before_reshape() {
        let text = "alpha\nbeta\ngamma\ndelta\n";
        let Some(mut state) = headless_or_skip(320, 240, text) else {
            return;
        };
        let first = BufferId::next();
        state.current_buffer_id = Some(first);
        let plain_width = state.buffer.size().0.expect("bounded code width");
        let _ = state.apply_attach_message(InstanceMessage::FileStyleSummary {
            buffer_id: first,
            generation: 1,
            lines: vec![CellStyle::default(); 4],
        });
        assert!(
            state.buffer.size().0.expect("bounded code width") < plain_width,
            "precondition: the summary reserves minimap width"
        );

        let doc = loro::LoroDoc::new();
        doc.get_text(LORO_TEXT_CONTAINER)
            .insert(0, text)
            .expect("insert snapshot text");
        let _ = state.apply_attach_message(InstanceMessage::BufferSnapshot {
            buffer_id: BufferId::next(),
            crdt_snapshot: doc.export(loro::ExportMode::Snapshot).expect("export"),
        });
        assert!(state.current_summary.is_none());
        assert!(
            (state.buffer.size().0.expect("bounded code width") - plain_width).abs() < 0.01,
            "a byte-identical snapshot must remove the prior buffer's minimap from the shaping clip"
        );
    }

    // -----------------------------------------------------------------
    // Themes (Q#TH5/Q#TH7/Q#TH8): GPU face application.
    // -----------------------------------------------------------------

    fn theme_face(name: &str, style: CellStyle) -> pmacs_protocol::ThemeFace {
        pmacs_protocol::ThemeFace {
            name: name.into(),
            style,
        }
    }

    fn apply_faces(state: &mut State, faces: Vec<pmacs_protocol::ThemeFace>) {
        let _ = state.apply_attach_message(InstanceMessage::ThemeFacts { faces });
    }

    fn px_at(px: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * width + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    }

    /// Exclusive pixel bounds of every RGBA pixel that differs.
    fn frame_diff_bounds(before: &[u8], after: &[u8], width: u32) -> Option<(u32, u32, u32, u32)> {
        assert_eq!(before.len(), after.len());
        let mut bounds: Option<(u32, u32, u32, u32)> = None;
        for (pixel_index, (left, right)) in before
            .chunks_exact(4)
            .zip(after.chunks_exact(4))
            .enumerate()
        {
            if left == right {
                continue;
            }
            let pixel_index = u32::try_from(pixel_index).expect("test frame fits u32");
            let pixel_x = pixel_index % width;
            let pixel_y = pixel_index / width;
            bounds = Some(match bounds {
                None => (pixel_x, pixel_y, pixel_x + 1, pixel_y + 1),
                Some((min_x, min_y, max_x, max_y)) => (
                    min_x.min(pixel_x),
                    min_y.min(pixel_y),
                    max_x.max(pixel_x + 1),
                    max_y.max(pixel_y + 1),
                ),
            });
        }
        bounds
    }
    fn statusline_segment(text: impl Into<String>, face: impl Into<String>) -> StatuslineSegment {
        StatuslineSegment {
            text: text.into(),
            face: face.into(),
        }
    }

    fn apply_statusline(
        state: &mut State,
        buffer_id: BufferId,
        left: Vec<StatuslineSegment>,
        right: Vec<StatuslineSegment>,
    ) {
        let _ = state.apply_attach_message(InstanceMessage::StatuslineSegments {
            buffer_id,
            left,
            right,
        });
    }

    fn status_facts(buffer_id: BufferId, message: Option<&str>) -> StatusFactsLocal {
        StatusFactsLocal {
            buffer_id,
            name: "main.rs".to_owned(),
            modified: true,
            diag_errors: 1,
            diag_warnings: 2,
            message: message.map(str::to_owned),
        }
    }

    #[test]
    fn statusline_wire_validation_is_atomic_and_accepts_exact_boundaries() {
        let Some(mut state) = headless_or_skip(420, 260, "text") else {
            return;
        };
        let buffer_id = BufferId::next();
        state.current_buffer_id = Some(buffer_id);
        state.status_facts = Some(status_facts(buffer_id, None));
        apply_statusline(
            &mut state,
            buffer_id,
            vec![statusline_segment("valid", "ui.modeline.good")],
            vec![statusline_segment("right", "ui.modeline")],
        );
        let valid_frame = state.render_offscreen();
        let valid_state = state
            .statusline_segments
            .clone()
            .expect("valid payload installed");
        let valid_right_cache = state.status_runs.clone();
        let valid_left_cache = state.status_left_runs.clone();

        let invalid_payloads = vec![
            (vec![statusline_segment("", "ui.modeline")], Vec::new()),
            (
                vec![statusline_segment(
                    "x".repeat(MAX_STATUSLINE_SEGMENT_BYTES + 1),
                    "ui.modeline",
                )],
                Vec::new(),
            ),
            (
                vec![statusline_segment("bad\ntext", "ui.modeline")],
                Vec::new(),
            ),
            (
                vec![statusline_segment(
                    "bad-face",
                    format!("ui.modeline.{}", "x".repeat(MAX_STATUSLINE_FACE_BYTES)),
                )],
                Vec::new(),
            ),
            (
                vec![statusline_segment("bad-face", "ui.modeline.\u{7f}")],
                Vec::new(),
            ),
            (
                vec![statusline_segment("wrong-family", "ui.statusline")],
                Vec::new(),
            ),
            (
                (0..=MAX_STATUSLINE_PROVIDERS)
                    .map(|index| statusline_segment(format!("s{index}"), "ui.modeline"))
                    .collect(),
                Vec::new(),
            ),
        ];
        for (left, right) in invalid_payloads {
            apply_statusline(&mut state, buffer_id, left, right);
            assert_eq!(state.statusline_segments.as_ref(), Some(&valid_state));
            assert_eq!(state.status_runs, valid_right_cache);
            assert_eq!(state.status_left_runs, valid_left_cache);
            assert_eq!(
                state.render_offscreen(),
                valid_frame,
                "a rejected replacement must retain the prior frame byte-for-byte"
            );
        }

        let max_face = format!(
            "ui.modeline.{}",
            "f".repeat(MAX_STATUSLINE_FACE_BYTES - "ui.modeline.".len())
        );
        let boundary: Vec<_> = (0..MAX_STATUSLINE_PROVIDERS)
            .map(|_| statusline_segment("x".repeat(MAX_STATUSLINE_SEGMENT_BYTES), &max_face))
            .collect();
        assert_eq!(
            boundary
                .iter()
                .map(|segment| segment.text.len())
                .sum::<usize>(),
            MAX_STATUSLINE_TOTAL_TEXT_BYTES
        );
        apply_statusline(&mut state, buffer_id, boundary, Vec::new());
        let installed = state.statusline_segments.as_ref().expect("boundary valid");
        assert_eq!(installed.left.len(), MAX_STATUSLINE_PROVIDERS);
        assert_eq!(installed.left[0].face.len(), MAX_STATUSLINE_FACE_BYTES);
    }

    #[test]
    fn buffer_snapshot_clears_statusline_mirror_but_keeps_theme_facts() {
        let Some(mut state) = headless_or_skip(320, 240, "same") else {
            return;
        };
        let first = BufferId::next();
        state.current_buffer_id = Some(first);
        apply_faces(
            &mut state,
            vec![theme_face(
                "ui.modeline.custom",
                CellStyle {
                    fg: CellColor::Rgb(10, 20, 30),
                    ..CellStyle::default()
                },
            )],
        );
        apply_statusline(
            &mut state,
            first,
            vec![statusline_segment("old", "ui.modeline.custom")],
            Vec::new(),
        );
        let _ = state.render_offscreen();
        assert!(state.statusline_segments.is_some());

        let doc = loro::LoroDoc::new();
        doc.get_text(LORO_TEXT_CONTAINER)
            .insert(0, "same")
            .expect("snapshot text");
        let _ = state.apply_attach_message(InstanceMessage::BufferSnapshot {
            buffer_id: BufferId::next(),
            crdt_snapshot: doc.export(loro::ExportMode::Snapshot).expect("snapshot"),
        });
        assert!(state.statusline_segments.is_none());
        assert!(state.status_runs.is_none());
        assert!(state.status_left_runs.is_none());
        assert!(state.faces.contains_key("ui.modeline.custom"));
    }

    #[test]
    fn statusline_rich_runs_preserve_builtins_separators_and_face_changes() {
        let Some(mut state) = headless_or_skip(500, 280, "text") else {
            return;
        };
        let buffer_id = BufferId::next();
        state.current_buffer_id = Some(buffer_id);
        state.status_facts = Some(status_facts(buffer_id, None));
        state.own_cursor = Some(OwnCursor { buffer_id, byte: 0 });
        apply_faces(
            &mut state,
            vec![
                theme_face(
                    "ui.modeline.red",
                    CellStyle {
                        fg: CellColor::Rgb(230, 20, 30),
                        ..CellStyle::default()
                    },
                ),
                theme_face(
                    "ui.modeline.green",
                    CellStyle {
                        fg: CellColor::Rgb(20, 220, 40),
                        ..CellStyle::default()
                    },
                ),
            ],
        );
        apply_statusline(
            &mut state,
            buffer_id,
            vec![
                statusline_segment("L1", "ui.modeline.red"),
                statusline_segment("L2", "ui.modeline"),
            ],
            vec![
                statusline_segment("R1", "ui.modeline.green"),
                statusline_segment("R2", "ui.modeline"),
            ],
        );

        let left = state.compose_status_left_runs();
        let right = state.compose_status_runs();
        let left_text: String = left.iter().map(|(text, _)| text.as_str()).collect();
        let right_text: String = right.iter().map(|(text, _)| text.as_str()).collect();
        assert_eq!(left_text, "main.rs ● L1 L2");
        assert_eq!(right_text, "R1 R2 E:1  W:2  L1:C1  All");
        let left_base = state.status_left_color();
        assert_eq!(left[1], (" ".to_owned(), left_base));
        assert_eq!(left[3], (" ".to_owned(), left_base));
        let right_base = state.status_right_base_color();
        assert_eq!(right[1], (" ".to_owned(), right_base));
        assert_eq!(right[3], (" ".to_owned(), right_base));
        assert_eq!(right[5], ("  ".to_owned(), right_base));
        assert_eq!(right[7], ("  ".to_owned(), right_base));
        assert_eq!(
            left[2].1,
            Color::rgb(230, 20, 30),
            "custom text takes the exact ThemeFacts foreground"
        );
        assert_eq!(right[0].1, Color::rgb(20, 220, 40));

        let _ = state.render_offscreen();
        let before_text: String = state
            .status_left_runs
            .as_ref()
            .expect("left shaped")
            .iter()
            .map(|(text, _)| text.as_str())
            .collect();
        apply_statusline(
            &mut state,
            buffer_id,
            vec![
                statusline_segment("L1", "ui.modeline.green"),
                statusline_segment("L2", "ui.modeline"),
            ],
            vec![
                statusline_segment("R1", "ui.modeline.green"),
                statusline_segment("R2", "ui.modeline"),
            ],
        );
        assert!(state.status_runs.is_none());
        assert!(state.status_left_runs.is_none());
        let _ = state.render_offscreen();
        let after = state.status_left_runs.as_ref().expect("left reshaped");
        assert_eq!(
            after
                .iter()
                .map(|(text, _)| text.as_str())
                .collect::<String>(),
            before_text,
            "changing only the face name keeps concatenated text constant"
        );
        assert_eq!(after[2].1, Color::rgb(20, 220, 40));
    }

    /// Worker identity Stage 1 (`docs/worker-identity-framing.md` §6):
    /// the GPU half of "both frontends render the segment".
    ///
    /// The activity indicator adds no wire message — it rides the
    /// existing `StatuslineSegments` vector as a fourth provider's
    /// element. But that is a claim about the **producer**, and says
    /// nothing about whether a consumer draws it, which is why this
    /// exists on the consumer side.
    ///
    /// Two properties specific to this segment, neither of which the
    /// existing rich-runs test covers:
    ///
    ///  * its face (`ui.modeline.activity`) is **deliberately absent
    ///    from `ThemeFacts`** — no theme sets it, and `theme_facts_msg`
    ///    ships only faces that resolve — so a consumer that dropped
    ///    segments with an unknown face would silently lose the one
    ///    thing telling the user the editor is busy;
    ///  * its text leads with a non-ASCII `⋯`, which a byte-oriented
    ///    composition step would mangle.
    #[test]
    fn the_activity_segment_survives_an_unthemed_face_and_a_non_ascii_lead() {
        let Some(mut state) = headless_or_skip(500, 280, "text") else {
            return;
        };
        let buffer_id = BufferId::next();
        state.current_buffer_id = Some(buffer_id);
        state.status_facts = Some(status_facts(buffer_id, None));
        state.own_cursor = Some(OwnCursor { buffer_id, byte: 0 });
        // One themed face, and NOT the activity one: the point is that
        // the theme has an opinion about some segments and none about
        // this one.
        apply_faces(
            &mut state,
            vec![theme_face(
                "ui.modeline.lsp",
                CellStyle {
                    fg: CellColor::Rgb(20, 220, 40),
                    ..CellStyle::default()
                },
            )],
        );
        apply_statusline(
            &mut state,
            buffer_id,
            Vec::new(),
            vec![
                statusline_segment("LSP:rust", "ui.modeline.lsp"),
                statusline_segment("⋯2 lsp textDocument/definition", "ui.modeline.activity"),
            ],
        );

        let right = state.compose_status_runs();
        let text: String = right.iter().map(|(text, _)| text.as_str()).collect();
        assert!(
            text.contains("⋯2 lsp textDocument/definition"),
            "the activity segment must reach the composed right runs \
             intact: {text:?}"
        );
        let activity = right
            .iter()
            .find(|(run, _)| run.contains('⋯'))
            .expect("activity run");
        assert_eq!(
            activity.1,
            state.status_right_base_color(),
            "an unthemed modeline face falls back to the base colour \
             rather than dropping the segment"
        );
        assert_eq!(
            right[0].1,
            Color::rgb(20, 220, 40),
            "and its themed neighbour still takes its own colour"
        );

        // And it survives the real shaping pass, not only composition.
        let _ = state.render_offscreen();
        let shaped: String = state
            .status_runs
            .as_ref()
            .expect("right shaped")
            .iter()
            .map(|(text, _)| text.as_str())
            .collect();
        assert!(
            shaped.contains("⋯2 lsp textDocument/definition"),
            "{shaped:?}"
        );
    }

    #[test]
    fn modal_left_precedence_suppresses_custom_left_but_preserves_right() {
        let Some(mut state) = headless_or_skip(420, 260, "text") else {
            return;
        };
        let buffer_id = BufferId::next();
        state.current_buffer_id = Some(buffer_id);
        state.status_facts = Some(status_facts(buffer_id, None));
        apply_statusline(
            &mut state,
            buffer_id,
            vec![statusline_segment("CUSTOM-L", "ui.modeline")],
            vec![statusline_segment("CUSTOM-R", "ui.modeline")],
        );
        assert!(state.compose_status_left_runs()[2].0.contains("CUSTOM-L"));
        let ordinary_right = state.compose_status_runs();

        state.minibuffer = Some(MinibufferLocal {
            prompt: "M-x ".to_owned(),
            input: "find".to_owned(),
            cursor: 4,
            rows: Vec::new(),
            selected: None,
            total: 0,
        });
        assert_eq!(state.compose_status_left_runs()[0].0, "M-x find");
        assert_eq!(state.compose_status_runs(), ordinary_right);

        state.minibuffer = None;
        state.search_prompt = Some(SearchPromptLocal {
            buffer_id,
            query: "needle".to_owned(),
            active: Some(0),
            total: 1,
            regex: false,
            invalid: false,
        });
        assert_eq!(
            state.compose_status_left_runs()[0].0,
            "I-search: needle (1/1)"
        );
        assert_eq!(state.compose_status_runs(), ordinary_right);

        state.search_prompt = None;
        state.status_facts = Some(status_facts(buffer_id, Some("CUSTOM-L")));
        assert_eq!(state.compose_status_left_runs()[0].0, "CUSTOM-L");
        assert_eq!(state.compose_status_left_runs().len(), 1);
        assert_eq!(state.compose_status_runs(), ordinary_right);

        state.status_facts = Some(status_facts(buffer_id, None));
        assert!(state.compose_status_left_runs()[2].0.contains("CUSTOM-L"));
    }

    #[test]
    fn theme_recolor_invalidates_both_rich_caches_and_repaints_custom_text() {
        let (width, height) = (420, 260);
        let Some(mut state) = headless_or_skip(width, height, "text") else {
            return;
        };
        let buffer_id = BufferId::next();
        state.current_buffer_id = Some(buffer_id);
        state.status_facts = Some(status_facts(buffer_id, None));
        apply_statusline(
            &mut state,
            buffer_id,
            vec![statusline_segment("RECOLOR", "ui.modeline.custom")],
            vec![statusline_segment("RECOLOR", "ui.modeline.custom")],
        );
        apply_faces(
            &mut state,
            vec![theme_face(
                "ui.modeline.custom",
                CellStyle {
                    fg: CellColor::Rgb(240, 10, 20),
                    ..CellStyle::default()
                },
            )],
        );
        let red = state.render_offscreen();
        assert_eq!(
            state.status_left_runs.as_ref().expect("left shaped")[2].1,
            Color::rgb(240, 10, 20)
        );

        apply_faces(
            &mut state,
            vec![theme_face(
                "ui.modeline.custom",
                CellStyle {
                    fg: CellColor::Rgb(10, 220, 40),
                    ..CellStyle::default()
                },
            )],
        );
        assert!(state.status_runs.is_none());
        assert!(state.status_left_runs.is_none());
        let green = state.render_offscreen();
        assert_ne!(red, green, "constant text must repaint after ThemeFacts");
        assert_eq!(
            state.status_left_runs.as_ref().expect("left reshaped")[2].1,
            Color::rgb(10, 220, 40)
        );
        let (_, min_y, _, max_y) =
            frame_diff_bounds(&red, &green, width).expect("recolor changes pixels");
        assert!(
            min_y >= status_band_top(height, state.fm).floor() as u32 && max_y <= height,
            "the recolor stays inside the status band"
        );
    }

    #[test]
    fn built_in_only_overwide_readout_clips_left_and_keeps_its_right_tail_pinned() {
        let (narrow_width, wide_width, height) = (96, 500, 260);
        let Some(mut narrow) = headless_or_skip(narrow_width, height, "text") else {
            return;
        };
        let Some(mut wide) = headless_or_skip(wide_width, height, "text") else {
            return;
        };
        for state in [&mut narrow, &mut wide] {
            let buffer_id = BufferId::next();
            state.current_buffer_id = Some(buffer_id);
            state.status_facts = Some(status_facts(buffer_id, None));
            state.own_cursor = Some(OwnCursor { buffer_id, byte: 0 });
        }

        let narrow_frame = narrow.render_offscreen();
        let wide_frame = wide.render_offscreen();
        assert!(
            narrow.statusline_segments.is_none() && wide.statusline_segments.is_none(),
            "fixture must exercise the built-in-only legacy surface"
        );
        let narrow_status_width = narrow
            .status_buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max);
        let wide_status_width = wide
            .status_buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max);
        assert!(
            (narrow_status_width - wide_status_width).abs() < 0.01,
            "surface width must not reshape the no-wrap readout"
        );
        assert!(
            narrow_width as f32 - STATUS_TEXT_PAD - narrow_status_width < 0.0,
            "fixture must force the built-in readout past the left edge"
        );
        assert!(
            wide_width as f32 - STATUS_TEXT_PAD - wide_status_width > 0.0,
            "comparison surface must fit the complete built-in readout"
        );

        let band_top = status_band_top(height, narrow.fm).floor() as u32;
        let pinned_tail_width = 80;
        for y in band_top..height {
            for offset in 0..pinned_tail_width {
                assert_eq!(
                    px_at(&narrow_frame, narrow_width, narrow_width - 1 - offset, y),
                    px_at(&wide_frame, wide_width, wide_width - 1 - offset, y),
                    "built-in readout tail moved at right-edge offset {offset}, y={y}"
                );
            }
        }
    }

    #[test]
    fn overwide_status_runs_never_wrap_and_keep_the_suffix_pinned() {
        let (width, height) = (800, 300);
        for size in [600, 7200] {
            let Some(mut state) = headless_or_skip(width, height, "text") else {
                return;
            };
            let buffer_id = BufferId::next();
            state.current_buffer_id = Some(buffer_id);
            state.status_facts = Some(status_facts(buffer_id, None));
            state.own_cursor = Some(OwnCursor { buffer_id, byte: 0 });
            state.apply_font_facts(None, Some(size));
            let baseline = state.render_offscreen();
            let suffix_width = state
                .status_buffer
                .layout_runs()
                .map(|run| run.line_w)
                .fold(0.0_f32, f32::max);
            let suffix_left = (width as f32 - STATUS_TEXT_PAD - suffix_width)
                .max(0.0)
                .ceil() as u32;

            apply_statusline(
                &mut state,
                buffer_id,
                vec![statusline_segment(
                    "L".repeat(MAX_STATUSLINE_SEGMENT_BYTES),
                    "ui.modeline",
                )],
                vec![statusline_segment(
                    "R".repeat(MAX_STATUSLINE_SEGMENT_BYTES),
                    "ui.modeline",
                )],
            );
            let overwide = state.render_offscreen();
            let full_width = state
                .status_buffer
                .layout_runs()
                .map(|run| run.line_w)
                .fold(0.0_f32, f32::max);
            let actual_origin = width as f32 - STATUS_TEXT_PAD - full_width;
            assert!(actual_origin < 0.0, "fixture must cross the left edge");
            assert_eq!(state.status_buffer.wrap(), Wrap::None);
            assert_eq!(state.status_left_buffer.wrap(), Wrap::None);
            assert_eq!(state.status_buffer.layout_runs().count(), 1);
            assert_eq!(state.status_left_buffer.layout_runs().count(), 1);

            let band_top = status_band_top(height, state.fm).floor() as u32;
            for y in band_top..height {
                for x in suffix_left..width {
                    assert_eq!(
                        px_at(&overwide, width, x, y),
                        px_at(&baseline, width, x, y),
                        "protected suffix pixel moved at size {size}, ({x},{y})"
                    );
                }
            }
            let (_, min_y, _, max_y) =
                frame_diff_bounds(&baseline, &overwide, width).expect("custom text paints");
            assert!(min_y >= band_top && max_y <= height);
        }
    }

    #[test]
    fn headless_theme_facts_empty_table_renders_identically() {
        // Acceptance 20: the authoritative empty table (an unthemed
        // daemon's first send) must change nothing.
        let Some(mut state) = headless_or_skip(320, 240, "fn main() {}") else {
            return;
        };
        let base = state.render_offscreen();
        apply_faces(&mut state, Vec::new());
        let themed = state.render_offscreen();
        assert_eq!(base, themed, "an empty face table must be a no-op");
    }

    #[test]
    fn headless_modeline_face_owns_the_band_and_reverse_swaps() {
        // Acceptance 20: ui.modeline bg retints the band quad, and
        // reverse swaps quad/text after the Default mapping. Sampled
        // at a band pixel left of the text pad — comparing two states
        // whose (fg, bg, reverse) SHOULD produce the same quad avoids
        // any dependence on the surface's exact color space.
        let (w, h) = (400u32, 300u32);
        let fg = CellColor::Rgb(200, 30, 30);
        let bg = CellColor::Rgb(10, 60, 110);
        let Some(mut plain) = headless_or_skip(w, h, "hello") else {
            return;
        };
        let plain_sample = px_at(&plain.render_offscreen(), w, 2, h - 2);

        apply_faces(
            &mut plain,
            vec![theme_face(
                "ui.modeline",
                CellStyle {
                    fg,
                    bg,
                    ..CellStyle::default()
                },
            )],
        );
        let tinted_sample = px_at(&plain.render_offscreen(), w, 2, h - 2);
        assert_ne!(
            plain_sample, tinted_sample,
            "a bg face must retint the band quad"
        );

        // Swapped face + reverse ⇒ the identical quad color…
        apply_faces(
            &mut plain,
            vec![theme_face(
                "ui.modeline",
                CellStyle {
                    fg: bg,
                    bg: fg,
                    reverse: true,
                    ..CellStyle::default()
                },
            )],
        );
        let swapped_sample = px_at(&plain.render_offscreen(), w, 2, h - 2);
        assert_eq!(
            tinted_sample, swapped_sample,
            "reverse must swap fg/bg after mapping (same quad both ways)"
        );

        // …while reverse on the ORIGINAL face tints the quad with fg.
        apply_faces(
            &mut plain,
            vec![theme_face(
                "ui.modeline",
                CellStyle {
                    fg,
                    bg,
                    reverse: true,
                    ..CellStyle::default()
                },
            )],
        );
        let reversed_sample = px_at(&plain.render_offscreen(), w, 2, h - 2);
        assert_ne!(
            tinted_sample, reversed_sample,
            "reverse must move the quad off the bg color"
        );
    }

    #[test]
    fn headless_gutter_face_recolors_only_the_gutter_region() {
        // Acceptance 20: a ui.gutter fg change alters the gutter
        // digits and nothing else — every differing pixel lies left
        // of the text origin and above the band.
        let (w, h) = (400u32, 300u32);
        let text = "alpha\nbeta\ngamma\ndelta\n";
        let Some(mut state) = headless_or_skip(w, h, text) else {
            return;
        };
        state.line_numbers = LineNumberMode::Absolute;
        let text_left = state.text_left().ceil() as u32;
        let band_top =
            document_text_bottom(h, FontMetrics::default(), PanelBandInset::ABSENT).floor() as u32;
        let base = state.render_offscreen();
        apply_faces(
            &mut state,
            vec![theme_face(
                "ui.gutter",
                CellStyle {
                    fg: CellColor::Rgb(220, 120, 40),
                    ..CellStyle::default()
                },
            )],
        );
        let themed = state.render_offscreen();
        let mut differing = 0usize;
        for (i, (a, b)) in base.iter().zip(&themed).enumerate() {
            if a != b {
                differing += 1;
                let pixel = (i / 4) as u32;
                let (x, y) = (pixel % w, pixel / w);
                assert!(
                    x < text_left && y < band_top,
                    "gutter face leaked outside the gutter region at ({x}, {y})"
                );
            }
        }
        assert!(differing > 20, "the digits must actually recolor");
    }

    #[test]
    fn headless_band_text_faces_follow_content_class() {
        // Acceptance 20: with the composed strings held constant,
        // ui.statusline recolors a transient message, ui.minibuffer
        // recolors a live minibuffer and an isearch band — and each
        // face is a NO-OP on the content classes it doesn't own.
        let (w, h) = (400u32, 300u32);
        let Some(mut state) = headless_or_skip(w, h, "hello") else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        let statusline_face = vec![theme_face(
            "ui.statusline",
            CellStyle {
                fg: CellColor::Rgb(240, 140, 40),
                ..CellStyle::default()
            },
        )];
        let minibuffer_face = vec![theme_face(
            "ui.minibuffer",
            CellStyle {
                fg: CellColor::Rgb(40, 220, 140),
                ..CellStyle::default()
            },
        )];

        // (a) Buffer name showing: ui.statusline must not repaint it.
        state.status_facts = Some(StatusFactsLocal {
            buffer_id: bid,
            name: "main.rs".into(),
            modified: false,
            diag_errors: 0,
            diag_warnings: 0,
            message: None,
        });
        let name_base = state.render_offscreen();
        apply_faces(&mut state, statusline_face.clone());
        assert_eq!(
            name_base,
            state.render_offscreen(),
            "ui.statusline must not color the buffer-name class"
        );
        apply_faces(&mut state, Vec::new());

        // (b) Transient message showing: ui.statusline recolors it.
        state.status_facts = Some(StatusFactsLocal {
            buffer_id: bid,
            name: "main.rs".into(),
            modified: false,
            diag_errors: 0,
            diag_warnings: 0,
            message: Some("12 references".into()),
        });
        let msg_base = state.render_offscreen();
        apply_faces(&mut state, statusline_face);
        assert_ne!(
            msg_base,
            state.render_offscreen(),
            "ui.statusline must recolor the transient message"
        );
        apply_faces(&mut state, Vec::new());

        // (c) Live minibuffer: ui.minibuffer recolors the band text.
        state.minibuffer = Some(MinibufferLocal {
            prompt: "M-x ".into(),
            input: "theme".into(),
            cursor: 5,
            rows: Vec::new(),
            selected: None,
            total: 0,
        });
        let mb_base = state.render_offscreen();
        apply_faces(&mut state, minibuffer_face.clone());
        assert_ne!(
            mb_base,
            state.render_offscreen(),
            "ui.minibuffer must recolor the live minibuffer"
        );
        apply_faces(&mut state, Vec::new());
        state.minibuffer = None;

        // (d) Isearch band: same face, same route.
        state.search_prompt = Some(SearchPromptLocal {
            buffer_id: bid,
            query: "needle".into(),
            active: Some(0),
            total: 2,
            regex: false,
            invalid: false,
        });
        let isearch_base = state.render_offscreen();
        apply_faces(&mut state, minibuffer_face);
        assert_ne!(
            isearch_base,
            state.render_offscreen(),
            "ui.minibuffer must recolor the isearch band"
        );
    }

    #[test]
    fn headless_candidate_face_recolors_the_dropdown_glyphs() {
        // Acceptance 20 (round 3 finding 1): ui.minibuffer.candidate's
        // GPU site is the dropdown's candidate glyph layer.
        let (w, h) = (400u32, 300u32);
        let Some(mut state) = headless_or_skip(w, h, "hello") else {
            return;
        };
        state.minibuffer = Some(MinibufferLocal {
            prompt: "M-x ".into(),
            input: "the".into(),
            cursor: 3,
            rows: detailless_rows(["theme-set", "theme-clear"]),
            selected: Some(0),
            total: 2,
        });
        let base = state.render_offscreen();
        apply_faces(
            &mut state,
            vec![theme_face(
                "ui.minibuffer.candidate",
                CellStyle {
                    fg: CellColor::Rgb(250, 80, 160),
                    ..CellStyle::default()
                },
            )],
        );
        assert_ne!(
            base,
            state.render_offscreen(),
            "the candidate glyphs must recolor"
        );
    }

    /// Discovery Stage 2: a row's `detail` reaches the shaped dropdown
    /// line, and BOTH wire families land in the same local shape.
    ///
    /// Driven through `apply_attach_message` rather than by assigning
    /// `state.minibuffer` — the mapping from wire variant to local row
    /// is exactly what this asserts, so constructing the local value
    /// would skip the thing under test. The shaped `layout_runs()` text
    /// is what glyphon rasterizes, so a description present there is a
    /// description on screen.
    #[test]
    fn a_minibuffer_row_detail_reaches_the_shaped_dropdown_line() {
        let Some(mut state) = headless_or_skip(600, 400, "hello") else {
            return;
        };

        // The v23 rows form: a row with a detail, and a row without.
        let _ = state.apply_attach_message(InstanceMessage::MinibufferPromptRows {
            prompt: Some("M-x ".into()),
            input: "buf".into(),
            cursor: 3,
            rows: vec![
                MinibufferRow {
                    label: "buffer.save".into(),
                    detail: Some("Write the buffer to its file".into()),
                },
                MinibufferRow {
                    label: "buffer.kill".into(),
                    detail: None,
                },
            ],
            selected: Some(0),
            total: 2,
        });
        state.refresh_mb_buffer();
        let lines: Vec<String> = state
            .mb_buffer
            .layout_runs()
            .map(|run| run.text.to_owned())
            .collect();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("buffer.save") && l.contains("Write the buffer to its file")),
            "the detail must be shaped into the row: {lines:?}"
        );
        assert_eq!(
            lines
                .iter()
                .find(|l| l.contains("buffer.kill"))
                .map(String::as_str),
            Some("buffer.kill"),
            "a row with no detail renders the bare label, exactly as before v23: {lines:?}"
        );
        // The geometry invariant the dropdown depends on: it derives
        // its height, its visible window and its selection-highlight
        // offset from `rows.len()`, so ONE physical line per logical
        // row is what keeps those aligned. The daemon clips a detail to
        // its first line (`Command::description_first_line`) precisely
        // so this holds for an MCP schema block.
        assert_eq!(
            lines.len(),
            state.minibuffer.as_ref().map_or(0, |mb| mb.rows.len()),
            "one physical line per candidate row: {lines:?}"
        );

        // The frozen `12..=22` form, which an older daemon still sends:
        // bare strings become detail-free rows.
        let _ = state.apply_attach_message(InstanceMessage::MinibufferPrompt {
            prompt: Some("M-x ".into()),
            input: "buf".into(),
            cursor: 3,
            candidates: vec!["buffer.save".into()],
            selected: Some(0),
            total: 1,
        });
        assert_eq!(
            state.minibuffer.as_ref().map(|mb| mb.rows.clone()),
            Some(vec![MinibufferRow {
                label: "buffer.save".into(),
                detail: None,
            }]),
            "the legacy variant lands as a detail-free row"
        );
        state.refresh_mb_buffer();
        let legacy: Vec<String> = state
            .mb_buffer
            .layout_runs()
            .map(|run| run.text.to_owned())
            .collect();
        assert_eq!(legacy, vec!["buffer.save".to_owned()]);

        // Either family closes the surface with `prompt: None`.
        let _ = state.apply_attach_message(InstanceMessage::MinibufferPromptRows {
            prompt: None,
            input: String::new(),
            cursor: 0,
            rows: Vec::new(),
            selected: None,
            total: 0,
        });
        assert!(
            state.minibuffer.is_none(),
            "a rows clear closes the surface"
        );
    }

    #[test]
    fn headless_diag_face_recolors_band_counter_despite_unchanged_text() {
        // Acceptance 22 — the round-1 finding-3 bite. The E: counter
        // color is baked into shaped rich text and the composed string
        // is CONSTANT here, so this fails without the ThemeFacts arm's
        // shaping-cache invalidation (Q#TH8). The empty diag child is
        // the built-in reset (Q#TH5): identical to unthemed.
        let (w, h) = (400u32, 300u32);
        let Some(mut state) = headless_or_skip(w, h, "alpha\nbeta\n") else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.view_range = (0, state.current_text.len() as u64);
        state.status_facts = Some(StatusFactsLocal {
            buffer_id: bid,
            name: "main.rs".into(),
            modified: false,
            diag_errors: 2,
            diag_warnings: 0,
            message: None,
        });
        state.current_decorations.push(Decoration {
            range: ByteRange { start: 0, end: 5 },
            kind: DecorationKind::DiagnosticError,
        });
        let base = state.render_offscreen();

        apply_faces(
            &mut state,
            vec![theme_face(
                "ui.diag.error",
                CellStyle {
                    fg: CellColor::Rgb(40, 200, 255),
                    ..CellStyle::default()
                },
            )],
        );
        assert_ne!(
            base,
            state.render_offscreen(),
            "the counter (and squiggle) must recolor with counts constant"
        );

        // An all-default diag child resets to the built-in color.
        apply_faces(
            &mut state,
            vec![theme_face("ui.diag.error", CellStyle::default())],
        );
        assert_eq!(
            base,
            state.render_offscreen(),
            "ui.diag.error = {{}} must render as the built-in severity color"
        );
    }

    #[test]
    fn headless_same_generation_summary_update_repaints_the_minimap() {
        // PR #120 round 1 finding 1: theme recolors and diagnostic
        // republishes ship a NEW summary at the SAME CRDT generation,
        // and the minimap cache keys only on (generation, dims,
        // scroll) — accepting a summary must drop the cached vertices
        // or the stale strokes survive until an edit/resize/scroll.
        let text = "alpha\nbeta\ngamma\ndelta\n";
        let (w, h) = (400u32, 300u32);
        let Some(mut state) = headless_or_skip(w, h, text) else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.view_range = (0, text.len() as u64);
        let summary = |color: CellColor| -> Vec<CellStyle> {
            (0..4)
                .map(|_| CellStyle {
                    fg: color,
                    ..CellStyle::default()
                })
                .collect()
        };
        let _ = state.apply_attach_message(InstanceMessage::FileStyleSummary {
            buffer_id: bid,
            generation: 7,
            lines: summary(CellColor::Rgb(200, 40, 40)),
        });
        let first = state.render_offscreen();
        // Same generation, different colors — a theme-recolor twin.
        let _ = state.apply_attach_message(InstanceMessage::FileStyleSummary {
            buffer_id: bid,
            generation: 7,
            lines: summary(CellColor::Rgb(40, 200, 40)),
        });
        let second = state.render_offscreen();
        assert_ne!(
            first, second,
            "a same-generation summary recolor must repaint the minimap"
        );
    }

    #[test]
    fn headless_snapshot_round_trip_summary_restores_the_minimap() {
        // PR #120 round 2 finding 1 (frontend half): a `BufferSnapshot`
        // drops `current_summary` with the rest of the buffer-scoped
        // state, so after an A → B → A round trip no stale minimap
        // may survive — and when the daemon's baseline reset re-ships
        // the summary at the unchanged generation, the first visit's
        // pixels must return exactly.
        let text = "alpha\nbeta\ngamma\ndelta\n";
        let (w, h) = (400u32, 300u32);
        let Some(mut state) = headless_or_skip(w, h, text) else {
            return;
        };
        // Both buffers carry the same text so the frames differ by
        // the minimap alone.
        let snapshot = || -> Vec<u8> {
            let doc = loro::LoroDoc::new();
            doc.get_text(LORO_TEXT_CONTAINER)
                .insert(0, text)
                .expect("insert snapshot text");
            doc.export(loro::ExportMode::Snapshot)
                .expect("export snapshot")
        };
        let bid_a = BufferId::next();
        let bid_b = BufferId::next();
        let visit = |state: &mut State, bid: BufferId| {
            let _ = state.apply_attach_message(InstanceMessage::BufferSnapshot {
                buffer_id: bid,
                crdt_snapshot: snapshot(),
            });
            state.view_range = (0, text.len() as u64);
        };
        let lines: Vec<CellStyle> = (0..4)
            .map(|_| CellStyle {
                fg: CellColor::Rgb(200, 40, 40),
                ..CellStyle::default()
            })
            .collect();

        visit(&mut state, bid_a);
        let _ = state.apply_attach_message(InstanceMessage::FileStyleSummary {
            buffer_id: bid_a,
            generation: 7,
            lines: lines.clone(),
        });
        let first_visit = state.render_offscreen();

        visit(&mut state, bid_b);
        visit(&mut state, bid_a);
        assert_ne!(
            first_visit,
            state.render_offscreen(),
            "the round trip dropped the summary — no stale minimap"
        );

        let _ = state.apply_attach_message(InstanceMessage::FileStyleSummary {
            buffer_id: bid_a,
            generation: 7,
            lines,
        });
        assert_eq!(
            first_visit,
            state.render_offscreen(),
            "the re-shipped summary restores the first visit's minimap"
        );
    }

    #[test]
    fn headless_snapshot_clears_search_menu_status_and_the_intercept_gate() {
        // PR #120 round 3 finding 1: search/menu popups anchor in the
        // prior buffer AND gate key/pointer interception, and the new
        // buffer's first CLOSED state is suppressed daemon-side, so
        // no close message ever comes — a `BufferSnapshot` must clear
        // them (and the stale status band) or the popup hijacks input
        // forever. The minibuffer is global and deliberately exempt.
        let text = "alpha\nbeta\n";
        let Some(mut state) = headless_or_skip(400, 300, text) else {
            return;
        };
        let bid_a = BufferId::next();
        state.current_buffer_id = Some(bid_a);
        let _ = state.apply_attach_message(InstanceMessage::DispatchIdle { idle: true });
        assert!(
            !state.daemon_intercepts_keys(),
            "idle with nothing open: keys apply locally"
        );

        let _ = state.apply_attach_message(InstanceMessage::SearchPrompt {
            buffer_id: bid_a,
            query: Some("al".into()),
            active: Some(0),
            total: 1,
            regex: false,
            invalid: false,
        });
        let _ = state.apply_attach_message(InstanceMessage::MenuPrompt {
            buffer_id: bid_a,
            rows: vec![MenuPromptRow {
                label: "Cut".into(),
                separator: false,
            }],
            active: Some(0),
        });
        let _ = state.apply_attach_message(InstanceMessage::StatusFacts {
            buffer_id: bid_a,
            name: "old.rs".into(),
            modified: false,
            diag_errors: 3,
            diag_warnings: 1,
            message: None,
        });
        assert!(
            state.daemon_intercepts_keys(),
            "an open search/menu round-trips every key"
        );
        let with_popups = state.render_offscreen();

        let doc = loro::LoroDoc::new();
        doc.get_text(LORO_TEXT_CONTAINER)
            .insert(0, text)
            .expect("insert snapshot text");
        let _ = state.apply_attach_message(InstanceMessage::BufferSnapshot {
            buffer_id: BufferId::next(),
            crdt_snapshot: doc.export(loro::ExportMode::Snapshot).expect("export"),
        });

        assert!(
            state.search_prompt.is_none() && state.menu.is_none() && state.status_facts.is_none(),
            "buffer-scoped search/menu/status facts die with the snapshot"
        );
        assert!(
            !state.daemon_intercepts_keys(),
            "the intercept gate releases — keys apply locally again"
        );
        assert_ne!(
            with_popups,
            state.render_offscreen(),
            "the popup pixels are gone"
        );
    }

    #[test]
    fn own_wash_faces_color_local_rects_peers_keep_the_constant() {
        // Acceptance 21 + 23: decode the emitted decoration vertex
        // colors — the LOCAL selection and both search washes resolve
        // their faces (site alpha preserved), while simultaneous PEER
        // selection/current-line rects keep the hardcoded constants.
        fn decode_quad_colors(bytes: &[u8]) -> Vec<[f32; 4]> {
            bytes
                .chunks_exact(24)
                .map(|v| {
                    let f = |i: usize| {
                        f32::from_ne_bytes(v[i * 4..i * 4 + 4].try_into().expect("4 bytes"))
                    };
                    [f(2), f(3), f(4), f(5)]
                })
                .collect()
        }
        let text = "alpha\nbeta\ngamma\ndelta\n";
        let Some(mut state) = headless_or_skip(400, 300, text) else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.view_range = (0, text.len() as u64);
        state.current_decorations.extend([
            Decoration {
                range: ByteRange { start: 0, end: 5 }, // "alpha"
                kind: DecorationKind::Selection,
            },
            Decoration {
                range: ByteRange { start: 6, end: 10 }, // "beta"
                kind: DecorationKind::SearchMatch,
            },
            Decoration {
                range: ByteRange { start: 11, end: 16 }, // "gamma"
                kind: DecorationKind::SearchMatchActive,
            },
        ]);
        state.peer_presences.insert(
            FrontendId(7),
            PeerPresence {
                buffer_id: bid,
                cursor: 17,
                selection: Some(SelectionSnapshot {
                    anchor: 17,
                    active: 22, // "delta"
                }),
            },
        );
        apply_faces(
            &mut state,
            vec![
                theme_face(
                    "ui.selection",
                    CellStyle {
                        bg: CellColor::Rgb(9, 99, 199),
                        ..CellStyle::default()
                    },
                ),
                theme_face(
                    "ui.search.match",
                    CellStyle {
                        bg: CellColor::Rgb(10, 20, 30),
                        ..CellStyle::default()
                    },
                ),
                theme_face(
                    "ui.search.match.active",
                    CellStyle {
                        bg: CellColor::Rgb(40, 50, 60),
                        ..CellStyle::default()
                    },
                ),
            ],
        );
        let colors = decode_quad_colors(&state.decoration_background_vertex_bytes());
        // Exact float equality is deliberate: both sides derive from
        // the same constants through the same arithmetic.
        let has = |c: [f32; 4]| colors.contains(&c);
        // Local rects: face RGB with each site's original alpha.
        assert!(
            has(glyphon_to_rgba(glyphon::Color::rgb(9, 99, 199), 0.30)),
            "local selection must use ui.selection"
        );
        assert!(
            has(glyphon_to_rgba(glyphon::Color::rgb(10, 20, 30), 0.30)),
            "search match must use ui.search.match"
        );
        assert!(
            has(glyphon_to_rgba(glyphon::Color::rgb(40, 50, 60), 0.48)),
            "active match must use ui.search.match.active"
        );
        // Peer rects: the hardcoded constants, face table ignored.
        assert!(
            has([0.31, 0.42, 0.82, 0.30]),
            "the peer selection must keep the Selection constant"
        );
        assert!(
            has([0.55, 0.60, 0.75, 0.22]),
            "the peer cursor line must keep the CurrentLine constant"
        );
    }

    #[test]
    fn source_color_folds_overlapping_child_over_parent() {
        // Framing acceptance #9 (GPU consumer): a styled markdown parent
        // span [0,10) (red) with an injected child span [3,6) (green) on top,
        // driven through the ACTUAL `StyleSpans { full: true }` transform
        // (`spans_from_segments`, the body of `replace_style_spans`) rather
        // than a hand-rolled sort. `source_color_at` must FOLD the covering
        // spans (child green wins at a shared byte), not return the first
        // (parent) span. Bite: the pre-fix first-covering-span code returns
        // red at byte 4.
        let red = style_with_fg(CellColor::Rgb(200, 0, 0));
        let green = style_with_fg(CellColor::Rgb(0, 200, 0));
        let segments = vec![StyleSegment {
            range: ByteRange { start: 0, end: 10 },
            spans: vec![
                StyleSpan {
                    range: ByteRange { start: 0, end: 10 },
                    style: red,
                },
                StyleSpan {
                    range: ByteRange { start: 3, end: 6 },
                    style: green,
                },
            ],
        }];
        let spans = spans_from_segments(segments); // full-frame message application

        // Byte 4 is covered by both: the child (green) wins the fold.
        assert_eq!(
            source_color_at(4, &spans),
            Some(glyphon::Color::rgb(0, 200, 0)),
            "the injected child color wins over the parent at a shared byte"
        );
        // Byte 1 is parent-only: stays red.
        assert_eq!(
            source_color_at(1, &spans),
            Some(glyphon::Color::rgb(200, 0, 0)),
            "a parent-only byte keeps the parent color"
        );
    }

    // -----------------------------------------------------------------
    // Arc 4 stage 2 (gpu-set-font framing Q#F6/Q#F7): apply_font_facts
    // and the visual-run caret/scroll substrate. Family routing is
    // hermetic: the four generated fixture faces enter the explicit
    // database before FontSystem construction (fonts/test/,
    // provenance in LICENSE.txt there).
    // -----------------------------------------------------------------

    fn font_facts(family: Option<&str>, size_centi_px: Option<u32>) -> InstanceMessage {
        InstanceMessage::FontFacts {
            family: family.map(str::to_owned),
            size_centi_px,
        }
    }

    #[test]
    fn fixture_monospaces_are_registered_in_font_system_at_construction() {
        let Some(state) = headless_or_skip(320, 240, "text") else {
            return;
        };
        let mono_id = query_normal_face(state.font_system.db(), "Pmacs Test Mono Two")
            .expect("fixture face is present in fontdb");
        let proportional_id = query_normal_face(state.font_system.db(), "Pmacs Test Proportional")
            .expect("proportional fixture is present");
        let regular_id = query_normal_face(state.font_system.db(), "Pmacs Test Family")
            .expect("regular fixture is present");
        let bold_id = state
            .font_system
            .db()
            .query(&fontdb::Query {
                families: &[fontdb::Family::Name("Pmacs Test Family")],
                weight: fontdb::Weight::BOLD,
                stretch: fontdb::Stretch::Normal,
                style: fontdb::Style::Normal,
            })
            .expect("bold fixture is present");
        for id in [mono_id, proportional_id, regular_id, bold_id] {
            assert!(
                state.font_defaults.extra_ids.contains(&id),
                "the fixture's load-time ID is retained"
            );
        }
        assert!(
            state
                .font_system
                .is_monospace(state.font_defaults.bundled_id),
            "the bundled ID is registered too"
        );
        assert!(
            state.font_system.is_monospace(mono_id),
            "the alternate monospace ID is registered at construction"
        );
        assert!(
            state.font_system.is_monospace(regular_id),
            "the regular test-family ID is registered at construction"
        );
        assert!(
            !state.font_system.is_monospace(proportional_id)
                && !state.font_system.is_monospace(bold_id),
            "proportional fixture IDs stay outside the monospace registry"
        );
    }

    /// Acceptance 16 — wire validation fails closed: an out-of-range
    /// size rejects the WHOLE message (family included), keeps every
    /// piece of current state, and re-declares nothing.
    #[test]
    #[allow(clippy::float_cmp)] // exact: assigned constants, not computed sums
    fn font_facts_out_of_range_sizes_fail_closed() {
        let Some(mut state) = headless_or_skip(320, 240, "fn main() {}") else {
            return;
        };
        let baseline = state.render_offscreen();
        for size in [0u32, 599, 7201, u32::MAX] {
            let vp =
                state.apply_attach_message(font_facts(Some("Pmacs Test Mono Two"), Some(size)));
            assert!(vp.is_none(), "a rejected FontFacts must not re-declare");
            assert_eq!(state.fm.scale, 1.0, "scale untouched at size {size}");
            assert_eq!(
                state.resolved_family, DEFAULT_FONT_FAMILY,
                "family untouched at size {size} — the whole message is rejected"
            );
            assert_eq!(
                state.render_offscreen(),
                baseline,
                "frame byte-identical at size {size}"
            );
        }
    }

    /// Acceptance 9 — the size route re-derives every metric, and the
    /// `(None, None)` reset reproduces the never-set frame
    /// byte-for-byte within the process.
    #[test]
    #[allow(clippy::float_cmp)] // exact: assigned constants, not computed sums
    fn font_size_applies_and_reset_is_byte_identical() {
        let Some(mut state) = headless_or_skip(320, 240, "fn main() {}") else {
            return;
        };
        let never_set = state.render_offscreen();
        let advance_before = state.mono_advance();
        let visible_before =
            estimated_visible_lines(state.config.height, state.fm, state.band_inset());
        state.apply_attach_message(font_facts(None, Some(2400)));
        assert!(
            (state.fm.scale - 1.5).abs() < f32::EPSILON,
            "2400 centi-px / 16 = 1.5"
        );
        assert!(
            state.mono_advance() > advance_before,
            "a larger size widens the measured advance (gutter geometry)"
        );
        assert!(
            estimated_visible_lines(state.config.height, state.fm, state.band_inset())
                < visible_before,
            "a larger size fits fewer source lines"
        );
        let big = state.render_offscreen();
        assert_ne!(big, never_set, "a 24px preference must change the ink");
        state.apply_attach_message(font_facts(None, None));
        assert_eq!(state.fm.scale, 1.0);
        assert_eq!(state.fm.advance_ratio, 1.0);
        assert_eq!(
            state.render_offscreen(),
            never_set,
            "the (None, None) reset must reproduce the never-set frame byte-for-byte"
        );
    }

    /// Acceptance 17 — metrics and REAL drawable dimensions change
    /// atomically on all seven buffers, and `resize()` keeps every
    /// buffer in step through the same helper (`Buffer::size()` is
    /// the witness).
    #[test]
    fn all_seven_buffers_track_metrics_and_dimensions() {
        fn assert_buffer_dims(state: &State) {
            let fm = state.fm;
            let width = state.config.width as f32;
            let height = state.config.height as f32;
            let code_metrics = Metrics::new(fm.code_font_size(), fm.code_line_height());
            let code_width = (state.text_bounds_right() as f32 - state.text_left()).max(0.0);
            let code_height = (document_text_bottom(state.config.height, fm, state.band_inset())
                - TEXT_TOP)
                .max(0.0);
            assert_eq!(state.buffer.metrics(), code_metrics);
            assert_eq!(state.buffer.size(), (Some(code_width), Some(code_height)));
            assert_eq!(state.gutter_buffer.metrics(), code_metrics);
            assert_eq!(state.gutter_buffer.size(), (Some(width), Some(code_height)));
            let status_metrics = Metrics::new(fm.status_font_size(), fm.status_line_height());
            for buffer in [&state.status_buffer, &state.status_left_buffer] {
                assert_eq!(buffer.metrics(), status_metrics);
                assert_eq!(buffer.size(), (Some(width), Some(fm.status_band_height())));
            }
            assert_eq!(
                state.menu_buffer.metrics(),
                Metrics::new(fm.menu_font_size(), fm.menu_line_height())
            );
            assert_eq!(
                state.menu_buffer.size(),
                (Some(MENU_MAX_WIDTH), Some(height))
            );
            let drop_metrics = Metrics::new(fm.mb_drop_font_size(), fm.mb_drop_line_height());
            for buffer in [&state.mb_buffer, &state.completion_buffer] {
                assert_eq!(buffer.metrics(), drop_metrics);
                assert_eq!(buffer.size(), (Some(MB_DROP_MAX_WIDTH), Some(height)));
            }
        }
        let Some(mut state) = headless_or_skip(320, 240, "one\ntwo\nthree\n") else {
            return;
        };
        state.apply_attach_message(font_facts(None, Some(2400)));
        assert_buffer_dims(&state);
        state.resize(500, 400);
        assert_buffer_dims(&state);
    }

    /// Acceptance 18 — rows stay rows at both preference bounds: no
    /// popup label may wrap onto a second visual run, and selection /
    /// hit geometry names the same semantic row.
    #[test]
    fn popup_rows_never_wrap_after_a_font_change() {
        for size in [600, 7200] {
            let Some(mut state) = headless_or_skip(320, 600, "text") else {
                return;
            };
            state.apply_attach_message(font_facts(None, Some(size)));
            let bid = BufferId::next();
            state.current_buffer_id = Some(bid);
            state.view_range = (0, state.current_text.len() as u64);
            let long = "a very long label ".repeat(8);
            state.completion = Some(CompletionLocal {
                buffer_id: bid,
                anchor: 0,
                prefix_len: 0,
                rows: vec![
                    CompletionPopupRow {
                        label: long.clone(),
                        kind: 3,
                        detail: Some(long.clone()),
                    },
                    CompletionPopupRow {
                        label: long.clone(),
                        kind: 3,
                        detail: None,
                    },
                ],
                selected: Some(1),
                total: 2,
            });
            state.refresh_completion_buffer();
            state.minibuffer = Some(MinibufferLocal {
                prompt: "P: ".into(),
                input: String::new(),
                cursor: 0,
                rows: detailless_rows([long.clone(), long.clone()]),
                selected: Some(1),
                total: 2,
            });
            state.refresh_mb_buffer();
            state.menu = Some(MenuLocal {
                rows: vec![
                    MenuPromptRow {
                        label: long.clone(),
                        separator: false,
                    },
                    MenuPromptRow {
                        label: long,
                        separator: false,
                    },
                ],
                active: Some(1),
                anchor_px: (10.0, 10.0),
            });
            state.refresh_menu_buffer();
            for (name, buffer) in [
                ("completion", &state.completion_buffer),
                ("minibuffer dropdown", &state.mb_buffer),
                ("context menu", &state.menu_buffer),
            ] {
                assert_eq!(
                    buffer.wrap(),
                    Wrap::None,
                    "{name} buffer must not wrap at {size}"
                );
                let mut seen = std::collections::HashSet::new();
                for run in buffer.layout_runs() {
                    assert!(
                        seen.insert(run.line_i),
                        "{name} row {} wrapped onto a second visual run at {size}",
                        run.line_i
                    );
                }
            }

            let (first, count) = state.mb_visible_window().expect("minibuffer rows fit");
            assert!(first <= 1 && 1 < first + count);
            assert_eq!(
                state.mb_dropdown_vertex_bytes().len(),
                2 * 6 * QUAD_VERTEX_STRIDE as usize,
                "minibuffer background + selected semantic row at {size}"
            );
            let (first, count, _, _) = state
                .completion_dropdown_layout()
                .expect("completion rows fit");
            assert!(first <= 1 && 1 < first + count);
            assert_eq!(
                state.completion_dropdown_vertex_bytes().len(),
                2 * 6 * QUAD_VERTEX_STRIDE as usize,
                "completion background + selected semantic row at {size}"
            );
            let row_center = 10.0 + 1.5 * f64::from(state.fm.menu_row_height());
            assert_eq!(
                state.menu_hit(15.0, row_center),
                Some((1, true)),
                "menu hit-testing must agree with painted row 1 at {size}"
            );
            assert_eq!(
                state.menu_vertex_bytes().len(),
                2 * 6 * QUAD_VERTEX_STRIDE as usize,
                "menu background + selected semantic row at {size}"
            );
        }
    }

    /// Acceptance 13 — the rich-run status caches drop on a font
    /// change, so unchanged content re-shapes with new attrs.
    #[test]
    #[allow(clippy::float_cmp)] // exact: assigned constants, not computed sums
    fn font_change_invalidates_the_status_shaping_caches() {
        let Some(mut state) = headless_or_skip(320, 240, "text") else {
            return;
        };
        let _ = state.render_offscreen();
        let right_before = state.status_runs.clone().expect("right cache shaped");
        let left_before = state
            .status_left_runs
            .clone()
            .expect("left cache shaped, including empty content");
        assert!(!right_before.is_empty(), "the readout is always present");

        state.apply_attach_message(font_facts(None, Some(3200)));
        assert!(state.status_runs.is_none());
        assert!(state.status_left_runs.is_none());

        let _ = state.render_offscreen();
        assert_eq!(
            state.status_buffer.metrics().font_size,
            state.fm.status_font_size(),
            "the re-shaped band must carry the derived metrics"
        );
        assert_eq!(state.status_runs.as_ref(), Some(&right_before));
        assert_eq!(state.status_left_runs.as_ref(), Some(&left_before));
    }

    /// Acceptance 14 — a size that shrinks the visible line count
    /// re-declares the scoped viewport from the final normalized
    /// origin.
    #[test]
    fn font_change_redeclares_a_shrunken_viewport() {
        let text = "line of text\n".repeat(40);
        let Some(mut state) = headless_or_skip(320, 400, &text) else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.reshape();
        let declared = state.view_range;
        state.last_viewport_sent = Some(declared);
        let vp = state
            .apply_attach_message(font_facts(None, Some(7200)))
            .expect("fewer lines fit at 72px — the viewport must be re-declared");
        assert_eq!(vp.buffer_id, bid);
        assert!(
            vp.visible.end < declared.1,
            "the re-declared range must shrink ({} vs {})",
            vp.visible.end,
            declared.1
        );
    }

    /// Acceptance 10 — both preference bounds keep code ink above the
    /// derived band, band ink inside it, and the two owned dropdown
    /// windows inside the surface. The context menu deliberately keeps
    /// its clipped raw-anchor policy.
    #[test]
    #[allow(clippy::too_many_lines)] // one end-to-end assertion block per rendered surface
    fn extreme_sizes_render_with_contained_popups() {
        const WIDTH: u32 = 320;
        const HEIGHT: u32 = 400;
        for size in [600, 7200] {
            let ink = "MMMMMMMM\n".repeat(20);
            let blank = "        \n".repeat(20);
            let Some(mut state) = headless_or_skip(WIDTH, HEIGHT, &ink) else {
                return;
            };
            state.apply_attach_message(font_facts(None, Some(size)));
            let bid = BufferId::next();
            state.current_buffer_id = Some(bid);
            let ink_frame = state.render_offscreen();
            assert!(state.set_text(&blank));
            let blank_frame = state.render_offscreen();
            let (_, _, _, code_max_y) =
                frame_diff_bounds(&ink_frame, &blank_frame, WIDTH).expect("code ink differs");
            let band_top = document_text_bottom(HEIGHT, state.fm, state.band_inset());
            assert!(
                code_max_y <= band_top.ceil() as u32,
                "code glyphs cross the derived band at {size}: {code_max_y} > {band_top}"
            );

            state.status_facts = Some(StatusFactsLocal {
                buffer_id: bid,
                name: "font-bounds".into(),
                modified: false,
                diag_errors: 0,
                diag_warnings: 0,
                message: Some("STATUS".into()),
            });
            let status_frame = state.render_offscreen();
            let (_, status_min_y, _, status_max_y) =
                frame_diff_bounds(&blank_frame, &status_frame, WIDTH).expect("status ink differs");
            assert!(
                status_min_y >= band_top.floor() as u32 && status_max_y <= HEIGHT,
                "status glyphs escape the derived band at {size}: \
                 {status_min_y}..{status_max_y}, band starts {band_top}"
            );

            state.view_range = (0, state.current_text.len() as u64);
            state.completion = Some(CompletionLocal {
                buffer_id: bid,
                anchor: 0,
                prefix_len: 0,
                rows: (0..8)
                    .map(|i| CompletionPopupRow {
                        label: format!("completion-{i}"),
                        kind: 3,
                        detail: None,
                    })
                    .collect(),
                selected: Some(1),
                total: 8,
            });
            state.refresh_completion_buffer();
            let (first, count, _, _) = state
                .completion_dropdown_layout()
                .expect("at least one completion row fits");
            let (left, top, width) = state
                .completion_dropdown_rect()
                .expect("completion rectangle");
            assert!(first <= 1 && 1 < first + count);
            assert!(
                top >= 0.0 && top + count as f32 * state.fm.mb_drop_row_height() <= HEIGHT as f32,
                "completion dropdown must be vertically surface-contained at {size}"
            );
            let completion_frame = state.render_offscreen();
            let (min_x, min_y, max_x, max_y) =
                frame_diff_bounds(&status_frame, &completion_frame, WIDTH)
                    .expect("completion pixels differ");
            assert!(
                min_x + 1 >= left.floor() as u32
                    && min_y + 1 >= top.floor() as u32
                    && max_x <= (left + width).ceil() as u32 + 1
                    && max_y
                        <= (top + count as f32 * state.fm.mb_drop_row_height()).ceil() as u32 + 1,
                "completion pixels escape their rectangle at {size}: \
                 {min_x},{min_y}..{max_x},{max_y}"
            );

            state.completion = None;
            state.minibuffer = Some(MinibufferLocal {
                prompt: "M-x ".into(),
                input: String::new(),
                cursor: 0,
                rows: detailless_rows((0..30).map(|i| format!("candidate-{i}"))),
                selected: Some(1),
                total: 30,
            });
            state.refresh_mb_buffer();
            let (first, count) = state
                .mb_visible_window()
                .expect("at least one minibuffer row fits");
            let (_left, top, _width) = state.mb_dropdown_rect().expect("minibuffer rectangle");
            assert!(first <= 1 && 1 < first + count);
            assert!(
                top >= 0.0 && top + count as f32 * state.fm.mb_drop_row_height() <= HEIGHT as f32,
                "minibuffer dropdown must be vertically surface-contained at {size}"
            );
            let px = state.render_offscreen();
            assert_eq!(px.len(), (WIDTH * HEIGHT * 4) as usize);
        }

        let Some(mut state) = headless_or_skip(WIDTH, HEIGHT, "text") else {
            return;
        };
        state.apply_attach_message(font_facts(None, Some(7200)));
        state.menu = Some(MenuLocal {
            rows: (0..20)
                .map(|i| MenuPromptRow {
                    label: format!("menu entry {i} with a fairly long label attached"),
                    separator: false,
                })
                .collect(),
            active: Some(0),
            anchor_px: (200.0, 300.0),
        });
        let px = state.render_offscreen();
        assert_eq!(px.len(), (WIDTH * HEIGHT * 4) as usize);
        assert_eq!(
            state.menu_hit(205.0, 305.0),
            Some((0, true)),
            "hit geometry stays coherent on the deliberately clipped popup"
        );
    }

    /// Acceptance 11 — a caret painted on a wrapped visual run
    /// survives 16px → 72px → 6px re-wraps, with the normalized
    /// A fresh frontend must already be CHARACTER wrapping, not word
    /// wrapping.
    ///
    /// The subtle failure this catches: `apply_line_wrap` short-circuits
    /// when the request matches `code_wrap`, so if the field said
    /// `Glyph` while the buffer was still on cosmic-text's inherited
    /// `WordOrGlyph`, the first `wrap: true` message would be a no-op
    /// and the document would keep **word** wrapping — the exact
    /// divergence from the grid renderer that framing Q#LL5 exists to
    /// close, surviving invisibly.
    ///
    /// Wrap-versus-truncate cannot see it, because both modes differ
    /// from each other either way. Word-versus-character can: with
    /// character wrap, spaces are just glyphs, so a spaced line and an
    /// unspaced line of the same length occupy the same number of rows.
    /// Word wrap breaks early at the spaces and needs more.
    #[test]
    fn a_fresh_frontend_wraps_by_character_not_by_word() {
        let Some(state) = headless_or_skip(320, 400, "hello world\n") else {
            return;
        };
        assert_eq!(
            state.buffer.wrap(),
            Wrap::Glyph,
            "a frontend told nothing must already be in the DEFAULT mode \
             and wrapping by CHARACTER. Inheriting cosmic-text's \
             WordOrGlyph would word-wrap the document forever against a \
             pre-v22 daemon, diverging from the grid renderer"
        );
    }

    /// The discriminating witness framing §7 asked for.
    ///
    /// `wrapped_caret_survives_size_changes` passes today against a wrap
    /// nobody configured — cosmic-text's constructor default — so it
    /// cannot tell "honors the setting" from "the default happened to
    /// match". The other value is what discriminates: with wrap OFF, an
    /// overlong line must NOT occupy a second row.
    #[test]
    fn the_gpu_honors_an_explicit_non_wrap_mode() {
        let long = "x".repeat(400);
        let Some(mut state) = headless_or_skip(320, 400, &format!("{long}\nsecond\n")) else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);

        state.apply_line_wrap(bid, true);
        state.reshape();
        let wrapped_rows = state.buffer.layout_runs().count();

        state.apply_line_wrap(bid, false);
        state.reshape();
        let truncated_rows = state.buffer.layout_runs().count();

        assert!(
            wrapped_rows > truncated_rows,
            "wrap must produce more visual rows than truncate \
             (wrapped={wrapped_rows}, truncated={truncated_rows}); equal counts \
             would mean the mode reached nothing"
        );
    }

    /// The scroll indicator token: the last builtin in the right-hand
    /// status runs, which are joined by a two-space separator.
    fn scroll_readout(state: &mut State) -> String {
        let runs = state.compose_status_runs();
        let text: String = runs.iter().map(|(text, _)| text.as_str()).collect();
        text.rsplit("  ").next().unwrap_or_default().to_owned()
    }

    /// Put `state` in a wrapped, scrolled-to-top state over `text`.
    fn wrapped_at_top(state: &mut State, wrap: bool) {
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.apply_line_wrap(bid, wrap);
        state.scroll_top = 0;
        state.code_scroll_residual = 0.0;
        state.reshape();
    }

    /// The defect the shared classifier exists for, in its purest form:
    /// **one** source line, wrapping to far more rows than fit.
    ///
    /// `format_scroll_indicator` returns "All" on its very first branch
    /// (`total_lines <= 1`) without consulting anything else — so the
    /// mode line claimed the whole buffer was on screen while most of it
    /// sat below the window. The truncate control below shows "All" is
    /// the right answer *in line space*; it is the space that is wrong.
    #[test]
    fn a_wrapped_single_line_is_not_all() {
        let long = "x".repeat(4000);
        let Some(mut state) = headless_or_skip(320, 240, &long) else {
            return;
        };
        wrapped_at_top(&mut state, true);
        assert_eq!(
            scroll_readout(&mut state),
            "Top",
            "one source line wrapping past the window bottom: the first \
             row is on screen and the last is not"
        );

        wrapped_at_top(&mut state, false);
        assert_eq!(
            scroll_readout(&mut state),
            "All",
            "control: truncating, this really is one row and all of it \
             is on screen — the line formula is right when it applies"
        );
    }

    /// `Bot` must mean the last byte is painted, not that the shaped
    /// slice happens to reach EOF.
    ///
    /// This is the discriminating case for [`State::code_byte_painted`]
    /// over the cheaper `view_range.1 == len`: few source lines, each
    /// wrapping to many rows, so `SCROLL_OVERSCAN` pulls EOF into the
    /// slice while EOF is nowhere near the screen.
    #[test]
    fn a_slice_that_reaches_eof_is_not_yet_bot() {
        let long = "y".repeat(1200);
        let text = format!("{long}\n{long}\n{long}\n");
        let Some(mut state) = headless_or_skip(320, 240, &text) else {
            return;
        };
        wrapped_at_top(&mut state, true);
        assert_eq!(
            state.view_range.1,
            state.current_text.len() as u64,
            "precondition: the shaped slice does reach EOF — otherwise \
             this test would pass for the wrong reason"
        );
        assert_eq!(
            scroll_readout(&mut state),
            "Top",
            "EOF is shaped but painted far below the band"
        );

        while state.scroll_by_lines(1).is_some() {}
        assert_eq!(
            scroll_readout(&mut state),
            "Bot",
            "scrolled to the last source line, EOF is now painted"
        );
    }

    /// A file ending in a newline has a final **empty** line, and a
    /// viewport parked on it has `view_range == (len, len)`.
    ///
    /// Named separately because the first version of
    /// [`State::code_byte_painted`] rejected an empty range outright —
    /// borrowed from the caret path, where it means "nothing shaped".
    /// Here it means "one empty row", and rejecting it reported a
    /// percentage at the precise moment the user reached the bottom.
    #[test]
    fn an_empty_final_line_still_counts_as_bot() {
        let Some(mut state) = headless_or_skip(320, 240, "alpha\nbeta\n") else {
            return;
        };
        wrapped_at_top(&mut state, true);
        while state.scroll_by_lines(1).is_some() {}
        assert_eq!(
            state.view_range.0, state.view_range.1,
            "precondition: parked on the trailing empty line"
        );
        assert_eq!(scroll_readout(&mut state), "Bot");
    }

    /// Scrolling *within* a wrapped first line moves the first byte off
    /// screen, and the readout has to notice.
    ///
    /// `scroll_top == 0` is still true here — which is why the cheap
    /// predicate would keep saying `Top` while row one of the document
    /// has scrolled away under the top edge.
    #[test]
    fn a_sub_line_residual_moves_off_top() {
        let long = "z".repeat(4000);
        let Some(mut state) = headless_or_skip(320, 240, &long) else {
            return;
        };
        wrapped_at_top(&mut state, true);
        assert_eq!(scroll_readout(&mut state), "Top");

        // Past several wrapped rows, still inside source line 0.
        state.code_scroll_residual = state.fm.code_line_height() * 4.0;
        state.reshape();
        assert_eq!(state.scroll_top, 0, "still the same source line");
        assert_ne!(
            scroll_readout(&mut state),
            "Top",
            "the document's first row is above the top edge, so `Top` \
             would be a claim about a row that is not painted"
        );
    }

    /// A mode for a buffer that is not on screen is not ours to apply.
    /// The daemon resends on buffer switch precisely so this holds.
    #[test]
    fn a_wrap_message_for_another_buffer_is_ignored() {
        let Some(mut state) = headless_or_skip(320, 200, "hello\n") else {
            return;
        };
        let mine = BufferId::next();
        let other = BufferId::next();
        state.current_buffer_id = Some(mine);
        state.apply_line_wrap(mine, true);
        let after_mine = state.buffer.wrap();
        state.apply_line_wrap(other, false);
        assert_eq!(
            state.buffer.wrap(),
            after_mine,
            "another buffer's mode must not reflow this one"
        );
    }

    /// scroll invariant intact throughout.
    #[test]
    #[allow(clippy::float_cmp)] // exact: assigned constants, not computed sums
    fn wrapped_caret_survives_size_changes() {
        let long = "x".repeat(180);
        let text = format!("{long}\nsecond\nthird\n");
        let Some(mut state) = headless_or_skip(320, 400, &text) else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: 180,
        });
        state.reshape();
        state.ensure_caret_painted();
        assert!(
            state.caret_painted_in_code_clip(),
            "precondition: the caret paints at the default size"
        );
        state.apply_attach_message(font_facts(None, Some(7200)));
        assert!(
            state.caret_painted_in_code_clip(),
            "the caret must survive the 16px → 72px re-wrap"
        );
        assert_eq!(
            state.buffer.scroll().line,
            0,
            "normalized: slice-local line 0"
        );
        assert_eq!(
            state.buffer.scroll().horizontal,
            0.0,
            "horizontal is discarded"
        );
        state.apply_attach_message(font_facts(None, Some(600)));
        assert!(
            state.caret_painted_in_code_clip(),
            "and the reverse 72px → 6px"
        );
        assert_eq!(state.buffer.scroll().line, 0);
    }

    /// Acceptance 11 — an overscan-only caret (shaped but below the
    /// drawable window) is NOT painted, and a font change must not
    /// snap the viewport back to it.
    #[test]
    fn overscan_caret_never_snaps_the_viewport() {
        let text = "line of text\n".repeat(40);
        let Some(mut state) = headless_or_skip(320, 400, &text) else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.reshape();
        let visible = estimated_visible_lines(state.config.height, state.fm, state.band_inset());
        let overscan_line = visible + 1;
        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: state.current_line_starts[overscan_line],
        });
        assert!(
            !state.caret_painted_in_code_clip(),
            "precondition: the overscan caret is not painted"
        );
        let top_before = state.scroll_top;
        state.apply_attach_message(font_facts(None, Some(2400)));
        assert_eq!(
            state.scroll_top, top_before,
            "an unpainted caret must never snap the viewport"
        );
    }

    /// Acceptance 11 — `resize()` applies the same painted-before
    /// policy: shrinking the window re-follows a painted caret
    /// through the coarse + visual-run + fold pipeline.
    #[test]
    #[allow(clippy::float_cmp)] // exact: assigned constants, not computed sums
    fn narrowing_resize_keeps_a_wrapped_caret_painted() {
        let mut text = "short\n".repeat(10);
        let long = "y".repeat(100);
        text.push_str(&long);
        let Some(mut state) = headless_or_skip(640, 400, &text) else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: text.len() as u64,
        });
        state.reshape();
        assert!(
            state.caret_painted_in_code_clip(),
            "precondition: everything fits at 640x400"
        );
        state.resize(320, 150);
        assert!(
            state.caret_painted_in_code_clip(),
            "the shrunken window must re-follow the caret into its wrapped run"
        );
        assert_eq!(state.buffer.scroll().line, 0);
        assert_eq!(state.buffer.scroll().horizontal, 0.0);
    }

    /// Acceptance 11 — resize preserves an intentionally caret-free
    /// viewport in both width directions instead of treating a
    /// stationary off-screen cursor as a follow request.
    #[test]
    fn width_resize_does_not_snap_a_scrolled_away_viewport() {
        let text = "line of text\n".repeat(80);
        let Some(mut state) = headless_or_skip(640, 240, &text) else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: 0,
        });
        let _ = state.scroll_by_lines(20);
        assert!(state.scroll_top > 0);
        assert!(!state.caret_painted_in_code_clip());

        let narrow_top = state.scroll_top;
        state.resize(320, 240);
        assert!(
            state.scroll_top >= narrow_top && !state.caret_painted_in_code_clip(),
            "narrowing must preserve the user's caret-free viewport"
        );
        let wide_top = state.scroll_top;
        state.resize(640, 240);
        assert!(
            state.scroll_top >= wide_top && !state.caret_painted_in_code_clip(),
            "widening must preserve the user's caret-free viewport"
        );
        assert!(state.buffer.layout_runs().next().is_some());
    }

    /// Acceptance 11 — a 72px caret-follow residual belongs to the
    /// viewport, not to the cursor. Moving the cursor off-screen and
    /// collapsing wraps at 6px normalizes that residual without a
    /// snap-back or a blank slice.
    #[test]
    fn large_to_small_rewrap_preserves_a_caret_free_viewport() {
        let line = "w".repeat(120);
        let text = (0..12)
            .map(|_| line.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let Some(mut state) = headless_or_skip(320, 240, &text) else {
            return;
        };
        state.apply_attach_message(font_facts(None, Some(7200)));
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: state.current_line_starts[8] + line.len() as u64,
        });
        state.ensure_caret_painted();
        assert!(
            state.scroll_top > 0 && state.code_scroll_residual > 0.0,
            "precondition: the large font follows into a deep wrapped run"
        );

        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: 0,
        });
        assert!(!state.caret_painted_in_code_clip());
        let top_before = state.scroll_top;
        state.apply_attach_message(font_facts(None, Some(600)));
        assert!(
            state.scroll_top >= top_before,
            "shrinking must not snap the caret-free viewport toward byte zero"
        );
        assert_eq!(state.buffer.scroll().line, 0);
        assert_eq!(
            state.view_range.0, state.current_line_starts[state.scroll_top],
            "the declared source range must agree with the normalized origin"
        );
        assert!(
            state.buffer.layout_runs().next().is_some(),
            "wrap collapse must leave a nonblank shaped slice"
        );
        assert!(!state.caret_painted_in_code_clip());
    }

    /// Acceptance 11 — completion uses the same visual-run-aware byte
    /// mapping as the caret, so an anchor at the end of a wrapped
    /// source line follows that run rather than its first run.
    #[test]
    fn completion_anchor_tracks_its_wrapped_visual_run() {
        let text = "c".repeat(180);
        let Some(mut state) = headless_or_skip(320, 400, &text) else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: text.len() as u64,
        });
        state.ensure_caret_painted();
        state.view_range = (0, text.len() as u64);
        state.completion = Some(CompletionLocal {
            buffer_id: bid,
            anchor: text.len() as u64,
            prefix_len: 0,
            rows: vec![CompletionPopupRow {
                label: "candidate".into(),
                kind: 3,
                detail: None,
            }],
            selected: Some(0),
            total: 1,
        });
        let (_, line_top, line_height) = state
            .completion_anchor_px()
            .expect("wrapped anchor is visible");
        assert!(
            line_top > TEXT_TOP + state.fm.code_line_height(),
            "the anchor must be on a continuation run, not source line 0's first run"
        );
        let (_, _, _, popup_top) = state
            .completion_dropdown_layout()
            .expect("the anchored popup fits");
        assert!(
            popup_top >= line_top + line_height - 0.01
                || popup_top + state.fm.mb_drop_row_height() <= line_top + 0.01,
            "the popup must be placed relative to the anchor's visual run"
        );
    }

    /// Acceptance 11 — the gutter projects one row per CODE visual
    /// run: the first run carries its source number and continuation
    /// rows stay blank, leaving the following source number aligned.
    #[test]
    fn gutter_rows_align_with_wrapped_code_continuations() {
        let text = format!("{}\nshort", "g".repeat(50));
        let Some(mut state) = headless_or_skip(320, 400, &text) else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        let _ = state.apply_attach_message(InstanceMessage::LineNumbers {
            buffer_id: bid,
            mode: LineNumberMode::Absolute,
        });
        state.apply_attach_message(font_facts(None, Some(2400)));
        state.refresh_gutter_buffer();

        let code_tops: Vec<f32> = state.buffer.layout_runs().map(|run| run.line_top).collect();
        let gutter_tops: Vec<f32> = state
            .gutter_buffer
            .layout_runs()
            .map(|run| run.line_top)
            .collect();
        assert!(
            state
                .buffer
                .layout_runs()
                .filter(|run| run.line_i == 0)
                .count()
                > 1,
            "precondition: source line 1 wraps"
        );
        assert_eq!(gutter_tops.len(), code_tops.len());
        for (code, gutter) in code_tops.iter().zip(&gutter_tops) {
            assert!(
                (code - gutter).abs() < 0.01,
                "code/gutter visual rows diverged: {code} vs {gutter}"
            );
        }

        let first_runs = state
            .buffer
            .layout_runs()
            .filter(|run| run.line_i == 0)
            .count();
        assert_eq!(state.gutter_buffer.lines[0].text().trim(), "1");
        assert!(
            state.gutter_buffer.lines[1..first_runs]
                .iter()
                .all(|line| line.text().is_empty()),
            "wrapped continuation rows must carry blank gutter labels"
        );
        assert_eq!(
            state.gutter_buffer.lines[first_runs].text().trim(),
            "2",
            "the next source-line number must follow all continuation blanks"
        );
    }

    /// Acceptance 11 — the caret projection inverts the chunk cache:
    /// injected adornment text shifts a caret past the anchor by the
    /// projected width, while a caret AT the anchor keeps left
    /// gravity (before the injected text).
    #[test]
    fn caret_projection_accounts_for_inline_adornments() {
        let Some(mut state) = headless_or_skip(320, 240, "ab\ncd") else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.reshape();
        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: 2,
        });
        let x_plain = state.caret_rect().expect("caret on line 0").x;
        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: 1,
        });
        let x1_plain = state.caret_rect().expect("caret at byte 1").x;
        state.current_adornments = vec![adornment(1, AdornmentPlacement::AtOffset, "hint")];
        state.reshape();
        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: 2,
        });
        let x_projected = state.caret_rect().expect("caret past the adornment").x;
        assert!(
            x_projected > x_plain + 3.0 * 9.0,
            "the caret must sit past the injected text ({x_projected} vs {x_plain})"
        );
        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: 1,
        });
        let x_anchor = state.caret_rect().expect("caret at the anchor").x;
        assert!(
            (x_anchor - x1_plain).abs() < 0.5,
            "an anchor byte keeps left gravity — before the adornment \
             ({x_anchor} vs {x1_plain})"
        );
    }

    #[test]
    fn source_tab_caret_uses_projected_leading_and_trailing_boundaries() {
        let Some(mut state) = headless_or_skip(320, 240, "\tX") else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.reshape();

        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: 0,
        });
        let before = state.caret_rect().expect("caret before tab").x;
        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: 1,
        });
        let after = state.caret_rect().expect("caret after tab").x;
        assert!(
            after - before > 7.0 * state.mono_advance(),
            "one source byte must span all eight projected spaces"
        );
    }

    #[test]
    fn source_tab_hit_testing_uses_projected_space_boundaries() {
        let Some(mut state) = headless_or_skip(320, 240, "\tX") else {
            return;
        };
        state.current_buffer_id = Some(BufferId::next());
        state.reshape();
        let advance = state.mono_advance();
        let y = f64::from(TEXT_TOP + state.fm.code_line_height() / 2.0);

        assert_eq!(
            state.hit_test_source_byte(f64::from(state.text_left()), y),
            Some(0),
            "the projected leading edge maps before the source tab"
        );
        assert_eq!(
            state.hit_test_source_byte(f64::from(state.text_left() + advance * 2.5), y,),
            Some(1),
            "a hit inside the expanded spaces maps after the source tab"
        );
    }

    #[test]
    fn tab_decoration_geometry_covers_spaces_split_by_soft_wrap() {
        let Some(mut state) = headless_or_skip(64, 240, "\tX") else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.current_decorations = vec![Decoration {
            range: ByteRange { start: 0, end: 1 },
            kind: DecorationKind::Selection,
        }];
        state.reshape();
        assert!(
            state
                .buffer
                .layout_runs()
                .filter(|run| run.line_i == 0)
                .count()
                > 1,
            "precondition: the eight projected spaces wrap"
        );

        let line_offsets = line_byte_offsets(&state.current_text);
        let mut rects = Vec::new();
        state.collect_own_decoration_rects(
            &mut rects,
            &line_offsets,
            state.view_range.0,
            state.view_range.1,
        );
        assert!(
            rects.len() > 1,
            "the source tab selection must fan out across wrapped visual runs"
        );
        assert!(
            rects.iter().all(|rect| rect.w > 0.0),
            "every wrapped piece must retain horizontal geometry"
        );
    }

    #[test]
    fn visible_line_tab_edit_refreshes_cached_projection() {
        let Some(mut state) = headless_or_skip(320, 240, "aX") else {
            return;
        };
        state.minimap_cache = Some(((0, 0, 0, 0), vec![1]));
        let edits = state
            .apply_loro_text_delta_batches(&[vec![
                loro::TextDelta::Retain {
                    retain: 1,
                    attributes: None,
                },
                loro::TextDelta::Insert {
                    insert: "\t".to_owned(),
                    attributes: None,
                },
            ]])
            .expect("visible edit applies");

        assert_eq!(
            edits,
            vec![TextProjectionEdit {
                start: 1,
                old_end: 1,
                inserted_len: 1,
            }]
        );
        assert_eq!(state.buffer.lines[0].text(), "a       X");
        assert_eq!(
            state.current_line_shapes[0],
            MinimapLineShape {
                indent_cols: 0,
                content_cols: 9,
            },
            "the minimap shape must refresh in the same edit transaction"
        );
        assert!(
            state.minimap_cache.is_none(),
            "text geometry changes must invalidate cached minimap vertices"
        );
        assert!(
            state.line_chunk_cache[0]
                .iter()
                .any(|chunk| matches!(chunk.source, ChunkSource::SourceTab { start: 1 })),
            "the incremental code-line cache must immediately carry tab provenance"
        );
    }

    /// Acceptance 11 — the `CursorByte` arm follows into a wrapped
    /// continuation run (the pre-existing source-line-only hole): the
    /// follow lands as a sub-line residual, normalized to slice-local
    /// line 0.
    #[test]
    #[allow(clippy::float_cmp)] // exact: assigned constants, not computed sums
    fn cursor_byte_follows_into_a_wrapped_run() {
        let long = "z".repeat(400);
        let Some(mut state) = headless_or_skip(320, 240, &long) else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.reshape();
        let _ = state.apply_attach_message(InstanceMessage::CursorByte {
            buffer_id: bid,
            byte_pos: 400,
        });
        assert!(
            state.caret_painted_in_code_clip(),
            "the CursorByte arm must follow into the wrapped run"
        );
        assert!(
            state.code_scroll_residual > 0.0,
            "the follow is a sub-line residual, not a source-line scroll"
        );
        assert_eq!(state.buffer.scroll().line, 0);
        assert_eq!(state.buffer.scroll().horizontal, 0.0);
    }

    /// Acceptance 11 — an optimistic insertion that creates a new
    /// bottom-edge wrap follows immediately (waiting a round trip
    /// reads as a hitch), and the identical confirming `CursorByte`
    /// needs no second repair.
    #[test]
    #[allow(clippy::float_cmp)] // exact: comparing a value to itself across a no-op
    fn optimistic_insertion_follows_a_new_bottom_edge_wrap() {
        // 320 wide → 304px drawable → 31 glyphs per row at the 9.6px
        // advance; 240 high → 198px drawable → 9 rows. Exactly 9 full
        // rows of text puts the caret at the painted bottom edge, and
        // one more glyph starts row 10 below the clip.
        let text = "q".repeat(31 * 9);
        let Some(mut state) = headless_or_skip(320, 240, "") else {
            return;
        };
        let bid = BufferId::next();
        let doc = loro::LoroDoc::new();
        doc.get_text(LORO_TEXT_CONTAINER)
            .insert(0, &text)
            .expect("insert snapshot text");
        let _ = state.apply_attach_message(InstanceMessage::BufferSnapshot {
            buffer_id: bid,
            crdt_snapshot: doc.export(loro::ExportMode::Snapshot).expect("export"),
        });
        state.set_frontend_id(FrontendId(9001));
        state.dispatch_idle = true;
        let _ = state.apply_attach_message(InstanceMessage::CursorByte {
            buffer_id: bid,
            byte_pos: text.len() as u64,
        });
        assert!(
            state.caret_painted_in_code_clip(),
            "precondition: the end caret paints on the last full row"
        );
        let send = state
            .optimistic_crdt_insert(ProtocolKey::Char('q'), Modifiers::NONE)
            .expect("the optimistic insert path is eligible");
        assert_eq!(send.buffer_id, bid);
        assert!(
            state.caret_painted_in_code_clip(),
            "the insertion wrapped a new bottom row — the caret must follow NOW"
        );
        assert!(
            state.code_scroll_residual > 0.0,
            "the follow is a visual-run residual"
        );
        let residual = state.code_scroll_residual;
        let top = state.scroll_top;
        // The daemon confirms the predicted byte: `moved == false`, so
        // no second repair may disturb the settled scroll.
        let _ = state.apply_attach_message(InstanceMessage::CursorByte {
            buffer_id: bid,
            byte_pos: text.len() as u64 + 1,
        });
        assert_eq!(state.code_scroll_residual, residual);
        assert_eq!(state.scroll_top, top);
    }

    /// Acceptance 11 — an explicit scroll clears the caret-follow
    /// residual, including a wheel-up at the top clamp whose only
    /// remaining motion IS the residual.
    #[test]
    #[allow(clippy::float_cmp)] // exact: assigned constants, not computed sums
    fn explicit_scroll_clears_the_caret_follow_residual() {
        let long = "w".repeat(400);
        let Some(mut state) = headless_or_skip(320, 240, &long) else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.reshape();
        let _ = state.apply_attach_message(InstanceMessage::CursorByte {
            buffer_id: bid,
            byte_pos: 400,
        });
        assert!(
            state.code_scroll_residual > 0.0,
            "precondition: residual armed"
        );
        assert_eq!(
            state.scroll_top, 0,
            "single source line: the residual IS the scroll"
        );
        let _ = state.scroll_by_lines(-1);
        assert_eq!(
            state.code_scroll_residual, 0.0,
            "wheel-up at the clamp edge must scroll the residual away"
        );
        assert!(
            !state.caret_painted_in_code_clip(),
            "the deep wrapped caret is off-screen again — the user owns the viewport"
        );
    }

    /// Acceptance 4 (GPU half) — the caret-follow residual is
    /// buffer-scoped view state: a `BufferSnapshot` resets it with
    /// the rest of the view, while the font preference and derived
    /// metrics survive the switch.
    #[test]
    #[allow(clippy::float_cmp)] // exact: assigned constants, not computed sums
    fn buffer_snapshot_resets_the_residual_but_not_the_font() {
        let long = "v".repeat(400);
        let Some(mut state) = headless_or_skip(320, 240, &long) else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.reshape();
        state.apply_attach_message(font_facts(None, Some(2400)));
        let _ = state.apply_attach_message(InstanceMessage::CursorByte {
            buffer_id: bid,
            byte_pos: 400,
        });
        assert!(
            state.code_scroll_residual > 0.0,
            "precondition: residual armed"
        );
        // The replacement buffer wraps too: a leaked residual would
        // NOT be zeroed by the EOF clamp, so the assertion below can
        // only pass through the snapshot arm's explicit reset.
        let doc = loro::LoroDoc::new();
        doc.get_text(LORO_TEXT_CONTAINER)
            .insert(0, &"u".repeat(400))
            .expect("insert snapshot text");
        let _ = state.apply_attach_message(InstanceMessage::BufferSnapshot {
            buffer_id: BufferId::next(),
            crdt_snapshot: doc.export(loro::ExportMode::Snapshot).expect("export"),
        });
        assert_eq!(
            state.code_scroll_residual, 0.0,
            "the residual is buffer-scoped view state"
        );
        assert!(
            (state.fm.scale - 1.5).abs() < f32::EPSILON,
            "the global font preference survives the switch"
        );
        assert_eq!(state.resolved_family, DEFAULT_FONT_FAMILY);
    }

    /// The follow decision is suspended while the minibuffer is open:
    /// its caret lives in the band, not the code area.
    #[test]
    fn open_minibuffer_suppresses_the_caret_follow_decision() {
        let Some(mut state) = headless_or_skip(320, 240, "hello") else {
            return;
        };
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: 2,
        });
        state.reshape();
        assert!(state.caret_painted_in_code_clip());
        state.minibuffer = Some(MinibufferLocal {
            prompt: ":".into(),
            input: String::new(),
            cursor: 0,
            rows: Vec::new(),
            selected: None,
            total: 0,
        });
        assert!(
            !state.caret_painted_in_code_clip(),
            "an open minibuffer owns the caret — the code-area decision is false"
        );
    }

    #[test]
    fn cursor_inside_combining_cluster_uses_the_containing_boundary() {
        let text = "xxe\u{301}z";
        let Some(mut state) = headless_or_skip(320, 240, text) else {
            return;
        };
        let inside = "xxe".len();
        let cluster_end = "xxe\u{301}".len();
        let (slice_i, projected) = state
            .code_byte_to_projected(inside as u64)
            .expect("the source byte is in the shaped slice");
        let ranges: Vec<(usize, usize)> = state
            .buffer
            .line_layout(&mut state.font_system, slice_i)
            .expect("line layout")
            .iter()
            .flat_map(|line| line.glyphs.iter().map(|glyph| (glyph.start, glyph.end)))
            .collect();
        assert!(
            ranges
                .iter()
                .any(|&(start, end)| start < projected && projected < end),
            "precondition: the byte between e and its combining mark is inside a shaped cluster; \
             ranges={ranges:?}"
        );

        let line_start_x = state.code_byte_px(0).expect("line start").0;
        let inside_x = state
            .code_byte_px(inside as u64)
            .expect("inside-cluster caret")
            .0;
        let cluster_end_x = state
            .code_byte_px(cluster_end as u64)
            .expect("cluster-end caret")
            .0;
        assert!(
            inside_x > line_start_x,
            "an inside-cluster cursor must not fall back to line start"
        );
        assert!(
            (inside_x - cluster_end_x).abs() < 0.01,
            "the unrepresentable interior position snaps to the containing cluster's end"
        );
    }

    #[test]
    fn caret_follow_inside_a_ligature_targets_its_visual_run() {
        let prefix = "x".repeat(320);
        let text = format!("{prefix}fiz");
        let inside = prefix.len() + 1;
        let Some(mut state) = headless_or_skip(320, 240, &text) else {
            return;
        };
        state.resolved_family = "Pmacs Test Mono Two".to_owned();
        state.reshape();

        let (slice_i, projected) = state
            .code_byte_to_projected(inside as u64)
            .expect("the ligature byte is in the shaped slice");
        let ranges: Vec<(usize, usize)> = state
            .buffer
            .line_layout(&mut state.font_system, slice_i)
            .expect("line layout")
            .iter()
            .flat_map(|line| line.glyphs.iter().map(|glyph| (glyph.start, glyph.end)))
            .collect();
        assert!(
            ranges
                .iter()
                .any(|&(start, end)| start < projected && projected < end),
            "precondition: the generated fixture shapes fi as one cluster; ranges={ranges:?}"
        );

        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: inside as u64,
        });
        state.ensure_caret_painted();
        assert!(
            state.code_scroll_residual > 0.0,
            "the ligature lives on a deep wrapped run, which must be followed"
        );
        assert!(
            state.caret_painted_in_code_clip(),
            "geometry and follow must agree on the normalized cluster boundary"
        );
    }

    /// Acceptance 19 — family-dependent geometry on an EMPTY document:
    /// the measured probe advance (not a shaped code glyph) drives the
    /// gutter fallback and menu char width through the advance ratio.
    #[test]
    #[allow(clippy::float_cmp)] // exact: the reset re-measures the identical face
    fn family_advance_ratio_scales_empty_document_geometry() {
        let Some(mut state) = headless_or_skip(320, 240, "") else {
            return;
        };
        // Line numbers + a context menu over the EMPTY buffer: no code
        // glyph has ever shaped, so any advance the geometry uses must
        // come from the probe.
        state.line_numbers = LineNumberMode::Absolute;
        state.menu = Some(MenuLocal {
            rows: vec![MenuPromptRow {
                // Long enough that the estimated width sits BETWEEN
                // the min/max clamps at both ratios — a short label
                // pins to MENU_MIN_WIDTH and hides the scaling.
                label: "a context menu entry armed here".into(),
                separator: false,
            }],
            active: Some(0),
            anchor_px: (40.0, 40.0),
        });
        let gutter_before = state.gutter_width_px();
        let menu_w_before = State::menu_width_px(state.menu.as_ref().unwrap(), state.fm);
        let frame_before = state.render_offscreen();
        state.apply_attach_message(font_facts(Some("Pmacs Test Mono Two"), None));
        assert_eq!(state.resolved_family, "Pmacs Test Mono Two");
        // Fixture advance 720/1000 vs JetBrains Mono 600/1000 → 1.2.
        assert!(
            (state.fm.advance_ratio - 1.2).abs() < 0.01,
            "measured selected/default ratio, got {}",
            state.fm.advance_ratio
        );
        assert!(
            (state.mono_advance() - 11.52).abs() < 0.1,
            "the measured NORMAL-face advance is authoritative on an empty \
             document, got {}",
            state.mono_advance()
        );
        assert!(
            (state.fm.menu_char_w() - BASE_MENU_CHAR_W * 1.2).abs() < 0.05,
            "menu hit width follows the ratio, got {}",
            state.fm.menu_char_w()
        );
        assert!(
            state.gutter_width_px() > gutter_before,
            "the gutter reservation follows the measured advance"
        );
        assert!(
            State::menu_width_px(state.menu.as_ref().unwrap(), state.fm) > menu_w_before,
            "the menu hit width follows the measured advance"
        );
        state.apply_attach_message(font_facts(None, None));
        assert_eq!(
            state.gutter_width_px(),
            gutter_before,
            "reset: exact gutter"
        );
        assert_eq!(
            State::menu_width_px(state.menu.as_ref().unwrap(), state.fm),
            menu_w_before,
            "reset: exact menu width"
        );
        assert_eq!(
            state.render_offscreen(),
            frame_before,
            "reset restores the original frame"
        );
    }

    /// Acceptance 12 — unresolvable and proportional families both
    /// take the total fallback to the sanitized default.
    #[test]
    #[allow(clippy::float_cmp)] // exact: assigned constants, not computed sums
    fn unresolvable_and_proportional_families_fall_back() {
        let Some(mut state) = headless_or_skip(320, 240, "text") else {
            return;
        };
        let never_set = state.render_offscreen();
        state.apply_attach_message(font_facts(Some("No Such Family Zzz"), None));
        assert_eq!(state.resolved_family, DEFAULT_FONT_FAMILY);
        assert_eq!(
            state.render_offscreen(),
            never_set,
            "the unresolvable route renders byte-identically to never-set"
        );
        state.apply_attach_message(font_facts(Some("Pmacs Test Proportional"), None));
        assert_eq!(
            state.resolved_family, DEFAULT_FONT_FAMILY,
            "a proportional family must fall back"
        );
        assert_eq!(
            state.fm.advance_ratio, 1.0,
            "the fallback is the default: ratio 1"
        );
        assert_eq!(
            state.render_offscreen(),
            never_set,
            "the rejected route renders byte-identically to never-set"
        );
    }

    /// Acceptance 12 — the four-style gate rejects a family whose
    /// BOLD sibling is proportional, exactly what a normal-only
    /// query check would wave through.
    #[test]
    fn four_style_gate_rejects_a_proportional_bold_sibling() {
        let Some(mut state) = headless_or_skip(320, 240, "text") else {
            return;
        };
        assert!(
            query_normal_face(state.font_system.db(), "Pmacs Test Family")
                .and_then(|id| state.font_system.db().face(id))
                .is_some_and(|face| face.monospaced),
            "precondition: the NORMAL face alone looks monospaced"
        );
        assert!(state.family_is_monospace_everywhere("Pmacs Test Mono Two"));
        assert!(
            !state.family_is_monospace_everywhere("Pmacs Test Family"),
            "the proportional BOLD sibling must fail the four-style gate"
        );
        state.apply_attach_message(font_facts(Some("Pmacs Test Family"), None));
        assert_eq!(state.resolved_family, DEFAULT_FONT_FAMILY);
    }

    /// Acceptance 12 — a valid second monospace family resolves,
    /// changes the ink, and the `(None, None)` reset restores the
    /// default frame byte-for-byte.
    #[test]
    fn second_monospace_family_changes_the_frame_and_reset_restores() {
        let Some(mut state) = headless_or_skip(320, 240, "0123456789") else {
            return;
        };
        let default_frame = state.render_offscreen();
        state.apply_attach_message(font_facts(Some("Pmacs Test Mono Two"), None));
        assert_eq!(state.resolved_family, "Pmacs Test Mono Two");
        assert_ne!(
            state.render_offscreen(),
            default_frame,
            "different outlines/advances must change the ink"
        );
        state.apply_attach_message(font_facts(None, None));
        assert_eq!(state.resolved_family, DEFAULT_FONT_FAMILY);
        assert_eq!(
            state.render_offscreen(),
            default_frame,
            "the reset restores the default frame byte-for-byte"
        );
    }

    /// Acceptance 12 — the sanitizer (parameterized, the production
    /// path) removes exactly the non-monospace same-family
    /// collisions: the monospaced default and unrelated families
    /// survive.
    #[test]
    fn sanitizer_removes_only_same_family_proportional_collisions() {
        let mut db = fontdb::Database::new();
        let bold_id = *db
            .load_font_source(fontdb::Source::Binary(std::sync::Arc::new(
                TEST_FAMILY_BOLD,
            )))
            .first()
            .expect("bold fixture loads");
        let regular_id = *db
            .load_font_source(fontdb::Source::Binary(std::sync::Arc::new(
                TEST_FAMILY_REGULAR,
            )))
            .first()
            .expect("regular fixture loads");
        let unrelated_id = *db
            .load_font_source(fontdb::Source::Binary(std::sync::Arc::new(
                TEST_PROPORTIONAL,
            )))
            .first()
            .expect("proportional fixture loads");
        sanitize_font_database(&mut db, "Pmacs Test Family", regular_id);
        assert!(
            db.face(bold_id).is_none(),
            "the proportional same-family BOLD collision is removed"
        );
        assert!(
            db.face(regular_id).is_some(),
            "the monospaced default survives"
        );
        assert!(
            db.face(unrelated_id).is_some(),
            "an unrelated proportional family is untouched"
        );
    }

    // --- Vterm Stage 3: terminal mode -----------------------------------

    // ===================================================================
    // Bottom panel Stage 2B-3 — the GPU band
    // ===================================================================

    fn panel_frame_of(rows: u32, cols: u32, geometry_epoch: u64, panel_epoch: u64) -> PanelFrame {
        let cells = (0..(rows as usize * cols as usize))
            .map(|_| terminal_cell(pmacs_protocol::Glyph::Char('x'), CellStyle::default()))
            .collect();
        PanelFrame {
            buffer_id: BufferId::from_raw(77),
            panel_epoch,
            geometry_epoch,
            size: CellSize::new(rows, cols),
            cells,
            cursor: None,
            focused: true,
        }
    }

    /// Bring a `State` to the point a real v21 session reaches: panel wire
    /// on, one geometry declaration made, one `Present` frame installed.
    fn present_panel(state: &mut State, rows: u32) -> PanelFrame {
        state.set_panel_wire(PANEL_MIN_VERSION);
        let (epoch, total) = state
            .next_geometry_declaration(GeometryTrigger::Surface)
            .expect("a panel-wire session declares its first geometry");
        let frame = panel_frame_of(rows, total.cols.max(1), epoch, 1);
        assert!(
            state.apply_panel_payload(PanelFramePayload::Present(frame.clone())),
            "installing a first frame changes the band"
        );
        frame
    }

    /// A2B-4 (contrast assertion) — installing a panel moves every
    /// document-owned boundary by exactly the band's pixel height while
    /// every status-owned boundary stays pixel-identical.
    ///
    /// Both halves in one scenario on purpose: a uniformly wrong
    /// implementation that subtracted the band from `status_band_top` too
    /// would pass the "everything moved" half by itself.
    #[test]
    fn installing_a_panel_moves_the_document_bottom_and_not_the_status_band() {
        let Some(mut state) = headless_or_skip(800, 600, "alpha\nbeta\ngamma\n") else {
            return;
        };
        let fm = state.fm;
        let status_before = status_band_top(state.config.height, fm);
        // Anchored to an INDEPENDENT formula, not to the helper's own earlier
        // value. Comparing `status_band_top` before and after is a fixed
        // point: a blanket rewrite that subtracted the band from it too would
        // move both readings together and pass. (It did — this assertion
        // exists because the first version of this test was vacuous under
        // exactly that mutation.)
        assert!(
            (status_before - (state.config.height as f32 - fm.status_band_height())).abs()
                < f32::EPSILON,
            "the status band top is the physical window bottom minus the band height"
        );
        let document_before = document_text_bottom(state.config.height, fm, state.band_inset());
        assert_eq!(
            state.band_inset(),
            PanelBandInset::ABSENT,
            "no panel yet, so the two boundaries coincide"
        );
        assert!((status_before - document_before).abs() < f32::EPSILON);

        let rows = 4;
        present_panel(&mut state, rows);

        let band = state.band_inset();
        assert_eq!(
            band,
            PanelBandInset::installed(rows, fm),
            "the inset is the panel's rows plus its divider"
        );
        assert!(band.px() > 0.0);
        assert!(
            (status_band_top(state.config.height, fm) - status_before).abs() < f32::EPSILON,
            "the status band must stay pixel-identical at the physical window bottom"
        );
        assert!(
            (document_text_bottom(state.config.height, fm, band) - (document_before - band.px()))
                .abs()
                < f32::EPSILON,
            "the document bottom must move by exactly the band height"
        );
        // Every document-owned consumer follows the moved boundary, and the
        // three easiest to misclassify are named because each has its own
        // visible symptom.
        assert!(
            estimated_visible_lines(state.config.height, fm, band)
                < estimated_visible_lines(state.config.height, fm, PanelBandInset::ABSENT),
            "the visible-line estimate must shrink with the document area"
        );
        assert!(
            minimap_height(state.config.height, fm, band)
                < minimap_height(state.config.height, fm, PanelBandInset::ABSENT),
            "the minimap is document-owned"
        );
        // Edge scrolling (the row a plausible implementation leaves on the
        // old bottom, which then auto-scrolls from INSIDE the panel). The
        // probe pixel sits near the BOTTOM of the band, inside the unmoved
        // boundary's own edge strip, so the two answers genuinely differ.
        let deep_in_band = status_before - 2.0;
        assert!(
            deep_in_band > document_text_bottom(state.config.height, fm, band),
            "fixture must place the probe inside the band"
        );
        assert_eq!(
            edge_scroll_direction(deep_in_band, state.config.height, fm, band),
            None,
            "a pixel inside the band must not arm document edge scrolling"
        );
        assert_eq!(
            edge_scroll_direction(
                deep_in_band,
                state.config.height,
                fm,
                PanelBandInset::ABSENT
            ),
            Some(1),
            "and it WOULD have, on the unmoved boundary — so this pins the move, \
             not merely the absence of a trigger"
        );
        // And the feature is not simply switched off: the moved boundary has
        // its own edge strip, which still arms.
        assert_eq!(
            edge_scroll_direction(
                document_text_bottom(state.config.height, fm, band) - 2.0,
                state.config.height,
                fm,
                band
            ),
            Some(1),
            "the document's own bottom edge strip still auto-scrolls"
        );
    }

    /// The geometry declaration reserves the divider while the panel is
    /// absent, and the document loses no pixels until a `Present` is
    /// painted. That asymmetry is what breaks the first-open cycle.
    #[test]
    fn geometry_capacity_reserves_the_divider_while_the_document_does_not() {
        let fm = FontMetrics::default();
        let height = 600;
        assert!(fm.divider_height() > 0.0);
        assert!(
            (status_band_top(height, fm)
                - geometry_capacity_bottom(height, fm)
                - fm.divider_height())
            .abs()
                < f32::EPSILON,
            "capacity always reserves the divider"
        );
        assert!(
            (document_text_bottom(height, fm, PanelBandInset::ABSENT)
                - status_band_top(height, fm))
            .abs()
                < f32::EPSILON,
            "while an absent panel costs the document nothing"
        );
    }

    /// All three boundaries clamp at zero on a surface shorter than its own
    /// chrome, preserving the pre-split `.max(0.0)` behavior.
    #[test]
    fn every_boundary_clamps_at_zero_on_a_surface_shorter_than_its_chrome() {
        let fm = FontMetrics::default();
        for height in [0, 1, 4, 10] {
            assert!(status_band_top(height, fm) >= 0.0);
            assert!(geometry_capacity_bottom(height, fm) >= 0.0);
            assert!(document_text_bottom(height, fm, PanelBandInset::installed(9, fm)) >= 0.0);
        }
    }

    /// A2B-3 — the declaration, the painter, and the hit test all resolve
    /// cells with the SAME advance, and it is the stable normal-face probe.
    ///
    /// Asserting the declaration alone is what the first version of this test
    /// did, and it was not enough: painting and hit-testing were still using
    /// the document-dependent advance, so the daemon's column count, the
    /// painted column positions, and the cell a click resolved to were three
    /// different grids while this test passed. All three directions are
    /// asserted here.
    #[test]
    fn declared_painted_and_hit_tested_cells_share_the_probe_advance() {
        let Some(mut state) = headless_or_skip(800, 600, "abcdefgh") else {
            return;
        };
        let probe = state
            .panel_probe_advance()
            .expect("the default family shapes a width");

        // Force the two derivations apart, because that is the only way to
        // pin WHICH one each consumer uses. In production they diverge
        // through `mono_advance`'s document-glyph fallback — a file opening
        // with a double-width or bold glyph — which is document-dependent and
        // would therefore make the panel's geometry depend on what happens to
        // be open. Separating them directly keeps the assertion off font
        // internals.
        state.measured_mono_advance = Some(probe * 2.0);
        let document_advance = state.mono_advance();
        assert!(
            (document_advance - probe).abs() > f32::EPSILON,
            "fixture must separate the two derivations"
        );

        // 1. The declaration. Full surface width from x = 0, NOT
        //    `width - TEXT_LEFT`: the band is not inset by document padding.
        let expected = crate::terminal::panel_cell_capacity(
            state.config.width as f32,
            (geometry_capacity_bottom(state.config.height, state.fm) - TEXT_TOP).max(0.0),
            probe,
            state.fm.code_line_height(),
        )
        .expect("an 800x600 surface admits panel cells");
        let (declared, declared_advance) = state.declared_cell_total();
        assert_eq!(
            declared, expected,
            "the declaration resolves its advance from the stable normal-face \
             probe and its width from the whole surface"
        );
        assert_eq!(declared_advance, Some(probe));
        assert_ne!(
            expected.cols,
            crate::terminal::panel_cell_capacity(
                state.config.width as f32,
                (geometry_capacity_bottom(state.config.height, state.fm) - TEXT_TOP).max(0.0),
                document_advance,
                state.fm.code_line_height(),
            )
            .expect("the wider advance still admits cells")
            .cols,
            "and the two genuinely produce different column counts here, so \
             every assertion below is discriminating"
        );

        // 2. The painter. A run at column N lands at N * the SAME advance,
        //    measured from x = 0.
        present_panel(&mut state, 3);
        let (ox, oy, bw, _) = state
            .panel_content_rect()
            .expect("a presented band has a rect");
        assert!(
            ox.abs() < f32::EPSILON,
            "the band starts at x = 0, not at the document's TEXT_LEFT"
        );
        assert!(
            (bw - state.config.width as f32).abs() < f32::EPSILON,
            "and spans the full surface width"
        );
        let (rx, _, rw, _) = state
            .panel_run_rect(crate::terminal::CellRun {
                row: 0,
                start_col: 5,
                end_col: 6,
            })
            .expect("a presented band places its runs");
        assert!(
            (rx - 5.0 * probe).abs() < 0.01,
            "the painter must place column 5 at 5 probe advances, got {rx}"
        );
        assert!(
            (rw - probe).abs() < 0.01,
            "and a one-cell run must be one probe advance wide, got {rw}"
        );

        // 3. The hit test. The pixel the painter puts column 5 at must
        //    resolve back to column 5.
        assert_eq!(
            state.panel_hit_test(rx + probe / 2.0, oy + 0.5),
            Some(pmacs_protocol::CellCoord::new(0, 5)),
            "a click on the painted cell must resolve to that cell"
        );
        // The last declared column is reachable, and the fractional remainder
        // past it maps to no cell and therefore emits no `PanelPointer`.
        let frame = state.panel.presented().expect("presented").clone();
        let last = frame.size.cols - 1;
        assert_eq!(
            state.panel_hit_test(last as f32 * probe + probe / 2.0, oy + 0.5),
            Some(pmacs_protocol::CellCoord::new(0, last))
        );
        assert!(
            state
                .panel_hit_test(frame.size.cols as f32 * probe + 0.5, oy + 0.5)
                .is_none(),
            "the right-edge remainder is band background, not a cell"
        );

        // A probe that shapes no width declares zero usable geometry rather
        // than reaching for a document sample.
        assert_eq!(
            crate::terminal::panel_cell_capacity(800.0, 600.0, 0.0, 22.0),
            None
        );
    }

    /// The pixel→cell conversion, including the daemon's virtual status row
    /// and floor rounding (parent 41, GPU half).
    #[test]
    fn panel_cell_capacity_floors_and_carries_the_virtual_status_row() {
        // 100px of usable height at a 22px line height is 4 whole rows, plus
        // the virtual status row the daemon subtracts back off.
        let size = crate::terminal::panel_cell_capacity(100.0, 100.0, 10.0, 22.0)
            .expect("a 100x100 rectangle admits cells");
        assert_eq!(size.rows, 5, "4 document rows + 1 virtual status row");
        assert_eq!(size.cols, 10);
        // Fractional widths floor rather than round.
        let fractional = crate::terminal::panel_cell_capacity(99.9, 43.9, 10.0, 22.0)
            .expect("fractional inputs still admit cells");
        assert_eq!((fractional.rows, fractional.cols), (2, 9));
        // A panel may legitimately be wider than a PTY: no 512 cap.
        let wide = crate::terminal::panel_cell_capacity(6000.0, 44.0, 1.0, 22.0)
            .expect("a wide surface admits a wide panel");
        assert_eq!(
            wide.cols, 6000,
            "the panel does not inherit the terminal's 512-column PTY cap"
        );
        assert!(
            crate::terminal::cell_viewport(6000.0, 44.0, 1.0, 22.0)
                .expect("terminal viewport")
                .cols
                <= u32::from(pmacs_protocol::MAX_TERMINAL_COLS),
            "while the terminal projection still clamps"
        );
    }

    /// Zero, non-finite, and non-positive metric inputs fail closed to zero
    /// usable geometry rather than an absurd row count (parent 41).
    #[test]
    fn degenerate_metrics_declare_zero_usable_geometry() {
        for (w, h, a, l) in [
            (0.0, 100.0, 10.0, 22.0),
            (100.0, 0.0, 10.0, 22.0),
            (100.0, 100.0, 0.0, 22.0),
            (100.0, 100.0, 10.0, 0.0),
            (f32::NAN, 100.0, 10.0, 22.0),
            (100.0, f32::INFINITY, 10.0, 22.0),
            (100.0, 100.0, -10.0, 22.0),
        ] {
            assert_eq!(
                crate::terminal::panel_cell_capacity(w, h, a, l),
                None,
                "degenerate input ({w}, {h}, {a}, {l}) must fail closed"
            );
        }
    }

    /// A2B-2 — a font or scale change that leaves `CellSize` IDENTICAL still
    /// produces a new `geometry_epoch`, and the older retained frame neither
    /// paints nor hit-tests until a matching `Present` arrives.
    #[test]
    fn identical_cell_totals_still_advance_the_epoch_on_a_metrics_change() {
        let Some(mut state) = headless_or_skip(800, 600, "alpha") else {
            return;
        };
        let frame = present_panel(&mut state, 3);
        let first_epoch = frame.geometry_epoch;
        assert!(state.band_inset().px() > 0.0);

        // The `Surface` trigger dedups an unchanged total.
        assert_eq!(
            state.next_geometry_declaration(GeometryTrigger::Surface),
            None,
            "an identical cell total is not re-declared on a resize"
        );
        assert_eq!(state.panel.geometry_epoch, first_epoch);

        // The `Metrics` trigger must not.
        let (second_epoch, second_total) = state
            .next_geometry_declaration(GeometryTrigger::Metrics)
            .expect("a metrics change always re-declares");
        assert_eq!(
            second_total,
            state.panel.declared.expect("declared"),
            "the total is genuinely unchanged — which is the whole point"
        );
        assert!(second_epoch > first_epoch);
        assert!(
            state.panel.presented().is_none(),
            "the retained frame answers a superseded declaration, so it must \
             neither paint nor hit-test"
        );
        assert_eq!(state.band_inset(), PanelBandInset::ABSENT);
        assert!(state.panel_hit_test(TEXT_LEFT + 1.0, 400.0).is_none());

        // Only a matching `Present` brings it back.
        let matching = panel_frame_of(3, second_total.cols.max(1), second_epoch, 2);
        assert!(state.apply_panel_payload(PanelFramePayload::Present(matching)));
        assert!(state.panel.presented().is_some());
    }

    /// A2B-1's frontend half — exhaustion latches, and the latch survives a
    /// retained `Present` whose epoch still matches.
    #[test]
    fn an_exhausted_frontend_latches_and_no_retained_frame_revives_the_band() {
        let Some(mut state) = headless_or_skip(800, 600, "alpha") else {
            return;
        };
        state.set_panel_wire(PANEL_MIN_VERSION);
        state.panel.geometry_epoch = u64::MAX;
        state.panel.declared = Some(CellSize::new(1, 1));
        let stale = panel_frame_of(3, 8, u64::MAX, 1);
        assert!(state.apply_panel_payload(PanelFramePayload::Present(stale.clone())));
        assert!(
            state.panel.presented().is_some(),
            "before exhaustion the matching frame is presentable"
        );

        assert_eq!(
            state.next_geometry_declaration(GeometryTrigger::Metrics),
            None,
            "checked allocation refuses to wrap"
        );
        assert!(state.panel.exhausted, "and latches for the session");
        assert!(state.panel.frame.is_none());
        assert_eq!(state.band_inset(), PanelBandInset::ABSENT);

        // An old matching `Present` must not resurrect the band, and no
        // further declaration is sent however the surface changes.
        assert!(!state.apply_panel_payload(PanelFramePayload::Present(stale)));
        assert!(state.panel.presented().is_none());
        assert_eq!(
            state.next_geometry_declaration(GeometryTrigger::Surface),
            None
        );
    }

    /// `Absent` is authoritative; silence retains; an invalid frame is
    /// rejected whole; a duplicate does no work.
    #[test]
    fn absent_clears_while_silence_retains_and_a_bad_frame_keeps_the_old_one() {
        let Some(mut state) = headless_or_skip(800, 600, "alpha") else {
            return;
        };
        let good = present_panel(&mut state, 3);
        assert!(
            !state.apply_panel_payload(PanelFramePayload::Present(good.clone())),
            "a duplicate valid frame does no work"
        );
        assert!(state.panel.presented().is_some(), "silence retains");

        // A frame whose cell count disagrees with its declared area.
        let mut bad = good.clone();
        bad.cells.pop();
        assert!(!state.apply_panel_payload(PanelFramePayload::Present(bad)));
        assert_eq!(
            state.panel.frame.as_ref(),
            Some(&good),
            "rejection is atomic: the previous valid frame is retained"
        );

        let declared_before = state.panel.declared;
        assert!(state.apply_panel_payload(PanelFramePayload::Absent));
        assert!(state.panel.presented().is_none());
        assert_eq!(state.band_inset(), PanelBandInset::ABSENT);
        assert_eq!(
            state.panel.declared, declared_before,
            "an Absent panel does not invalidate the frame-capacity declaration"
        );
        assert!(
            !state.apply_panel_payload(PanelFramePayload::Absent),
            "a duplicate Absent does no work either"
        );
    }

    /// Criterion 48 / 47 — the band hit-tests to cells, the divider strip is
    /// the painted rect, and a drag names ROWS.
    #[test]
    fn the_band_hit_tests_cells_and_the_divider_strip_drags_rows() {
        let Some(mut state) = headless_or_skip(800, 600, "alpha") else {
            return;
        };
        let frame = present_panel(&mut state, 4);
        let (ox, oy, w, h) = state
            .panel_content_rect()
            .expect("a presented band has a rect");

        // Cells inside; nothing outside.
        assert_eq!(
            state.panel_hit_test(ox + 0.5, oy + 0.5),
            Some(pmacs_protocol::CellCoord::new(0, 0))
        );
        assert!(state.panel_hit_test(ox - 1.0, oy + 0.5).is_none());
        assert!(
            state.panel_hit_test(ox + 0.5, oy - 1.0).is_none(),
            "a pixel above the band belongs to the document"
        );
        assert!(state.panel_hit_test(ox + 0.5, oy + h + 1.0).is_none());
        assert!(state.panel_hit_test(ox + w + 1.0, oy + 0.5).is_none());

        // The divider strip sits directly above the cells, and its painted
        // rect IS its hit rect.
        let (dx, dy, dw, dh) = state.panel_divider_rect().expect("divider rect");
        assert!((dy + dh - oy).abs() < f32::EPSILON, "strip abuts the cells");
        assert!((dh - state.fm.divider_height()).abs() < f32::EPSILON);
        assert!(state.panel_divider_contains(dx + dw / 2.0, dy + dh / 2.0));
        assert!(!state.panel_divider_contains(dx + dw / 2.0, dy - 1.0));
        assert!(!state.panel_divider_contains(dx + dw / 2.0, dy + dh));

        // A drag UP grows the panel; the request names rows and repeats are
        // suppressed.
        assert!(state.begin_panel_drag(dx + 1.0, dy + 1.0));
        let line = state.fm.code_line_height();
        let request = state
            .panel_drag_request(dy + 1.0 - 2.0 * line)
            .expect("two lines up is a two-row request");
        assert_eq!(request.rows, frame.size.rows + 2);
        assert_eq!(request.geometry_epoch, frame.geometry_epoch);
        assert_eq!(request.panel_epoch, frame.panel_epoch);
        state.note_panel_drag_sent(request.rows);
        assert_eq!(
            state.panel_drag_request(dy + 1.0 - 2.0 * line),
            None,
            "re-crossing the same row boundary does not re-send"
        );
        assert!(state.end_panel_drag());
        assert_eq!(state.panel_drag_request(dy + 1.0 - 3.0 * line), None);
    }

    /// A drag whose presentation has been replaced under it is dropped, not
    /// applied to the successor.
    #[test]
    fn a_drag_that_outlives_its_panel_is_dropped() {
        let Some(mut state) = headless_or_skip(800, 600, "alpha") else {
            return;
        };
        let frame = present_panel(&mut state, 4);
        let (dx, dy, _, dh) = state.panel_divider_rect().expect("divider rect");
        assert!(state.begin_panel_drag(dx + 1.0, dy + dh / 2.0));

        // A new presentation of the SAME buffer under a new panel epoch.
        let replaced = PanelFrame {
            panel_epoch: frame.panel_epoch + 1,
            ..frame.clone()
        };
        assert!(state.apply_panel_payload(PanelFramePayload::Present(replaced)));
        assert_eq!(
            state.panel_drag_request(dy - 3.0 * state.fm.code_line_height()),
            None,
            "the gesture addressed a presentation that is gone"
        );
        assert!(
            state.panel.drag.is_none(),
            "and the stale drag is discarded rather than left armed"
        );
    }

    /// The divider hover bit drives the `RowResize` icon and reports only on
    /// change, so the cursor is not reset on every pixel of motion.
    #[test]
    fn divider_hover_reports_only_on_change() {
        let Some(mut state) = headless_or_skip(800, 600, "alpha") else {
            return;
        };
        present_panel(&mut state, 3);
        assert!(state.set_panel_divider_hover(true));
        assert!(!state.set_panel_divider_hover(true));
        assert!(state.set_panel_divider_hover(false));
        assert!(!state.set_panel_divider_hover(false));
    }

    /// A session that did not negotiate the panel wire declares nothing and
    /// can never present a band.
    #[test]
    fn a_pre_panel_session_declares_no_geometry() {
        let Some(mut state) = headless_or_skip(800, 600, "alpha") else {
            return;
        };
        state.set_panel_wire(PANEL_MIN_VERSION - 1);
        assert_eq!(
            state.next_geometry_declaration(GeometryTrigger::Surface),
            None
        );
        assert_eq!(state.panel.geometry_epoch, 0, "0 means never declared");
        assert_eq!(state.band_inset(), PanelBandInset::ABSENT);
    }

    /// F1 — every pointer handler routes through ONE classifier, and the band
    /// claims the pixels it owns.
    ///
    /// This is the pin that makes the partial port visible: three of the four
    /// handlers used to decide for themselves and simply did not ask about the
    /// band, so a right-click and a wheel tick fell through to the document
    /// underneath. Pinning the classifier pins all four, because all four now
    /// have no other way to ask.
    #[test]
    fn one_classifier_decides_which_surface_a_pointer_pixel_belongs_to() {
        let Some(mut state) = headless_or_skip(800, 600, "alpha") else {
            return;
        };
        assert_eq!(
            state.classify_pointer_surface(10.0, 10.0),
            PointerSurface::Elsewhere,
            "with no band, nothing is the band's"
        );
        let frame = present_panel(&mut state, 4);
        let (ox, oy, _, h) = state
            .panel_content_rect()
            .expect("a presented band has a rect");
        let (dx, dy, _, dh) = state.panel_divider_rect().expect("divider rect");

        assert_eq!(
            state.classify_pointer_surface(dx + 1.0, dy + dh / 2.0),
            PointerSurface::PanelDivider,
            "the strip is a drag handle, tested before the cells it sits above"
        );
        assert_eq!(
            state.classify_pointer_surface(ox + 0.5, oy + 0.5),
            PointerSurface::PanelCell(pmacs_protocol::CellCoord::new(0, 0))
        );
        // One pixel above the CELLS is still the divider; the document starts
        // above the strip. Getting that boundary wrong in either direction is
        // how a resize gesture and a selection gesture steal each other.
        assert_eq!(
            state.classify_pointer_surface(ox + 0.5, oy - 1.0),
            PointerSurface::PanelDivider
        );
        assert_eq!(
            state.classify_pointer_surface(ox + 0.5, dy - 1.0),
            PointerSurface::Elsewhere,
            "a pixel above the divider strip belongs to the document"
        );
        assert_eq!(
            state.classify_pointer_surface(ox + 0.5, oy + h + 1.0),
            PointerSurface::Elsewhere,
            "and one below it belongs to the status band"
        );
        // The fractional right-edge remainder is the band's, but maps to no
        // cell — so it emits no `PanelPointer` while still not falling through
        // to the document.
        let advance = state.panel_cell_advance().expect("declared advance");
        let past_last_column = frame.size.cols as f32 * advance + 0.5;
        if past_last_column < state.config.width as f32 {
            assert_eq!(
                state.classify_pointer_surface(past_last_column, oy + 0.5),
                PointerSurface::PanelBackground,
                "the remainder is band background, not a cell and not the document"
            );
        }
    }

    /// F1 — a held left button makes motion a `Drag(Left)`, and the dedupe
    /// re-arms on every press and release.
    #[test]
    fn a_held_button_makes_panel_motion_a_drag_and_a_release_lands_outside() {
        let Some(mut state) = headless_or_skip(800, 600, "alpha") else {
            return;
        };
        present_panel(&mut state, 4);
        let (ox, oy, _, _) = state
            .panel_content_rect()
            .expect("a presented band has a rect");

        assert_eq!(
            state.panel_motion_kind(),
            ProtocolMouseKind::Move,
            "bare motion neither focuses the panel nor claims a controller"
        );
        state.set_panel_pointer_held(true);
        assert_eq!(
            state.panel_motion_kind(),
            ProtocolMouseKind::Drag(ProtocolMouseButton::Left),
            "a held button is a drag — without this, panel selection is silent"
        );

        // Sub-cell motion says nothing new; a new cell does.
        let first = pmacs_protocol::CellCoord::new(0, 0);
        assert!(state.panel_motion_is_new(first));
        assert!(!state.panel_motion_is_new(first));
        assert!(state.panel_motion_is_new(pmacs_protocol::CellCoord::new(0, 1)));
        // A press or release re-arms it: the first drag after a press must
        // reach the daemon even at the cell the press landed on.
        state.set_panel_pointer_held(true);
        assert!(state.panel_motion_is_new(pmacs_protocol::CellCoord::new(0, 1)));

        // A release inside the band lands on the cell under the pointer; one
        // outside still ends the gesture, at the last cell reported.
        assert_eq!(
            state.panel_release_cell(ox + 0.5, oy + 0.5),
            Some(pmacs_protocol::CellCoord::new(0, 0))
        );
        state.panel_motion_is_new(pmacs_protocol::CellCoord::new(1, 7));
        assert_eq!(
            state.panel_release_cell(ox + 0.5, 0.0),
            Some(pmacs_protocol::CellCoord::new(1, 7)),
            "a selection drag routinely ends past the band's edge, and dropping \
             that release leaves the daemon holding a button down forever"
        );
    }

    /// F3 — only a FOCUSED panel paints its caret.
    ///
    /// The producer ships `cursor` for a passive panel too (it is the window's
    /// real point and the daemon does not suppress it), so painting it
    /// unconditionally puts a second insertion caret on screen.
    #[test]
    fn a_passive_panel_paints_no_caret() {
        let Some(mut state) = headless_or_skip(800, 600, "alpha") else {
            return;
        };
        let mut frame = present_panel(&mut state, 3);
        frame.cursor = Some(pmacs_protocol::CellCoord::new(1, 2));

        frame.focused = true;
        frame.panel_epoch += 1;
        assert!(state.apply_panel_payload(PanelFramePayload::Present(frame.clone())));
        let focused_quads = state.panel_quad_vertex_bytes();

        frame.focused = false;
        frame.panel_epoch += 1;
        assert!(state.apply_panel_payload(PanelFramePayload::Present(frame.clone())));
        let passive_quads = state.panel_quad_vertex_bytes();

        assert!(
            focused_quads.len() > passive_quads.len(),
            "the focused band must paint one more quad — its caret — than the \
             passive one: {} vs {}",
            focused_quads.len(),
            passive_quads.len()
        );
        // And the difference is exactly the caret: a cursor-free focused frame
        // matches the passive one.
        frame.focused = true;
        frame.cursor = None;
        frame.panel_epoch += 1;
        assert!(state.apply_panel_payload(PanelFramePayload::Present(frame)));
        assert_eq!(
            state.panel_quad_vertex_bytes().len(),
            passive_quads.len(),
            "with no cursor at all, focused and passive paint the same quads"
        );
    }

    /// F5 — the band's underlines are rendered, split across the two pipelines
    /// exactly as the terminal path splits them.
    #[test]
    fn panel_underlines_reach_both_the_quad_and_squiggle_pipelines() {
        let Some(mut state) = headless_or_skip(800, 600, "alpha") else {
            return;
        };
        let plain = present_panel(&mut state, 3);
        let plain_quads = state.panel_quad_vertex_bytes().len();
        assert!(
            state.panel_squiggle_vertex_bytes().is_empty(),
            "a plain band has no curly underlines"
        );

        let underlined = |style: UnderlineStyle, epoch: u64| {
            let mut frame = plain.clone();
            frame.panel_epoch = epoch;
            for cell in &mut frame.cells {
                cell.style.underline = style;
                cell.style.underline_color = CellColor::Rgb(200, 40, 40);
            }
            frame
        };

        // A straight form rides the quad batch.
        assert!(
            state.apply_panel_payload(PanelFramePayload::Present(underlined(
                UnderlineStyle::Single,
                plain.panel_epoch + 1
            )))
        );
        assert!(
            state.panel_quad_vertex_bytes().len() > plain_quads,
            "straight underlines must reach the quad batch"
        );
        assert!(
            state.panel_squiggle_vertex_bytes().is_empty(),
            "and not the squiggle one"
        );

        // Curly rides the squiggle pipeline, which owns the sine wave.
        assert!(
            state.apply_panel_payload(PanelFramePayload::Present(underlined(
                UnderlineStyle::Curly,
                plain.panel_epoch + 2
            )))
        );
        assert!(
            !state.panel_squiggle_vertex_bytes().is_empty(),
            "curly underlines must reach the squiggle pipeline"
        );
    }

    /// A disconnect tears the band down, declaration and all.
    ///
    /// A retained band is the daemon's projection of a window nobody is
    /// updating; leaving it beside a disconnect notice is the same frozen,
    /// live-looking surface the terminal arm already refuses.
    #[test]
    fn a_disconnect_clears_the_band_and_its_declaration() {
        let Some(mut state) = headless_or_skip(800, 600, "alpha") else {
            return;
        };
        present_panel(&mut state, 3);
        assert!(state.panel.presented().is_some());
        state.on_daemon_disconnected("(daemon disconnected)");
        assert!(state.panel.presented().is_none());
        assert_eq!(state.panel.geometry_epoch, 0, "0 means never declared");
        assert_eq!(state.panel.declared_advance, None);
        assert_eq!(state.band_inset(), PanelBandInset::ABSENT);
        assert!(state.panel_text_buffers.is_empty());
    }

    /// Criterion 46 — the band and divider really do paint, they take their
    /// pixels from the document, and the status band is untouched.
    ///
    /// Asserts CONTENT PRODUCED, not merely an invariant preserved. The
    /// first version of this test only checked that no pixel above the band
    /// changed and none below it did — and it passed with the band painting
    /// nothing at all, because installing a panel reshapes the document to
    /// the smaller height and *that* produced the whole diff. So the band's
    /// own rows and the divider row are now counted directly.
    #[test]
    fn the_painted_band_takes_pixels_from_the_document_and_not_the_status_band() {
        let Some(mut state) = headless_or_skip(400, 300, "alpha\nbeta\ngamma\ndelta\n") else {
            return;
        };
        let before = state.render_offscreen();
        let fm = state.fm;
        let width = state.config.width;
        let height = state.config.height;
        // Anchored independently of the boundary helpers, so a blanket
        // rewrite of them cannot move this reference frame with them.
        let status_top = (height as f32 - fm.status_band_height()).floor() as u32;

        present_panel(&mut state, 3);
        state.sync_buffer_dimensions();
        let after = state.render_offscreen();
        assert_eq!(before.len(), after.len());

        let differing_rows = |y0: u32, y1: u32| -> usize {
            let mut count = 0;
            for y in y0..y1.min(height) {
                for x in 0..width {
                    let i = ((y * width + x) * 4) as usize;
                    if before[i..i + 4] != after[i..i + 4] {
                        count += 1;
                    }
                }
            }
            count
        };

        let band = state.band_inset();
        let (_, cells_top, _, cells_h) = state
            .panel_content_rect()
            .expect("a presented band has a rect");
        let (_, divider_top, _, divider_h) = state
            .panel_divider_rect()
            .expect("a presented band has a divider");

        assert!(
            differing_rows(
                divider_top.floor() as u32,
                (divider_top + divider_h).ceil() as u32
            ) > 0,
            "the divider strip must actually paint"
        );
        assert!(
            differing_rows(
                cells_top.floor() as u32,
                (cells_top + cells_h).ceil() as u32
            ) > 0,
            "the band's cells must actually paint"
        );
        assert_eq!(
            differing_rows(status_top, height),
            0,
            "and the status band stays pixel-identical at the physical window bottom"
        );
        assert_eq!(
            differing_rows(0, document_text_bottom(height, fm, band).floor() as u32),
            0,
            "no pixel above the band's own top may change: the document keeps \
             the area it still has"
        );
    }

    fn terminal_cell(glyph: pmacs_protocol::Glyph, style: CellStyle) -> pmacs_protocol::Cell {
        pmacs_protocol::Cell {
            glyph,
            style,
            attachment: None,
        }
    }

    fn terminal_frame_of(
        buffer_id: BufferId,
        rows: u32,
        cols: u32,
        cells: Vec<pmacs_protocol::Cell>,
    ) -> TerminalFrame {
        TerminalFrame {
            buffer_id,
            size: CellSize::new(rows, cols),
            cells,
            cursor: None,
            title: Some("sh".into()),
            screen_generation: 1,
            selection: Vec::new(),
            scroll_offset: 0,
            at_bottom: true,
            pid: 4242,
            process: pmacs_protocol::TerminalProcessState::Running,
        }
    }

    fn plain_terminal_frame(buffer_id: BufferId, text: &str, cols: u32) -> TerminalFrame {
        let mut cells: Vec<pmacs_protocol::Cell> = text
            .chars()
            .map(|ch| terminal_cell(pmacs_protocol::Glyph::Char(ch), CellStyle::default()))
            .collect();
        while cells.len() < cols as usize {
            cells.push(terminal_cell(
                pmacs_protocol::Glyph::Char(' '),
                CellStyle::default(),
            ));
        }
        cells.truncate(cols as usize);
        terminal_frame_of(buffer_id, 1, cols, cells)
    }

    /// Acceptance 35: a snapshot leaves terminal mode; a valid matching
    /// frame enters it; a stale-buffer frame is ignored; a duplicate
    /// valid frame rebuilds nothing.
    #[test]
    fn a35_terminal_mode_transitions_are_explicit_and_duplicates_do_no_work() {
        let Some(mut state) = headless_or_skip(400, 300, "document text") else {
            return;
        };
        let terminal_buffer = BufferId::next();
        let other_buffer = BufferId::next();
        state.current_buffer_id = Some(terminal_buffer);

        // A frame for a buffer this window is not showing changes nothing.
        state.apply_terminal_frame(plain_terminal_frame(other_buffer, "nope", 8));
        assert!(
            state.terminal.is_none(),
            "a stale-buffer frame must not enter terminal mode"
        );

        let frame = plain_terminal_frame(terminal_buffer, "hello", 8);
        state.apply_terminal_frame(frame.clone());
        let terminal = state.terminal.as_ref().expect("terminal mode entered");
        assert_eq!(terminal.buffer_id, terminal_buffer);
        assert_eq!(terminal.plan.size, CellSize::new(1, 8));
        let buffers_after_first = state.terminal_text_buffers.len();
        assert!(buffers_after_first > 0, "runs were shaped");

        // A byte-identical frame is retained without a rebuild.
        state.terminal_text_buffers.clear();
        state.apply_terminal_frame(frame);
        assert!(
            state.terminal_text_buffers.is_empty(),
            "a duplicate valid frame must not rebuild shaping"
        );

        // A changed frame does rebuild.
        state.apply_terminal_frame(plain_terminal_frame(terminal_buffer, "world", 8));
        assert!(!state.terminal_text_buffers.is_empty());

        // Leaving terminal mode drops every terminal-only cache.
        state.exit_terminal_mode();
        assert!(state.terminal.is_none());
        assert!(state.terminal_text_buffers.is_empty());
        assert!(state.last_terminal_size_sent.is_none());
    }

    /// Acceptance 29 (frontend half): an invalid frame is rejected whole
    /// and the previous valid frame survives.
    #[test]
    fn a29_invalid_terminal_frame_retains_the_previous_valid_one() {
        let Some(mut state) = headless_or_skip(400, 300, "document text") else {
            return;
        };
        let terminal_buffer = BufferId::next();
        state.current_buffer_id = Some(terminal_buffer);
        let good = plain_terminal_frame(terminal_buffer, "good", 8);
        state.apply_terminal_frame(good.clone());

        // Cell count disagreeing with the declared area.
        let mut short = plain_terminal_frame(terminal_buffer, "bad", 8);
        short.cells.pop();
        state.apply_terminal_frame(short);
        assert_eq!(
            state.terminal.as_ref().expect("still terminal").frame,
            good,
            "an invalid frame must not partially apply"
        );

        // An orphan continuation.
        let mut orphan = plain_terminal_frame(terminal_buffer, "bad", 8);
        orphan.cells[0] = terminal_cell(pmacs_protocol::Glyph::Continuation, CellStyle::default());
        state.apply_terminal_frame(orphan);
        assert_eq!(state.terminal.as_ref().expect("still terminal").frame, good);

        // An attachment in a terminal cell.
        let mut attached = plain_terminal_frame(terminal_buffer, "bad", 8);
        attached.cells[1].attachment = Some(pmacs_protocol::Attachment::ImageCell {
            image_id: 1,
            sub_x: 0,
            sub_y: 0,
        });
        state.apply_terminal_frame(attached);
        assert_eq!(state.terminal.as_ref().expect("still terminal").frame, good);

        // A later valid frame still lands, so the latch is not a wedge.
        let next = plain_terminal_frame(terminal_buffer, "next", 8);
        state.apply_terminal_frame(next.clone());
        assert_eq!(state.terminal.as_ref().expect("terminal").frame, next);
    }

    /// Acceptance 35: geometry declaration emits exactly one changed
    /// size and suppresses identical ones.
    #[test]
    fn a35_cell_declaration_emits_once_per_change() {
        let Some(mut state) = headless_or_skip(400, 300, "document text") else {
            return;
        };
        let buffer_id = BufferId::next();
        state.current_buffer_id = Some(buffer_id);

        let first = state
            .terminal_declaration_if_changed()
            .expect("a 400x300 window admits a cell grid");
        assert_eq!(first.0, buffer_id);
        assert!(first.1.rows >= 1 && first.1.cols >= 1);

        // Review round 1, finding 4: the query does not record. Until
        // the caller confirms the send, the declaration stays PENDING,
        // so a failed write is retried rather than suppressed as
        // already-declared.
        assert_eq!(
            state.terminal_declaration_if_changed(),
            Some(first),
            "an unsent declaration must still be offered"
        );
        state.note_terminal_declaration_sent(first.0, first.1);
        assert!(
            state.terminal_declaration_if_changed().is_none(),
            "an unchanged size must be silent once it has been sent"
        );

        // A real geometry change re-declares exactly once.
        state.resize(600, 300);
        let widened = state
            .terminal_declaration_if_changed()
            .expect("a wider window is a new size");
        assert!(widened.1.cols > first.1.cols);
        state.note_terminal_declaration_sent(widened.0, widened.1);
        assert!(state.terminal_declaration_if_changed().is_none());

        // A buffer switch forces a fresh declaration even at the same size.
        state.exit_terminal_mode();
        let after_switch = state
            .terminal_declaration_if_changed()
            .expect("a snapshot forces re-declaration");
        assert_eq!(after_switch.1, widened.1);
    }

    /// Acceptance 34: pixel hit testing yields only in-bounds cells and
    /// never leaves the terminal rectangle.
    #[test]
    fn a34_terminal_hit_testing_stays_inside_the_declared_grid() {
        let Some(mut state) = headless_or_skip(400, 300, "document text") else {
            return;
        };
        let buffer_id = BufferId::next();
        state.current_buffer_id = Some(buffer_id);
        state.apply_terminal_frame(plain_terminal_frame(buffer_id, "abcd", 4));
        let advance = f64::from(state.mono_advance());
        let line = f64::from(state.fm.code_line_height());
        let size = state.terminal.as_ref().expect("terminal").plan.size;

        let origin = (f64::from(TEXT_LEFT), f64::from(TEXT_TOP));
        let hit = |x: f64, y: f64| {
            crate::terminal::hit_test_cell(
                x as f32,
                y as f32,
                (TEXT_LEFT, TEXT_TOP),
                state.mono_advance(),
                state.fm.code_line_height(),
                size,
            )
        };
        assert_eq!(hit(origin.0, origin.1), Some(CellCoord::new(0, 0)));
        assert_eq!(
            hit(origin.0 + advance * 2.5, origin.1 + line * 0.5),
            Some(CellCoord::new(0, 2))
        );
        // Past the last declared column, and above the grid origin (the
        // window chrome), are not terminal hits.
        assert_eq!(
            hit(origin.0 + advance * f64::from(size.cols), origin.1),
            None
        );
        assert_eq!(hit(origin.0, origin.1 - 1.0), None);
        // The status band is below the grid and never hits.
        assert_eq!(
            hit(
                origin.0,
                f64::from(status_band_top(state.config.height, state.fm)) + 2.0
            ),
            None
        );
    }

    /// Acceptance 33: a real headless frame paints terminal cells and
    /// drops every document layer.
    #[test]
    fn a33_headless_terminal_frame_paints_cells_without_document_layers() {
        let text = "alpha\nbeta\ngamma\ndelta\n";
        let Some(mut document) = headless_or_skip(400, 300, text) else {
            return;
        };
        let buffer_id = BufferId::next();
        document.current_buffer_id = Some(buffer_id);
        document.view_range = (0, text.len() as u64);
        document.line_numbers = LineNumberMode::Absolute;
        let document_px = document.render_offscreen();

        let mut terminal = State::new_headless(400, 300, text).expect("adapter was just available");
        terminal.current_buffer_id = Some(buffer_id);
        terminal.view_range = (0, text.len() as u64);
        terminal.line_numbers = LineNumberMode::Absolute;
        // A red-on-blue row: both colors are far from the defaults, so
        // their presence is unambiguous evidence the cells painted.
        let styled = CellStyle {
            fg: CellColor::Rgb(255, 0, 0),
            bg: CellColor::Rgb(0, 0, 255),
            ..CellStyle::default()
        };
        let cells: Vec<pmacs_protocol::Cell> = "TERMINAL"
            .chars()
            .map(|ch| terminal_cell(pmacs_protocol::Glyph::Char(ch), styled))
            .collect();
        let mut frame = terminal_frame_of(buffer_id, 1, cells.len() as u32, cells);
        frame.cursor = Some(CellCoord::new(0, 0));
        assert_eq!(frame.validate(), Ok(()));
        terminal.apply_terminal_frame(frame);
        let terminal_px = terminal.render_offscreen();

        assert_eq!(document_px.len(), terminal_px.len());
        let differing = document_px
            .iter()
            .zip(&terminal_px)
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            differing > 1000,
            "terminal mode must repaint the code area ({differing} bytes differ)"
        );

        // The blue cell background must actually be on screen.
        let blue = terminal_px
            .chunks_exact(4)
            .filter(|px| px[2] > 100 && px[0] < 90 && px[1] < 90)
            .count();
        assert!(
            blue > 100,
            "the terminal cell background did not paint ({blue} blue pixels)"
        );

        // Document overlays are suppressed. The minimap is the
        // unambiguous probe: it occupies a band no terminal cell can
        // reach, and it paints only when a file summary is present.
        // The comparison is DIFFERENTIAL rather than a brightness
        // threshold — the window background is itself a nonzero sRGB
        // value, so "is there ink here" is not a question absolute
        // pixel levels can answer.
        let summary = FileStyleSummaryState {
            generation: 1,
            lines: vec![
                CellStyle {
                    fg: CellColor::Rgb(255, 255, 255),
                    ..CellStyle::default()
                };
                40
            ],
        };
        let band_left = minimap_left(400).expect("a 400px window has a minimap band") as u32;
        let code_bottom = document_text_bottom(300, document.fm, document.band_inset()) as u32;
        let band_differs = |a: &[u8], b: &[u8]| {
            let mut count = 0usize;
            for row in 0..code_bottom {
                for col in band_left..400 {
                    let i = ((row * 400 + col) * 4) as usize;
                    if a[i..i + 4] != b[i..i + 4] {
                        count += 1;
                    }
                }
            }
            count
        };

        document.current_summary = Some(summary.clone());
        document.current_line_shapes = minimap_line_shapes(text);
        document.minimap_cache = None;
        let document_minimap_px = document.render_offscreen();
        assert!(
            band_differs(&document_px, &document_minimap_px) > 0,
            "the probe is vacuous unless a summary makes the document frame's \
             minimap band change"
        );

        terminal.current_summary = Some(summary);
        terminal.current_line_shapes = minimap_line_shapes(text);
        terminal.minimap_cache = None;
        let terminal_minimap_px = terminal.render_offscreen();
        assert_eq!(
            band_differs(&terminal_px, &terminal_minimap_px),
            0,
            "terminal mode must not paint the document minimap"
        );
    }

    /// Acceptance 33: reverse video, selection, and the cursor each
    /// change the rendered frame, and a continuation paints no glyph of
    /// its own.
    #[test]
    fn a33_reverse_selection_and_cursor_each_change_the_frame() {
        let Some(mut base) = headless_or_skip(400, 300, "x") else {
            return;
        };
        let buffer_id = BufferId::next();
        let build = |state: &mut State, mutate: &dyn Fn(&mut TerminalFrame)| -> Vec<u8> {
            state.current_buffer_id = Some(buffer_id);
            state.exit_terminal_mode();
            state.current_buffer_id = Some(buffer_id);
            let mut frame = plain_terminal_frame(buffer_id, "SAMPLE", 12);
            mutate(&mut frame);
            assert_eq!(frame.validate(), Ok(()));
            state.apply_terminal_frame(frame);
            state.render_offscreen()
        };

        let plain = build(&mut base, &|_| {});
        let reversed = build(&mut base, &|frame| {
            for cell in &mut frame.cells {
                cell.style.reverse = true;
            }
        });
        let selected = build(&mut base, &|frame| {
            frame.selection = vec![pmacs_protocol::TerminalSelectionSpan {
                row: 0,
                start_col: 0,
                end_col: 6,
            }];
        });
        let with_cursor = build(&mut base, &|frame| {
            frame.cursor = Some(CellCoord::new(0, 3));
        });

        let differs = |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|(x, y)| x != y).count();
        assert!(
            differs(&plain, &reversed) > 500,
            "reverse video must repaint the run"
        );
        assert!(
            differs(&plain, &selected) > 200,
            "a selection wash must reach the frame"
        );
        assert!(
            differs(&plain, &with_cursor) > 50,
            "the cursor block must reach the frame"
        );

        // A wide lead plus its continuation paints one glyph across two
        // cells — the continuation contributes no run of its own.
        base.exit_terminal_mode();
        base.current_buffer_id = Some(buffer_id);
        let mut cells = vec![
            terminal_cell(
                pmacs_protocol::Glyph::Char('\u{4e00}'),
                CellStyle::default(),
            ),
            terminal_cell(pmacs_protocol::Glyph::Continuation, CellStyle::default()),
        ];
        cells.resize(
            4,
            terminal_cell(pmacs_protocol::Glyph::Char(' '), CellStyle::default()),
        );
        let frame = terminal_frame_of(buffer_id, 1, 4, cells);
        assert_eq!(frame.validate(), Ok(()));
        base.apply_terminal_frame(frame);
        let plan = &base.terminal.as_ref().expect("terminal").plan;
        let wide = plan.runs.first().expect("a wide run");
        assert_eq!(wide.cells, 2, "the wide lead owns both columns");
        assert!(
            plan.runs.iter().all(|run| run.col != 1),
            "the continuation contributes no run"
        );
    }

    /// Review round 1, finding 3: terminal motion reports only on a cell
    /// change.
    ///
    /// A unit test on the memo rather than on the wire: the send site
    /// lives in `window_event`, which needs a real winit event and a
    /// live attach client. This cannot bite against the pre-fix tree —
    /// the seam it calls did not exist there — so it is a contract pin,
    /// not fix evidence.
    #[test]
    fn terminal_motion_reports_once_per_cell_and_rearms_on_press() {
        let Some(mut state) = headless_or_skip(400, 300, "doc") else {
            return;
        };
        let buffer_id = BufferId::next();
        state.current_buffer_id = Some(buffer_id);
        state.apply_terminal_frame(plain_terminal_frame(buffer_id, "abcd", 8));

        let cell = CellCoord::new(0, 2);
        assert!(state.terminal_motion_is_new(cell), "first sight of a cell");
        assert!(
            !state.terminal_motion_is_new(cell),
            "sub-cell motion inside one cell is not new information"
        );
        assert!(
            state.terminal_motion_is_new(CellCoord::new(0, 3)),
            "crossing into another cell reports"
        );

        // A press/release re-arms the memo, so the first drag after a
        // press reaches the daemon even at the press cell.
        let press_cell = CellCoord::new(1, 1);
        assert!(state.terminal_motion_is_new(press_cell));
        state.last_terminal_pointer_cell = None;
        assert!(
            state.terminal_motion_is_new(press_cell),
            "a press re-arms the memo at its own cell"
        );

        // Leaving terminal mode drops it too: the next terminal's cells
        // are a different grid entirely.
        state.exit_terminal_mode();
        assert!(state.last_terminal_pointer_cell.is_none());
    }

    /// Review round 2, finding 1: a disconnect must leave terminal mode,
    /// or the notice is invisible.
    ///
    /// Terminal mode prepares no document code layer while the terminal
    /// glyph layer keeps painting its last frame, so a notice written
    /// without leaving terminal mode never reaches the screen and the
    /// user sees a frozen terminal that ignores input.
    #[test]
    fn a_disconnect_leaves_terminal_mode_so_the_notice_is_visible() {
        let Some(mut state) = headless_or_skip(400, 300, "document text") else {
            return;
        };
        let buffer_id = BufferId::next();
        state.current_buffer_id = Some(buffer_id);
        state.apply_terminal_frame(plain_terminal_frame(buffer_id, "live", 8));
        assert!(
            state.terminal.is_some(),
            "the probe starts in terminal mode"
        );
        let terminal_px = state.render_offscreen();

        state.on_daemon_disconnected("(daemon disconnected)");
        assert!(
            state.terminal.is_none(),
            "a disconnect must leave terminal mode"
        );
        assert!(
            state.terminal_text_buffers.is_empty(),
            "terminal-only caches go with it"
        );
        assert_eq!(state.current_text, "(daemon disconnected)");

        // The notice actually reaches the screen: the frame differs from
        // the terminal frame it replaced, which is the whole point of
        // the fix.
        let notice_px = state.render_offscreen();
        let differing = terminal_px
            .iter()
            .zip(&notice_px)
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            differing > 500,
            "the disconnect notice must repaint over the frozen terminal \
             ({differing} bytes differ)"
        );
    }

    /// Acceptance 36: the terminal statusline metadata reaches the band
    /// as text, never as a host-title or control effect.
    #[test]
    fn a36_terminal_title_stays_sanitized_statusline_metadata() {
        let Some(mut state) = headless_or_skip(400, 300, "doc") else {
            return;
        };
        let buffer_id = BufferId::next();
        state.current_buffer_id = Some(buffer_id);
        let mut frame = plain_terminal_frame(buffer_id, "sh", 8);
        frame.title = Some("build: cargo test".into());
        state.apply_terminal_frame(frame);
        // The frame retains the title verbatim for the provider to
        // render; nothing in the GPU turns it into a window-title or a
        // terminal control sequence.
        assert_eq!(
            state
                .terminal
                .as_ref()
                .expect("terminal")
                .frame
                .title
                .as_deref(),
            Some("build: cargo test")
        );
        // Control characters can never arrive here: validation rejects
        // them before the frame is installed.
        let mut hostile = plain_terminal_frame(buffer_id, "sh", 8);
        hostile.title = Some("\u{1b}]0;pwned\u{7}".into());
        assert!(hostile.validate().is_err());
    }
    #[test]
    fn gpu_cli_accepts_only_explicit_exact_modes() {
        let args = |values: &[&str]| values.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            parse_args(&args(&["--attach", "/tmp/pmacs.sock"])),
            Ok(Mode::Attach {
                socket: PathBuf::from("/tmp/pmacs.sock"),
            })
        );
        assert_eq!(
            parse_args(&args(&[
                "--managed-attach",
                "/tmp/pmacs.sock",
                "/bin/pmacs"
            ])),
            Ok(Mode::ManagedAttach {
                socket: PathBuf::from("/tmp/pmacs.sock"),
                daemon_executable: PathBuf::from("/bin/pmacs"),
                initial_target: None,
            })
        );
        assert_eq!(
            parse_args(&args(&[
                "--headless-probe",
                "/tmp/pmacs.sock",
                "/tmp/report"
            ])),
            Ok(Mode::HeadlessProbe {
                socket: PathBuf::from("/tmp/pmacs.sock"),
                report: PathBuf::from("/tmp/report"),
            })
        );
        assert_eq!(
            parse_args(&args(&[
                "--headless-managed-probe",
                "/tmp/pmacs.sock",
                "/tmp/report",
                "/bin/pmacs"
            ])),
            Ok(Mode::HeadlessManagedProbe {
                socket: PathBuf::from("/tmp/pmacs.sock"),
                report: PathBuf::from("/tmp/report"),
                daemon_executable: PathBuf::from("/bin/pmacs"),
                initial_target: None,
            })
        );
    }

    #[test]
    fn gpu_private_target_marker_preserves_raw_file_bytes() {
        use std::os::unix::ffi::OsStringExt;

        let raw_path = OsString::from_vec(vec![b'-', b'n', b'o', b't', b'e', 0xff]);
        let argv = vec![
            OsString::from("--managed-attach"),
            OsString::from("/tmp/pmacs.sock"),
            OsString::from("/bin/pmacs"),
            OsString::from("--initial-target"),
            OsString::from("/launcher"),
            raw_path.clone(),
        ];
        assert_eq!(
            parse_args(&argv),
            Ok(Mode::ManagedAttach {
                socket: PathBuf::from("/tmp/pmacs.sock"),
                daemon_executable: PathBuf::from("/bin/pmacs"),
                initial_target: Some(InitialTargetPaths {
                    cwd: PathBuf::from("/launcher"),
                    path: PathBuf::from(&raw_path),
                }),
            })
        );

        let bad_cwd = [
            OsString::from("--managed-attach"),
            OsString::from("/tmp/pmacs.sock"),
            OsString::from("/bin/pmacs"),
            OsString::from("--initial-target"),
            OsString::from("--cwd"),
            OsString::from("note"),
        ];
        assert!(
            parse_args(&bad_cwd)
                .expect_err("option-like cwd must fail")
                .contains("option-like")
        );
        let bad_option = [OsString::from_vec(vec![b'-', 0xff])];
        assert_eq!(
            parse_args(&bad_option).expect_err("non-UTF-8 option must fail"),
            "option names must be valid UTF-8"
        );
    }

    #[test]
    fn gpu_cli_rejects_bare_missing_and_trailing_arguments() {
        let invalid = [
            vec![],
            vec!["--attach"],
            vec!["--attach", "/tmp/pmacs.sock", "ignored"],
            vec!["--headless-probe", "/tmp/pmacs.sock"],
            vec![
                "--headless-probe",
                "/tmp/pmacs.sock",
                "/tmp/report",
                "ignored",
            ],
            vec!["--managed-attach", "/tmp/pmacs.sock"],
            vec!["--headless-managed-probe", "/tmp/pmacs.sock", "/tmp/report"],
            vec!["--attach", "--help"],
            vec!["--managed-attach", "/tmp/pmacs.sock", "--version"],
            vec!["--headless-probe", "/tmp/pmacs.sock", "--help"],
            vec![
                "--headless-managed-probe",
                "/tmp/pmacs.sock",
                "/tmp/report",
                "--version",
            ],
            vec!["research"],
        ];
        for values in invalid {
            let args = values.iter().map(OsString::from).collect::<Vec<_>>();
            assert!(
                parse_args(&args).is_err(),
                "accepted invalid argv: {values:?}"
            );
        }
        let error = parse_args(&[OsString::from("--attach"), OsString::from("--help")])
            .expect_err("option-like socket operand must fail");
        assert!(error.contains("option-like path operand --help"));
    }

    #[test]
    fn pre_state_app_events_are_buffered_in_arrival_order() {
        let mut pending = Vec::new();
        for reason in ["snapshot-predecessor", "snapshot-successor"] {
            let event = AppEvent::Attach(AttachEvent::Disconnected(reason.to_owned()));
            assert!(defer_app_event(false, &mut pending, event).is_none());
        }
        assert_eq!(pending.len(), 2);
        let reasons = pending
            .into_iter()
            .map(|event| match event {
                AppEvent::Attach(AttachEvent::Disconnected(reason)) => reason,
                AppEvent::Attach(AttachEvent::Message(_)) => panic!("unexpected message"),
            })
            .collect::<Vec<_>>();
        assert_eq!(reasons, ["snapshot-predecessor", "snapshot-successor"]);

        let immediate = AppEvent::Attach(AttachEvent::Disconnected("ready".to_owned()));
        assert!(defer_app_event(true, &mut Vec::new(), immediate).is_some());
    }

    #[test]
    fn gpu_cli_points_bare_invocation_to_broker_and_labels_direct_attach() {
        let bare = parse_args(&[]).expect_err("bare GPU invocation must fail");
        assert!(
            bare.contains("pmacs --gpu"),
            "unexpected bare error: {bare}"
        );
        assert!(GPU_USAGE.contains("NORMAL STARTUP"));
        assert!(GPU_USAGE.contains("ADVANCED DIRECT ATTACH"));

        let extra = ["--help", "extra"].map(OsString::from);
        let error = parse_args(&extra).expect_err("help operands must fail");
        assert_eq!(error, "--help does not accept operands");
    }

    // --- Horizontal scroll (QoL Stage 5, framing Q#G5) -------------------
    //
    // Every test below drives the offset through `ensure_caret_painted`,
    // the only thing that moves it (Q#G2 — automatic follow, no command
    // surface). Two of them are deliberately the exception: the wrap and
    // snapshot resets must be observed BEFORE any cursor motion, because
    // a motion afterwards repairs the offset anyway and a witness that
    // waits for one cannot tell "reset" from "repaired later" — which is
    // the bug.
    //
    // The gutter is on in all of them. With it off `code_clip_left` is 0
    // and every left-edge assertion below is vacuously true, so a suite
    // that forgot to turn it on would pass against an unclipped build.

    /// One long line, gutter on, wrapping off — the configuration the
    /// Stage 5 witnesses share. `truncate` is set through the real
    /// `apply_line_wrap` rather than by poking `buffer.set_wrap`, so the
    /// buffer never sits on cosmic-text's constructor default.
    fn truncating_state(width: u32, height: u32, text: &str) -> Option<(State, BufferId)> {
        let mut state = headless_or_skip(width, height, text)?;
        let bid = BufferId::next();
        state.current_buffer_id = Some(bid);
        state.line_numbers = LineNumberMode::Absolute;
        state.apply_line_wrap(bid, false);
        state.reshape();
        Some((state, bid))
    }

    /// Follow the caret to `byte`, returning the resulting offset.
    fn follow_to(state: &mut State, bid: BufferId, byte: u64) -> f32 {
        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte,
        });
        state.ensure_caret_painted();
        state.code_scroll_left
    }

    /// A line long enough that its end is far off the right edge of the
    /// windows used here.
    fn long_line() -> String {
        "abcdefghij".repeat(30)
    }

    /// Q#G5 — **the gutter is unchanged by scrolling.**
    ///
    /// Not "nothing paints left of the edge": with line numbers on, the
    /// gutter deliberately holds digit glyphs, so that assertion would
    /// fail on a correct build. The checkable form of the same intent is
    /// byte-identity of the gutter band across a scroll — a code painter
    /// bleeding leftward changes exactly those pixels.
    #[test]
    fn the_gutter_band_is_byte_identical_across_a_horizontal_scroll() {
        let text = long_line();
        let Some((mut state, bid)) = truncating_state(320, 240, &text) else {
            return;
        };
        // A wash over the WHOLE line, so that once the view scrolls its
        // rect genuinely starts left of the gutter. Without a decoration
        // here this test would say nothing about the quad batch — which
        // is the batch with no scissor of its own, and so the only one
        // that can bleed into the gutter.
        state.current_decorations = vec![Decoration {
            range: ByteRange {
                start: 0,
                end: text.len() as u64,
            },
            kind: DecorationKind::Selection,
        }];
        let before = state.render_offscreen();
        let offset = follow_to(&mut state, bid, text.len() as u64);
        assert!(offset > 0.0, "precondition: the caret's column scrolled");
        let after = state.render_offscreen();

        let gutter_right = state.code_clip_left() as u32;
        assert!(gutter_right > 0, "precondition: the gutter is on");
        let bottom = document_text_bottom(240, state.fm, state.band_inset()) as u32;
        assert_eq!(
            region(&before, 320, 0, gutter_right, 0, bottom),
            region(&after, 320, 0, gutter_right, 0, bottom),
            "scrolling the code must not touch one pixel of the gutter"
        );
        // Without this the test passes on a build where scrolling does
        // nothing at all.
        assert_ne!(
            region(&before, 320, gutter_right, 320, 0, bottom),
            region(&after, 320, gutter_right, 320, 0, bottom),
            "the code area must actually have moved"
        );
    }

    /// Q#G5 — **the glyphs themselves move**, which is the one thing
    /// every other witness here takes for granted.
    ///
    /// The gutter test above asserts the code area changes under a
    /// scroll, but a decoration wash and the caret both satisfy that on
    /// their own — it passes with `TextArea.left` pinned to `text_left`.
    /// So this isolates the glyph layer: **no decorations**, and the
    /// pixels examined are a source line with **no caret on it**, whose
    /// only ink is text glyphon placed.
    ///
    /// The fixture makes the claim binary rather than a diff: line 1 is
    /// blank for its first 295 columns and marked at its end, so at
    /// offset 0 its band holds no ink at all, and ink appearing there is
    /// unambiguously text that scrolled in from off the right edge.
    #[test]
    fn scrolling_moves_the_text_glyphs_not_only_the_decorations() {
        let text = format!("{}\n{}#####", long_line(), " ".repeat(295));
        let Some((mut state, bid)) = truncating_state(320, 240, &text) else {
            return;
        };
        let line_h = state.fm.code_line_height();
        // Row band of source line 1, which never holds the caret.
        let (y0, y1) = (
            (TEXT_TOP + line_h) as u32 + 1,
            (TEXT_TOP + 2.0 * line_h) as u32 - 1,
        );
        let left = state.code_clip_left() as u32;

        let before = state.render_offscreen();
        let bg = bg_sample(&before, 320);
        assert!(
            !(left..320).any(|x| (y0..y1).any(|y| differs_from_bg(&before, 320, bg, x, y))),
            "precondition: line 1's visible span is blank at offset 0"
        );

        // Follow the caret on line 0 — line 1 is dragged along by the
        // shared offset, and has no cursor of its own.
        let end_of_first_line = long_line().len() as u64;
        assert!(follow_to(&mut state, bid, end_of_first_line) > 0.0);
        let after = state.render_offscreen();
        assert!(
            (left..320).any(|x| (y0..y1).any(|y| differs_from_bg(&after, 320, bg, x, y))),
            "line 1's end must have scrolled into view — with `TextArea.left` \
             pinned to the code origin this band stays blank forever"
        );
    }

    /// Q#G5 — **a caret scrolled off the left is not painted.**
    ///
    /// Both predicates, because they are what the scroll indicator and
    /// the font/resize re-follow read. Revision 1 of the framing assumed
    /// this came for free; it did not — `code_caret_rect_in_clip` tested
    /// three edges and not this one.
    #[test]
    fn a_caret_scrolled_off_the_left_reports_not_painted() {
        let text = long_line();
        let Some((mut state, bid)) = truncating_state(320, 240, &text) else {
            return;
        };
        assert!(follow_to(&mut state, bid, text.len() as u64) > 0.0);
        // Move the cursor home WITHOUT following: the offset stays put,
        // so byte 0 now sits behind the gutter. This is the state a
        // frame can genuinely be in — the daemon's `CursorByte` and the
        // follow are separate steps.
        state.own_cursor = Some(OwnCursor {
            buffer_id: bid,
            byte: 0,
        });
        assert!(
            state.code_scroll_left > 0.0,
            "precondition: the view is still scrolled right"
        );
        let raw = state.caret_rect().expect("the caret still has geometry");
        assert!(
            raw.x < state.code_clip_left(),
            "precondition: byte 0 is left of the gutter edge, at {}",
            raw.x
        );
        assert!(
            state.code_caret_rect_in_clip().is_none(),
            "an off-left caret must not be painted"
        );
        assert!(
            !state.caret_painted_in_code_clip(),
            "and the predicate the re-follow reads must agree"
        );
    }

    /// The caret's left-edge predicate, tested **at** the edge — the
    /// audit the completion defect prompted.
    ///
    /// The caret's use of `survives_code_clip_left` is correct where the
    /// completion anchor's was not: a caret quad is `CARET_WIDTH` wide,
    /// so that IS its horizontal extent. What this pins is the
    /// consequence, which is otherwise only derivable: because
    /// `horizontal_follow` snaps the offset to whole columns, a caret is
    /// never *partly* behind the gutter. It is either at a visible
    /// column or a whole advance left of one, and `CARET_WIDTH` (2px) is
    /// far smaller than any code advance — so the straddle case the
    /// completion path got wrong cannot arise here at all.
    ///
    /// Written because that argument is load-bearing and invisible. A
    /// future change to the snap, or a font whose advance approached
    /// 2px, would break it silently.
    ///
    /// Mutation-checked: substituting `rect.h` for `rect.w` — the exact
    /// error the completion path shipped — fails this test. An
    /// over-width *smaller than one advance* does not, and that is the
    /// invariant rather than a gap: under column snapping an extent
    /// error too small to cross a column cannot put the caret in the
    /// gutter, which is why only the height-sized error is visible.
    #[test]
    fn the_caret_never_straddles_the_gutter_edge_under_column_snapping() {
        let text = long_line();
        let Some((mut state, bid)) = truncating_state(320, 240, &text) else {
            return;
        };
        let advance = state.mono_advance();
        assert!(
            advance > CARET_WIDTH,
            "the no-straddle argument needs an advance wider than the caret"
        );
        let clip = state.code_clip_left();

        // Walk the caret across the boundary one column at a time and
        // check every frame: painted carets are wholly inside the code
        // area, hidden ones are wholly outside it.
        assert!(follow_to(&mut state, bid, text.len() as u64) > 0.0);
        let scrolled = state.code_scroll_left;
        for col in 0..text.len() as u64 {
            state.own_cursor = Some(OwnCursor {
                buffer_id: bid,
                byte: col,
            });
            state.code_scroll_left = scrolled; // no follow: hold the view
            let Some(rect) = state.caret_rect() else {
                continue;
            };
            if state.code_caret_rect_in_clip().is_some() {
                assert!(
                    rect.x >= clip,
                    "a PAINTED caret at column {col} starts at {}, inside \
                     the gutter edge {clip}",
                    rect.x
                );
            } else {
                assert!(
                    rect.x + rect.w <= clip || rect.x >= state.text_bounds_right() as f32,
                    "a HIDDEN caret at column {col} spans [{}, {}) — it \
                     overlaps the code area, so hiding it loses the cursor",
                    rect.x,
                    rect.x + rect.w
                );
            }
        }
    }

    /// Q#G5 — **math boxes and their rules obey the same clip.**
    ///
    /// The glyph mini-buffers are clipped by their layer's `TextBounds`;
    /// the fraction rules are quads in the background batch with no
    /// scissor of their own, so they are cropped by hand and this is the
    /// test that says so.
    #[test]
    fn math_rules_are_cropped_at_the_gutter_when_scrolled_off_left() {
        let text = format!(r"$\frac{{ab}}{{cd}}$ {}", long_line());
        let Some((mut state, bid)) = truncating_state(320, 240, &text) else {
            return;
        };
        let (_, rules) = state.build_math_paint();
        assert!(
            !rules.is_empty(),
            "precondition: the fraction produced a rule quad"
        );
        let unscrolled = rules[0].x;
        assert!(follow_to(&mut state, bid, text.len() as u64) > 0.0);

        let clip = state.code_clip_left();
        let (glyphs, rules) = state.build_math_paint();
        for rule in &rules {
            assert!(
                rule.x >= clip - 0.01 && rule.w > 0.0,
                "a rule at x={} w={} escaped the clip at {clip}",
                rule.x,
                rule.w
            );
        }
        // Cropped or culled — but not left where it was. A rule that
        // never moved would also satisfy the loop above.
        assert!(
            rules.is_empty() || (rules[0].x - unscrolled).abs() > 0.5,
            "the rule must ride the offset"
        );
        for (_, left, _) in &glyphs {
            assert!(
                *left < unscrolled,
                "the glyph origins ride the offset too ({left} vs {unscrolled})"
            );
        }
    }

    /// Q#G5 — **an off-left completion anchor HIDES the popup.**
    ///
    /// It does not close it. Closure is `CompletionPopup { anchor: None
    /// }` and belongs to the daemon; a viewport lane must not quietly
    /// redefine when a completion ends. So: nothing paints while the
    /// anchor is off-left, the session state survives, and scrolling
    /// back brings the popup straight back.
    #[test]
    fn an_off_left_completion_anchor_hides_the_popup_without_closing_it() {
        let text = long_line();
        let Some((mut state, bid)) = truncating_state(480, 240, &text) else {
            return;
        };
        state.view_range = (0, text.len() as u64);
        state.completion = Some(CompletionLocal {
            buffer_id: bid,
            anchor: 0,
            prefix_len: 0,
            rows: vec![CompletionPopupRow {
                label: "candidate".into(),
                kind: 3,
                detail: None,
            }],
            selected: Some(0),
            total: 1,
        });
        assert!(
            state.completion_anchor_px().is_some(),
            "precondition: the anchor is visible at offset 0"
        );

        assert!(follow_to(&mut state, bid, text.len() as u64) > 0.0);
        state.view_range = (0, text.len() as u64);
        assert!(
            state.completion_anchor_px().is_none(),
            "an off-left anchor must not paint"
        );
        assert!(
            state.completion_dropdown_layout().is_none(),
            "and neither must the dropdown that hangs off it"
        );
        assert!(
            state.completion.is_some(),
            "HIDDEN, not closed — the daemon owns the session and its keys"
        );

        assert!(follow_to(&mut state, bid, 0).abs() < f32::EPSILON);
        state.view_range = (0, text.len() as u64);
        assert!(
            state.completion_anchor_px().is_some(),
            "scrolling back must bring the same popup back"
        );
    }

    /// Review finding — **the completion boundary is a POINT, and it is
    /// exact to within a fraction of a pixel.**
    ///
    /// The far-off-left test above cannot catch this. The first version
    /// of the predicate passed `line_height` as the horizontal extent —
    /// a vertical dimension standing in for a horizontal one — so an
    /// anchor anywhere within a line-height left of the gutter survived,
    /// and `completion_dropdown_rect` clamps `ax` against the right
    /// margin only. The popup painted over the line numbers. An anchor
    /// 200px off-left survives neither predicate, which is exactly why
    /// that test stayed green through the defect.
    ///
    /// So this straddles the edge instead: the same anchor a twentieth
    /// of a pixel either side of it, which no width-based predicate can
    /// separate.
    #[test]
    fn a_completion_anchor_a_hair_left_of_the_gutter_hides() {
        let text = long_line();
        let Some((mut state, bid)) = truncating_state(480, 240, &text) else {
            return;
        };
        state.view_range = (0, text.len() as u64);
        state.completion = Some(CompletionLocal {
            buffer_id: bid,
            anchor: 0,
            prefix_len: 0,
            rows: vec![CompletionPopupRow {
                label: "candidate".into(),
                kind: 3,
                detail: None,
            }],
            selected: Some(0),
            total: 1,
        });
        let (code_x, _, _) = state.code_byte_px(0).expect("the anchor has geometry");
        // Offset that puts the anchor exactly on the clip edge. Set
        // directly: this witnesses the PREDICATE, not the follow, and
        // the follow cannot land on a fractional boundary by design.
        let flush = state.text_left() + code_x - state.code_clip_left();

        state.code_scroll_left = flush + 0.05; // a hair LEFT of the edge
        assert!(
            state.completion_anchor_px().is_none(),
            "an anchor {:.2}px left of the gutter must hide — a width-based \
             predicate lets it through by most of a line height",
            0.05
        );
        assert!(
            state.completion.is_some(),
            "still HIDDEN, not closed, at the boundary too"
        );
        assert!(
            state.completion_dropdown_rect().is_none(),
            "and nothing downstream may resurrect a hidden anchor"
        );

        state.code_scroll_left = flush - 0.05; // a hair RIGHT of the edge
        let (ax, _, _) = state.completion_anchor_px().expect(
            "a hair right of the edge is visible — the predicate is \
                     a boundary, not a margin",
        );
        assert!(
            ax >= state.code_clip_left(),
            "the anchor the popup is placed at must be inside the code area"
        );
        // The claim that `completion_dropdown_rect` needs no left clamp
        // of its own, checked rather than asserted in a comment.
        let (left, _, _) = state
            .completion_dropdown_rect()
            .expect("the visible anchor places a popup");
        assert!(
            left >= state.code_clip_left(),
            "the popup's left edge must not enter the gutter ({left} vs {})",
            state.code_clip_left()
        );
    }

    /// Q#G5 — **the three consumers agree at a non-zero offset.**
    ///
    /// Caret, decoration geometry, and hit test, all for one byte. A
    /// partial fix — one painter translated and another not — shows up
    /// here as disagreement and nowhere else, because each of them looks
    /// correct on its own.
    #[test]
    fn caret_decoration_and_hit_test_agree_at_a_non_zero_offset() {
        let text = long_line();
        let Some((mut state, bid)) = truncating_state(320, 240, &text) else {
            return;
        };
        let target = text.len() as u64 - 4;
        assert!(follow_to(&mut state, bid, target) > 0.0);

        let caret_x = state.caret_rect().expect("caret is in the slice").x;
        assert!(
            caret_x >= state.code_clip_left(),
            "precondition: the followed caret is inside the code area"
        );

        state.current_decorations = vec![Decoration {
            range: ByteRange {
                start: target,
                end: target + 1,
            },
            kind: DecorationKind::Selection,
        }];
        let mut rects = Vec::new();
        let slice = &state.current_text[state.view_range.0 as usize..state.view_range.1 as usize];
        let line_offsets = line_byte_offsets(slice);
        state.collect_own_decoration_rects(
            &mut rects,
            &line_offsets,
            state.view_range.0,
            state.view_range.1,
        );
        let wash = rects
            .first()
            .copied()
            .expect("the selection produced a rect");
        assert!(
            (wash.x - caret_x).abs() < 1.0,
            "the wash over the caret's byte must start where the caret is \
             ({} vs {caret_x})",
            wash.x
        );

        let y = f64::from(TEXT_TOP + state.fm.code_line_height() / 2.0);
        assert_eq!(
            state.hit_test_source_byte(f64::from(caret_x) + 0.5, y),
            Some(target),
            "and a click on that pixel must land on that same byte"
        );
    }

    /// Q#G5 — **pixel → byte → pixel round trips at a non-zero offset.**
    ///
    /// The forward transform and its inverse are separate functions, and
    /// a sign error in either survives every one-directional test.
    #[test]
    fn hit_testing_round_trips_through_a_non_zero_offset() {
        let text = long_line();
        let Some((mut state, bid)) = truncating_state(320, 240, &text) else {
            return;
        };
        assert!(follow_to(&mut state, bid, text.len() as u64) > 0.0);

        let advance = state.mono_advance();
        let y = f64::from(TEXT_TOP + state.fm.code_line_height() / 2.0);
        let start = state.text_left() + advance * 2.5;
        for step in 0..5 {
            let x = start + advance * step as f32;
            let byte = state
                .hit_test_source_byte(f64::from(x), y)
                .expect("a pixel inside the code area hits a byte");
            let (code_x, _, _) = state.code_byte_px(byte).expect("that byte has geometry");
            let back = state.code_x_to_screen(code_x);
            assert!(
                (back - x).abs() <= advance,
                "round trip drifted: {x} -> byte {byte} -> {back}"
            );
        }
        // The inverse in isolation, exactly.
        assert!(
            (state.screen_x_to_code(state.code_x_to_screen(41.5)) - 41.5).abs() < 0.01,
            "screen_x_to_code must be the exact inverse of code_x_to_screen"
        );
    }

    /// Q#G5 — **the wrap transition resets the offset to zero**, and it
    /// is observed BEFORE any cursor motion.
    ///
    /// An implementation that merely ignores the offset while wrapped
    /// passes a "wrap looks right" test and fails this one: toggling
    /// back to `truncate` would surface the stale viewport instantly.
    #[test]
    fn toggling_wrap_and_back_resets_the_offset_before_any_motion() {
        let text = long_line();
        let Some((mut state, bid)) = truncating_state(320, 240, &text) else {
            return;
        };
        assert!(follow_to(&mut state, bid, text.len() as u64) > 0.0);

        state.apply_line_wrap(bid, true);
        assert!(
            state.code_scroll_left.abs() < f32::EPSILON,
            "wrapping zeroes the offset"
        );
        state.apply_line_wrap(bid, false);
        assert!(
            state.code_scroll_left.abs() < f32::EPSILON,
            "and it is still zero on the way back — no cursor has moved"
        );
    }

    /// Q#G5 — **a buffer snapshot resets the offset to zero**, observed
    /// before any `CursorByte` arrives.
    ///
    /// The pre-cursor scoping is the whole test. A later motion repairs
    /// the offset anyway, so a witness that waits for one cannot
    /// distinguish "reset on snapshot" from "repaired on first motion" —
    /// and it is the window between them, where buffer B renders scrolled
    /// sideways for no reason the user can see, that this rule exists for.
    #[test]
    fn a_buffer_snapshot_resets_the_offset_before_any_cursor_arrives() {
        let text = long_line();
        let Some((mut state, bid)) = truncating_state(320, 240, &text) else {
            return;
        };
        assert!(follow_to(&mut state, bid, text.len() as u64) > 0.0);

        let doc = loro::LoroDoc::new();
        doc.get_text(LORO_TEXT_CONTAINER)
            .insert(0, "buffer B")
            .expect("insert snapshot text");
        let other = BufferId::next();
        let _ = state.apply_attach_message(InstanceMessage::BufferSnapshot {
            buffer_id: other,
            crdt_snapshot: doc.export(loro::ExportMode::Snapshot).expect("export"),
        });

        assert!(
            state.own_cursor.is_none(),
            "precondition: no cursor has arrived for the new buffer yet"
        );
        assert!(
            state.code_scroll_left.abs() < f32::EPSILON,
            "the new buffer must not inherit the old one's viewport"
        );
        // And it must actually RENDER at the origin, not merely hold a
        // zeroed field: the first glyph of buffer B sits at the code
        // origin, which is the thing the user would see going wrong.
        let first_glyph_x = state
            .buffer
            .layout_runs()
            .next()
            .and_then(|run| run.glyphs.first().map(|g| g.x))
            .expect("buffer B shaped at least one glyph");
        assert!(
            (state.code_x_to_screen(first_glyph_x) - state.text_left()).abs() < 0.01,
            "buffer B must render at its code origin"
        );
    }

    /// Q#G5 / Q#G4 — **the minimap does not move.**
    ///
    /// It derives from the summary, the surface dimensions and
    /// `scroll_top`, with no horizontal input, so this pins an existing
    /// property rather than asking for new work. That is exactly why it
    /// is worth writing: an offset threaded one seam too far would break
    /// it silently.
    #[test]
    fn a_horizontal_offset_leaves_the_minimap_vertices_unchanged() {
        let text = long_line();
        let Some((mut state, bid)) = truncating_state(480, 300, &text) else {
            return;
        };
        state.current_summary = Some(FileStyleSummaryState {
            generation: 1,
            lines: vec![
                CellStyle {
                    fg: CellColor::Rgb(255, 255, 255),
                    ..CellStyle::default()
                };
                40
            ],
        });
        state.current_line_shapes = minimap_line_shapes(&text);
        state.minimap_cache = None;

        let before = state.minimap_vertex_bytes();
        assert!(
            !before.is_empty(),
            "precondition: the minimap actually paints, or this is vacuous"
        );
        let top = state.scroll_top;

        assert!(follow_to(&mut state, bid, text.len() as u64) > 0.0);
        assert_eq!(state.scroll_top, top, "vertical state must be unchanged");
        state.minimap_cache = None;
        assert_eq!(
            before,
            state.minimap_vertex_bytes(),
            "a horizontal offset must not reach the minimap"
        );
    }

    /// Q#G5 — **TUI parity, unconditional.**
    ///
    /// The same buffer and the same cursor column put the same character
    /// first in both frontends. This is checkable rather than asserted
    /// because there is one rule: `pmacs_protocol::scroll::follow_left`,
    /// which `src/editor.rs` also calls. What this test proves is the
    /// part that is genuinely GPU-local — that the px ↔ column
    /// conversion around that call is **exact**, which is what Q#G3
    /// (monospace code fonts only) buys and what the snap-back to a
    /// column multiple in `horizontal_follow` preserves.
    #[test]
    fn the_gpu_offset_is_the_tui_column_rule_converted_exactly() {
        let text = long_line();
        let Some((mut state, bid)) = truncating_state(320, 240, &text) else {
            return;
        };
        let advance = state.mono_advance();
        let width = state.text_bounds_right() as f32 - state.text_left();
        // Derived from the window and the font, not from the offset
        // under test — otherwise this would compare a value to itself.
        let cols = (width / advance).floor() as u32;
        assert!(cols > 0 && (text.len() as u32) > cols, "precondition");

        for cursor_col in [cols, cols + 1, text.len() as u32 - 1] {
            let offset = follow_to(&mut state, bid, u64::from(cursor_col));
            let expected = pmacs_protocol::scroll::follow_left(0, cursor_col, cols);
            // Every char in the fixture is one column wide, so the
            // cursor's byte IS its column and the TUI's `view_left`
            // would be `expected`.
            let in_columns = offset / advance;
            assert!(
                (in_columns - expected as f32).abs() < 0.01,
                "column {cursor_col}: the GPU is at {in_columns} columns, \
                 the TUI would be at {expected}"
            );
            // Exactness is the claim, so the offset must land on a
            // column boundary rather than merely near one.
            assert!(
                (offset - expected as f32 * advance).abs() < 0.01,
                "the offset must be an exact multiple of the advance"
            );
            // Reset so each case starts from a known edge, matching the
            // `follow_left(0, ..)` above.
            let _ = follow_to(&mut state, bid, 0);
        }
    }
}
