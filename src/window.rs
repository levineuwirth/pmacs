// window.rs --- Window tree, splits, and per-window state (T M2.8).

//! A *window* displays a buffer in a region of the cell grid. The
//! editor maintains a tree of windows: leaves render a single
//! buffer; splits divide their parent's area horizontally
//! (children stack vertically) or vertically (children sit
//! side-by-side). The active window is identified by its
//! [`WindowId`]; key events route to it.
//!
//! # Per-window state
//!
//! [`Window`] owns the cursor, scroll position, sticky goal column,
//! and a [`TextView`] specific to its buffer. Two windows on the
//! same buffer have independent cursors but share buffer content;
//! when the buffer mutates, both windows' text views are notified
//! by [`crate::editor_core::EditorCore`].
//!
//! # Layout
//!
//! [`Layout::compute`] walks the tree given the available [`Rect`]
//! and produces a per-window viewport rectangle. Splits are
//! proportional with integer weights, so a SIGWINCH-driven resize
//! is automatic: the new terminal area is just fed back through
//! `compute` --- ratios are intrinsic to the tree, not derived from
//! the previous absolute sizes.
//!
//! # Threading
//!
//! Single-threaded, like the rest of the editor core. Lives inside
//! [`crate::editor_core::EditorCore`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::buffer::BufferId;
use crate::cell::{CellCoord, CellSize};
use crate::rope::Position;
use crate::text_view::TextView;
use crate::view::View;

// ---------------------------------------------------------------------------
// WindowId
// ---------------------------------------------------------------------------

/// Stable identifier for a window. Allocated in monotonic order;
/// reusing a freed id is not currently supported (window-close just
/// drops the id permanently).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(u64);

impl WindowId {
    /// Mint a new id. Allocates from a process-wide counter; ids are
    /// unique across the lifetime of the process.
    #[must_use]
    pub fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// The raw id, useful for debug formatting and tests.
    #[must_use]
    pub fn raw(self) -> u64 {
        self.0
    }
}

// ---------------------------------------------------------------------------
// Rect
// ---------------------------------------------------------------------------

/// Rectangular region of the cell grid (rows × cols at a given
/// origin). Used for window viewports.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Rect {
    /// Top-left corner.
    pub origin: CellCoord,
    /// Width and height.
    pub size: CellSize,
}

impl Rect {
    /// New rect at `(row, col)` of `(rows, cols)` size.
    #[must_use]
    pub fn new(row: u32, col: u32, rows: u32, cols: u32) -> Self {
        Self {
            origin: CellCoord::new(row, col),
            size: CellSize::new(rows, cols),
        }
    }

    /// True iff the rect has positive area.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.size.rows == 0 || self.size.cols == 0
    }
}

// ---------------------------------------------------------------------------
// Orientation
// ---------------------------------------------------------------------------

/// Which axis a split divides.
///
/// Naming follows Emacs's convention, which can be confusing: a
/// **horizontal** split produces children stacked top-to-bottom (the
/// dividing line is horizontal). A **vertical** split produces
/// children side-by-side (the dividing line is vertical).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Orientation {
    /// Children stack top-to-bottom; rows are divided.
    Horizontal,
    /// Children sit side-by-side; columns are divided.
    Vertical,
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

/// An active selection in a window.
///
/// The region runs from `anchor` to the window's `cursor`. Either
/// endpoint can be the lower bound; [`Selection::range`] returns them
/// in canonical (lo, hi) order. A `Selection` with `anchor == cursor`
/// is *active but empty*: useful for "shift-click extends" semantics.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Where the selection began (mouse-down position, typically).
    pub anchor: Position,
}

/// Line-number display mode for a window's left gutter (UX gutter arc).
///
/// Defined in `pmacs-protocol` so the wire, the daemon, and both frontends
/// share one enum and one number rule ([`LineNumberMode::number_for`],
/// Q#UX7); re-exported here so `crate::window::LineNumberMode` stays the
/// in-crate path.
pub use pmacs_protocol::LineNumberMode;

/// Cells of horizontal padding the line-number gutter adds around the
/// digit field: a leading and a trailing blank, so `gutter_w = digits +
/// PAD` (Q#UX3). Kept as a named constant so both frontends can share the
/// convention (Q#UX7). `u32` to match the cell-grid column type.
pub const LINE_NUMBER_GUTTER_PAD: u32 = 2;

/// Number of decimal digits in `n` (for `n >= 1`). Allocation-free.
#[must_use]
pub fn decimal_digits(mut n: usize) -> u32 {
    let mut d = 1u32;
    while n >= 10 {
        n /= 10;
        d += 1;
    }
    d
}

// ---------------------------------------------------------------------------
// Window parameters (bottom-panel arc, Q#BP2)
// ---------------------------------------------------------------------------

/// Which edge of the frame a *side window* is pinned to.
///
/// Stage 1 of the bottom-panel arc ships exactly one side. Left / right /
/// top are named deferrals, so the enum stays closed rather than
/// accepting a value no allocator honors: a Lua caller asking for an
/// unsupported side gets a pointed error at the boundary instead of a
/// silently ordinary window.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    /// Pinned to the bottom of the frame (the panel slot).
    Bottom,
}

impl Side {
    /// Parse the Lua-facing spelling. `None` for every unsupported value.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "bottom" => Some(Self::Bottom),
            _ => None,
        }
    }

    /// The Lua-facing spelling.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Bottom => "bottom",
        }
    }
}

/// Structural floor for a window's **outer** row extent: one text row
/// plus its mode line (`content = outer - 1`).
///
/// Every programmatic source of `fixed_rows` clamps a nonzero request up
/// to this floor; a request of `0` is rejected rather than being an
/// invisible "open" (Q#BP2). This is *not* a promise that the layout can
/// never produce a smaller rect — [`Layout::compute`] has always been
/// allowed to hand out zero extents on an intrinsically tiny frame. The
/// bounded promise is narrower: the panel allocator never makes an
/// otherwise satisfiable document tree unsatisfiable.
pub const MIN_WINDOW_OUTER_ROWS: u32 = 2;

/// Default `window.panel-height`: outer rows a freshly created panel
/// takes when the caller supplies no explicit `height` (Q#BP11).
pub const DEFAULT_PANEL_ROWS: u32 = 12;

