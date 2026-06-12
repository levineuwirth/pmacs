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

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use loro::{ContainerTrait, ExportMode};
use pmacs_protocol::{
    AdornmentContent, AdornmentPlacement, BufferId, ByteRange, CrdtOp, Decoration, DecorationKind,
    DecorationSegment, FrontendId, InlineAdornment, InstanceMessage, Key as ProtocolKey, Modifiers,
    PointerKind, SelectionSnapshot, StyleSegment, StyleSpan,
    cell::{Color as CellColor, Style as CellStyle},
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
const CODE_LINE_HEIGHT: f32 = 22.0;
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
const STATUS_BAND_HEIGHT: f32 = 26.0;
const STATUS_BAND_BG: [f32; 4] = [0.105, 0.105, 0.145, 1.0];
const STATUS_TEXT_PAD: f32 = 10.0;
const STATUS_FONT_SIZE: f32 = 13.0;
const STATUS_LINE_HEIGHT: f32 = 18.0;
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
    window: Arc<Window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    quad_renderer: QuadRenderer,
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
    /// Minimap vertex bytes cached by [`MinimapCacheKey`] —
    /// rebuilding rescanned every line shape per frame.
    minimap_cache: Option<(MinimapCacheKey, Vec<u8>)>,
}

/// The wire-authoritative status facts (Q#S1, protocol v8),
/// mirrored from `InstanceMessage::StatusFacts`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct StatusFactsLocal {
    buffer_id: BufferId,
    name: String,
    modified: bool,
    diag_errors: u32,
    diag_warnings: u32,
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
        if let Err(e) = client.send_pointer(buffer_id, byte, kind, mods) {
            eprintln!("pmacs-gpu: send_pointer failed: {e}");
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
                        state.set_text("(attach failed; see stderr)");
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
                // Escape stays a local quit (no daemon round trip).
                if matches!(key.logical_key, Key::Named(NamedKey::Escape)) {
                    event_loop.exit();
                    return;
                }
                // Session B2 forwards cursor motion + plain text editing
                // (Char / Backspace / Enter / Delete / Tab). Ctrl/Alt/
                // Meta chords are withheld — they drive commands and
                // minibuffer flows the GUI can't render or interact with
                // yet (a later session adds GUI minibuffer + chords).
                if let Some((pkey, pmods)) = translate_key(&key.logical_key, self.modifiers)
                    && should_forward_key(pkey, pmods)
                    && let Some(client) = self.attach_client.as_ref()
                {
                    if let Some(op) = self.state.as_mut().and_then(|state| {
                        state
                            .optimistic_crdt_insert(pkey, pmods)
                            .or_else(|| state.optimistic_crdt_delete(pkey, pmods))
                    }) {
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
                        // An optimistic Enter near the bottom edge can
                        // scroll; re-declare the scoped viewport so
                        // the producer styles the newly visible lines.
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
                    edge_scroll_direction(position.y as f32, state.config.height);
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
                        (-(p.y as f32) / CODE_LINE_HEIGHT).round() as i64
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
                state.window.request_redraw();
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
/// `Enter` and `Tab` qualify alongside printable chars because their
/// default bindings (`buffer.newline` / `buffer.tab`) reduce to plain
/// `insert_char(10)` / `insert_char(9)` — byte-identical to a
/// self-insert, so the local application cannot diverge from what the
/// daemon will do with the same op. Two caveats are the caller's job:
/// `optimistic_crdt_insert` round-trips when an own-window selection
/// is active (the daemon commands consume the region first — CUA
/// type-over — which a raw op can't), and modified variants (`S-RET`,
/// `C-TAB`, …) return `None` here: a keymap may bind them to anything.
fn optimistic_insert_text(key: ProtocolKey, mods: Modifiers, chbuf: &mut [u8; 4]) -> Option<&str> {
    if !is_plain_text_modifiers(mods) {
        return None;
    }
    match key {
        ProtocolKey::Char(ch) if !ch.is_control() => Some(ch.encode_utf8(chbuf)),
        ProtocolKey::Enter if mods.is_empty() => Some("\n"),
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

        let mut font_system = FontSystem::new();
        font_system.db_mut().load_font_data(JETBRAINS_MONO.to_vec());
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
        let mut atlas = TextAtlas::new(&device, &queue, &cache, surface_format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);
        let quad_renderer = QuadRenderer::new(&device, surface_format);

        // Smaller font in attach mode (file contents tend to be more
        // than one line); larger only fits "hello, pmacs"-shaped
        // strings. Picked metrics that look reasonable for code at
        // 800px wide.
        let mut buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 22.0));
        buffer.set_size(
            &mut font_system,
            Some(config.width as f32),
            Some(config.height as f32),
        );
        let mut status_buffer = Buffer::new(
            &mut font_system,
            Metrics::new(STATUS_FONT_SIZE, STATUS_LINE_HEIGHT),
        );
        status_buffer.set_size(
            &mut font_system,
            Some(config.width as f32),
            Some(STATUS_BAND_HEIGHT),
        );
        let mut status_left_buffer = Buffer::new(
            &mut font_system,
            Metrics::new(STATUS_FONT_SIZE, STATUS_LINE_HEIGHT),
        );
        status_left_buffer.set_size(
            &mut font_system,
            Some(config.width as f32),
            Some(STATUS_BAND_HEIGHT),
        );
        buffer.set_text(
            &mut font_system,
            initial_text,
            &Attrs::new().family(Family::Name("JetBrains Mono")),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut font_system, false);
        let (current_line_starts, current_line_char_starts) = line_offset_tables(initial_text);

        Self {
            window,
            device,
            queue,
            surface,
            config,
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            quad_renderer,
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
            caret_vertex_buffer: ReusableVertexBuffer::new(),
            minimap_vertex_buffer: ReusableVertexBuffer::new(),
            status_buffer,
            status_text: String::new(),
            status_left_buffer,
            status_left_text: String::new(),
            status_facts: None,
            minimap_cache: None,
        }
    }

    fn set_frontend_id(&mut self, frontend_id: FrontendId) {
        self.local_frontend_id = Some(frontend_id);
        if let Some(doc) = self.loro_doc.as_ref()
            && let Err(e) = doc.set_peer_id(frontend_id.0)
        {
            eprintln!("pmacs-gpu: failed to set local Loro peer id: {e:?}");
        }
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
        // `CursorByte` confirms — an optimistic Enter on the bottom
        // visible line (or a Backspace pulling the caret above the
        // top) moves it outside the slice, and waiting a round trip
        // to scroll reads as a hitch.
        let viewport = if self.scroll_to_cursor() {
            self.rebuild_lines_reusing_scroll();
            self.viewport_send_if_changed(predicted.buffer_id)
        } else {
            None
        };
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
                self.cursor_fresh = false;
                self.optimistic_cursor_floor = None;
                self.optimistic_floor_set_at = None;
                self.deferred_round_trip_keys.clear();
                self.unconfirmed_edits.clear();
                // New buffer ⇒ back to the top, and force a viewport
                // re-declaration for the new buffer's scoped range.
                self.scroll_top = 0;
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
                self.window.request_redraw();
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
            // Q#S1 (protocol v8) — the wire-authoritative half of the
            // status band: name, modified, whole-file diag counts.
            InstanceMessage::StatusFacts {
                buffer_id,
                name,
                modified,
                diag_errors,
                diag_warnings,
            } => {
                self.status_facts = Some(StatusFactsLocal {
                    buffer_id,
                    name,
                    modified,
                    diag_errors,
                    diag_warnings,
                });
                self.window.request_redraw();
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
                self.window.request_redraw();
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
                if moved && self.scroll_to_cursor() {
                    // Pure scroll: retained lines keep their shape
                    // caches; only newly exposed lines shape.
                    self.rebuild_lines_reusing_scroll();
                    if let Some(vp) = self.viewport_send_if_changed(buffer_id) {
                        return Some(vp);
                    }
                }
                self.window.request_redraw();
                None
            }
            InstanceMessage::DispatchIdle { idle } => {
                self.dispatch_idle = idle;
                None
            }
            _ => None,
        }
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
        let visible = estimated_visible_lines(self.config.height).max(1);
        let old = self.scroll_top;
        if cursor_line < self.scroll_top {
            self.scroll_top = cursor_line;
        } else if cursor_line >= self.scroll_top + visible {
            self.scroll_top = cursor_line + 1 - visible;
        }
        self.scroll_top != old
    }

    /// Resolve a window-pixel position to an **absolute source byte**
    /// (Q#M2): pixel → cosmic-text hit (shaped line + byte within
    /// line) → projected byte → run map → slice byte → + `vstart`.
    /// `None` when no buffer is attached or the position is outside
    /// anything hit-testable.
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
        let rel_x = x as f32 - TEXT_LEFT;
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
        if new_top == self.scroll_top {
            return None;
        }
        self.scroll_top = new_top;
        self.rebuild_lines_reusing_scroll();
        self.current_buffer_id
            .and_then(|bid| self.viewport_send_if_changed(bid))
    }

    /// True when the pixel position lies inside the minimap band
    /// (Q#M6). Presses here are consumed locally and never become
    /// `Pointer` events.
    fn in_minimap_band(&self, x: f64, y: f64) -> bool {
        minimap_band_contains(x as f32, y as f32, self.config.width, self.config.height)
    }

    /// Center the viewport on the source line the minimap pixel `y`
    /// maps to — the inverse of the painter's linear line→y
    /// interpolation. Reuses [`Self::scroll_by_lines`] for the
    /// clamp / rebuild / viewport-send plumbing.
    fn minimap_jump_to(&mut self, y: f64) -> Option<ViewportSend> {
        let target =
            minimap_y_to_line(y as f32, self.config.height, self.current_line_starts.len())?;
        let centered = target.saturating_sub(estimated_visible_lines(self.config.height) / 2);
        self.scroll_by_lines(centered as i64 - self.scroll_top as i64)
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
            self.window.request_redraw();
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
        self.buffer.lines[shaped_idx] = line_from_chunks(&chunks);
        self.line_chunk_cache[shaped_idx] = chunks;
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        self.view_range = (vstart, vend);
        self.hit_map_dirty = true;
        self.window.request_redraw();
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
                lines.push(line_from_chunks(&chunks));
                cache.push(chunks);
            }
        }
        self.buffer.lines = lines;
        self.line_chunk_cache = cache;
        self.shaped_top = new_top;
        self.buffer
            .set_scroll(glyphon::cosmic_text::Scroll::default());
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        self.hit_map_dirty = true;
        if any_reused || self.line_chunk_cache.is_empty() {
            self.styled_redraw_deadline = None;
            self.window.request_redraw();
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
                self.buffer.lines[i] = line_from_chunks(&chunks);
                self.line_chunk_cache[i] = chunks;
                any = true;
            }
        }
        if any {
            self.buffer.shape_until_scroll(&mut self.font_system, false);
            self.hit_map_dirty = true;
        }
        // Fresh styling reached the slice — release any held
        // post-jump frame (Q#M6, bet #2).
        self.styled_redraw_deadline = None;
        self.window.request_redraw();
    }

    /// Compose the status-band readout (Q#S1): diagnostic counts
    /// (wire-authoritative, severity-colored, omitted when zero),
    /// then cursor L:C from the *optimistic* caret (so it tracks
    /// typing bursts instead of lagging a round trip), then the
    /// All/Top/Bot/NN% scroll indicator. Returns the colored spans.
    fn compose_status_spans(&self) -> Vec<(String, Option<Color>)> {
        let mut spans: Vec<(String, Option<Color>)> = Vec::new();
        if let Some(facts) = self
            .status_facts
            .as_ref()
            .filter(|f| Some(f.buffer_id) == self.current_buffer_id)
        {
            if facts.diag_errors > 0 {
                spans.push((
                    format!("E:{}", facts.diag_errors),
                    Some(Color::rgb(241, 76, 76)),
                ));
            }
            if facts.diag_warnings > 0 {
                spans.push((
                    format!("W:{}", facts.diag_warnings),
                    Some(Color::rgb(245, 245, 67)),
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
            readout.push_str(&format!("L{}:C{}", line + 1, col + 1));
            readout.push_str("  ");
        }
        readout.push_str(&format_scroll_indicator(
            self.scroll_top,
            estimated_visible_lines(self.config.height),
            self.current_line_starts.len(),
            cursor_row,
        ));
        spans.push((readout, None));
        spans
    }

    /// The band's left side: buffer name + modified dot, from the
    /// v8 `StatusFacts` (empty until the daemon ships them).
    fn compose_status_left(&self) -> String {
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
        let default_attrs = Attrs::new().family(Family::Name("JetBrains Mono"));
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
        let rect = MinimapRect {
            x: 0.0,
            y: text_area_bottom(self.config.height),
            w: self.config.width as f32,
            h: STATUS_BAND_HEIGHT,
            color: STATUS_BAND_BG,
        };
        rects_to_vertex_bytes(&[rect], self.config.width, self.config.height)
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
        self.window.request_redraw();
    }

    /// `full = true` path: discard prior styling, take the segments'
    /// spans as authoritative for the declared viewport.
    fn replace_style_spans(&mut self, segments: Vec<StyleSegment>) {
        self.current_spans.clear();
        for seg in segments {
            self.current_spans.extend(seg.spans);
        }
        self.current_spans.sort_by_key(|s| s.range.start);
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
        let span = estimated_visible_lines(self.config.height).max(1) + SCROLL_OVERSCAN;
        let vstart = line_starts[top];
        let bottom = top.saturating_add(span).min(n);
        let vend = if bottom < n {
            line_starts[bottom]
        } else {
            self.current_text.len() as u64
        };
        (vstart, vend)
    }

    fn reshape(&mut self) {
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
            lines.push(line_from_chunks(&chunks));
            cache.push(chunks);
        }
        self.buffer.lines = lines;
        self.line_chunk_cache = cache;
        self.shaped_top = top;
        self.buffer
            .set_scroll(glyphon::cosmic_text::Scroll::default());
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        // The pointer hit map rebuilds lazily from the same caches
        // (Q#R2) — clicks are rare next to keystrokes/frames.
        self.hit_map_dirty = true;
        // Full restyle: release any held post-jump frame (Q#M6).
        self.styled_redraw_deadline = None;
        self.window.request_redraw();
    }

    fn resize(&mut self, width: u32, height: u32) -> Option<ViewportSend> {
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.viewport
            .update(&self.queue, Resolution { width, height });
        self.buffer.set_size(
            &mut self.font_system,
            Some(width as f32),
            Some(height as f32),
        );
        self.status_buffer.set_size(
            &mut self.font_system,
            Some(width as f32),
            Some(STATUS_BAND_HEIGHT),
        );
        self.status_left_buffer.set_size(
            &mut self.font_system,
            Some(width as f32),
            Some(STATUS_BAND_HEIGHT),
        );
        // A taller/shorter window changes the visible line count, so the
        // slice + scoped viewport change (session S1).
        self.reshape();
        self.window.request_redraw();
        self.current_buffer_id
            .and_then(|bid| self.viewport_send_if_changed(bid))
    }

    #[allow(clippy::too_many_lines)] // linear per-frame GPU sequence + optional timing.
    fn render(&mut self) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Validation => {
                eprintln!("surface acquisition raised a validation error");
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let frame_start = debug_frame().then(std::time::Instant::now);
        self.refresh_status_line();
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
        let status_top =
            text_area_bottom(self.config.height) + (STATUS_BAND_HEIGHT - STATUS_LINE_HEIGHT) / 2.0;
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
                        left: TEXT_LEFT,
                        top: TEXT_TOP,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: 0,
                            top: 0,
                            right: text_bounds_right,
                            // Clip at the status band (Q#S3): a final
                            // partially-visible line must not bleed
                            // into the band.
                            bottom: text_area_bottom(self.config.height).round() as i32,
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
                            top: text_area_bottom(self.config.height).round() as i32,
                            right: self.config.width.cast_signed(),
                            bottom: self.config.height.cast_signed(),
                        },
                        default_color: Color::rgb(168, 168, 180),
                        custom_glyphs: &[],
                    },
                    TextArea {
                        buffer: &self.status_left_buffer,
                        left: STATUS_TEXT_PAD,
                        top: status_top,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: 0,
                            top: text_area_bottom(self.config.height).round() as i32,
                            // Stop before the right-aligned readout.
                            right: (status_left - STATUS_TEXT_PAD).max(0.0).round() as i32,
                            bottom: self.config.height.cast_signed(),
                        },
                        default_color: Color::rgb(200, 200, 210),
                        custom_glyphs: &[],
                    },
                ],
                &mut self.swash_cache,
            )
            .expect("text_renderer prepare");

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("pmacs-gpu frame encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("pmacs-gpu pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("text_renderer render");
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
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
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
        let visible_lines = estimated_visible_lines(self.config.height);
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
        rects_to_vertex_bytes(&rects, self.config.width, self.config.height)
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
            if let Some(color) = decoration_kind_to_bg_color(d.kind) {
                self.push_glyph_extent_rects(rects, line_offsets, lo, hi, color, None);
            }
            if let Some(color) = decoration_kind_to_underline_color(d.kind) {
                self.push_glyph_extent_rects(
                    rects,
                    line_offsets,
                    lo,
                    hi,
                    color,
                    Some(DIAG_UNDERLINE_PX),
                );
            }
        }
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
    /// Empty when no own cursor is known, it's in another buffer, or it
    /// is scrolled out of the visible slice.
    fn caret_vertex_bytes(&self) -> Vec<u8> {
        let (vstart, vend) = self.view_range;
        if vend <= vstart {
            return Vec::new();
        }
        let slice = &self.current_text[vstart as usize..vend as usize];
        let line_offsets = line_byte_offsets(slice);
        let Some(rect) = self.caret_rect(slice, &line_offsets, vstart, vend) else {
            return Vec::new();
        };
        rects_to_vertex_bytes(&[rect], self.config.width, self.config.height)
    }

    /// The caret rectangle for the own cursor, in slice coordinates: a
    /// thin bar at the left edge of the glyph the cursor sits before (or
    /// the right edge of the last glyph at line end). `None` when the
    /// cursor is outside the visible slice. Byte→glyph mapping rebases
    /// per line (QB3); the cursor is rebased onto the slice first (S1).
    fn caret_rect(
        &self,
        slice: &str,
        line_offsets: &[u64],
        vstart: u64,
        vend: u64,
    ) -> Option<MinimapRect> {
        let own = self.own_cursor?;
        if self.current_buffer_id != Some(own.buffer_id) {
            return None;
        }
        let cursor = own.byte;
        if cursor < vstart || cursor > vend {
            return None; // scrolled off-screen
        }
        let slice_cursor = cursor - vstart;
        let (line_lo, _) = source_line_range(slice, slice_cursor);
        for run in self.buffer.layout_runs() {
            if line_offsets.get(run.line_i).copied().unwrap_or(0) != line_lo {
                continue;
            }
            let line_base = line_lo;
            let mut x = TEXT_LEFT;
            for glyph in run.glyphs {
                if line_base + glyph.start as u64 >= slice_cursor {
                    x = TEXT_LEFT + glyph.x;
                    break;
                }
                // Cursor is past this glyph; track its right edge so a
                // cursor at line end lands after the final glyph.
                x = TEXT_LEFT + glyph.x + glyph.w;
            }
            return Some(MinimapRect {
                x,
                y: TEXT_TOP + run.line_top,
                w: CARET_WIDTH,
                h: run.line_height,
                color: CARET_COLOR,
            });
        }
        None
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
                // `bar_px`: an underline bar hugging the bottom of the
                // line box instead of a full-height wash — the GPU's
                // diagnostic squiggle (T M4.6 parity, straight-bar v1).
                let (y, h) = match bar_px {
                    Some(bar) => (TEXT_TOP + run.line_top + run.line_height - bar, bar),
                    None => (TEXT_TOP + run.line_top, run.line_height),
                };
                rects.push(MinimapRect {
                    x: TEXT_LEFT + x0,
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
fn text_area_bottom(surface_height: u32) -> f32 {
    (surface_height as f32 - STATUS_BAND_HEIGHT).max(0.0)
}

/// The minimap's drawable height: the text area minus its own
/// top/bottom insets.
fn minimap_height(surface_height: u32) -> f32 {
    text_area_bottom(surface_height) - MINIMAP_TOP - MINIMAP_BOTTOM
}

fn estimated_visible_lines(surface_height: u32) -> usize {
    ((text_area_bottom(surface_height) - TEXT_TOP.max(0.0)) / CODE_LINE_HEIGHT)
        .ceil()
        .max(1.0) as usize
}

/// True when `(x, y)` lies inside the minimap band — the painter's
/// geometry (`minimap_left` × the `MINIMAP_TOP..bottom` column),
/// shared by the Q#M6 press hit-test.
fn minimap_band_contains(x: f32, y: f32, surface_width: u32, surface_height: u32) -> bool {
    let Some(left) = minimap_left(surface_width) else {
        return false;
    };
    let height = minimap_height(surface_height);
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
fn edge_scroll_direction(y: f32, surface_height: u32) -> Option<i64> {
    if y < TEXT_TOP + EDGE_SCROLL_BAND {
        Some(-1)
    } else if y > text_area_bottom(surface_height) - EDGE_SCROLL_BAND {
        Some(1)
    } else {
        None
    }
}

/// Map a minimap pixel `y` to a whole-file source line — the inverse
/// of the painter's `y = MINIMAP_TOP + line * height / total`
/// interpolation, clamped into the file. `None` for an empty file or
/// a degenerate surface.
fn minimap_y_to_line(y: f32, surface_height: u32, total_lines: usize) -> Option<usize> {
    if total_lines == 0 {
        return None;
    }
    let height = minimap_height(surface_height);
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
) -> Vec<MinimapRect> {
    let Some(x) = minimap_left(surface_width) else {
        return Vec::new();
    };
    if lines.is_empty() || minimap_height(surface_height) <= 0.0 {
        return Vec::new();
    }
    let height = minimap_height(surface_height);
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
        InstanceMessage::BlockAdornments { .. } => "BlockAdornments",
        InstanceMessage::FoldState { .. } => "FoldState",
        InstanceMessage::ResourceOffer { .. } => "ResourceOffer",
        InstanceMessage::DispatchIdle { .. } => "DispatchIdle",
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
        if let Some(range) = translate_byte_range(decoration.range, edit) {
            decoration.range = range;
            translated.push(decoration);
        }
    }
    *decorations = translated;
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
fn line_from_chunks(chunks: &[RichChunk]) -> glyphon::cosmic_text::BufferLine {
    let default_attrs = Attrs::new().family(Family::Name("JetBrains Mono"));
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

fn source_color_at(byte: u64, spans: &[StyleSpan]) -> Option<glyphon::Color> {
    for sp in spans {
        if sp.range.start <= byte && byte < sp.range.end {
            return cell_color_to_glyphon(sp.style.fg);
        }
    }
    None
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

/// Height of the diagnostic underline bar, in pixels. Straight-bar
/// v1; a wavy squiggle needs shader/texture work and waits until the
/// straight bar is proven (framing Q#D1).
const DIAG_UNDERLINE_PX: f32 = 2.0;

/// Map a [`DecorationKind`] to an underline-bar color, or `None` for
/// kinds that don't underline.
///
/// Session 5 originally rendered diagnostics by *recoloring the text
/// foreground*, which clobbered the syntax color of the very token
/// the diagnostic points at — the same flaw the TUI fixed with
/// protocol v6's `underline_color` (T M4.6). The GPU's equivalent is
/// a [`DIAG_UNDERLINE_PX`]-tall quad hugging the bottom of the glyph
/// extent; the text keeps its syntax color. Same RGB palette the fg
/// path used (red / yellow / light blue / dim gray), so the window's
/// severity language is unchanged.
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
        // Deferred to the search-feature arc.
        DecorationKind::SearchMatch | DecorationKind::SearchMatchActive => None,
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
    fn optimistic_insert_text_covers_plain_chars_enter_and_tab() {
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
            Some("\n"),
            "RET is bound to buffer.newline = insert_char(10): identical to a self-insert"
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
    fn source_line_range_handles_empty_and_leading_newline() {
        assert_eq!(source_line_range("", 0), (0, 0));
        // "\nx": cursor 0 is on the empty first line [0, 1).
        assert_eq!(source_line_range("\nx", 0), (0, 1));
        // cursor 1 is on line 1 → [1, 2).
        assert_eq!(source_line_range("\nx", 1), (1, 2));
    }

    #[test]
    fn bg_color_helper_covers_selection_and_current_line() {
        // Sessions 9.1 + 9.2: Selection and CurrentLine paint.
        assert!(decoration_kind_to_bg_color(DecorationKind::Selection).is_some());
        assert!(decoration_kind_to_bg_color(DecorationKind::CurrentLine).is_some());

        // Search-feature arc — still deferred.
        assert!(decoration_kind_to_bg_color(DecorationKind::SearchMatch).is_none());
        assert!(decoration_kind_to_bg_color(DecorationKind::SearchMatchActive).is_none());

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
            // Both helpers return None for the search pair — deferred
            // to the search-feature arc. That is the "neither yet"
            // state — the exclusive-or test exempts it.
            let deferred = matches!(
                kind,
                DecorationKind::SearchMatch | DecorationKind::SearchMatchActive
            );
            assert!(
                deferred || (ul ^ bg),
                "{kind:?}: underline={ul} bg={bg} — should be exactly one (unless deferred)"
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
        assert!(minimap_band_contains(750.0, 100.0, 800, 600));
        assert!(
            !minimap_band_contains(739.0, 100.0, 800, 600),
            "left of band"
        );
        assert!(
            !minimap_band_contains(788.0, 100.0, 800, 600),
            "right of band"
        );
        assert!(!minimap_band_contains(750.0, 5.0, 800, 600), "above band");
        assert!(
            !minimap_band_contains(750.0, 563.0, 800, 600),
            "below band (status strip)"
        );
        // Too-narrow surfaces have no minimap at all.
        assert!(!minimap_band_contains(100.0, 100.0, 150, 600));

        // Inverse mapping: height = 550; 100 lines. Top → line 0,
        // bottom → last line, midpoint → ~half.
        assert_eq!(minimap_y_to_line(12.0, 600, 100), Some(0));
        assert_eq!(minimap_y_to_line(561.9, 600, 100), Some(99));
        assert_eq!(minimap_y_to_line(12.0 + 275.0, 600, 100), Some(50));
        // Out-of-band y clamps rather than panics (scrubbing wanders).
        assert_eq!(minimap_y_to_line(0.0, 600, 100), Some(0));
        assert_eq!(minimap_y_to_line(9999.0, 600, 100), Some(99));
        assert_eq!(minimap_y_to_line(100.0, 600, 0), None, "empty file");
    }

    #[test]
    fn edge_scroll_direction_bands() {
        // 600px surface: up-band y < 16 + 24 = 40; the text area
        // ends at 574 (status band, Q#S3), so the down-band is
        // y > 574 - 24 = 550.
        assert_eq!(edge_scroll_direction(10.0, 600), Some(-1));
        assert_eq!(edge_scroll_direction(39.9, 600), Some(-1));
        assert_eq!(edge_scroll_direction(40.0, 600), None, "interior");
        assert_eq!(edge_scroll_direction(300.0, 600), None);
        assert_eq!(
            edge_scroll_direction(550.0, 600),
            None,
            "band edge exclusive"
        );
        assert_eq!(edge_scroll_direction(551.0, 600), Some(1));
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
        let rects = minimap_rects(&[red, red, blue, blue], &shapes, 240, 80, 0, 2);

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

        let rects = minimap_rects(&lines, &shapes, 240, 120, 0, 30);

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

        assert!(minimap_rects(&lines, &shapes, 120, 120, 0, 1).is_empty());
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

        let rects = minimap_rects(&[red, red], &shapes, 240, 80, 0, 2);
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
}
