//! pmacs-gpu — GPU/GUI frontend for pmacs.
//!
//! Two run modes:
//!
//! - **Hello-world** (no `--attach` argument; session 2 default).
//!   Opens a window and renders "hello, pmacs" in the bundled
//!   `JetBrains` Mono. Used to confirm the wgpu/winit/glyphon stack
//!   without depending on a daemon.
//! - **Attach** (`--attach <unix-socket-path>`; session 3+). Connects
//!   to a running pmacs daemon, negotiates `semantic_render +
//!   crdt_replica`, imports the daemon's `BufferSnapshot` into a
//!   local loro replica, sends a `Viewport` back to request scoped
//!   styling, and consumes the `StyleSpans` stream — rendering the
//!   rope with per-span colors via cosmic-text's `set_rich_text`.
//!   Live `CrdtOp` updates apply to the doc; subsequent `StyleSpans`
//!   frames re-style.
//!
//! See `docs/pmacs-gpu-design.md` for the arc framing. Phase A's
//! adversarial-verification framing applies from session 4 forward;
//! findings classified per rule (iii) at surface-time.
//!
//! The bundled font is `JetBrains` Mono Regular, distributed under
//! the SIL Open Font License 1.1 (see `fonts/OFL.txt`).

mod attach;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use glyphon::cosmic_text::{Cursor, Scroll, Wrap};
use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, fontdb,
};
use loro::{ContainerTrait, ExportMode};
use pmacs_protocol::{
    AdornmentContent, AdornmentPlacement, BufferId, ByteRange, CompletionPopupRow, CrdtOp,
    Decoration, DecorationKind, DecorationSegment, FrontendId, InlineAdornment, InstanceMessage,
    InstanceSignal, Key as ProtocolKey, LineNumberMode, MenuPromptRow, Modifiers, PointerKind,
    SelectionSnapshot, StyleSegment, StyleSpan,
    cell::{Color as CellColor, Style as CellStyle},
    is_builtin_pair_char,
};
use wgpu::MultisampleState;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::attach::{AttachClient, AttachEvent};