/// How far back [`QuitAction::Restore`] chains may be retained before the
/// oldest retained presentation is truncated to [`QuitAction::Delete`]
/// (Q#BP2c, R4-B6). Repeated panel replacement would otherwise grow the
/// recursive history without bound.
pub const MAX_PANEL_QUIT_DEPTH: usize = 64;

/// What `window.quit` does to a side window (Q#BP2c).
///
/// Present only on a side window; ordinary windows and every capability
/// fallback carry `None`. Replacing a side presentation captures the
/// outgoing one in `Restore` so `C → B → A → delete` restores the actual
/// presentations rather than forgetting `A` or leaking `C`'s height and
/// dedication into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuitAction {
    /// Close the side window and collapse its wrapper.
    Delete,
    /// Reinstate a previously displayed presentation, then fall back to
    /// `then` on the next quit.
    Restore {
        /// Buffer that was displayed. Revalidated at quit time: a killed
        /// buffer degrades the whole entry to [`QuitAction::Delete`].
        buffer_id: BufferId,
        /// Requested outer rows of that presentation.
        fixed_rows: u32,
        /// Whether that presentation was dedicated.
        dedicated: bool,
        /// Saved cursor, clamped against the buffer's current contents.
        cursor: Position,
        /// Saved first visible line.
        view_top: usize,
        /// Saved sticky goal column.
        goal_col: Option<u32>,
        /// Saved region, if one was active.
        selection: Option<Selection>,
        /// The action that was in force *before* this presentation
        /// replaced its predecessor.
        then: Box<QuitAction>,
    },
}

impl QuitAction {
    /// Number of retained presentations in this chain, counted
    /// iteratively so a long history can never blow the stack.
    #[must_use]
    pub fn depth(&self) -> usize {
        let mut depth = 0usize;
        let mut cursor = self;
        while let Self::Restore { then, .. } = cursor {
            depth += 1;
            cursor = then;
        }
        depth
    }

    /// Truncate the oldest retained `Restore` to [`QuitAction::Delete`]
    /// so the chain holds at most `cap` presentations. Iterative, like
    /// [`Self::depth`].
    pub fn truncate_to(&mut self, cap: usize) {
        if cap == 0 {
            *self = Self::Delete;
            return;
        }
        let mut kept = 0usize;
        let mut cursor = self;
        loop {
            match cursor {
                Self::Delete => return,
                Self::Restore { then, .. } => {
                    kept += 1;
                    if kept >= cap {
                        **then = Self::Delete;
                        return;
                    }
                    cursor = then;
                }
            }
        }
    }
}

/// Per-window display-policy parameters (Q#BP2).
///
/// `side` is immutable after placement; `quit_action` and
/// `origin_document` are implementation-owned bookkeeping that the Lua
/// `set_params` surface refuses to write (Q#BP2c), so Lua cannot forge a
/// window id, a buffer restore chain, or stale cursor state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowParams {
    /// Side this window is pinned to, or `None` for an ordinary
    /// document window. Immutable after placement (Q#BP2a).
    pub side: Option<Side>,
    /// Requested **outer** rows (including the mode line) when this is a
    /// side window. Inert on any other window — the fixed map is built
    /// from side windows only.
    pub fixed_rows: Option<u32>,
    /// Whether `display_buffer` may replace this window's buffer.
    ///
    /// Binds the **policy layer only**: raw `pmacs.window.switch_buffer`
    /// and `switch_active_buffer_for` deliberately ignore it, because
    /// they are the low-level escape hatch and every existing caller
    /// predates this arc (Q#BP2c).
    pub dedicated: bool,
    /// See [`WindowParams::quit_action`].
    quit_action: Option<QuitAction>,
    /// See [`WindowParams::origin_document`].
    origin_document: Option<WindowId>,
}

impl WindowParams {
    /// What `window.quit` does here, if anything.
    #[must_use]
    pub fn quit_action(&self) -> Option<&QuitAction> {
        self.quit_action.as_ref()
    }

    /// Install (or clear) the quit action. Rust-internal: no Lua path
    /// reaches this.
    pub fn set_quit_action(&mut self, action: Option<QuitAction>) {
        self.quit_action = action;
    }

    /// The remembered document window this side window was entered
    /// from (Q#BP2c). Recorded at panel creation, refreshed on every
    /// focus transition from a non-side window into the panel, and
    /// revalidated on every use.
    #[must_use]
    pub fn origin_document(&self) -> Option<WindowId> {
        self.origin_document
    }

    /// Record (or clear) the remembered document window. Rust-internal.
    pub fn set_origin_document(&mut self, origin: Option<WindowId>) {
        self.origin_document = origin;
    }

    /// True iff this window is pinned to a side.
    #[must_use]
    pub fn is_side(&self) -> bool {
        self.side.is_some()
    }
}

