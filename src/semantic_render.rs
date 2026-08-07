// semantic_render.rs --- Instance-side semantic projection (T M11.2).

//! The semantic projection seam.
//!
//! [`crate::instance_render::RenderState`] rasterizes the editor to a
//! cell grid and ships [`InstanceMessage::CellDelta`]. `SemanticRenderState`
//! is its sibling for `semantic_render` sessions: it reads the same
//! [`EditorState`] but exits the pipeline *earlier* — it emits the
//! structured byte-range styling the cell painter would otherwise have
//! consumed, mapped through the active [`crate::highlight::Theme`],
//! without the grid-packing step. Styling has one authority per
//! language (policy A): tree-sitter spans from [`crate::syntax`] for
//! grammar-backed languages, LSP semantic tokens
//! ([`crate::lsp::LspManager::semantic_style_context`]) for languages
//! with no bundled grammar (C/C++, …). The frontend lays the styling
//! out locally over rope text it already holds via its `crdt_replica`
//! `BufferMirror`.
//!
//! Contract boundary (see `docs/semantic-frontend-protocol.md`): the
//! instance never learns a pixel. The only spatial fact it consumes is
//! the buffer byte range the frontend declared on screen via
//! [`crate::protocol::FrontendEvent::Viewport`]; styling is scoped to
//! that range so a 100k-line file's styling is never shipped wholesale.
//!
//! Produced families: `StyleSpans` (M11.2; dual authority per above)
//! and `Decorations` (M11.3), both span-granularity diffed (M11.4);
//! `InlineAdornments` (Step 3, from the LSP inlay-hint store,
//! M11.2-level suppression); `FileStyleSummary` (resolving Open Q#2 —
//! per-line dominant style for a minimap, generation-keyed); `FoldState`
//! (Arc 6 — the instance's authoritative fold set, authoritative-empty).
//! `BlockAdornments` / `ResourceOffer` remain wire-declared but
//! unproduced.

use std::collections::HashMap;

use crate::buffer::BufferId;
use crate::cell::{CellSize, Style};
use crate::editor::EditorState;
use crate::protocol::{
    AdornmentContent, AdornmentPlacement, ByteRange, Decoration, DecorationKind, DecorationSegment,
    FrontendId, InlineAdornment, InstanceMessage, MenuPromptRow, PANEL_MIN_VERSION,
    StatuslineSegment, StyleSegment, StyleSpan,
};
use crate::statusline::{
    StatuslineEvaluation, StatuslineEvaluationOutcome, StatuslineEvaluationTarget,
    StatuslineWindowSegments, evaluate_statusline,
};
use crate::terminal::TerminalFrame;
use crate::window::WindowId;
use pmacs_protocol::panel::{PanelFrame, PanelFramePayload};

/// The viewport a `semantic_render` frontend last declared.
#[derive(Clone, Debug, Eq, PartialEq)]
struct DeclaredViewport {
    buffer_id: BufferId,
    visible: ByteRange,
    /// The CRDT generation the frontend computed `visible` against.
    /// Recorded for the M11.4 "ignore a viewport that races a
    /// not-yet-applied edit" refinement; M11.2 always honors the most
    /// recent declaration verbatim.
    frontend_generation: u64,
}

/// The diff baseline for one family on one buffer: the
/// declared-viewport region the set was computed for, the full
/// scoped item set last shipped, and the CRDT generation that set
/// was computed against. The next frame diffs against `items`;
/// `visible` changing (or no entry) forces a `full` resync, and
/// `generation` changing also forces a `full` — see T M11.7.
///
/// **T M11.7 — `generation`-tracked full-resync.** Without this,
/// edits broke the consumer's incremental-update contract:
/// `changed_intervals` only ships dirty-range items on `full=false`
/// frames, but after a text-shift the frontend's cached spans
/// (indexed by *pre-edit* byte positions) need to be replaced
/// wholesale — the post-edit positions have shifted under them.
/// Forcing `full=true` on every generation transition makes the
/// next emission a `replace_*` on the frontend side, which is the
/// correct behavior. Cost: one extra full-viewport ship per edit;
/// negligible on a local Unix socket and bounded by the viewport
/// size.
struct LastFrame<T> {
    visible: ByteRange,
    items: Vec<T>,
    generation: u64,
}

/// Cached `SearchPrompt` payload for cached-compare suppression
/// (Q#SR5 / Q#RX6): `(query, active, total, regex, invalid)`. A `None`
/// query means the last emission cleared the band.
type SearchPromptFacts = (Option<String>, Option<u32>, u32, bool, bool);

/// Cached `MenuPrompt` payload for cached-compare suppression (Q#CM1):
/// `(rows, active)`. Empty `rows` means the last emission closed the
/// menu.
type MenuPromptFacts = (Vec<MenuPromptRow>, Option<u32>);

/// Cached `MinibufferPrompt` payload for cached-compare suppression
/// (Q#MB1): `(prompt, input, cursor, candidates-window, selected, total)`.
/// A `None` prompt means the minibuffer is closed.
type MinibufferFacts = (Option<String>, String, u32, Vec<String>, Option<u32>, u32);

/// Cached `CompletionPopup` payload for cached-compare suppression
/// (Arc 1a Q#C5): `(anchor, prefix_len, rows-window, selected, total)`.
/// A `None` anchor means the popup is closed.
type CompletionPopupFacts = (
    Option<u64>,
    u32,
    Vec<crate::protocol::CompletionPopupRow>,
    Option<u32>,
    u32,
);

/// How many completion candidates the minibuffer ships per frame — a
/// scrolled window around the selection, not the full (≤1024) list.
const MB_VISIBLE: usize = 10;

/// A window of up to [`MB_VISIBLE`] candidates around `selected`, plus
/// the selection's index *within* that window. Keeps the selected row
/// visible as the user cycles a long list.
fn minibuffer_window(candidates: &[String], selected: Option<usize>) -> (Vec<String>, Option<u32>) {
    if candidates.is_empty() {
        return (Vec::new(), None);
    }
    let sel = selected.unwrap_or(0).min(candidates.len() - 1);
    let start = sel
        .saturating_sub(MB_VISIBLE / 2)
        .min(candidates.len().saturating_sub(MB_VISIBLE));
    let end = (start + MB_VISIBLE).min(candidates.len());
    let window = candidates[start..end].to_vec();
    let selected_in_window = selected.map(|s| (s - start) as u32);
    (window, selected_in_window)
}

/// Owns one `semantic_render` session's projection state: the last
/// viewport the frontend declared, and the diff baseline per buffer
/// for the `StyleSpans` and `Decorations` families.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent per-peer capability and latch flags"
)]
pub struct SemanticRenderState {
    /// The session this projection serves. Selection is per-window
    /// (per-frontend) state, so the decoration projection needs the
    /// fid to resolve *this* session's active window via
    /// `active_window_for`. Styling and diagnostics are per-buffer and
    /// do not consult it.
    frontend_id: FrontendId,
    /// `None` until the frontend's first [`Self::set_viewport`]. While
    /// `None`, [`Self::render_frame`] emits nothing: the frontend
    /// bootstraps its rope from `BufferSnapshot`, declares what is on
    /// screen, and only then receives styling for exactly that range.
    viewport: Option<DeclaredViewport>,
    /// Styling diff baseline, keyed by buffer (T M11.4). An unchanged
    /// frame ships nothing; a changed frame ships only the dirty
    /// byte-range segments.
    last_sent: HashMap<BufferId, LastFrame<StyleSpan>>,
    /// Decorations diff baseline, tracked independently of `last_sent`
    /// so a styling change does not force a decorations re-send and
    /// vice versa.
    last_decorations: HashMap<BufferId, LastFrame<Decoration>>,
    /// `InlineAdornments` baseline (T M11 producer arc, Step 3). The
    /// wire variant carries no `generation`/`full`/`segments`, so
    /// unlike the two families above this is only M11.2-level
    /// suppression: a whole-set re-send on any change, nothing when
    /// byte-identical. `LastFrame::items` reuse keeps the shape
    /// uniform even though no segment diffing applies.
    last_adornments: HashMap<BufferId, LastFrame<InlineAdornment>>,
    /// `FoldState` baseline (Arc 6). Whole-buffer, not viewport-clipped,
    /// so a plain `Vec<ByteRange>` per buffer suffices. Authoritative-
    /// empty and diff-suppressed: the first sight of a buffer emits only
    /// if a fold exists (no empty-frame spam), an unchanged set emits
    /// nothing, and a `non-empty → empty` transition emits exactly one
    /// empty frame so the frontend clears its fold mirror. Resets on
    /// `BufferSnapshot` — the snapshot's own frontend-side fold-mirror
    /// clear is what makes the empty-after-revert suppression correct
    /// (Q#FD8, #120 class).
    last_folds: HashMap<BufferId, Vec<ByteRange>>,
    /// `FileStyleSummary` baseline (post-M11 minimap producer,
    /// resolving design-note Open Q#2). The whole-file dominant-style
    /// summary is expensive to compute on a 100k-line file, so the
    /// producer short-circuits on the last sent CRDT generation: a
    /// buffer at the same generation re-uses what the frontend
    /// already has and emits nothing. First emission happens on the
    /// first frame for a buffer; further emissions only after edits.
    /// `(crdt_generation, diag_epoch, syntax_epoch, face_epoch)` the
    /// last summary was computed against, plus that computed payload.
    /// Diagnostics arrive without a generation bump, so the diag
    /// epoch catches republishes (minimap marks, T M4.6 GPU parity);
    /// the theme epochs (Q#TH6) catch mid-session recolors —
    /// `face_epoch` belongs in the key because `ui.diag.*` feeds the
    /// marks. The payload copy backs the Q#TH6 payload-equality
    /// suppression: a face edit that leaves the summary unchanged
    /// (e.g. `ui.modeline`) recomputes once per mutation and emits
    /// nothing. The key advances on COMPUTATION, not emission — a
    /// suppressed send still inserts, or the whole-file recompute
    /// repeats every tick.
    last_summary: HashMap<BufferId, SummaryCache>,
    /// `(name, modified, diag_errors, diag_warnings, message)` last
    /// emitted as `StatusFacts` (Q#S1; `message` since v15) —
    /// cached-compare suppression. A peer emission baseline ONLY:
    /// the diagnostic-count freeze deliberately holds no session
    /// state (rounds 3–4) — it is sourced from the diag store's
    /// retained vector, so it needs nothing here to survive the
    /// `on_buffer_snapshot_sent` reset and it holds for sessions
    /// with no history (a late joiner attaching mid-edit).
    last_status: HashMap<BufferId, (String, bool, u32, u32, Option<String>)>,
    /// Last-emitted line-number gutter mode (UX gutter arc, protocol v14) —
    /// cached-compare suppression. Seeded to `Some(Off)` (the frontend's
    /// default) so an off gutter never emits. Per-frontend (one value),
    /// since this state carries one frontend's `frontend_id`.
    last_line_numbers: Option<crate::window::LineNumberMode>,
    /// Last emitted `SearchPrompt` payload per buffer, for
    /// cached-compare suppression (see [`SearchPromptFacts`]).
    last_search_prompt: HashMap<BufferId, SearchPromptFacts>,
    /// Last emitted `MenuPrompt` payload per buffer (Q#CM1), for
    /// cached-compare suppression (see [`MenuPromptFacts`]).
    last_menu_prompt: HashMap<BufferId, MenuPromptFacts>,
    /// Last emitted `MinibufferPrompt` payload (Q#MB1) — a single value,
    /// not per-buffer, because the minibuffer is one global core
    /// instance.
    last_minibuffer: Option<MinibufferFacts>,
    /// Last emitted `CompletionPopup` payload per buffer (Arc 1a
    /// Q#C5), for cached-compare suppression (see
    /// [`CompletionPopupFacts`]).
    last_completion_popup: HashMap<BufferId, CompletionPopupFacts>,
    /// `StyleSpans` recompute gate (perf). `scoped_style_spans` runs
    /// the tree-sitter highlights query over the *whole declared
    /// viewport* (which the GPU frontend sets to the entire buffer)
    /// and clones the theme — too expensive to repeat on every tick.
    /// The styling depends only on the parse bundle, the CRDT
    /// generation, the viewport, and the theme's syntax epoch
    /// (Q#TH6) — never the cursor — so a gate built from those lets
    /// cursor-only ticks skip the query entirely while a mid-session
    /// `pmacs.theme.set` still re-ships recolored spans without an
    /// edit. Only the grammar (tree-sitter) path is gated; the
    /// LSP-token path has no comparably cheap handle and recomputes
    /// as before.
    last_style_gate: HashMap<BufferId, StyleGate>,
    /// The theme `face_epoch` the `ThemeFacts` producer last
    /// INSPECTED (Q#TH7) — `Option`, not a bare zero, because an
    /// unthemed daemon sits at `face_epoch == 0` and a `0 == 0`
    /// short-circuit would starve the first authoritative send.
    /// Advances on computation, not emission: an identical rebuild
    /// records the epoch it inspected even though nothing ships.
    last_face_epoch: Option<u64>,
    /// The face table the frontend believes (Q#TH7), seeded `None` so
    /// every attachment receives exactly one authoritative table —
    /// the empty table included — with its first emission after
    /// viewport declaration. A frontend retaining face state across
    /// attachments is therefore corrected even by an unthemed daemon.
    last_theme_faces: Option<Vec<crate::protocol::ThemeFace>>,
    /// For v18 peers, the enabled provider-face set epoch inspected by
    /// `theme_facts_msg`. Kept separate from `last_face_epoch` so
    /// priority-only provider changes do not rebuild the face table.
    /// v16/v17 peers never read the registry and leave this `None`.
    last_statusline_face_set_epoch: Option<u64>,
    /// Whether the peer negotiated protocol >= 16 (PR #120 round 1
    /// finding 3). Faces reach a semantic frontend through TWO
    /// channels: `ThemeFacts` (daemon write-loop gated) and the
    /// `ui.diag.*` colors folded into `FileStyleSummary` — an OLDER
    /// channel the version gate does not filter. A v15 peer must not
    /// receive face-derived minimap marks while its squiggles, signs,
    /// and counters stay unthemed, so this producer resolves faces
    /// only when the peer can apply the whole face table.
    peer_knows_theme_facts: bool,
    /// The font-pref `epoch` this producer last INSPECTED (Q#F5) —
    /// `Option`, not a bare zero, or an all-default daemon's `0 == 0`
    /// short-circuit would starve the first authoritative send.
    /// Advances on computation, not emission.
    last_font_epoch: Option<u64>,
    /// The preference the frontend believes (Q#F5), seeded `None` so
    /// every attachment receives exactly one authoritative
    /// `FontFacts` — the all-default `(None, None)` included.
    /// Bufferless: `on_buffer_snapshot_sent` never touches it.
    last_font_facts: Option<(Option<String>, Option<u32>)>,
    /// Whether the peer negotiated protocol >= 17 (Q#F4). Unlike the
    /// theme case there is no pre-v17 side channel that could leak
    /// font state, so this gate has no summary-style companion
    /// filter.
    peer_knows_font_facts: bool,
    /// `LineWrapFacts` is v22; a v21 peer keeps its own behavior.
    peer_knows_line_wrap: bool,
    /// Last `(buffer, wrap)` pair sent. Keyed on the PAIR, not the
    /// mode: that is what makes a BUFFER SWITCH re-emit without a
    /// config event, which a value-keyed cache would miss entirely.
    last_line_wrap: Option<(crate::buffer::BufferId, bool)>,
    /// Whether the peer negotiated protocol >= 18 (Q#SL7). This gates
    /// callback evaluation in the producer, independently of the daemon's
    /// write-loop gate.
    peer_knows_statusline_segments: bool,
    /// Complete replacement baseline per buffer. `None` means the peer has
    /// never received an authoritative payload, so the first empty result
    /// must still be emitted.
    last_statusline: HashMap<BufferId, (Vec<StatuslineSegment>, Vec<StatuslineSegment>)>,
    /// Cached byte↔line table for the diagnostics projection, keyed
    /// by buffer revision. Building it costs an O(buffer) rope copy
    /// plus a full scan; before this cache, that ran on *every tick*
    /// while diagnostics were on screen (the table is only consulted
    /// when the store is non-stale and non-empty) — a steady-state
    /// CPU burn for a value that changes only when the buffer does.
    diag_line_cache: HashMap<BufferId, DiagLineCache>,
    /// Whether the peer negotiated protocol >= 19 (Vterm Stage 3). A
    /// v18 semantic peer receives the terminal identity buffer's empty
    /// snapshot and nothing else: terminal use is unsupported and
    /// invisible there, while ordinary document editing is unchanged.
    peer_knows_terminal_frames: bool,
    /// The terminal-cell geometry this frontend last declared, or
    /// `None` before its first accepted `TerminalResize`. One value,
    /// not a map: a frontend displays at most one terminal at a time
    /// (its active window), and the buffer travels with the size so a
    /// declaration outliving a switch cannot project the wrong session.
    terminal_viewport: Option<(BufferId, CellSize)>,
    /// The last terminal frame this peer received, compared in FULL.
    ///
    /// Not keyed on `screen_generation`: selection, scroll, viewport,
    /// and process state all change without advancing that counter, so
    /// a generation-keyed baseline goes silent on exactly the view-only
    /// updates the frontend needs. `None` means the peer has received
    /// no frame, so the next valid one is authoritative.
    last_terminal_frame: Option<TerminalFrame>,
    /// Whether an invalid terminal snapshot was already reported since
    /// the last valid frame. Bounds the log to one line per distinct
    /// failure rather than one per tick while the condition persists.
    terminal_error_latched: bool,
    /// Whether the most recent render pass projected a terminal.
    terminal_active: bool,
    /// Whether the peer negotiated protocol v21, which is where
    /// [`InstanceMessage::PanelFrame`] was appended (Q#BP9). A v20 peer
    /// receives no band at all — and, per Q#BP13, is never *placed* in a
    /// side window either, because denying only the message would leave
    /// its window invisible.
    peer_knows_panel_frames: bool,
    /// The panel payload this peer last received, compared in FULL.
    ///
    /// Seeded to `Absent` rather than `None`: a fresh session starts with
    /// no band, so the opening state is a fact rather than an absence,
    /// and seeding it keeps every session from paying one redundant
    /// `Absent` before it ever shows a panel.
    ///
    /// `Absent` is authoritative and duplicate-suppressed like any other
    /// payload — hide and close must both send it, because the receiver
    /// retains its last valid frame and silence would leave a stale band
    /// on screen indefinitely (Q#BP15).
    last_panel_payload: Option<PanelFramePayload>,
    /// Highest presentation epoch allocated for this session; `0` means
    /// none has been. Advanced only when a frame is actually shipped, so
    /// a frame that fails validation does not burn an identity the peer
    /// never saw.
    panel_epoch_used: u64,
    /// Identity behind the `Present` in `last_panel_payload`, or `None`
    /// when the last payload was `Absent`.
    ///
    /// Cleared by every `Absent`, which is what makes hide/reopen and
    /// close/reopen of the **same** persistent buffer allocate a fresh
    /// epoch — the hole a `buffer_id` alone cannot close (Q#BP16).
    panel_presentation: Option<PanelPresentation>,
    /// Whether an invalid panel frame was already reported since the last
    /// valid one. Bounds the log exactly like `terminal_error_latched`.
    panel_error_latched: bool,
    /// The band's last PUBLISHED statusline segments and the side-window
    /// presentation they belong to (review rounds 1 and 2, R2-4).
    ///
    /// The band repaints its whole mode line every frame, so
    /// "publish nothing" has to be expressed as "paint what was published
    /// last" — there is no wire-level suppression to fall back on the way
    /// `StatuslineSegments` has. Side affinity can replace a buffer in the
    /// same window, so both identities are required to prevent one panel
    /// presentation from inheriting its predecessor's provider text.
    last_panel_statusline: Option<((WindowId, BufferId), StatuslineWindowSegments)>,
}

/// The presentation identity a shipped [`PanelFrame`] carries.
///
/// `window_id` moves when a new side window is created and `buffer_id`
/// when the panel's buffer is replaced; either one changing allocates a
/// new `panel_epoch`, and that is what stops a stale `PanelPointer` from
/// addressing a reopened panel as if it were the old one (Q#BP16).
/// `WindowId` deliberately stays off the wire — the epoch is the opaque
/// stand-in for it.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct PanelPresentation {
    window_id: WindowId,
    buffer_id: BufferId,
    panel_epoch: u64,
}

/// One [`SemanticRenderState::diag_line_cache`] entry: the line-start
/// offsets and source length of a buffer at `revision`.
struct DiagLineCache {
    revision: u64,
    line_starts: Vec<u64>,
    source_len: u64,
}

/// One [`SemanticRenderState::last_summary`] entry: the inputs the
/// summary was computed against and the computed per-line payload.
struct SummaryCache {
    /// `(crdt_generation, diag_epoch, syntax_epoch, face_epoch)`.
    key: (u64, u64, u64, u64),
    /// The computed summary — compared before emitting (Q#TH6
    /// payload-equality suppression).
    lines: Vec<Style>,
}

/// The stage-1 UI face inventory (themes arc Q#TH3): the names the
/// `ThemeFacts` producer resolves through [`crate::highlight::Theme::face`]
/// and ships. Resolution is daemon-side — frontends do exact-name
/// lookup on the shipped table, no walk (Q#TH7). Kept sorted; the
/// wire table's deterministic ordering rides on it.
const UI_FACES: &[&str] = &[
    "ui.diag.error",
    "ui.diag.hint",
    "ui.diag.info",
    "ui.diag.warning",
    "ui.gutter",
    "ui.minibuffer",
    "ui.minibuffer.candidate",
    "ui.modeline",
    "ui.search.match",
    "ui.search.match.active",
    "ui.selection",
    "ui.statusline",
];

/// Read both theme mutation counters under one lock (Q#TH6).
fn theme_epochs(state: &EditorState) -> (u64, u64) {
    let theme = state.syntax_registry.theme();
    let th = theme.lock().expect("theme mutex poisoned");
    (th.syntax_epoch, th.face_epoch)
}

/// Recompute gate for [`scoped_style_spans`] on a grammar-backed
/// buffer. Holds the current parse bundle `Arc` so its address stays
/// stable while cached — comparing by `Arc::ptr_eq` then can't be
/// fooled by a freed bundle's address being reused (ABA). Equal gates
/// ⇒ identical spans ⇒ the tree-sitter query can be skipped.
/// `generation` is included so a CRDT edit still forces the M11.7
/// full-resync even when the parse bundle hasn't re-landed yet.
#[derive(Clone)]
struct StyleGate {
    /// Current parse bundle, or `None` when none has landed yet.
    bundle: Option<std::sync::Arc<crate::syntax::ParseTreeBundle>>,
    /// CRDT generation of the buffer.
    generation: u64,
    /// Declared viewport.
    visible: ByteRange,
    /// The theme's syntax mutation counter (Q#TH6): spans are a pure
    /// function of the theme too, and before this half the gate a
    /// mid-session `pmacs.theme.set` shipped nothing until the next
    /// buffer edit — the GPU kept stale colors.
    syntax_epoch: u64,
}