/// Bundled font (SIL Open Font License 1.1 — see `fonts/OFL.txt`).
const JETBRAINS_MONO: &[u8] = include_bytes!("../fonts/JetBrainsMono-Regular.ttf");

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
fn build_font_system() -> (FontSystem, FontDefaults) {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let bundled_ids =
        db.load_font_source(fontdb::Source::Binary(std::sync::Arc::new(JETBRAINS_MONO)));
    let bundled_id = *bundled_ids
        .first()
        .expect("bundled JetBrains Mono contains one face");
    sanitize_font_database(&mut db, DEFAULT_FONT_FAMILY, bundled_id);
    db.set_monospace_family("Noto Sans Mono");
    db.set_sans_serif_family("Open Sans");
    db.set_serif_family("DejaVu Serif");
    let locale = sys_locale::get_locale().unwrap_or_else(|| String::from("en-US"));
    let font_system = FontSystem::new_with_locale_and_db(locale, db);
    let defaults = FontDefaults {
        default_family: DEFAULT_FONT_FAMILY.to_owned(),
        bundled_id,
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

/// The fixed ASCII advance probe (framing Q#F6): digits shape one
/// glyph per char in any face (no ligatures), so the first glyph's
/// advance is the family's monospace advance at these metrics.
const ADVANCE_PROBE: &str = "0123456789";

/// Wire-validation bounds for `FontFacts::size_centi_px` — 6.0..=72.0
/// logical px in integer hundredths (framing Q#F6, fail closed): 0
/// would panic `Buffer::set_metrics`, and the GPU re-checks on
/// arrival because this is deserialized protocol input — the
/// daemon-side Lua range check is a UX courtesy, not a trust
/// boundary.
const FONT_SIZE_CENTI_PX_RANGE: std::ops::RangeInclusive<u32> = 600..=7200;

/// Measure `family`'s normal-face glyph advance at `metrics` by
/// shaping [`ADVANCE_PROBE`] in a scratch buffer — independent of
/// document contents, so the measurement is deterministic and the
/// NORMAL face is authoritative even when the first code glyph is
/// bold/italic. `None` when the family shapes no glyphs.
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
    probe
        .layout_runs()
        .flat_map(|run| run.glyphs.iter())
        .next()
        .map(|glyph| glyph.w)
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
/// Caret bar width in px, and its color (bright, near-opaque — drawn
/// over the text so it reads as the active insertion point). Session
/// B1.
const CARET_WIDTH: f32 = 2.0;
const CARET_COLOR: [f32; 4] = [0.90, 0.90, 0.96, 0.90];
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

/// Text the hello-world (and attach-pre-snapshot / attach-failed)
/// modes render. Once the daemon's `BufferSnapshot` arrives the
/// rendered text becomes the rope contents instead.
const HELLO_TEXT: &str = "hello, pmacs";

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
#[derive(Debug, Clone)]
enum Mode {
    /// `pmacs-gpu` (no args): inert hello-world.
    HelloWorld,
    /// `pmacs-gpu --attach <socket>`: connect + render the daemon's
    /// rope.
    Attach { socket: PathBuf },
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
    let mode = parse_args(std::env::args().skip(1).collect());
    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .expect("create winit event loop");
    let proxy = event_loop.create_proxy();
    let mut app = App {
        mode,
        proxy: Some(proxy),
        state: None,
        attach_client: None,
        modifiers: winit::keyboard::ModifiersState::empty(),
    };
    event_loop
        .run_app(&mut app)
        .expect("winit event loop run_app");
}

/// Tiny argv parser. No `clap` because the surface is genuinely two
/// shapes; full CLI parsing arrives when there's more to parse. The
/// `for` ranges over a small set: at most one `--attach <socket>` or
/// `--help` arrives, plus any stray unrecognized flag.
fn parse_args(args: Vec<String>) -> Mode {
    let mut iter = args.into_iter();
    let Some(first) = iter.next() else {
        return Mode::HelloWorld;
    };
    match first.as_str() {
        "--attach" => {
            let socket = iter.next().unwrap_or_else(|| {
                eprintln!("pmacs-gpu: --attach requires a socket path");
                std::process::exit(2);
            });
            Mode::Attach {
                socket: PathBuf::from(socket),
            }
        }
        "--help" | "-h" => {
            eprintln!(
                "pmacs-gpu — GPU/GUI frontend for pmacs\n\nUSAGE:\n  pmacs-gpu                       \
                 hello-world (renders \"hello, pmacs\")\n  pmacs-gpu --attach <socket>     \
                 connect to a daemon's Unix socket and render its rope\n"
            );
            std::process::exit(0);
        }
        other => {
            eprintln!("pmacs-gpu: unrecognized argument: {other}");
            std::process::exit(2);
        }
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
    /// Held both for stream lifetime and for the main loop's
    /// `send_viewport` / `send_key` write-back path.
    attach_client: Option<AttachClient>,
    /// Latest modifier state from winit (`ModifiersChanged`). winit
    /// delivers modifiers separately from key presses, so we track the
    /// current set and apply it when a key is sent (session B1).
    modifiers: winit::keyboard::ModifiersState,
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
    /// per line. Refreshed when a new summary lands, keeping this
    /// cache in cadence with the debounced minimap data rather than
    /// rebuilding it for every typed byte.
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
    /// Absolute source-line index of `buffer.lines[0]`.
    shaped_top: usize,
    bg_vertex_buffer: ReusableVertexBuffer,
    squiggle_vertex_buffer: ReusableVertexBuffer,
    caret_vertex_buffer: ReusableVertexBuffer,
    minimap_vertex_buffer: ReusableVertexBuffer,
    /// Q#S2 — the status band's one-line text. Shaped only when the
    /// composed status string changes; rendered as a second
    /// `TextArea` in the same prepare pass as the main buffer.
    status_buffer: Buffer,
    /// The string `status_buffer` currently holds, for change
    /// detection.
    status_text: String,
    /// Q#S2 — the band's left side (buffer name + modified dot),
    /// its own buffer so it left-aligns independently of the
    /// right-aligned readout.
    status_left_buffer: Buffer,
    /// Change-detection twin of `status_text` for the left side.
    status_left_text: String,
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

/// The live minibuffer (Q#MB1, protocol v12), mirrored from a
/// `MinibufferPrompt` whose `prompt` was `Some`. The prompt+input draw
/// in the bottom band with a caret; `candidates` (a windowed slice) feed
/// the dropdown.
#[derive(Clone, Debug, PartialEq)]
struct MinibufferLocal {
    prompt: String,
    input: String,
    cursor: u32,
    candidates: Vec<String>,
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
        if client.server_protocol_version() < 5 {
            return;
        }
        // TripleDown is a v7 variant; a pre-v7 instance would
        // hard-error decoding it. Downgrade to a plain Down — the
        // exact behavior the third click had before v7 (the chain
        // restarting).
        let kind = if kind == PointerKind::TripleDown && client.server_protocol_version() < 7 {
            PointerKind::Down
        } else {
            kind
        };
        // Context (right-click, Q#CM1) is a v11 variant; a pre-v11
        // instance can't open a menu, so drop the gesture rather than
        // sending an undecodable variant.
        if kind == PointerKind::Context && client.server_protocol_version() < 11 {
            return;
        }
        if let Err(e) = client.send_pointer(buffer_id, byte, kind, mods) {
            eprintln!("pmacs-gpu: send_pointer failed: {e}");
        }
    }

    /// Ship a [`pmacs_protocol::FrontendEvent::MenuPointer`] if the
    /// daemon speaks v11+ (Q#CM1). Navigates the open menu the daemon
    /// owns; pixels stay local, only the resolved row index crosses.
    fn send_menu_pointer(&self, index: Option<u32>, invoke: bool) {
        let Some(client) = self.attach_client.as_ref() else {
            return;
        };
        if client.server_protocol_version() < 11 {
            return;
        }
        if let Err(e) = client.send_menu_pointer(index, invoke) {
            eprintln!("pmacs-gpu: send_menu_pointer failed: {e}");
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let initial_text = match &self.mode {
            Mode::HelloWorld => HELLO_TEXT,
            Mode::Attach { .. } => "(connecting...)",
        };
        self.state = Some(State::new(event_loop, initial_text));

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
                    }
                    self.attach_client = Some(client);
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
    }

    #[allow(clippy::too_many_lines)] // linear per-event dispatch; splitting hides the input flow.
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::ModifiersChanged(mods) => self.modifiers = mods.state(),
            WindowEvent::KeyboardInput { event: key, .. } => {
                if key.state != ElementState::Pressed {
                    return;
                }
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
                        event_loop.exit();
                    }
                    return;
                }

                let Some((pkey, mut pmods)) = translate_key(&key.logical_key, self.modifiers)
                else {
                    return;
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
                if matches!(pkey, ProtocolKey::Char(_))
                    && is_layout_text(key.text.as_deref(), pmods)
                {
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
                    return;
                }

                let Some(client) = self.attach_client.as_ref() else {
                    return;
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
                    return;
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
                    return;
                }

                // Session B2 forwards cursor motion + plain text editing
                // (Char / Backspace / Enter / Delete / Tab). Command chords
                // are handled above; Meta/Super-only chords fall through
                // here and are withheld, leaving OS/WM shortcuts (Cmd-Q,
                // Cmd-C) to the platform.
                if !should_forward_key(pkey, pmods) {
                    return;
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
                        && let Err(e) =
                            client.send_viewport(vp.buffer_id, vp.visible, vp.generation)
                    {
                        eprintln!("pmacs-gpu: send Viewport failed: {e}");
                    }
                    return;
                }
                if let Some(state) = self.state.as_mut() {
                    if state.defer_round_trip_key_if_needed(pkey, pmods) {
                        if debug_input() {
                            eprintln!(
                                "pmacs-gpu defer_key: {pkey:?} mods={pmods:?} \
                                 pending optimistic cursor"
                            );
                        }
                        return;
                    }
                    state.mark_cursor_stale_after_round_trip();
                }
                if debug_input() {
                    eprintln!("pmacs-gpu send_key: {pkey:?} mods={pmods:?}");
                }
                if let Err(e) = client.send_key(pkey, pmods) {
                    eprintln!("pmacs-gpu: send_key failed: {e}");
                }
            }
            WindowEvent::Resized(size) => {
                let vp = self
                    .state
                    .as_mut()
                    .and_then(|state| state.resize(size.width.max(1), size.height.max(1)));
                if let Some(vp) = vp
                    && let Some(client) = self.attach_client.as_ref()
                    && let Err(e) = client.send_viewport(vp.buffer_id, vp.visible, vp.generation)
                {
                    eprintln!("pmacs-gpu: resize send_viewport failed: {e}");
                }
            }
            // Session M-2 — pointer input (docs/pmacs-gpu-mouse-framing.md).
            WindowEvent::CursorMoved { position, .. } => {
                let Some(state) = self.state.as_mut() else {
                    return;
                };
                state.pointer_pos = Some((position.x, position.y));
                // Q#CM1 — while the menu is open, motion only moves the
                // highlight; send a hover when the item under the pointer
                // changes from the daemon's current active row.
                if state.menu.is_some() {
                    let hit = state.menu_hit(position.x, position.y);
                    let active = state.menu.as_ref().and_then(|m| m.active);
                    if let Some((row, true)) = hit
                        && active != Some(row)
                    {
                        self.send_menu_pointer(Some(row), false);
                    }
                    return;
                }
                if state.minimap_scrub_active {
                    // Scrubbing (Q#M6): the press began on the
                    // minimap; motion keeps jumping, even if the
                    // pointer wanders out of the band.
                    let vp = state.minimap_jump_to(position.y);
                    if let Some(vp) = vp
                        && let Some(client) = self.attach_client.as_ref()
                        && let Err(e) =
                            client.send_viewport(vp.buffer_id, vp.visible, vp.generation)
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
                    edge_scroll_direction(position.y as f32, state.config.height, state.fm);
                // Drag coalescing (predicted finding #4): pixel-rate
                // motion only ships when the hit byte changes.
                let Some(byte) = state.hit_test_source_byte(position.x, position.y) else {
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
            WindowEvent::MouseInput {
                state: button_state,
                button: winit::event::MouseButton::Left,
                ..
            } => {
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
                            Some((_, false)) => None, // separator — ignore
                            None => Some((None, true)), // outside — dismiss
                        };
                        if let Some((index, invoke)) = action {
                            self.send_menu_pointer(index, invoke);
                        }
                    }
                    return;
                }
                let mods = translate_mods(self.modifiers);
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
                        let kind =
                            state.classify_pointer_down(byte, mods.contains(Modifiers::SHIFT));
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
            // Q#CM1 — right-click opens the context menu at the hit byte
            // (or dismisses an open one). The anchor pixel is remembered
            // so the popup the daemon sends back draws at the click.
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: winit::event::MouseButton::Right,
                ..
            } => {
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
            WindowEvent::MouseWheel { delta, .. } => {
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
                let vp = state.scroll_by_lines(lines);
                if let Some(vp) = vp
                    && let Some(client) = self.attach_client.as_ref()
                    && let Err(e) = client.send_viewport(vp.buffer_id, vp.visible, vp.generation)
                {
                    eprintln!("pmacs-gpu: wheel send_viewport failed: {e}");
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(state) = self.state.as_mut() {
                    state.render();
                }
            }
            _ => {}
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
        let Some(state) = self.state.as_mut() else {
            return;
        };
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
                state.set_text("(daemon disconnected)");
            }
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
        )
    }

    /// Build a windowless `State` that renders to an offscreen texture, for
    /// the headless render tests (F-014). Returns `None` when no GPU
    /// adapter is available (a dev box with no working Vulkan, or CI
    /// without lavapipe), so the caller skips rather than fails.
    #[cfg(test)]
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
        let (mut font_system, font_defaults) = build_font_system();
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
        // Arc 1a Q#C5 — a renderer for the completion dropdown layer.
        let completion_text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        // UX gutter — a renderer for the line-number layer.
        let gutter_text_renderer =
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
        let mut status_left_buffer = Buffer::new(
            &mut font_system,
            Metrics::new(fm.status_font_size(), fm.status_line_height()),
        );
        status_left_buffer.set_size(
            &mut font_system,
            Some(config.width as f32),
            Some(fm.status_band_height()),
        );
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
            shaped_top: 0,
            bg_vertex_buffer: ReusableVertexBuffer::new(),
            squiggle_vertex_buffer: ReusableVertexBuffer::new(),
            caret_vertex_buffer: ReusableVertexBuffer::new(),
            minimap_vertex_buffer: ReusableVertexBuffer::new(),
            status_buffer,
            status_text: String::new(),
            status_left_buffer,
            status_left_text: String::new(),
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
        };
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
        self.translate_cached_anchors(&edits);
        // Q#R1 — the keystroke case (one edit, no line-structure
        // change) re-shapes only the affected BufferLine; everything
        // else falls back to the full slice reshape.
        let single_line_edit = edits.len() == 1
            && self.current_line_starts.len() == line_count_before
            && !self.current_text
                [edits[0].start as usize..(edits[0].start + edits[0].inserted_len) as usize]
                .contains('\n');
        if !(single_line_edit && self.try_reshape_line(edits[0])) {
            self.reshape();
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
        self.current_text.clear();
        self.current_text.push_str(text);
        let (line_starts, line_char_starts) = line_offset_tables(text);
        self.current_line_starts = line_starts;
        self.current_line_char_starts = line_char_starts;
        self.reshape();
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
                self.last_viewport_sent = None;
                if !self.set_text(&text) {
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
                // shape_until_scroll.
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
            } => {
                self.apply_file_style_summary(buffer_id, generation, lines);
                None
            }
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
                    self.line_numbers = mode;
                    self.request_redraw();
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
                self.status_text.clear();
                self.status_left_text.clear();
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
            // None` closes it.
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
                    candidates,
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
                self.current_buffer_id
                    .and_then(|bid| self.viewport_send_if_changed(bid))
            }
            _ => None,
        }
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
        let visible = estimated_visible_lines(self.config.height, self.fm).max(1);
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
        if let Some((slice_i, projected)) = self.code_byte_to_projected(byte) {
            self.buffer.shape_until_cursor(
                &mut self.font_system,
                Cursor::new(slice_i, projected),
                false,
            );
        }
        self.normalize_code_scroll();
        self.request_redraw();
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
        let raw_x = x as f32 - self.text_left();
        if self.line_numbers.is_on() && raw_x < 0.0 {
            0.0
        } else {
            raw_x
        }
    }

    fn hit_test_source_byte(&mut self, x: f64, y: f64) -> Option<u64> {
        self.current_buffer_id?;
        if self.hit_map_dirty {
            // Q#R2 — a per-line reshape deferred this; rebuild from
            // the same chunk source the shaped buffer was built from.
            let (vstart, vend) = self.view_range;
            let rich = clipped_chunks_for_range(
                &self.current_text,
                &self.current_spans,
                &self.current_adornments,
                vstart,
                vend,
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
        )?;
        let centered =
            target.saturating_sub(estimated_visible_lines(self.config.height, self.fm) / 2);
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
        if shaped_idx >= self.buffer.lines.len() || shaped_idx >= self.line_chunk_cache.len() {
            // E.g. typing on the phantom empty line after a trailing
            // newline — no BufferLine exists for it; full reshape
            // handles those shapes correctly.
            return false;
        }
        let chunks = self.chunks_for_line(line_start, content_end);
        self.buffer.lines[shaped_idx] = line_from_chunks(&chunks, &self.resolved_family);
        self.line_chunk_cache[shaped_idx] = chunks;
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

    fn chunks_for_line(&self, line_start: u64, content_end: u64) -> Vec<RichChunk> {
        clipped_chunks_for_range(
            &self.current_text,
            &self.current_spans,
            &self.current_adornments,
            line_start,
            content_end,
        )
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
        let mut lines = Vec::with_capacity(ranges.len());
        let mut cache = Vec::with_capacity(ranges.len());
        let mut any_reused = false;
        for (i, &(ls, ce)) in ranges.iter().enumerate() {
            let abs = new_top + i;
            let reused = abs.checked_sub(old_top).and_then(|j| {
                if j < old_lines.len() && j < old_cache.len() {
                    old_lines[j].take().zip(old_cache[j].take())
                } else {
                    None
                }
            });
            if let Some((line, chunks)) = reused {
                any_reused = true;
                lines.push(line);
                cache.push(chunks);
            } else {
                let chunks = self.chunks_for_line(ls, ce);
                lines.push(line_from_chunks(&chunks, &self.resolved_family));
                cache.push(chunks);
            }
        }
        self.buffer.lines = lines;
        self.line_chunk_cache = cache;
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
            || ranges.len() != self.buffer.lines.len()
        {
            self.reshape();
            return;
        }
        let mut any = false;
        for (i, &(ls, ce)) in ranges.iter().enumerate() {
            let chunks = self.chunks_for_line(ls, ce);
            if chunks != self.line_chunk_cache[i] {
                self.buffer.lines[i] = line_from_chunks(&chunks, &self.resolved_family);
                self.line_chunk_cache[i] = chunks;
                any = true;
            }
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

    /// The band's left-segment text color, mirroring
    /// [`Self::compose_status_left`]'s priority: minibuffer/isearch
    /// content follows `ui.minibuffer`, a transient message follows
    /// `ui.statusline`, and the buffer name follows `ui.modeline`
    /// (the framing's content-class applicability, Q#TH3).
    fn status_left_color(&self) -> Color {
        const LEFT_DEFAULT: (u8, u8, u8) = (200, 200, 210);
        let fallback = Color::rgb(LEFT_DEFAULT.0, LEFT_DEFAULT.1, LEFT_DEFAULT.2);
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
        self.modeline_face_colors().map_or(fallback, |(_, t)| t)
    }

    /// Compose the status-band readout (Q#S1): diagnostic counts
    /// (wire-authoritative, severity-colored, omitted when zero),
    /// then cursor L:C from the *optimistic* caret (so it tracks
    /// typing bursts instead of lagging a round trip), then the
    /// All/Top/Bot/NN% scroll indicator. Returns the colored spans.
    fn compose_status_spans(&self) -> Vec<(String, Option<Color>)> {
        use std::fmt::Write as _;
        let mut spans: Vec<(String, Option<Color>)> = Vec::new();
        if let Some(facts) = self
            .status_facts
            .as_ref()
            .filter(|f| Some(f.buffer_id) == self.current_buffer_id)
        {
            if facts.diag_errors > 0 {
                // Themes Q#TH5: the counters follow the diag faces
                // (fg mask; the shaping-cache invalidation in the
                // ThemeFacts arm makes a recolor with constant counts
                // actually re-shape, Q#TH8).
                spans.push((
                    format!("E:{}", facts.diag_errors),
                    Some(self.diag_face_fg_or("ui.diag.error", Color::rgb(241, 76, 76))),
                ));
            }
            if facts.diag_warnings > 0 {
                spans.push((
                    format!("W:{}", facts.diag_warnings),
                    Some(self.diag_face_fg_or("ui.diag.warning", Color::rgb(245, 245, 67))),
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
                .partition_point(|&s| s as usize <= byte)
                .saturating_sub(1);
            cursor_row = line;
            let ls = self.current_line_starts.get(line).copied().unwrap_or(0) as usize;
            let col = self
                .current_text
                .get(ls..byte)
                .map_or(0, |s| s.chars().count());
            let _ = write!(readout, "L{}:C{}", line + 1, col + 1);
            readout.push_str("  ");
        }
        readout.push_str(&format_scroll_indicator(
            self.scroll_top,
            estimated_visible_lines(self.config.height, self.fm),
            self.current_line_starts.len(),
            cursor_row,
        ));
        spans.push((readout, None));
        spans
    }

    /// The band's left side. While an incremental search is running
    /// (Q#SR5) it shows `I-search: <query> (n/m)` — the prompt takes
    /// over the band like Emacs's echo area, returning to the buffer
    /// name + modified dot (v8 `StatusFacts`) when the search ends.
    fn compose_status_left(&self) -> String {
        // Q#MB1 — an open minibuffer takes over the band: prompt + input
        // (the candidates render separately as a dropdown). Measured by
        // the band caret, so it must stay exactly `prompt + input`.
        if let Some(mb) = self.minibuffer.as_ref() {
            return format!("{}{}", mb.prompt, mb.input);
        }
        if let Some(sp) = self
            .search_prompt
            .as_ref()
            .filter(|s| Some(s.buffer_id) == self.current_buffer_id)
        {
            let label = if sp.regex {
                "Regex I-search: "
            } else {
                "I-search: "
            };
            let count = if sp.query.is_empty() {
                String::new()
            } else if sp.invalid {
                " [invalid]".to_string()
            } else if sp.total == 0 {
                " [no match]".to_string()
            } else {
                format!(" ({}/{})", sp.active.map_or(0, |a| a + 1), sp.total)
            };
            return format!("{}{}{}", label, sp.query, count);
        }
        // A transient status message (v15 `StatusFacts.message` — LSP
        // command summaries like "12 references", error reports) takes
        // the band over echo-area style; the daemon clears it on the
        // next keypress, which ships a fresh `StatusFacts` and returns
        // the band to the buffer name.
        if let Some(msg) = self
            .status_facts
            .as_ref()
            .filter(|f| Some(f.buffer_id) == self.current_buffer_id)
            .and_then(|f| f.message.as_deref())
        {
            return msg.to_owned();
        }
        match self
            .status_facts
            .as_ref()
            .filter(|f| Some(f.buffer_id) == self.current_buffer_id)
        {
            Some(facts) if facts.modified => format!("{} ●", facts.name),
            Some(facts) => facts.name.clone(),
            None => String::new(),
        }
    }

    /// Re-shape the status-band text iff the composed content
    /// changed (short lines — shaping is trivial, but not free per
    /// frame).
    fn refresh_status_line(&mut self) {
        let spans = self.compose_status_spans();
        let composed: String = spans
            .iter()
            .map(|(t, _)| t.as_str())
            .collect::<Vec<_>>()
            .join("  ");
        let family = self.resolved_family.clone();
        let default_attrs = Attrs::new().family(Family::Name(&family));
        if composed != self.status_text {
            let mut rich: Vec<(&str, Attrs)> = Vec::new();
            for (i, (t, c)) in spans.iter().enumerate() {
                if i > 0 {
                    rich.push(("  ", default_attrs.clone()));
                }
                let attrs = match c {
                    Some(color) => default_attrs.clone().color(*color),
                    None => default_attrs.clone(),
                };
                rich.push((t.as_str(), attrs));
            }
            self.status_buffer.set_rich_text(
                &mut self.font_system,
                rich,
                &default_attrs,
                Shaping::Advanced,
                None,
            );
            self.status_buffer
                .shape_until_scroll(&mut self.font_system, false);
            self.status_text = composed;
        }
        let left = self.compose_status_left();
        if left != self.status_left_text {
            self.status_left_buffer.set_text(
                &mut self.font_system,
                &left,
                &default_attrs,
                Shaping::Advanced,
                None,
            );
            self.status_left_buffer
                .shape_until_scroll(&mut self.font_system, false);
            self.status_left_text = left;
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
            y: text_area_bottom(self.config.height, self.fm),
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
    fn refresh_mb_buffer(&mut self) {
        let text = self
            .minibuffer
            .as_ref()
            .map_or_else(String::new, |mb| mb.candidates.join("\n"));
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
        let band_top = text_area_bottom(self.config.height, self.fm);
        mb_dropdown_window(
            mb.candidates.len(),
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
        let band_top = text_area_bottom(self.config.height, self.fm);
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
        let bottom = text_area_bottom(self.config.height, self.fm);
        if y >= bottom || y + line_height <= TEXT_TOP {
            return None;
        }
        Some((self.text_left() + x, y, line_height))
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
        let band_top = text_area_bottom(self.config.height, self.fm);
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
    ) {
        if self.current_buffer_id != Some(buffer_id) {
            return;
        }
        if self
            .current_summary
            .as_ref()
            .is_some_and(|summary| generation < summary.generation)
        {
            return;
        }
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
        self.request_redraw();
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
        let span = estimated_visible_lines(self.config.height, self.fm).max(1) + SCROLL_OVERSCAN;
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
        for &(ls, ce) in &ranges {
            let chunks = self.chunks_for_line(ls, ce);
            lines.push(line_from_chunks(&chunks, &self.resolved_family));
            cache.push(chunks);
        }
        self.buffer.lines = lines;
        self.line_chunk_cache = cache;
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
    /// this eagerly is cheap.
    fn sync_buffer_dimensions(&mut self) {
        let fm = self.fm;
        let width = self.config.width as f32;
        let height = self.config.height as f32;
        let code_metrics = Metrics::new(fm.code_font_size(), fm.code_line_height());
        let code_width = (self.text_bounds_right() as f32 - self.text_left()).max(0.0);
        let code_height = (text_area_bottom(self.config.height, fm) - TEXT_TOP).max(0.0);
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
        let advance_ratio = if resolved == self.font_defaults.default_family {
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
        self.measured_mono_advance = selected_advance;
        // Rows stay rows: idempotent no-wrap on the popup buffers
        // (assembly set it; a set_wrap no-op costs a comparison).
        self.menu_buffer.set_wrap(&mut self.font_system, Wrap::None);
        self.mb_buffer.set_wrap(&mut self.font_system, Wrap::None);
        self.completion_buffer
            .set_wrap(&mut self.font_system, Wrap::None);
        // Metrics + current dimensions atomically on all seven.
        self.sync_buffer_dimensions();
        // The two string-equality shaping gates (the popups rebuild
        // unconditionally per frame). NUL can never equal a composed
        // status string, so the next frame re-shapes with new attrs
        // even when its composed text is unchanged.
        "\0".clone_into(&mut self.status_text);
        "\0".clone_into(&mut self.status_left_text);
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
    /// RGBA8 (`width * height * 4` bytes, row padding removed). Test-only,
    /// the entry point for the headless render harness (F-014).
    #[cfg(test)]
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
        // The band's strip rides the bg quad batch so it draws under
        // the band text (text renders after the first quad draw).
        let mut bg_vertices = self.decoration_background_vertex_bytes();
        bg_vertices.extend(self.status_band_vertex_bytes());
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
        let squiggle_vertices = self.squiggle_vertex_bytes();
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
        let caret_vertices = self.caret_vertex_bytes();
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
        let minimap_vertices = &self.minimap_cache.as_ref().expect("just filled").1;
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

        // Right-align the status readout: measure the shaped width
        // and place the area flush to the right pad (Q#S2).
        let status_width = self
            .status_buffer
            .layout_runs()
            .map(|r| r.line_w)
            .fold(0.0_f32, f32::max);
        let status_left =
            (self.config.width as f32 - STATUS_TEXT_PAD - status_width).max(TEXT_LEFT);
        let status_top = text_area_bottom(self.config.height, self.fm)
            + (self.fm.status_band_height() - self.fm.status_line_height()) / 2.0;
        // UX gutter: the code's left origin (past the gutter) and the
        // main-text clip-left. Computed here as locals — calling `self.*`
        // inside the `prepare` args would conflict with its `&mut` borrows.
        let text_left = self.text_left();
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
        self.text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [
                    TextArea {
                        buffer: &self.buffer,
                        left: text_left,
                        top: TEXT_TOP,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: gutter_clip_left,
                            top: 0,
                            right: text_bounds_right,
                            // Clip at the status band (Q#S3): a final
                            // partially-visible line must not bleed
                            // into the band.
                            bottom: text_area_bottom(self.config.height, self.fm).round() as i32,
                        },
                        default_color: Color::rgb(230, 230, 235),
                        custom_glyphs: &[],
                    },
                    TextArea {
                        buffer: &self.status_buffer,
                        left: status_left,
                        top: status_top,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: 0,
                            top: text_area_bottom(self.config.height, self.fm).round() as i32,
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
                            top: text_area_bottom(self.config.height, self.fm).round() as i32,
                            // Stop before the right-aligned readout.
                            right: (status_left - STATUS_TEXT_PAD).max(0.0).round() as i32,
                            bottom: self.config.height.cast_signed(),
                        },
                        // Themes Q#TH3: the left segment's face follows
                        // its CONTENT class (minibuffer/isearch →
                        // ui.minibuffer; message → ui.statusline; name
                        // → ui.modeline).
                        default_color: left_color,
                        custom_glyphs: &[],
                    },
                ],
                &mut self.swash_cache,
            )
            .expect("text_renderer prepare");

        // UX gutter: prepare the line-number layer in the reserved left
        // strip (empty when off → renders nothing). Same `top` + line
        // height as the code, so numbers align row-for-row.
        let gutter_areas: Vec<TextArea> = if self.line_numbers.is_on() {
            vec![TextArea {
                buffer: &self.gutter_buffer,
                left: TEXT_LEFT,
                top: TEXT_TOP,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: gutter_clip_left,
                    bottom: text_area_bottom(self.config.height, self.fm).round() as i32,
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
                    bottom: text_area_bottom(self.config.height, self.fm).round() as i32,
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
        let visible_lines = estimated_visible_lines(self.config.height, self.fm);
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
        let status_top = text_area_bottom(self.config.height, self.fm)
            + (self.fm.status_band_height() - self.fm.status_line_height()) / 2.0;
        Some(MinimapRect {
            x: STATUS_TEXT_PAD + advance * cursor_chars,
            y: status_top,
            w: CARET_WIDTH,
            h: self.fm.status_line_height(),
            color: CARET_COLOR,
        })
    }

    /// Map an absolute source `byte` to `(slice line index, projected
    /// byte offset within that shaped line)` by inverting the line's
    /// `line_chunk_cache` projection (framing Q#F6): source bytes are
    /// not projected bytes once inline adornments inject text. An
    /// adornment anchor maps to the EARLIEST projected boundary — the
    /// current left-gravity caret placement, before the injected
    /// text. `None` when the byte's source line is outside the shaped
    /// slice.
    fn code_byte_to_projected(&self, byte: u64) -> Option<(usize, usize)> {
        let line_idx = self
            .current_line_starts
            .partition_point(|&s| s <= byte)
            .saturating_sub(1);
        let slice_i = line_idx.checked_sub(self.shaped_top)?;
        if slice_i >= self.line_chunk_cache.len() {
            return None;
        }
        let rel = byte - self.current_line_starts[line_idx];
        let mut projected = 0usize;
        for chunk in &self.line_chunk_cache[slice_i] {
            match chunk.source {
                ChunkSource::Source { start } => {
                    let len = chunk.text.len() as u64;
                    if rel >= start && rel < start + len {
                        return Some((slice_i, projected + (rel - start) as usize));
                    }
                }
                ChunkSource::Adornment { anchor } => {
                    // Source chunks tile the line, so reaching an
                    // adornment chunk unmatched means the byte sits at
                    // its anchor boundary (or past line end).
                    if rel <= anchor {
                        return Some((slice_i, projected));
                    }
                }
            }
            projected += chunk.text.len();
        }
        // Line end (the `\n` position, or EOF).
        Some((slice_i, projected))
    }

    /// The caret geometry `(x, top, line_height)` for an absolute
    /// source `byte`, in code-area-local space (x excludes
    /// `text_left`, top excludes `TEXT_TOP`) — visual-run aware
    /// (framing Q#F6). Inverts the chunk projection, then uses
    /// cosmic-text's `layout_cursor` on the same Before-affinity
    /// `Cursor` that `ensure_caret_painted` shapes toward, so a wrap
    /// boundary selects the same visual run. The vertical position
    /// accumulates the laid-out run heights above the selected run
    /// under the normalized scroll; callers decide visibility by
    /// intersecting with the drawable clip.
    fn code_byte_px(&mut self, byte: u64) -> Option<(f32, f32, f32)> {
        let (slice_i, projected) = self.code_byte_to_projected(byte)?;
        let cursor = Cursor::new(slice_i, projected);
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
            // UX gutter: the caret sits in the code area, past the gutter.
            x: self.text_left() + x,
            y: TEXT_TOP + top,
            w: CARET_WIDTH,
            h: line_height,
            color: CARET_COLOR,
        })
    }

    /// [`Self::caret_rect`] intersected with the drawable code clip —
    /// the painter's own bounds (right of the gutter isn't needed:
    /// the caret x can't precede `text_left`), NOT `view_range`, so
    /// the two-line source overscan and wrapped runs clipped below
    /// the band don't count as painted (framing Q#F6).
    fn code_caret_rect_in_clip(&mut self) -> Option<MinimapRect> {
        let rect = self.caret_rect()?;
        let bottom = text_area_bottom(self.config.height, self.fm);
        let right = self.text_bounds_right() as f32;
        (rect.y < bottom && rect.y + rect.h > TEXT_TOP && rect.x < right).then_some(rect)
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

    /// Push one rect per visual line whose glyphs overlap the
    /// buffer-absolute byte range `[lo, hi)`, spanning the matching
    /// glyphs' horizontal extent. A range crossing visual-line
    /// boundaries (wrapped or multi-line) fans out into one rect per
    /// run. `line_offsets[run.line_i]` rebases the run's line-relative
    /// glyph offsets into buffer-absolute space for the comparison.
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
        // UX gutter: washes/squiggles are code-relative, past the gutter.
        let text_left = self.text_left();
        for run in self.buffer.layout_runs() {
            let line_base = line_offsets.get(run.line_i).copied().unwrap_or(0);
            let mut min_x: Option<f32> = None;
            let mut max_x: Option<f32> = None;
            for glyph in run.glyphs {
                let g_start = line_base + glyph.start as u64;
                let g_end = line_base + glyph.end as u64;
                if g_end <= lo || g_start >= hi {
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
                rects.push(MinimapRect {
                    x: text_left + x0,
                    y,
                    w: x1 - x0,
                    h,
                    color,
                });
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
    /// Injected adornment text (inlay hint) anchored at this slice
    /// byte offset. Hits inside it snap to the anchor.
    Adornment { anchor: u64 },
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

/// Map a projected byte offset back to a slice-relative source byte
/// (Q#M2). Hits inside an adornment run snap to its anchor; offsets
/// past the last run clamp to its end.
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
        ChunkSource::Adornment { anchor } => Some(anchor),
    }
}

fn minimap_left(surface_width: u32) -> Option<f32> {
    if surface_width < MINIMAP_MIN_SURFACE_WIDTH {
        return None;
    }
    let x = surface_width as f32 - MINIMAP_RIGHT - MINIMAP_WIDTH;
    (x > TEXT_LEFT + TEXT_RIGHT_GAP).then_some(x)
}

/// Where editor content stops and the status band begins (Q#S3) —
/// the single source for every bottom-of-text computation.
fn text_area_bottom(surface_height: u32, fm: FontMetrics) -> f32 {
    (surface_height as f32 - fm.status_band_height()).max(0.0)
}

/// The minimap's drawable height: the text area minus its own
/// top/bottom insets.
fn minimap_height(surface_height: u32, fm: FontMetrics) -> f32 {
    text_area_bottom(surface_height, fm) - MINIMAP_TOP - MINIMAP_BOTTOM
}

fn estimated_visible_lines(surface_height: u32, fm: FontMetrics) -> usize {
    ((text_area_bottom(surface_height, fm) - TEXT_TOP.max(0.0)) / fm.code_line_height())
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
) -> bool {
    let Some(left) = minimap_left(surface_width) else {
        return false;
    };
    let height = minimap_height(surface_height, fm);
    height > 0.0
        && x >= left
        && x < surface_width as f32 - MINIMAP_RIGHT
        && y >= MINIMAP_TOP
        && y < MINIMAP_TOP + height
}

/// The TUI mode line's scroll readout, ported verbatim (Q#S1): "All"
/// when the buffer fits, "Top"/"Bot" at the extremes, else the cursor
/// row as a percentage of the file.
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
fn edge_scroll_direction(y: f32, surface_height: u32, fm: FontMetrics) -> Option<i64> {
    if y < TEXT_TOP + EDGE_SCROLL_BAND {
        Some(-1)
    } else if y > text_area_bottom(surface_height, fm) - EDGE_SCROLL_BAND {
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
) -> Option<usize> {
    if total_lines == 0 {
        return None;
    }
    let height = minimap_height(surface_height, fm);
    if height <= 0.0 {
        return None;
    }
    let frac = ((y - MINIMAP_TOP) / height).clamp(0.0, 1.0);
    Some(((frac * total_lines as f32) as usize).min(total_lines - 1))
}

fn minimap_rects(
    lines: &[CellStyle],
    shapes: &[MinimapLineShape],
    surface_width: u32,
    surface_height: u32,
    first_visible_line: usize,
    visible_lines: usize,
    fm: FontMetrics,
) -> Vec<MinimapRect> {
    let Some(x) = minimap_left(surface_width) else {
        return Vec::new();
    };
    if lines.is_empty() || minimap_height(surface_height, fm) <= 0.0 {
        return Vec::new();
    }
    let height = minimap_height(surface_height, fm);
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
    (count > 0).then_some(MinimapLineShape {
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
        ((col / 4) + 1) * 4
    } else {
        col + 1
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
        InstanceMessage::BlockAdornments { .. } => "BlockAdornments",
        InstanceMessage::FoldState { .. } => "FoldState",
        InstanceMessage::ResourceOffer { .. } => "ResourceOffer",
        InstanceMessage::DispatchIdle { .. } => "DispatchIdle",
        InstanceMessage::LineNumbers { .. } => "LineNumbers",
        InstanceMessage::CompletionPopup { .. } => "CompletionPopup",
        InstanceMessage::ThemeFacts { .. } => "ThemeFacts",
        InstanceMessage::FontFacts { .. } => "FontFacts",
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
    projected_rich_chunks(range_text, &spans, &adornments)
}

fn projected_rich_chunks(
    text: &str,
    spans: &[StyleSpan],
    adornments: &[InlineAdornment],
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
    chunks
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
            )));
        }

        assert_eq!(
            per_line, full,
            "per-line chunk walks must reproduce the full walk exactly \
             (text and colors, newlines excluded)"
        );

        // The boundary hint landed on line 0 (before its newline), not
        // line 1.
        let line0 = clipped_chunks_for_range(text, &spans, &adornments, 0, 10);
        assert!(
            line0.iter().any(|c| c.text == "<eol>"),
            "newline-anchored hint belongs to the line it terminates"
        );
        let line1 = clipped_chunks_for_range(text, &spans, &adornments, 11, 22);
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
        );

        assert_eq!(chunk_texts(&chunks), vec!["abcd"]);
    }

    #[test]
    fn adornment_anchor_past_end_clamps_to_end() {
        let chunks = projected_rich_chunks(
            "abcd",
            &[],
            &[adornment(99, AdornmentPlacement::AtOffset, "X")],
        );

        assert_eq!(chunk_texts(&chunks), vec!["abcd", "X"]);
    }

    #[test]
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
            FontMetrics::default()
        ));
        assert!(
            !minimap_band_contains(739.0, 100.0, 800, 600, FontMetrics::default()),
            "left of band"
        );
        assert!(
            !minimap_band_contains(788.0, 100.0, 800, 600, FontMetrics::default()),
            "right of band"
        );
        assert!(
            !minimap_band_contains(750.0, 5.0, 800, 600, FontMetrics::default()),
            "above band"
        );
        assert!(
            !minimap_band_contains(750.0, 563.0, 800, 600, FontMetrics::default()),
            "below band (status strip)"
        );
        // Too-narrow surfaces have no minimap at all.
        assert!(!minimap_band_contains(
            100.0,
            100.0,
            150,
            600,
            FontMetrics::default()
        ));

        // Inverse mapping: height = 550; 100 lines. Top → line 0,
        // bottom → last line, midpoint → ~half.
        assert_eq!(
            minimap_y_to_line(12.0, 600, 100, FontMetrics::default()),
            Some(0)
        );
        assert_eq!(
            minimap_y_to_line(561.9, 600, 100, FontMetrics::default()),
            Some(99)
        );
        assert_eq!(
            minimap_y_to_line(12.0 + 275.0, 600, 100, FontMetrics::default()),
            Some(50)
        );
        // Out-of-band y clamps rather than panics (scrubbing wanders).
        assert_eq!(
            minimap_y_to_line(0.0, 600, 100, FontMetrics::default()),
            Some(0)
        );
        assert_eq!(
            minimap_y_to_line(9999.0, 600, 100, FontMetrics::default()),
            Some(99)
        );
        assert_eq!(
            minimap_y_to_line(100.0, 600, 0, FontMetrics::default()),
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
            edge_scroll_direction(10.0, 600, FontMetrics::default()),
            Some(-1)
        );
        assert_eq!(
            edge_scroll_direction(39.9, 600, FontMetrics::default()),
            Some(-1)
        );
        assert_eq!(
            edge_scroll_direction(40.0, 600, FontMetrics::default()),
            None,
            "interior"
        );
        assert_eq!(
            edge_scroll_direction(300.0, 600, FontMetrics::default()),
            None
        );
        assert_eq!(
            edge_scroll_direction(550.0, 600, FontMetrics::default()),
            None,
            "band edge exclusive"
        );
        assert_eq!(
            edge_scroll_direction(551.0, 600, FontMetrics::default()),
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

        let rects = minimap_rects(&lines, &shapes, 240, 120, 0, 30, FontMetrics::default());

        let pixel_rows = (120.0 - MINIMAP_TOP - MINIMAP_BOTTOM).round() as usize;
        assert!(
            rects.len() <= pixel_rows + 4,
            "minimap must bucket by visible rows, not emit per source line"
        );
    }

    #[test]
    fn minimap_hidden_when_surface_is_too_narrow() {
        let lines = [style_with_fg(CellColor::Rgb(255, 0, 0))];
        let shapes = [MinimapLineShape {
            indent_cols: 0,
            content_cols: 10,
        }];

        assert!(minimap_rects(&lines, &shapes, 120, 120, 0, 1, FontMetrics::default()).is_empty());
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

        let rects = minimap_rects(&[red, red], &shapes, 240, 80, 0, 2, FontMetrics::default());
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
        let band_top = text_area_bottom(h, FontMetrics::default()).floor() as u32;
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
            candidates: Vec::new(),
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
            candidates: vec!["theme-set".into(), "theme-clear".into()],
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
}