/// One leaf of the window tree: a buffer plus per-window state.
pub struct Window {
    /// Unique identifier.
    pub id: WindowId,
    /// Buffer displayed in this window.
    pub buffer_id: BufferId,
    /// Plain-text view of the buffer for this window. Each window
    /// owns its own; line offsets are independent.
    pub text_view: TextView,
    /// Composition stack: views that render after `text_view` into the
    /// same cell grid (T M2.9). See [`crate::view::View`] for the
    /// composition contract. Stored as trait objects so user-defined
    /// view kinds can join the stack via Lua in later milestones.
    pub overlays: Vec<Box<dyn View>>,
    /// Byte position of this window's cursor.
    pub cursor: Position,
    /// Active region, if any (T M2.12). Mouse drag sets the anchor
    /// at mouse-down and updates the cursor as the mouse moves; the
    /// selection lives until cleared (mouse-up with no movement, a
    /// keystroke that cancels, or a region-aware command consumes it).
    pub selection: Option<Selection>,
    /// First buffer line shown at the top of this window's viewport.
    pub view_top: usize,
    /// First display column shown at the left of this window's viewport
    /// — the horizontal scroll offset (Stage 4, framing Q#HS7).
    ///
    /// **Per window**, exactly as `view_top` is: two panes on one buffer
    /// must scroll independently. Note the deliberate asymmetry with
    /// `ui.line-wrap`, which is **buffer**-local — the two halves of one
    /// user-facing concept live at different scopes, accepted in Stage
    /// 3's Q#LL2 as a decision rather than discovered here.
    ///
    /// Always `0` while this window's buffer wraps; see
    /// [`LayoutCtx::effective_left`](crate::view::LayoutCtx::effective_left).
    pub view_left: u32,
    /// GUI Stage 1b, lifetime clause 2 — **the user's horizontal origin
    /// outranks the caret's**.
    ///
    /// Set when a deliberate horizontal scroll *effectively* moves
    /// [`Self::view_left`]; while set, the caret-following pass
    /// re-clamps the origin but does not drag it back. Without it a
    /// sideways wheel is undone by the very next paint, because
    /// `horizontal_follow` runs on every frame and knows only the
    /// caret.
    ///
    /// Cleared by a genuine cursor move (clause 4), by wrap, and by
    /// buffer replacement (clause 5).
    pub manual_left_authority: bool,
    /// The cursor as it stood when [`Self::manual_left_authority`] was
    /// armed, so clause 4 can tell a *genuine* cursor change from the
    /// follow merely running again. Meaningless while the latch is
    /// clear.
    pub manual_left_cursor: Position,
    /// Sticky display column for vertical motion.
    pub goal_col: Option<u32>,
    /// Number of text rows that fit in this window's viewport at last
    /// render. Updated by the renderer; consumed by `cursor.page-down`
    /// / `cursor.page-up`. `0` until the first render lands.
    pub last_visible_rows: u32,
    /// Width in cells of this window's **content** area at last render
    /// — the text columns, with the line-number gutter already
    /// subtracted. Updated by the renderer alongside
    /// [`Self::last_visible_rows`]; `0` until the first render lands.
    ///
    /// Content, not window, width: the gutter grows at the line-count
    /// digit boundary (9 -> 10, 99 -> 100), so the two differ and only
    /// this one is where text actually wraps.
    ///
    /// Needed because line wrapping makes the cursor's display
    /// coordinate width-dependent, and the callers that ask for it —
    /// vertical motion, paging, overlay placement — hold a window but
    /// not the frame's geometry. `last_visible_rows` established this
    /// pattern for rows; wrapping needs the other axis.
    pub last_content_cols: u32,
    /// Wrap mode this window's buffer resolved to at last render.
    ///
    /// Recorded by the driver beside [`Self::last_content_cols`], for
    /// the same reason: the mode is **buffer-local** config and the
    /// registry has no ambient buffer, so only the driver can resolve
    /// it — but vertical motion, paging and overlay placement all need
    /// it and hold a window rather than a registry.
    ///
    /// One resolution, recorded once, consumed everywhere. The
    /// alternative — each consumer resolving for itself — is how two
    /// callers end up disagreeing about the same buffer.
    pub last_wrap: crate::view::WrapMode,
    /// Line-number gutter mode for this window (UX gutter arc). `Off` by
    /// default → no gutter, no coordinate change.
    pub line_numbers: LineNumberMode,
    /// Display-policy parameters (bottom-panel arc, Q#BP2). Default for
    /// every ordinary window: no side, no fixed extent, undedicated.
    pub params: WindowParams,
}

impl Window {
    /// New window for `buffer_id`, with an attached `text_view` and
    /// cursor at the start.
    #[must_use]
    pub fn new(id: WindowId, buffer_id: BufferId, text_view: TextView) -> Self {
        Self {
            id,
            buffer_id,
            text_view,
            overlays: Vec::new(),
            cursor: 0,
            selection: None,
            view_top: 0,
            view_left: 0,
            manual_left_authority: false,
            manual_left_cursor: 0,
            goal_col: None,
            last_visible_rows: 0,
            last_content_cols: 0,
            last_wrap: crate::view::WrapMode::Truncate,
            line_numbers: LineNumberMode::Off,
            params: WindowParams::default(),
        }
    }

    /// True iff this window is pinned to a side (bottom-panel arc).
    #[must_use]
    pub fn is_side(&self) -> bool {
        self.params.is_side()
    }

    /// The layout facts a coordinate mapping needs for this window.
    ///
    /// Reads what the last render recorded, so every consumer sees the
    /// same answer the renderer used rather than deriving its own.
    #[must_use]
    pub fn layout_ctx(&self) -> crate::view::LayoutCtx {
        crate::view::LayoutCtx {
            cols: self.last_content_cols,
            wrap: self.last_wrap,
            view_left: self.view_left,
        }
    }

    /// Width in cells this window's line-number gutter occupies, or `0`
    /// when disabled (UX gutter arc, Q#UX3). `digits(line_count) + PAD`;
    /// the renderer caps this against the window width and applies it as a
    /// left offset to the text area. Every gutter coordinate-math site
    /// reads this one function so the width stays consistent.
    #[must_use]
    pub fn gutter_width(&self) -> u32 {
        // Every on-mode reserves the same width — sized for the largest
        // number any mode could show (the absolute line count, which
        // bounds relative distances and hybrid's cursor-line number). A
        // fixed width keeps the text from jittering as the cursor moves in
        // relative/hybrid modes.
        if self.line_numbers.is_on() {
            decimal_digits(self.text_view.line_count().max(1)) + LINE_NUMBER_GUTTER_PAD
        } else {
            0
        }
    }

    /// Push an overlay onto the composition stack. Overlays render
    /// after `text_view`, in the order they were pushed.
    pub fn push_overlay(&mut self, view: Box<dyn View>) {
        self.overlays.push(view);
    }

    /// Push `view` unless an overlay with the same
    /// [`View::overlay_identity`] is already attached — attachment
    /// of store-backed overlays must be idempotent per window
    /// (PR #113 round-6 finding 1: repeated switches into a buffer
    /// stacked duplicate render views on passive panes, each cloning
    /// every span and rescanning the buffer per frame). Views
    /// without an identity always push.
    pub fn ensure_overlay(&mut self, view: Box<dyn View>) {
        if let Some(id) = view.overlay_identity()
            && self
                .overlays
                .iter()
                .any(|v| v.overlay_identity() == Some(id))
        {
            return;
        }
        self.overlays.push(view);
    }