impl StyleGate {
    /// True when both gates would produce identical style spans.
    fn matches(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.visible == other.visible
            && self.syntax_epoch == other.syntax_epoch
            && match (&self.bundle, &other.bundle) {
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

impl SemanticRenderState {
    /// Fresh session state for a peer that negotiated
    /// `negotiated_protocol_version` — the real daemon construction
    /// path (PR #120 round 1 finding 3): a `< 16` peer gets no
    /// `ThemeFacts` produced at all and, crucially, no face-derived
    /// colors folded into its `FileStyleSummary` marks.
    #[must_use]
    pub fn for_peer(frontend_id: FrontendId, negotiated_protocol_version: u32) -> Self {
        let mut s = Self::new(frontend_id);
        s.peer_knows_theme_facts = negotiated_protocol_version >= 16;
        s.peer_knows_font_facts = negotiated_protocol_version >= 17;
        s.peer_knows_line_wrap = negotiated_protocol_version >= 22;
        s.peer_knows_statusline_segments = negotiated_protocol_version >= 18;
        s.peer_knows_terminal_frames = negotiated_protocol_version >= 19;
        s.peer_knows_panel_frames = negotiated_protocol_version >= PANEL_MIN_VERSION;
        s
    }

    /// Fresh session state for frontend `frontend_id`: no viewport
    /// declared, nothing sent. Assumes a current-build peer (>= 18);
    /// daemon sessions with a real negotiated version use
    /// [`Self::for_peer`].
    #[must_use]
    pub fn new(frontend_id: FrontendId) -> Self {
        Self {
            frontend_id,
            viewport: None,
            last_sent: HashMap::new(),
            last_decorations: HashMap::new(),
            last_adornments: HashMap::new(),
            last_folds: HashMap::new(),
            last_search_prompt: HashMap::new(),
            last_menu_prompt: HashMap::new(),
            last_minibuffer: None,
            last_completion_popup: HashMap::new(),
            last_summary: HashMap::new(),
            last_status: HashMap::new(),
            // Seed to the frontend's default (gutter off): a plain default
            // window never emits `LineNumbers`, so the common case adds no
            // traffic and the first frame is unchanged. Only an actual
            // toggle-on (or later toggle-off) ships a message.
            last_line_numbers: Some(crate::window::LineNumberMode::Off),
            last_style_gate: HashMap::new(),
            // Q#TH7: both seeded None — the first frame after viewport
            // declaration always ships an authoritative face table
            // (empty included), and the epoch gate cannot short-circuit
            // an epoch-0 daemon before that send.
            last_face_epoch: None,
            last_statusline_face_set_epoch: None,
            last_theme_faces: None,
            peer_knows_theme_facts: true,
            // Q#F5: both seeded None — the first frame after viewport
            // declaration always ships an authoritative FontFacts
            // (the all-default preference included), and the epoch
            // gate cannot short-circuit an epoch-0 daemon before it.
            last_font_epoch: None,
            last_font_facts: None,
            peer_knows_font_facts: true,
            peer_knows_line_wrap: true,
            last_line_wrap: None,
            peer_knows_statusline_segments: true,
            last_statusline: HashMap::new(),
            diag_line_cache: HashMap::new(),
            peer_knows_terminal_frames: true,
            terminal_viewport: None,
            last_terminal_frame: None,
            terminal_error_latched: false,
            terminal_active: false,
            peer_knows_panel_frames: true,
            // Q#BP15: a fresh session has no band, and that is a fact the
            // peer already holds. Seeding the baseline keeps the first
            // frame from shipping a redundant authoritative `Absent`.
            last_panel_payload: Some(PanelFramePayload::Absent),
            panel_epoch_used: 0,
            panel_presentation: None,
            panel_error_latched: false,
            last_panel_statusline: None,
        }
    }

    /// The `Present` panel declaration this session last shipped.
    ///
    /// The daemon reads it to run steps 3 and 4 of Q#BP16's validation
    /// ladder: an inbound `PanelPointer` or `PanelResizeRows` must name
    /// the geometry and presentation epochs of the frame the frontend was
    /// actually looking at. `None` after an `Absent` — which is precisely
    /// how `Absent` "clears input authority".
    #[must_use]
    pub fn panel_declaration(&self) -> Option<&PanelFrame> {
        match self.last_panel_payload.as_ref()? {
            PanelFramePayload::Present(frame) => Some(frame),
            PanelFramePayload::Absent => None,
        }
    }

    /// Whether the last shipped declaration is a `Present` whose epochs
    /// both match an inbound panel event **and** which still describes
    /// the side window that is live now (Q#BP16 steps 2–4).
    ///
    /// Rolled into one predicate so no caller can check the geometry
    /// epoch and forget the presentation epoch: they close different
    /// holes and neither subsumes the other.
    ///
    /// `live_presentation` is the frontend's **current** side window and
    /// its buffer. Comparing it is what closes review round 1's R1-2:
    /// closing and reopening the same *persistent* buffer inside one
    /// dispatcher burst leaves the shipped declaration intact while the
    /// window it describes is already dead, and same-buffer/same-size
    /// makes the successor indistinguishable by every other field. A
    /// presentation epoch only identifies a presentation if something
    /// checks that the presentation it names is still the one on screen.
    ///
    /// `None` means "no side window right now", which never matches — an
    /// event cannot address a panel that does not exist.
    #[must_use]
    pub fn panel_declaration_matches(
        &self,
        geometry_epoch: u64,
        panel_epoch: u64,
        live_presentation: Option<(WindowId, BufferId)>,
    ) -> bool {
        let Some((window_id, buffer_id)) = live_presentation else {
            return false;
        };
        self.panel_presentation.is_some_and(|presentation| {
            presentation.window_id == window_id && presentation.buffer_id == buffer_id
        }) && self.panel_declaration().is_some_and(|frame| {
            frame.geometry_epoch == geometry_epoch && frame.panel_epoch == panel_epoch
        })
    }

    /// Record the frontend's declared on-screen byte range. Called by
    /// the dispatcher when it receives
    /// [`crate::protocol::FrontendEvent::Viewport`]. Replaces any
    /// prior declaration wholesale — the latest viewport wins.
    pub fn set_viewport(&mut self, buffer_id: BufferId, visible: ByteRange, generation: u64) {
        self.viewport = Some(DeclaredViewport {
            buffer_id,
            visible,
            frontend_generation: generation,
        });
    }

    /// Record the terminal-cell geometry this frontend declared
    /// (`FrontendEvent::TerminalResize`, accepted by the daemon only
    /// when it names the authenticated source's active terminal).
    ///
    /// Replacing the declaration for a DIFFERENT buffer drops the frame
    /// baseline: the next frame describes another session entirely, and
    /// comparing it against the old one could suppress it.
    pub fn set_terminal_viewport(&mut self, buffer_id: BufferId, size: CellSize) {
        if self
            .terminal_viewport
            .is_some_and(|(previous, _)| previous != buffer_id)
        {
            self.clear_terminal_baseline();
        }
        self.terminal_viewport = Some((buffer_id, size));
    }

    /// The terminal geometry this frontend last declared.
    ///
    /// The daemon reads this to apply the semantic layout sync beside
    /// the landed grid sync, before the next child-output drain.
    #[must_use]
    pub fn terminal_viewport(&self) -> Option<(BufferId, CellSize)> {
        self.terminal_viewport
    }

    /// Whether the last render pass projected a terminal.
    ///
    /// The daemon consults this to suppress the document `CursorByte`
    /// and the presence sweep: a terminal identity buffer is empty, so
    /// both would describe a cursor at byte 0 of a buffer with no text.
    /// It tracks the PASS, not the baseline — a first frame rejected by
    /// validation still means this frontend is displaying a terminal,
    /// and falling back to the document path there would paint an empty
    /// buffer over a live session.
    #[must_use]
    pub fn in_terminal_mode(&self) -> bool {
        self.terminal_active
    }

    /// Forget the terminal frame baseline so the next valid frame is
    /// authoritative, and re-arm the invalid-frame log.
    fn clear_terminal_baseline(&mut self) {
        self.last_terminal_frame = None;
        self.terminal_error_latched = false;
    }

    /// Drop terminal projection state on detach or context replacement.
    pub fn on_terminal_context_released(&mut self) {
        self.terminal_viewport = None;
        self.clear_terminal_baseline();
    }

    /// Snapshot/baseline reset contract (PR #120 round 2 finding 1).
    ///
    /// A `BufferSnapshot` resets the receiving frontend's
    /// buffer-scoped render state wholesale — spans, decorations,
    /// adornments, minimap summary, completion popup, status facts, and
    /// statusline segments (see the GPU's `BufferSnapshot` arm) — so
    /// every buffer-scoped emission baseline this producer holds for
    /// that buffer must die with the send.
    /// Otherwise an unchanged-key revisit (the A → B → A round
    /// trip at one CRDT generation) suppresses every re-send and the
    /// frontend never regains the state until an edit, diagnostic
    /// republish, or theme mutation happens to move the key.
    ///
    /// Called by the daemon wherever it writes a `BufferSnapshot` to
    /// this session's stream. Resetting when the write later fails is
    /// harmless — the failure mode is one redundant re-send, never
    /// staleness.
    ///
    /// Deliberately NOT reset: `last_face_epoch`,
    /// `last_statusline_face_set_epoch`, and `last_theme_faces`
    /// (`ThemeFacts` is bufferless — the frontend keeps its face
    /// table across snapshots), `last_minibuffer` (one global core
    /// instance, not buffer-scoped), `last_line_numbers`
    /// (per-frontend gutter mode, kept by the frontend across the
    /// switch), and `diag_line_cache` (a revision-keyed compute
    /// cache, not a peer-state baseline). The diagnostic-count
    /// freeze survives by construction (rounds 3–4): it is sourced
    /// from the diag store's retained vector, never from session
    /// state, so this reset cannot zero mid-edit counts. Baselines
    /// for OTHER buffers also survive — the snapshot names one
    /// buffer, and any buffer the frontend navigates to receives its
    /// own snapshot first.
    pub fn on_buffer_snapshot_sent(&mut self, buffer_id: BufferId) {
        // Vterm Stage 3: a snapshot takes the GPU out of terminal mode
        // unconditionally — it clears the prior frame and every
        // terminal-only cache before painting. Both sides must forget
        // together, and the frontend re-declares its geometry
        // immediately after applying the snapshot, so dropping the
        // declaration here costs one message, not a stuck terminal.
        self.terminal_viewport = None;
        self.clear_terminal_baseline();
        self.last_sent.remove(&buffer_id);
        self.last_style_gate.remove(&buffer_id);
        self.last_decorations.remove(&buffer_id);
        self.last_adornments.remove(&buffer_id);
        self.last_folds.remove(&buffer_id);
        self.last_summary.remove(&buffer_id);
        self.last_status.remove(&buffer_id);
        self.last_search_prompt.remove(&buffer_id);
        self.last_menu_prompt.remove(&buffer_id);
        self.last_completion_popup.remove(&buffer_id);
        self.last_statusline.remove(&buffer_id);
    }

    /// Project one frame.
    ///
    /// Returns up to three messages — [`InstanceMessage::StyleSpans`]
    /// (T M11.2), [`InstanceMessage::Decorations`] (T M11.3), and
    /// [`InstanceMessage::InlineAdornments`] (Step 3, from the LSP
    /// inlay-hint store) — each scoped to the declared viewport and
    /// each suppressed independently when byte-identical to its last
    /// send. Returns an empty vec before the frontend declares a
    /// viewport.
    ///
    /// `BlockAdornments` is still deliberately *not* produced: pmacs has
    /// no instance-side blame / lens / diff source yet. (`FoldState` IS
    /// produced now — Arc 6 — authoritative-empty via `fold_state_msg`.)
    /// Its wire variant exists (T M11.1); its producer wires in when that
    /// feature lands — the same "declared, not yet wired" discipline.
    /// Emitting an empty message
    /// every frame would be waste, not honesty, so `InlineAdornments` is
    /// suppressed both when unchanged and when there is simply nothing
    /// to say (no hints, no prior non-empty send).
    #[allow(clippy::too_many_lines)]
    pub fn render_frame(&mut self, state: &EditorState) -> Vec<InstanceMessage> {
        // Vterm Stage 3: a terminal window suppresses the whole document
        // projection. It is checked FIRST because the terminal identity
        // buffer is a valid (empty) document — running the document path
        // over it would ship an authoritative empty styling/summary
        // resync on top of the live cell grid.
        if let Some(messages) = self.terminal_frame_pass(state) {
            return messages;
        }
        let Some(vp) = self.viewport.clone() else {
            // Emit nothing document-scoped before the frontend declares a
            // viewport — but the band is a SEPARATE surface (Q#BP15a),
            // and gating it on the document declaration would make the
            // first panel unpaintable on a frontend that has not declared
            // one yet. Its statusline is `None` here for the same reason:
            // the semantic fan-out is keyed on the declared buffer.
            let mut out = Vec::new();
            self.emit_panel_frame(state, None, &mut out);
            return out;
        };

        // Evaluate callbacks before any long-lived core borrow and before
        // ThemeFacts is computed. A callback may change the registry; the
        // post-evaluation face inventory must then precede the authoritative
        // segment replacement in this same frame. Unsupported peers skip the
        // evaluator entirely and therefore pay no Lua callback/dynamic-face cost.
        // Bottom-panel A2A-2, round 3: the document identity used to
        // FILTER the results must be the PRE-CALLBACK one. Both outcome
        // arms carry phase-1 contexts, and a provider that closes the
        // primary document split changes `primary_document_window`
        // mid-evaluation — reading it after the fact would compare
        // phase-1 contexts against a replacement identity, match
        // nothing, and silently suppress the authoritative clear.
        let statusline_document_window = self
            .peer_knows_statusline_segments
            .then(|| {
                state
                    .core
                    .borrow()
                    .primary_document_window(self.frontend_id)
            })
            .flatten();
        let statusline_evaluation = self.peer_knows_statusline_segments.then(|| {
            evaluate_statusline(
                state.lua_host.lua(),
                &state.core,
                &state.statusline_registry,
                StatuslineEvaluationTarget::Semantic {
                    frontend_id: self.frontend_id,
                    declared_buffer: vp.buffer_id,
                },
            )
        });

        let generation = buffer_generation(state, vp.buffer_id);
        let mut out = Vec::new();

        // --- StyleSpans (T M11.2 producer, T M11.4 diff) ---
        // Perf gate: `scoped_style_spans` runs the tree-sitter query
        // over the whole viewport + clones the theme. For a grammar-
        // backed buffer it's a pure function of (bundle revision,
        // generation, viewport), so a cursor-only tick — same key,
        // already-sent baseline — can skip the whole block. The LSP-
        // token path returns `None` (no cheap revision) and recomputes
        // every tick as before.
        let style_parse_not_ready = grammar_style_parse_not_ready(state, vp.buffer_id);
        // The LSP-token styling authority (grammar-less buffers, e.g.
        // C++) gets the same hold: while the semantic-token store is
        // stale (document edited since the last token response),
        // `lsp_scoped_style_spans` would compute an empty set, and
        // shipping that clears the frontend's colors for the whole
        // stale window — the styling twin of the diagnostics blink.
        let style_tokens_stale = lsp_style_tokens_stale(state, vp.buffer_id);
        let style_hold = style_parse_not_ready || style_tokens_stale;
        let style_gate = (!style_hold).then(|| grammar_style_key(state, &vp, generation));
        let style_gate = style_gate.flatten();
        let style_unchanged = match (&style_gate, self.last_style_gate.get(&vp.buffer_id)) {
            (Some(g), Some(prev)) => g.matches(prev) && self.last_sent.contains_key(&vp.buffer_id),
            _ => false,
        };
        if style_hold || style_unchanged {
            // If the style key is unchanged, styling cannot have
            // changed since the last computation. If a grammar parse is
            // still pending (or the LSP token store is stale), keep the
            // previous spans briefly rather than querying and reshaping
            // stale syntax on every typed byte; the parse-bundle
            // revision (or the next token response) will force a fresh
            // frame as soon as it settles.
        } else {
            match style_gate {
                Some(g) => {
                    self.last_style_gate.insert(vp.buffer_id, g);
                }
                None => {
                    self.last_style_gate.remove(&vp.buffer_id);
                }
            }
            self.emit_style_spans(state, &vp, generation, &mut out);
        }

        // --- Decorations (T M11.3 producer, T M11.4 diff) ---
        let mut decorations = self.scoped_decorations(state, &vp);
        let prev = self.last_decorations.get(&vp.buffer_id);
        // Hold-while-stale, part 2 (selection navigation): while the
        // diag store is stale, CARRY the previously shipped diagnostic
        // items through this frame's set instead of dropping them.
        // A shift+arrow during the post-burst stale window then diffs
        // as a tiny selection-only segment (the carried diag ranges
        // are unchanged, so they fall outside the changed intervals
        // and are never re-shipped at stale positions) — instead of
        // a full frame per keypress that also blinked the frontend's
        // held diagnostics out.
        let diag_hold = diagnostics_store_stale(state, vp.buffer_id);
        if diag_hold && let Some(p) = prev {
            decorations.extend(
                p.items
                    .iter()
                    .filter(|d| is_diagnostic_kind(d.kind))
                    .cloned(),
            );
            decorations.sort_by_key(|d| d.range.start);
        }
        // Hold-while-stale: while the diag store is stale (document
        // edited since the last `publishDiagnostics`), this frame has
        // no authoritative diagnostic positions. The frontend's
        // last-received set — which it translates through its own
        // local edits — is strictly better than anything we can ship:
        // an empty frame wipes it (diagnostics blink out on the first
        // keystroke of every burst and back in after the next publish,
        // one full frontend reshape each way), and re-shipping the
        // store's items would anchor pre-edit positions over post-edit
        // text (the M11.8 artifact). So as long as the
        // *non-diagnostic* part is unchanged, say nothing and leave
        // the baseline untouched — staleness clears on the next
        // publishDiagnostics absorption, and the generation transition
        // since the held baseline forces that frame full.
        let held = diag_hold
            && prev
                .is_some_and(|p| p.visible == vp.visible && decorations.iter().eq(p.items.iter()));
        // During a hold, only the GENERATION trigger for a full frame
        // is suppressed (edits bump it every keystroke; the carried
        // set diffs instead). First-ever frames and viewport changes
        // still resync in full.
        let full = prev
            .is_none_or(|p| p.visible != vp.visible || (!diag_hold && p.generation != generation));
        if held {
            // No new information for the frontend this frame. Keep
            // the baseline's generation current so the eventual
            // unstale frame diffs instead of full-resyncing (the diff
            // covers the diagnostics' post-publish positions).
            if let Some(p) = self.last_decorations.get_mut(&vp.buffer_id) {
                p.generation = generation;
            }
        } else if full {
            let suppress_empty_generation_bump = prev.is_some_and(|p| {
                p.visible == vp.visible && p.items.is_empty() && decorations.is_empty()
            });
            self.last_decorations.insert(
                vp.buffer_id,
                LastFrame {
                    visible: vp.visible,
                    items: decorations.clone(),
                    generation,
                },
            );
            if !suppress_empty_generation_bump {
                out.push(InstanceMessage::Decorations {
                    buffer_id: vp.buffer_id,
                    generation,
                    full: true,
                    segments: vec![DecorationSegment {
                        range: vp.visible,
                        decorations,
                    }],
                });
            }
        } else {
            let prev = prev.expect("checked is_none_or above");
            let intervals = changed_intervals(&prev.items, &decorations, |d| d.range);
            if !intervals.is_empty() {
                let segments = intervals
                    .into_iter()
                    .map(|range| DecorationSegment {
                        range,
                        decorations: clip_decorations(range, &decorations),
                    })
                    .collect();
                self.last_decorations.insert(
                    vp.buffer_id,
                    LastFrame {
                        visible: vp.visible,
                        items: decorations,
                        generation,
                    },
                );
                out.push(InstanceMessage::Decorations {
                    buffer_id: vp.buffer_id,
                    generation,
                    full: false,
                    segments,
                });
            }
        }

        // --- InlineAdornments (Step 3 producer) ---
        out.extend(self.inline_adornments_msg(state, &vp));
        // --- FoldState (Arc 6 producer; authoritative-empty) ---
        out.extend(self.fold_state_msg(state, vp.buffer_id));
        // --- FileStyleSummary (minimap producer; Open Q#2) ---
        out.extend(self.file_style_summary_msg(state, vp.buffer_id, generation));
        // --- StatusFacts (status band; Q#S1, protocol v8) ---
        out.extend(self.status_facts_msg(state, vp.buffer_id));
        // --- LineNumbers (gutter toggle; UX gutter arc, protocol v13) ---
        out.extend(self.line_numbers_msg(state, vp.buffer_id));
        // --- SearchPrompt (isearch band; Q#SR5, protocol v9) ---
        out.extend(self.search_prompt_msg(state, vp.buffer_id));
        // --- MenuPrompt (context menu; Q#CM1, protocol v11) ---
        out.extend(self.menu_prompt_msg(state, vp.buffer_id));
        // --- MinibufferPrompt (Q#MB1, protocol v12) ---
        out.extend(self.minibuffer_prompt_msg(state, vp.buffer_id));
        // --- CompletionPopup (Arc 1a Q#C5, protocol v15) ---
        out.extend(self.completion_popup_msg(state, vp.buffer_id));
        // --- ThemeFacts (UI faces; themes arc Q#TH7, protocol v16) ---
        out.extend(self.theme_facts_msg(state));
        out.extend(self.font_facts_msg(state));
        out.extend(self.line_wrap_msg(state, vp.buffer_id));
        // Q#SL6/Q#SL8: face inventory must precede segment text.
        // Parent acceptance 45: ONE provider invocation supplies both the
        // primary-document wire segments and the panel mode line, so the
        // side half is taken from this same evaluation before it is
        // consumed.
        let panel_presentation = self.panel_statusline_presentation(state);
        let panel_statusline =
            self.panel_statusline(statusline_evaluation.as_ref(), panel_presentation);
        if let Some(evaluation) = statusline_evaluation {
            self.emit_statusline_segments(evaluation, statusline_document_window, &mut out);
        }
        self.emit_panel_frame(state, panel_statusline.as_ref(), &mut out);
        out
    }

    /// Project this frontend's active terminal, or `None` when it is
    /// not displaying one (so the document path runs instead).
    ///
    /// Returning `Some` means terminal mode: the caller emits exactly
    /// these messages and no document family at all. What survives is
    /// the buffer-independent chrome the native frontend still needs —
    /// status band, theme, font, statusline, menu, and minibuffer — plus
    /// the frame itself.
    fn terminal_frame_pass(&mut self, state: &EditorState) -> Option<Vec<InstanceMessage>> {
        // A v18 peer has no terminal surface at all: it keeps the
        // document path over the empty identity buffer, exactly as it
        // did before this protocol version existed.
        //
        // Every path out of terminal mode clears `terminal_active`
        // explicitly. An early `?` that left it set would keep the
        // daemon suppressing this frontend's `CursorByte` and presence
        // long after it went back to editing a document.
        let declaration = self
            .peer_knows_terminal_frames
            .then_some(self.terminal_viewport)
            .flatten();
        let Some((buffer_id, size)) = declaration else {
            self.terminal_active = false;
            return None;
        };
        let Some(snapshot) =
            state.prepare_semantic_terminal_view(self.frontend_id, buffer_id, size)
        else {
            // The window switched away, the session died, or the
            // declared size went out of range. Leave terminal mode and
            // let the document path resume.
            self.terminal_active = false;
            self.clear_terminal_baseline();
            return None;
        };
        self.terminal_active = true;

        // Evaluate callbacks before `ThemeFacts` for the same reason the
        // document path does: a callback may register a face, and the
        // face inventory must precede the segment text that names it.
        // Same pre-callback capture as the document path (round 3).
        let statusline_document_window = self
            .peer_knows_statusline_segments
            .then(|| {
                state
                    .core
                    .borrow()
                    .primary_document_window(self.frontend_id)
            })
            .flatten();
        let statusline_evaluation = self.peer_knows_statusline_segments.then(|| {
            evaluate_statusline(
                state.lua_host.lua(),
                &state.core,
                &state.statusline_registry,
                StatuslineEvaluationTarget::Semantic {
                    frontend_id: self.frontend_id,
                    declared_buffer: buffer_id,
                },
            )
        });

        let mut out = Vec::new();
        let frame = snapshot.into_terminal_frame();
        // Complete-payload comparison FIRST, and not on
        // `screen_generation`: a scroll, a selection change, or a
        // process exit must reach the frontend even though the screen
        // itself is byte-identical.
        //
        // Comparing before validating is also what keeps the steady
        // state cheap. Only validated frames are ever stored, so a frame
        // equal to the baseline has already passed — re-running the
        // per-cell width and topology checks every tick would recompute
        // a verdict we hold.
        if self.last_terminal_frame.as_ref() == Some(&frame) {
            self.terminal_error_latched = false;
            out.extend(self.terminal_chrome(
                state,
                buffer_id,
                statusline_evaluation,
                statusline_document_window,
            ));
            return Some(out);
        }
        match frame.validate() {
            Ok(()) => {
                self.terminal_error_latched = false;
                self.last_terminal_frame = Some(frame.clone());
                out.push(InstanceMessage::TerminalFrame(frame));
            }
            Err(error) => {
                // Never emit a malformed or truncated frame. The peer
                // keeps the last valid one; one bounded log line marks
                // the condition until a valid frame clears the latch.
                if !self.terminal_error_latched {
                    self.terminal_error_latched = true;
                    eprintln!(
                        "pmacs: terminal frame for {:?} on {:?} failed validation, \
                         retaining the last valid frame: {error}",
                        buffer_id, self.frontend_id
                    );
                }
            }
        }

        out.extend(self.terminal_chrome(
            state,
            buffer_id,
            statusline_evaluation,
            statusline_document_window,
        ));
        Some(out)
    }

    /// The buffer-independent chrome a terminal-mode frontend still
    /// needs, in the order the document path emits it.
    ///
    /// Shared by both terminal-pass exits so an unchanged frame and a
    /// changed one ship exactly the same chrome — the suppression is
    /// about the FRAME, never about the status band going quiet.
    fn terminal_chrome(
        &mut self,
        state: &EditorState,
        buffer_id: BufferId,
        statusline_evaluation: Option<StatuslineEvaluation>,
        statusline_document_window: Option<crate::window::WindowId>,
    ) -> Vec<InstanceMessage> {
        let mut out = Vec::new();
        out.extend(self.status_facts_msg(state, buffer_id));
        out.extend(self.menu_prompt_msg(state, buffer_id));
        out.extend(self.minibuffer_prompt_msg(state, buffer_id));
        out.extend(self.theme_facts_msg(state));
        out.extend(self.font_facts_msg(state));
        out.extend(self.line_wrap_msg(state, buffer_id));
        // Q#SL6/Q#SL8: face inventory must precede segment text.
        // The band rides the terminal path too: a frontend whose DOCUMENT
        // surface is a full-window terminal can still hold a side window,
        // and suppressing the panel here would leave the peer's retained
        // band on screen with no way to clear it.
        let panel_presentation = self.panel_statusline_presentation(state);
        let panel_statusline =
            self.panel_statusline(statusline_evaluation.as_ref(), panel_presentation);
        if let Some(evaluation) = statusline_evaluation {
            self.emit_statusline_segments(evaluation, statusline_document_window, &mut out);
        }
        self.emit_panel_frame(state, panel_statusline.as_ref(), &mut out);
        out
    }

    /// Apply the lead evaluator's publication outcome to the v18 wire
    /// baseline. Invalidated evaluations discard all callback text and
    /// publish authoritative empty replacements for the captured old
    /// contexts; phase-1 stale outcomes publish nothing.
    fn emit_statusline_segments(
        &mut self,
        evaluation: StatuslineEvaluation,
        document_window: Option<crate::window::WindowId>,
        out: &mut Vec<InstanceMessage>,
    ) {
        let to_wire = |segments: Vec<crate::statusline::EvaluatedStatuslineSegment>| {
            segments
                .into_iter()
                .map(|segment| StatuslineSegment {
                    text: segment.text,
                    face: segment.face,
                })
                .collect()
        };
        let frontend_id = self.frontend_id;
        match evaluation.outcome {
            StatuslineEvaluationOutcome::Ready(windows) => {
                // Bottom-panel A2A-2: the fan-out now yields the primary
                // document AND the visible side window, so the wire
                // segments must be selected by WINDOW IDENTITY. Taking
                // "the first context for my frontend" would silently
                // depend on capture order and could ship the panel's
                // mode-line text as the document status band.
                if let Some(window) = windows.into_iter().find(|window| {
                    window.context.frontend_id == frontend_id
                        && Some(window.context.window_id) == document_window
                }) {
                    self.emit_statusline_payload(
                        window.context.buffer_id,
                        to_wire(window.left),
                        to_wire(window.right),
                        out,
                    );
                }
            }
            StatuslineEvaluationOutcome::Invalidated {
                authoritative_empty,
            } => {
                // Bottom-panel A2A-2: the clear must be filtered by
                // DOCUMENT WINDOW exactly like the Ready arm. The
                // semantic peer has ONE statusline slot, so publishing
                // the panel context's clear here replaces the document's
                // payload with the panel's — the same misrouting the
                // Ready arm was fixed for, on the clear path.
                //
                // A panel's own clear belongs to the future panel
                // painter (`PanelFrame`, Stage 2B), not to this wire.
                for context in authoritative_empty.into_iter().filter(|context| {
                    context.frontend_id == frontend_id && Some(context.window_id) == document_window
                }) {
                    self.emit_statusline_payload(context.buffer_id, Vec::new(), Vec::new(), out);
                }
            }
            StatuslineEvaluationOutcome::NoMessage(_) => {}
        }
    }

    /// The side window's evaluated segments from **this frame's** single
    /// provider invocation (parent acceptance 45).
    ///
    /// Selected by window identity, exactly like the document half: the
    /// fan-out yields the primary document *and* the visible side window,
    /// and taking "some context for my frontend" would depend on capture
    /// order and could paint the document's status text into the panel's
    /// mode line.
    ///
    /// The three outcomes are **not** interchangeable, and review round 1
    /// (R2-4) found two of them collapsed:
    ///
    /// * `Ready` is authoritative — including an empty result. It
    ///   replaces the retained baseline.
    /// * `Invalidated` discards all evaluated text: a callback mutated
    ///   registry, layout, or focus mid-evaluation, so the band clears to
    ///   its plain mode line and the baseline dies with it.
    /// * `NoMessage` means **publish nothing**. Phase 1 was already stale
    ///   — most reachably a buffer-follow mismatch, where the primary
    ///   document window has moved off the buffer the frontend declared.
    ///   The band therefore keeps what it last published. Treating this
    ///   like `Invalidated` *removes* provider text on a transient
    ///   condition that said nothing about it.
    ///
    /// The baseline is keyed by both window and buffer identity. Side
    /// affinity deliberately replaces the buffer in an existing side
    /// window, and that replacement is a new presentation even though the
    /// `WindowId` is stable.
    fn panel_statusline_presentation(&self, state: &EditorState) -> Option<(WindowId, BufferId)> {
        let core = state.core.borrow();
        let window_id = core.side_window_for(self.frontend_id)?;
        let buffer_id = core.windows.get(&window_id)?.buffer_id;
        Some((window_id, buffer_id))
    }

    fn panel_statusline(
        &mut self,
        evaluation: Option<&StatuslineEvaluation>,
        panel_presentation: Option<(WindowId, BufferId)>,
    ) -> Option<StatuslineWindowSegments> {
        let Some((side_window, side_buffer)) = panel_presentation else {
            // No band to publish for; drop any baseline so a later panel
            // cannot inherit a dead window's text.
            self.last_panel_statusline = None;
            return None;
        };
        let retained = |state: &Self| {
            state
                .last_panel_statusline
                .as_ref()
                .filter(|(presentation, _)| *presentation == (side_window, side_buffer))
                .map(|(_, segments)| segments.clone())
        };
        let Some(evaluation) = evaluation else {
            // No evaluation ran at all (an unsupported peer, or a frame
            // before the document viewport exists). Nothing was
            // published, so nothing is retracted.
            return retained(self);
        };
        match &evaluation.outcome {
            StatuslineEvaluationOutcome::Ready(windows) => {
                let found = windows
                    .iter()
                    .find(|window| {
                        window.context.frontend_id == self.frontend_id
                            && window.context.window_id == side_window
                            && window.context.buffer_id == side_buffer
                    })
                    .cloned();
                self.last_panel_statusline = found
                    .clone()
                    .map(|segments| ((side_window, side_buffer), segments));
                found
            }
            StatuslineEvaluationOutcome::Invalidated { .. } => {
                self.last_panel_statusline = None;
                None
            }
            StatuslineEvaluationOutcome::NoMessage(_) => retained(self),
        }
    }

    /// Project this frontend's side window as an
    /// [`InstanceMessage::PanelFrame`] (Q#BP15).
    ///
    /// Runs on every frame of a panel-capable v21 semantic session,
    /// independently of the document byte viewport: the band is a
    /// separate surface, and gating it on a declared viewport would leave
    /// the first panel unpaintable on a frontend that has not yet
    /// declared one.
    ///
    /// Not reset by [`Self::on_buffer_snapshot_sent`]: a `BufferSnapshot`
    /// resets *document* mirror state, and the band is neither
    /// buffer-scoped to the document nor rebuilt from it.
    fn emit_panel_frame(
        &mut self,
        state: &EditorState,
        statusline: Option<&StatuslineWindowSegments>,
        out: &mut Vec<InstanceMessage>,
    ) {
        // Q#BP13: capability, not merely wire version. A session that
        // cannot render a band must not be shipped one — and, on the
        // production path, is never placed in a side window either.
        if !self.peer_knows_panel_frames || !state.core.borrow().panel_capable_for(self.frontend_id)
        {
            return;
        }
        // Q#BP15a: `Present` echoes the daemon's latest ACCEPTED geometry
        // declaration. Read before painting so the frame cannot answer a
        // declaration that arrived mid-projection.
        let geometry = state.core.borrow().frame_geometry_for(self.frontend_id);
        let projection =
            geometry.and_then(|_| state.prepare_panel_projection(self.frontend_id, statusline));
        let (Some(geometry), Some(projection)) = (geometry, projection) else {
            self.publish_absent_panel(out);
            return;
        };
        let identity = (projection.window_id, projection.buffer_id);
        let panel_epoch = match self.panel_presentation {
            Some(presentation) if (presentation.window_id, presentation.buffer_id) == identity => {
                Some(presentation.panel_epoch)
            }
            // A new side window, a replaced buffer, or any `Absent` →
            // `Present` transition (which cleared `panel_presentation`)
            // takes a fresh identity.
            _ => self.panel_epoch_used.checked_add(1),
        };
        let Some(panel_epoch) = panel_epoch else {
            // Q#BP15: allocation is checked and exhaustion fails closed
            // to `Absent`. Wrapping would let a new panel inherit a live
            // identity and accept gestures aimed at its predecessor.
            //
            // **The one knowingly per-frame `Absent` left in this file**,
            // and it is recorded rather than fixed. Review round 1's R1-1
            // established that a band cleared on the wire must also move
            // the durable `panel_hidden` state, or keys keep reaching an
            // invisible window; this arm cannot, because the producer
            // holds session state and `panel_hidden` is recomputed by
            // core reconciliation from geometry alone. Making it durable
            // needs a new "presentation permanently unavailable" reason
            // in `FrontendView`, which is machinery for a state that
            // takes 2^64 shipped presentation changes in ONE session to
            // reach — unlike the wire-area exhaustion in
            // `presentable_panel_grid`, which any frontend can trigger
            // with one declaration. If the epoch ever becomes
            // frontend-supplied, this stops being unreachable and needs
            // the durable arm.
            self.publish_absent_panel(out);
            return;
        };
        let payload = PanelFramePayload::Present(PanelFrame {
            buffer_id: projection.buffer_id,
            panel_epoch,
            geometry_epoch: geometry.geometry_epoch,
            size: projection.size,
            cells: projection.cells,
            cursor: projection.cursor,
            focused: projection.focused,
        });
        // Complete-payload comparison FIRST, like the terminal pass: only
        // validated payloads are ever stored, so a payload equal to the
        // baseline has already passed and re-running the per-cell width
        // and topology checks would recompute a verdict we hold.
        if self.last_panel_payload.as_ref() == Some(&payload) {
            self.panel_error_latched = false;
            return;
        }
        let PanelFramePayload::Present(frame) = &payload else {
            unreachable!("the Present payload was constructed immediately above");
        };
        match frame.validate() {
            Ok(()) => {
                self.panel_error_latched = false;
                self.panel_epoch_used = self.panel_epoch_used.max(panel_epoch);
                self.panel_presentation = Some(PanelPresentation {
                    window_id: projection.window_id,
                    buffer_id: projection.buffer_id,
                    panel_epoch,
                });
                self.last_panel_payload = Some(payload.clone());
                out.push(InstanceMessage::PanelFrame(payload));
            }
            Err(error) => {
                // Atomic rejection: the peer keeps its last valid frame,
                // this session keeps the presentation identity behind it,
                // and one bounded log line marks the condition. Advancing
                // `panel_epoch_used` here would burn an identity the peer
                // never saw.
                if !self.panel_error_latched {
                    self.panel_error_latched = true;
                    eprintln!(
                        "pmacs: panel frame for {:?} on {:?} failed validation, \
                         retaining the last valid frame: {error}",
                        projection.buffer_id, self.frontend_id
                    );
                }
            }
        }
    }

    /// Publish the authoritative `Absent` for every non-presentable state
    /// (Q#BP15, Q#BP2b).
    ///
    /// Clears the declared presentation on this side before any later
    /// event can validate against it — that is what "`Absent` clears
    /// input authority" means. The whole-frame geometry declaration
    /// deliberately survives: it is answered by the frontend, not by the
    /// panel's presence.
    fn publish_absent_panel(&mut self, out: &mut Vec<InstanceMessage>) {
        self.panel_presentation = None;
        // `Absent` also clears the peer's retained mode line. A later
        // `Present` under `NoMessage` therefore has nothing it can
        // legitimately retain, even if the same window and buffer reopen.
        self.last_panel_statusline = None;
        if self.last_panel_payload.as_ref() == Some(&PanelFramePayload::Absent) {
            // A duplicate `Absent` does no wire work — but both clears
            // above still run, so the state stays idempotent rather than
            // depending on which duplicate arrived first.
            return;
        }
        self.panel_error_latched = false;
        self.last_panel_payload = Some(PanelFramePayload::Absent);
        out.push(InstanceMessage::PanelFrame(PanelFramePayload::Absent));
    }

    fn emit_statusline_payload(
        &mut self,
        buffer_id: BufferId,
        left: Vec<StatuslineSegment>,
        right: Vec<StatuslineSegment>,
        out: &mut Vec<InstanceMessage>,
    ) {
        if self
            .last_statusline
            .get(&buffer_id)
            .is_some_and(|(old_left, old_right)| old_left == &left && old_right == &right)
        {
            return;
        }
        let baseline = (left.clone(), right.clone());
        out.push(InstanceMessage::StatuslineSegments {
            buffer_id,
            left,
            right,
        });
        // Advance only after the complete replacement has entered the
        // frame output, including the authoritative-empty invalidation path.
        self.last_statusline.insert(buffer_id, baseline);
    }

    /// The `CompletionPopup` message for this frame, or `None` when the
    /// popup state for `buffer_id` is unchanged (Arc 1a Q#C5). Only the
    /// active buffer carries a live popup, and — the multi-frontend
    /// rule — only the frontend whose *own window* owns the session
    /// sees it open: the session is window-stamped at open
    /// (`completion_popup_open`), and this producer state is
    /// per-frontend, so a popup opened by TUI typing never renders in
    /// an attached GPU and vice versa. Closed = `anchor: None`; first
    /// sight of a buffer with no popup stays silent (like
    /// `search_prompt_msg`). The daemon keeps the variant off wires
    /// negotiated `< 15`.
    fn completion_popup_msg(
        &mut self,
        state: &EditorState,
        buffer_id: BufferId,
    ) -> Option<InstanceMessage> {
        let facts: CompletionPopupFacts = {
            let core = state.core.borrow();
            if buffer_id != core.active_buffer_id() {
                return None;
            }
            let own_window = core.views.get(&self.frontend_id).map(|v| v.active);
            let guard = core
                .completion_popup
                .lock()
                .expect("completion popup poisoned");
            match guard.as_ref() {
                Some(p)
                    if p.buffer_id == buffer_id
                        && p.window_id.is_some()
                        && p.window_id == own_window =>
                {
                    let (start, len) = crate::completion::popup_window(
                        p.candidates.len(),
                        p.selected,
                        crate::completion::POPUP_MAX_ROWS as usize,
                    );
                    let rows: Vec<crate::protocol::CompletionPopupRow> = p.candidates
                        [start..start + len]
                        .iter()
                        .map(|c| crate::protocol::CompletionPopupRow {
                            label: c.label.clone(),
                            kind: c.kind as u8,
                            detail: c.detail.clone(),
                        })
                        .collect();
                    (
                        Some(p.anchor),
                        u32::try_from(p.prefix.len()).unwrap_or(u32::MAX),
                        rows,
                        u32::try_from(p.selected - start).ok(),
                        u32::try_from(p.total).unwrap_or(u32::MAX),
                    )
                }
                _ => (None, 0, Vec::new(), None, 0),
            }
        };
        let cached = self.last_completion_popup.get(&buffer_id);
        if cached == Some(&facts) {
            return None;
        }
        // First sight of this buffer with no popup: nothing to clear,
        // stay silent (the search-prompt rule).
        if cached.is_none() && facts.0.is_none() {
            self.last_completion_popup.insert(buffer_id, facts);
            return None;
        }
        let msg = InstanceMessage::CompletionPopup {
            buffer_id,
            anchor: facts.0,
            prefix_len: facts.1,
            rows: facts.2.clone(),
            selected: facts.3,
            total: facts.4,
        };
        self.last_completion_popup.insert(buffer_id, facts);
        Some(msg)
    }

    /// The `SearchPrompt` message for this frame, or `None` when the
    /// search state for `buffer_id` is unchanged. Only the active
    /// buffer carries a live prompt: a search shadows dispatch, so it
    /// always runs in the active buffer, and emitting for that buffer's
    /// viewport keeps the per-buffer cached-compare honest. When no
    /// search runs the active buffer emits `query: None` once (to clear
    /// the frontend's band), then stays silent. The daemon's write loop
    /// keeps the variant off wires negotiated `< 9`.
    fn search_prompt_msg(
        &mut self,
        state: &EditorState,
        buffer_id: BufferId,
    ) -> Option<InstanceMessage> {
        // Off-active-buffer viewports never touch the search band — the
        // active buffer owns it. (Without this, switching buffers mid-
        // session would let an inactive viewport clobber the cache.)
        let facts = {
            let core = state.core.borrow();
            if buffer_id != core.active_buffer_id() {
                return None;
            }
            if core.search_active() {
                let (active_idx, total) = core.search_match_summary();
                (
                    Some(core.search_query().to_owned()),
                    active_idx.and_then(|i| u32::try_from(i).ok()),
                    u32::try_from(total).unwrap_or(u32::MAX),
                    core.search_is_regex(),
                    core.search_is_invalid(),
                )
            } else {
                // No search → a cleared band. active/total/regex/invalid
                // are zeroed so the inactive state is one canonical tuple
                // (the GPU only reads them when `query` is `Some`). The
                // accepted matches keep highlighting via Decorations.
                (None, None, 0, false, false)
            }
        };
        let cached = self.last_search_prompt.get(&buffer_id);
        if cached == Some(&facts) {
            return None;
        }
        // First sight of this buffer with no active search: there is
        // nothing to clear, so stay silent rather than ship an empty
        // band on every fresh buffer. Record the baseline so a *later*
        // search→clear transition still diffs. (Mirrors the inline-
        // adornments "speak only if there's something to show" rule.)
        if cached.is_none() && facts.0.is_none() {
            self.last_search_prompt.insert(buffer_id, facts);
            return None;
        }
        let msg = InstanceMessage::SearchPrompt {
            buffer_id,
            query: facts.0.clone(),
            active: facts.1,
            total: facts.2,
            regex: facts.3,
            invalid: facts.4,
        };
        self.last_search_prompt.insert(buffer_id, facts);
        Some(msg)
    }

    /// The `MenuPrompt` message for this frame, or `None` when the menu
    /// state for `buffer_id` is unchanged (Q#CM1). Only the active
    /// buffer carries a live menu (it shadows dispatch). Closed = empty
    /// `rows`; first sight of a buffer with no menu stays silent (like
    /// `search_prompt_msg`). The daemon keeps the variant off wires
    /// negotiated `< 11`.
    fn menu_prompt_msg(
        &mut self,
        state: &EditorState,
        buffer_id: BufferId,
    ) -> Option<InstanceMessage> {
        let facts: MenuPromptFacts = {
            let core = state.core.borrow();
            if buffer_id != core.active_buffer_id() {
                return None;
            }
            let guard = core.menu.lock().expect("menu mutex poisoned");
            match guard.as_ref() {
                Some(m) => {
                    let rows = m
                        .rows
                        .iter()
                        .map(|r| match r {
                            crate::menu::MenuRow::Item { label, .. } => MenuPromptRow {
                                label: label.clone(),
                                separator: false,
                            },
                            crate::menu::MenuRow::Separator => MenuPromptRow {
                                label: String::new(),
                                separator: true,
                            },
                        })
                        .collect();
                    (rows, u32::try_from(m.active).ok())
                }
                None => (Vec::new(), None),
            }
        };
        let cached = self.last_menu_prompt.get(&buffer_id);
        if cached == Some(&facts) {
            return None;
        }
        // First sight, menu closed: nothing to clear, stay silent (record
        // the baseline so a later open→close still diffs).
        if cached.is_none() && facts.0.is_empty() {
            self.last_menu_prompt.insert(buffer_id, facts);
            return None;
        }
        let msg = InstanceMessage::MenuPrompt {
            buffer_id,
            rows: facts.0.clone(),
            active: facts.1,
        };
        self.last_menu_prompt.insert(buffer_id, facts);
        Some(msg)
    }

    /// The `MinibufferPrompt` message for this frame, or `None` when the
    /// (global) minibuffer state is unchanged (Q#MB1). Emitted only from
    /// the active buffer's viewport so the bufferless message ships once
    /// per frame. Closed = `prompt: None`; first sight while closed stays
    /// silent. The daemon keeps the variant off wires negotiated `< 12`.
    fn minibuffer_prompt_msg(
        &mut self,
        state: &EditorState,
        buffer_id: BufferId,
    ) -> Option<InstanceMessage> {
        let facts: MinibufferFacts = {
            let core = state.core.borrow();
            if buffer_id != core.active_buffer_id() {
                return None;
            }
            let mb = &core.minibuffer;
            match mb.session.as_ref() {
                Some(session) => {
                    let input = mb.contents();
                    let cursor_byte = mb.cursor as usize;
                    let cursor = input
                        .char_indices()
                        .take_while(|(i, _)| *i < cursor_byte)
                        .count() as u32;
                    let total = session.candidates.len() as u32;
                    let (candidates, selected) =
                        minibuffer_window(&session.candidates, session.selected);
                    (
                        Some(session.prompt.clone()),
                        input,
                        cursor,
                        candidates,
                        selected,
                        total,
                    )
                }
                None => (None, String::new(), 0, Vec::new(), None, 0),
            }
        };
        if self.last_minibuffer.as_ref() == Some(&facts) {
            return None;
        }
        // First sight while closed: nothing to clear, stay silent.
        if self.last_minibuffer.is_none() && facts.0.is_none() {
            self.last_minibuffer = Some(facts);
            return None;
        }
        let msg = InstanceMessage::MinibufferPrompt {
            prompt: facts.0.clone(),
            input: facts.1.clone(),
            cursor: facts.2,
            candidates: facts.3.clone(),
            selected: facts.4,
            total: facts.5,
        };
        self.last_minibuffer = Some(facts);
        Some(msg)
    }

    /// The `StatusFacts` message for this frame, or `None` when
    /// nothing changed. Carries the facts a semantic frontend cannot
    /// derive locally: buffer name, modified flag, whole-file
    /// diagnostic counts (errors / warnings). Counts freeze at their
    /// last published value while the diag store is stale — mid-edit
    /// positions are wrong but *counts* merely lag, and flickering
    /// to zero on every keystroke would be worse. The freeze IS the
    /// store's retained state (rounds 3–5): `mark_stale` keeps the
    /// last published diagnostics and their cached severity totals,
    /// so reading the totals while stale yields the frozen value in
    /// O(1) with no session state to lose — not to a snapshot reset,
    /// and not by attaching mid-edit. The daemon's write loop keeps
    /// the variant off wires negotiated `< 8`.
    fn status_facts_msg(
        &mut self,
        state: &EditorState,
        buffer_id: BufferId,
    ) -> Option<InstanceMessage> {
        let (name, modified, message) = {
            let core = state.core.borrow();
            // The transient status message (`pmacs.editor.set_status`
            // — LSP command summaries, error reports). The attached
            // TUI reads it off the rendered bottom row; a semantic
            // frontend only sees this wire (v15).
            let message = (!core.status.is_empty()).then(|| core.status.clone());
            let registry = core.registry.clone();
            let reg = registry.borrow();
            let buf = reg.get(buffer_id).ok()?;
            (buf.name().to_owned(), buf.is_modified(), message)
        };
        let (diag_errors, diag_warnings) = {
            let core = state.core.borrow();
            buffer_file_uri(&core, buffer_id).map_or((0, 0), |uri| {
                let store = state.lsp_manager.borrow().diag_store();
                let guard = store.lock().expect("diag store mutex poisoned");
                // Read even while the store is STALE (round 4):
                // `mark_stale` keeps the last published diagnostics
                // (T M11.8) — positions are invalid mid-edit, but
                // counts merely lag, so the retained totals ARE the
                // frozen value. Sourcing the freeze from the store
                // rather than any per-session cache means a session
                // first rendering during staleness — a late joiner,
                // or a buffer first visited mid-edit — reports the
                // preserved counts instead of zeros, and the snapshot
                // reset has nothing count-related to preserve.
                // `DiagnosticStore::set` computes this tuple once
                // (round 5). StatusFacts runs at frame cadence for
                // every semantic session, so rescanning the retained
                // vector here would make stale intervals
                // O(frames * diagnostics).
                let (errors, warnings, _, _) = guard.severity_counts_for(&uri);
                (errors, warnings)
            })
        };
        let facts = (name, modified, diag_errors, diag_warnings, message);
        if self.last_status.get(&buffer_id) == Some(&facts) {
            return None;
        }
        let msg = InstanceMessage::StatusFacts {
            buffer_id,
            name: facts.0.clone(),
            modified: facts.1,
            diag_errors,
            diag_warnings,
            message: facts.4.clone(),
        };
        self.last_status.insert(buffer_id, facts);
        Some(msg)
    }

    /// The `LineNumbers` message for this frame, or `None` when the gutter
    /// mode hasn't changed (UX gutter arc, protocol v14). The mode lives on
    /// this frontend's active window; a semantic frontend renders the gutter
    /// locally (it owns the text + its cursor line, so relative/hybrid need
    /// no round trip) but the daemon owns the mode, so it ships which one.
    /// The daemon's write loop keeps the variant off wires negotiated `< 14`.
    fn line_numbers_msg(
        &mut self,
        state: &EditorState,
        buffer_id: BufferId,
    ) -> Option<InstanceMessage> {
        // Bottom-panel §1.3 #4 — Projection. `LineNumbers` describes the
        // replica's DOCUMENT surface; a focused panel must not replace
        // the document's gutter mode with the panel window's.
        let mode = {
            let core = state.core.borrow();
            core.primary_document_window(self.frontend_id)
                .and_then(|win_id| core.windows.get(&win_id))
                .map_or(crate::window::LineNumberMode::Off, |w| w.line_numbers)
        };
        if self.last_line_numbers == Some(mode) {
            return None;
        }
        self.last_line_numbers = Some(mode);
        Some(InstanceMessage::LineNumbers { buffer_id, mode })
    }

    /// The `InlineAdornments` message for this frame, or `None` when
    /// nothing should be sent. The wire variant has no
    /// `generation`/`full`/`segments`, so this is M11.2-level
    /// suppression only: the whole scoped set re-sends on any change,
    /// nothing when byte-identical, and never an empty frame when
    /// there is simply nothing to say. Updates the baseline on send.
    fn inline_adornments_msg(
        &mut self,
        state: &EditorState,
        vp: &DeclaredViewport,
    ) -> Option<InstanceMessage> {
        // Hold-while-stale — mirrors the Decorations hold in
        // `render_frame`. An empty frame here wipes the frontend's
        // cached virtual text mid-typing-burst, and inline adornments
        // occupy layout space: the wipe visibly shifts real glyphs
        // (and forces a reshape), then the post-refresh re-emit
        // shifts them back. The frontend's locally-translated cache
        // is the better picture until a fresh `inlayHint` response
        // clears the stale flag and re-emits through the diff below.
        if inlay_store_stale(state, vp.buffer_id) {
            return None;
        }
        let adornments = scoped_inline_adornments(state, vp);
        let should_emit = match self.last_adornments.get(&vp.buffer_id) {
            // First sight of this buffer: speak only if there is
            // something to show — no empty-frame spam.
            None => !adornments.is_empty(),
            // Re-send on a real change. `empty → empty` conveys
            // nothing and is suppressed; `non-empty → empty` *is* a
            // change worth sending so the frontend clears its overlay.
            Some(p) => {
                (p.visible != vp.visible || p.items != adornments)
                    && !(adornments.is_empty() && p.items.is_empty())
            }
        };
        if !should_emit {
            return None;
        }
        // Adornments use the same `LastFrame` struct as
        // StyleSpans / Decorations, so we populate `generation` for
        // consistency. Adornments' diff predicate doesn't consult
        // it — the whole-set comparison at line 310 catches post-
        // edit position shifts directly — but tracking it keeps
        // the struct shape uniform.
        let generation = buffer_generation(state, vp.buffer_id);
        self.last_adornments.insert(
            vp.buffer_id,
            LastFrame {
                visible: vp.visible,
                items: adornments.clone(),
                generation,
            },
        );
        Some(InstanceMessage::InlineAdornments {
            buffer_id: vp.buffer_id,
            items: adornments,
        })
    }

    /// The `FoldState` message for this frame, or `None` (Arc 6). The
    /// instance's authoritative fold set for `buffer_id`, whole-buffer
    /// (folds are a handful; `close-all` is top-level only, so the set is
    /// O(top-level blocks) — no viewport scoping). Authoritative-empty and
    /// diff-suppressed exactly like `inline_adornments_msg`: the first
    /// sight of a buffer speaks only if a fold exists, an unchanged set is
    /// silent, and a `non-empty → empty` transition emits one empty frame
    /// so the frontend clears its mirror. Its baseline resets on
    /// `BufferSnapshot`; see `on_buffer_snapshot_sent`.
    fn fold_state_msg(
        &mut self,
        state: &EditorState,
        buffer_id: BufferId,
    ) -> Option<InstanceMessage> {
        let folds = state.fold_registry.folds(buffer_id);
        let should_emit = match self.last_folds.get(&buffer_id) {
            // First sight: speak only if there is a fold to show.
            None => !folds.is_empty(),
            // Any change: `empty → empty` is byte-identical and suppressed,
            // `non-empty → empty` differs and emits one clearing frame.
            Some(prev) => *prev != folds,
        };
        if !should_emit {
            return None;
        }
        self.last_folds.insert(buffer_id, folds.clone());
        Some(InstanceMessage::FoldState { buffer_id, folds })
    }

    /// The `FileStyleSummary` message for this frame, or `None`. The
    /// summary is keyed on CRDT `generation`: a buffer with an
    /// unchanged generation re-uses the cached summary and emits
    /// nothing. The first frame for a buffer always emits (the
    /// frontend needs the baseline). Updates the baseline on send.
    fn file_style_summary_msg(
        &mut self,
        state: &EditorState,
        buffer_id: BufferId,
        generation: u64,
    ) -> Option<InstanceMessage> {
        // The summary is a *whole-file* tree-sitter pass (the minimap
        // needs every line). Recomputing it on every edit's generation
        // bump was a per-keystroke O(file) cost — a major part of the
        // typing slowness. For grammar-backed buffers, debounce it to
        // reparse-completion. `pending_edit_count()` alone is not
        // enough: dispatch drains that list immediately, leaving the
        // expensive summary path free to run while a parse job is still
        // in flight. Wait until there is an installed parse, no pending
        // edits, and no recorded parse job for this buffer.
        if grammar_style_parse_not_ready(state, buffer_id) {
            return None;
        }
        // Diagnostics fold into the summary (minimap marks) but
        // publish without a generation bump — key the cache on the
        // diag store's per-URI epoch as well, so a republish
        // refreshes the marks and anything else stays suppressed.
        // The theme epochs (Q#TH6) join the key so a mid-session
        // recolor refreshes the strokes — face_epoch included, since
        // `ui.diag.*` feeds the marks.
        let diag_epoch = diagnostics_epoch(state, buffer_id);
        let (syntax_epoch, face_epoch) = theme_epochs(state);
        // A v15 peer's marks never resolve faces (finding 3), so a
        // face mutation cannot change its summary either — zero the
        // key component rather than recompute a whole-file pass per
        // face edit just to payload-suppress it.
        let face_key = if self.peer_knows_theme_facts {
            face_epoch
        } else {
            0
        };
        let key = (generation, diag_epoch, syntax_epoch, face_key);
        if self
            .last_summary
            .get(&buffer_id)
            .is_some_and(|c| c.key == key)
        {
            return None;
        }
        let lines = scoped_file_summary(state, buffer_id, self.peer_knows_theme_facts);
        // Payload-equality suppression (Q#TH6): a face edit that
        // leaves the summary unchanged (e.g. `ui.modeline`) emits
        // nothing — but the key still advances, on computation rather
        // than emission, or this whole-file pass would repeat every
        // tick.
        let unchanged = self
            .last_summary
            .get(&buffer_id)
            .is_some_and(|c| c.lines == lines);
        self.last_summary.insert(
            buffer_id,
            SummaryCache {
                key,
                lines: lines.clone(),
            },
        );
        if unchanged {
            return None;
        }
        Some(InstanceMessage::FileStyleSummary {
            buffer_id,
            generation,
            lines,
        })
    }

    /// Build the authoritative `ThemeFacts` table. v16/v17 peers retain the
    /// fixed stage-1 inventory and never inspect the statusline registry.
    /// v18 peers union in enabled provider faces and key recomputation on
    /// `(theme.face_epoch, registry.face_set_epoch)`.
    fn theme_facts_msg(&mut self, state: &EditorState) -> Option<InstanceMessage> {
        if !self.peer_knows_theme_facts {
            return None;
        }

        let theme = state.syntax_registry.theme();
        let th = theme.lock().expect("theme mutex poisoned");
        let (face_set_epoch, dynamic_faces) = if self.peer_knows_statusline_segments {
            let registry = state.statusline_registry.borrow();
            let face_set_epoch = registry.face_set_epoch();
            if self.last_face_epoch == Some(th.face_epoch)
                && self.last_statusline_face_set_epoch == Some(face_set_epoch)
            {
                return None;
            }
            (Some(face_set_epoch), registry.enabled_face_names())
        } else {
            if self.last_face_epoch == Some(th.face_epoch) {
                return None;
            }
            (None, Vec::new())
        };

        let mut names: Vec<String> = UI_FACES.iter().map(|name| (*name).to_owned()).collect();
        names.extend(dynamic_faces);
        names.sort_unstable();
        names.dedup();
        let faces = names
            .into_iter()
            .filter_map(|name| {
                let style = if UI_FACES.binary_search(&name.as_str()).is_ok() {
                    th.face(&name)
                } else {
                    th.modeline_segment_face(&name)
                };
                style.map(|style| crate::protocol::ThemeFace { name, style })
            })
            .collect::<Vec<_>>();
        let face_epoch = th.face_epoch;
        drop(th);

        self.last_face_epoch = Some(face_epoch);
        self.last_statusline_face_set_epoch = face_set_epoch;
        let unchanged = self.last_theme_faces.as_ref() == Some(&faces);
        self.last_theme_faces = Some(faces.clone());
        if unchanged {
            return None;
        }
        Some(InstanceMessage::ThemeFacts { faces })
    }

    /// The `FontFacts` message for this frame, or `None` when the
    /// preference is unchanged (Arc 4 stage 2, Q#F5, protocol v17).
    /// The `theme_facts_msg` discipline exactly: an `Option`-seeded
    /// epoch gate keeps unchanged ticks to one `u64` compare, the
    /// `Option`-seeded payload baseline decides emission, both
    /// advance on computation, and every attachment ships exactly
    /// one authoritative preference — the all-default `(None, None)`
    /// included — on its first frame after viewport declaration.
    /// Bufferless: `on_buffer_snapshot_sent` never touches these
    /// baselines.
    fn font_facts_msg(&mut self, state: &EditorState) -> Option<InstanceMessage> {
        // Never produced for a peer below v17 (the daemon write-loop
        // gate remains as the belt-and-braces filter).
        if !self.peer_knows_font_facts {
            return None;
        }
        let (facts, epoch) = {
            let pref = state.font_pref.lock().expect("font pref mutex poisoned");
            if self.last_font_epoch == Some(pref.epoch) {
                return None;
            }
            ((pref.family.clone(), pref.size_centi_px), pref.epoch)
        };
        self.last_font_epoch = Some(epoch);
        let unchanged = self.last_font_facts.as_ref() == Some(&facts);
        self.last_font_facts = Some(facts.clone());
        if unchanged {
            return None;
        }
        Some(InstanceMessage::FontFacts {
            family: facts.0,
            size_centi_px: facts.1,
        })
    }

    /// The wrap mode for the buffer this session is showing (v22).
    ///
    /// `pmacs-gpu` lays out locally and ignores the grid family, so
    /// without this it would never hear `ui.line-wrap` at all and would
    /// keep wrapping while the TUI truncated — the cross-frontend
    /// disagreement the long-lines stage exists to close.
    ///
    /// Deduped on the `(buffer, wrap)` PAIR. That is not a
    /// micro-optimisation: the mode is buffer-local, so switching from a
    /// truncating buffer to a wrapping one changes the effective mode
    /// with **no config event at all**. A cache keyed on the mode alone
    /// would stay silent through exactly that transition — and would
    /// look correct in every single-buffer test.
    fn line_wrap_msg(
        &mut self,
        state: &EditorState,
        buffer_id: crate::buffer::BufferId,
    ) -> Option<InstanceMessage> {
        if !self.peer_knows_line_wrap {
            return None;
        }
        let wrap = matches!(
            crate::lua_bindings::config_line_wrap(state.lua_host.lua(), Some(buffer_id)),
            crate::view::WrapMode::Wrap
        );
        if self.last_line_wrap == Some((buffer_id, wrap)) {
            return None;
        }
        self.last_line_wrap = Some((buffer_id, wrap));
        Some(InstanceMessage::LineWrapFacts { buffer_id, wrap })
    }

    /// Project the [`Decoration`] set intersecting the declared
    /// viewport: the session's selection (instance-authoritative,
    /// byte-native) and LSP diagnostics (line/col → byte, severity →
    /// kind). Search-hit and current-line decorations are
    /// deliberately absent: pmacs has no instance-side search-hit
    /// store, and current-line is a pure cursor derivation the
    /// frontend already owns (it has `CursorByte`) — emitting it would
    /// couple a visual-motion concern to the instance, against the
    /// contract boundary.
    /// Compute the scoped style spans and push a `StyleSpans` message
    /// (full resync or M11.4 incremental) when they differ from the
    /// last sent baseline. Extracted from `render_frame` so the perf
    /// gate there can skip it wholesale on unchanged ticks.
    fn emit_style_spans(
        &mut self,
        state: &EditorState,
        vp: &DeclaredViewport,
        generation: u64,
        out: &mut Vec<InstanceMessage>,
    ) {
        let spans = scoped_style_spans(state, vp);
        let prev = self.last_sent.get(&vp.buffer_id);
        // Resync when there is no baseline, the declared viewport
        // region moved (scoping window changed), OR the CRDT
        // generation advanced (T M11.7: text edits shift byte
        // positions, so prior spans are stale and incremental
        // updates can't restore the full viewport).
        let full = prev.is_none_or(|p| p.visible != vp.visible || p.generation != generation);
        if full {
            self.last_sent.insert(
                vp.buffer_id,
                LastFrame {
                    visible: vp.visible,
                    items: spans.clone(),
                    generation,
                },
            );
            out.push(InstanceMessage::StyleSpans {
                buffer_id: vp.buffer_id,
                generation,
                full: true,
                segments: vec![StyleSegment {
                    range: vp.visible,
                    spans,
                }],
            });
        } else {
            let prev = prev.expect("checked is_none_or above");
            let intervals = changed_intervals(&prev.items, &spans, |s| s.range);
            if !intervals.is_empty() {
                let segments = intervals
                    .into_iter()
                    .map(|range| StyleSegment {
                        range,
                        spans: clip_style_spans(range, &spans),
                    })
                    .collect();
                self.last_sent.insert(
                    vp.buffer_id,
                    LastFrame {
                        visible: vp.visible,
                        items: spans,
                        generation,
                    },
                );
                out.push(InstanceMessage::StyleSpans {
                    buffer_id: vp.buffer_id,
                    generation,
                    full: false,
                    segments,
                });
            }
            // No dirty interval → styling unchanged → emit nothing.
        }
    }

    fn scoped_decorations(
        &mut self,
        state: &EditorState,
        vp: &DeclaredViewport,
    ) -> Vec<Decoration> {
        let core = state.core.borrow();
        let registry = core.registry.clone();
        let reg = registry.borrow();
        let mut out = Vec::new();

        // Selection is per-window (per-frontend) state. CurrentLine is
        // deliberately not emitted for semantic frontends: the GPU has
        // CursorByte and paints its own caret/current-line affordances.
        // Emitting CurrentLine here forced a whole-buffer line table on
        // every frame even though pmacs-gpu ignores its own current-line
        // wash.
        // Bottom-panel §1.3 #5 — Projection. Selection decorations
        // belong to the document surface the viewport describes; a
        // selection made inside a focused panel must not paint into it.
        if let Some(win) = core
            .primary_document_window(self.frontend_id)
            .and_then(|win_id| core.windows.get(&win_id))
            && win.buffer_id == vp.buffer_id
            && let Some((lo, hi)) = win.region()
            && let Some(range) = clip_to_viewport(lo, hi, vp)
        {
            out.push(Decoration {
                range,
                kind: DecorationKind::Selection,
            });
        }

        // Diagnostics — keyed in the shared store by the file URI the
        // Lua LSP glue opened the document under. The URI is derived
        // from `vp.buffer_id` (see [`buffer_file_uri`]'s docs for why
        // not from `core.active_buffer_path()`).
        //
        // T M11.8 — skip emission while the store is stale (the
        // document has been edited since the last `publishDiagnostics`
        // absorption). Without this, the producer ships diagnostics
        // whose byte positions point at pre-edit text — the visible
        // wrong-position color artifact session-5 validation surfaced.
        // The LSP layer's `did_change_full` marks the URI stale; the
        // next `publishDiagnostics` absorbs and clears the flag.
        if let Some(uri) = buffer_file_uri(&core, vp.buffer_id) {
            let (diags, is_stale) = {
                let store = state.lsp_manager.borrow().diag_store();
                let guard = store.lock().expect("diag store mutex poisoned");
                (guard.for_uri(&uri).to_vec(), guard.is_stale(&uri))
            };
            if !is_stale
                && !diags.is_empty()
                && let Ok(buf) = reg.get(vp.buffer_id)
            {
                // Byte<->line mapping, cached per buffer revision —
                // rebuilding it is an O(buffer) rope copy + scan, far
                // too expensive to repeat on every tick a diagnostic
                // is on screen.
                let cache = self
                    .diag_line_cache
                    .entry(vp.buffer_id)
                    .and_modify(|c| {
                        if c.revision != buf.revision() {
                            let s = buffer_source_bytes(buf);
                            c.revision = buf.revision();
                            c.line_starts = line_start_offsets(&s);
                            c.source_len = s.len() as u64;
                        }
                    })
                    .or_insert_with(|| {
                        let s = buffer_source_bytes(buf);
                        DiagLineCache {
                            revision: buf.revision(),
                            line_starts: line_start_offsets(&s),
                            source_len: s.len() as u64,
                        }
                    });
                let (line_starts, source_len) = (&cache.line_starts, cache.source_len);
                for d in &diags {
                    let lo = line_col_to_byte(line_starts, source_len, d.start_line, d.start_col);
                    let hi = line_col_to_byte(line_starts, source_len, d.end_line, d.end_col);
                    let (lo, hi) =
                        widen_zero_width_diag(lo, hi, d.start_line, line_starts, source_len);
                    if let Some(range) = clip_to_viewport(lo, hi, vp) {
                        out.push(Decoration {
                            range,
                            kind: severity_to_kind(d.severity),
                        });
                    }
                }
            }
        }

        // In-buffer search matches (Q#SR3). Already byte ranges — no
        // line/col conversion. Skipped while stale (an edit leaves the
        // matches at pre-edit positions until the next re-search, the
        // M11.8 model). The active match emits `SearchMatchActive`,
        // the rest `SearchMatch`; matches are non-overlapping so each
        // range carries exactly one kind.
        {
            let store = core.search_store.clone();
            let guard = store.lock().expect("search store mutex poisoned");
            if !guard.is_stale(vp.buffer_id)
                && let Some(search) = guard.for_buffer(vp.buffer_id)
            {
                let active = search.active_match();
                for m in search.matches() {
                    if let Some(range) = clip_to_viewport(m.start, m.end, vp) {
                        out.push(Decoration {
                            range,
                            kind: if Some(*m) == active {
                                DecorationKind::SearchMatchActive
                            } else {
                                DecorationKind::SearchMatch
                            },
                        });
                    }
                }
            }
        }

        out
    }
}

/// Project the LSP inlay-hint set intersecting the declared viewport
/// into [`InlineAdornment`]s. Mirrors the diagnostics half of
/// `scoped_decorations`: same path → URI → store (`for_uri`) lookup.
/// Step 0 established inlay-hint columns are already pmacs byte
/// offsets by the time they reach the store (the absorb path's
/// `inbound_converted` rewrites the `Position`-shaped
/// `InlayHint.position`), so `line_col_to_byte` — which treats the
/// column as a byte offset — is exact here, no per-server encoding
/// needed (unlike semantic-token styling). Takes no `self`: the
/// session id is irrelevant (inlay hints are per-buffer, not
/// per-window like selection), mirroring `scoped_style_spans`.
///
/// Every hint is `AtOffset` (inlay hints are inline by definition)
/// carrying `Text` with the (padding-applied) label and the default
/// style — the instance has no inlay-specific theme face yet, and a
/// fabricated one would be dishonest. Stale store entries are
/// suppressed the same way stale diagnostics / semantic tokens are:
/// a zero-width hint anchored to pre-edit text is still a byte range
/// bug, even though it has no source-byte width of its own.
fn scoped_inline_adornments(state: &EditorState, vp: &DeclaredViewport) -> Vec<InlineAdornment> {
    let core = state.core.borrow();
    let Some(uri) = buffer_file_uri(&core, vp.buffer_id) else {
        return Vec::new();
    };
    let hints = {
        let store = state.lsp_manager.borrow().inlay_hint_store();
        let guard = store.lock().expect("inlay-hint store mutex poisoned");
        if guard.is_stale(&uri) {
            return Vec::new();
        }
        match guard.for_uri(&uri) {
            Some(resp) => resp.hints.clone(),
            None => return Vec::new(),
        }
    };
    if hints.is_empty() {
        return Vec::new();
    }
    let registry = core.registry.clone();
    let reg = registry.borrow();
    let Ok(buf) = reg.get(vp.buffer_id) else {
        return Vec::new();
    };
    let source = buffer_source_bytes(buf);
    let source_len = source.len() as u64;
    let line_starts = line_start_offsets(&source);
    let vis_start = vp.visible.start.min(source_len);
    let vis_end = vp.visible.end.min(source_len);

    let mut out = Vec::new();
    for h in &hints {
        let at = line_col_to_byte(&line_starts, source_len, h.line, h.col);
        // An inlay hint occupies no bytes; include it when its anchor
        // lies within the declared viewport (half-open).
        if at < vis_start || at >= vis_end {
            continue;
        }
        let mut text = String::new();
        if h.padding_left {
            text.push(' ');
        }
        text.push_str(&h.label);
        if h.padding_right {
            text.push(' ');
        }
        out.push(InlineAdornment {
            at,
            placement: AdornmentPlacement::AtOffset,
            content: AdornmentContent::Text {
                text,
                style: Style::default(),
            },
        });
    }
    out
}

/// Intersect `[lo, hi)` with the declared viewport (itself clamped to
/// the source length is the caller's concern for styling; for
/// decorations we clamp against the viewport only). `None` when the
/// intersection is empty or degenerate.
/// Widen a zero-width diagnostic range to one byte so it survives
/// the wire and overlaps a glyph at the frontend (a zero-width range
/// clips to nothing and underlines nothing). Parsers anchor
/// "expected COMMA"-style errors one past the last token —
/// rust-analyzer reports the missing comma as `col N → col N` at end
/// of line — the same shape the TUI's `DiagnosticView` special-cases
/// at its anchor cell (T M4.6). Mid-line anchors widen forward; an
/// anchor at/past the line's content end widens backward instead,
/// because forward would cover only the `\n`, which shapes no glyph.
/// Non-empty ranges pass through untouched.
fn widen_zero_width_diag(
    lo: u64,
    hi: u64,
    start_line: u32,
    line_starts: &[u64],
    source_len: u64,
) -> (u64, u64) {
    if hi > lo {
        return (lo, hi);
    }
    // Content end excludes the trailing newline, same semantics as
    // the summary's per-line ranges.
    let content_end = line_starts
        .get(start_line as usize + 1)
        .map_or(source_len, |&next| next.saturating_sub(1));
    if lo >= content_end {
        (lo.saturating_sub(1), lo)
    } else {
        (lo, (lo + 1).min(source_len))
    }
}

fn clip_to_viewport(lo: u64, hi: u64, vp: &DeclaredViewport) -> Option<ByteRange> {
    let start = lo.max(vp.visible.start);
    let end = hi.min(vp.visible.end);
    if end <= start {
        return None;
    }
    Some(ByteRange { start, end })
}

/// T M11.4 — the dirty byte intervals between two ordered item sets.
///
/// Items are byte-anchored (`range_of` extracts the range). The
/// symmetric difference (items in exactly one set, by `==`) bounds
/// every byte whose covering set changed; its ranges are coalesced
/// into maximal disjoint intervals — the segments the frontend will
/// clear and repaint. Empty result ⇒ unchanged ⇒ the caller emits
/// nothing.
///
/// O(n·m) membership scans: a screenful is a few hundred items, far
/// cheaper than re-shipping the whole viewport every frame, and only
/// runs when the fast `prev == curr` slice check (caller side, via
/// the order-stable producers) would have failed anyway.
fn changed_intervals<T: PartialEq>(
    prev: &[T],
    curr: &[T],
    range_of: impl Fn(&T) -> ByteRange,
) -> Vec<ByteRange> {
    let mut changed: Vec<ByteRange> = Vec::new();
    for p in prev {
        if !curr.contains(p) {
            changed.push(range_of(p));
        }
    }
    for c in curr {
        if !prev.contains(c) {
            changed.push(range_of(c));
        }
    }
    coalesce_ranges(&mut changed)
}

/// Sort and merge overlapping or touching ranges into maximal
/// disjoint intervals. Zero-width ranges are dropped (nothing to
/// repaint). Consumes `ranges` (sorts in place).
fn coalesce_ranges(ranges: &mut Vec<ByteRange>) -> Vec<ByteRange> {
    ranges.retain(|r| r.end > r.start);
    ranges.sort_by_key(|r| (r.start, r.end));
    let mut out: Vec<ByteRange> = Vec::new();
    for r in ranges.iter().copied() {
        match out.last_mut() {
            // Touching (`>=`) merges too: adjacent dirty ranges become
            // one segment rather than two abutting clears.
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

/// Every span intersecting `iv`, clipped to it, order preserved.
fn clip_style_spans(iv: ByteRange, spans: &[StyleSpan]) -> Vec<StyleSpan> {
    spans
        .iter()
        .filter_map(|s| {
            let start = s.range.start.max(iv.start);
            let end = s.range.end.min(iv.end);
            (end > start).then_some(StyleSpan {
                range: ByteRange { start, end },
                style: s.style,
            })
        })
        .collect()
}

/// Every decoration intersecting `iv`, clipped to it, order preserved.
fn clip_decorations(iv: ByteRange, decos: &[Decoration]) -> Vec<Decoration> {
    decos
        .iter()
        .filter_map(|d| {
            let start = d.range.start.max(iv.start);
            let end = d.range.end.min(iv.end);
            (end > start).then_some(Decoration {
                range: ByteRange { start, end },
                kind: d.kind,
            })
        })
        .collect()
}

/// True for the four diagnostic-underline decoration kinds — the
/// family whose emission is gated on diag-store staleness by the
/// hold-while-stale logic in `render_frame`.
fn is_diagnostic_kind(kind: DecorationKind) -> bool {
    matches!(
        kind,
        DecorationKind::DiagnosticError
            | DecorationKind::DiagnosticWarning
            | DecorationKind::DiagnosticInfo
            | DecorationKind::DiagnosticHint
    )
}

/// True when `buffer_id`'s entry in the diagnostics store is stale
/// (the document changed since the last `publishDiagnostics`
/// absorption). Buffers with no file URI are never stale.
fn diagnostics_store_stale(state: &EditorState, buffer_id: BufferId) -> bool {
    let core = state.core.borrow();
    let Some(uri) = buffer_file_uri(&core, buffer_id) else {
        return false;
    };
    let store = state.lsp_manager.borrow().diag_store();
    let guard = store.lock().expect("diag store mutex poisoned");
    guard.is_stale(&uri)
}

/// The diag store's per-URI change epoch for `buffer_id`'s file, `0`
/// for buffers with no file URI or no diagnostics history. Keys the
/// `FileStyleSummary` cache (see [`SemanticRenderState::last_summary`]).
fn diagnostics_epoch(state: &EditorState, buffer_id: BufferId) -> u64 {
    let core = state.core.borrow();
    let Some(uri) = buffer_file_uri(&core, buffer_id) else {
        return 0;
    };
    let store = state.lsp_manager.borrow().diag_store();
    let guard = store.lock().expect("diag store mutex poisoned");
    guard.epoch_for(&uri)
}

/// Style-family staleness for the LSP-token authority. True only for
/// a buffer with **no** tree-sitter view (policy A routes those
/// through `lsp_scoped_style_spans`) whose semantic-token store entry
/// is stale. Grammar-backed buffers always return `false` — their
/// styling freshness is `grammar_style_parse_not_ready`'s job.
fn lsp_style_tokens_stale(state: &EditorState, buffer_id: BufferId) -> bool {
    if state.syntax_registry.view(buffer_id).is_some() {
        return false;
    }
    let core = state.core.borrow();
    let Some(uri) = buffer_file_uri(&core, buffer_id) else {
        return false;
    };
    let store = state.lsp_manager.borrow().semantic_token_store();
    let guard = store.lock().expect("semantic token store mutex poisoned");
    guard.is_stale(&uri)
}

/// Inlay-hint twin of [`diagnostics_store_stale`].
fn inlay_store_stale(state: &EditorState, buffer_id: BufferId) -> bool {
    let core = state.core.borrow();
    let Some(uri) = buffer_file_uri(&core, buffer_id) else {
        return false;
    };
    let store = state.lsp_manager.borrow().inlay_hint_store();
    let guard = store.lock().expect("inlay-hint store mutex poisoned");
    guard.is_stale(&uri)
}

/// Map an LSP diagnostic severity onto the wire decoration kind.
fn severity_to_kind(sev: crate::diag::DiagnosticSeverity) -> DecorationKind {
    use crate::diag::DiagnosticSeverity as S;
    match sev {
        S::Error => DecorationKind::DiagnosticError,
        S::Warning => DecorationKind::DiagnosticWarning,
        S::Information => DecorationKind::DiagnosticInfo,
        S::Hint => DecorationKind::DiagnosticHint,
    }
}

/// Resolve `buffer_id` → `file://…` URI, using the same encoding the
/// Lua side uses in `file_uri_for` so the result is byte-identical to
/// the diag-store / inlay-store / semantic-token-store keys.
///
/// **Bug-fix anchor (post-session-5 finding):** the producer used to
/// derive this URI from `core.active_buffer_path()` — the *editor's*
/// active buffer, not the buffer the frame is projecting. In multi-
/// frontend setups (TUI + `pmacs-gpu`) those diverge: each frontend
/// has its own active window, and when the daemon renders frontend B's
/// frame it temporarily flips `active_frontend` to B but B's active
/// buffer may be a scratch buffer with no file path. The diag /
/// inlay / semantic-token lookups would then run against the wrong
/// URI (or `None`) and return empty, so frontend B got no
/// `Decorations` / `InlineAdornments` / LSP-driven `StyleSpans`.
/// Routing the URI through `vp.buffer_id` fixes the multi-frontend
/// case without disturbing the single-frontend one
/// (`active_buffer_id == vp.buffer_id` there, so the resolved path is
/// the same).
fn buffer_file_uri(core: &crate::editor_core::EditorCore, buffer_id: BufferId) -> Option<String> {
    let reg = core.registry.borrow();
    let buf = reg.get(buffer_id).ok()?;
    let path = buf.file_path()?;
    Some(crate::lsp::path_to_file_uri(path))
}

/// Snapshot a buffer's bytes (refcount-cheap rope slice, mirroring
/// `diag.rs`'s render-time snapshot).
fn buffer_source_bytes(buf: &crate::buffer::Buffer) -> Vec<u8> {
    let len = buf.len();
    let mut bytes = vec![0u8; len as usize];
    if !bytes.is_empty() {
        buf.snapshot_rope().slice(0, len, &mut bytes);
    }
    bytes
}

/// Byte offset of the start of each line (index 0 = byte 0; one entry
/// per line, where a line is a maximal run ended by `\n`).
fn line_start_offsets(source: &[u8]) -> Vec<u64> {
    let mut starts = vec![0u64];
    for (i, b) in source.iter().enumerate() {
        if *b == b'\n' {
            starts.push(i as u64 + 1);
        }
    }
    starts
}

/// Translate an LSP `(line, col)` to a byte offset. pmacs v0.1 treats
/// the LSP column as a byte offset within the line (see
/// `crate::diag::Diagnostic`'s field docs); we clamp to the line's
/// end and the source length so a stale diagnostic from before an
/// edit can never index out of range.
fn line_col_to_byte(line_starts: &[u64], source_len: u64, line: u32, col: u32) -> u64 {
    let li = line as usize;
    let Some(&line_start) = line_starts.get(li) else {
        return source_len;
    };
    let line_end = line_starts
        .get(li + 1)
        .map_or(source_len, |&next| next.saturating_sub(1));
    (line_start + u64::from(col)).min(line_end).min(source_len)
}

/// Cheap recompute-gate key for [`scoped_style_spans`] on a grammar-
/// backed buffer. Returns `None` for buffers with no tree-sitter view
/// (the LSP-token path), which has no comparably cheap revision handle
/// and therefore is never gated. `bundle.source_revision` is read via
/// the same `current()` accessor `scoped_style_spans` uses, so the key
/// flips exactly when the spans it would produce can change.
fn grammar_style_key(
    state: &EditorState,
    vp: &DeclaredViewport,
    generation: u64,
) -> Option<StyleGate> {
    let handle = state.syntax_registry.view(vp.buffer_id)?;
    Some(StyleGate {
        bundle: handle.current(),
        generation,
        visible: vp.visible,
        syntax_epoch: theme_epochs(state).0,
    })
}

fn grammar_style_parse_not_ready(state: &EditorState, buffer_id: BufferId) -> bool {
    let Some(handle) = state.syntax_registry.view(buffer_id) else {
        return false;
    };
    handle.current().is_none()
        || handle.pending_edit_count() > 0
        || state.syntax_registry.has_pending_parse_job_for(buffer_id)
}

/// Compute the styled byte runs intersecting the declared viewport,
/// mapped through the active theme. Spans are clipped to the viewport
/// and to the parsed source length; runs that resolve to the default
/// style are dropped (wire economy, and consistent with the grid
/// path, which skips default-style merges).
fn scoped_style_spans(state: &EditorState, vp: &DeclaredViewport) -> Vec<StyleSpan> {
    // Policy A — per-language styling authority. A grammar-backed
    // language (the registry hands out a view only for those) is
    // styled *solely* by tree-sitter; a language with no bundled
    // grammar (C/C++, …) is styled *solely* by LSP semantic tokens.
    // Never both: this is why the no-view branch hands off to the LSP
    // producer while a grammar-backed buffer whose parse isn't ready
    // yet returns empty rather than briefly borrowing LSP styling
    // (which would flicker two authorities on one buffer).
    let Some(handle) = state.syntax_registry.view(vp.buffer_id) else {
        return lsp_scoped_style_spans(state, vp);
    };
    let Some(bundle) = handle.current() else {
        return Vec::new();
    };
    let theme = state
        .syntax_registry
        .theme()
        .lock()
        .expect("theme mutex poisoned")
        .clone();

    let source: &[u8] = bundle.source.as_ref();
    let source_len = source.len() as u64;
    let vis_start = vp.visible.start.min(source_len);
    let vis_end = vp.visible.end.min(source_len);
    if vis_end <= vis_start {
        return Vec::new();
    }

    // Collect the styled spans from every injection layer, scoping each
    // capture walk to the visible byte range so re-styling on each edit is
    // O(visible), not O(file) (framing Q#S6). Each span carries a priority
    // `(layer_index, capture_order)`: a deeper layer wins over a shallower
    // one, a later same-depth sibling wins over an earlier one, and within a
    // layer the wider-first order lets narrower captures override (framing
    // Q#IJ6). Fully-default styles are dropped (they fold as identity anyway).
    let mut styled: Vec<StyledLayerSpan> = Vec::new();
    for (layer_idx, layer) in bundle.layers.iter().enumerate() {
        let Some(query) = layer.highlight_query.as_ref() else {
            continue;
        };
        let names = query.capture_names();
        let highlights = crate::syntax::compute_highlight_spans_for(
            query,
            &layer.tree,
            source,
            layer.local_facts.as_deref(),
            Some(vis_start as usize..vis_end as usize),
        );
        for (order, hs) in highlights.iter().enumerate() {
            let s = u64::from(hs.start_byte).max(vis_start);
            let e = u64::from(hs.end_byte).min(vis_end);
            if e <= s {
                continue;
            }
            let Some(name) = names.get(hs.capture_index as usize) else {
                continue;
            };
            let style = theme.lookup(name);
            if style == Style::default() {
                continue;
            }
            styled.push(StyledLayerSpan {
                start: s,
                end: e,
                style,
                priority: layer_span_priority(layer_idx, layer.depth, order),
            });
        }
    }
    flatten_layer_spans(&styled)
}

type LayerSpanPriority = (u32, u32);

/// Build the total ordering shared by the wire flattener and its sibling
/// precedence regression test. Keeping the layer index and depth as separate
/// inputs makes the former `(depth, capture_order)` bug directly falsifiable:
/// two siblings tie on depth but must differ on layer index.
fn layer_span_priority(layer_index: usize, _depth: u16, capture_order: usize) -> LayerSpanPriority {
    (layer_index as u32, capture_order as u32)
}

/// One styled span from a single injection layer, tagged with a priority
/// used to resolve overlaps: `(layer_index, capture_order)`, higher wins.
/// `layer_index` is the position in `bundle.layers` — depth-ascending, so a
/// deeper layer AND a later same-depth sibling both sort after (and thus
/// override) an earlier one, exactly matching the grid's shallow-to-deep,
/// layer-by-layer paint order. `capture_order` is the index in the layer's
/// wider-first list, so within a layer a narrower capture overrides a wider
/// one. The pair is unique per span, so the fold order is total (no ties).
struct StyledLayerSpan {
    start: u64,
    end: u64,
    style: Style,
    priority: LayerSpanPriority,
}

/// Flatten possibly-overlapping per-layer styled spans into **disjoint**
/// `StyleSpan`s whose per-byte style is the priority-ordered fold of every
/// covering span (framing Q#IJ6). Emitting disjoint spans makes the result
/// robust to the GPU wire re-sorting spans by start (`replace_style_spans`
/// / `merge_style_spans`), which would otherwise destroy producer order.
/// A boundary sweep over the (viewport-bounded) span endpoints; adjacent
/// equal-style runs are merged for wire economy.
fn flatten_layer_spans(styled: &[StyledLayerSpan]) -> Vec<StyleSpan> {
    if styled.is_empty() {
        return Vec::new();
    }
    // Boundary sweep. Every unique span endpoint is a boundary; between
    // consecutive boundaries the covering set is constant. An ordered
    // active-set (activate on start, expire on end) keeps each interval's
    // fold O(active) rather than O(all spans), so the whole pass is
    // O(spans·log spans + Σ active) — linear in practice (active is bounded
    // by overlap depth, not the total span count). This matters because the
    // file-style summary runs this over the *entire* buffer, not just the
    // viewport.
    let mut bounds: Vec<u64> = Vec::with_capacity(styled.len() * 2);
    for sp in styled {
        bounds.push(sp.start);
        bounds.push(sp.end);
    }
    bounds.sort_unstable();
    bounds.dedup();

    // Span indices ordered by start; activated as the sweep reaches them.
    let mut by_start: Vec<usize> = (0..styled.len()).collect();
    by_start.sort_by_key(|&i| styled[i].start);
    let mut next = 0usize;

    // Active covering spans, kept in ascending-priority order so the fold
    // is a single in-order pass.
    let mut active: Vec<usize> = Vec::new();

    let mut out: Vec<StyleSpan> = Vec::new();
    for win in bounds.windows(2) {
        let (a, b) = (win[0], win[1]);
        // Activate spans starting at or before `a` (each activates once).
        while next < by_start.len() && styled[by_start[next]].start <= a {
            let idx = by_start[next];
            let pos = active.partition_point(|&j| styled[j].priority < styled[idx].priority);
            active.insert(pos, idx);
            next += 1;
        }
        // Expire spans that ended at or before `a` (ranges are half-open).
        active.retain(|&j| styled[j].end > a);
        if active.is_empty() {
            continue;
        }
        // Fold the active set in ascending priority: a deeper/narrower span
        // overrides, an attribute-only span still composes (matches the
        // semantic-client `effective_style_at` contract).
        let mut style = Style::default();
        for &j in &active {
            style = crate::overlay::merge_styles(style, styled[j].style);
        }
        if style == Style::default() {
            continue;
        }
        if let Some(last) = out.last_mut()
            && last.range.end == a
            && last.style == style
        {
            last.range.end = b;
            continue;
        }
        out.push(StyleSpan {
            range: ByteRange { start: a, end: b },
            style,
        });
    }
    out
}

/// Policy-A fallback for languages with no bundled tree-sitter
/// grammar (C/C++, …): project the LSP semantic-token store into the
/// same `StyleSpan` shape the tree-sitter path emits, so the existing
/// M11.4 diff pipeline (`render_frame`) consumes it unchanged and the
/// frontend never learns which producer fed it. The instance stays
/// the single styling authority.
///
/// `SemanticToken` `start`/`length` are LSP encoding units (UTF-16
/// for clangd's default) and — unlike inlay hints — are *not*
/// byte-rewritten upstream, so this converts them per line via the
/// owning server's negotiated encoding
/// ([`crate::lsp::LspManager::semantic_style_context`]). Tokens are
/// single-line by the LSP grammar, so per-line conversion is exact.
fn lsp_scoped_style_spans(state: &EditorState, vp: &DeclaredViewport) -> Vec<StyleSpan> {
    let core = state.core.borrow();
    let Some(uri) = buffer_file_uri(&core, vp.buffer_id) else {
        return Vec::new();
    };

    // Styling context (encoding + legend) and the token set resolve
    // the *same* server for `uri` (both via `for_uri`'s lowest-id
    // rule), so they describe one coherent source.
    let mgr = state.lsp_manager.borrow();
    let Some(ctx) = mgr.semantic_style_context(&uri) else {
        return Vec::new();
    };
    let tokens = {
        let store = mgr.semantic_token_store();
        let guard = store.lock().expect("semantic-token store mutex poisoned");
        if guard.is_stale(&uri) {
            return Vec::new();
        }
        match guard.for_uri(&uri) {
            Some((_, resp)) => resp.tokens.clone(),
            None => return Vec::new(),
        }
    };
    drop(mgr);

    let registry = core.registry.clone();
    let reg = registry.borrow();
    let Ok(buf) = reg.get(vp.buffer_id) else {
        return Vec::new();
    };
    let source = buffer_source_bytes(buf);
    let source_len = source.len() as u64;
    let vis_start = vp.visible.start.min(source_len);
    let vis_end = vp.visible.end.min(source_len);
    if vis_end <= vis_start {
        return Vec::new();
    }
    let line_starts = line_start_offsets(&source);

    let theme = state
        .syntax_registry
        .theme()
        .lock()
        .expect("theme mutex poisoned")
        .clone();

    let mut out = Vec::new();
    for t in &tokens {
        let li = t.line as usize;
        let Some(&ls) = line_starts.get(li) else {
            continue; // Token line past EOF (stale response) — skip.
        };
        let le = line_starts
            .get(li + 1)
            .map_or(source_len, |&n| n.saturating_sub(1));
        let Ok(line_text) = std::str::from_utf8(&source[ls as usize..le as usize]) else {
            continue; // Non-UTF-8 line — cannot do encoded conversion.
        };
        let start_b = ls + crate::lsp::char_to_byte(line_text, t.start, ctx.encoding) as u64;
        let end_char = t.start.saturating_add(t.length);
        let end_b = ls + crate::lsp::char_to_byte(line_text, end_char, ctx.encoding) as u64;
        let s = start_b.max(vis_start);
        let e = end_b.min(vis_end);
        if e <= s {
            continue; // Empty, or no overlap with the viewport.
        }
        let Some(name) = ctx
            .legend
            .as_ref()
            .and_then(|lg| lg.type_name(t.token_type))
        else {
            continue; // No legend / unknown type ⇒ cannot name a style.
        };
        let style = theme.lookup(name);
        if style == Style::default() {
            continue; // Nothing to render — skip the wire byte (parity
            // with the tree-sitter path's default-style drop).
        }
        out.push(StyleSpan {
            range: ByteRange { start: s, end: e },
            style,
        });
    }
    out
}

/// Compute the per-line dominant style summary for the whole buffer:
/// one [`Style`] per source line, in line order. The "dominant" style
/// for a line is the one covering the most bytes among the styled
/// runs (`scoped_style_spans` for the full buffer); a line with no
/// styled runs takes [`Style::default`]. Reuses [`scoped_style_spans`]
/// so the policy-A authority choice (tree-sitter for grammar-backed
/// languages, LSP semantic tokens otherwise) is inherited automatically.
///
/// `O(spans × lines)` in the worst case; the caller short-circuits on
/// unchanged CRDT generation so this only runs on first sight of a
/// buffer or after an edit, not per frame.
fn scoped_file_summary(
    state: &EditorState,
    buffer_id: BufferId,
    resolve_faces: bool,
) -> Vec<Style> {
    let source = {
        let core = state.core.borrow();
        let registry = core.registry.clone();
        let reg = registry.borrow();
        match reg.get(buffer_id) {
            Ok(buf) => buffer_source_bytes(buf),
            Err(_) => return Vec::new(),
        }
    };
    let source_len = source.len() as u64;
    let line_starts = line_start_offsets(&source);
    let line_count = line_starts.len();
    let mut out = vec![Style::default(); line_count];

    // Reuse the existing producer with a whole-buffer "viewport". The
    // clip is then a no-op and `scoped_style_spans` yields every
    // styled run; policy A's authority pick (tree-sitter / LSP) is
    // therefore identical to the per-frame styling.
    let vp_all = DeclaredViewport {
        buffer_id,
        visible: ByteRange {
            start: 0,
            end: source_len,
        },
        frontend_generation: 0,
    };
    let spans = scoped_style_spans(state, &vp_all);
    if spans.is_empty() {
        // No styled runs — but diagnostic marks are independent of
        // syntax styling (a plain-text buffer can still have lints).
        overlay_diagnostic_marks(state, buffer_id, &mut out, resolve_faces);
        return out;
    }

    // Per-line dominant style: tally bytes per style and pick the
    // winner. `Style` doesn't implement `Hash`, so a small linear-scan
    // tally is fine — the number of distinct styles per line is
    // bounded by the theme's vocabulary (single digits in practice).
    for (li, line_dominant) in out.iter_mut().enumerate() {
        let line_start = line_starts[li];
        // Same trailing-newline semantics as `line_col_to_byte`: the
        // line's byte range excludes the `\n` (`next - 1`), and the
        // trailing line (no next entry) runs to EOF.
        let line_end = line_starts
            .get(li + 1)
            .map_or(source_len, |&next| next.saturating_sub(1));
        let mut tally: Vec<(Style, u64)> = Vec::new();
        for sp in &spans {
            let lo = sp.range.start.max(line_start);
            let hi = sp.range.end.min(line_end);
            if hi > lo {
                let bytes = hi - lo;
                if let Some(entry) = tally.iter_mut().find(|(s, _)| *s == sp.style) {
                    entry.1 += bytes;
                } else {
                    tally.push((sp.style, bytes));
                }
            }
        }
        if let Some(winner) = tally.into_iter().max_by_key(|(_, c)| *c) {
            *line_dominant = winner.0;
        }
    }
    overlay_diagnostic_marks(state, buffer_id, &mut out, resolve_faces);
    out
}

/// Fold diagnostics into the file summary: each line a diagnostic
/// touches gets the most severe severity's canonical color in
/// `underline_color` (protocol v6) — the minimap's line marks, the
/// GPU's equivalent of the TUI's column-0 gutter signs (T M4.6).
/// Skipped while the URI's store entry is stale: the positions
/// describe pre-edit text, same discipline as the decorations
/// producer (the marks return on republish, which bumps the diag
/// epoch and recomputes this summary).
fn overlay_diagnostic_marks(
    state: &EditorState,
    buffer_id: BufferId,
    lines: &mut [Style],
    resolve_faces: bool,
) {
    let uri = {
        let core = state.core.borrow();
        let Some(uri) = buffer_file_uri(&core, buffer_id) else {
            return;
        };
        uri
    };
    let store = state.lsp_manager.borrow().diag_store();
    let guard = store.lock().expect("diag store mutex poisoned");
    if guard.is_stale(&uri) {
        return;
    }
    // Most severe per line wins; LSP numbering makes that the
    // minimum severity value.
    let mut best: Vec<Option<crate::diag::DiagnosticSeverity>> = vec![None; lines.len()];
    for d in guard.for_uri(&uri) {
        for li in d.start_line..=d.end_line {
            let Some(slot) = best.get_mut(li as usize) else {
                break;
            };
            *slot = Some(slot.map_or(d.severity, |s| s.min(d.severity)));
        }
    }
    // Themes Q#TH5: the mark color is the RESOLVED severity color —
    // `ui.diag.*` faces reach the minimap through this summary. The
    // diag `Default`-fg policy guarantees a diagnosed line never
    // writes `Default` here, which the GPU reads as "no mark".
    // `resolve_faces` is false for a peer below v16 (PR #120 round 1
    // finding 3): this summary is an ungated pre-v16 channel, and a
    // v15 frontend must not get face-derived marks on one surface
    // while every other severity surface stays unthemed.
    let theme = resolve_faces.then(|| {
        let handle = state.syntax_registry.theme();
        let t = handle.lock().expect("theme mutex poisoned");
        t.clone()
    });
    for (line, severity) in lines.iter_mut().zip(best) {
        if let Some(s) = severity {
            line.underline_color = crate::diag::severity_color(theme.as_ref(), s);
        }
    }
}

/// The buffer's CRDT version projected to a monotonic scalar — the
/// `generation` anchor for the semantic frame. `0` when the buffer is
/// absent or not CRDT-backed (a `semantic_render` session always
/// negotiates `crdt_replica`, so in practice the buffer is CRDT-backed
/// before any semantic frame is produced; the fallback keeps this
/// total).
#[cfg(feature = "crdt")]
fn buffer_generation(state: &EditorState, buffer_id: BufferId) -> u64 {
    let core = state.core.borrow();
    let registry = core.registry.clone();
    let reg = registry.borrow();
    reg.get(buffer_id)
        .ok()
        .and_then(crate::buffer::Buffer::crdt_state)
        .map_or(0, crate::crdt::CrdtState::version_scalar)
}

/// Non-CRDT builds cannot host a semantic session (the negotiation
/// dependency rule requires `crdt_replica`, gated on the `crdt`
/// feature), so this is never reached with a live viewport; it exists
/// only to keep `render_frame` total across feature flavors.
#[cfg(not(feature = "crdt"))]
#[allow(clippy::missing_const_for_fn)]
fn buffer_generation(_state: &EditorState, _buffer_id: BufferId) -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellSize;
    use crate::editor::EditorState;
    use crate::instance_render::RenderState;
    use crate::protocol::FrontendId;

    fn empty_state() -> EditorState {
        EditorState::new()
    }

    fn local() -> SemanticRenderState {
        // FrontendId::LOCAL always has a registered FrontendView
        // (EditorCore invariant), so `active_window_for(LOCAL)` — the
        // selection projection's lookup — resolves in a fresh editor.
        SemanticRenderState::new(FrontendId::LOCAL)
    }

    fn active_buffer(state: &EditorState) -> BufferId {
        state.core.borrow().active_window().buffer_id
    }

    #[test]
    fn fixed_ui_face_inventory_is_strictly_sorted() {
        assert!(
            UI_FACES.windows(2).all(|pair| pair[0] < pair[1]),
            "theme_facts_msg uses binary_search; duplicates or unsorted insertions misclassify faces"
        );
    }

    #[test]
    fn line_numbers_emitted_on_toggle_then_suppressed() {
        // UX gutter (protocol v13): the daemon ships the per-window gutter
        // mode. Off is the default → no message; toggling on emits
        // `LineNumbers { enabled: true }`; an unchanged next frame suppresses.
        let state = empty_state();
        let mut s = local();
        let buffer_id = active_buffer(&state);
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);

        // Default (gutter off): no LineNumbers on the first frame.
        let first = s.render_frame(&state);
        assert!(
            !first
                .iter()
                .any(|m| matches!(m, InstanceMessage::LineNumbers { .. })),
            "off gutter must not emit LineNumbers"
        );

        // Toggle the active window on → next frame emits the mode.
        state.core.borrow_mut().active_window_mut().line_numbers =
            crate::window::LineNumberMode::Absolute;
        let on = s.render_frame(&state);
        assert!(
            on.iter().any(|m| matches!(
                m,
                InstanceMessage::LineNumbers {
                    mode: crate::window::LineNumberMode::Absolute,
                    ..
                }
            )),
            "toggling the gutter on must emit LineNumbers with the mode"
        );

        // No further change → suppressed.
        let again = s.render_frame(&state);
        assert!(
            !again
                .iter()
                .any(|m| matches!(m, InstanceMessage::LineNumbers { .. })),
            "an unchanged gutter mode must not re-emit"
        );
    }

    /// Pull the `ThemeFacts` table out of a frame, if any.
    fn theme_facts_of(msgs: &[InstanceMessage]) -> Option<Vec<crate::protocol::ThemeFace>> {
        msgs.iter().find_map(|m| match m {
            InstanceMessage::ThemeFacts { faces } => Some(faces.clone()),
            _ => None,
        })
    }

    fn statusline_of(
        msgs: &[InstanceMessage],
    ) -> Option<(BufferId, Vec<StatuslineSegment>, Vec<StatuslineSegment>)> {
        msgs.iter().find_map(|message| match message {
            InstanceMessage::StatuslineSegments {
                buffer_id,
                left,
                right,
            } => Some((*buffer_id, left.clone(), right.clone())),
            _ => None,
        })
    }

    /// Simulate a committed face mutation: what `pmacs.theme.merge`
    /// does after its transactional parse (insert + face-epoch bump).
    fn merge_face(state: &EditorState, name: &str, style: Style) {
        let theme = state.syntax_registry.theme();
        let mut th = theme.lock().expect("theme mutex poisoned");
        th.insert(name, style);
        th.face_epoch += 1;
    }

    #[test]
    fn statusline_first_empty_is_authoritative_then_silent_and_snapshot_resends() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let lua = state.lua_host.lua();
        let callback = lua
            .load("return function(_) return __baseline_text end")
            .eval()
            .expect("callback");
        state
            .statusline_registry
            .borrow_mut()
            .register(
                "baseline".into(),
                crate::statusline::StatuslineSide::Left,
                0,
                "ui.modeline".into(),
                callback,
                crate::command::SourceLocation::default(),
            )
            .expect("register");
        let mut semantic = SemanticRenderState::for_peer(FrontendId::LOCAL, 18);
        semantic.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);

        assert_eq!(
            statusline_of(&semantic.render_frame(&state)),
            Some((buffer_id, Vec::new(), Vec::new()))
        );
        assert_eq!(statusline_of(&semantic.render_frame(&state)), None);

        lua.globals()
            .set("__baseline_text", "changed")
            .expect("set");
        let changed = vec![StatuslineSegment {
            text: "changed".into(),
            face: "ui.modeline".into(),
        }];
        assert_eq!(
            statusline_of(&semantic.render_frame(&state)),
            Some((buffer_id, changed.clone(), Vec::new()))
        );
        assert_eq!(
            statusline_of(&semantic.render_frame(&state)),
            None,
            "byte-identical callback output is silent"
        );

        semantic.on_buffer_snapshot_sent(buffer_id);
        assert_eq!(
            statusline_of(&semantic.render_frame(&state)),
            Some((buffer_id, changed, Vec::new())),
            "snapshot reset makes an unchanged revisit authoritative again"
        );
    }

    #[test]
    fn v17_skips_callbacks_and_dynamic_faces_while_v18_orders_theme_first() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        merge_face(
            &state,
            "ui.modeline.project",
            Style {
                fg: crate::cell::Color::Indexed(6),
                bg: crate::cell::Color::Indexed(2),
                bold: true,
                reverse: true,
                ..Style::default()
            },
        );
        let lua = state.lua_host.lua();
        lua.globals().set("__statusline_calls", 0).expect("set");
        let callback = lua
            .load(
                "return function(_) \
                 __statusline_calls = __statusline_calls + 1; return 'project' end",
            )
            .eval()
            .expect("callback");
        let provider_id = state
            .statusline_registry
            .borrow_mut()
            .register(
                "project".into(),
                crate::statusline::StatuslineSide::Left,
                0,
                "ui.modeline.project".into(),
                callback,
                crate::command::SourceLocation::default(),
            )
            .expect("register");

        let mut v17 = SemanticRenderState::for_peer(FrontendId::LOCAL, 17);
        v17.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let old_frame = v17.render_frame(&state);
        assert_eq!(lua.globals().get::<i64>("__statusline_calls").unwrap(), 0);
        assert_eq!(statusline_of(&old_frame), None);
        assert!(
            theme_facts_of(&old_frame)
                .expect("v17 still receives fixed ThemeFacts")
                .iter()
                .all(|face| face.name != "ui.modeline.project")
        );

        let mut v18 = SemanticRenderState::for_peer(FrontendId::LOCAL, 18);
        v18.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let frame = v18.render_frame(&state);
        assert_eq!(lua.globals().get::<i64>("__statusline_calls").unwrap(), 1);
        let theme_index = frame
            .iter()
            .position(|message| matches!(message, InstanceMessage::ThemeFacts { .. }))
            .expect("dynamic ThemeFacts");
        let segments_index = frame
            .iter()
            .position(|message| matches!(message, InstanceMessage::StatuslineSegments { .. }))
            .expect("segments");
        assert!(theme_index < segments_index);
        let project_face = theme_facts_of(&frame)
            .unwrap()
            .into_iter()
            .find(|face| face.name == "ui.modeline.project")
            .expect("exact dynamic face");
        assert_eq!(
            project_face.style,
            Style {
                fg: crate::cell::Color::Indexed(6),
                ..Style::default()
            }
        );

        let face_epoch = v18.last_statusline_face_set_epoch;
        assert!(
            state
                .statusline_registry
                .borrow_mut()
                .set_priority(provider_id, 10)
        );
        assert_eq!(theme_facts_of(&v18.render_frame(&state)), None);
        assert_eq!(v18.last_statusline_face_set_epoch, face_epoch);

        assert!(
            state
                .statusline_registry
                .borrow_mut()
                .set_enabled(provider_id, false)
        );
        let disabled = v18.render_frame(&state);
        assert!(
            theme_facts_of(&disabled)
                .expect("face-set shrink emits")
                .iter()
                .all(|face| face.name != "ui.modeline.project")
        );
        assert_eq!(
            statusline_of(&disabled),
            Some((buffer_id, Vec::new(), Vec::new()))
        );
    }

    #[test]
    fn invalidated_statusline_publishes_one_empty_baseline() {
        let buffer_id = BufferId::from_raw(77);
        let mut semantic = local();
        let mut initial = Vec::new();
        semantic.emit_statusline_payload(
            buffer_id,
            vec![StatuslineSegment {
                text: "old".into(),
                face: "ui.modeline".into(),
            }],
            Vec::new(),
            &mut initial,
        );
        assert_eq!(initial.len(), 1);

        let mut stale = Vec::new();
        semantic.emit_statusline_segments(
            StatuslineEvaluation {
                outcome: StatuslineEvaluationOutcome::NoMessage(
                    crate::statusline::StatuslineNoMessageReason::DeclaredBufferMismatch,
                ),
                new_failures: Vec::new(),
            },
            None,
            &mut stale,
        );
        assert!(stale.is_empty(), "phase-1 stale evaluation emits nothing");
        assert_eq!(
            semantic.last_statusline[&buffer_id].0[0].text, "old",
            "stale evaluation retains the prior baseline until snapshot reset"
        );

        // Bottom-panel A2A-2: the clear is filtered by DOCUMENT window
        // identity, so the context under test must BE the document
        // window — passing `None` here would assert nothing.
        let document_window = crate::window::WindowId::next();
        let invalidated = || StatuslineEvaluation {
            outcome: StatuslineEvaluationOutcome::Invalidated {
                authoritative_empty: vec![crate::statusline::StatuslineContext {
                    frontend_id: FrontendId::LOCAL,
                    window_id: document_window,
                    buffer_id,
                    active: true,
                }],
            },
            new_failures: Vec::new(),
        };
        let mut replacement = Vec::new();
        semantic.emit_statusline_segments(invalidated(), Some(document_window), &mut replacement);
        assert_eq!(
            statusline_of(&replacement),
            Some((buffer_id, Vec::new(), Vec::new()))
        );
        let mut unchanged = Vec::new();
        semantic.emit_statusline_segments(invalidated(), Some(document_window), &mut unchanged);
        assert!(
            unchanged.is_empty(),
            "the empty invalidation became baseline"
        );
    }

    #[test]
    fn theme_facts_authoritative_empty_then_silent_then_face_change_emits() {
        // Q#TH7: the first frame after viewport declaration ships the
        // authoritative table — EMPTY for an unthemed daemon, which
        // the Option epoch gate must not short-circuit at 0 == 0 —
        // then unchanged ticks say nothing; a face commit re-emits
        // the resolved table.
        let state = empty_state();
        let mut s = local();
        let buffer_id = active_buffer(&state);
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);

        let first = s.render_frame(&state);
        assert_eq!(
            theme_facts_of(&first),
            Some(Vec::new()),
            "an unthemed attachment still receives one authoritative empty table"
        );
        assert_eq!(
            theme_facts_of(&s.render_frame(&state)),
            None,
            "unchanged ticks emit nothing"
        );

        merge_face(
            &state,
            "ui.gutter",
            Style {
                fg: crate::cell::Color::Indexed(99),
                ..Style::default()
            },
        );
        let facts = theme_facts_of(&s.render_frame(&state)).expect("face change emits");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].name, "ui.gutter");
        assert_eq!(facts[0].style.fg, crate::cell::Color::Indexed(99));
        assert_eq!(
            theme_facts_of(&s.render_frame(&state)),
            None,
            "and suppresses again once shipped"
        );
    }

    #[test]
    fn theme_facts_resolution_is_daemon_side() {
        // Q#TH7 / acceptance 15: with only `ui.diag` set, the shipped
        // table carries the four concrete `ui.diag.*` children — the
        // walk happens here, never in a frontend.
        let state = empty_state();
        let mut s = local();
        let buffer_id = active_buffer(&state);
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let _ = s.render_frame(&state);

        merge_face(
            &state,
            "ui.diag",
            Style {
                fg: crate::cell::Color::Indexed(93),
                ..Style::default()
            },
        );
        let facts = theme_facts_of(&s.render_frame(&state)).expect("emits");
        let names: Vec<&str> = facts.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "ui.diag.error",
                "ui.diag.hint",
                "ui.diag.info",
                "ui.diag.warning"
            ],
            "only the concrete stage-1 children ship, sorted"
        );
        assert!(
            facts
                .iter()
                .all(|f| f.style.fg == crate::cell::Color::Indexed(93)),
            "each child resolved through the parent"
        );
    }

    #[test]
    fn theme_facts_identical_rebuild_advances_epoch_without_emitting() {
        // Q#TH7 / acceptance 14: an epoch bump with an unchanged
        // table (an identical re-merge) emits nothing but still
        // records the inspected epoch — the cache advances on
        // computation, or every subsequent tick would rebuild.
        let state = empty_state();
        let mut s = local();
        let buffer_id = active_buffer(&state);
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let _ = s.render_frame(&state);

        let bumped = {
            let theme = state.syntax_registry.theme();
            let mut th = theme.lock().expect("lock");
            th.face_epoch += 1; // identical re-merge: no table change
            th.face_epoch
        };
        assert_eq!(
            theme_facts_of(&s.render_frame(&state)),
            None,
            "identical rebuild is suppressed"
        );
        assert_eq!(
            s.last_face_epoch,
            Some(bumped),
            "the inspected epoch advanced despite the suppressed send"
        );
    }

    #[test]
    fn summary_cache_key_advances_on_suppressed_emission() {
        // Q#TH6 / acceptance 14: a face mutation that leaves the
        // summary unchanged (ui.modeline touches no minimap stroke)
        // recomputes once, emits nothing, and STILL advances the
        // cache key — otherwise the whole-file pass repeats per tick.
        let state = empty_state();
        let mut s = local();
        let buffer_id = active_buffer(&state);
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let first = s.render_frame(&state);
        assert!(
            first
                .iter()
                .any(|m| matches!(m, InstanceMessage::FileStyleSummary { .. })),
            "first frame ships the summary"
        );

        merge_face(&state, "ui.modeline", Style::default());
        let (_, _, _, face_epoch) = {
            let theme = state.syntax_registry.theme();
            let th = theme.lock().expect("lock");
            (0, 0, th.syntax_epoch, th.face_epoch)
        };
        let next = s.render_frame(&state);
        assert!(
            !next
                .iter()
                .any(|m| matches!(m, InstanceMessage::FileStyleSummary { .. })),
            "an unchanged summary is suppressed"
        );
        assert_eq!(
            s.last_summary
                .get(&buffer_id)
                .expect("cache entry exists")
                .key
                .3,
            face_epoch,
            "the cache key advanced despite the suppressed send"
        );
    }

    /// Pull the `FontFacts` payload out of a frame, if any.
    fn font_facts_of(msgs: &[InstanceMessage]) -> Option<(Option<String>, Option<u32>)> {
        msgs.iter().find_map(|m| match m {
            InstanceMessage::FontFacts {
                family,
                size_centi_px,
            } => Some((family.clone(), *size_centi_px)),
            _ => None,
        })
    }

    /// Simulate a committed `pmacs.gpu.set_font`: what the Lua setter
    /// does after its parse/validate/quantize (write + epoch bump).
    fn set_font(state: &EditorState, family: Option<&str>, size_centi_px: Option<u32>) {
        let mut pref = state.font_pref.lock().expect("font pref");
        pref.family = family.map(str::to_owned);
        pref.size_centi_px = size_centi_px;
        pref.epoch += 1;
    }

    #[test]
    fn font_facts_authoritative_default_then_silent_then_set_emits() {
        // Q#F5 / acceptance 2-3: the first frame ships the
        // authoritative all-default preference — the Option epoch
        // gate must not short-circuit at 0 == 0 — then unchanged
        // ticks say nothing; a set_font re-ships; an identical
        // re-set advances the inspected epoch without emitting.
        let state = empty_state();
        let mut s = local();
        let buffer_id = active_buffer(&state);
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);

        let first = s.render_frame(&state);
        assert_eq!(
            font_facts_of(&first),
            Some((None, None)),
            "an all-default daemon still ships one authoritative preference"
        );
        assert_eq!(
            font_facts_of(&s.render_frame(&state)),
            None,
            "unchanged ticks emit nothing"
        );

        set_font(&state, Some("Iosevka"), Some(1800));
        assert_eq!(
            font_facts_of(&s.render_frame(&state)),
            Some((Some("Iosevka".into()), Some(1800))),
            "a live set_font re-ships on the next frame"
        );
        assert_eq!(
            font_facts_of(&s.render_frame(&state)),
            None,
            "and suppresses again once shipped"
        );

        // Identical re-set: epoch bumps, payload unchanged — nothing
        // emits, but the inspected epoch advances (cache advances on
        // computation, or every later tick would rebuild).
        set_font(&state, Some("Iosevka"), Some(1800));
        let bumped = state.font_pref.lock().expect("font pref").epoch;
        assert_eq!(
            font_facts_of(&s.render_frame(&state)),
            None,
            "identical re-set is suppressed"
        );
        assert_eq!(
            s.last_font_epoch,
            Some(bumped),
            "the inspected epoch advanced despite the suppressed send"
        );
    }

    #[test]
    fn font_facts_never_produced_for_a_v16_peer() {
        // Q#F4 / acceptance 5 (producer half; the daemon skip arm is
        // the belt-and-braces filter).
        let state = empty_state();
        set_font(&state, None, Some(2000));
        let buffer_id = active_buffer(&state);
        let mut v16 = SemanticRenderState::for_peer(FrontendId::LOCAL, 16);
        v16.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let frame = v16.render_frame(&state);
        assert_eq!(font_facts_of(&frame), None, "v16 peers get no FontFacts");
        assert!(
            frame
                .iter()
                .any(|m| matches!(m, InstanceMessage::ThemeFacts { .. })),
            "the same peer still receives v16 facts"
        );
        let mut v17 = SemanticRenderState::for_peer(FrontendId::LOCAL, 17);
        v17.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        assert_eq!(
            font_facts_of(&v17.render_frame(&state)),
            Some((None, Some(2000))),
            "a v17 peer receives the current preference"
        );
    }

    #[test]
    fn snapshot_reset_drops_one_buffers_baselines_and_keeps_the_rest() {
        // PR #120 round 2 finding 1 — the reset contract's scope: a
        // `BufferSnapshot` for buffer A kills A's buffer-scoped
        // emission baselines (here: summary and status, the two the
        // A → B → A round trip visibly strands) while OTHER buffers'
        // baselines and the bufferless ThemeFacts pair survive.
        let state = empty_state();
        let mut s = local();
        let a = active_buffer(&state);
        let b = {
            let core = state.core.borrow();
            let mut reg = core.registry.borrow_mut();
            reg.create("other")
        };
        s.set_viewport(a, ByteRange { start: 0, end: 64 }, 0);
        let first = s.render_frame(&state);
        assert!(
            first
                .iter()
                .any(|m| matches!(m, InstanceMessage::FileStyleSummary { .. })),
            "first frame ships A's summary"
        );
        s.set_viewport(b, ByteRange { start: 0, end: 64 }, 0);
        let _ = s.render_frame(&state);
        assert!(s.last_summary.contains_key(&a));
        assert!(s.last_summary.contains_key(&b));
        assert!(s.last_status.contains_key(&a));
        let facts_baseline = s.last_theme_faces.clone();
        assert!(facts_baseline.is_some(), "first frame shipped ThemeFacts");

        s.on_buffer_snapshot_sent(a);

        assert!(
            !s.last_summary.contains_key(&a) && !s.last_status.contains_key(&a),
            "A's baselines die with A's snapshot"
        );
        assert!(
            s.last_summary.contains_key(&b),
            "B's baselines survive A's snapshot"
        );
        assert_eq!(
            s.last_theme_faces, facts_baseline,
            "ThemeFacts is bufferless — the face table survives snapshots"
        );
        assert_eq!(
            s.last_font_facts,
            Some((None, None)),
            "FontFacts is bufferless too — the preference baseline survives"
        );

        // And the behavioral consequence: revisiting A at the SAME
        // generation re-ships the summary the frontend just dropped.
        s.set_viewport(a, ByteRange { start: 0, end: 64 }, 0);
        let back = s.render_frame(&state);
        assert!(
            back.iter()
                .any(|m| matches!(m, InstanceMessage::FileStyleSummary { .. })),
            "the unchanged-generation revisit re-ships A's summary"
        );
    }

    #[test]
    fn style_gate_differs_when_syntax_epoch_bumps() {
        // Q#TH6: the gate is a pure function of the theme too — a
        // syntax-epoch bump must force the span recompute (the
        // pre-existing mid-session staleness bug).
        let g1 = StyleGate {
            bundle: None,
            generation: 1,
            visible: ByteRange { start: 0, end: 10 },
            syntax_epoch: 0,
        };
        let mut g2 = g1.clone();
        assert!(g1.matches(&g2), "identical gates match");
        g2.syntax_epoch = 1;
        assert!(!g1.matches(&g2), "a theme mutation breaks the match");
    }

    /// All `InstanceMessage` variants the semantic projection may
    /// emit are `StyleSpans`, `Decorations`, `InlineAdornments`,
    /// `FoldState` (Q#FD8), `FileStyleSummary`, `StatusFacts` (Q#S1),
    /// `SearchPrompt` (Q#SR5), `LineNumbers`, `ThemeFacts` (Q#TH7),
    /// `FontFacts` (Q#F5), or `StatuslineSegments` (Q#SL7) — never
    /// `CellDelta`, grid `Cursor`, or the still-unwired `BlockAdornments`
    /// family.
    fn assert_semantic_only(msgs: &[InstanceMessage]) {
        for m in msgs {
            assert!(
                matches!(
                    m,
                    InstanceMessage::StyleSpans { .. }
                        | InstanceMessage::Decorations { .. }
                        | InstanceMessage::InlineAdornments { .. }
                        | InstanceMessage::FoldState { .. }
                        | InstanceMessage::FileStyleSummary { .. }
                        | InstanceMessage::StatusFacts { .. }
                        | InstanceMessage::SearchPrompt { .. }
                        | InstanceMessage::LineNumbers { .. }
                        | InstanceMessage::ThemeFacts { .. }
                        | InstanceMessage::FontFacts { .. }
                        | InstanceMessage::LineWrapFacts { .. }
                        | InstanceMessage::StatuslineSegments { .. }
                ),
                "semantic projection emitted an unexpected variant: {m:?}"
            );
        }
    }

    /// Find the `Decorations` message and flatten its segments into
    /// `(full, all decorations across segments)`.
    fn decorations_of(msgs: &[InstanceMessage]) -> Option<(bool, Vec<Decoration>)> {
        msgs.iter().find_map(|m| match m {
            InstanceMessage::Decorations { full, segments, .. } => Some((
                *full,
                segments
                    .iter()
                    .flat_map(|s| s.decorations.clone())
                    .collect(),
            )),
            _ => None,
        })
    }

    /// Find the `StyleSpans` message: `(full, segment ranges)`.
    fn style_segments(msgs: &[InstanceMessage]) -> Option<(bool, Vec<ByteRange>)> {
        msgs.iter().find_map(|m| match m {
            InstanceMessage::StyleSpans { full, segments, .. } => {
                Some((*full, segments.iter().map(|s| s.range).collect()))
            }
            _ => None,
        })
    }

    fn set_selection(state: &EditorState, anchor: u64, cursor: u64) {
        let mut core = state.core.borrow_mut();
        let win = core
            .active_window_mut_for(FrontendId::LOCAL)
            .expect("LOCAL always has a window");
        win.selection = Some(crate::window::Selection { anchor });
        win.cursor = cursor;
    }

    fn seed_diagnostic(state: &EditorState, buffer_id: BufferId) {
        let mut core = state.core.borrow_mut();
        core.registry
            .clone()
            .borrow_mut()
            .get_mut(buffer_id)
            .expect("active buffer")
            .apply_edit(crate::buffer::EditOp::Insert {
                pos: 0,
                bytes: b"abc\nde",
            })
            .expect("seed buffer text");
        core.set_buffer_path(buffer_id, Some(std::path::PathBuf::from("/tmp/m114.rs")));
        drop(core);
        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/m114.rs"));
        let store = state.lsp_manager.borrow().diag_store();
        store.lock().expect("diag store").set(
            &uri,
            vec![crate::diag::Diagnostic {
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 2,
                severity: crate::diag::DiagnosticSeverity::Warning,
                message: "x".into(),
                source: None,
                code: None,
            }],
        );
    }

    #[test]
    fn search_matches_emit_as_decorations_with_active_distinguished() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        // "lo lo lo" — three "lo" matches at 0..2, 3..5, 6..8.
        {
            let core = state.core.borrow();
            core.registry
                .clone()
                .borrow_mut()
                .get_mut(bid)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"lo lo lo",
                })
                .expect("seed");
        }
        {
            let store = state.core.borrow().search_store.clone();
            let matches = crate::search::find_all(b"lo lo lo", "lo");
            store.lock().expect("search store").set(bid, "lo", matches);
        }
        s.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);

        let (_full, decos) =
            decorations_of(&s.render_frame(&state)).expect("search frame ships decorations");
        let search: Vec<_> = decos
            .iter()
            .filter(|d| {
                matches!(
                    d.kind,
                    DecorationKind::SearchMatch | DecorationKind::SearchMatchActive
                )
            })
            .collect();
        assert_eq!(search.len(), 3, "three matches highlighted; got {decos:?}");
        let active: Vec<_> = decos
            .iter()
            .filter(|d| d.kind == DecorationKind::SearchMatchActive)
            .collect();
        assert_eq!(active.len(), 1, "exactly one active match");
        assert_eq!(
            active[0].range,
            ByteRange { start: 0, end: 2 },
            "the first match is active by default"
        );

        // Marking the store stale suppresses search emission (M11.8):
        // the next frame ships a clearing diff, never a search kind.
        state
            .core
            .borrow()
            .search_store
            .clone()
            .lock()
            .expect("search store")
            .mark_stale(bid);
        if let Some((_full, decos)) = decorations_of(&s.render_frame(&state)) {
            assert!(
                decos.iter().all(|d| !matches!(
                    d.kind,
                    DecorationKind::SearchMatch | DecorationKind::SearchMatchActive
                )),
                "stale search store paints no matches; got {decos:?}"
            );
        }
    }

    #[test]
    fn emits_nothing_before_viewport_declared() {
        let mut s = local();
        assert!(
            s.render_frame(&empty_state()).is_empty(),
            "nothing may be emitted before the frontend declares a viewport"
        );
    }

    #[test]
    fn first_post_viewport_frame_is_full_for_both_then_suppresses() {
        let state = empty_state();
        let mut s = local();
        let buffer_id = active_buffer(&state);
        s.set_viewport(
            buffer_id,
            ByteRange {
                start: 0,
                end: 4096,
            },
            0,
        );

        // Empty scratch: no spans, no selection, no diagnostics — but
        // the first frame is a `full` resync for both diffable families
        // (the frontend clears its viewport), carrying empty segments.
        // FileStyleSummary also emits on the first frame for this buffer
        // (post-M11 minimap producer, generation-keyed), as does
        // StatusFacts (Q#S1, cached-compare), the authoritative
        // ThemeFacts table (Q#TH7 — empty for an unthemed daemon), and
        // the authoritative FontFacts preference (Q#F5 — all-default), and
        // authoritative empty statusline segments (Q#SL8), and the
        // buffer's authoritative wrap mode (v22 — a semantic frontend
        // lays out locally, so it has to be told on the first frame or
        // it never learns the setting at all).
        let first = s.render_frame(&state);
        assert_eq!(
            first.len(),
            8,
            "first frame ships StyleSpans + Decorations + FileStyleSummary \
             + StatusFacts + ThemeFacts + FontFacts + StatuslineSegments \
             + LineWrapFacts"
        );
        assert_semantic_only(&first);
        let (style_full, _) = style_segments(&first).expect("StyleSpans present");
        let (deco_full, decos) = decorations_of(&first).expect("Decorations present");
        assert!(style_full, "first styling frame must be full");
        assert!(deco_full, "first decorations frame must be full");
        assert!(decos.is_empty(), "empty scratch has no decorations");

        // Nothing changed → both families suppressed.
        assert!(
            s.render_frame(&state).is_empty(),
            "an unchanged frame must be fully suppressed"
        );
    }

    #[test]
    fn selection_projects_as_a_decoration_clipped_to_viewport() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        // region (2,5) on LOCAL's window; region() compares offsets
        // only, so the empty scratch buffer is fine here.
        set_selection(&state, 2, 5);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 3, end: 64 }, 0);

        let msgs = s.render_frame(&state);
        assert_semantic_only(&msgs);
        let (full, decos) = decorations_of(&msgs).expect("a Decorations message");
        assert!(full, "first frame is a full resync");
        assert_eq!(decos.len(), 1, "exactly the selection decoration");
        assert_eq!(decos[0].kind, DecorationKind::Selection);
        // region (2,5) clipped to viewport [3,64) → [3,5).
        assert_eq!(decos[0].range, ByteRange { start: 3, end: 5 });
    }

    #[test]
    fn semantic_projection_does_not_emit_current_line_decoration() {
        // CurrentLine is a frontend-local visual for semantic sessions;
        // the daemon should not copy the whole buffer to derive it.
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        seed_diagnostic(&state, buffer_id);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);

        let (_full, decos) =
            decorations_of(&s.render_frame(&state)).expect("a Decorations message");
        assert!(
            decos.iter().all(|d| d.kind != DecorationKind::CurrentLine),
            "semantic projection must not emit CurrentLine; got {decos:?}"
        );
    }

    #[test]
    fn semantic_current_line_absence_does_not_depend_on_active_buffer() {
        let state = empty_state();
        let scratch_id = active_buffer(&state);
        let file_id = {
            let core = state.core.borrow();
            core.registry
                .borrow_mut()
                .create_from_bytes("secondary".to_owned(), b"abc\nde")
        };
        assert_ne!(scratch_id, file_id);

        let mut s = local();
        // Project the *non-active* file buffer.
        s.set_viewport(file_id, ByteRange { start: 0, end: 64 }, 0);
        let (_full, decos) =
            decorations_of(&s.render_frame(&state)).expect("a Decorations message");
        assert!(
            decos.iter().all(|d| d.kind != DecorationKind::CurrentLine),
            "semantic projection must not emit CurrentLine for any viewport; got {decos:?}"
        );
    }

    #[test]
    fn cursor_motion_does_not_re_emit_decorations() {
        // Cursor-only movement should not ship Decorations. Semantic
        // frontends receive CursorByte separately and derive local
        // cursor visuals without daemon decoration churn.
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        {
            let core = state.core.borrow();
            core.registry
                .borrow_mut()
                .get_mut(buffer_id)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"abcdefghij\nklmno",
                })
                .expect("seed");
        }
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let _first = s.render_frame(&state); // initial full
        assert!(
            s.render_frame(&state).is_empty(),
            "steady state must be silent"
        );

        // Move cursor from byte 0 to byte 5 (same line).
        {
            let mut core = state.core.borrow_mut();
            core.active_window_mut().cursor = 5;
        }
        assert!(
            s.render_frame(&state).is_empty(),
            "same-line cursor motion must not re-emit Decorations"
        );

        // Cross a `\n` (byte 10). Still no Decorations frame.
        {
            let mut core = state.core.borrow_mut();
            core.active_window_mut().cursor = 12;
        }
        assert!(
            s.render_frame(&state).is_empty(),
            "line-crossing cursor motion must not re-emit Decorations"
        );
    }

    #[test]
    fn diagnostics_project_with_line_col_to_byte_and_severity() {
        // "abc\nde": line 0 at byte 0, line 1 at byte 4.
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        seed_diagnostic(&state, buffer_id);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);

        let (_full, decos) =
            decorations_of(&s.render_frame(&state)).expect("a Decorations message");
        // This test pins the diagnostic projection's byte math without
        // relying on any cursor-line decoration.
        let warning = decos
            .iter()
            .find(|d| d.kind == DecorationKind::DiagnosticWarning)
            .expect("the seeded warning");
        // line 1 starts at byte 4; cols [0,2) → bytes [4,6).
        assert_eq!(warning.range, ByteRange { start: 4, end: 6 });
    }

    /// T M11.8 regression: when the diag store's entry for the URI
    /// is marked stale (an edit has been issued since the last
    /// `publishDiagnostics`), the producer must suppress diagnostic
    /// emission so the frontend doesn't paint colors at pre-edit
    /// byte positions over post-edit text. Closes the bet-#1
    /// surface that session-5 validation exposed.
    #[test]
    fn diagnostics_suppressed_while_diag_store_stale() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        seed_diagnostic(&state, buffer_id);

        // Mark the diag store stale for the seeded URI. The producer
        // should now emit zero diagnostic decorations.
        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/m114.rs"));
        state
            .lsp_manager
            .borrow()
            .diag_store()
            .lock()
            .expect("diag store")
            .mark_stale(uri.clone());

        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let (_full, decos) =
            decorations_of(&s.render_frame(&state)).expect("a Decorations message");
        assert!(
            decos.iter().all(|d| !matches!(
                d.kind,
                DecorationKind::DiagnosticError
                    | DecorationKind::DiagnosticWarning
                    | DecorationKind::DiagnosticInfo
                    | DecorationKind::DiagnosticHint
            )),
            "stale diag store ⇒ no diagnostic decorations emitted; got {decos:?}"
        );

        // Once a fresh `set` clears the stale flag, the decoration
        // re-appears on the next render frame.
        state
            .lsp_manager
            .borrow()
            .diag_store()
            .lock()
            .expect("diag store")
            .set(
                &uri,
                vec![crate::diag::Diagnostic {
                    start_line: 1,
                    start_col: 0,
                    end_line: 1,
                    end_col: 2,
                    severity: crate::diag::DiagnosticSeverity::Warning,
                    message: "x".into(),
                    source: None,
                    code: None,
                }],
            );

        let (_full, decos) =
            decorations_of(&s.render_frame(&state)).expect("a Decorations message");
        assert!(
            decos
                .iter()
                .any(|d| d.kind == DecorationKind::DiagnosticWarning),
            "fresh set ⇒ stale flag cleared ⇒ decoration re-emitted; got {decos:?}"
        );
    }

    /// Hold-while-stale (diagnostics churn fix): once diagnostics
    /// have shipped, marking the store stale (which happens per edit)
    /// must NOT ship a clearing frame — the frontend keeps its
    /// last-received set, translated through its own local edits,
    /// until the next `publishDiagnostics`. The pre-fix behavior
    /// shipped a full empty frame on the first keystroke of every
    /// burst (diagnostics blinked out, one full frontend reshape) and
    /// re-added them after the next publish (blink in, another
    /// reshape).
    #[test]
    fn diagnostics_hold_emission_while_store_stale_after_shipping() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        seed_diagnostic(&state, buffer_id);

        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let (_full, decos) =
            decorations_of(&s.render_frame(&state)).expect("baseline ships the diagnostic");
        assert!(
            decos.iter().any(|d| is_diagnostic_kind(d.kind)),
            "baseline contains the seeded diagnostic; got {decos:?}"
        );

        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/m114.rs"));
        state
            .lsp_manager
            .borrow()
            .diag_store()
            .lock()
            .expect("diag store")
            .mark_stale(uri.clone());

        assert!(
            decorations_of(&s.render_frame(&state)).is_none(),
            "stale store holds Decorations emission instead of shipping a clearing frame"
        );
        assert!(
            decorations_of(&s.render_frame(&state)).is_none(),
            "the hold is stable across frames"
        );

        // A selection change during the stale window still ships
        // (without diagnostic kinds) — the hold must not pin a dead
        // selection just to protect the diagnostics.
        set_selection(&state, 0, 2);
        let (_full, decos) = decorations_of(&s.render_frame(&state))
            .expect("selection change ships during the stale window");
        assert!(
            decos.iter().any(|d| d.kind == DecorationKind::Selection),
            "fresh selection present; got {decos:?}"
        );
        assert!(
            decos.iter().all(|d| !is_diagnostic_kind(d.kind)),
            "no stale-positioned diagnostics ride along; got {decos:?}"
        );

        // The next publishDiagnostics clears the flag. An IDENTICAL
        // publish stays silent — the carried baseline already matches
        // the frontend's cache. A *changed* diagnostic diffs through.
        state
            .lsp_manager
            .borrow()
            .diag_store()
            .lock()
            .expect("diag store")
            .set(
                &uri,
                vec![crate::diag::Diagnostic {
                    start_line: 1,
                    start_col: 1,
                    end_line: 1,
                    end_col: 2,
                    severity: crate::diag::DiagnosticSeverity::Warning,
                    message: "x".into(),
                    source: None,
                    code: None,
                }],
            );
        let (_full, decos) = decorations_of(&s.render_frame(&state))
            .expect("post-publish frame ships the moved diagnostic");
        assert!(
            decos
                .iter()
                .any(|d| d.kind == DecorationKind::DiagnosticWarning),
            "diagnostics update once the store is fresh; got {decos:?}"
        );
    }

    /// Regression: in a multi-frontend setup the editor's *active*
    /// buffer (set by `core.active_buffer_id()`, derived from the
    /// active frontend's view) can differ from the buffer a given
    /// frontend's `SemanticRenderState` is projecting. The producer
    /// used to derive the diagnostic URI from `active_buffer_path()`,
    /// which returned the wrong path (or `None`) in that case and
    /// emitted empty decorations — the session-5 manual-validation
    /// finding. Fix routes the URI lookup through `vp.buffer_id` via
    /// `buffer_file_uri`.
    #[test]
    fn decorations_use_vp_buffer_not_active_buffer() {
        let state = empty_state();
        // The default LOCAL frontend's active buffer is a scratch
        // with no file path. We seed text + a file path on a buffer
        // that is NOT the active one, then project that buffer via
        // a separate `SemanticRenderState`. Pre-fix: the producer
        // looks up `active_buffer_path()` (the scratch, no path) and
        // gets `None`, emitting empty decorations. Post-fix: it
        // resolves the URI from `vp.buffer_id`, finds the diagnostic.
        let scratch_id = active_buffer(&state);

        // Create a second buffer with a real file path + a diagnostic.
        let file_id = {
            let core = state.core.borrow();
            core.registry
                .borrow_mut()
                .create_from_bytes("secondary".to_owned(), b"abc\nde")
        };
        {
            let mut core = state.core.borrow_mut();
            core.set_buffer_path(file_id, Some(std::path::PathBuf::from("/tmp/multi-fid.rs")));
        }
        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/multi-fid.rs"));
        state
            .lsp_manager
            .borrow()
            .diag_store()
            .lock()
            .expect("diag store")
            .set(
                &uri,
                vec![crate::diag::Diagnostic {
                    start_line: 1,
                    start_col: 0,
                    end_line: 1,
                    end_col: 2,
                    severity: crate::diag::DiagnosticSeverity::Error,
                    message: "boom".into(),
                    source: None,
                    code: None,
                }],
            );

        // Sanity: active buffer is still the scratch, not `file_id`.
        assert_eq!(scratch_id, active_buffer(&state));
        assert_ne!(scratch_id, file_id);

        // Project the file buffer through the semantic renderer.
        let mut s = local();
        s.set_viewport(file_id, ByteRange { start: 0, end: 64 }, 0);

        let (_full, decos) =
            decorations_of(&s.render_frame(&state)).expect("a Decorations message");
        assert_eq!(
            decos.len(),
            1,
            "decoration must surface for the projected buffer even when it's not the editor's active buffer"
        );
        assert_eq!(decos[0].kind, DecorationKind::DiagnosticError);
        assert_eq!(decos[0].range, ByteRange { start: 4, end: 6 });
    }

    #[test]
    fn styles_and_decorations_suppress_independently() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let _ = s.render_frame(&state); // first frame: both full
        assert!(s.render_frame(&state).is_empty(), "steady state silent");

        // A selection appears → only Decorations re-emits, and as an
        // incremental (full = false) frame since the viewport region
        // did not move.
        set_selection(&state, 1, 4);
        let msgs = s.render_frame(&state);
        assert_eq!(msgs.len(), 1, "only the changed family re-emits");
        let (full, decos) = decorations_of(&msgs).expect("Decorations re-emitted");
        assert!(!full, "viewport unchanged → incremental, not full");
        assert_eq!(decos.len(), 1);
        assert_eq!(decos[0].kind, DecorationKind::Selection);
    }

    #[test]
    fn full_resync_on_viewport_region_change() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        let _ = s.render_frame(&state); // full
        assert!(s.render_frame(&state).is_empty(), "unchanged → silent");

        // Declaring a different on-screen range forces a full resync:
        // prior styling/decorations are positioned for the old window.
        s.set_viewport(
            buffer_id,
            ByteRange {
                start: 200,
                end: 264,
            },
            0,
        );
        let msgs = s.render_frame(&state);
        let (style_full, _) = style_segments(&msgs).expect("StyleSpans");
        let (deco_full, _) = decorations_of(&msgs).expect("Decorations");
        assert!(
            style_full && deco_full,
            "viewport jump must be a full resync"
        );
    }

    /// T M11.7 regression: a CRDT generation transition forces a
    /// `full=true` resync on the next frame for both `StyleSpans`
    /// and `Decorations`, even when the declared viewport is
    /// unchanged. Without this, an edit at byte position N would
    /// leave the frontend with stale spans/decorations indexed at
    /// pre-edit byte positions while only a small dirty-range
    /// increment ships — which violates the incremental contract
    /// (the increment expects the frontend to retain non-dirty
    /// items). The bet-#1 surface for `pmacs-gpu`.
    #[cfg(feature = "crdt")]
    #[test]
    fn full_resync_on_generation_transition() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        set_selection(&state, 0, 1);
        let _ = s.render_frame(&state); // initial full
        assert!(
            s.render_frame(&state).is_empty(),
            "unchanged frame → silent"
        );

        // Upgrade the buffer to CRDT-backed so `buffer_generation`
        // tracks edits — without this, version_scalar stays 0 for a
        // v0.1-style buffer and the producer can't detect transitions.
        // Then bump the buffer's generation by editing it.
        {
            let core = state.core.borrow();
            let mut reg = core.registry.borrow_mut();
            let buf = reg.get_mut(buffer_id).expect("active buffer");
            buf.upgrade_to_crdt(1).expect("upgrade to crdt");
            buf.apply_edit(crate::buffer::EditOp::Insert {
                pos: 0,
                bytes: b"x",
            })
            .expect("buffer edit");
        }

        // Generation transitioned → next frame must be full for both
        // diff-shaped families when there is state to re-anchor.
        let msgs = s.render_frame(&state);
        let (style_full, _) = style_segments(&msgs).expect("StyleSpans re-emitted");
        let (deco_full, _) = decorations_of(&msgs).expect("Decorations re-emitted");
        assert!(
            style_full,
            "post-edit StyleSpans must be full=true (positions shifted)"
        );
        assert!(
            deco_full,
            "post-edit Decorations must be full=true (positions shifted)"
        );
    }

    #[test]
    fn incremental_decoration_change_ships_only_dirty_intervals() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 256 }, 0);
        set_selection(&state, 10, 12);
        let _ = s.render_frame(&state); // full: selection [10,12)
        assert!(s.render_frame(&state).is_empty());

        // Move the selection far away. The symmetric difference is the
        // old range [10,12) (removed) and the new [40,42) (added);
        // they are disjoint and non-adjacent → two segments.
        set_selection(&state, 40, 42);
        let msgs = s.render_frame(&state);
        let deco_msg = msgs
            .iter()
            .find_map(|m| match m {
                InstanceMessage::Decorations { full, segments, .. } => Some((*full, segments)),
                _ => None,
            })
            .expect("Decorations");
        assert!(!deco_msg.0, "incremental");
        let ranges: Vec<ByteRange> = deco_msg.1.iter().map(|s| s.range).collect();
        assert_eq!(
            ranges,
            vec![
                ByteRange { start: 10, end: 12 },
                ByteRange { start: 40, end: 42 }
            ],
            "two disjoint dirty intervals: old (cleared) + new"
        );
        // The [10,12) segment carries no decorations (selection moved
        // away → frontend clears it); [40,42) carries the new one.
        let s1 = &deco_msg.1[0];
        assert!(s1.decorations.is_empty(), "old selection range cleared");
        let s2 = &deco_msg.1[1];
        assert_eq!(s2.decorations.len(), 1);
        assert_eq!(s2.decorations[0].kind, DecorationKind::Selection);
    }

    #[test]
    fn unchanged_decoration_overlapping_a_dirty_interval_is_reconstructed() {
        // A diagnostic at [4,6) never changes; the selection moves to
        // overlap it. The dirty segment must still carry the (clipped)
        // diagnostic so the frontend, replacing styling within the
        // range, faithfully reconstructs the unchanged decoration.
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        seed_diagnostic(&state, buffer_id); // Warning [4,6)
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        set_selection(&state, 20, 22);
        let _ = s.render_frame(&state); // full: Sel[20,22) + Warn[4,6)
        assert!(s.render_frame(&state).is_empty());

        // Selection moves to [5,7), overlapping the diagnostic.
        set_selection(&state, 5, 7);
        let msgs = s.render_frame(&state);
        let (_full, segs) = msgs
            .iter()
            .find_map(|m| match m {
                InstanceMessage::Decorations { full, segments, .. } => Some((*full, segments)),
                _ => None,
            })
            .expect("Decorations");
        // The segment covering [5,7) must include the unchanged,
        // overlapping diagnostic (clipped into the dirty range),
        // not just the moved selection.
        let overlapping = segs
            .iter()
            .find(|s| s.range.start <= 5 && s.range.end >= 6)
            .expect("a segment covering the diagnostic's bytes");
        assert!(
            overlapping
                .decorations
                .iter()
                .any(|d| d.kind == DecorationKind::DiagnosticWarning),
            "unchanged overlapping diagnostic must be reconstructed in the dirty segment"
        );
    }

    #[test]
    fn block_adornments_still_never_emitted() {
        // BlockAdornments has no instance-side source yet, so the
        // projection never produces it (not even empty). FoldState is now
        // wired (Arc 6) but stays authoritative-empty — see
        // `fold_state_not_emitted_without_folds`.
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        for _ in 0..3 {
            for m in s.render_frame(&state) {
                assert!(
                    !matches!(m, InstanceMessage::BlockAdornments { .. }),
                    "a still-unwired block-adornment family was emitted: {m:?}"
                );
            }
        }
    }

    #[test]
    fn fold_state_not_emitted_without_folds() {
        // FoldState IS wired but authoritative-empty: a buffer with no
        // folds must never emit an (empty) FoldState frame — no empty-
        // frame spam. (Its positive transitions are pinned in the
        // folding acceptance suite.)
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        for _ in 0..3 {
            assert!(
                !s.render_frame(&state)
                    .iter()
                    .any(|m| matches!(m, InstanceMessage::FoldState { .. })),
                "no folds ⇒ no FoldState message"
            );
        }
    }

    #[test]
    fn fold_state_producer_transitions() {
        // Arc 6 Q#FD8 / acceptance 7: the three authoritative-empty
        // transitions to a semantic session — nothing until a fold exists,
        // nothing when unchanged, exactly one empty frame on
        // non-empty→empty — plus the per-session baseline reset on
        // BufferSnapshot, while BlockAdornments stays never-emitted.
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);

        let fold_frames = |s: &mut SemanticRenderState, st: &EditorState| -> Vec<Vec<ByteRange>> {
            s.render_frame(st)
                .into_iter()
                .filter_map(|m| match m {
                    InstanceMessage::FoldState { folds, .. } => Some(folds),
                    InstanceMessage::BlockAdornments { .. } => {
                        panic!("BlockAdornments must stay unproduced")
                    }
                    _ => None,
                })
                .collect()
        };

        // Nothing until a fold exists.
        assert!(fold_frames(&mut s, &state).is_empty());

        // Add a fold → exactly one FoldState frame carrying it.
        let range = ByteRange { start: 3, end: 7 };
        {
            let core = state.core.borrow();
            let mut reg = core.registry.borrow_mut();
            let buf = reg.get_mut(buffer_id).expect("buffer");
            state
                .fold_registry
                .store_or_attach(buf)
                .lock()
                .unwrap()
                .insert(range);
        }
        assert_eq!(fold_frames(&mut s, &state), vec![vec![range]]);

        // Unchanged → nothing.
        assert!(fold_frames(&mut s, &state).is_empty());

        // Clear → exactly one empty frame so the frontend drops its mirror.
        {
            let store = state.fold_registry.store(buffer_id).expect("store exists");
            store.lock().unwrap().clear();
        }
        assert_eq!(fold_frames(&mut s, &state), vec![Vec::<ByteRange>::new()]);
        // Empty → empty is suppressed.
        assert!(fold_frames(&mut s, &state).is_empty());

        // A snapshot resets the baseline: the still-empty set is again
        // suppressed as "initial empty" (the frontend cleared its mirror
        // when it applied the snapshot — the Stage 3 pairing).
        s.on_buffer_snapshot_sent(buffer_id);
        assert!(fold_frames(&mut s, &state).is_empty());
        // …and a fold added after the reset is re-shipped.
        {
            let store = state.fold_registry.store(buffer_id).expect("store exists");
            store.lock().unwrap().insert(range);
        }
        assert_eq!(fold_frames(&mut s, &state), vec![vec![range]]);
    }

    #[test]
    fn inline_adornments_not_emitted_without_hints() {
        // Step 3: InlineAdornments IS wired, but a buffer with no LSP
        // inlay-hint store entry must still never emit an (empty)
        // InlineAdornments frame — no empty-frame spam.
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let mut s = local();
        s.set_viewport(buffer_id, ByteRange { start: 0, end: 64 }, 0);
        for _ in 0..3 {
            assert!(
                !s.render_frame(&state)
                    .iter()
                    .any(|m| matches!(m, InstanceMessage::InlineAdornments { .. })),
                "no inlay hints ⇒ no InlineAdornments message"
            );
        }
    }

    /// A buffer switch re-emits the wrap mode, with no config event.
    ///
    /// This is the trigger a `FontFacts`-shaped design misses. Font size
    /// is global, so caching it by value is right; wrap mode is
    /// **buffer-local**, so a value-keyed cache stays silent when the
    /// user moves from one buffer to another with a different mode —
    /// and looks perfectly correct in every single-buffer test.
    ///
    /// Keying the cache on the `(buffer, wrap)` pair is what makes the
    /// switch re-emit, so that is what this pins.
    #[test]
    fn a_buffer_switch_re_emits_the_wrap_mode() {
        let state = empty_state();
        let mut sem = local();
        let first = active_buffer(&state);
        let second = crate::buffer::BufferId::from_raw(first.raw() + 1);

        let a = sem
            .line_wrap_msg(&state, first)
            .expect("the first buffer's mode is authoritative");
        assert!(matches!(
            a,
            InstanceMessage::LineWrapFacts { buffer_id, .. } if buffer_id == first
        ));
        assert!(
            sem.line_wrap_msg(&state, first).is_none(),
            "the same pair is suppressed"
        );

        let b = sem
            .line_wrap_msg(&state, second)
            .expect("a different buffer must be told, even at the same mode");
        assert!(matches!(
            b,
            InstanceMessage::LineWrapFacts { buffer_id, .. } if buffer_id == second
        ));
    }

    /// A pre-v22 peer is never sent the variant.
    #[test]
    fn a_v21_peer_is_not_told_about_wrapping() {
        let state = empty_state();
        let mut sem = local();
        sem.peer_knows_line_wrap = false;
        assert!(
            sem.line_wrap_msg(&state, active_buffer(&state)).is_none(),
            "gated at v22; an older peer keeps its own behavior"
        );
    }

    #[test]
    fn sibling_of_render_state_reads_same_editor_state() {
        // The dispatcher selects the projection per session, not per
        // buffer: a grid RenderState and a SemanticRenderState observe
        // the same EditorState without interfering.
        let state = empty_state();
        let mut grid = RenderState::new(CellSize::new(24, 80));
        let mut sem = local();
        let buffer_id = active_buffer(&state);
        sem.set_viewport(buffer_id, ByteRange { start: 0, end: 80 }, 0);

        let grid_msgs = grid.render_frame(&state, FrontendId::LOCAL, &HashMap::new(), &[]);
        let sem_msgs = sem.render_frame(&state);
        assert!(
            matches!(grid_msgs[0], InstanceMessage::CellDelta { .. }),
            "grid projection still produces CellDelta"
        );
        assert_semantic_only(&sem_msgs);
        assert!(
            !sem_msgs
                .iter()
                .any(|m| matches!(m, InstanceMessage::CellDelta { .. })),
            "semantic projection never produces CellDelta"
        );
    }

    // --- Step 2: LSP-semantic-token styling authority (policy A) ---

    fn tok(line: u32, start: u32, length: u32) -> crate::semantic_tokens::SemanticToken {
        crate::semantic_tokens::SemanticToken {
            line,
            start,
            length,
            token_type: 0, // index 0 in the seeded legend → "kw"
            token_modifiers: 0,
        }
    }

    /// Overwrite the semantic-token store entry for the test client
    /// `sid` on the `/tmp/x.cpp` URI.
    fn set_tokens(
        state: &EditorState,
        sid: crate::lsp::LspServerId,
        tokens: Vec<crate::semantic_tokens::SemanticToken>,
    ) {
        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/x.cpp"));
        let store = state.lsp_manager.borrow().semantic_token_store();
        store.lock().expect("sem token store").set(
            crate::semantic_tokens::SemanticTokenKey::new(sid.raw().to_string(), uri),
            crate::semantic_tokens::SemanticTokensResponse {
                tokens,
                result_id: None,
                raw: Vec::new(),
            },
        );
    }

    /// Seed a grammar-less (`.cpp`) buffer plus an Initialized test
    /// LSP client advertising a one-entry legend (`["kw"]`, UTF-16),
    /// a theme face for `kw`, and `tokens` in the store. Returns the
    /// client id so a test can re-seed the same `(server, uri)`.
    fn seed_lsp_style(
        state: &EditorState,
        buffer_id: BufferId,
        text: &[u8],
        tokens: Vec<crate::semantic_tokens::SemanticToken>,
    ) -> crate::lsp::LspServerId {
        {
            let mut core = state.core.borrow_mut();
            core.registry
                .clone()
                .borrow_mut()
                .get_mut(buffer_id)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: text,
                })
                .expect("seed buffer text");
            // `.cpp` has no bundled tree-sitter grammar → the registry
            // yields no view → policy A routes to the LSP producer.
            core.set_buffer_path(buffer_id, Some(std::path::PathBuf::from("/tmp/x.cpp")));
        }
        state.syntax_registry.theme().lock().expect("theme").insert(
            "kw",
            crate::cell::Style {
                bold: true,
                ..crate::cell::Style::default()
            },
        );
        let sid = state
            .lsp_manager
            .borrow_mut()
            .insert_initialized_test_client(
                serde_json::json!({
                    "semanticTokensProvider": {
                        "legend": { "tokenTypes": ["kw"], "tokenModifiers": [] }
                    }
                }),
                crate::lsp::PositionEncoding::Utf16,
            );
        set_tokens(state, sid, tokens);
        sid
    }

    fn seed_parse_view(
        state: &EditorState,
        buffer_id: BufferId,
        text: &[u8],
        language_name: &str,
        path: &str,
    ) -> crate::syntax::ParseViewHandle {
        let language = state
            .syntax_registry
            .language(language_name)
            .unwrap_or_else(|| panic!("{language_name} language"));
        let mut core = state.core.borrow_mut();
        let registry_handle = core.registry.clone();
        let mut registry = registry_handle.borrow_mut();
        let buf = registry.get_mut(buffer_id).expect("active buffer");
        if !text.is_empty() {
            buf.apply_edit(crate::buffer::EditOp::Insert {
                pos: 0,
                bytes: text,
            })
            .expect("seed syntax text");
        }
        let parse_view = crate::syntax::ParseView::new(buf, language, language_name.to_owned());
        let handle = parse_view.handle();
        let req = handle.make_request();
        let bundle = crate::syntax::run_parse(req).expect("initial syntax parse");
        // Mirror the production settle path: queries and lexical facts travel
        // with the same bundle the producer reads.
        handle.install(state.syntax_registry.resolve_layer_queries(&bundle));
        buf.attach_view(Box::new(parse_view));
        drop(registry);
        core.set_buffer_path(buffer_id, Some(std::path::PathBuf::from(path)));
        drop(core);
        state.syntax_registry.attach_view(buffer_id, handle.clone());
        handle
    }

    fn seed_rust_parse_view(
        state: &EditorState,
        buffer_id: BufferId,
        text: &[u8],
    ) -> crate::syntax::ParseViewHandle {
        seed_parse_view(state, buffer_id, text, "rust", "/tmp/x.rs")
    }

    fn seed_markdown_parse_view(
        state: &EditorState,
        buffer_id: BufferId,
        text: &[u8],
    ) -> crate::syntax::ParseViewHandle {
        let language = state
            .syntax_registry
            .language("markdown")
            .expect("markdown grammar");
        let mut core = state.core.borrow_mut();
        let registry_handle = core.registry.clone();
        let mut registry = registry_handle.borrow_mut();
        let buf = registry.get_mut(buffer_id).expect("active buffer");
        if !text.is_empty() {
            buf.apply_edit(crate::buffer::EditOp::Insert {
                pos: 0,
                bytes: text,
            })
            .expect("seed markdown text");
        }
        let parse_view = crate::syntax::ParseView::new(buf, language, "markdown".to_owned());
        let handle = parse_view.handle();
        let mut req = handle.make_request();
        req.injection_aliases = state.syntax_registry.injection_alias_snapshot();
        let bundle = crate::syntax::run_parse(req).expect("markdown parse");
        handle.install(state.syntax_registry.resolve_layer_queries(&bundle));
        buf.attach_view(Box::new(parse_view));
        drop(registry);
        core.set_buffer_path(buffer_id, Some(std::path::PathBuf::from("/tmp/x.md")));
        drop(core);
        state.syntax_registry.attach_view(buffer_id, handle.clone());
        handle
    }

    #[test]
    fn wire_producer_emits_disjoint_child_spans_in_fence() {
        // Framing acceptance #8: `scoped_style_spans` over a ```rust fence
        // emits a styled span on the `fn` keyword INSIDE the fence — which
        // only the injected rust layer can produce (the markdown root has
        // no keyword styling there). The pre-injection single-layer producer
        // fails this. Emitted spans are disjoint (framing Q#IJ6 flatten).
        let state = empty_state();
        let bid = active_buffer(&state);
        let src = b"# T\n\n```rust\nfn demo() {}\n```\n";
        seed_markdown_parse_view(&state, bid, src);
        let vp = DeclaredViewport {
            buffer_id: bid,
            visible: ByteRange {
                start: 0,
                end: src.len() as u64,
            },
            frontend_generation: 0,
        };
        let spans = scoped_style_spans(&state, &vp);
        assert!(!spans.is_empty(), "layered producer emits style spans");

        // Disjoint (flattened): no two spans overlap.
        let mut sorted = spans.clone();
        sorted.sort_by_key(|s| s.range.start);
        for w in sorted.windows(2) {
            assert!(
                w[0].range.end <= w[1].range.start,
                "flattened spans must be disjoint: {:?} then {:?}",
                w[0].range,
                w[1].range
            );
        }

        // The `fn` keyword inside the fence carries a non-default style.
        let fn_off = src
            .windows(2)
            .position(|w| w == b"fn")
            .expect("`fn` present in source") as u64;
        let covering = spans
            .iter()
            .find(|s| s.range.start <= fn_off && fn_off < s.range.end)
            .expect("a span covers the `fn` keyword inside the fence");
        assert_ne!(
            covering.style,
            Style::default(),
            "the injected rust keyword is styled"
        );
    }

    #[test]
    fn viewport_style_producer_uses_settled_local_facts() {
        let state = empty_state();
        let buffer_id = active_buffer(&state);
        let source = b"console;\nfunction f(console) { console; }\n";
        seed_parse_view(&state, buffer_id, source, "javascript", "/tmp/locals.js");
        state.syntax_registry.theme().lock().expect("theme").insert(
            "variable.builtin",
            Style {
                fg: crate::cell::Color::Indexed(6),
                ..Style::default()
            },
        );

        let style_at = |visible: ByteRange, offset: u64| {
            scoped_style_spans(
                &state,
                &DeclaredViewport {
                    buffer_id,
                    visible,
                    frontend_generation: 0,
                },
            )
            .into_iter()
            .find(|span| span.range.start <= offset && offset < span.range.end)
            .map_or_else(Style::default, |span| span.style)
        };

        assert_eq!(
            style_at(ByteRange { start: 0, end: 7 }, 0).fg,
            crate::cell::Color::Indexed(6),
            "unresolved outer `console` receives the builtin capture"
        );
        let inner = source
            .windows("console".len())
            .rposition(|window| window == b"console")
            .expect("inner console") as u64;
        assert_ne!(
            style_at(
                ByteRange {
                    start: inner,
                    end: inner + "console".len() as u64,
                },
                inner,
            )
            .fg,
            crate::cell::Color::Indexed(6),
            "viewport-only highlighting still sees the parameter definition outside the viewport"
        );
    }

    #[test]
    fn full_buffer_summary_flatten_scales_on_large_grammar_file() {
        // Perf gate (round-1 finding 1): the file-style summary runs the
        // FLATTENER over the WHOLE buffer, not the viewport. The ordered
        // active-set sweep keeps the flatten O(n·log n + Σ active); the
        // pre-sweep O(spans^2) flatten stalled a large grammar-backed file
        // here. (This guards the *flattener* regression specifically — the
        // summary's separate per-line dominant-style tally is a pre-existing
        // O(lines × spans) loop, not addressed or claimed linear here.)
        use std::fmt::Write as _;
        let state = empty_state();
        let bid = active_buffer(&state);
        let mut src = String::new();
        for i in 0..1500 {
            writeln!(
                src,
                "pub fn f_{i}(x: u32) -> u32 {{ let y = x + {i}; y * 2 }}"
            )
            .expect("write");
        }
        seed_rust_parse_view(&state, bid, src.as_bytes());

        let start = std::time::Instant::now();
        let summary = scoped_file_summary(&state, bid, false);
        let elapsed = start.elapsed();
        assert!(!summary.is_empty(), "summary produced for a styled buffer");
        assert!(
            elapsed < std::time::Duration::from_secs(1),
            "full-buffer flatten took {elapsed:?}; the event sweep must stay \
             ~linear (a quadratic flatten regresses here)"
        );
    }

    #[test]
    fn flatten_same_depth_sibling_later_layer_wins() {
        // Round-2 finding: two overlapping spans from different sibling
        // layers must resolve to the LATER layer (higher layer_index),
        // matching the grid's layer-by-layer paint order. The old
        // `(depth, order)` priority tied same-depth siblings, and the
        // active-set insert then reversed them — making the earlier sibling
        // win, the opposite of the grid. Layer index in the priority fixes
        // it.
        use crate::cell::Color;
        let red = Style {
            fg: Color::Indexed(1),
            ..Style::default()
        };
        let green = Style {
            fg: Color::Indexed(2),
            ..Style::default()
        };
        // Both spans are depth-1 siblings. Layer index 2 follows layer index
        // 1, so green must win even though their depth/capture order tie.
        let sibling_depth = 1;
        let earlier = layer_span_priority(1, sibling_depth, 0);
        let later = layer_span_priority(2, sibling_depth, 0);
        assert!(
            earlier < later,
            "the later sibling has higher priority despite equal depth"
        );
        let styled = vec![
            StyledLayerSpan {
                start: 0,
                end: 10,
                style: red,
                priority: earlier,
            },
            StyledLayerSpan {
                start: 3,
                end: 6,
                style: green,
                priority: later,
            },
        ];
        let out = flatten_layer_spans(&styled);
        // Byte 4 (covered by both) folds to the later layer (green).
        let covering = out
            .iter()
            .find(|s| s.range.start <= 4 && 4 < s.range.end)
            .expect("byte 4 covered");
        assert_eq!(
            covering.style.fg,
            Color::Indexed(2),
            "the later sibling layer wins at the overlap"
        );
        // Byte 1 (layer 0 only) keeps the base layer's color.
        let c1 = out
            .iter()
            .find(|s| s.range.start <= 1 && 1 < s.range.end)
            .expect("byte 1 covered");
        assert_eq!(
            c1.style.fg,
            Color::Indexed(1),
            "a non-overlapping byte keeps the base layer color"
        );
    }

    #[test]
    fn cpp_style_comes_from_lsp_when_no_tree_sitter_grammar() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        // "int" on line 0, bytes [0,3).
        seed_lsp_style(&state, bid, b"int x;\n", vec![tok(0, 0, 3)]);
        s.set_viewport(
            bid,
            ByteRange {
                start: 0,
                end: 4096,
            },
            0,
        );

        let msgs = s.render_frame(&state);
        assert_semantic_only(&msgs);
        let spans = msgs
            .iter()
            .find_map(|m| match m {
                InstanceMessage::StyleSpans { full, segments, .. } => {
                    assert!(*full, "first frame is a full resync");
                    Some(
                        segments
                            .iter()
                            .flat_map(|seg| seg.spans.clone())
                            .collect::<Vec<_>>(),
                    )
                }
                _ => None,
            })
            .expect("StyleSpans emitted from the LSP authority");
        assert_eq!(spans.len(), 1, "the one LSP token → one span");
        assert_eq!(spans[0].range, ByteRange { start: 0, end: 3 });
        assert!(
            spans[0].style.bold,
            "token_type resolved through legend → theme face"
        );
    }

    #[test]
    fn grammar_style_spans_wait_for_pending_parse() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        let handle = seed_rust_parse_view(&state, bid, b"fn main() {}\n");
        s.set_viewport(
            bid,
            ByteRange {
                start: 0,
                end: 4096,
            },
            0,
        );

        let first = s.render_frame(&state);
        assert!(
            style_segments(&first).is_some(),
            "installed parse emits the baseline style frame"
        );

        {
            let core = state.core.borrow();
            core.registry
                .borrow_mut()
                .get_mut(bid)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"// editing\n",
                })
                .expect("typing edit");
        }
        assert!(
            handle.pending_edit_count() > 0,
            "attached parse view recorded the edit"
        );
        let pending = s.render_frame(&state);
        assert!(
            style_segments(&pending).is_none(),
            "style query is skipped while edits are waiting for parse dispatch"
        );

        let req = handle.make_request();
        state.syntax_registry.record_parse_job(9001, bid);
        let in_flight = s.render_frame(&state);
        assert!(
            style_segments(&in_flight).is_none(),
            "style query is skipped while the parse job is in flight"
        );

        let bundle = crate::syntax::run_parse(req).expect("settled rust parse");
        handle.install(state.syntax_registry.resolve_layer_queries(&bundle));
        assert_eq!(state.syntax_registry.take_parse_job(9001), Some(bid));
        let settled = s.render_frame(&state);
        assert!(
            style_segments(&settled).is_some(),
            "new parse bundle emits refreshed style spans"
        );
    }

    /// Hold-while-stale for the LSP-token styling authority: once a
    /// grammar-less buffer's colors have shipped, marking the token
    /// store stale (which happens per edit) must NOT ship a clearing
    /// frame — the styling twin of the diagnostics hold. The next
    /// token response clears the flag and re-emits.
    #[test]
    fn lsp_style_holds_while_token_store_stale() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        let sid = seed_lsp_style(&state, bid, b"int x;\n", vec![tok(0, 0, 3)]);
        s.set_viewport(
            bid,
            ByteRange {
                start: 0,
                end: 4096,
            },
            0,
        );
        assert!(
            style_segments(&s.render_frame(&state)).is_some(),
            "baseline ships the LSP-token styling"
        );

        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/x.cpp"));
        {
            let store = state.lsp_manager.borrow().semantic_token_store();
            let mut guard = store.lock().expect("semantic token store");
            guard.mark_stale(uri.clone());
        }

        assert!(
            style_segments(&s.render_frame(&state)).is_none(),
            "stale token store holds StyleSpans instead of clearing the colors"
        );
        assert!(
            style_segments(&s.render_frame(&state)).is_none(),
            "the hold is stable across frames"
        );

        // A fresh token response (absorbed via `set`) clears the flag.
        // Identical tokens produce no frame — the frontend's cache was
        // never cleared, so there is nothing to say. Changed tokens
        // diff against the held baseline and ship.
        set_tokens(&state, sid, vec![tok(0, 0, 3)]);
        assert!(
            style_segments(&s.render_frame(&state)).is_none(),
            "fresh-but-identical tokens stay silent (cache was never wiped)"
        );
        set_tokens(&state, sid, vec![tok(0, 0, 5)]);
        assert!(
            style_segments(&s.render_frame(&state)).is_some(),
            "fresh changed tokens re-emit the styling"
        );
    }

    #[test]
    fn lsp_style_suppressed_when_unchanged() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        seed_lsp_style(&state, bid, b"int x;\n", vec![tok(0, 0, 3)]);
        s.set_viewport(
            bid,
            ByteRange {
                start: 0,
                end: 4096,
            },
            0,
        );

        let _ = s.render_frame(&state); // full baseline
        let again = s.render_frame(&state);
        assert!(
            !again
                .iter()
                .any(|m| matches!(m, InstanceMessage::StyleSpans { .. })),
            "unchanged LSP styling reuses the M11.4 byte-identical suppression"
        );
    }

    #[test]
    fn lsp_style_incremental_on_token_change() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        let sid = seed_lsp_style(&state, bid, b"int x;\n", vec![tok(0, 0, 3)]);
        s.set_viewport(
            bid,
            ByteRange {
                start: 0,
                end: 4096,
            },
            0,
        );
        let _ = s.render_frame(&state); // full baseline

        // Token now covers a different range ([4,5) = "x").
        set_tokens(&state, sid, vec![tok(0, 4, 1)]);
        let delta = s.render_frame(&state);
        let (full, _) = style_segments(&delta).expect("StyleSpans re-emitted");
        assert!(!full, "a changed token set ships an incremental frame");
    }

    #[test]
    fn lsp_style_empty_when_no_tokens_for_grammarless_buffer() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        {
            let mut core = state.core.borrow_mut();
            core.registry
                .clone()
                .borrow_mut()
                .get_mut(bid)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"int x;\n",
                })
                .expect("seed text");
            core.set_buffer_path(bid, Some(std::path::PathBuf::from("/tmp/x.cpp")));
        }
        s.set_viewport(
            bid,
            ByteRange {
                start: 0,
                end: 4096,
            },
            0,
        );

        let msgs = s.render_frame(&state);
        // Still a full resync (the frontend must be told "nothing
        // here"), but with no spans — no LSP store, no panic, honest
        // empty rather than a fabricated style.
        let styled = msgs.iter().find_map(|m| match m {
            InstanceMessage::StyleSpans { full, segments, .. } => Some((*full, segments.clone())),
            _ => None,
        });
        let (full, segments) = styled.expect("a full StyleSpans resync");
        assert!(full);
        assert!(
            segments.iter().all(|sg| sg.spans.is_empty()),
            "grammar-less buffer with no LSP tokens ⇒ zero spans"
        );
    }

    // --- Step 3: InlineAdornments from the LSP inlay-hint store ---

    fn inlay_uri() -> String {
        crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/h.rs"))
    }

    /// Seed the inlay-hint store. Keyed by `(server, uri)`; `for_uri`
    /// picks the lowest server, so a fixed `"1"` is fine. No LSP
    /// client needed — the inlay path never consults
    /// `semantic_style_context` (cols are already byte offsets, Step 0).
    fn set_inlay_store(state: &EditorState, uri: &str, hints: Vec<crate::inlay_hint::InlayHint>) {
        let store = state.lsp_manager.borrow().inlay_hint_store();
        store.lock().expect("inlay store").set(
            crate::inlay_hint::InlayHintKey::new("1", uri),
            crate::inlay_hint::InlayHintResponse { hints },
        );
    }

    /// Seed buffer text + path and an inlay-hint store entry.
    fn seed_inlay(
        state: &EditorState,
        buffer_id: BufferId,
        hints: Vec<crate::inlay_hint::InlayHint>,
    ) {
        {
            let mut core = state.core.borrow_mut();
            core.registry
                .clone()
                .borrow_mut()
                .get_mut(buffer_id)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"let x = f();\n",
                })
                .expect("seed buffer text");
            core.set_buffer_path(buffer_id, Some(std::path::PathBuf::from("/tmp/h.rs")));
        }
        set_inlay_store(state, &inlay_uri(), hints);
    }

    fn hint(line: u32, col: u32, label: &str) -> crate::inlay_hint::InlayHint {
        crate::inlay_hint::InlayHint {
            line,
            col,
            label: label.into(),
            kind: None,
            padding_left: false,
            padding_right: true,
            tooltip: None,
        }
    }

    fn adornments_of(msgs: &[InstanceMessage]) -> Option<Vec<InlineAdornment>> {
        msgs.iter().find_map(|m| match m {
            InstanceMessage::InlineAdornments { items, .. } => Some(items.clone()),
            _ => None,
        })
    }

    #[test]
    fn inlay_hints_project_as_inline_adornments_clipped_to_viewport() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        // `: i32` after `x` (col 5, in-viewport) and a hint past the
        // declared viewport that must be excluded.
        seed_inlay(&state, bid, vec![hint(0, 5, ": i32"), hint(0, 11, "OUT")]);
        s.set_viewport(bid, ByteRange { start: 0, end: 8 }, 0);

        let msgs = s.render_frame(&state);
        assert_semantic_only(&msgs);
        let items = adornments_of(&msgs).expect("InlineAdornments emitted");
        assert_eq!(items.len(), 1, "the out-of-viewport hint is clipped");
        let a = &items[0];
        assert_eq!(a.at, 5);
        assert_eq!(a.placement, AdornmentPlacement::AtOffset);
        match &a.content {
            AdornmentContent::Text { text, style } => {
                assert_eq!(text, ": i32 ", "padding_right adds a trailing space");
                assert_eq!(*style, Style::default());
            }
            AdornmentContent::Resource { .. } => panic!("expected Text, got Resource"),
        }
    }

    #[test]
    fn inline_adornments_suppressed_then_resync_on_change() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        seed_inlay(&state, bid, vec![hint(0, 5, ": i32")]);
        s.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);

        assert!(
            adornments_of(&s.render_frame(&state)).is_some(),
            "first frame ships the adornments"
        );
        assert!(
            adornments_of(&s.render_frame(&state)).is_none(),
            "byte-identical next frame is suppressed"
        );

        // Hint set changes → whole-set re-send (no segment diffing on
        // this wire variant).
        seed_inlay(&state, bid, vec![hint(0, 5, ": String")]);
        let again = adornments_of(&s.render_frame(&state)).expect("re-sent on change");
        match &again[0].content {
            AdornmentContent::Text { text, .. } => assert_eq!(text, ": String "),
            AdornmentContent::Resource { .. } => panic!("expected Text, got Resource"),
        }
    }

    #[test]
    fn inline_adornments_hold_while_inlay_store_stale() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        seed_inlay(&state, bid, vec![hint(0, 5, ": i32")]);
        s.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);

        assert!(
            adornments_of(&s.render_frame(&state)).is_some(),
            "first frame ships the adornments"
        );

        let uri = inlay_uri();
        let store = state.lsp_manager.borrow().inlay_hint_store();
        store.lock().expect("inlay store").mark_stale(uri.clone());

        // Hold-while-stale: no frame at all. The frontend keeps its
        // last-received hints, translated through its own local
        // edits — an empty frame here would wipe them and visibly
        // shift the line layout on the first keystroke of a burst.
        assert!(
            adornments_of(&s.render_frame(&state)).is_none(),
            "stale store holds emission (frontend keeps its translated cache)"
        );
        assert!(
            adornments_of(&s.render_frame(&state)).is_none(),
            "the hold is stable across frames"
        );

        // A fresh inlayHint response clears the flag and re-emits at
        // the server's (possibly shifted) positions.
        set_inlay_store(&state, &uri, vec![hint(0, 7, ": i32")]);
        let refreshed = adornments_of(&s.render_frame(&state)).expect("fresh hints re-emit");
        assert_eq!(refreshed.len(), 1);
        assert_eq!(refreshed[0].at, 7);
    }

    #[cfg(feature = "crdt")]
    #[test]
    fn session8_temporal_probe_sustained_edits_hold_stale_inlays_until_refresh() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        let path = std::path::PathBuf::from("/tmp/session8-inlay.txt");
        let uri = crate::lsp::path_to_file_uri(&path);

        {
            let mut core = state.core.borrow_mut();
            let mut reg = core.registry.borrow_mut();
            let buf = reg.get_mut(bid).expect("active buffer");
            buf.apply_edit(crate::buffer::EditOp::Insert {
                pos: 0,
                bytes: b"let x = f();\n",
            })
            .expect("seed buffer text");
            buf.upgrade_to_crdt(1).expect("upgrade to crdt");
            drop(reg);
            core.set_buffer_path(bid, Some(path));
        }

        set_inlay_store(&state, &uri, vec![hint(0, 5, ": i32")]);
        s.set_viewport(
            bid,
            ByteRange {
                start: 0,
                end: 4096,
            },
            0,
        );
        assert!(
            adornments_of(&s.render_frame(&state)).is_some(),
            "baseline emits the fresh inlay hint"
        );

        let mut clear_frames = 0;
        let mut full_style_frames = 0;
        let mut full_deco_frames = 0;
        for _ in 0..1000 {
            {
                let core = state.core.borrow();
                let mut reg = core.registry.borrow_mut();
                let buf = reg.get_mut(bid).expect("active buffer");
                buf.apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"x",
                })
                .expect("typing edit");
            }
            let store = state.lsp_manager.borrow().inlay_hint_store();
            store.lock().expect("inlay store").mark_stale(uri.clone());

            let msgs = s.render_frame(&state);
            if let Some((full, _)) = style_segments(&msgs)
                && full
            {
                full_style_frames += 1;
            }
            if let Some((full, _)) = decorations_of(&msgs)
                && full
            {
                full_deco_frames += 1;
            }
            if let Some(items) = adornments_of(&msgs) {
                assert!(
                    items.is_empty(),
                    "stale inlay hints must not render during sustained typing"
                );
                clear_frames += 1;
            }
        }

        assert_eq!(
            clear_frames, 0,
            "stale frames hold emission entirely — the frontend keeps \
             its locally-translated hints instead of blinking them out"
        );
        assert_eq!(
            full_style_frames, 1000,
            "each CRDT generation transition forces a StyleSpans full resync"
        );
        assert_eq!(
            full_deco_frames, 0,
            "empty Decorations state stays silent across generation transitions"
        );

        set_inlay_store(&state, &uri, vec![hint(0, 1005, ": i32")]);
        let refreshed = adornments_of(&s.render_frame(&state)).expect("fresh hints re-emit");
        assert_eq!(refreshed.len(), 1);
        assert_eq!(refreshed[0].at, 1005);
    }

    // --- M1: FileStyleSummary (minimap producer, Open Q#2) ---

    fn summary_of(msgs: &[InstanceMessage]) -> Option<(u64, Vec<Style>)> {
        msgs.iter().find_map(|m| match m {
            InstanceMessage::FileStyleSummary {
                generation, lines, ..
            } => Some((*generation, lines.clone())),
            _ => None,
        })
    }

    #[test]
    fn file_style_summary_emitted_on_first_frame_and_suppressed_thereafter() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        // Tiny buffer with no LSP / grammar setup → no styled runs →
        // per-line dominant style stays `Style::default()`. The point
        // of this test is the emit / suppress lifecycle, not content.
        {
            let core = state.core.borrow_mut();
            core.registry
                .clone()
                .borrow_mut()
                .get_mut(bid)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"a\nb\nc\n",
                })
                .expect("seed");
        }
        s.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);

        let first = s.render_frame(&state);
        assert_semantic_only(&first);
        let (_gen, lines) = summary_of(&first).expect("first frame ships a summary");
        // 4 lines: "a", "b", "c", "" (trailing empty after final \n).
        assert_eq!(lines.len(), 4);
        assert!(
            lines.iter().all(|s| *s == Style::default()),
            "no styled runs ⇒ every line takes the default style"
        );

        // Same generation → no re-emission.
        let again = s.render_frame(&state);
        assert!(
            summary_of(&again).is_none(),
            "unchanged generation suppresses the summary"
        );
    }

    #[test]
    fn file_style_summary_dominant_style_from_lsp_per_line() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        // `.cpp` buffer (no tree-sitter grammar) so styling comes from
        // the LSP semantic-token authority (policy A). Three lines;
        // line 0 has a token spanning bytes 0..3, line 2 has one
        // spanning bytes 8..11. Line 1 has no tokens.
        //
        // Buffer layout (newlines included):
        //   bytes 0..4  "abc\n"   line 0 = [0,3)  ← token [0,3)
        //   bytes 4..8  "def\n"   line 1 = [4,7)  ← no token
        //   bytes 8..12 "ghi\n"   line 2 = [8,11) ← token [8,11)
        let sid = seed_lsp_style(
            &state,
            bid,
            b"abc\ndef\nghi\n",
            vec![tok(0, 0, 3), tok(2, 0, 3)],
        );
        let _ = sid;
        s.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);

        let msgs = s.render_frame(&state);
        let (_, lines) = summary_of(&msgs).expect("summary emitted");
        assert_eq!(lines.len(), 4, "three lines + trailing empty");
        let kw_style = crate::cell::Style {
            bold: true,
            ..crate::cell::Style::default()
        };
        assert_eq!(lines[0], kw_style, "line 0 dominated by the LSP token");
        assert_eq!(lines[1], Style::default(), "line 1 has no token → default");
        assert_eq!(lines[2], kw_style, "line 2 dominated by the LSP token");
        assert_eq!(lines[3], Style::default(), "trailing empty line → default");
    }

    fn facts_of(msgs: &[InstanceMessage]) -> Option<(String, bool, u32, u32)> {
        msgs.iter().find_map(|m| match m {
            InstanceMessage::StatusFacts {
                name,
                modified,
                diag_errors,
                diag_warnings,
                ..
            } => Some((name.clone(), *modified, *diag_errors, *diag_warnings)),
            _ => None,
        })
    }

    fn search_prompt_of(msgs: &[InstanceMessage]) -> Option<(Option<String>, Option<u32>, u32)> {
        msgs.iter().find_map(|m| match m {
            InstanceMessage::SearchPrompt {
                query,
                active,
                total,
                ..
            } => Some((query.clone(), *active, *total)),
            _ => None,
        })
    }

    #[test]
    fn search_prompt_emits_on_change_and_clears_on_finish() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        // Three "foo" matches.
        {
            let core = state.core.borrow();
            core.registry
                .clone()
                .borrow_mut()
                .get_mut(bid)
                .expect("active buffer")
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"foo foo foo",
                })
                .expect("seed");
        }
        s.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);

        // No search yet: any prompt that ships carries a cleared query.
        if let Some((q, _, _)) = search_prompt_of(&s.render_frame(&state)) {
            assert!(q.is_none(), "no search ⇒ no live query");
        }

        // Begin + type "foo": the live query + active/total ship.
        {
            let mut core = state.core.borrow_mut();
            core.search_begin(true, false);
            for ch in "foo".chars() {
                core.search_input_char(ch);
            }
        }
        assert_eq!(
            search_prompt_of(&s.render_frame(&state)),
            Some((Some("foo".to_owned()), Some(0), 3)),
            "live isearch ships query + (active, total)"
        );
        // Unchanged → suppressed (cached-compare).
        assert!(search_prompt_of(&s.render_frame(&state)).is_none());

        // Step: active index advances and re-emits.
        state.core.borrow_mut().search_step(true);
        assert_eq!(
            search_prompt_of(&s.render_frame(&state)),
            Some((Some("foo".to_owned()), Some(1), 3))
        );

        // Accept: the prompt band clears (query None) even though the
        // matches stay in the store for navigation + highlight.
        state.core.borrow_mut().search_finish(true);
        assert_eq!(
            search_prompt_of(&s.render_frame(&state)),
            Some((None, None, 0)),
            "accept clears the prompt band"
        );
    }

    #[test]
    fn minibuffer_window_scrolls_to_keep_selection_visible() {
        let cands: Vec<String> = (0..30).map(|i| format!("c{i}")).collect();
        // No selection → top window, no highlight.
        let (w, sel) = minibuffer_window(&cands, None);
        assert_eq!(w.len(), MB_VISIBLE);
        assert_eq!(w[0], "c0");
        assert_eq!(sel, None);
        // Deep selection scrolls; the selected row stays inside the window.
        let (w, sel) = minibuffer_window(&cands, Some(20));
        assert_eq!(w.len(), MB_VISIBLE);
        assert_eq!(w[sel.unwrap() as usize], "c20");
        // End selection clamps the window to the tail.
        let (w, sel) = minibuffer_window(&cands, Some(29));
        assert_eq!(w.last().unwrap(), "c29");
        assert_eq!(w[sel.unwrap() as usize], "c29");
        // Short list passes through with the selection intact.
        let short: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(minibuffer_window(&short, Some(2)), (short.clone(), Some(2)));
        // Empty.
        assert_eq!(minibuffer_window(&[], Some(0)), (Vec::new(), None));
    }

    fn minibuffer_prompt_of(
        msgs: &[InstanceMessage],
    ) -> Option<(Option<String>, String, Vec<String>)> {
        msgs.iter().find_map(|m| match m {
            InstanceMessage::MinibufferPrompt {
                prompt,
                input,
                candidates,
                ..
            } => Some((prompt.clone(), input.clone(), candidates.clone())),
            _ => None,
        })
    }

    #[test]
    fn minibuffer_prompt_emits_prompt_input_and_windowed_candidates() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        s.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);

        // No minibuffer: the producer stays silent on first sight.
        assert!(minibuffer_prompt_of(&s.render_frame(&state)).is_none());

        // Open an `M-x` prompt (command completion) via the Lua API.
        state
            .lua_host
            .lua()
            .load("pmacs.minibuffer.read{ prompt = 'M-x ', source = 'commands', on_accept = function() end }")
            .exec()
            .expect("open minibuffer");
        let (prompt, input, cands) =
            minibuffer_prompt_of(&s.render_frame(&state)).expect("minibuffer prompt emitted");
        assert_eq!(prompt.as_deref(), Some("M-x "));
        assert_eq!(input, "");
        // Empty input matches every command; the wire carries a window.
        assert!(!cands.is_empty(), "M-x seeds command candidates");
        assert!(cands.len() <= MB_VISIBLE, "candidates ship windowed");

        // Unchanged → suppressed (cached-compare).
        assert!(minibuffer_prompt_of(&s.render_frame(&state)).is_none());

        // Cancel: the prompt clears (None).
        state
            .lua_host
            .lua()
            .load("pmacs.minibuffer.cancel()")
            .exec()
            .expect("cancel");
        let (prompt, _, _) = minibuffer_prompt_of(&s.render_frame(&state)).expect("clear emitted");
        assert!(prompt.is_none(), "cancel clears the minibuffer band");
    }

    #[test]
    fn status_facts_emit_on_change_and_freeze_counts_while_stale() {
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        // "abc\nde" + a Warning diagnostic + file path; the seeding
        // edit flips `modified`.
        seed_diagnostic(&state, bid);
        s.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);

        let first = s.render_frame(&state);
        let (_, modified, errors, warnings) = facts_of(&first).expect("first frame ships facts");
        assert!(modified, "the seeding edit dirtied the buffer");
        assert_eq!((errors, warnings), (0, 1));

        // Nothing changed → suppressed.
        assert!(facts_of(&s.render_frame(&state)).is_none());

        // Republish as an Error → re-emit with new counts.
        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/m114.rs"));
        let store = state.lsp_manager.borrow().diag_store();
        store.lock().expect("diag store").set(
            &uri,
            vec![crate::diag::Diagnostic {
                start_line: 0,
                start_col: 0,
                end_line: 0,
                end_col: 3,
                severity: crate::diag::DiagnosticSeverity::Error,
                message: "boom".into(),
                source: None,
                code: None,
            }],
        );
        let (_, _, errors, warnings) =
            facts_of(&s.render_frame(&state)).expect("republish re-emits");
        assert_eq!((errors, warnings), (1, 0));

        // Stale store: counts freeze at the cached value instead of
        // flickering to zero, so no re-emission either.
        store.lock().expect("diag store").mark_stale(&uri);
        assert!(facts_of(&s.render_frame(&state)).is_none());
    }

    #[test]
    fn status_facts_carry_the_transient_message() {
        // v15: `pmacs.editor.set_status` output must reach semantic
        // frontends — the "12 references" class of LSP summaries was
        // TUI-only before (the grid renders the bottom row; the wire
        // never carried the message).
        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        s.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);
        let _ = s.render_frame(&state); // baseline facts

        let message_of = |frame: &[InstanceMessage]| {
            frame.iter().find_map(|m| match m {
                InstanceMessage::StatusFacts { message, .. } => Some(message.clone()),
                _ => None,
            })
        };

        state.core.borrow_mut().status = "12 references".to_owned();
        assert_eq!(
            message_of(&s.render_frame(&state)),
            Some(Some("12 references".into())),
            "a fresh status message re-ships the facts"
        );
        // Unchanged → suppressed.
        assert_eq!(message_of(&s.render_frame(&state)), None);
        // Cleared → re-ships with None so the frontend's band returns
        // to the buffer name.
        state.core.borrow_mut().status.clear();
        assert_eq!(
            message_of(&s.render_frame(&state)),
            Some(None),
            "clearing the message re-ships the facts"
        );
    }

    #[test]
    fn zero_width_diagnostics_widen_to_a_visible_byte() {
        // "abc\nde" — line starts [0, 4], source_len 6; line 0
        // content is bytes [0, 3) (newline excluded).
        let ls = vec![0u64, 4];
        // Mid-line anchor: widen forward.
        assert_eq!(widen_zero_width_diag(1, 1, 0, &ls, 6), (1, 2));
        // End-of-line anchor (the rust-analyzer "expected COMMA"
        // shape): widen backward — forward would cover only the \n.
        assert_eq!(widen_zero_width_diag(3, 3, 0, &ls, 6), (2, 3));
        // End-of-file anchor on the last line.
        assert_eq!(widen_zero_width_diag(6, 6, 1, &ls, 6), (5, 6));
        // Non-empty ranges pass through untouched.
        assert_eq!(widen_zero_width_diag(1, 3, 0, &ls, 6), (1, 3));
    }

    #[test]
    fn file_style_summary_marks_diagnostic_lines_and_refreshes_on_republish() {
        use crate::cell::Color;
        use crate::diag::DiagnosticSeverity;

        let state = empty_state();
        let mut s = local();
        let bid = active_buffer(&state);
        // "abc\nde" with a Warning on line 1 (and a file path so the
        // buffer has a URI).
        seed_diagnostic(&state, bid);
        s.set_viewport(bid, ByteRange { start: 0, end: 64 }, 0);

        let first = s.render_frame(&state);
        let (_, lines) = summary_of(&first).expect("first frame ships a summary");
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0].underline_color,
            Color::Default,
            "clean line carries no mark"
        );
        assert_eq!(
            lines[1].underline_color,
            DiagnosticSeverity::Warning.underline_color(),
            "diagnostic line carries the severity mark"
        );

        // Unchanged generation + diag epoch → suppressed.
        assert!(summary_of(&s.render_frame(&state)).is_none());

        // A republish moves the diagnostic to line 0 and escalates it.
        // No CRDT edit happened — the diag epoch alone must re-emit
        // the summary with refreshed marks.
        let uri = crate::lsp::path_to_file_uri(std::path::Path::new("/tmp/m114.rs"));
        state
            .lsp_manager
            .borrow()
            .diag_store()
            .lock()
            .expect("diag store")
            .set(
                &uri,
                vec![crate::diag::Diagnostic {
                    start_line: 0,
                    start_col: 0,
                    end_line: 0,
                    end_col: 3,
                    severity: DiagnosticSeverity::Error,
                    message: "boom".into(),
                    source: None,
                    code: None,
                }],
            );
        let refreshed = s.render_frame(&state);
        let (_, lines) = summary_of(&refreshed).expect("diag republish re-emits the summary");
        assert_eq!(
            lines[0].underline_color,
            DiagnosticSeverity::Error.underline_color()
        );
        assert_eq!(
            lines[1].underline_color,
            Color::Default,
            "the old warning mark is gone after the republish"
        );
    }
}