    /// Stable kind identifiers of every overlay on this window, in
    /// push order. Test seam used by `pmacs.window._overlay_kinds()`
    /// to verify that a specific overlay type actually attached
    /// (e.g. a code-format prompt result buffer expects a
    /// `"syntax-highlight"` overlay after the wire-up step).
    pub fn overlay_kinds(&self) -> Vec<&'static str> {
        self.overlays.iter().map(|v| v.kind()).collect()
    }

    /// Active region as `(lo, hi)` byte positions, if any. Returns
    /// `None` when no selection is active or when the selection is
    /// empty (anchor == cursor).
    #[must_use]
    pub fn region(&self) -> Option<(Position, Position)> {
        let sel = self.selection?;
        match sel.anchor.cmp(&self.cursor) {
            std::cmp::Ordering::Less => Some((sel.anchor, self.cursor)),
            std::cmp::Ordering::Greater => Some((self.cursor, sel.anchor)),
            std::cmp::Ordering::Equal => None,
        }
    }
}

// ---------------------------------------------------------------------------
// LayoutNode + Layout
// ---------------------------------------------------------------------------

/// One node in the window tree.
#[derive(Clone, Debug)]
pub enum LayoutNode {
    /// A single window occupying its parent's area.
    Leaf(WindowId),
    /// A split with proportional integer weights. The weights vector
    /// always has the same length as `children`; weights of `0` are
    /// treated as `1` (defensive against empty weight specs from
    /// Lua).
    Split {
        /// Direction of the dividing line.
        orientation: Orientation,
        /// Per-child weights. Sum of weights determines proportional
        /// allocation across the parent's primary axis.
        weights: Vec<u32>,
        /// Children in display order (left→right or top→bottom).
        children: Vec<LayoutNode>,
    },
}

/// Window tree + active focus.
#[derive(Clone, Debug)]
pub struct Layout {
    /// Root of the tree.
    pub root: LayoutNode,
}

/// A frontend's last authoritative cell-equivalent frame capacity
/// (Q#BP2b / Q#BP15a).
///
/// `geometry_epoch` is a monotonically increasing declaration id owned by
/// the frontend. Grid / `LOCAL` views cache their real attach and resize
/// sizes here with an internal epoch; a semantic view stays `None` —
/// **unknown**, never `24×80` — until Stage 2's authenticated
/// `FrontendCellGeometry` fills it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DeclaredFrameGeometry {
    /// Monotonic declaration id. A lower or repeated epoch carrying
    /// different data is stale.
    pub geometry_epoch: u64,
    /// Whole-frame capacity in cells, including the one global status row.
    pub total: CellSize,
}

/// T M10.8 — one attached frontend's view of the editor.
///
/// Per-frontend state for multi-frontend operation: the split tree
/// the frontend sees and which window within it is focused.
/// `WindowId`s are globally unique across all frontends — the
/// `EditorCore::windows` flat map holds every window, and each
/// frontend's `FrontendView` references a subset via its `Layout`.
///
/// The buffers themselves remain shared in `EditorCore::registry` —
/// two frontends with windows onto the same `BufferId` see the same
/// content but each window owns its own cursor / `view_top` / `goal_col`.
#[derive(Clone, Debug)]
pub struct FrontendView {
    /// Window tree visible to this frontend.
    pub layout: Layout,
    /// Focused window within `layout`. Always a `WindowId` that
    /// `layout` references (invariant: `layout.iter_ids()` contains
    /// `active`).
    pub active: WindowId,
    /// Whether this frontend's display *projects* folds — i.e. whether
    /// it collapses hidden lines away (Arc 6 Stage 2, Q#FD21).
    ///
    /// Motion, paging, wheel scrolling, click inverses, and the
    /// auto-scroll clamp all live in shared
    /// [`EditorCore`](crate::editor_core::EditorCore) code, but Stage 2
    /// collapses only the **grid** renderer — a `semantic_render` (GPU)
    /// session still displays every source line until Stage 3, and both
    /// kinds may attach to one buffer at once over the same shared fold
    /// store. Reckoning in visible lines unconditionally would make that
    /// GPU session's cursor skip lines it is still showing, so every
    /// command/event-time visible-line reckoning is gated on the
    /// **acting** frontend's flag.
    ///
    /// **Render-time clamps used to need no gate, on the premise that a
    /// semantic session never enters `paint_frame`. The bottom-panel
    /// band breaks that premise** (Q#BP17): the daemon projects a
    /// semantic frontend's side window through the same per-window
    /// painter. So the extracted painters
    /// (`prepare_window_cursor_visible`, `paint_window_content`) take the
    /// visible-line map as a **parameter**, and the panel path passes
    /// `None` when the *owning* frontend's `fold_projection` is false.
    /// That path must not call `EditorCore::fold_map_for_window`, which
    /// gates on the **active** frontend — correct for command-time
    /// reckoning, wrong for painting another frontend's panel.
    ///
    /// Set at attach from the negotiated selected-render bit (grid ⇒
    /// `true`, semantic ⇒ `false`), cleared with the view at detach, and
    /// `true` for [`FrontendId::LOCAL`](crate::protocol::FrontendId).
    /// Deliberately has no `Default`: every construction site chooses
    /// explicitly, so the projection is never inferred from a
    /// `FrontendId` (**Bet B8**).
    pub fold_projection: bool,
    /// Whether this frontend can *render* a side window (bottom-panel
    /// arc, Q#BP13).
    ///
    /// `true` for [`FrontendId::LOCAL`](crate::protocol::FrontendId) and
    /// every grid session. Stage 1 sets `false` for every semantic
    /// session — the GPU band is Stage 2 — so a `display` carrying a
    /// `side` falls back to the non-side target and **discards every
    /// side-specific parameter** rather than pinning a document window it
    /// could not show. Like `fold_projection`, deliberately has no
    /// `Default`: every construction site chooses explicitly.
    pub panel_capable: bool,
    /// This frontend's last authoritative frame capacity, or `None` while
    /// it is **unknown** (Q#BP2b).
    ///
    /// The panel allocator is the only consumer, and it must never guess:
    /// a panel requested before a real declaration stays non-presentable
    /// rather than being sized against the GPU attach request's permanent
    /// `24×80` placeholder.
    pub frame_geometry: Option<DeclaredFrameGeometry>,
    /// Cached derived layout state: the side window exists but cannot be
    /// satisfied on the current frame (Q#BP2b).
    ///
    /// Recomputed from authoritative geometry by
    /// `EditorState::reconcile_panel_layout`; never persisted, never set
    /// from Lua, and never `true` while no side window exists.
    pub panel_hidden: bool,
}

impl Layout {
    /// A trivial single-window layout.
    #[must_use]
    pub fn single(window: WindowId) -> Self {
        Self {
            root: LayoutNode::Leaf(window),
        }
    }

    /// Walk the tree and assign each leaf a viewport rectangle.
    ///
    /// Splits divide proportionally according to their weights, except
    /// that a leaf listed in `fixed` takes exactly that many **rows** out
    /// of a horizontal split before the remainder is divided (Q#BP2).
    /// The map is the *effective* allocation, not the stored request: a
    /// hidden panel is passed as `0`, which gives it an empty rect and
    /// hands every reclaimed row back to the document subtree.
    ///
    /// `fixed` is interpreted only on leaves of a **horizontal** split —
    /// a vertical split divides columns, where a row count means nothing
    /// — and the last flexible child still takes the remainder, so a tree
    /// with no fixed leaves computes byte-identically to before this arc.
    /// If a child's allocated extent is `0` (terminal too small for the
    /// split), that child receives an empty rect, and renderers must
    /// skip it.
    #[must_use]
    pub fn compute(&self, area: Rect, fixed: &HashMap<WindowId, u32>) -> HashMap<WindowId, Rect> {
        let mut out = HashMap::new();
        compute_node(&self.root, area, fixed, &mut out);
        out
    }

    /// The single side leaf among `sides`, if this layout holds one.
    ///
    /// `sides` answers "is this window pinned to a side"; the caller owns
    /// the `Window` table, so the predicate is injected rather than
    /// duplicated here. At most one bottom side leaf exists per
    /// `FrontendView` (Q#BP2a).
    #[must_use]
    pub fn side_leaf(&self, sides: impl Fn(WindowId) -> bool) -> Option<WindowId> {
        self.iter_ids().into_iter().find(|id| sides(*id))
    }

    /// The document subtree beneath the root-level panel wrapper.
    ///
    /// A side window is installed as the final child of a horizontal
    /// split wrapping the entire prior root (Q#BP2a), so the document
    /// subtree is that wrapper's first child. Returns `None` when the
    /// tree does not have that exact shape.
    #[must_use]
    pub fn document_subtree(&self, side: WindowId) -> Option<&LayoutNode> {
        match &self.root {
            LayoutNode::Split {
                orientation: Orientation::Horizontal,
                children,
                ..
            } if children.len() == 2
                && matches!(children[1], LayoutNode::Leaf(id) if id == side) =>
            {
                Some(&children[0])
            }
            _ => None,
        }
    }

    /// Wrap the entire current root in a horizontal split whose final
    /// child is `side` (Q#BP2a).
    ///
    /// `fixed_rows` makes the panel's weight inert, so the prior root
    /// keeps the flexible remainder and its **structure** — nodes,
    /// weights, order, ids — is untouched (Bet B6).
    pub fn install_side_leaf(&mut self, side: WindowId) {
        let prior = std::mem::replace(&mut self.root, LayoutNode::Leaf(side));
        self.root = LayoutNode::Split {
            orientation: Orientation::Horizontal,
            weights: vec![1, 1],
            children: vec![prior, LayoutNode::Leaf(side)],
        };
    }

    /// All [`WindowId`]s in left→right / top→bottom order.
    #[must_use]
    pub fn iter_ids(&self) -> Vec<WindowId> {
        let mut out = Vec::new();
        collect_ids(&self.root, &mut out);
        out
    }

    /// Replace the leaf currently displaying `target` with a split.
    /// Returns `true` if the leaf was found and replaced.
    pub fn split_window(
        &mut self,
        target: WindowId,
        orientation: Orientation,
        new_window: WindowId,
    ) -> bool {
        split_node(&mut self.root, target, orientation, new_window)
    }

    /// Remove the leaf for `target`. Returns `true` if removed.
    /// Collapses single-child splits in the cleanup pass.
    pub fn close_window(&mut self, target: WindowId) -> bool {
        let removed = remove_leaf(&mut self.root, target).is_some();
        if removed {
            collapse_single_child_splits(&mut self.root);
        }
        removed
    }

    /// Collapse the layout to just `keep`. Returns `false` if `keep`
    /// is not a leaf in the tree.
    pub fn keep_only(&mut self, keep: WindowId) -> bool {
        if !self.iter_ids().contains(&keep) {
            return false;
        }
        self.root = LayoutNode::Leaf(keep);
        true
    }

    /// Step focus from `current` to the next window in iteration
    /// order, wrapping around. Returns the new focus, or `current`
    /// if the layout has only one window.
    #[must_use]
    pub fn focus_next(&self, current: WindowId) -> WindowId {
        self.focus_step(current, true, &|_| true)
    }

    /// Step focus to the previous window.
    #[must_use]
    pub fn focus_prev(&self, current: WindowId) -> WindowId {
        self.focus_step(current, false, &|_| true)
    }

    /// [`Self::focus_next`] / [`Self::focus_prev`] restricted to windows
    /// `eligible` accepts (Q#BP6: a hidden panel is never a focus
    /// destination, though it becomes one again as soon as it reappears).
    ///
    /// A currently focused ineligible window can always leave, so the
    /// caller can never strand focus: `current` itself is not filtered.
    #[must_use]
    pub fn focus_step(
        &self,
        current: WindowId,
        forward: bool,
        eligible: &impl Fn(WindowId) -> bool,
    ) -> WindowId {
        let ids = self.iter_ids();
        if ids.is_empty() {
            return current;
        }
        let Some(start) = ids.iter().position(|&id| id == current) else {
            return ids
                .iter()
                .copied()
                .find(|id| eligible(*id))
                .unwrap_or_else(|| *ids.first().unwrap_or(&current));
        };
        let n = ids.len();
        for step in 1..=n {
            let i = if forward {
                (start + step) % n
            } else {
                (start + n - (step % n)) % n
            };
            if eligible(ids[i]) {
                return ids[i];
            }
        }
        current
    }

    /// Index path from the root to `target`'s leaf, or `None` when the
    /// layout does not hold it.
    #[must_use]
    pub fn path_to(&self, target: WindowId) -> Option<Vec<usize>> {
        let mut path = Vec::new();
        path_to_node(&self.root, target, &mut path).then_some(path)
    }

    /// The node at `path`, or `None` when the path does not resolve.
    #[must_use]
    pub fn node_at(&self, path: &[usize]) -> Option<&LayoutNode> {
        let mut node = &self.root;
        for &i in path {
            match node {
                LayoutNode::Split { children, .. } => node = children.get(i)?,
                LayoutNode::Leaf(_) => return None,
            }
        }
        Some(node)
    }

    /// Mutable [`Self::node_at`].
    pub fn node_at_mut(&mut self, path: &[usize]) -> Option<&mut LayoutNode> {
        let mut node = &mut self.root;
        for &i in path {
            match node {
                LayoutNode::Split { children, .. } => node = children.get_mut(i)?,
                LayoutNode::Leaf(_) => return None,
            }
        }
        Some(node)
    }

    /// The horizontal boundary immediately **below** `target` (Q#BP5b
    /// rule 2), or `None` when there is none.
    ///
    /// Walk up from the leaf to the nearest horizontal-split ancestor at
    /// which the path child has a **following sibling**. "Nearest
    /// horizontal ancestor" alone is wrong: when the subtree is that
    /// ancestor's *final* child there is no boundary below it there, and
    /// the real one is further up. This is also the boundary a drag on
    /// `target`'s bottom mode-line row moves, so keyboard resize and drag
    /// are the same operation (acceptance 31).
    #[must_use]
    pub fn boundary_below(&self, target: WindowId) -> Option<SplitBoundary> {
        let path = self.path_to(target)?;
        for depth in (0..path.len()).rev() {
            let parent_path = &path[..depth];
            let child_index = path[depth];
            let LayoutNode::Split {
                orientation: Orientation::Horizontal,
                children,
                ..
            } = self.node_at(parent_path)?
            else {
                continue;
            };
            if child_index + 1 < children.len() {
                return Some(SplitBoundary {
                    path: parent_path.to_vec(),
                    upper: child_index,
                });
            }
        }
        None
    }
}

/// One horizontal split boundary: the split node plus the index of the
/// child immediately **above** the dividing line (Q#BP5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitBoundary {
    /// Index path from the root to the horizontal split node.
    pub path: Vec<usize>,
    /// Index of the child above the boundary; `upper + 1` is below it.
    pub upper: usize,
}

fn path_to_node(node: &LayoutNode, target: WindowId, path: &mut Vec<usize>) -> bool {
    match node {
        LayoutNode::Leaf(id) => *id == target,
        LayoutNode::Split { children, .. } => {
            for (i, child) in children.iter().enumerate() {
                path.push(i);
                if path_to_node(child, target, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
    }
}

/// Minimum **outer** rows a subtree needs for every one of its leaves to
/// clear [`MIN_WINDOW_OUTER_ROWS`] (Q#BP2).
///
/// The recursion is the point: "leave the document tree two rows" is
/// wrong, because two rows at the root does not give each nested leaf two
/// rows. Horizontal splits stack rows, so minima add; vertical splits
/// share rows, so the tallest child governs.
#[must_use]
pub fn subtree_min_rows(node: &LayoutNode) -> u32 {
    match node {
        LayoutNode::Leaf(_) => MIN_WINDOW_OUTER_ROWS,
        LayoutNode::Split {
            orientation: Orientation::Horizontal,
            children,
            ..
        } => children.iter().map(subtree_min_rows).sum(),
        LayoutNode::Split {
            orientation: Orientation::Vertical,
            children,
            ..
        } => children.iter().map(subtree_min_rows).max().unwrap_or(0),
    }
}

/// The same sum/max recursion over the user's `window.min-height`
/// *preference* (Q#BP2).
///
/// `per_leaf` resolves the setting against that window's own buffer
/// (buffer-local override → global → default) and is snapshotted once per
/// gesture, before any geometry changes. Only **interactive** resize —
/// drag, keyboard, and the Stage 2 `PanelResizeRows` — consults this; the
/// ordinary layout pass and frame-resize reconciliation use
/// [`subtree_min_rows`] alone, so changing a preference can never
/// invalidate an existing layout.
#[must_use]
pub fn interactive_min_rows(node: &LayoutNode, per_leaf: &impl Fn(WindowId) -> u32) -> u32 {
    match node {
        LayoutNode::Leaf(id) => per_leaf(*id),
        LayoutNode::Split {
            orientation: Orientation::Horizontal,
            children,
            ..
        } => children
            .iter()
            .map(|child| interactive_min_rows(child, per_leaf))
            .sum(),
        LayoutNode::Split {
            orientation: Orientation::Vertical,
            children,
            ..
        } => children
            .iter()
            .map(|child| interactive_min_rows(child, per_leaf))
            .max()
            .unwrap_or(0),
    }
}

fn compute_node(
    node: &LayoutNode,
    area: Rect,
    fixed: &HashMap<WindowId, u32>,
    out: &mut HashMap<WindowId, Rect>,
) {
    match node {
        LayoutNode::Leaf(id) => {
            out.insert(*id, area);
        }
        LayoutNode::Split {
            orientation,
            weights,
            children,
        } => {
            let primary = match orientation {
                Orientation::Horizontal => area.size.rows,
                Orientation::Vertical => area.size.cols,
            };
            // Pass 1 — subtract the fixed children. Only a horizontal
            // split divides rows, so `fixed` is inert anywhere else.
            let mut extents: Vec<Option<u32>> = vec![None; children.len()];
            let mut fixed_total: u32 = 0;
            if matches!(orientation, Orientation::Horizontal) {
                for (i, child) in children.iter().enumerate() {
                    if let LayoutNode::Leaf(id) = child
                        && let Some(rows) = fixed.get(id).copied()
                    {
                        // Saturating: a request larger than the frame
                        // takes what is left rather than wrapping. The
                        // caller has already clamped against the document
                        // minimum; this is the last-resort floor.
                        let take = rows.min(primary.saturating_sub(fixed_total));
                        extents[i] = Some(take);
                        fixed_total += take;
                    }
                }
            }
            // Pass 2 — divide the remainder by weight among the flexible
            // children, preserving last-flexible-takes-the-remainder.
            let remainder = primary.saturating_sub(fixed_total);
            let total: u32 = children
                .iter()
                .enumerate()
                .filter(|(i, _)| extents[*i].is_none())
                .map(|(i, _)| weights.get(i).copied().unwrap_or(1).max(1))
                .sum();
            let last_flexible = children
                .iter()
                .enumerate()
                .rev()
                .find(|(i, _)| extents[*i].is_none())
                .map(|(i, _)| i);
            let mut flexible_used: u32 = 0;
            let mut cursor: u32 = 0;
            for (i, child) in children.iter().enumerate() {
                let extent = if let Some(rows) = extents[i] {
                    rows
                } else {
                    let w = weights.get(i).copied().unwrap_or(1).max(1);
                    // u64 intermediates: `remainder * w` is the only
                    // place this arithmetic could overflow a u32, and a
                    // saturating fallback there would hand a non-last
                    // child the whole remainder and underflow the last
                    // one. Widening deletes the case outright.
                    let e = if Some(i) == last_flexible {
                        remainder - flexible_used
                    } else if total == 0 {
                        0
                    } else {
                        u32::try_from(u64::from(remainder) * u64::from(w) / u64::from(total))
                            .unwrap_or(remainder)
                    };
                    flexible_used += e;
                    e
                };
                let child_area = match orientation {
                    Orientation::Horizontal => Rect {
                        origin: CellCoord::new(area.origin.row + cursor, area.origin.col),
                        size: CellSize::new(extent, area.size.cols),
                    },
                    Orientation::Vertical => Rect {
                        origin: CellCoord::new(area.origin.row, area.origin.col + cursor),
                        size: CellSize::new(area.size.rows, extent),
                    },
                };
                compute_node(child, child_area, fixed, out);
                cursor += extent;
            }
        }
    }
}

/// Every [`WindowId`] beneath `node`, in layout order.
#[must_use]
pub fn node_ids(node: &LayoutNode) -> Vec<WindowId> {
    let mut out = Vec::new();
    collect_ids(node, &mut out);
    out
}

fn collect_ids(node: &LayoutNode, out: &mut Vec<WindowId>) {
    match node {
        LayoutNode::Leaf(id) => out.push(*id),
        LayoutNode::Split { children, .. } => {
            for c in children {
                collect_ids(c, out);
            }
        }
    }
}

fn split_node(
    node: &mut LayoutNode,
    target: WindowId,
    orientation: Orientation,
    new_window: WindowId,
) -> bool {
    match node {
        LayoutNode::Leaf(id) if *id == target => {
            let original = *id;
            *node = LayoutNode::Split {
                orientation,
                weights: vec![1, 1],
                children: vec![LayoutNode::Leaf(original), LayoutNode::Leaf(new_window)],
            };
            true
        }
        LayoutNode::Leaf(_) => false,
        LayoutNode::Split { children, .. } => children
            .iter_mut()
            .any(|c| split_node(c, target, orientation, new_window)),
    }
}

fn remove_leaf(node: &mut LayoutNode, target: WindowId) -> Option<()> {
    match node {
        LayoutNode::Leaf(_) => None,
        LayoutNode::Split {
            children, weights, ..
        } => {
            // Direct child match?
            if let Some(idx) = children
                .iter()
                .position(|c| matches!(c, LayoutNode::Leaf(id) if *id == target))
            {
                children.remove(idx);
                if idx < weights.len() {
                    weights.remove(idx);
                }
                return Some(());
            }
            // Recurse into split children.
            for c in children.iter_mut() {
                if remove_leaf(c, target).is_some() {
                    return Some(());
                }
            }
            None
        }
    }
}

fn collapse_single_child_splits(node: &mut LayoutNode) {
    if let LayoutNode::Split { children, .. } = node {
        for c in children.iter_mut() {
            collapse_single_child_splits(c);
        }
        if children.len() == 1 {
            let only = children.remove(0);
            *node = only;
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
    fn line_number_mode_number_for_covers_all_modes() {
        use LineNumberMode::{Absolute, Hybrid, Off, Relative};
        // Cursor on buffer line 5 (0-based). Lines 3 and 7 are 2 away.
        assert_eq!(Off.number_for(3, 5), None);
        // Absolute ignores the cursor line: 1-based.
        assert_eq!(Absolute.number_for(3, 5), Some(4));
        assert_eq!(Absolute.number_for(5, 5), Some(6));
        // Relative: distance from the cursor line; cursor line is 0.
        assert_eq!(Relative.number_for(3, 5), Some(2));
        assert_eq!(Relative.number_for(7, 5), Some(2));
        assert_eq!(Relative.number_for(5, 5), Some(0));
        // Hybrid: absolute on the cursor line, relative elsewhere.
        assert_eq!(Hybrid.number_for(5, 5), Some(6));
        assert_eq!(Hybrid.number_for(3, 5), Some(2));
        // Every on-mode reserves a gutter; Off does not.
        assert!(!Off.is_on());
        assert!(Absolute.is_on() && Relative.is_on() && Hybrid.is_on());
    }

    #[test]
    fn decimal_digits_counts_correctly() {
        assert_eq!(decimal_digits(1), 1);
        assert_eq!(decimal_digits(9), 1);
        assert_eq!(decimal_digits(10), 2);
        assert_eq!(decimal_digits(99), 2);
        assert_eq!(decimal_digits(100), 3);
        assert_eq!(decimal_digits(1000), 4);
        // A 6-digit file → 6 digits + PAD gutter.
        assert_eq!(decimal_digits(123_456), 6);
    }

    fn id() -> WindowId {
        WindowId::next()
    }

    fn rect_24x80() -> Rect {
        Rect::new(0, 0, 24, 80)
    }

    #[test]
    fn single_window_takes_full_area() {
        let w = id();
        let layout = Layout::single(w);
        let placements = layout.compute(rect_24x80(), &HashMap::new());
        assert_eq!(placements.get(&w), Some(&rect_24x80()));
    }

    #[test]
    fn vertical_split_divides_columns() {
        let a = id();
        let b = id();
        let mut layout = Layout::single(a);
        assert!(layout.split_window(a, Orientation::Vertical, b));
        let placements = layout.compute(rect_24x80(), &HashMap::new());
        let ra = placements[&a];
        let rb = placements[&b];
        assert_eq!(ra.size.rows, 24);
        assert_eq!(rb.size.rows, 24);
        assert_eq!(ra.size.cols + rb.size.cols, 80);
        assert_eq!(ra.origin.col, 0);
        assert_eq!(rb.origin.col, ra.size.cols);
    }

    #[test]
    fn horizontal_split_divides_rows() {
        let a = id();
        let b = id();
        let mut layout = Layout::single(a);
        assert!(layout.split_window(a, Orientation::Horizontal, b));
        let placements = layout.compute(rect_24x80(), &HashMap::new());
        let ra = placements[&a];
        let rb = placements[&b];
        assert_eq!(ra.size.cols, 80);
        assert_eq!(rb.size.cols, 80);
        assert_eq!(ra.size.rows + rb.size.rows, 24);
    }

    #[test]
    fn ratios_are_preserved_under_resize() {
        // 2:1 horizontal split. Resizing should preserve ratio.
        let a = id();
        let b = id();
        let mut layout = Layout::single(a);
        layout.split_window(a, Orientation::Vertical, b);
        if let LayoutNode::Split { weights, .. } = &mut layout.root {
            *weights = vec![2, 1];
        } else {
            panic!("expected split");
        }
        let p1 = layout.compute(Rect::new(0, 0, 24, 90), &HashMap::new());
        assert_eq!(p1[&a].size.cols, 60);
        assert_eq!(p1[&b].size.cols, 30);
        // Resize down by 1/3.
        let p2 = layout.compute(Rect::new(0, 0, 24, 60), &HashMap::new());
        assert_eq!(p2[&a].size.cols, 40);
        assert_eq!(p2[&b].size.cols, 20);
        // Resize wide.
        let p3 = layout.compute(Rect::new(0, 0, 24, 300), &HashMap::new());
        assert_eq!(p3[&a].size.cols, 200);
        assert_eq!(p3[&b].size.cols, 100);
    }

    #[test]
    fn eight_splits_render_in_distinct_rects() {
        // Build an 8-way layout: vertical-of-4 over horizontal-of-2,
        // achieved by 3 vertical splits then 1 horizontal split per
        // column. Verify all 8 leaves get unique non-empty rects.
        let initial = id();
        let mut layout = Layout::single(initial);
        let mut leaves = vec![initial];
        // Split each existing leaf vertically until we have 4.
        for _ in 0..3 {
            let pivot = *leaves.last().unwrap();
            let new = id();
            assert!(layout.split_window(pivot, Orientation::Vertical, new));
            leaves.push(new);
        }
        // Now horizontally split each leaf.
        let mut more = Vec::new();
        for &l in &leaves {
            let new = id();
            assert!(layout.split_window(l, Orientation::Horizontal, new));
            more.push(new);
        }
        leaves.extend(more);
        assert_eq!(leaves.len(), 8);
        let placements = layout.compute(rect_24x80(), &HashMap::new());
        assert_eq!(placements.len(), 8);
        // Every rect must be non-empty (terminal large enough).
        for id in &leaves {
            let r = placements[id];
            assert!(!r.is_empty(), "rect for {id:?} was empty");
        }
        // No two rects overlap (compare pairwise).
        let rects: Vec<_> = leaves.iter().map(|id| placements[id]).collect();
        for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                assert!(!rects_overlap(&rects[i], &rects[j]));
            }
        }
    }

    fn rects_overlap(a: &Rect, b: &Rect) -> bool {
        let a_r0 = a.origin.row;
        let a_r1 = a.origin.row + a.size.rows;
        let a_c0 = a.origin.col;
        let a_c1 = a.origin.col + a.size.cols;
        let b_r0 = b.origin.row;
        let b_r1 = b.origin.row + b.size.rows;
        let b_c0 = b.origin.col;
        let b_c1 = b.origin.col + b.size.cols;
        a_r0 < b_r1 && b_r0 < a_r1 && a_c0 < b_c1 && b_c0 < a_c1
    }

    #[test]
    fn focus_next_walks_in_iteration_order() {
        let a = id();
        let b = id();
        let c = id();
        let mut layout = Layout::single(a);
        layout.split_window(a, Orientation::Vertical, b);
        layout.split_window(b, Orientation::Horizontal, c);
        let order = layout.iter_ids();
        assert_eq!(order.len(), 3);
        let mut cur = order[0];
        for expected in &[order[1], order[2], order[0], order[1]] {
            cur = layout.focus_next(cur);
            assert_eq!(&cur, expected);
        }
    }

    #[test]
    fn focus_prev_is_inverse_of_focus_next() {
        let a = id();
        let b = id();
        let c = id();
        let mut layout = Layout::single(a);
        layout.split_window(a, Orientation::Vertical, b);
        layout.split_window(b, Orientation::Horizontal, c);
        let order = layout.iter_ids();
        let mut cur = order[0];
        cur = layout.focus_next(cur);
        cur = layout.focus_prev(cur);
        assert_eq!(cur, order[0]);
    }

    #[test]
    fn close_window_collapses_single_child_split() {
        let a = id();
        let b = id();
        let mut layout = Layout::single(a);
        layout.split_window(a, Orientation::Vertical, b);
        assert_eq!(layout.iter_ids().len(), 2);
        assert!(layout.close_window(b));
        assert_eq!(layout.iter_ids(), vec![a]);
        assert!(matches!(layout.root, LayoutNode::Leaf(_)));
    }

    #[test]
    fn keep_only_collapses_to_target() {
        let a = id();
        let b = id();
        let c = id();
        let mut layout = Layout::single(a);
        layout.split_window(a, Orientation::Vertical, b);
        layout.split_window(a, Orientation::Horizontal, c);
        assert!(layout.keep_only(c));
        assert_eq!(layout.iter_ids(), vec![c]);
    }

    #[test]
    fn keep_only_returns_false_for_unknown_id() {
        let a = id();
        let bogus = id();
        let mut layout = Layout::single(a);
        assert!(!layout.keep_only(bogus));
        assert_eq!(layout.iter_ids(), vec![a]);
    }
}
