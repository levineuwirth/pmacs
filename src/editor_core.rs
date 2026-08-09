// editor_core.rs --- Mutable world state shared between Rust and Lua.

//! [`EditorCore`] is the editor's world state: a buffer registry, a
//! window tree, a focused window, file metadata, and the
//! minibuffer. Lives behind a `Rc<RefCell<...>>` so the Lua-bound
//! primitives in [`crate::lua_bindings`] (`pmacs.editor.*`,
//! `pmacs.window.*`) can mutate it from inside command bodies
//! invoked through [`crate::lua::LuaHost::invoke_command`].
//!
//! # Window model (T M2.8)
//!
//! Buffers live in [`BufferRegistry`]. Each [`Window`] points at one
//! by [`BufferId`] and owns its own cursor / view-top / goal-column /
//! [`TextView`]. The [`Layout`] tree maps the cell grid to per-window
//! viewport rectangles. A single [`WindowId`] is "active": every
//! `pmacs.editor.*` primitive operates on it; cursor and edits in
//! the run loop dispatch through it.
//!
//! When the active buffer mutates, [`EditorCore::apply_active_edit`]
//! notifies *every* window whose `buffer_id` matches the active
//! window's --- two windows on the same buffer keep their layout
//! caches synchronized.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::buffer::{Buffer, BufferId, EditOp};
use crate::file_io::{FileMeta, save_atomic};
use crate::lua_bindings::SharedRegistry;
use crate::minibuffer::Minibuffer;
use crate::protocol::FrontendId;
use crate::rope::Edit;
use crate::rope::{Position, Range};
use crate::text_view::TextView;
use crate::view::{DisplayCoord, View};
use crate::window::{
    FrontendView, Layout, LayoutNode, MAX_PANEL_QUIT_DEPTH, MIN_WINDOW_OUTER_ROWS, Orientation,
    QuitAction, Side, Window, WindowId, subtree_min_rows,
};

/// T M10.10 post-audit-round-3 F16 — origin of a queued CRDT op.
///
/// Records **whether the originating frontend already applied the
/// op to its local mirror**, which determines whether the broadcast
/// sweep should exclude that frontend.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum CrdtOpOrigin {
    /// A replica frontend's `FrontendEvent::CrdtOp` path applied the
    /// op to its local mirror before sending. Broadcast must exclude
    /// that frontend (it would double-apply otherwise — see
    /// `BufferMirror::apply_local_insert` /
    /// `apply_local_delete` and `optimistic::apply_incoming_crdt_op`'s
    /// echo-skip rule).
    OptimisticReplica(FrontendId),
    /// Daemon-side mutation (a `FrontendEvent::Key` round-trip, a
    /// Lua-driven edit, a fallback path) generated the op. No
    /// frontend has applied it locally; broadcast to every replica
    /// frontend, including the one whose `Key` event drove the
    /// daemon path (its mirror is otherwise stale).
    DaemonKey,
}

/// One recorded jump origin (bottom-panel arc, Q#BP11c).
///
/// `window_id` and `side_origin` are what make `M-,` correct once a panel
/// can be a separate window: restoring into the recorded window keeps the
/// document window untouched, and a *side* origin that no longer
/// revalidates is **skipped** rather than degrading to an active-window
/// switch — that degradation is exactly the duplicate-panel corruption
/// this design removes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct JumpEntry {
    /// Window the origin was recorded in.
    pub window_id: WindowId,
    /// Buffer displayed there at the time.
    pub buffer_id: BufferId,
    /// Cursor position to restore.
    pub position: Position,
    /// Whether `window_id` was a side window when recorded.
    pub side_origin: bool,
}

/// Which lifecycle hook Phase 2 of the display transaction must fire
/// **with the target window active** (Q#BP4 / Q#BP11b).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HookKind {
    /// `buffer.after-switch` — a reuse, including a same-buffer no-op.
    AfterSwitch,
    /// `buffer.after-load` — a fresh load. saveplace, recentf, syntax
    /// and LSP all require the document target to be active for this.
    AfterLoad,
    /// Nothing to fire (a newly created path-backed buffer for a
    /// `NotFound` path, matching initial-target / local-startup).
    None,
}

/// What a path resolved to (Journey Stage 1a, Q#JR5).
///
/// A sum type rather than `(Option<BufferId>, HookKind)`: that pair
/// admits three states that cannot occur (`None` with `AfterLoad`,
/// `Some` with a directory, …), and every caller would have to
/// re-establish by hand which combinations are real.
///
/// **Do not confuse [`HookKind`] here with [`crate::hook::HookKind`]** —
/// unrelated types sharing a name. This one says *which* lifecycle hook
/// to fire; that one says how a hook's callbacks fan out. Every site
/// touching both writes them path-qualified (Q#JR5b).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolvedTarget {
    /// A file buffer, plus the hook the caller must fire with the
    /// destination window active.
    Buffer {
        /// The resolved buffer.
        id: BufferId,
        /// Which lifecycle hook this resolution owes.
        fire: HookKind,
    },
    /// A directory. No buffer is created (Q#JR6) — the directory
    /// resolver chain decides what surfaces it, and dired builds its own
    /// buffer through `claim_handle` rather than adopting one.
    ///
    /// `path` is **normalized** — absolute, tilde-expanded, lexically
    /// clean. This is not free and must not be assumed: normalization
    /// otherwise happens inside [`Self::set_buffer_path`], which never
    /// runs on this arm, so a caller resolving `"."` would keep `"."`
    /// (Q#JR8). A handler keying state by path needs the canonical form.
    Directory {
        /// The normalized directory path.
        path: PathBuf,
    },
}

/// Where an asynchronous continuation's result belongs, captured
/// **synchronously** at request time (Journey Stage 1a, Q#JR14;
/// generalized by `docs/destination-capture-framing.md`).
///
/// The work that satisfies such a request is asynchronous (a directory
/// listing is worker-dispatched and must be awaited; so is a `git`
/// invocation), so the code that finally builds and displays the result
/// runs a tick or more later — outside interactive dispatch, where
/// `pmacs.window.*` acts on the *ambient* frontend by documented design
/// (`builtin/runtime/dired.lua`). Without a captured destination, a
/// second frontend dispatching in the meantime silently redirects the
/// result.
///
/// The fields are load-bearing, and the document pair is **optional**
/// (Q#DC-4) because a frontend showing only a side window can still host
/// a panel result:
///
/// * `frontend` — the scope the commit must run in. Always present.
/// * `window` — the exact destination; the ambient selected window is
///   not it. Absent when the frontend had no document window at capture
///   time.
/// * `buffer` — what that window held at capture time, so **stale
///   intent loses to the user** (Q#JR14c). A user who replaced the
///   buffer while the work was in flight is newer information than the
///   launch argument, and must not be overwritten. Present exactly when
///   `window` is.
///
/// The pair is set or cleared together — see
/// [`EditorCore::capture_view_destination`], which is the only place
/// that reads them off ambient state.
///
/// Which of those a commit actually requires is the **profile**, chosen
/// at `pmacs.window.commit_to` rather than at capture (Q#DC-2/Q#DC-5):
/// the document profile requires all of them, the panel profile requires
/// only a live `frontend`. Capture stays profile-blind so a caller does
/// not have to know at capture time what it will do at commit time.
///
/// Exposed to Lua only as nonconstructible userdata (Q#JR14d): as a
/// table, the *same* value is handed to every resolver listener in turn,
/// so one could mutate it and then decline — redirecting later listeners
/// — and any Lua could fabricate a plausible triple.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewDestination {
    /// Frontend that requested the work.
    pub frontend: FrontendId,
    /// Window the result must land in, when there is one.
    pub window: Option<WindowId>,
    /// Buffer that window held at capture time (stale-intent check).
    pub buffer: Option<BufferId>,
}

/// A `display_buffer` request (Q#BP3).
///
/// `height` and `dedicated` are deliberately option-valued at the policy
/// boundary: omission is **not** silently equivalent to an explicit
/// zero/false, which is what lets a user-resized panel keep its height as
/// compile and listview replace one another.
#[derive(Clone, Debug)]
pub struct DisplayRequest {
    /// Buffer to display.
    pub buffer_id: BufferId,
    /// Exact target window. Mutually exclusive with `side`.
    pub window: Option<WindowId>,
    /// Requested side. Mutually exclusive with `window`.
    pub side: Option<Side>,
    /// Explicit requested outer rows for a side placement.
    pub height: Option<u32>,
    /// Explicit dedication for the installed presentation.
    pub dedicated: Option<bool>,
    /// Explicit final-focus request. Omission defaults to `false` for an
    /// actual side target and `true` for an ordinary one; an explicit
    /// value survives fallback unchanged.
    pub select: Option<bool>,
    /// The caller's resolved `window.panel-height`, used only when a side
    /// slot is **created** with no explicit `height`.
    pub default_panel_rows: u32,
}

impl DisplayRequest {
    /// A bare ordinary-placement request for `buffer_id`.
    #[must_use]
    pub fn new(buffer_id: BufferId) -> Self {
        Self {
            buffer_id,
            window: None,
            side: None,
            height: None,
            dedicated: None,
            select: None,
            default_panel_rows: crate::window::DEFAULT_PANEL_ROWS,
        }
    }
}

/// What Phase 1 of the display transaction decided (Q#BP4).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DisplayOutcome {
    /// Window the buffer was installed in.
    pub target: WindowId,
    /// The frontend's focused window before Phase 1 ran.
    pub saved_active: WindowId,
    /// Resolved final-focus request.
    pub select: bool,
    /// Whether this call created the side window — the adopter rollback
    /// hook (a terminal whose session fails to start must remove the
    /// wrapper it just created).
    pub created_side: bool,
}

/// What [`EditorCore::reconcile_panel_layout_core`] resolved (Q#BP2b).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PanelReconciliation {
    /// The panel's effective visibility after the transaction.
    pub hidden: bool,
    /// Whether `hidden` changed in this transaction — Stage 2 keys its
    /// authoritative `PanelFrame::Absent` / fresh `Present` on this.
    pub changed: bool,
    /// A side window whose terminal controller the caller must release,
    /// because focus just left an invisible panel.
    pub released_terminal: Option<WindowId>,
}

/// What a frame-geometry declaration did (Q#BP2S1, Stage 2 §3.1).
///
/// Three-valued rather than a boolean because the caller must act
/// differently on each, and collapsing the middle arm is a defect in one
/// direction or the other: folded into `Advanced` it reconciles panel
/// layout on every repeated declaration; folded into `Rejected` it
/// reports a stale-event condition that never happened. A `Duplicate`
/// **is** accepted — which is why a narrower internal boolean would have
/// to be named `advanced`, never `accepted`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GeometryUpdate {
    /// The epoch advanced and the declaration was stored verbatim. Run
    /// panel reconciliation.
    Advanced,
    /// Same epoch, same total: already current. Do no work.
    Duplicate,
    /// Same epoch with a different total, a lower epoch, the reserved
    /// epoch `0`, an unknown frontend, or allocator exhaustion. Drop the
    /// event before any reconciliation.
    Rejected,
}

/// Row extent of an arbitrary subtree, derived from its leaves' computed
/// rects: leaves tile their parent, so the union's height is the node's.
fn node_row_extent(node: &LayoutNode, placements: &HashMap<WindowId, crate::window::Rect>) -> u32 {
    let ids = crate::window::node_ids(node);
    let mut lo = u32::MAX;
    let mut hi = 0u32;
    for id in ids {
        let Some(rect) = placements.get(&id) else {
            continue;
        };
        lo = lo.min(rect.origin.row);
        hi = hi.max(rect.origin.row + rect.size.rows);
    }
    if lo == u32::MAX { 0 } else { hi - lo }
}

/// What Phase 1 of `window.quit` did (Q#BP2c).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum QuitOutcome {
    /// The side window was closed and its wrapper collapsed.
    Deleted {
        /// Where focus landed, when the frontend still has a view.
        focus: Option<WindowId>,
    },
    /// A saved presentation was reinstalled; Phase 2 must fire the
    /// ordinary switch hook so overlays reattach.
    Restored {
        /// The window that was restored.
        target: WindowId,
        /// The buffer now displayed there.
        buffer_id: BufferId,
    },
}

#[derive(Copy, Clone, Debug)]
struct Placement {
    target: WindowId,
    kind: PlacementKind,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum PlacementKind {
    Ordinary,
    Side { created: bool, replacing: bool },
}

/// Live state of an in-progress incremental search (Q#SR5).
///
/// Present only while an isearch is running (`EditorCore::search`);
/// `None` otherwise. Holds the query as typed so far plus the cursor
/// origin to restore on cancel. The *matches* themselves live in
/// [`crate::search::SearchStore`] (shared with the decorations
/// producer and the TUI overlay); this struct is the per-session
/// input state that drives `find_all`.
#[derive(Clone, Debug)]
pub struct SearchSession {
    /// The query as typed so far. Each edit re-runs `find_all`.
    query: String,
    /// Buffer + cursor position when the search began. `C-g` / `Esc`
    /// restores this; `RET` keeps the current (match) cursor. The
    /// buffer id also anchors `find_all` to the buffer the search
    /// started in.
    origin: (BufferId, Position),
    /// Direction of the most recent step/begin. `true` = forward.
    /// Drives the prompt label ("I-search" vs "I-search backward")
    /// and the wrap direction of an empty-query repeat.
    forward: bool,
    /// Whether the query is a regex (Q#RX3). `false` = smart-case
    /// substring (`find_all`); `true` = smart-case regex
    /// (`find_all_regex`). Toggled live by `M-r`.
    regex: bool,
    /// `true` when the last recompute's regex pattern failed to compile
    /// — the prompt shows `[invalid]` instead of a match count. Always
    /// `false` in literal mode (substring never "fails to compile").
    invalid: bool,
}

/// Live state of an in-progress query-replace (Arc 2, Q#QR1).
///
/// Present only while a query-replace's interactive phase is running
/// (`EditorCore::query_replace`); `None` otherwise. Unlike
/// [`SearchSession`], the buffer is usually already mutated by the
/// time this ends, so `origin` is used *only* for the nothing-matched
/// restore (Q#QR10); every other exit leaves point at the inspected
/// match. Matching runs forward from `next_from` on the *live* buffer
/// (Q#QR2), so offset shifts and never-re-matching-replacements fall
/// out for free.
#[derive(Clone, Debug)]
pub struct QueryReplaceSession {
    /// The literal substring or regex source being replaced.
    from: String,
    /// The replacement text (may be empty — Q#QR3 deletion).
    to: String,
    /// Compiled regex engine when in regex mode (Q#QR9), cached for
    /// the whole run so `!` stays linear; `None` = smart-case literal.
    re: Option<regex::bytes::Regex>,
    /// Buffer + cursor when the session began. Restored on cancel
    /// *only* when nothing ever matched (Q#QR10).
    origin: (BufferId, Position),
    /// Byte offset the next forward search starts from — advanced past
    /// each replacement so inserted text is never re-matched.
    next_from: Position,
    /// The match currently being prompted, or `None` before the first
    /// advance / after finishing.
    current: Option<crate::protocol::ByteRange>,
    /// Number of replacements applied so far.
    replaced: usize,
    /// Whether any match was ever found (distinguishes "nothing
    /// matched → restore origin" from "matched, then quit").
    found_any: bool,
}

/// One frontend's command boundary — Emacs's `this-command` /
/// `last-command` pair (kill ring, Q#KR2).
///
/// `this` is the command currently (or most recently) executing for the
/// frontend; `last` is the one before it. A chain-sensitive command
/// (kill append, `M-y`) reads `last` *while it runs* — its own rotation
/// already moved its predecessor there. `this = None` is a broken
/// chain: some non-command input (an optimistic CRDT edit, a pointer
/// gesture, a paste, an unbound key) intervened, so the next rotation
/// makes `last = None` and every chain check fails.
#[derive(Debug, Clone, Default)]
pub struct CommandBoundary {
    /// The command executing now / most recently, or `None` after a
    /// non-command input.
    pub this: Option<String>,
    /// The command before `this`.
    pub last: Option<String>,
}

/// Exact provenance of one typed self-insert (auto-pairing Q#AP9).
///
/// `this_command() == "buffer.self-insert"` proves only the *input
/// class*; it cannot say which character was typed, where the edit
/// actually landed after intercepts, or whether the command that ran
/// under that name performed the insert at all. This record carries
/// the exact facts for the one consumer contract that needs them (the
/// pairing hook): the decoded codepoint, the requested and effective
/// ranges, and the post-edit cursor, plus a `clean` verdict (effective
/// triple equals the request). It is ephemeral — armed by the two
/// self-insert producers (dispatch fallback, optimistic CRDT arm) for
/// exactly one `buffer.after-edit` fan-out, consumable once via
/// `pmacs.editor.take_typed_edit()`, and cleared when the fan-out
/// returns. Paste, programmatic mutation, manual hook runs, and a
/// stale `this_command` therefore observe nil, not a leftover record.
#[derive(Debug, Clone)]
pub struct TypedEditRecord {
    /// Buffer the self-insert landed in.
    pub buffer: BufferId,
    /// Window that was active when the self-insert ran.
    pub window: WindowId,
    /// The exact typed codepoint (payload immutability makes this
    /// authoritative even when an intercept relocated the edit).
    pub codepoint: char,
    /// Requested edit range: `start == end` for a plain insert; a CUA
    /// type-over requests a `Replace` over the consumed region.
    pub requested_start: u64,
    /// End of the requested range (see `requested_start`).
    pub requested_end: u64,
    /// Effective (post-intercept) range start in the old rope.
    pub effective_start: u64,
    /// Effective (post-intercept) range end in the old rope.
    pub effective_end: u64,
    /// Bytes actually inserted at `effective_start`.
    pub inserted_len: u64,
    /// The window cursor immediately after the self-insert.
    pub post_cursor: u64,
    /// True iff the effective triple equals the request.
    pub clean: bool,
    /// The edited buffer's revision immediately after the completing
    /// edit — a producer-side postcondition, not consumer surface (it
    /// is not exposed on the Lua record). `typed_edit_finish` drops
    /// the record when the buffer's revision has moved past this: a
    /// redefined `buffer.self-insert` that edits again after the
    /// insert (removing or replacing the typed character) must not
    /// leave a stale-but-"clean" record for the pairing hook (PR #110
    /// round 1, finding 1).
    pub revision: u64,
}

/// In-flight arm for a [`TypedEditRecord`] (auto-pairing Q#AP9): the
/// dispatch fallback declares "the next matching self-insert edit is
/// the typed one" before invoking `buffer.self-insert`; the insert
/// primitives complete the record when the edit lands. Private —
/// nothing outside the arm/complete/finish trio observes the pending
/// state.
#[derive(Debug)]
struct TypedEditPending {
    /// Frontend whose dispatch armed this.
    fid: FrontendId,
    /// The codepoint the dispatcher decoded from the keystroke; a
    /// completing edit must match it exactly.
    codepoint: char,
    /// Filled by the first matching insert primitive.
    record: Option<TypedEditRecord>,
}

/// The world state mutated by editor commands.
pub struct EditorCore {
    /// Shared buffer registry. The registry is the canonical owner
    /// of every buffer; windows reference buffers by [`BufferId`].
    pub registry: SharedRegistry,
    /// Per-buffer fold stores (Arc 6). Shared with `EditorState`, the
    /// semantic `FoldState` producer, and the `pmacs.fold` Lua surface
    /// — the same `Rc`. The core reaches it so the six point-anchored
    /// edit primitives can run the dispatch-layer pre-edit unfold
    /// (Q#FD5): a command-path self-insert/delete at a point inside a
    /// fold unfolds it before the edit applies.
    pub fold_registry: crate::fold::SharedFoldRegistry,
    /// All windows, keyed by id for stable iteration. `WindowId`s
    /// are globally unique across all frontends; each
    /// [`FrontendView`] in `views` references a subset via its
    /// `Layout`.
    pub windows: BTreeMap<WindowId, Window>,
    /// T M10.8 — per-frontend views. Each attached frontend has its
    /// own `Layout` (split tree) + `active: WindowId`. Buffers are
    /// shared via `registry`; cursors / `view_top`s live in the
    /// per-frontend `Window` instances.
    ///
    /// Invariant: `FrontendId::LOCAL` always has an entry. The
    /// in-process editor uses this view; daemon-attached frontends
    /// register additional entries on attach (M10.8 Day 3 wires
    /// the per-attach registration via the dispatcher; Day 2 ships
    /// a fallback-to-LOCAL accessor so single-frontend tests pass
    /// before per-attach registration lands).
    pub views: HashMap<FrontendId, FrontendView>,
    /// One-line message shown in the status line.
    pub status: String,
    /// True iff the editor should exit at the next iteration.
    pub quit: bool,
    /// Universal minibuffer (T M2.7).
    pub minibuffer: Minibuffer,
    /// The frontend that produced the most recent input event
    /// dispatched to this core (T M5.4). v0.1 has a single frontend
    /// per instance, so this stays at [`FrontendId::LOCAL`] in
    /// practice; the field is load-bearing for v0.3 multi-frontend
    /// (multi-window, multi-user) where each input event must be
    /// attributable to its source frontend.
    pub active_frontend: FrontendId,
    /// T M10.8 Day 4 — pending CRDT ops queue.
    ///
    /// Each [`CrdtOpOrigin`] entry records both **what** to broadcast
    /// and **who already applied it locally** (the sender-exclusion
    /// signal). The dispatcher drains the queue per-tick and
    /// broadcasts each op to multi-frontend sessions with
    /// `crdt_replica` negotiated.
    ///
    /// # M10.10 post-audit-round-3 F16: origin tagging
    ///
    /// Sender exclusion depends on **whether the originating
    /// frontend already applied the op to its local mirror**:
    ///
    /// - [`CrdtOpOrigin::OptimisticReplica`] — a replica frontend's
    ///   `FrontendEvent::CrdtOp` path applied the op to its mirror
    ///   before sending. Broadcast must exclude that frontend so it
    ///   doesn't double-apply.
    /// - [`CrdtOpOrigin::DaemonKey`] — daemon-side mutation (a
    ///   `FrontendEvent::Key` round-trip, a Lua-driven edit, etc.)
    ///   generated the op. No frontend's mirror has applied it
    ///   locally; broadcast must include every replica frontend
    ///   *including* the active one. Without this, the
    ///   active frontend's mirror would silently drift from daemon
    ///   state after every fallback / Key-path edit.
    pub pending_crdt_ops: Vec<(CrdtOpOrigin, BufferId, crate::rope::CrdtOp)>,
    /// T M4.5 L1 — bounded jump ring. Cross-file navigation
    /// (`go-to-definition`, references, symbol jumps) pushes the
    /// pre-jump `(BufferId, Position)` here before moving the cursor;
    /// `M-,` (`jump_back`) pops the most recent entry and restores
    /// it. Bounded at [`Self::JUMP_RING_CAP`]: the oldest entry is
    /// evicted when full, so a long navigation session can't grow
    /// this without limit. Entries naming a now-removed buffer are
    /// skipped on pop (stale-handle safe, mirrors the registry's
    /// `Missing` contract).
    ///
    /// **Per frontend** (bottom-panel arc, Q#BP11c), matching
    /// `command_history`. Once a panel is a separate window, an entry
    /// must remember *which window* it was recorded in — otherwise `M-,`
    /// from a source file would switch the **document** window to the
    /// panel's buffer while the panel stays open, duplicating the
    /// presentation. Keying the whole ring by frontend additionally
    /// stops one frontend consuming or destroying another's navigation
    /// trail; detach purges the vector.
    pub jump_ring: HashMap<FrontendId, Vec<JumpEntry>>,
    /// In-buffer incremental search store (Q#SR1). Per-buffer query +
    /// matches + active index, written by the search session /
    /// `search.*` commands and read by the decorations producer
    /// ([`crate::semantic_render`]) and the TUI search overlay.
    /// Cheaply cloneable (`Arc<Mutex>`); shared with both readers.
    pub search_store: crate::search::SharedSearchStore,
    /// Shared theme handle (themes arc Q#TH9), injected once at editor
    /// bring-up right after `SyntaxRegistry` construction — the core
    /// owns no syntax state, but `ensure_search_overlay` constructs
    /// `SearchView`s that resolve wash faces through it. A bare core
    /// (unit-test construction) carries `None` and paints today's
    /// literals.
    pub theme: Option<crate::highlight::ThemeHandle>,
    /// Live incremental-search session (Q#SR5), or `None` when no
    /// search is running. Frontend-agnostic: the TUI run loop and the
    /// daemon's `FrontendEvent::Key` path both drive it through the
    /// same `search_*` methods, so isearch behaves identically in the
    /// terminal and GPU frontends. Only the *prompt surface* differs
    /// (TUI bottom row vs GPU status band).
    pub search: Option<SearchSession>,
    /// In-core clipboard slot (Q#CM6) --- the bytes a paste inserts.
    /// Written by copy/cut and by an inbound OS paste; read by paste.
    /// The frontend-agnostic source of truth, so paste behaves
    /// identically in the terminal and GPU frontends.
    clipboard_slot: Vec<u8>,
    /// One-shot outbound clipboard publish (Q#CM6). A copy/cut queues
    /// `(originating frontend, bytes)`; the dispatcher drains it and
    /// sends [`crate::protocol::InstanceSignal::Clipboard`] to that
    /// frontend, which writes the OS clipboard (OSC 52 in the TUI,
    /// `arboard` in the GPU). Drained per-tick like `pending_crdt_ops`.
    pending_clipboard: Option<(FrontendId, Vec<u8>)>,
    /// Per-frontend command boundaries (kill ring, Q#KR2) — Emacs's
    /// `this-command` / `last-command`, tracked **per frontend**: two
    /// attached frontends interleave their own command streams, and a
    /// kill chain or yank session on frontend A must not survive into
    /// frontend B's checks. Every input path updates this — commands
    /// rotate; non-command inputs (optimistic CRDT edits, pointer
    /// gestures, pastes, unbound keys) break the chain. Entries are
    /// pruned on `SessionDetached` (Q#KR11).
    pub command_history: HashMap<FrontendId, CommandBoundary>,
    /// Open context menu (Q#CM1), or `None` when closed. Shared
    /// `Arc<Mutex>` so the TUI [`crate::menu::MenuView`] overlay renders
    /// from the same state the dispatch path mutates — the menu twin of
    /// `search_store`.
    pub menu: crate::menu::SharedMenu,
    /// Open in-buffer completion popup (Arc 1a, Q#C2), or `None` when
    /// closed. Shared `Arc<Mutex>` so the TUI
    /// [`crate::completion::CompletionView`] overlay renders from the
    /// same state the dispatch path navigates and the Lua driver
    /// publishes into — the completion twin of `menu`.
    pub completion_popup: crate::completion::SharedCompletionPopup,
    /// Buffers whose input must round-trip (Arc 1b, Q#P6). While one
    /// of these is the active buffer,
    /// [`crate::editor::EditorState::dispatch_idle`] reports `false`,
    /// so semantic frontends' optimistic-apply stays off: RET reaches
    /// buffer-local bindings (a panel's visit) instead of locally
    /// inserting `\n`, and plain typing dispatches into the edit path
    /// where a read-only intercept can reject it — a CRDT-import
    /// write would bypass the intercept chain entirely. Marked from
    /// Lua via `pmacs.buffer.set_round_trip_input`; pruned on kill.
    round_trip_buffers: std::collections::HashSet<BufferId>,
    /// Live query-replace interactive session (Arc 2), or `None`. The
    /// query-replace twin of `search`; drives the fifth dispatcher
    /// shadow.
    query_replace: Option<QueryReplaceSession>,
    /// In-flight typed-edit arm (auto-pairing Q#AP9): set by the
    /// dispatch fallback just before it invokes `buffer.self-insert`,
    /// completed by the insert primitives, taken back by the
    /// dispatcher via [`Self::typed_edit_finish`] in the same
    /// dispatch. Never survives a dispatch cycle.
    typed_edit_pending: Option<TypedEditPending>,
    /// The armed typed-edit record (auto-pairing Q#AP9), exposed to
    /// Lua as `pmacs.editor.take_typed_edit()` for the duration of
    /// exactly one `buffer.after-edit` fan-out. Keyed by frontend so
    /// two attached frontends can never see or consume each other's
    /// slot; the producer clears any untaken record when the fan-out
    /// returns.
    typed_edit_armed: Option<(FrontendId, TypedEditRecord)>,
}

impl EditorCore {
    /// A fresh core with one window on a `*scratch*` buffer.
    #[must_use]
    pub fn new(registry: SharedRegistry) -> Self {
        let buffer_id = registry.borrow_mut().create("*scratch*");
        let text_view = {
            let r = registry.borrow();
            let buf = r.get(buffer_id).expect("just-created scratch buffer");
            TextView::new(buf)
        };
        let id = WindowId::next();
        let window = Window::new(id, buffer_id, text_view);
        let mut windows = BTreeMap::new();
        windows.insert(id, window);
        let mut views = HashMap::new();
        views.insert(
            FrontendId::LOCAL,
            FrontendView {
                layout: Layout::single(id),
                active: id,
                // LOCAL is the in-process grid editor (Q#FD21).
                fold_projection: true,
                // …and it renders side windows natively (Q#BP13).
                panel_capable: true,
                // Real geometry arrives with the first render/resize.
                frame_geometry: None,
                panel_hidden: false,
            },
        );
        Self {
            registry,
            fold_registry: crate::fold::make_shared_fold_registry(),
            windows,
            views,
            status: String::new(),
            quit: false,
            minibuffer: Minibuffer::new(),
            active_frontend: FrontendId::LOCAL,
            pending_crdt_ops: Vec::new(),
            jump_ring: HashMap::new(),
            search_store: crate::search::make_shared_store(),
            theme: None,
            search: None,
            clipboard_slot: Vec::new(),
            pending_clipboard: None,
            command_history: HashMap::new(),
            menu: crate::menu::make_shared_menu(),
            completion_popup: crate::completion::make_shared_popup(),
            round_trip_buffers: std::collections::HashSet::new(),
            query_replace: None,
            typed_edit_pending: None,
            typed_edit_armed: None,
        }
    }

    /// Build a core from raw bytes under `name`. Used by tests.
    /// Replaces the scratch buffer's content; the active window is
    /// retained.
    #[must_use]
    pub fn from_bytes(registry: SharedRegistry, name: impl Into<String>, bytes: &[u8]) -> Self {
        let mut core = Self::new(registry);
        let id = core.active_window().buffer_id;
        let new_id = {
            let mut reg = core.registry.borrow_mut();
            let new_id = reg.create_from_bytes(name, bytes);
            // Replace the active window's buffer with the new one.
            let _ = reg.remove(id);
            new_id
        };
        let text_view = {
            let reg = core.registry.borrow();
            TextView::new(reg.get(new_id).unwrap())
        };
        let aw = core.active_window_mut();
        aw.buffer_id = new_id;
        aw.text_view = text_view;
        aw.cursor = 0;
        aw.view_top = 0;
        aw.goal_col = None;
        core
    }

    // ---- accessors ---------------------------------------------------------

    /// T M10.8 — the active frontend's view (layout + active window).
    ///
    /// **Day 2 transitional behavior**: if `active_frontend` has no
    /// registered view (the daemon-attached frontend case before Day
    /// 3's dispatcher refactor wires `register_frontend_view`), fall
    /// back to `FrontendId::LOCAL`'s view. The invariant "LOCAL
    /// always has a view" is enforced by the constructor.
    #[must_use]
    pub fn active_view(&self) -> &FrontendView {
        self.views.get(&self.active_frontend).unwrap_or_else(|| {
            self.views.get(&FrontendId::LOCAL).expect(
                "invariant: FrontendId::LOCAL always has a registered FrontendView; \
                 populated by EditorCore::new and never removed",
            )
        })
    }

    /// Mutable view of the active frontend's [`FrontendView`].
    ///
    /// Same fallback semantics as [`active_view`].
    pub fn active_view_mut(&mut self) -> &mut FrontendView {
        // Choose the key first to avoid borrowing `self.views`
        // twice with overlapping lifetimes (the fallback path).
        let key = if self.views.contains_key(&self.active_frontend) {
            self.active_frontend
        } else {
            FrontendId::LOCAL
        };
        self.views.get_mut(&key).expect(
            "invariant: FrontendId::LOCAL always has a registered FrontendView; \
             populated by EditorCore::new and never removed",
        )
    }

    /// The active frontend's window-split tree.
    #[must_use]
    pub fn active_layout(&self) -> &Layout {
        &self.active_view().layout
    }

    /// Mutable access to the active frontend's window-split tree.
    pub fn active_layout_mut(&mut self) -> &mut Layout {
        &mut self.active_view_mut().layout
    }

    /// `WindowId` of the active frontend's focused window.
    #[must_use]
    pub fn active_window_id(&self) -> WindowId {
        self.active_view().active
    }

    /// Set the active frontend's focused window.
    pub fn set_active_window_id(&mut self, id: WindowId) {
        self.active_view_mut().active = id;
    }

    /// Reference the active [`Window`] — the window currently
    /// focused in the active frontend's view.
    #[must_use]
    pub fn active_window(&self) -> &Window {
        let id = self.active_window_id();
        self.windows
            .get(&id)
            .expect("active window present in core.windows")
    }

    /// Mutably reference the active [`Window`].
    pub fn active_window_mut(&mut self) -> &mut Window {
        let id = self.active_window_id();
        self.windows
            .get_mut(&id)
            .expect("active window present in core.windows")
    }

    /// Reference a specific frontend's active [`Window`].
    ///
    /// Returns `None` if `fid` has no registered view (no fallback —
    /// callers explicitly asking about a specific frontend get a
    /// truthful answer about whether that frontend has state).
    #[must_use]
    pub fn active_window_for(&self, fid: FrontendId) -> Option<&Window> {
        let view = self.views.get(&fid)?;
        self.windows.get(&view.active)
    }

    /// Mutably reference a specific frontend's active [`Window`].
    pub fn active_window_mut_for(&mut self, fid: FrontendId) -> Option<&mut Window> {
        let win_id = self.views.get(&fid)?.active;
        self.windows.get_mut(&win_id)
    }

    /// Whether the **acting** frontend's display collapses folds (Arc 6
    /// Stage 2, Q#FD21).
    ///
    /// The gate on every command/event-time visible-line reckoning —
    /// motion, paging, wheel, the click inverse, the auto-scroll clamp.
    /// A `semantic_render` (GPU) session still displays every source
    /// line until Stage 3, so with this `false` those sites keep their
    /// raw-line behavior and its cursor never skips a line it is showing
    /// (even while a grid session folds the same shared buffer).
    #[must_use]
    pub fn fold_projection_active(&self) -> bool {
        self.active_view().fold_projection
    }

    /// The visible-line map for `win_id`'s buffer, or `None` when the
    /// acting frontend does not project folds, `win_id` is unknown, or
    /// that buffer has no folds (Q#FD12).
    ///
    /// The two axes are deliberately separate (round-3 F1): the **acting
    /// frontend** supplies the projection policy, while the
    /// **operation's target window** supplies the buffer and line
    /// offsets. Motion, paging, and auto-scroll target the active
    /// window; the click inverse and wheel scrolling name an explicit
    /// `win_id` — a wheel event over an inactive pane does not activate
    /// it, so deriving the active window's map there would project one
    /// buffer's folds onto another.
    #[must_use]
    pub fn fold_map_for_window(
        &self,
        win_id: WindowId,
    ) -> Option<crate::fold_view::VisibleLineMap> {
        if !self.fold_projection_active() {
            return None;
        }
        let window = self.windows.get(&win_id)?;
        crate::fold_view::map_for_window(&self.fold_registry, window)
    }

    /// The layout facts a coordinate mapping needs for `win_id`.
    ///
    /// **One resolver, so every consumer flips together.** The wrap mode
    /// is buffer-local and the registry has no ambient buffer, so the
    /// resolution belongs somewhere that holds both — here — rather than
    /// at each of the twenty call sites. When `ui.line-wrap` is
    /// registered, only this function changes and every caller becomes
    /// wrap-aware at once.
    ///
    /// Width comes from the last render (`Window::last_content_cols`);
    /// `0` until the first frame lands, which
    /// [`LayoutCtx::wrapping`](crate::view::LayoutCtx::wrapping) already
    /// treats as unwrapped.
    #[must_use]
    pub fn layout_ctx(&self, win_id: WindowId) -> crate::view::LayoutCtx {
        self.windows.get(&win_id).map_or_else(
            crate::view::LayoutCtx::truncated,
            crate::window::Window::layout_ctx,
        )
    }

    /// [`Self::layout_ctx`] for the active window.
    #[must_use]
    pub fn layout_ctx_active(&self) -> crate::view::LayoutCtx {
        self.layout_ctx(self.active_window_id())
    }

    /// [`Self::fold_map_for_window`] for the active window — the target
    /// of motion, paging, and the auto-scroll clamp.
    #[must_use]
    pub fn fold_map_active(&self) -> Option<crate::fold_view::VisibleLineMap> {
        self.fold_map_for_window(self.active_window_id())
    }

    /// T M10.8 — register a `FrontendView` for `fid`. Called by the
    /// daemon on attach (Day 3 dispatcher work). Day 2's fallback
    /// path makes this optional; Day 3 makes it required.
    pub fn register_frontend_view(&mut self, fid: FrontendId, view: FrontendView) {
        self.views.insert(fid, view);
    }

    /// T M10.8 — drop a frontend's view on detach. The frontend's
    /// windows remain in `self.windows` until explicit cleanup (M10.x
    /// may add per-detach window pruning); for M10.8 they're
    /// orphaned but accessible by id (matches v0.1 behavior where
    /// closing a window left others intact).
    pub fn unregister_frontend_view(&mut self, fid: FrontendId) {
        self.views.remove(&fid);
        // Bottom-panel arc (Q#BP11c): a detached frontend's navigation
        // trail dies with its view — its `WindowId`s are gone, and no
        // other frontend may pop or destroy those entries.
        self.jump_ring.remove(&fid);
        if self.active_frontend == fid {
            self.active_frontend = FrontendId::LOCAL;
        }
    }

    /// [`BufferId`] of the active window's buffer.
    #[must_use]
    pub fn active_buffer_id(&self) -> BufferId {
        self.active_window().buffer_id
    }

    /// Path bound to the active window's buffer, if any. T M4.5 L1:
    /// replaces the old `EditorCore.file_path` field — it now lives
    /// per-buffer so cross-file navigation keeps each buffer's
    /// identity straight.
    #[must_use]
    pub fn active_buffer_path(&self) -> Option<PathBuf> {
        let id = self.active_buffer_id();
        self.registry
            .borrow()
            .get(id)
            .ok()
            .and_then(|b| b.file_path().map(Path::to_path_buf))
    }

    /// Filesystem metadata recorded for the active window's buffer.
    #[must_use]
    pub fn active_file_meta(&self) -> Option<FileMeta> {
        let id = self.active_buffer_id();
        self.registry
            .borrow()
            .get(id)
            .ok()
            .and_then(|b| b.file_meta().cloned())
    }

    /// Bind a path (and clear metadata) on a specific buffer. Used by
    /// file open / `pmacs.buffer.from_file`.
    ///
    /// The path is normalized to an absolute, lexically-clean form
    /// first ([`normalize_buffer_path`]). This is the single seam
    /// every buffer identity flows through (CLI open, Lua find-file,
    /// `WorkspaceEdit` rename ops), so doing it here keeps the invariant
    /// "a buffer's `file_path` is always absolute" — which the LSP
    /// layer relies on to build a resolvable `file:///…` URI (a
    /// relative or `~`-prefixed path produced `file://ipc.cpp`, which
    /// clangd rejected with `-32602 unresolvable URI`) and which
    /// cross-file navigation relies on for buffer-identity matching.
    pub fn set_buffer_path(&mut self, id: BufferId, path: Option<PathBuf>) {
        let path = path.map(normalize_buffer_path);
        if let Ok(b) = self.registry.borrow_mut().get_mut(id) {
            b.set_file_path(path);
        }
    }

    /// Record filesystem metadata on a specific buffer.
    pub fn set_buffer_meta(&mut self, id: BufferId, meta: Option<FileMeta>) {
        if let Ok(b) = self.registry.borrow_mut().get_mut(id) {
            b.set_file_meta(meta);
        }
    }

    /// Find the buffer already showing `path`, or load it fresh from
    /// disk into a new buffer — **without** switching the active window
    /// (Arc 3 desktop-restore builds its windows explicitly). Returns
    /// `(id, newly_loaded)`; `newly_loaded` is `false` on a dedup hit
    /// (the same file in two split panes) so the caller fires
    /// `buffer.after-load` at most once per buffer.
    ///
    /// # Errors
    /// Propagates a load failure (e.g. a since-deleted file) so restore
    /// can skip that leaf rather than abort.
    pub fn get_or_load_buffer(&mut self, path: &Path) -> std::io::Result<(BufferId, bool)> {
        if let Some(id) = self.find_buffer_for_path(path) {
            return Ok((id, false));
        }
        let normalized = normalize_buffer_path(path.to_path_buf());
        let (bytes, meta) = crate::file_io::load_file(path)?;
        // The name is the path **as given** — a relative open is named
        // `foo.rs` while `file_path` below is absolute. Recording the
        // provenance (Q#DR30) is what lets rename reconciliation move
        // this name without having to guess from the string.
        let display_name = path.display().to_string();
        let id = self
            .registry
            .borrow_mut()
            .create_from_bytes(display_name.clone(), &bytes);
        if let Ok(b) = self.registry.borrow_mut().get_mut(id) {
            b.set_path_derived_name(display_name);
        }
        self.set_buffer_path(id, Some(normalized));
        self.set_buffer_meta(id, Some(meta));
        Ok((id, true))
    }

    /// The buffer already bound to `path`, under the same normalization
    /// [`Self::get_or_load_buffer`] uses — **side-effect free**, so a
    /// target-aware display can resolve its destination *before* any I/O
    /// (Q#BP11b step 1: an ineligible destination must fail without
    /// loading the file).
    #[must_use]
    pub fn find_buffer_for_path(&self, path: &Path) -> Option<BufferId> {
        let normalized = normalize_buffer_path(path.to_path_buf());
        self.registry.borrow().find_by_path(&normalized)
    }

    /// The shared resolve/load-without-switch primitive behind both
    /// `pmacs.window.display_file` and the daemon's initial-target
    /// bootstrap (Q#BP11b).
    ///
    /// Returns the buffer plus the hook Phase 2 must fire **with the
    /// destination window active**: `AfterSwitch` for a dedup hit
    /// (including a same-buffer no-op), `AfterLoad` for a fresh load, and
    /// `None` for a path that does not exist yet — a `NotFound` path
    /// becomes an empty path-backed buffer and fires nothing, matching
    /// the initial-target and local-startup contract.
    ///
    /// One primitive, so two path-normalization, dedup, and hook
    /// transactions cannot drift apart.
    ///
    /// A **directory** resolves to [`ResolvedTarget::Directory`] before
    /// any load is attempted (Journey Stage 1a, Q#JR5/Q#JR6). Without
    /// that arm the load runs and fails: `File::open` succeeds on a
    /// directory and `read_to_end` then returns `EISDIR`, which is not
    /// `NotFound`, so the `[new file]` arm never fires and every caller
    /// saw a hard error — the reason `pmacs .` exited 1 and the golden
    /// journey was graded broken at step 3 (`COHERENCE.md` §2).
    ///
    /// # Errors
    /// Any load failure other than `NotFound`.
    pub fn resolve_target_buffer(&mut self, path: &Path) -> Result<ResolvedTarget, String> {
        // Ahead of the load, deliberately: see the EISDIR note above.
        if path.is_dir() {
            return Ok(ResolvedTarget::Directory {
                path: normalize_buffer_path(path.to_path_buf()),
            });
        }
        match self.get_or_load_buffer(path) {
            Ok((id, true)) => Ok(ResolvedTarget::Buffer {
                id,
                fire: HookKind::AfterLoad,
            }),
            Ok((id, false)) => Ok(ResolvedTarget::Buffer {
                id,
                fire: HookKind::AfterSwitch,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let display_path = path.display().to_string();
                let buffer_id = self.registry.borrow_mut().create(display_path.clone());
                // Path-backed creation site (Q#DR30): the name is the
                // path, so a later rename may move it.
                if let Ok(b) = self.registry.borrow_mut().get_mut(buffer_id) {
                    b.set_path_derived_name(display_path);
                }
                self.set_buffer_path(buffer_id, Some(path.to_path_buf()));
                "[new file]".clone_into(&mut self.status);
                Ok(ResolvedTarget::Buffer {
                    id: buffer_id,
                    fire: HookKind::None,
                })
            }
            Err(error) => Err(format!("cannot open {}: {error}", path.display())),
        }
    }

    /// Cursor of the active window (compatibility shim for callers
    /// migrated from pre-M2.8 code).
    #[must_use]
    pub fn cursor(&self) -> Position {
        self.active_window().cursor
    }

    /// `view_top` of the active window.
    #[must_use]
    pub fn view_top(&self) -> usize {
        self.active_window().view_top
    }

    /// Set the active window's cursor to a byte offset, clamped to the
    /// buffer extent (Arc 3 Q#PS1 — saveplace/desktop restore). Resets
    /// the goal column. Since `switch_active_buffer` zeroes the cursor,
    /// restore calls this *after* the open/switch.
    pub fn set_cursor_byte(&mut self, byte: u64) {
        let clamped = byte.min(self.active_buffer_len());
        let aw = self.active_window_mut();
        aw.cursor = clamped;
        aw.goal_col = None;
    }

    /// Set the active window's `view_top` (first visible source line),
    /// clamped to the buffer's line count (Arc 3 Q#PS1 — desktop
    /// restore). A file that shrank since the desktop was saved can't
    /// scroll past its end.
    /// Arc 6 Stage 2 (Q#FD12/Q#FD18, round-4 F3): `view_top` stays a
    /// source-line index but must never *name* a hidden line, so a
    /// fold-projecting frontend also clamps **backward** to the visible
    /// head here. The render pass repairs a hidden `view_top` too, but
    /// only at the next frame — until then [`Self::view_top`] would hand
    /// out a collapsed line and command/event reckoning would start from
    /// a non-visible origin. This setter is the contract's home (it is
    /// what `saveplace` and `pmacs.editor.set_view_top` call), so the
    /// invariant is established here rather than repaired downstream.
    pub fn set_view_top(&mut self, top: usize) {
        let lines = self.active_window().text_view.line_count();
        let clamped = top.min(lines.saturating_sub(1));
        let clamped = self
            .fold_map_active()
            .map_or(clamped, |map| map.clamp_view_top(clamped));
        self.active_window_mut().view_top = clamped;
    }

    /// Active buffer's byte length.
    #[must_use]
    pub fn active_buffer_len(&self) -> u64 {
        let id = self.active_buffer_id();
        self.registry.borrow().get(id).map_or(0, Buffer::len)
    }

    /// Active buffer's name. Returns an owned String to release the
    /// registry borrow promptly.
    #[must_use]
    pub fn active_buffer_name(&self) -> String {
        let id = self.active_buffer_id();
        self.registry
            .borrow()
            .get(id)
            .map(|b| b.name().to_owned())
            .unwrap_or_default()
    }

    /// Returns true iff the active buffer has unsaved modifications.
    #[must_use]
    pub fn active_buffer_is_modified(&self) -> bool {
        let id = self.active_buffer_id();
        self.registry
            .borrow()
            .get(id)
            .is_ok_and(Buffer::is_modified)
    }

    /// 0-based line index containing the active window's cursor.
    #[must_use]
    pub fn cursor_line(&self) -> usize {
        let aw = self.active_window();
        aw.text_view.line_at_offset(aw.cursor)
    }

    /// Move the active window's cursor to the start of a 0-based line.
    /// Out-of-range line numbers clamp to the last line.
    pub fn move_to_line(&mut self, line: usize) {
        let line_count = self.active_window().text_view.line_count().max(1);
        let target_line = line.min(line_count - 1);
        let target = self
            .active_window()
            .text_view
            .line_offset(target_line)
            .unwrap_or_else(|| self.active_buffer_len());
        let aw = self.active_window_mut();
        aw.cursor = target;
        aw.goal_col = None;
    }

    // ---- jump ring (T M4.5 L1) ---------------------------------------------

    /// Bound on [`Self::jump_ring`]. Large enough for a deep
    /// cross-file dig (definition → definition → references …),
    /// small enough that a stuck loop can't grow memory unbounded.
    pub const JUMP_RING_CAP: usize = 64;

    /// Record the active window's current `(buffer, cursor)` as a
    /// jump origin. Call this *before* moving the cursor on a
    /// navigation action (go-to-definition, references, symbol jump)
    /// so `M-,` can return here.
    ///
    /// When the ring is at [`Self::JUMP_RING_CAP`], the oldest
    /// origin is evicted (front drop) — the user keeps the most
    /// recent trail, which is the one they're likely to unwind.
    pub fn push_jump(&mut self) {
        let fid = self.active_frontend;
        let window_id = self.active_window_id();
        let entry = JumpEntry {
            window_id,
            buffer_id: self.active_buffer_id(),
            position: self.cursor(),
            side_origin: self
                .windows
                .get(&window_id)
                .is_some_and(crate::window::Window::is_side),
        };
        let ring = self.jump_ring.entry(fid).or_default();
        if ring.len() >= Self::JUMP_RING_CAP {
            ring.remove(0);
        }
        ring.push(entry);
    }

    /// Drop one detached frontend's navigation trail (Q#BP11c).
    pub fn purge_jump_ring(&mut self, fid: FrontendId) {
        self.jump_ring.remove(&fid);
    }

    /// Pop the most recent jump origin and move there. Returns
    /// `true` if a jump was performed.
    ///
    /// Stale entries — a recorded buffer that has since been removed
    /// from the registry — are skipped (the loop keeps popping until
    /// it finds a live target or the ring empties), so a jump-back
    /// never lands on a missing buffer. The restored cursor is
    /// clamped to the (possibly now shorter) buffer length.
    ///
    /// # Origin windows (Q#BP11c)
    ///
    /// The entry restores into its **origin window** when that window is
    /// live, belongs to the acting frontend's layout, is not a hidden
    /// side window, and **still shows the recorded buffer**. A live panel
    /// that has since been replaced does not resurrect its old buffer.
    ///
    /// When revalidation fails the entry degrades differently by origin
    /// kind. A **non-side** origin falls back to today's active-window
    /// switch. A **side** origin is *skipped*: switching a panel's buffer
    /// into the document window is precisely the duplicate-presentation
    /// corruption this design removes.
    pub fn jump_back(&mut self) -> bool {
        let fid = self.active_frontend;
        loop {
            let Some(entry) = self.jump_ring.get_mut(&fid).and_then(std::vec::Vec::pop) else {
                return false;
            };
            if !self.registry.borrow().contains(entry.buffer_id) {
                continue;
            }
            let origin_valid = self
                .views
                .get(&fid)
                .is_some_and(|view| view.layout.iter_ids().contains(&entry.window_id))
                && self
                    .windows
                    .get(&entry.window_id)
                    .is_some_and(|window| window.buffer_id == entry.buffer_id)
                && !self.side_window_is_hidden(fid, entry.window_id);
            if origin_valid {
                // Through `focus_window`, not `set_active_window_id`:
                // returning INTO a panel from a document window is a
                // focus transition like any other, so it refreshes
                // `origin_document` and a later `window.quit` returns to
                // the window the jump came from.
                self.focus_window(fid, entry.window_id);
            } else {
                // A stale SIDE origin is skipped outright: switching a
                // panel's buffer into the document window is exactly the
                // duplicate-presentation corruption this design removes.
                // A stale non-side origin keeps today's active-window
                // fallback.
                if entry.side_origin {
                    continue;
                }
                if self.active_buffer_id() != entry.buffer_id
                    && self.switch_active_buffer(entry.buffer_id).is_err()
                {
                    continue;
                }
            }
            let clamped = entry.position.min(self.active_buffer_len());
            let aw = self.active_window_mut();
            aw.cursor = clamped;
            aw.goal_col = None;
            return true;
        }
    }

    /// True when `win` is a side window on `fid` and that frontend's
    /// panel is currently derived-hidden (Q#BP2b).
    #[must_use]
    fn side_window_is_hidden(&self, fid: FrontendId, win: WindowId) -> bool {
        self.windows
            .get(&win)
            .is_some_and(crate::window::Window::is_side)
            && self.views.get(&fid).is_some_and(|view| view.panel_hidden)
    }

    // ---- incremental search (Q#SR5) ----------------------------------------
    //
    // Frontend-agnostic isearch driven entirely through these methods.
    // The TUI's `dispatch_search_key` and (later) the GPU's round-tripped
    // keystrokes both call into here, so search behaves identically in
    // both frontends. Matches live in `search_store` (shared with the
    // decorations producer and the TUI overlay); `search` holds the
    // live query + origin.

    /// `true` iff an incremental search is in progress.
    #[must_use]
    pub fn search_active(&self) -> bool {
        self.search.is_some()
    }

    /// The current isearch query (empty when no search is running).
    #[must_use]
    pub fn search_query(&self) -> &str {
        self.search.as_ref().map_or("", |s| s.query.as_str())
    }

    /// Direction of the active search (`true` = forward). Defaults to
    /// forward when no search is running — callers should gate on
    /// [`Self::search_active`] first.
    #[must_use]
    pub fn search_forward(&self) -> bool {
        self.search.as_ref().is_none_or(|s| s.forward)
    }

    /// `true` iff the active search is in regex mode (Q#RX3). `false`
    /// for literal substring, or when no search is running.
    #[must_use]
    pub fn search_is_regex(&self) -> bool {
        self.search.as_ref().is_some_and(|s| s.regex)
    }

    /// `true` iff the active regex search's pattern failed to compile —
    /// the prompt shows `[invalid]` rather than a match count. Always
    /// `false` in literal mode / when no search is running.
    #[must_use]
    pub fn search_is_invalid(&self) -> bool {
        self.search.as_ref().is_some_and(|s| s.invalid)
    }

    /// `(active_index, total)` for the active buffer's matches, for the
    /// prompt's "n/m" readout. `active_index` is 0-based and `None`
    /// when there are no matches. Stale matches read as absent (Q#AI8
    /// fail-closed): the highlights they count are already suppressed,
    /// so the prompt must not advertise them either.
    #[must_use]
    pub fn search_match_summary(&self) -> (Option<usize>, usize) {
        let bid = self.active_buffer_id();
        let guard = self
            .search_store
            .lock()
            .expect("search store mutex poisoned");
        if guard.is_stale(bid) {
            return (None, 0);
        }
        guard
            .for_buffer(bid)
            .map_or((None, 0), |s| (s.active_index(), s.len()))
    }

    /// Begin an incremental search anchored at the active buffer +
    /// cursor. `forward` sets the initial step direction; `regex`
    /// selects regex (`true`) vs literal substring (`false`) matching.
    /// A no-op if a search is already running (the entry chord is
    /// intercepted while active, so this is only reached from an
    /// inactive state — the guard is belt-and-suspenders).
    pub fn search_begin(&mut self, forward: bool, regex: bool) {
        if self.search.is_some() {
            return;
        }
        let origin = (self.active_buffer_id(), self.cursor());
        // Attach the TUI match-wash overlay to the active window (once)
        // so matches highlight live as the query grows. It
        // self-suppresses when the store has no matches / is stale, so
        // leaving it attached across searches is safe. The GPU gets the
        // same matches via SearchMatch decorations and never reads this.
        self.ensure_search_overlay();
        self.search = Some(SearchSession {
            query: String::new(),
            origin,
            forward,
            regex,
            invalid: false,
        });
    }

    /// Toggle the active search between literal and regex matching
    /// (Q#RX3, `M-r`), re-running the current query in the new mode. A
    /// no-op when no search is running.
    pub fn search_toggle_regex(&mut self) {
        let Some(session) = self.search.as_mut() else {
            return;
        };
        session.regex = !session.regex;
        self.search_recompute();
    }

    /// Ensure the active window carries a [`crate::search::SearchView`]
    /// overlay, attaching one if absent (deduped by overlay kind). The
    /// view reads the per-buffer [`Self::search_store`] keyed on the
    /// rendered buffer, so one instance suffices per window.
    fn ensure_search_overlay(&mut self) {
        let store = self.search_store.clone();
        // Themes Q#TH9: pass the injected theme through unconditionally
        // — a bare core (None) constructs a working unthemed view.
        let theme = self.theme.clone();
        let win = self.active_window_mut();
        if !win.overlay_kinds().contains(&"search") {
            win.push_overlay(Box::new(crate::search::SearchView::new(store, theme)));
        }
    }

    /// Append a character to the query and re-search.
    pub fn search_input_char(&mut self, ch: char) {
        let Some(session) = self.search.as_mut() else {
            return;
        };
        session.query.push(ch);
        self.search_recompute();
    }

    /// Drop the last character of the query and re-search. With an
    /// empty query this is a no-op (the search stays open, empty).
    pub fn search_backspace(&mut self) {
        let Some(session) = self.search.as_mut() else {
            return;
        };
        session.query.pop();
        self.search_recompute();
    }

    /// Re-run `find_all` for the current query against the origin
    /// buffer, refresh the store, and move the cursor to the match
    /// nearest the origin (first match at/after the origin cursor,
    /// wrapping). An empty query or no match anchors the cursor back
    /// at the origin so a failing search never drifts the view.
    fn search_recompute(&mut self) {
        let Some(session) = self.search.as_ref() else {
            return;
        };
        let bid = session.origin.0;
        let origin_byte = session.origin.1;
        let query = session.query.clone();
        let regex = session.regex;
        let bytes = self.buffer_bytes(bid);
        // Regex: `None` ⇒ the pattern won't compile (mark invalid, drop
        // matches). Literal substring never fails. An invalid pattern
        // clears the store (no stale matches paint) and shows
        // `[invalid]` via the prompt.
        let (matches, invalid) = if regex {
            match crate::search::find_all_regex(&bytes, &query) {
                Some(m) => (m, false),
                None => (Vec::new(), true),
            }
        } else {
            (crate::search::find_all(&bytes, &query), false)
        };
        if let Some(session) = self.search.as_mut() {
            session.invalid = invalid;
        }
        let focus = {
            let mut guard = self
                .search_store
                .lock()
                .expect("search store mutex poisoned");
            guard.set(bid, query, matches);
            guard.focus_from(bid, origin_byte)
        };
        let target = focus.map_or(origin_byte, |range| range.start);
        self.search_place_cursor(target);
    }

    /// Step the active buffer's match focus forward/backward (wrapping)
    /// and move the cursor to it. Operates on [`Self::search_store`]
    /// directly, so it works both during a live session (C-s / C-r)
    /// and after accept (a `search.next` navigation command). A no-op
    /// when the active buffer has no matches.
    pub fn search_step(&mut self, forward: bool) {
        if let Some(session) = self.search.as_mut() {
            session.forward = forward;
        }
        let bid = self.active_buffer_id();
        let stepped = {
            let mut guard = self
                .search_store
                .lock()
                .expect("search store mutex poisoned");
            guard.step(bid, forward)
        };
        if let Some(range) = stepped {
            self.search_place_cursor(range.start);
        }
    }

    /// End the active search. `accept` keeps the cursor at the current
    /// match and leaves the matches in the store (so they stay
    /// highlighted, and `search.next` can resume, until the next edit
    /// marks them stale). Cancel restores the origin cursor and clears
    /// the matches. A no-op when no search is running.
    pub fn search_finish(&mut self, accept: bool) {
        let Some(session) = self.search.take() else {
            return;
        };
        if accept {
            return;
        }
        let (bid, origin_byte) = session.origin;
        if self.active_buffer_id() == bid {
            self.search_place_cursor(origin_byte);
        }
        self.search_store
            .lock()
            .expect("search store mutex poisoned")
            .clear(bid);
    }

    /// Move the active window's cursor to a byte offset (clamped to the
    /// buffer extent), resetting the goal column. Shared by the search
    /// motions.
    fn search_place_cursor(&mut self, byte: u64) {
        let clamped = byte.min(self.active_buffer_len());
        let aw = self.active_window_mut();
        aw.cursor = clamped;
        aw.goal_col = None;
    }

    // ---- query-replace (Arc 2, Q#QR1-10) -----------------------------------

    /// True while a query-replace interactive session is running (the
    /// fifth dispatcher-shadow predicate; also drives `dispatch_idle`
    /// and the modal-close guard).
    #[must_use]
    pub fn query_replace_active(&self) -> bool {
        self.query_replace.is_some()
    }

    /// The buffer a running query-replace is pinned to, or `None`. The
    /// dispatcher reads this so the `buffer.after-edit` revision compare
    /// targets the *edited* buffer, not whichever is active.
    #[must_use]
    pub fn query_replace_origin_buffer(&self) -> Option<BufferId> {
        self.query_replace.as_ref().map(|s| s.origin.0)
    }

    /// Query-replace's wrong-buffer guard. Every edit and cursor move it
    /// makes goes through the *active* window/buffer, but the session is
    /// pinned to the buffer it started in — and focus can drift
    /// mid-session (a click into another split, a key from another
    /// frontend). Before touching the buffer, verify the active buffer
    /// is still the origin buffer; if not, **abort without editing** so
    /// a match found in the origin buffer can never be applied to an
    /// unrelated one. Returns `true` when it is safe to proceed.
    fn query_replace_on_origin(&mut self) -> bool {
        let Some(origin_bid) = self.query_replace.as_ref().map(|s| s.origin.0) else {
            return false;
        };
        if self.active_buffer_id() == origin_bid {
            return true;
        }
        // Focus moved off the origin buffer — abort, don't corrupt.
        if let Some(session) = self.query_replace.take() {
            self.search_store
                .lock()
                .expect("search store mutex poisoned")
                .clear(session.origin.0);
        }
        self.status = "query-replace aborted: active buffer changed".into();
        false
    }

    /// Begin a query-replace from the cursor forward (Q#QR8). `regex`
    /// selects `query-replace-regexp` (Q#QR9). An invalid regex refuses
    /// to start (Q#QR2). Immediately advances to (and prompts on) the
    /// first match, or finishes with "No matches" when there are none.
    pub fn query_replace_begin(&mut self, from: String, to: String, regex: bool) {
        if self.query_replace.is_some() || from.is_empty() {
            return;
        }
        let re = if regex {
            let Some(re) = crate::search::compile_search_regex(&from) else {
                self.status = format!("Invalid regex: {from}");
                return;
            };
            Some(re)
        } else {
            None
        };
        let origin = (self.active_buffer_id(), self.cursor());
        // Reuse the isearch match-wash overlay to highlight the current
        // match in the TUI; the GPU gets it via SearchMatch decorations.
        self.ensure_search_overlay();
        self.query_replace = Some(QueryReplaceSession {
            from,
            to,
            re,
            origin,
            next_from: origin.1,
            current: None,
            replaced: 0,
            found_any: false,
        });
        self.query_replace_advance();
    }

    /// Find the next match at/after `next_from` on the live buffer. On
    /// a hit: highlight it, reveal it (cursor to its start, Q#QR2), and
    /// prompt. On a miss: finish (natural end / nothing-matched).
    fn query_replace_advance(&mut self) {
        let Some(session) = self.query_replace.as_ref() else {
            return;
        };
        let bid = session.origin.0;
        let bytes = self.buffer_bytes(bid);
        let start = (session.next_from as usize).min(bytes.len());
        let found = match &session.re {
            Some(re) => crate::search::find_first_regex_from(&bytes, re, start),
            None => crate::search::find_first_from(&bytes, &session.from, start),
        };
        let Some(range) = found else {
            self.query_replace_finish();
            return;
        };
        let from = session.from.clone();
        if let Some(session) = self.query_replace.as_mut() {
            session.current = Some(range);
            session.found_any = true;
        }
        // Highlight just this match: a single-element store set renders
        // it as SearchMatchActive in both frontends (Q#QR5).
        {
            let mut guard = self
                .search_store
                .lock()
                .expect("search store mutex poisoned");
            guard.set(bid, from, vec![range]);
        }
        self.search_place_cursor(range.start);
        self.query_replace_set_prompt();
    }

    /// Set `core.status` to the per-match prompt (Q#QR4) — shown in
    /// both frontends via the v15 `StatusFacts.message` band.
    fn query_replace_set_prompt(&mut self) {
        if let Some(session) = self.query_replace.as_ref() {
            self.status = format!(
                "Query replacing '{}' with '{}' (y/n, ! all, . last, q quit)",
                session.from, session.to
            );
        }
    }

    /// Replace the current match with the to-string as a single edit
    /// (Q#QR7), advancing `next_from` past the inserted text so it is
    /// never re-matched (Q#QR2). Returns `true` when an edit was
    /// applied. Does NOT advance to the next match — callers chain
    /// `query_replace_advance` (or finish) as their flow needs.
    fn query_replace_apply_current(&mut self) -> bool {
        let Some(session) = self.query_replace.as_ref() else {
            return false;
        };
        let Some(range) = session.current else {
            return false;
        };
        let to = session.to.clone();
        if let Err(e) = self.apply_active_edit(EditOp::Replace {
            range: Range {
                start: range.start,
                end: range.end,
            },
            bytes: to.as_bytes(),
        }) {
            self.status = format!("query-replace: {e}");
            return false;
        }
        let new_next = range.start + to.len() as u64;
        if let Some(session) = self.query_replace.as_mut() {
            session.next_from = new_next;
            session.current = None;
            session.replaced += 1;
        }
        self.search_place_cursor(new_next);
        true
    }

    /// `y` / `SPC` — replace the current match, then advance to the next.
    pub fn query_replace_replace(&mut self) {
        if self.query_replace_on_origin() && self.query_replace_apply_current() {
            self.query_replace_advance();
        }
    }

    /// `n` / `DEL` — leave the current match, advance past it to the next.
    pub fn query_replace_skip(&mut self) {
        if !self.query_replace_on_origin() {
            return;
        }
        if let Some(session) = self.query_replace.as_mut()
            && let Some(range) = session.current
        {
            session.next_from = range.end;
            session.current = None;
        }
        self.query_replace_advance();
    }

    /// `!` — replace the current match and all remaining without
    /// prompting, then finish (Q#QR6). One `after-edit` hook fires for
    /// the batch (the dispatcher compares revision across the handler).
    pub fn query_replace_all(&mut self) {
        if !self.query_replace_on_origin() {
            return;
        }
        while self.query_replace_apply_current() {
            // Find the next match (mirrors advance's search, without the
            // highlight/prompt work — we're not stopping to ask).
            let Some(session) = self.query_replace.as_ref() else {
                break;
            };
            let bid = session.origin.0;
            let bytes = self.buffer_bytes(bid);
            let start = (session.next_from as usize).min(bytes.len());
            let found = match &session.re {
                Some(re) => crate::search::find_first_regex_from(&bytes, re, start),
                None => crate::search::find_first_from(&bytes, &session.from, start),
            };
            match found {
                Some(range) => {
                    if let Some(session) = self.query_replace.as_mut() {
                        session.current = Some(range);
                    }
                }
                None => break,
            }
        }
        self.query_replace_finish();
    }

    /// `.` — replace the current match, then finish (Q#QR6).
    pub fn query_replace_replace_and_quit(&mut self) {
        if !self.query_replace_on_origin() {
            return;
        }
        self.query_replace_apply_current();
        self.query_replace_finish();
    }

    /// End the session (Q#QR10): clear the highlight, restore the origin
    /// cursor *only* if nothing ever matched, and set the count status.
    /// Every other exit leaves point where the last step put it.
    pub fn query_replace_finish(&mut self) {
        let Some(session) = self.query_replace.take() else {
            return;
        };
        let bid = session.origin.0;
        self.search_store
            .lock()
            .expect("search store mutex poisoned")
            .clear(bid);
        if session.found_any {
            let n = session.replaced;
            self.status = format!("Replaced {n} occurrence{}", if n == 1 { "" } else { "s" });
        } else {
            if self.active_buffer_id() == bid {
                self.search_place_cursor(session.origin.1);
            }
            self.status = format!("No matches for '{}'", session.from);
        }
    }

    /// Snapshot a buffer's full byte content (empty if the id is
    /// stale). O(1) rope snapshot + one copy; used to feed `find_all`.
    fn buffer_bytes(&self, buffer_id: BufferId) -> Vec<u8> {
        let reg = self.registry.borrow();
        let Ok(buf) = reg.get(buffer_id) else {
            return Vec::new();
        };
        let len = buf.len();
        let mut out = vec![0u8; len as usize];
        buf.snapshot_rope().slice(0, len, &mut out);
        out
    }

    // ---- editing primitives ------------------------------------------------

    /// Apply `op` to the active buffer; notify every window
    /// displaying that buffer. Returns the effective [`Edit`] — the
    /// post-intercept range and inserted length (auto-pairing Q#AP9
    /// needs the effective triple; every other caller reads
    /// `new_rope.len()` or discards it).
    ///
    /// # Errors
    ///
    /// Returns a stringified error on buffer or view failure.
    pub fn apply_active_edit(&mut self, op: EditOp<'_>) -> Result<Edit, String> {
        // Arc 6 Stage 2 (Q#FD19): ONE pre-edit unfold funnel for every
        // local point-anchored edit. This subsumes the six `dispatch_key`
        // primitives' individual calls (retired) and widens the behavior
        // to **yank** and **query-replace**, which reach the buffer
        // through here rather than through those primitives — both place
        // point at the edit site first, so keying on the active point
        // covers them exactly. `apply_active_edit` is never the
        // remote-apply path, so this funnel is inherently local: a remote
        // peer's edit inside my fold must not unfold it (Stage 3).
        self.unfold_before_point_edit();
        let buffer_id = self.active_buffer_id();
        // Scope the registry borrow: the origin translation below needs
        // `&mut self` after the views have been notified.
        let edit = {
            let mut reg = self.registry.borrow_mut();
            let buffer = reg.get_mut(buffer_id).map_err(|e| e.to_string())?;
            let edit = buffer.apply_edit(op).map_err(|e| e.to_string())?;
            for win in self.windows.values_mut() {
                if win.buffer_id == buffer_id {
                    let _ = win.text_view.on_edit(buffer, &edit);
                    for overlay in &mut win.overlays {
                        let _ = overlay.on_edit(buffer, &edit);
                    }
                }
            }
            edit
        };
        // T M10.8 Day 4 — capture CRDT op (if the buffer was in
        // CRDT mode and produced one) for the dispatcher to
        // broadcast on the next tick.
        //
        // M10.10 post-audit-round-3 F16: this is the **daemon-side**
        // mutation path (e.g. `FrontendEvent::Key` round-trip,
        // Lua-driven edit, fallback). The source frontend's mirror
        // has NOT applied this op locally; the queued origin is
        // [`CrdtOpOrigin::DaemonKey`] so the broadcast sweep includes
        // every replica (no sender exclusion).
        if let Some(crdt_op) = edit.crdt_op.as_ref() {
            self.pending_crdt_ops
                .push((CrdtOpOrigin::DaemonKey, buffer_id, (**crdt_op).clone()));
        }
        // Search matches were computed against the pre-edit text, so
        // their byte positions are now wrong (M11.8): the producer /
        // TUI overlay suppress them until a fresh search re-runs. The
        // headline isearch bet — "stale-after-edit linger" — is
        // closed here.
        self.search_invalidate_for_edit(buffer_id, &edit);
        Ok(edit)
    }

    /// Q#AI8 search invalidation for a landed edit: mark the buffer's
    /// matches stale (no-op without search state) and right-gravity-
    /// translate the live session origin. ONE helper so every edit
    /// path — dispatch ([`Self::apply_active_edit`]), direct
    /// notification ([`Self::notify_buffer_edit`]), and history
    /// ([`Self::undo`] / [`Self::redo`]) — invalidates identically;
    /// a path that skips this leaves highlights, step targets, and
    /// the n/m count pointing at pre-edit offsets.
    fn search_invalidate_for_edit(&mut self, buffer_id: BufferId, edit: &Edit) {
        self.search_store
            .lock()
            .expect("search store mutex poisoned")
            .mark_stale(buffer_id);
        self.translate_search_origin(buffer_id, edit);
    }

    /// Right-gravity-translate the live search origin through an edit
    /// to `buffer_id` (Q#AI8; the `src/daemon.rs` optimistic-arm
    /// shape). The origin is a raw byte offset captured at
    /// [`Self::search_begin`]; without translation an insert/delete
    /// before it skews every later recompute focus and the cancel
    /// restore, even when the match set itself is fresh.
    fn translate_search_origin(&mut self, buffer_id: BufferId, edit: &Edit) {
        let Some(session) = self.search.as_mut() else {
            return;
        };
        if session.origin.0 != buffer_id {
            return;
        }
        let start = edit.range.start;
        let end = edit.range.end;
        let pos = session.origin.1;
        session.origin.1 = if pos < start {
            pos
        } else if pos > end {
            pos - (end - start) + edit.inserted_len
        } else {
            start + edit.inserted_len
        };
    }

    /// Notify every window displaying `buffer_id` that the buffer was
    /// just edited externally — used by code paths that mutate a buffer
    /// without going through [`Self::apply_active_edit`] (the most
    /// notable one being [`crate::lua::LuaHost::append_to_errors_buffer`],
    /// which writes to `*errors*` from inside Lua callbacks).
    ///
    /// Without this notification, any window currently displaying the
    /// edited buffer would keep a stale [`crate::text_view::TextView`]
    /// line cache, causing later cursor motions to land at offsets the
    /// view cannot map back to display coordinates.
    ///
    /// Q#AI8: direct edits must also invalidate search state exactly
    /// like [`Self::apply_active_edit`] does — mark the matches stale
    /// and translate the live origin — otherwise accepted-match
    /// highlights and the session origin survive at pre-edit offsets
    /// for every Lua mutator edit and applied CRDT op.
    ///
    /// Q#GB6: each window coordinate is also clamped against **its own**
    /// post-edit bound, which [`Self::rebuild_views_for`] already does
    /// (`:1853-1857`) and this path did not. A generated refresh that
    /// shrinks its buffer otherwise leaves `cursor` past the end of the
    /// rope indefinitely — neither paint nor a motion command recovers
    /// it, because motion is computed from the stale value. The two
    /// coordinates fail on different axes and are therefore clamped
    /// separately and **unconditionally**: `cursor` is a byte position
    /// bounded by [`Buffer::len`], while `view_top` is a line index
    /// bounded by [`TextView::line_count`]. A replace can grow in bytes
    /// while collapsing many lines into one, so "the buffer shrank" is
    /// not a usable trigger for the second.
    ///
    /// The selection anchor is a **third** coordinate. It is clamped to
    /// the buffer extent, and the selection is cleared only when moving
    /// an endpoint collapses it — see
    /// [`Self::clamp_cursor_and_selection`].
    pub fn notify_buffer_edit(&mut self, buffer_id: BufferId, edit: &Edit) {
        self.search_invalidate_for_edit(buffer_id, edit);
        let reg = self.registry.borrow();
        let Ok(buffer) = reg.get(buffer_id) else {
            return;
        };
        let len = buffer.len();
        for win in self.windows.values_mut() {
            if win.buffer_id == buffer_id {
                let _ = win.text_view.on_edit(buffer, edit);
                for overlay in &mut win.overlays {
                    let _ = overlay.on_edit(buffer, edit);
                }
                Self::clamp_cursor_and_selection(win, len);
                let max_top = win.text_view.line_count().saturating_sub(1);
                if win.view_top > max_top {
                    win.view_top = max_top;
                }
            }
        }
    }

    /// Clamp `win`'s cursor and selection anchor to `len`, clearing the
    /// selection only when the clamp collapses it.
    ///
    /// This mirrors terminal selection normalization: a surviving,
    /// shortened region remains selected, while a region whose content
    /// disappeared does not become an accidental active-but-empty
    /// selection. Looking at whether either endpoint moved distinguishes
    /// that case from a zero-width selection the user created.
    fn clamp_cursor_and_selection(win: &mut Window, len: Position) {
        let old_cursor = win.cursor;
        win.cursor = win.cursor.min(len);
        win.selection = win.selection.and_then(|mut selection| {
            let old_anchor = selection.anchor;
            selection.anchor = selection.anchor.min(len);
            let collapsed_by_clamp = selection.anchor == win.cursor
                && (selection.anchor != old_anchor || win.cursor != old_cursor);
            (!collapsed_by_clamp).then_some(selection)
        });
    }

    /// Force every window currently showing `buffer_id` to rebuild
    /// its [`TextView`] from scratch.
    ///
    /// Used by code paths that rewrite a buffer end-to-end without
    /// emitting a useful [`Edit`] (the help renderer issues a
    /// delete-all + insert pair on `*help*`; `*buffer-list*` is
    /// regenerated from scratch on every C-x C-b). Calling
    /// [`Self::notify_buffer_edit`] for each step works but is more
    /// fiddly; rebuild is simpler and still O(buffer length) which is
    /// what an end-to-end rewrite cost anyway.
    ///
    /// Cursor and `view_top` are clamped to the new buffer extent so
    /// they don't dangle past the end after a shrinking rewrite.
    /// Selection normalization uses the same clamp-or-clear rule as
    /// [`Self::notify_buffer_edit`].
    pub fn rebuild_views_for(&mut self, buffer_id: BufferId) {
        let reg = self.registry.borrow();
        let Ok(buffer) = reg.get(buffer_id) else {
            return;
        };
        let len = buffer.len();
        for win in self.windows.values_mut() {
            if win.buffer_id == buffer_id {
                win.text_view = TextView::new(buffer);
                Self::clamp_cursor_and_selection(win, len);
                let max_top = win.text_view.line_count().saturating_sub(1);
                if win.view_top > max_top {
                    win.view_top = max_top;
                }
            }
        }
    }

    /// Save the active buffer to its backing file. Returns `true` on
    /// successful write; `false` if no path is associated, the buffer
    /// could not be read, or the atomic save failed. Callers (the
    /// `buffer.save` Lua command) use the return value to gate
    /// `buffer.after-save` firing.
    pub fn save(&mut self) -> bool {
        self.save_inner(false)
    }

    /// [`save`](Self::save), overwriting the file even though it changed on
    /// disk since this buffer read it. The escape hatch for when the user
    /// has looked and decided their buffer wins.
    pub fn save_ignoring_disk_changes(&mut self) -> bool {
        self.save_inner(true)
    }

    /// True when writing this buffer to `path` would destroy content the
    /// buffer has never seen — i.e. a file exists there whose identity
    /// differs from the [`FileMeta`] recorded when the buffer last read or
    /// wrote it.
    ///
    /// Two cases count as "changed":
    ///
    /// * the buffer recorded a meta and the on-disk meta differs — someone
    ///   else edited the file (another editor, a `git checkout`);
    /// * the buffer recorded **no** meta (a `[new file]`, or a buffer whose
    ///   path was set without reading) yet a file now exists — it was
    ///   created underneath us, and we have never seen its contents.
    ///
    /// A **missing** file is not a clobber: there is nothing there to
    /// destroy, so recreating a deleted file saves normally. An unstattable
    /// path likewise falls through, and `save_atomic` reports the real
    /// error.
    #[must_use]
    pub fn save_would_clobber(&self, id: BufferId, path: &Path) -> bool {
        let Ok(current) = crate::file_io::current_meta(path) else {
            return false; // absent, or we cannot stat it
        };
        let reg = self.registry.borrow();
        let Ok(buffer) = reg.get(id) else {
            return false;
        };
        buffer.file_meta() != Some(&current)
    }

    fn save_inner(&mut self, force: bool) -> bool {
        let id = self.active_buffer_id();
        let Some(path) = self.active_buffer_path() else {
            self.status = "no file (M1: open a file from argv)".into();
            return false;
        };
        // Refuse to silently overwrite a file that changed underneath us.
        // Without this, pmacs clobbers another editor's (or a `git
        // checkout`'s) writes with a buffer that never saw them.
        if !force && self.save_would_clobber(id, &path) {
            self.status = format!(
                "{} changed on disk since it was read --- M-x buffer.save-anyway to overwrite",
                path.display()
            );
            return false;
        }
        let len_and_bytes = {
            let reg = self.registry.borrow();
            let buffer = match reg.get(id) {
                Ok(b) => b,
                Err(e) => {
                    self.status = format!("save failed: {e}");
                    return false;
                }
            };
            let len = buffer.len();
            let mut content = vec![0u8; len as usize];
            if len > 0 {
                buffer.snapshot_rope().slice(0, len, &mut content);
            }
            (len, content)
        };
        let (_, content) = len_and_bytes;
        match save_atomic(&path, &content) {
            Ok(meta) => {
                if let Ok(buf) = self.registry.borrow_mut().get_mut(id) {
                    buf.set_file_meta(Some(meta));
                    buf.mark_clean();
                }
                self.status = format!("saved {}", path.display());
                true
            }
            Err(e) => {
                self.status = format!("save failed: {e}");
                false
            }
        }
    }

    /// Move the cursor by one codepoint to the left. No-op at start.
    pub fn move_left(&mut self) {
        let cursor = self.active_window().cursor;
        if cursor == 0 {
            self.active_window_mut().goal_col = None;
            return;
        }
        let new = {
            let id = self.active_buffer_id();
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            prev_codepoint(buffer, cursor)
        };
        let aw = self.active_window_mut();
        aw.cursor = new;
        aw.goal_col = None;
    }

    /// Move the cursor by one codepoint to the right. No-op at end.
    pub fn move_right(&mut self) {
        let cursor = self.active_window().cursor;
        let new = {
            let id = self.active_buffer_id();
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            if cursor >= buffer.len() {
                cursor
            } else {
                next_codepoint(buffer, cursor)
            }
        };
        let aw = self.active_window_mut();
        aw.cursor = new;
        aw.goal_col = None;
    }

    /// Move the cursor up one line, preserving display column.
    ///
    /// Arc 6 Stage 2 (Q#FD17, ruled: include): on a fold-projecting
    /// frontend this steps to the previous **visible** line, so a
    /// collapsed region is one motion step and the cursor never comes to
    /// rest hidden. Motion that *begins* from a hidden logical cursor (a
    /// shared fold, or goto-line into one) first normalizes to the
    /// visible head. Scoped by Q#FD21 — a semantic frontend keeps
    /// raw-line motion until Stage 3.
    pub fn move_up(&mut self) {
        let folds = self.fold_map_active();
        // Normalize FIRST and as a real mutation (round-4 F2): a hidden
        // logical cursor moves to its component's head POSITION — head
        // row *and* head end-of-content column — before any step is
        // computed. Deriving the goal column from the hidden line would
        // carry a column the head may not even have, and returning early
        // at a buffer boundary (a fold headed on line 0) would leave the
        // cursor hidden entirely.
        self.normalize_cursor_to_visible(folds.as_ref());
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let ctx = self.layout_ctx_active();
        let goal_col = self.active_window().goal_col;
        let result = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            let aw = self.active_window();
            let coord = aw
                .text_view
                .pos_to_display(buffer, cursor, ctx)
                .unwrap_or_default();
            let from_row = coord.row as usize;
            if from_row == 0 {
                return;
            }
            let target_row = folds
                .as_ref()
                .map_or(from_row - 1, |map| map.prev_visible(from_row));
            let goal = goal_col.unwrap_or(coord.col);
            let Ok(target_row) = u32::try_from(target_row) else {
                return;
            };
            let target = DisplayCoord::new(target_row, goal);
            let new_pos = aw.text_view.display_to_pos(buffer, target, ctx);
            (goal, new_pos)
        };
        let (goal, new_pos) = result;
        let aw = self.active_window_mut();
        aw.goal_col = Some(goal);
        if let Some(p) = new_pos {
            aw.cursor = p;
        }
    }

    /// Move the cursor down one line, preserving display column.
    /// Visible-line stepping mirrors [`Self::move_up`] (Q#FD17/FD21).
    pub fn move_down(&mut self) {
        let folds = self.fold_map_active();
        self.normalize_cursor_to_visible(folds.as_ref());
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let ctx = self.layout_ctx_active();
        let goal_col = self.active_window().goal_col;
        let result = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            let aw = self.active_window();
            let coord = aw
                .text_view
                .pos_to_display(buffer, cursor, ctx)
                .unwrap_or_default();
            let from_row = coord.row as usize;
            let next_row = folds
                .as_ref()
                .map_or(from_row + 1, |map| map.next_visible(from_row));
            if next_row >= aw.text_view.line_count() {
                return;
            }
            let goal = goal_col.unwrap_or(coord.col);
            let Ok(next_row) = u32::try_from(next_row) else {
                return;
            };
            let target = DisplayCoord::new(next_row, goal);
            let new_pos = aw.text_view.display_to_pos(buffer, target, ctx);
            (goal, new_pos)
        };
        let (goal, new_pos) = result;
        let aw = self.active_window_mut();
        aw.goal_col = Some(goal);
        if let Some(p) = new_pos {
            aw.cursor = p;
        }
    }

    /// Move to the start of the current line.
    pub fn move_line_start(&mut self) {
        let cursor = self.active_window().cursor;
        let new = {
            let aw = self.active_window();
            let line = aw.text_view.line_at_offset(cursor);
            aw.text_view.line_offset(line).unwrap_or(cursor)
        };
        let aw = self.active_window_mut();
        aw.cursor = new;
        aw.goal_col = None;
    }

    /// Move the cursor forward by one word.
    ///
    /// Skips runs of non-word characters, then a run of word
    /// characters. Word characters are alphanumerics plus `_`, the
    /// Emacs default. Multi-byte characters are handled correctly:
    /// `is_word` runs after a full UTF-8 codepoint is decoded.
    pub fn move_word_right(&mut self) {
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let new = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            forward_word(buffer, cursor)
        };
        let aw = self.active_window_mut();
        aw.cursor = new;
        aw.goal_col = None;
    }

    /// Move the cursor backward by one word. Mirror of
    /// [`Self::move_word_right`].
    pub fn move_word_left(&mut self) {
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let new = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            backward_word(buffer, cursor)
        };
        let aw = self.active_window_mut();
        aw.cursor = new;
        aw.goal_col = None;
    }

    /// Select the word at the active cursor. Returns `false` when the
    /// cursor is not on a word character.
    pub fn select_word_at_cursor(&mut self) -> bool {
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let range = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else {
                return false;
            };
            word_range_at(buffer, cursor)
        };
        let Some((start, end)) = range else {
            return false;
        };
        let aw = self.active_window_mut();
        aw.selection = Some(crate::window::Selection { anchor: start });
        aw.cursor = end;
        aw.goal_col = None;
        true
    }

    /// Select the whole line at the active cursor, trailing newline
    /// included — the convention that makes consecutive triple-click
    /// lines abut (Q#M4). The cursor lands at the selection end (the
    /// start of the next line). No-op when the buffer is gone.
    pub fn select_line_at_cursor(&mut self) {
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let (start, end) = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else {
                return;
            };
            let view = &self.active_window().text_view;
            let line = view.line_at_offset(cursor);
            let start = view.line_offset(line).unwrap_or(0);
            let end = view.line_offset(line + 1).unwrap_or_else(|| buffer.len());
            (start, end)
        };
        let aw = self.active_window_mut();
        aw.selection = Some(crate::window::Selection { anchor: start });
        aw.cursor = end;
        aw.goal_col = None;
    }

    /// Move the cursor forward to the next paragraph break.
    ///
    /// A paragraph break is a blank line (empty or whitespace-only).
    /// If the cursor is currently in a paragraph, the cursor lands at
    /// the start of the first blank line after it. If the cursor is
    /// already on a blank line, blanks are skipped first, then the
    /// next blank line is found. Lands at the end of the buffer when
    /// there are no further paragraph breaks. Mirrors GNU Emacs's
    /// (and Doom's) `forward-paragraph` semantics.
    pub fn move_paragraph_down(&mut self) {
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let new = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            let aw = self.active_window();
            forward_paragraph(buffer, &aw.text_view, cursor)
        };
        let aw = self.active_window_mut();
        aw.cursor = new;
        aw.goal_col = None;
    }

    /// Move the cursor backward to the previous paragraph break.
    /// Mirror of [`Self::move_paragraph_down`].
    pub fn move_paragraph_up(&mut self) {
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let new = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            let aw = self.active_window();
            backward_paragraph(buffer, &aw.text_view, cursor)
        };
        let aw = self.active_window_mut();
        aw.cursor = new;
        aw.goal_col = None;
    }

    /// Move the cursor down by approximately one screenful, scrolling
    /// `view_top` to match. The step is the active window's last
    /// rendered viewport height minus one (so the user keeps a line
    /// of context); falls back to a sane default before the first
    /// frame has rendered.
    pub fn move_page_down(&mut self) {
        // Arc 6 Stage 2 (Q#FD18/FD21): a screenful is a screenful of
        // VISIBLE lines, and `view_top` never lands hidden. Paging shares
        // vertical motion's hidden-cursor normalization (round-4 F2) —
        // the goal column must come from the head, not the hidden line.
        let folds = self.fold_map_active();
        self.normalize_cursor_to_visible(folds.as_ref());
        let step = self.page_step();
        let cursor = self.active_window().cursor;
        let ctx = self.layout_ctx_active();
        let view_top = self.active_window().view_top;
        let id = self.active_buffer_id();
        let result = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            let aw = self.active_window();
            let coord = aw
                .text_view
                .pos_to_display(buffer, cursor, ctx)
                .unwrap_or_default();
            let max_line = aw.text_view.line_count().saturating_sub(1);
            let goal_col = aw.goal_col.unwrap_or(coord.col);
            let (target_row, new_top) = match folds.as_ref() {
                Some(map) => (
                    map.nth_visible_from(coord.row as usize, step as usize)
                        .min(map.visible_head_of(max_line)),
                    map.nth_visible_from(view_top, step as usize),
                ),
                None => (
                    (coord.row as usize + step as usize).min(max_line),
                    view_top.saturating_add(step as usize),
                ),
            };
            let Ok(target_row) = u32::try_from(target_row) else {
                return;
            };
            let target = DisplayCoord::new(target_row, goal_col);
            let new_pos = aw.text_view.display_to_pos(buffer, target, ctx);
            (goal_col, new_pos, new_top)
        };
        let (goal, new_pos, new_top) = result;
        let aw = self.active_window_mut();
        aw.goal_col = Some(goal);
        if let Some(p) = new_pos {
            aw.cursor = p;
        }
        // Also nudge view_top; render's scroll-into-view will clamp
        // and align further if needed.
        let max_top = aw.text_view.line_count().saturating_sub(1);
        let clamped = new_top.min(max_top);
        aw.view_top = folds
            .as_ref()
            .map_or(clamped, |map| map.clamp_view_top(clamped));
    }

    /// Move the cursor up by approximately one screenful. Mirror of
    /// [`Self::move_page_down`].
    pub fn move_page_up(&mut self) {
        let folds = self.fold_map_active();
        self.normalize_cursor_to_visible(folds.as_ref());
        let step = self.page_step();
        let cursor = self.active_window().cursor;
        let ctx = self.layout_ctx_active();
        let view_top = self.active_window().view_top;
        let id = self.active_buffer_id();
        let result = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            let aw = self.active_window();
            let coord = aw
                .text_view
                .pos_to_display(buffer, cursor, ctx)
                .unwrap_or_default();
            let goal_col = aw.goal_col.unwrap_or(coord.col);
            let (target_row, new_top) = match folds.as_ref() {
                Some(map) => (
                    map.nth_visible_back(coord.row as usize, step as usize),
                    map.nth_visible_back(view_top, step as usize),
                ),
                None => (
                    (coord.row as usize).saturating_sub(step as usize),
                    view_top.saturating_sub(step as usize),
                ),
            };
            let Ok(target_row) = u32::try_from(target_row) else {
                return;
            };
            let target = DisplayCoord::new(target_row, goal_col);
            let new_pos = aw.text_view.display_to_pos(buffer, target, ctx);
            (goal_col, new_pos, new_top)
        };
        let (goal, new_pos, new_top) = result;
        let aw = self.active_window_mut();
        aw.goal_col = Some(goal);
        if let Some(p) = new_pos {
            aw.cursor = p;
        }
        aw.view_top = new_top;
    }

    /// Number of lines a "page" advances. Uses the active window's
    /// last rendered viewport height minus one (one line of context
    /// at the seam, like Emacs's `next-screen-context-lines`),
    /// clamped to a sensible default for headless tests where no
    /// frame has rendered.
    fn page_step(&self) -> u32 {
        const DEFAULT_PAGE: u32 = 20;
        let rows = self.active_window().last_visible_rows;
        if rows >= 2 { rows - 1 } else { DEFAULT_PAGE }
    }

    /// Move to the end of the current line (before any trailing newline).
    pub fn move_line_end(&mut self) {
        let id = self.active_buffer_id();
        let cursor = self.active_window().cursor;
        let new = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            let aw = self.active_window();
            let line = aw.text_view.line_at_offset(cursor);
            let Some(start) = aw.text_view.line_offset(line) else {
                return;
            };
            let len = aw.text_view.line_len(buffer, line).unwrap_or(0);
            start + len
        };
        let aw = self.active_window_mut();
        aw.cursor = new;
        aw.goal_col = None;
    }

    /// Pre-edit unfold (Arc 6, Q#FD5 / Stage 2 Q#FD19). Before a local
    /// point-anchored edit, unfold every fold containing the active point
    /// so an edit inside a collapsed region reveals it rather than
    /// landing invisibly. Keyed on the authenticated source frontend's
    /// active point (`active_window().cursor`), not the transport. A
    /// no-op when the buffer has no folds.
    ///
    /// **Stage 2 widening:** Stage 1 called this from each of the six
    /// `dispatch_key` edit primitives. It now runs once at the top of
    /// [`Self::apply_active_edit`] — the single funnel those primitives
    /// (and yank, and query-replace) all pass through. Interactive
    /// Lua-mutator edits (comment-toggle, yank-pop) take a *different*
    /// path and are hooked at `run_buffer_edit` in the Lua bindings; the
    /// remote/optimistic-CRDT apply path is deliberately excluded
    /// (Stage 3), as is undo/redo (deferred).
    /// Move a hidden logical cursor to its component's visible head
    /// **position** — the head row *and* that head's end-of-content
    /// column, i.e. exactly where Stage 1 moves point on a fold-at-cursor
    /// (Q#FD16/Q#FD17, round-4 F2).
    ///
    /// A cursor can be hidden without this frontend ever having moved it
    /// there: another frontend folds through the shared store, or
    /// goto-line targets a line inside a collapse. Vertical motion and
    /// paging normalize through here *before* computing their step, so
    /// the goal column is the head's — never a column of a line that
    /// renders no row — and so a step that turns out to be a no-op at a
    /// buffer boundary still leaves the cursor visible.
    ///
    /// The jump is discontinuous, so the sticky goal column is dropped
    /// (the click/goto convention), and only when the cursor actually
    /// was hidden: ordinary motion keeps its sticky column untouched.
    fn normalize_cursor_to_visible(&mut self, folds: Option<&crate::fold_view::VisibleLineMap>) {
        let Some(map) = folds else { return };
        let aw = self.active_window();
        let line = aw.text_view.line_at_offset(aw.cursor);
        let projected = map.visible_position(line, aw.cursor);
        if projected != aw.cursor {
            let aw = self.active_window_mut();
            aw.cursor = projected;
            aw.goal_col = None;
        }
    }

    fn unfold_before_point_edit(&self) {
        let id = self.active_buffer_id();
        let point = self.active_window().cursor;
        self.fold_registry.unfold_containing(id, point);
    }

    /// Insert a single character at the cursor. Returns `true` iff the
    /// edit landed: a rejecting buffer intercept reports via the status
    /// line and returns `false`, and callers must not mutate dependent
    /// state (e.g. selection anchors) on a failed insert (Q#AI9).
    pub fn insert_char(&mut self, ch: char) -> bool {
        self.active_window_mut().goal_col = None;
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        let bytes = s.as_bytes();
        let pos = self.active_window().cursor;
        // Q#AP9: the buffer/window the request was made in, captured
        // BEFORE the edit — a legal intercept may switch the active
        // context mid-edit, and the record must name where the
        // self-insert actually landed, not where the intercept went.
        let (buffer_id, window_id) = (self.active_buffer_id(), self.active_window_id());
        let edit = match self.apply_active_edit(EditOp::Insert { pos, bytes }) {
            Ok(edit) => edit,
            Err(e) => {
                self.status = format!("insert failed: {e}");
                return false;
            }
        };
        self.active_window_mut().cursor += bytes.len() as u64;
        self.typed_edit_complete(
            ch,
            (buffer_id, window_id),
            Range::new(pos, pos),
            bytes.len() as u64,
            &edit,
        );
        true
    }

    /// CUA type-over: insert `ch`, replacing the active region if one
    /// exists. With a region this is a *single* `EditOp::Replace` — one
    /// undo step — rather than the former `delete_region()` +
    /// `insert_char()` pair, which recorded two. With no region it
    /// delegates to [`Self::insert_char`] (a plain insert). The cursor
    /// lands just past the inserted bytes and any selection is cleared.
    pub fn insert_char_over_region(&mut self, ch: char) {
        let Some((lo, hi)) = self.active_region() else {
            // Q#AI9: an empty selection (anchor == cursor) reports no
            // region yet stays armed — the insert moves the cursor off
            // the anchor and the very NEXT key type-overs the fresh
            // text. Clear it, but only when the edit landed: a
            // rejecting intercept must leave the anchor untouched.
            if self.insert_char(ch) {
                self.active_window_mut().selection = None;
            }
            return;
        };
        self.active_window_mut().goal_col = None;
        let mut buf = [0u8; 4];
        let bytes = ch.encode_utf8(&mut buf).as_bytes();
        // Q#AP9: capture the request's context before the edit (see
        // the twin comment in [`Self::insert_char`]).
        let (buffer_id, window_id) = (self.active_buffer_id(), self.active_window_id());
        let edit = match self.apply_active_edit(EditOp::Replace {
            range: Range { start: lo, end: hi },
            bytes,
        }) {
            Ok(edit) => edit,
            Err(e) => {
                self.status = format!("replace failed: {e}");
                return;
            }
        };
        let aw = self.active_window_mut();
        aw.cursor = lo + bytes.len() as u64;
        aw.selection = None;
        self.typed_edit_complete(
            ch,
            (buffer_id, window_id),
            Range::new(lo, hi),
            bytes.len() as u64,
            &edit,
        );
    }

    /// Delete the codepoint immediately before the cursor.
    pub fn backspace(&mut self) {
        self.active_window_mut().goal_col = None;
        let cursor = self.active_window().cursor;
        if cursor == 0 {
            return;
        }
        let prev = {
            let id = self.active_buffer_id();
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            prev_codepoint(buffer, cursor)
        };
        let range = Range::new(prev, cursor);
        if let Err(e) = self.apply_active_edit(EditOp::Delete { range }) {
            self.status = format!("delete failed: {e}");
            return;
        }
        self.active_window_mut().cursor = prev;
    }

    /// Delete the codepoint at the cursor (forward delete).
    pub fn delete_forward(&mut self) {
        self.active_window_mut().goal_col = None;
        let cursor = self.active_window().cursor;
        let id = self.active_buffer_id();
        let next = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            if cursor >= buffer.len() {
                return;
            }
            next_codepoint(buffer, cursor)
        };
        let range = Range::new(cursor, next);
        if let Err(e) = self.apply_active_edit(EditOp::Delete { range }) {
            self.status = format!("delete failed: {e}");
        }
    }

    /// Delete from the cursor backward to the start of the previous
    /// word. The CUA-style `Ctrl+Backspace`. No-op at start-of-buffer.
    /// Mirrors [`Self::backspace`] but the deleted range is the gap
    /// between the cursor and where [`Self::move_word_left`] would
    /// land.
    pub fn delete_word_backward(&mut self) {
        self.active_window_mut().goal_col = None;
        let cursor = self.active_window().cursor;
        if cursor == 0 {
            return;
        }
        let new = {
            let id = self.active_buffer_id();
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            backward_word(buffer, cursor)
        };
        if new == cursor {
            return;
        }
        let range = Range::new(new, cursor);
        if let Err(e) = self.apply_active_edit(EditOp::Delete { range }) {
            self.status = format!("delete failed: {e}");
            return;
        }
        self.active_window_mut().cursor = new;
    }

    /// Delete from the cursor forward to the end of the next word. The
    /// CUA-style `Ctrl+Delete`. No-op at end-of-buffer. Mirrors
    /// [`Self::delete_forward`] over the gap from the cursor to where
    /// [`Self::move_word_right`] would land.
    pub fn delete_word_forward(&mut self) {
        self.active_window_mut().goal_col = None;
        let cursor = self.active_window().cursor;
        let id = self.active_buffer_id();
        let new = {
            let reg = self.registry.borrow();
            let Ok(buffer) = reg.get(id) else { return };
            if cursor >= buffer.len() {
                return;
            }
            forward_word(buffer, cursor)
        };
        if new == cursor {
            return;
        }
        let range = Range::new(cursor, new);
        if let Err(e) = self.apply_active_edit(EditOp::Delete { range }) {
            self.status = format!("delete failed: {e}");
        }
    }

    /// Undo the most recent edit on the active buffer; clamp the
    /// active window's cursor to the new length and notify all
    /// windows on this buffer.
    pub fn undo(&mut self) {
        self.active_window_mut().goal_col = None;
        let buffer_id = self.active_buffer_id();
        let edit = {
            let mut reg = self.registry.borrow_mut();
            let Ok(buffer) = reg.get_mut(buffer_id) else {
                return;
            };
            buffer.undo()
        };
        match edit {
            Ok(edit) => {
                let reg = self.registry.borrow();
                let Ok(buffer) = reg.get(buffer_id) else {
                    return;
                };
                for win in self.windows.values_mut() {
                    if win.buffer_id == buffer_id {
                        let _ = win.text_view.on_edit(buffer, &edit);
                        let max = buffer.len();
                        if win.cursor > max {
                            win.cursor = max;
                        }
                    }
                }
                drop(reg);
                // Q#AI8 (PR #109 round 1): history edits move bytes
                // like any other edit — invalidate search state.
                self.search_invalidate_for_edit(buffer_id, &edit);
                // Post-audit-round-5 F27: undo on a CRDT-backed
                // buffer produces a crdt_op that must broadcast to
                // every replica frontend (including the one whose
                // command triggered the undo — its BufferMirror has
                // no other way to converge with the post-undo state).
                self.queue_daemon_origin_crdt_op(buffer_id, &edit);
            }
            Err(_) => self.status = "nothing to undo".into(),
        }
    }

    /// Redo the most recently undone edit on the active buffer.
    pub fn redo(&mut self) {
        self.active_window_mut().goal_col = None;
        let buffer_id = self.active_buffer_id();
        let edit = {
            let mut reg = self.registry.borrow_mut();
            let Ok(buffer) = reg.get_mut(buffer_id) else {
                return;
            };
            buffer.redo()
        };
        match edit {
            Ok(edit) => {
                let reg = self.registry.borrow();
                let Ok(buffer) = reg.get(buffer_id) else {
                    return;
                };
                for win in self.windows.values_mut() {
                    if win.buffer_id == buffer_id {
                        let _ = win.text_view.on_edit(buffer, &edit);
                        let max = buffer.len();
                        if win.cursor > max {
                            win.cursor = max;
                        }
                    }
                }
                drop(reg);
                // Q#AI8 — same as undo above.
                self.search_invalidate_for_edit(buffer_id, &edit);
                // Post-audit-round-5 F27 — same as undo above.
                self.queue_daemon_origin_crdt_op(buffer_id, &edit);
            }
            Err(_) => self.status = "nothing to redo".into(),
        }
    }

    /// T M10.10 post-audit-round-5 F27 + F28 — queue a CRDT op
    /// produced by a daemon-origin edit (undo/redo via core, Lua
    /// bindings, command pipeline) for broadcast.
    ///
    /// Pushes into `pending_crdt_ops` with
    /// [`CrdtOpOrigin::DaemonKey`] semantics: the broadcast sweep
    /// includes every replica frontend (no sender exclusion). The
    /// originating frontend's `BufferMirror` has not applied the op
    /// locally — only the daemon's authoritative buffer has — so
    /// the source's mirror needs the broadcast just like every
    /// other replica.
    ///
    /// No-op when the edit doesn't carry a `crdt_op` (the buffer
    /// wasn't CRDT-backed at the time of the edit). Callers can
    /// invoke this unconditionally after any daemon-origin
    /// `apply_*` that returns an `Edit`; non-CRDT buffers pay no
    /// cost beyond the early return.
    pub fn queue_daemon_origin_crdt_op(&mut self, buffer_id: BufferId, edit: &Edit) {
        if let Some(crdt_op) = edit.crdt_op.as_ref() {
            self.pending_crdt_ops
                .push((CrdtOpOrigin::DaemonKey, buffer_id, (**crdt_op).clone()));
        }
    }

    // ---- window operations -------------------------------------------------

    /// [`Self::split_active`], refusing a side window (Q#BP6): the panel
    /// is a leaf of the root-level wrapper, so splitting it would produce
    /// a second, unallocatable side slot.
    ///
    /// # Errors
    /// When the active window is a side window.
    pub fn try_split_active(
        &mut self,
        orientation: Orientation,
        same_buffer: bool,
    ) -> Result<WindowId, String> {
        if self
            .windows
            .get(&self.active_window_id())
            .is_some_and(crate::window::Window::is_side)
        {
            return Err("window.split: not available in a side window".into());
        }
        Ok(self.split_active(orientation, same_buffer))
    }

    /// Split the active window. Returns the new window's id.
    /// `same_buffer` controls whether the new window opens on the
    /// active buffer (Emacs default) or a fresh `*scratch*` buffer.
    pub fn split_active(&mut self, orientation: Orientation, same_buffer: bool) -> WindowId {
        let active_buf = self.active_buffer_id();
        let (buffer_id, text_view) = if same_buffer {
            let reg = self.registry.borrow();
            let buf = reg.get(active_buf).expect("active buffer present");
            (active_buf, TextView::new(buf))
        } else {
            let mut reg = self.registry.borrow_mut();
            let new_id = reg.create("*scratch*");
            let buf = reg.get(new_id).unwrap();
            (new_id, TextView::new(buf))
        };
        let new_id = WindowId::next();
        let mut new_window = Window::new(new_id, buffer_id, text_view);
        let active = self.active_window_id();
        // A same-buffer split starts from an empty overlay list and
        // fires no switch hook, so store-backed render overlays
        // (ANSI styling on a compile buffer) would silently vanish
        // from the new pane (PR #113 round-6 finding 1). Views that
        // carry across splits say so via `clone_for_split`.
        if same_buffer && let Some(src) = self.windows.get(&active) {
            for overlay in &src.overlays {
                if let Some(copy) = overlay.clone_for_split() {
                    new_window.overlays.push(copy);
                }
            }
        }
        self.windows.insert(new_id, new_window);
        self.active_layout_mut()
            .split_window(active, orientation, new_id);
        new_id
    }

    /// Move focus to the next window in iteration order.
    pub fn focus_next(&mut self) {
        self.focus_step(true);
    }

    /// Move focus to the previous window in iteration order.
    pub fn focus_prev(&mut self) {
        self.focus_step(false);
    }

    /// Shared `C-x o` traversal, skipping a **hidden** side window
    /// (Q#BP6): keys must never route to an invisible panel, and once it
    /// reappears traversal reaches it normally again.
    ///
    /// Also the seam that refreshes `origin_document` (Q#BP2c): entering
    /// the panel from document window B must retarget `display_target`,
    /// panel visits, and a `Delete`-form `window.quit` at B rather than
    /// at whichever window happened to create the panel.
    fn focus_step(&mut self, forward: bool) {
        let fid = self.active_frontend_key();
        let active = self.active_window_id();
        let hidden_panel = if self.views.get(&fid).is_some_and(|view| view.panel_hidden) {
            self.side_window_for(fid)
        } else {
            None
        };
        let next = self
            .active_layout()
            .focus_step(active, forward, &|id| Some(id) != hidden_panel);
        self.set_active_window_id(next);
        self.note_focus_transition(fid, active, next);
    }

    /// Focus an explicit window in the acting frontend, refreshing the
    /// panel's remembered document origin on the way (Q#BP2c).
    ///
    /// **The caller must have validated `target`** — that it is live and
    /// belongs to `fid`'s layout. Every Lua path does so through
    /// `lookup_window` or the display transaction's own revalidation;
    /// this function only `debug_assert!`s it, so a release-mode caller
    /// passing a foreign or dead id would leave `view.active` dangling.
    pub fn focus_window(&mut self, fid: FrontendId, target: WindowId) {
        let Some(view) = self.views.get_mut(&fid) else {
            return;
        };
        let previous = view.active;
        view.active = target;
        self.note_focus_transition(fid, previous, target);
    }

    /// Close the active window. Returns false when the layout would be
    /// left with no **document** window.
    ///
    /// Q#BP6 narrows the pre-arc "unless it's the only one" rule: a side
    /// window is never load-bearing, so closing the panel itself is
    /// always legal — including when it is the only other window — while
    /// closing the last *non-side* window is always refused.
    pub fn close_active(&mut self) -> bool {
        // Per-frontend: gate on the *active frontend's* window count, not
        // the global `self.windows` set. Every attached frontend keeps its
        // own windows in `self.windows`, so a global `<= 1` check let a
        // multi-frontend session close a frontend's last window and then
        // panic picking a successor from the now-empty layout.
        let fid = self.active_frontend_key();
        let target = self.active_window_id();
        let target_is_side = self
            .windows
            .get(&target)
            .is_some_and(crate::window::Window::is_side);
        if !target_is_side {
            let remaining_documents = self
                .active_layout()
                .iter_ids()
                .into_iter()
                .filter(|id| {
                    *id != target
                        && !self
                            .windows
                            .get(id)
                            .is_some_and(crate::window::Window::is_side)
                })
                .count();
            if remaining_documents == 0 {
                return false;
            }
        }
        self.active_layout_mut().close_window(target);
        self.windows.remove(&target);
        if target_is_side && let Some(view) = self.views.get_mut(&fid) {
            view.panel_hidden = false;
        }
        // Pick an adjacent window as the new focus, preferring a document.
        let ids = self.active_layout().iter_ids();
        let next = ids
            .iter()
            .copied()
            .find(|id| {
                !self
                    .windows
                    .get(id)
                    .is_some_and(crate::window::Window::is_side)
            })
            .unwrap_or_else(|| *ids.first().expect("at least one window remains"));
        let previous = self.active_window_id();
        self.set_active_window_id(next);
        self.note_focus_transition(fid, previous, next);
        true
    }

    /// Close every window except the active one, *within the active
    /// frontend* — including the panel (Q#BP6).
    ///
    /// # Errors
    /// From a side window: a panel cannot swallow the document tree.
    pub fn close_others(&mut self) -> Result<(), String> {
        // Per-frontend: only prune the active frontend's own layout. The
        // global `self.windows` set holds every frontend's windows, so a
        // global `retain(|id| id == keep)` deleted OTHER frontends'
        // windows — leaving their `view.active` dangling and panicking the
        // next `active_window()` (the multi-frontend close-others crash).
        let keep = self.active_window_id();
        if self
            .windows
            .get(&keep)
            .is_some_and(crate::window::Window::is_side)
        {
            return Err("window.close-others: not available in a side window".into());
        }
        let fid = self.active_frontend_key();
        let doomed: Vec<WindowId> = self
            .active_layout()
            .iter_ids()
            .into_iter()
            .filter(|id| *id != keep)
            .collect();
        self.active_layout_mut().keep_only(keep);
        for id in doomed {
            self.windows.remove(&id);
        }
        if let Some(view) = self.views.get_mut(&fid) {
            view.panel_hidden = false;
        }
        Ok(())
    }

    /// The `views` key the active-frontend accessors resolve to.
    #[must_use]
    pub fn active_frontend_key(&self) -> FrontendId {
        if self.views.contains_key(&self.active_frontend) {
            self.active_frontend
        } else {
            FrontendId::LOCAL
        }
    }

    // ---- side windows + display policy (bottom-panel arc) ------------------

    /// The one side leaf in `fid`'s layout, if it has one (Q#BP2a).
    #[must_use]
    pub fn side_window_for(&self, fid: FrontendId) -> Option<WindowId> {
        let view = self.views.get(&fid)?;
        view.layout.side_leaf(|id| {
            self.windows
                .get(&id)
                .is_some_and(crate::window::Window::is_side)
        })
    }

    /// Whether `fid`'s side window exists but is currently hidden.
    #[must_use]
    pub fn panel_hidden_for(&self, fid: FrontendId) -> bool {
        self.views.get(&fid).is_some_and(|view| view.panel_hidden)
            && self.side_window_for(fid).is_some()
    }

    /// Whether `fid` can render a side window at all (Q#BP13).
    #[must_use]
    pub fn panel_capable_for(&self, fid: FrontendId) -> bool {
        self.views.get(&fid).is_some_and(|view| view.panel_capable)
    }

    /// **The** primary document window for `fid` (Q#BP14).
    ///
    /// The frontend's active window when it is non-side, else its
    /// non-side target. Every consumer classified *Projection* in the
    /// framing's §1.3 census routes through this rather than through
    /// `active_window_for` / `active_buffer_id`, so focusing a panel
    /// re-sends no snapshot, suppresses no document, swaps no mirror,
    /// and cannot leak into a newly attached frontend's document view.
    #[must_use]
    pub fn primary_document_window(&self, fid: FrontendId) -> Option<WindowId> {
        let view = self.views.get(&fid)?;
        if !self
            .windows
            .get(&view.active)
            .is_some_and(crate::window::Window::is_side)
        {
            return Some(view.active);
        }
        self.non_side_target(fid).ok()
    }

    /// Capture where `fid`'s next asynchronous result belongs (Q#JR14,
    /// generalized by Q#DC-1/Q#DC-4).
    ///
    /// **Profile-blind and total**: it records what is there rather than
    /// what a caller intends to do later, and it never fails while a
    /// frontend id exists. A frontend with no document window yields a
    /// destination carrying only `frontend` — enough for a panel commit,
    /// and refused by a document commit with a reason naming the missing
    /// window. Returning `None` here instead would push the caller back
    /// onto ambient state, which is the misrouting the capture exists to
    /// remove.
    ///
    /// The document pair is set or cleared **together**: a window whose
    /// entry has gone yields neither half, so no consumer has to handle
    /// a window without its captured buffer.
    #[must_use]
    pub fn capture_view_destination(&self, fid: FrontendId) -> ViewDestination {
        let pair = self
            .primary_document_window(fid)
            .and_then(|window| Some((window, self.windows.get(&window)?.buffer_id)));
        ViewDestination {
            frontend: fid,
            window: pair.map(|(window, _)| window),
            buffer: pair.map(|(_, buffer)| buffer),
        }
    }

    /// [`Self::primary_document_window`]'s buffer, falling back to the
    /// focused window's when the layout is degenerate.
    #[must_use]
    pub fn primary_document_buffer(&self, fid: FrontendId) -> Option<BufferId> {
        let win = self.primary_document_window(fid)?;
        self.windows.get(&win).map(|window| window.buffer_id)
    }

    /// The non-side target rule (Q#BP11a).
    ///
    /// 1. the selected window when it is **not** a side window
    ///    (byte-identical to pre-arc behavior),
    /// 2. else the remembered `origin_document`, when it revalidates,
    /// 3. else the first non-side window in `iter_ids()` order,
    /// 4. else a pointed error. There is no document leaf from which a
    ///    valid fallback could be fabricated, and Q#BP6 forbids this as
    ///    a resting state, so the broken invariant is asserted rather
    ///    than papered over.
    ///
    /// # Errors
    /// When `fid` has no view, or its layout holds no non-side window.
    pub fn non_side_target(&self, fid: FrontendId) -> Result<WindowId, String> {
        let view = self
            .views
            .get(&fid)
            .ok_or_else(|| format!("frontend {fid:?} has no window layout"))?;
        let is_side = |id: WindowId| {
            self.windows
                .get(&id)
                .is_some_and(crate::window::Window::is_side)
        };
        if !is_side(view.active) {
            return Ok(view.active);
        }
        if let Some(origin) = self
            .windows
            .get(&view.active)
            .and_then(|w| w.params.origin_document())
            && view.layout.iter_ids().contains(&origin)
            && !is_side(origin)
        {
            return Ok(origin);
        }
        if let Some(first) = view.layout.iter_ids().into_iter().find(|id| !is_side(*id)) {
            return Ok(first);
        }
        debug_assert!(
            false,
            "invariant (Q#BP6): a frontend layout always retains at least one non-side window"
        );
        Err("no document window is available".into())
    }

    /// Record the document window a focus transition into the panel came
    /// from (Q#BP2c).
    ///
    /// Called on every focus change. Only a **non-side → side**
    /// transition refreshes the memory: panel→panel redisplay and
    /// passive display must not overwrite it, and a creation-only
    /// origin would go stale the moment the user entered the panel from
    /// a different document split.
    pub fn note_focus_transition(&mut self, fid: FrontendId, from: WindowId, to: WindowId) {
        if from == to {
            return;
        }
        debug_assert!(
            self.views
                .get(&fid)
                .is_some_and(|view| view.layout.iter_ids().contains(&to)),
            "focus transition target must belong to the acting frontend's layout"
        );
        let from_side = self
            .windows
            .get(&from)
            .is_some_and(crate::window::Window::is_side);
        let to_side = self
            .windows
            .get(&to)
            .is_some_and(crate::window::Window::is_side);
        if from_side || !to_side {
            return;
        }
        if let Some(window) = self.windows.get_mut(&to) {
            window.params.set_origin_document(Some(from));
        }
    }

    /// Minimum outer rows the document subtree beneath `fid`'s panel
    /// wrapper needs (Q#BP2). Falls back to the whole root when the tree
    /// does not have the wrapper shape.
    #[must_use]
    fn document_min_rows(&self, fid: FrontendId) -> u32 {
        let Some(view) = self.views.get(&fid) else {
            return MIN_WINDOW_OUTER_ROWS;
        };
        let node = self
            .side_window_for(fid)
            .and_then(|side| view.layout.document_subtree(side))
            .unwrap_or(&view.layout.root);
        subtree_min_rows(node)
    }

    /// The panel's **effective** row allocation on a frame whose window
    /// area is `area_rows` (Q#BP2), or `None` when it cannot be
    /// satisfied and must be hidden.
    ///
    /// `min(requested, area_rows - subtree_min_rows(document_root))`, then
    /// the structural floor. This is the whole bounded promise: the panel
    /// allocator never makes an otherwise satisfiable document tree
    /// unsatisfiable, and what the frame does to a document tree that
    /// could not fit anyway is unchanged behavior.
    #[must_use]
    pub fn panel_allocation(&self, fid: FrontendId, area_rows: u32) -> Option<u32> {
        let side = self.side_window_for(fid)?;
        let requested = self.windows.get(&side)?.params.fixed_rows?;
        let allowed = area_rows.saturating_sub(self.document_min_rows(fid));
        let alloc = requested.min(allowed);
        (alloc >= MIN_WINDOW_OUTER_ROWS).then_some(alloc)
    }

    /// The fixed-extent map both [`crate::window::Layout::compute`]
    /// production callers feed in (Q#BP2, R5-B1).
    ///
    /// Derived by this one shared helper rather than assembled at each
    /// call site: `window_placements` and the peer-presence overlay pass
    /// build different areas, and leaving the second on unfixed geometry
    /// would paint every peer cursor at the row it would occupy with no
    /// panel open.
    ///
    /// A hidden panel maps to `0`, which is Q#BP2's exact effective
    /// geometry for that state: the side leaf gets an empty rect, the
    /// document subtree receives every reclaimed row, and the stored
    /// request, wrapper, ids, weights, and order all stay intact.
    #[must_use]
    pub fn panel_fixed_rows(&self, fid: FrontendId, area_rows: u32) -> HashMap<WindowId, u32> {
        let mut fixed = HashMap::new();
        let Some(side) = self.side_window_for(fid) else {
            return fixed;
        };
        if self.views.get(&fid).is_some_and(|view| view.panel_hidden) {
            fixed.insert(side, 0);
            return fixed;
        }
        fixed.insert(side, self.panel_allocation(fid, area_rows).unwrap_or(0));
        fixed
    }

    /// Delete `side` from `fid`'s layout, collapsing the root-level
    /// wrapper and rehoming focus (Q#BP2a).
    ///
    /// Idempotent and safe to call from `kill_buffer`: the wrapper
    /// collapse is `Layout::close_window`'s existing
    /// `collapse_single_child_splits` pass, so no new tree code runs.
    pub fn remove_side_window(&mut self, fid: FrontendId, side: WindowId) {
        let Some(view) = self.views.get_mut(&fid) else {
            return;
        };
        if !view.layout.close_window(side) {
            return;
        }
        view.panel_hidden = false;
        let was_active = view.active == side;
        if was_active {
            let fallback = *view
                .layout
                .iter_ids()
                .first()
                .expect("Q#BP6: a document leaf always survives the wrapper collapse");
            view.active = fallback;
        }
        self.windows.remove(&side);
        if was_active
            && let Ok(target) = self.non_side_target(fid)
            && let Some(view) = self.views.get_mut(&fid)
        {
            view.active = target;
        }
        // A remembered origin pointing at a now-dead window is cleared by
        // `non_side_target`'s revalidation on next use; nothing else here
        // may reference the removed id.
        for window in self.windows.values_mut() {
            if window.params.origin_document() == Some(side) {
                window.params.set_origin_document(None);
            }
        }
    }

    /// Phase 1 of `window.quit` (Q#BP2c / Q#BP11b).
    ///
    /// Executes the window's recorded [`QuitAction`], returning the
    /// Phase-2 transaction Q#BP4 owns. A `Restore` whose buffer has been
    /// killed fails closed to `Delete`, dropping the unusable chain.
    ///
    /// # Errors
    /// A window with no recorded action returns a pointed error **without
    /// closing or switching anything** — non-side adopter fallbacks call
    /// their own existing restore path instead.
    pub fn quit_window(
        &mut self,
        fid: FrontendId,
        target: WindowId,
    ) -> Result<QuitOutcome, String> {
        let action = self
            .windows
            .get(&target)
            .ok_or_else(|| format!("window {} is not live", target.raw()))?
            .params
            .quit_action()
            .cloned()
            .ok_or_else(|| "window.quit: this window has no quit action".to_string())?;
        let action = match action {
            QuitAction::Restore { buffer_id, .. }
                if !self.registry.borrow().contains(buffer_id) =>
            {
                QuitAction::Delete
            }
            other => other,
        };
        match action {
            QuitAction::Delete => {
                // Capture the remembered origin BEFORE the window dies:
                // executing `Delete` focuses the revalidated origin, not
                // merely whatever leaf the wrapper collapse surfaced
                // (Q#BP11b). Entering the panel from document window B
                // must therefore return focus to B, not to the window
                // that happened to create the panel.
                let origin = self
                    .windows
                    .get(&target)
                    .and_then(|window| window.params.origin_document());
                self.remove_side_window(fid, target);
                let origin_valid = origin.is_some_and(|origin| {
                    self.views
                        .get(&fid)
                        .is_some_and(|view| view.layout.iter_ids().contains(&origin))
                        && !self
                            .windows
                            .get(&origin)
                            .is_some_and(crate::window::Window::is_side)
                });
                if origin_valid
                    && let Some(origin) = origin
                    && let Some(view) = self.views.get_mut(&fid)
                {
                    view.active = origin;
                }
                Ok(QuitOutcome::Deleted {
                    focus: self.views.get(&fid).map(|view| view.active),
                })
            }
            QuitAction::Restore {
                buffer_id,
                fixed_rows,
                dedicated,
                cursor,
                view_top,
                goal_col,
                selection,
                then,
            } => {
                self.install_buffer_in_window(target, buffer_id)?;
                let len = {
                    let reg = self.registry.borrow();
                    reg.get(buffer_id).map_or(0, Buffer::len)
                };
                let window = self
                    .windows
                    .get_mut(&target)
                    .ok_or_else(|| "window.quit: target vanished".to_string())?;
                window.params.fixed_rows = Some(fixed_rows.max(MIN_WINDOW_OUTER_ROWS));
                window.params.dedicated = dedicated;
                window.params.set_quit_action(Some(*then));
                // Clamp saved positions against the buffer's CURRENT
                // contents: it may have shrunk while the panel showed
                // something else. Derived `last_visible_rows` and
                // trait-object overlays are deliberately not snapshotted —
                // the switch hook reattaches overlays.
                window.cursor = cursor.min(len);
                window.view_top = view_top;
                window.goal_col = goal_col;
                window.selection = selection.filter(|sel| sel.anchor <= len);
                Ok(QuitOutcome::Restored { target, buffer_id })
            }
        }
    }

    /// Clamp a programmatic `fixed_rows` request (Q#BP2).
    ///
    /// # Errors
    /// A request of `0` is rejected rather than being an invisible
    /// "open".
    pub fn clamp_panel_rows(rows: u32) -> Result<u32, String> {
        if rows == 0 {
            return Err("panel height must be at least 1 row".into());
        }
        Ok(rows.max(MIN_WINDOW_OUTER_ROWS))
    }

    /// The window area a frontend's layout is computed into: the whole
    /// declared frame minus the one global status row, matching
    /// `window_placements`. `None` while geometry is **unknown**.
    #[must_use]
    pub fn frontend_area_rows(&self, fid: FrontendId) -> Option<u32> {
        let geometry = self.views.get(&fid)?.frame_geometry?;
        (geometry.total.rows >= 2 && geometry.total.cols > 0).then(|| geometry.total.rows - 1)
    }

    /// A frontend's current authoritative frame-geometry declaration.
    ///
    /// The panel producer echoes `geometry_epoch` into every
    /// [`pmacs_protocol::panel::PanelFrame`] it ships, and the daemon
    /// compares an inbound panel event's epoch against it (Q#BP16 step
    /// 3), so the epoch has to be readable, not only the size.
    #[must_use]
    pub fn frame_geometry_for(
        &self,
        fid: FrontendId,
    ) -> Option<crate::window::DeclaredFrameGeometry> {
        self.views.get(&fid)?.frame_geometry
    }

    /// Cache a frontend's authoritative frame capacity — the **grid /
    /// `LOCAL`** allocator (Q#BP2b, Stage 2 §3.1).
    ///
    /// Grid / `LOCAL` views call this from their real attach and resize
    /// sizes with an internally minted epoch; a semantic view never takes
    /// this path at all — it goes through
    /// [`Self::accept_frame_geometry`], which applies the frontend-owned
    /// epoch verbatim and does **no** value dedup.
    ///
    /// Value dedup is correct *here* and only here: a grid frontend's
    /// cells are the unit it declares, so an unchanged grid under
    /// unchanged metrics leaves any existing frame valid. It is wrong on
    /// the semantic path, where a font or scale change can invalidate a
    /// panel frame while [`crate::cell::CellSize`] is identical — the
    /// case daemon-side dedup cannot see (Q#BP2S1).
    ///
    /// **Exhaustion fails closed.** Allocation is checked rather than
    /// saturating: `saturating_add` pins at `u64::MAX`, after which two
    /// different geometries share one declaration id. On exhaustion the
    /// authoritative declaration is *cleared* to `None` (unknown), which
    /// is already non-presentable under Q#BP2b, so the caller's
    /// reconciliation hides the panel. Retaining the last valid geometry
    /// would keep painting a panel sized to a frame that no longer
    /// exists.
    pub fn declare_frame_geometry(
        &mut self,
        fid: FrontendId,
        total: crate::cell::CellSize,
    ) -> GeometryUpdate {
        let Some(view) = self.views.get_mut(&fid) else {
            return GeometryUpdate::Rejected;
        };
        if view
            .frame_geometry
            .is_some_and(|geometry| geometry.total == total)
        {
            return GeometryUpdate::Duplicate;
        }
        let next = match view.frame_geometry {
            None => Some(1),
            Some(geometry) => geometry.geometry_epoch.checked_add(1),
        };
        let Some(next) = next else {
            view.frame_geometry = None;
            return GeometryUpdate::Rejected;
        };
        view.frame_geometry = Some(crate::window::DeclaredFrameGeometry {
            geometry_epoch: next,
            total,
        });
        GeometryUpdate::Advanced
    }

    /// Accept a **semantic** frontend's authoritative geometry
    /// declaration (Q#BP15a, Stage 2 §3.1).
    ///
    /// Deliberately a second method rather than
    /// [`Self::declare_frame_geometry`] with an optional epoch: the two
    /// regimes differ in whether value dedup applies, and one ambiguous
    /// entry point would let a future caller silently take the wrong one.
    ///
    /// | Incoming declaration | Result |
    /// | --- | --- |
    /// | epoch **greater** than stored | [`GeometryUpdate::Advanced`], stored **verbatim**, even when `total` is unchanged |
    /// | same epoch, same `total` | [`GeometryUpdate::Duplicate`] |
    /// | same epoch, **different** `total` | [`GeometryUpdate::Rejected`] |
    /// | **lower** epoch, any `total` | [`GeometryUpdate::Rejected`] |
    ///
    /// The last row is not an optimization: a lower epoch carrying
    /// *identical* data is still stale, and accepting it would let a
    /// reordered declaration resurrect geometry the frontend has moved
    /// past.
    ///
    /// Epoch `0` is reserved for "never declared" and is rejected on the
    /// wire.
    pub fn accept_frame_geometry(
        &mut self,
        fid: FrontendId,
        geometry_epoch: u64,
        total: crate::cell::CellSize,
    ) -> GeometryUpdate {
        if geometry_epoch == 0 {
            return GeometryUpdate::Rejected;
        }
        let Some(view) = self.views.get_mut(&fid) else {
            return GeometryUpdate::Rejected;
        };
        match view.frame_geometry {
            Some(stored) if geometry_epoch < stored.geometry_epoch => GeometryUpdate::Rejected,
            Some(stored) if geometry_epoch == stored.geometry_epoch => {
                if stored.total == total {
                    GeometryUpdate::Duplicate
                } else {
                    GeometryUpdate::Rejected
                }
            }
            _ => {
                view.frame_geometry = Some(crate::window::DeclaredFrameGeometry {
                    geometry_epoch,
                    total,
                });
                GeometryUpdate::Advanced
            }
        }
    }

    /// The **third** geometry of Q#BP15a: the panel grid the daemon
    /// derives and paints, or `None` when no panel is presentable.
    ///
    /// Columns are the frontend's full declared width. Rows are the
    /// stored `fixed_rows` request clamped by Q#BP2's recursive document
    /// minimum ([`Self::panel_allocation`]) and then by the shared wire
    /// area budget, so a very wide frame cannot produce a frame the
    /// protocol would reject. **The stored request is never rewritten**
    /// — a later narrower geometry restores it.
    ///
    /// Returns `None` — the Q#BP2b hidden arm — when geometry is unknown,
    /// when the panel is hidden or absent, when the frame declares zero
    /// columns, or when even [`MIN_WINDOW_OUTER_ROWS`] rows would exceed
    /// the area budget.
    #[must_use]
    pub fn panel_grid_size(&self, fid: FrontendId) -> Option<crate::cell::CellSize> {
        if self.views.get(&fid)?.panel_hidden {
            return None;
        }
        self.presentable_panel_grid(fid)
    }

    /// The panel grid this frontend's layout and geometry **could**
    /// present, ignoring the cached `panel_hidden` bit.
    ///
    /// This is the single derivation behind both [`Self::panel_grid_size`]
    /// and [`Self::reconcile_panel_layout_core`]'s satisfiability test,
    /// and it is one function on purpose (review round 1, R1-1). When the
    /// wire-area clamp lived only in the renderer, the daemon shipped an
    /// authoritative `Absent` while `panel_hidden` stayed `false` — so
    /// keys still reached the invisible window and a panel terminal kept
    /// its controller. Q#BP2b is explicit that hiding is a **durable
    /// state transition**, never a per-frame effect, and two derivations
    /// of "can this panel be shown" is exactly how it became one.
    ///
    /// The area bound is a transport-safety limit rather than a frontend
    /// policy, so it is applied uniformly rather than only on the
    /// semantic path. It cannot bind for a grid frontend at any physically
    /// reachable width — two rows fit until roughly 131,000 columns — so
    /// one shared rule costs nothing and removes the drift.
    #[must_use]
    fn presentable_panel_grid(&self, fid: FrontendId) -> Option<crate::cell::CellSize> {
        let view = self.views.get(&fid)?;
        self.side_window_for(fid)?;
        let geometry = view.frame_geometry?;
        let cols = geometry.total.cols;
        if cols == 0 {
            return None;
        }
        let area_rows = self.frontend_area_rows(fid)?;
        let rows = self.panel_allocation(fid, area_rows)?;
        let budget_rows =
            u32::try_from(pmacs_protocol::panel::MAX_PANEL_VISIBLE_CELLS / (cols as usize).max(1))
                .unwrap_or(u32::MAX);
        let rows = rows.min(budget_rows);
        (rows >= MIN_WINDOW_OUTER_ROWS).then(|| crate::cell::CellSize::new(rows, cols))
    }

    /// Core half of the idempotent panel-reconciliation transaction
    /// (Q#BP2b). The caller owns the terminal manager, so releasing a
    /// controller is reported rather than performed.
    ///
    /// Hiding is a **durable state transition**, not a per-frame effect:
    /// a render-time dodge would still route keys to an invisible window
    /// and would leave the terminal controller claimed, because the
    /// resize path merely returns on zero content without releasing it.
    pub fn reconcile_panel_layout_core(&mut self, fid: FrontendId) -> PanelReconciliation {
        let mut result = PanelReconciliation::default();
        let Some(side) = self.side_window_for(fid) else {
            // `panel_hidden` never describes a panel that no longer
            // exists.
            if let Some(view) = self.views.get_mut(&fid) {
                result.changed = view.panel_hidden;
                view.panel_hidden = false;
            }
            return result;
        };
        let was_hidden = self.views.get(&fid).is_some_and(|view| view.panel_hidden);
        // Unknown geometry (a semantic view before Stage 2's declaration),
        // a zero-column frame, a layout that cannot spare the rows, and a
        // grid the shared wire budget cannot carry are ALL non-presentable
        // and all follow the hidden arm. One derivation, shared with the
        // renderer (R1-1): a condition that only the renderer knew about
        // produced a blank band with the durable state still saying
        // "visible".
        let satisfiable = self.presentable_panel_grid(fid).is_some();
        let Some(view) = self.views.get_mut(&fid) else {
            return result;
        };
        view.panel_hidden = !satisfiable;
        result.hidden = !satisfiable;
        result.changed = was_hidden != result.hidden;
        if satisfiable {
            // Focus is deliberately NOT restored when the panel
            // reappears — the user moved on; `C-x o` returns.
            return result;
        }
        if view.active == side {
            // Durable transition: move focus out and tell the caller to
            // release the terminal controller for this view key.
            result.released_terminal = Some(side);
            if let Ok(target) = self.non_side_target(fid)
                && let Some(view) = self.views.get_mut(&fid)
            {
                view.active = target;
            }
        }
        result
    }

    /// Move the horizontal boundary that `win` owns by `delta_rows`,
    /// growing `win` (Q#BP5 / Q#BP5b).
    ///
    /// `min_for` resolves each leaf's `window.min-height` preference; it
    /// is snapshotted by the caller **before** any geometry changes, so
    /// one gesture uses one set of minima.
    ///
    /// # Errors
    /// When `win` is not live in `fid`'s layout, when the panel is
    /// hidden, or when no adjustable horizontal boundary exists.
    #[allow(
        clippy::too_many_lines,
        reason = "one boundary-resize transaction: resolve, snapshot minima, clamp, write back"
    )]
    pub fn resize_boundary(
        &mut self,
        fid: FrontendId,
        win: WindowId,
        delta_rows: i32,
        area_rows: u32,
        min_for: &impl Fn(WindowId) -> u32,
    ) -> Result<(), String> {
        let view = self
            .views
            .get(&fid)
            .ok_or_else(|| format!("frontend {fid:?} has no window layout"))?;
        if !view.layout.iter_ids().contains(&win) {
            return Err(format!(
                "window {} does not belong to this frontend",
                win.raw()
            ));
        }
        let win_is_side = self
            .windows
            .get(&win)
            .is_some_and(crate::window::Window::is_side);
        if win_is_side && view.panel_hidden {
            return Err("window.resize: the panel is not currently visible".into());
        }
        // Q#BP5b rule 1: a side window resolves to its OWN fixed
        // boundary; rule 2: any other window resolves to the nearest
        // horizontal ancestor at which its path child has a following
        // sibling — the same boundary a drag on its bottom mode-line row
        // moves.
        let (boundary, lower_grows) = if win_is_side {
            let side = self
                .side_window_for(fid)
                .ok_or_else(|| "window.resize: no side window".to_string())?;
            let path = view
                .layout
                .path_to(side)
                .ok_or_else(|| "window.resize: side window is not in the layout".to_string())?;
            let (&last, parent) = path
                .split_last()
                .ok_or_else(|| "window.resize: no adjustable horizontal boundary".to_string())?;
            if last == 0 {
                return Err("window.resize: no adjustable horizontal boundary".into());
            }
            (
                crate::window::SplitBoundary {
                    path: parent.to_vec(),
                    upper: last - 1,
                },
                true,
            )
        } else {
            (
                view.layout.boundary_below(win).ok_or_else(|| {
                    "window.resize: no adjustable horizontal boundary".to_string()
                })?,
                false,
            )
        };

        let placements = view.layout.compute(
            crate::window::Rect::new(0, 0, area_rows, 1),
            &self.panel_fixed_rows(fid, area_rows),
        );
        let view = self
            .views
            .get(&fid)
            .ok_or_else(|| format!("frontend {fid:?} has no window layout"))?;
        let LayoutNode::Split { children, .. } = view
            .layout
            .node_at(&boundary.path)
            .ok_or_else(|| "window.resize: boundary vanished".to_string())?
        else {
            return Err("window.resize: boundary is not a split".into());
        };
        let upper_node = &children[boundary.upper];
        let lower_node = &children[boundary.upper + 1];
        let upper_rows = node_row_extent(upper_node, &placements);
        let lower_rows = node_row_extent(lower_node, &placements);
        let total = upper_rows + lower_rows;
        let min_upper = crate::window::interactive_min_rows(upper_node, min_for);
        let min_lower = crate::window::interactive_min_rows(lower_node, min_for);
        // Preserve the preferred minimum on BOTH sides when the frame can
        // satisfy it; when it is already smaller, the motion may not make
        // either side worse than it already is.
        let floor_upper = min_upper.min(upper_rows);
        let floor_lower = min_lower.min(lower_rows);
        let boundary_delta = if lower_grows { -delta_rows } else { delta_rows };
        let proposed = i64::from(upper_rows) + i64::from(boundary_delta);
        let lo = i64::from(floor_upper);
        let hi = i64::from(total.saturating_sub(floor_lower));
        if hi < lo {
            return Err("window.resize: no room to move this boundary".into());
        }
        let new_upper = u32::try_from(proposed.clamp(lo, hi))
            .map_err(|_| "window.resize: boundary out of range".to_string())?;
        let new_lower = total - new_upper;

        // A side window writes `fixed_rows` (its ABSOLUTE height survives
        // a terminal resize); a flexible pair writes weights (its RATIO
        // survives). That difference is the point.
        let lower_id = match lower_node {
            LayoutNode::Leaf(id) => Some(*id),
            LayoutNode::Split { .. } => None,
        };
        let lower_is_side = lower_id.is_some_and(|id| {
            self.windows
                .get(&id)
                .is_some_and(crate::window::Window::is_side)
        });
        if lower_is_side {
            let id = lower_id.expect("checked above");
            if let Some(window) = self.windows.get_mut(&id) {
                window.params.fixed_rows = Some(new_lower.max(MIN_WINDOW_OUTER_ROWS));
            }
            return Ok(());
        }
        // Rewrite every flexible child's weight as its current row
        // extent, with the two adjacent children replaced. Untouched
        // siblings therefore keep the extents they already had.
        let extents: Vec<u32> = children
            .iter()
            .enumerate()
            .map(|(i, child)| {
                if i == boundary.upper {
                    new_upper
                } else if i == boundary.upper + 1 {
                    new_lower
                } else {
                    node_row_extent(child, &placements)
                }
            })
            .collect();
        let fixed = self.panel_fixed_rows(fid, area_rows);
        let view = self
            .views
            .get_mut(&fid)
            .ok_or_else(|| format!("frontend {fid:?} has no window layout"))?;
        let Some(LayoutNode::Split {
            weights, children, ..
        }) = view.layout.node_at_mut(&boundary.path)
        else {
            return Err("window.resize: boundary vanished".into());
        };
        weights.resize(children.len(), 1);
        for (i, child) in children.iter().enumerate() {
            let pinned = matches!(child, LayoutNode::Leaf(id) if fixed.contains_key(id));
            if !pinned {
                weights[i] = extents[i].max(1);
            }
        }
        Ok(())
    }

    /// Install `buffer_id` in an explicit window, resetting its view
    /// state exactly as [`Self::switch_active_buffer_for`] does — except
    /// that redisplaying the buffer a window **already shows** is a no-op
    /// on cursor, viewport, selection, and overlays.
    ///
    /// # Errors
    /// Unknown window or buffer.
    pub fn install_buffer_in_window(
        &mut self,
        window_id: WindowId,
        buffer_id: BufferId,
    ) -> Result<(), String> {
        let text_view = {
            let reg = self.registry.borrow();
            let buf = reg.get(buffer_id).map_err(|e| e.to_string())?;
            TextView::new(buf)
        };
        let window = self
            .windows
            .get_mut(&window_id)
            .ok_or_else(|| format!("window {window_id:?} is not live"))?;
        if window.buffer_id == buffer_id {
            return Ok(());
        }
        window.buffer_id = buffer_id;
        window.text_view = text_view;
        window.overlays.clear();
        window.cursor = 0;
        window.selection = None;
        window.view_top = 0;
        window.goal_col = None;
        Ok(())
    }

    /// Phase 1 of the display transaction (Q#BP4): choose a target,
    /// install the buffer, and report what Phase 2 must do.
    ///
    /// Contains **no** Lua: the hook fan-out, the reconciliation, and the
    /// final-focus matrix all belong to the layer that owns the Lua host.
    ///
    /// # Errors
    /// An unusable exact target, an unsatisfiable placement request, or a
    /// layout with no eligible document window.
    pub fn display_buffer(
        &mut self,
        fid: FrontendId,
        request: &DisplayRequest,
    ) -> Result<DisplayOutcome, String> {
        let saved_active = self
            .views
            .get(&fid)
            .ok_or_else(|| format!("frontend {fid:?} has no window layout"))?
            .active;
        let placement = self.resolve_placement(fid, request)?;
        self.apply_placement(fid, request, &placement)?;
        let select = request
            .select
            .unwrap_or(!matches!(placement.kind, PlacementKind::Side { .. }));
        Ok(DisplayOutcome {
            target: placement.target,
            saved_active,
            select,
            created_side: matches!(placement.kind, PlacementKind::Side { created: true, .. }),
        })
    }

    /// Answer "is there a usable destination for this visit?" **without
    /// loading anything** (Q#BP11b step 2, R3-B17).
    ///
    /// `existing` is the side-effect-free dedup result: `None` means the
    /// file is not open yet, in which case an eligible destination must
    /// not be dedicated to *any* buffer — otherwise a dedicated origin
    /// could force a load that then has nowhere to go.
    ///
    /// # Errors
    /// An exact target that is dead, foreign, or dedicated; or a layout
    /// with no eligible document window.
    pub fn probe_display_target(
        &self,
        fid: FrontendId,
        existing: Option<BufferId>,
        window: Option<WindowId>,
    ) -> Result<WindowId, String> {
        self.probe_display_target_inner(fid, existing, window)
    }

    /// Whether `window` will accept `incoming` as its buffer — the one
    /// dedication rule, shared by every consumer (Journey Stage 1a,
    /// Q#JR14f).
    ///
    /// A dedicated window refuses anything other than what it already
    /// shows; an undedicated one accepts anything. `incoming` is
    /// deliberately optional, and the `None` case is not a degenerate
    /// spelling of "don't care" — it means **the replacement buffer does
    /// not exist yet**, and a dedicated window must therefore be treated
    /// as ineligible:
    ///
    /// | caller | `incoming` | dedicated window |
    /// |---|---|---|
    /// | [`Self::display_buffer`] exact-target arm | `Some(request.buffer_id)` | eligible only when already showing it |
    /// | [`Self::probe_display_target`] | its existing-buffer result | preserves the load-before-placement probe |
    /// | `commit_to` preflight | `None` | always ineligible |
    ///
    /// `commit_to` passes `None` because a directory open's destination
    /// is validated *before* the handler builds its buffer. Passing the
    /// captured bootstrap buffer instead would approve a window
    /// dedicated to *that* buffer, the handler would then claim and paint
    /// a different one, and the exact display would refuse afterwards —
    /// after the mutations the preflight exists to prevent.
    ///
    /// Extracted rather than reimplemented per caller: two copies of a
    /// rule that must agree is exactly the drift this stage's
    /// path-resolution unification exists to close, and a future
    /// eligibility rule added to only one copy would reopen it.
    #[must_use]
    pub fn window_accepts_buffer(&self, window: WindowId, incoming: Option<BufferId>) -> bool {
        self.windows.get(&window).is_some_and(|w| {
            !w.params.dedicated || incoming.is_some_and(|buffer_id| w.buffer_id == buffer_id)
        })
    }

    fn probe_display_target_inner(
        &self,
        fid: FrontendId,
        existing: Option<BufferId>,
        window: Option<WindowId>,
    ) -> Result<WindowId, String> {
        let view = self
            .views
            .get(&fid)
            .ok_or_else(|| format!("frontend {fid:?} has no window layout"))?;
        let eligible = |id: WindowId| self.window_accepts_buffer(id, existing);
        if let Some(target) = window {
            if !view.layout.iter_ids().contains(&target) {
                return Err(format!(
                    "display_file: window {} does not belong to this frontend",
                    target.raw()
                ));
            }
            if !eligible(target) {
                return Err(format!(
                    "display_file: window {} is dedicated to another buffer",
                    target.raw()
                ));
            }
            return Ok(target);
        }
        let is_side = |id: WindowId| {
            self.windows
                .get(&id)
                .is_some_and(crate::window::Window::is_side)
        };
        if let Some(buffer_id) = existing
            && let Some(showing) = view.layout.iter_ids().into_iter().find(|id| {
                !is_side(*id)
                    && self
                        .windows
                        .get(id)
                        .is_some_and(|w| w.buffer_id == buffer_id)
            })
        {
            return Ok(showing);
        }
        let mut candidates: Vec<WindowId> = Vec::new();
        if let Ok(preferred) = self.non_side_target(fid) {
            candidates.push(preferred);
        }
        candidates.extend(
            view.layout
                .iter_ids()
                .into_iter()
                .filter(|id| !is_side(*id)),
        );
        candidates
            .into_iter()
            .find(|id| eligible(*id))
            .ok_or_else(|| "display_file: no eligible document window is available".into())
    }

    /// Q#BP3's precedence: exact target, then side affinity, then
    /// ordinary reuse. Placement affinity precedes generic reuse —
    /// otherwise a persistent `*compilation*` buffer already visible in a
    /// document window makes `{side = "bottom"}` silently ignore its
    /// requested placement.
    #[allow(
        clippy::too_many_lines,
        reason = "Q#BP3's precedence ladder reads as one ordered policy"
    )]
    fn resolve_placement(
        &self,
        fid: FrontendId,
        request: &DisplayRequest,
    ) -> Result<Placement, String> {
        if request.window.is_some() && request.side.is_some() {
            return Err("display: `window` and `side` are mutually exclusive".into());
        }
        let view = self
            .views
            .get(&fid)
            .ok_or_else(|| format!("frontend {fid:?} has no window layout"))?;

        // 1. Exact target.
        if let Some(target) = request.window {
            if !view.layout.iter_ids().contains(&target) {
                return Err(format!(
                    "display: window {} does not belong to this frontend",
                    target.raw()
                ));
            }
            let window = self
                .windows
                .get(&target)
                .ok_or_else(|| format!("display: window {} is not live", target.raw()))?;
            if !self.window_accepts_buffer(target, Some(request.buffer_id)) {
                return Err(format!(
                    "display: window {} is dedicated to another buffer",
                    target.raw()
                ));
            }
            if request.height.is_some() && !window.is_side() {
                return Err("display: `height` requires a side window".into());
            }
            return Ok(Placement {
                target,
                kind: if window.is_side() {
                    PlacementKind::Side {
                        created: false,
                        replacing: window.buffer_id != request.buffer_id,
                    }
                } else {
                    PlacementKind::Ordinary
                },
            });
        }

        // 2. Side target — only on a panel-capable frontend.
        if request.side.is_some() && view.panel_capable {
            match self.side_window_for(fid) {
                Some(side) => {
                    let window = self
                        .windows
                        .get(&side)
                        .ok_or_else(|| "display: side window is not live".to_string())?;
                    if window.buffer_id == request.buffer_id {
                        return Ok(Placement {
                            target: side,
                            kind: PlacementKind::Side {
                                created: false,
                                replacing: false,
                            },
                        });
                    }
                    if !window.params.dedicated {
                        return Ok(Placement {
                            target: side,
                            kind: PlacementKind::Side {
                                created: false,
                                replacing: true,
                            },
                        });
                    }
                    // The one side slot is dedicated to another buffer.
                    // Never create a second one: fall through to the
                    // ordinary policy, discarding every side-specific
                    // parameter (Q#BP3 2.iii).
                }
                None => {
                    return Ok(Placement {
                        target: WindowId::next(),
                        kind: PlacementKind::Side {
                            created: true,
                            replacing: false,
                        },
                    });
                }
            }
        } else if request.side.is_none() && request.height.is_some() {
            // A freestanding `height` with no side request is a mistake.
            // A `height` that arrived WITH a side request and fell
            // through (not panel-capable, or the one slot is dedicated
            // elsewhere) is discarded, not rejected — capability
            // fallback must not turn into an error (Q#BP2c).
            return Err("display: `height` requires a side window".into());
        }

        // 3. Ordinary target.
        let is_side = |id: WindowId| {
            self.windows
                .get(&id)
                .is_some_and(crate::window::Window::is_side)
        };
        // 3.i — reuse a visible NON-side window already showing it. An
        // ordinary display never selects the panel by coincidence.
        if let Some(existing) = view.layout.iter_ids().into_iter().find(|id| {
            !is_side(*id)
                && self
                    .windows
                    .get(id)
                    .is_some_and(|w| w.buffer_id == request.buffer_id)
        }) {
            return Ok(Placement {
                target: existing,
                kind: PlacementKind::Ordinary,
            });
        }
        // 3.ii — the Q#BP11a candidate, then `iter_ids()` order, skipping
        // any window dedicated to a different buffer.
        let mut candidates: Vec<WindowId> = Vec::new();
        if let Ok(preferred) = self.non_side_target(fid) {
            candidates.push(preferred);
        }
        candidates.extend(
            view.layout
                .iter_ids()
                .into_iter()
                .filter(|id| !is_side(*id)),
        );
        for candidate in candidates {
            let eligible = self
                .windows
                .get(&candidate)
                .is_some_and(|w| !w.params.dedicated || w.buffer_id == request.buffer_id);
            if eligible {
                return Ok(Placement {
                    target: candidate,
                    kind: PlacementKind::Ordinary,
                });
            }
        }
        Err("display: no eligible document window is available".into())
    }

    /// Create the side window when needed, then install the buffer and
    /// reconcile the parameter semantics of Q#BP3.
    fn apply_placement(
        &mut self,
        fid: FrontendId,
        request: &DisplayRequest,
        placement: &Placement,
    ) -> Result<(), String> {
        let side = match placement.kind {
            PlacementKind::Ordinary => {
                // Reaching Ordinary while a side was REQUESTED means the
                // request fell back (not panel-capable, or the one slot
                // is dedicated elsewhere). A failed placement request may
                // never pin or dedicate a document window, so `side`,
                // `height`, `dedicated`, and quit bookkeeping are all
                // discarded here; only an explicit `select` survives, and
                // that is Phase 2's business.
                let fell_back = request.side.is_some();
                let same_buffer_redisplay = self
                    .windows
                    .get(&placement.target)
                    .is_some_and(|w| w.buffer_id == request.buffer_id);
                self.install_buffer_in_window(placement.target, request.buffer_id)?;
                let window = self
                    .windows
                    .get_mut(&placement.target)
                    .ok_or_else(|| "display: target window vanished".to_string())?;
                match request.dedicated {
                    Some(dedicated) if !fell_back => window.params.dedicated = dedicated,
                    // A same-buffer redisplay must not silently unpin a
                    // window; a genuine replacement starts undedicated.
                    _ if !same_buffer_redisplay => window.params.dedicated = false,
                    _ => {}
                }
                return Ok(());
            }
            PlacementKind::Side { created, replacing } => (created, replacing),
        };
        let (created, replacing) = side;
        let requested_side = request.side.unwrap_or(Side::Bottom);

        if created {
            let rows =
                Self::clamp_panel_rows(request.height.unwrap_or(request.default_panel_rows))?;
            let origin = self.non_side_target(fid).ok();
            let text_view = {
                let reg = self.registry.borrow();
                let buf = reg.get(request.buffer_id).map_err(|e| e.to_string())?;
                TextView::new(buf)
            };
            let mut window = Window::new(placement.target, request.buffer_id, text_view);
            window.params.side = Some(requested_side);
            window.params.fixed_rows = Some(rows);
            window.params.dedicated = request.dedicated.unwrap_or(false);
            window.params.set_quit_action(Some(QuitAction::Delete));
            window.params.set_origin_document(origin);
            self.windows.insert(placement.target, window);
            self.views
                .get_mut(&fid)
                .ok_or_else(|| format!("frontend {fid:?} has no window layout"))?
                .layout
                .install_side_leaf(placement.target);
            return Ok(());
        }

        // Reusing the existing slot. Capture the outgoing presentation
        // BEFORE the install resets the window's view state.
        let prior = {
            let window = self
                .windows
                .get(&placement.target)
                .ok_or_else(|| "display: side window vanished".to_string())?;
            QuitAction::Restore {
                buffer_id: window.buffer_id,
                fixed_rows: window.params.fixed_rows.unwrap_or(MIN_WINDOW_OUTER_ROWS),
                dedicated: window.params.dedicated,
                cursor: window.cursor,
                view_top: window.view_top,
                goal_col: window.goal_col,
                selection: window.selection,
                then: Box::new(
                    window
                        .params
                        .quit_action()
                        .cloned()
                        .unwrap_or(QuitAction::Delete),
                ),
            }
        };
        self.install_buffer_in_window(placement.target, request.buffer_id)?;
        let height = match request.height {
            Some(rows) => Some(Self::clamp_panel_rows(rows)?),
            None => None,
        };
        let window = self
            .windows
            .get_mut(&placement.target)
            .ok_or_else(|| "display: side window vanished".to_string())?;
        if let Some(rows) = height {
            window.params.fixed_rows = Some(rows);
        }
        if replacing {
            // A replacement's new presentation defaults to undedicated so
            // the one slot stays replaceable; an explicit dedication
            // applies only after the OLD presentation already passed
            // eligibility, so `dedicated = false` cannot clear-and-bypass
            // an existing dedication in the same call.
            window.params.dedicated = request.dedicated.unwrap_or(false);
            let mut action = prior;
            action.truncate_to(MAX_PANEL_QUIT_DEPTH);
            window.params.set_quit_action(Some(action));
        } else if let Some(dedicated) = request.dedicated {
            window.params.dedicated = dedicated;
        }
        Ok(())
    }

    // ---- selection / region (T M2.12) --------------------------------------

    /// Active region of the active window, as `(lo, hi)` byte
    /// positions, or `None` if no region is set or it is empty.
    #[must_use]
    pub fn active_region(&self) -> Option<(Position, Position)> {
        self.active_window().region()
    }

    /// Begin a selection at `anchor` on the active window.
    pub fn begin_selection(&mut self, anchor: Position) {
        self.active_window_mut().selection = Some(crate::window::Selection { anchor });
    }

    /// Drop any active selection on the active window.
    pub fn clear_selection(&mut self) {
        self.active_window_mut().selection = None;
    }

    /// Delete the active region (if any) from the active buffer and
    /// move the cursor to the deletion's start. No-op if there is no
    /// region. Returns the new buffer length.
    ///
    /// # Errors
    ///
    /// Returns the same stringified error shape as
    /// [`Self::apply_active_edit`] if the underlying delete fails.
    pub fn delete_region(&mut self) -> Result<u64, String> {
        let Some((lo, hi)) = self.active_region() else {
            return Ok(self.active_buffer_len());
        };
        let new_len = self
            .apply_active_edit(EditOp::Delete {
                range: Range { start: lo, end: hi },
            })?
            .new_rope
            .len();
        let aw = self.active_window_mut();
        aw.cursor = lo;
        aw.selection = None;
        aw.goal_col = None;
        Ok(new_len)
    }

    // ---- clipboard (Q#CM6) -------------------------------------------------

    /// The identifier under (or immediately left of) the cursor, or
    /// `None` when the cursor isn't on a word (Q#CM3, the `symbol`
    /// context). A word is a run of ASCII alphanumerics / `_`; since
    /// those are all single-byte, the slice always lands on UTF-8
    /// boundaries.
    #[must_use]
    pub fn word_at_cursor(&self) -> Option<String> {
        let bytes = self.buffer_bytes(self.active_buffer_id());
        let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        let cursor = (self.cursor() as usize).min(bytes.len());
        let mut start = cursor;
        while start > 0 && is_word(bytes[start - 1]) {
            start -= 1;
        }
        let mut end = cursor;
        while end < bytes.len() && is_word(bytes[end]) {
            end += 1;
        }
        if start == end {
            return None;
        }
        String::from_utf8(bytes[start..end].to_vec()).ok()
    }

    /// Bytes of the active region, or `None` when nothing is selected.
    #[must_use]
    pub fn region_bytes(&self) -> Option<Vec<u8>> {
        let (lo, hi) = self.active_region()?;
        let reg = self.registry.borrow();
        let buf = reg.get(self.active_buffer_id()).ok()?;
        let mut out = vec![0u8; (hi - lo) as usize];
        buf.snapshot_rope().slice(lo, hi, &mut out);
        Some(out)
    }

    // ---- command boundaries (kill ring, Q#KR2) --------------------------

    /// Record `name` as `fid`'s executing command: `last = this;
    /// this = name`. Called once per interactive command dispatch —
    /// keybound commands, the self-insert fallback, menu items, and
    /// `pmacs.command.invoke_interactive` (`M-x`).
    pub fn rotate_command(&mut self, fid: FrontendId, name: &str) {
        let entry = self.command_history.entry(fid).or_default();
        entry.last = entry.this.take();
        entry.this = Some(name.to_owned());
    }

    /// Break `fid`'s command chain: a non-command input happened (an
    /// optimistic CRDT edit, a point-moving pointer gesture, an inbound
    /// paste, an unbound key). Sets `this = None`, so the next
    /// rotation yields `last = None` and every chain-sensitive check
    /// (kill append, `M-y`) fails.
    pub fn break_command_chain(&mut self, fid: FrontendId) {
        self.command_history.entry(fid).or_default().this = None;
    }

    /// The active frontend's previous command — Emacs's `last-command`
    /// as observed *from inside* the currently-running command (its own
    /// rotation already happened).
    #[must_use]
    pub fn last_command(&self) -> Option<&str> {
        self.command_history
            .get(&self.active_frontend)?
            .last
            .as_deref()
    }

    /// The active frontend's *current* command — Emacs's `this-command`.
    /// Inside a `buffer.after-edit` hook this names the command that
    /// produced the edit, which is the **input-origin signal**:
    /// `"buffer.self-insert"` means the edit was a typed character
    /// (keybound or optimistic), while a paste / pointer / unbound input
    /// left it `None`. Per-frontend, so two attached frontends never
    /// misclassify each other's input.
    #[must_use]
    pub fn this_command(&self) -> Option<&str> {
        self.command_history
            .get(&self.active_frontend)?
            .this
            .as_deref()
    }

    // ---- typed-edit provenance (auto-pairing, Q#AP9) ---------------------

    /// Declare that `fid`'s dispatch is about to invoke
    /// `buffer.self-insert` for `codepoint`: the next insert primitive
    /// whose character matches completes the [`TypedEditRecord`].
    /// Called by the dispatch fallback only — programmatic
    /// `pmacs.command.invoke("buffer.self-insert")` deliberately never
    /// arms, so a hook run after it observes no record.
    pub fn typed_edit_arm(&mut self, fid: FrontendId, codepoint: char) {
        self.typed_edit_pending = Some(TypedEditPending {
            fid,
            codepoint,
            record: None,
        });
    }

    /// Complete the pending typed-edit record from the effective edit,
    /// if one is armed for this character and hasn't completed yet.
    /// First match wins: a command body that somehow self-inserts the
    /// same character twice records the first landing (the one the
    /// dispatcher's keystroke produced). `context` is the caller's
    /// pre-edit `(buffer, window)` — the buffer the edit landed in
    /// even when an intercept switched the active context mid-edit.
    fn typed_edit_complete(
        &mut self,
        ch: char,
        context: (BufferId, WindowId),
        requested: Range,
        requested_len: u64,
        edit: &Edit,
    ) {
        let matches = self.typed_edit_pending.as_ref().is_some_and(|p| {
            p.record.is_none() && p.codepoint == ch && p.fid == self.active_frontend
        });
        if !matches {
            return;
        }
        // The revision postcondition anchor: if the buffer vanished
        // (killed mid-command), no record — absence fails closed.
        let Some(revision) = self
            .registry
            .borrow()
            .get(context.0)
            .ok()
            .map(Buffer::revision)
        else {
            return;
        };
        let clean = edit.range == requested && edit.inserted_len == requested_len;
        let record = TypedEditRecord {
            buffer: context.0,
            window: context.1,
            codepoint: ch,
            requested_start: requested.start,
            requested_end: requested.end,
            effective_start: edit.range.start,
            effective_end: edit.range.end,
            inserted_len: edit.inserted_len,
            post_cursor: self.active_window().cursor,
            clean,
            revision,
        };
        if let Some(p) = self.typed_edit_pending.as_mut() {
            p.record = Some(record);
        }
    }

    /// Take back the pending arm at the end of `fid`'s dispatch,
    /// yielding the completed record (or `None` if the self-insert
    /// never landed — rejected edit, command error). Always clears the
    /// pending state: an arm never survives its dispatch cycle.
    ///
    /// Postcondition (PR #110 round 1, finding 1): the record is
    /// yielded only if the edited buffer's revision still equals the
    /// one captured at completion. A command body that edited again
    /// after the self-insert — replacing or removing the typed
    /// character while leaving the cursor in place — produced state
    /// the record no longer describes; the record dies here, before
    /// it can be armed for the hook.
    pub fn typed_edit_finish(&mut self, fid: FrontendId) -> Option<TypedEditRecord> {
        let pending = self.typed_edit_pending.take()?;
        if pending.fid != fid {
            return None;
        }
        let record = pending.record?;
        let current = self
            .registry
            .borrow()
            .get(record.buffer)
            .ok()
            .map(Buffer::revision);
        if current != Some(record.revision) {
            return None;
        }
        Some(record)
    }

    /// Arm `record` for consumption during the `buffer.after-edit`
    /// fan-out the caller is about to run. The caller MUST clear the
    /// slot when the fan-out returns ([`Self::typed_edit_clear_armed`]),
    /// error paths included — the record must never outlive its hook.
    pub fn typed_edit_set_armed(&mut self, fid: FrontendId, record: TypedEditRecord) {
        self.typed_edit_armed = Some((fid, record));
    }

    /// Drop any untaken armed record. Producers call this immediately
    /// after their `buffer.after-edit` fan-out returns.
    pub fn typed_edit_clear_armed(&mut self) {
        self.typed_edit_armed = None;
    }

    /// One-shot consume of the armed typed-edit record, per frontend:
    /// yields the record iff one is armed for the *active* frontend,
    /// clearing the slot. Second and later takes — including from a
    /// nested manual `pmacs.hook.run("buffer.after-edit")` — observe
    /// `None`, as does any context where no producer armed a record
    /// (paste, programmatic mutation, standalone manual hook runs).
    pub fn take_typed_edit(&mut self) -> Option<TypedEditRecord> {
        if self.typed_edit_armed.as_ref()?.0 != self.active_frontend {
            return None;
        }
        self.typed_edit_armed.take().map(|(_, rec)| rec)
    }

    /// Copy the active region into the clipboard slot and queue an
    /// outbound OS-clipboard publish to the originating frontend.
    /// Returns `false` (a no-op) when there is no region.
    pub fn clipboard_copy(&mut self) -> bool {
        let Some(bytes) = self.region_bytes() else {
            return false;
        };
        self.clipboard_slot.clone_from(&bytes);
        self.pending_clipboard = Some((self.active_frontend, bytes));
        true
    }

    /// Cut: copy the region, then delete it. Returns `false` (a no-op)
    /// when there is no region.
    ///
    /// # Errors
    ///
    /// Propagates [`Self::delete_region`]'s error.
    pub fn clipboard_cut(&mut self) -> Result<bool, String> {
        if !self.clipboard_copy() {
            return Ok(false);
        }
        self.delete_region()?;
        Ok(true)
    }

    /// Set the clipboard slot to arbitrary bytes and queue the
    /// OS-clipboard publish to the acting frontend (kill ring Q#KR1).
    /// The ring's kills have no region for [`Self::clipboard_copy`] to
    /// read (`C-k`'s killed line, an appended chain), so the Lua ring
    /// pushes the exact bytes here.
    pub fn clipboard_set(&mut self, bytes: Vec<u8>) {
        self.clipboard_set_for(self.active_frontend, bytes);
    }

    /// Set and publish clipboard bytes to one authenticated frontend.
    pub fn clipboard_set_for(&mut self, frontend_id: FrontendId, bytes: Vec<u8>) {
        self.clipboard_slot.clone_from(&bytes);
        self.pending_clipboard = Some((frontend_id, bytes));
    }

    /// The clipboard slot's current bytes, or `None` when empty (kill
    /// ring Q#KR6 — the yank-time "did external content arrive via a
    /// paste since our last kill" check).
    #[must_use]
    pub fn clipboard_get(&self) -> Option<&[u8]> {
        if self.clipboard_slot.is_empty() {
            None
        } else {
            Some(&self.clipboard_slot)
        }
    }

    /// Paste the clipboard slot at the cursor, replacing the active
    /// region if one exists (one undo step, like CUA type-over).
    /// Returns `false` when the slot is empty.
    ///
    /// # Errors
    ///
    /// Propagates the underlying edit error.
    pub fn clipboard_paste(&mut self) -> Result<bool, String> {
        if self.clipboard_slot.is_empty() {
            return Ok(false);
        }
        let bytes = self.clipboard_slot.clone();
        self.insert_bytes_over_region(&bytes)?;
        Ok(true)
    }

    /// Insert externally-pasted bytes at the cursor (inbound OS paste:
    /// terminal bracketed paste, or GPU Ctrl-V via `arboard`),
    /// refreshing the slot so a later in-app paste repeats them.
    /// Replaces the active region if one exists.
    ///
    /// # Errors
    ///
    /// Propagates the underlying edit error.
    pub fn paste_inbound(&mut self, data: &[u8]) -> Result<(), String> {
        self.clipboard_slot = data.to_vec();
        self.insert_bytes_over_region(data)
    }

    /// Shared insert/replace for paste: `Replace` over the active
    /// region, else `Insert` at the cursor. The cursor lands just past
    /// the inserted bytes and any selection is cleared. No-op insert for
    /// empty `bytes`.
    fn insert_bytes_over_region(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.active_window_mut().goal_col = None;
        let start = if let Some((lo, hi)) = self.active_region() {
            self.apply_active_edit(EditOp::Replace {
                range: Range { start: lo, end: hi },
                bytes,
            })?;
            lo
        } else {
            let pos = self.active_window().cursor;
            self.apply_active_edit(EditOp::Insert { pos, bytes })?;
            pos
        };
        let aw = self.active_window_mut();
        aw.cursor = start + bytes.len() as u64;
        aw.selection = None;
        Ok(())
    }

    /// Select the whole active buffer (anchor at 0, cursor at the end).
    pub fn select_all(&mut self) {
        let len = self.active_buffer_len();
        self.begin_selection(0);
        let aw = self.active_window_mut();
        aw.cursor = len;
        aw.goal_col = None;
    }

    /// Drain the one-shot outbound clipboard publish, if any. Called by
    /// the dispatcher each tick (alongside `pending_crdt_ops`).
    pub fn take_pending_clipboard(&mut self) -> Option<(FrontendId, Vec<u8>)> {
        self.pending_clipboard.take()
    }

    /// The current clipboard slot bytes (testing / introspection).
    #[must_use]
    pub fn clipboard_slot(&self) -> &[u8] {
        &self.clipboard_slot
    }

    // ---- context menu (Q#CM1) ----------------------------------------------

    /// True while a context menu is open.
    #[must_use]
    pub fn menu_is_open(&self) -> bool {
        self.menu.lock().expect("menu mutex poisoned").is_some()
    }

    /// Open a menu of resolved `rows` anchored at the absolute `anchor`
    /// cell. A no-op (stays closed) when no row is selectable. Attaches
    /// the TUI overlay on first open (deduped by kind).
    pub fn menu_open(&mut self, rows: Vec<crate::menu::MenuRow>, anchor: (u32, u32)) {
        let state = crate::menu::MenuState::new(rows, anchor);
        let opened = state.is_some();
        *self.menu.lock().expect("menu mutex poisoned") = state;
        if opened {
            self.ensure_menu_overlay();
        }
    }

    /// Close the menu (the overlay then self-suppresses).
    pub fn menu_close(&mut self) {
        *self.menu.lock().expect("menu mutex poisoned") = None;
    }

    /// Move the highlight by `delta` items (wrapping, skipping separators).
    pub fn menu_step(&mut self, delta: isize) {
        if let Some(m) = self.menu.lock().expect("menu mutex poisoned").as_mut() {
            m.step(delta);
        }
    }

    /// Set the highlight to `row` if it names a selectable item (mouse
    /// hover / click).
    pub fn menu_set_active_row(&mut self, row: usize) {
        if let Some(m) = self.menu.lock().expect("menu mutex poisoned").as_mut()
            && matches!(m.rows.get(row), Some(crate::menu::MenuRow::Item { .. }))
        {
            m.active = row;
        }
    }

    /// The active item's command name, if a menu is open.
    #[must_use]
    pub fn menu_active_command(&self) -> Option<String> {
        self.menu
            .lock()
            .expect("menu mutex poisoned")
            .as_ref()
            .and_then(|m| m.active_command().map(str::to_owned))
    }

    /// Hit-test an absolute cell against the open popup, returning the
    /// selectable row it covers (or `None`).
    #[must_use]
    pub fn menu_hit(&self, row: u32, col: u32) -> Option<usize> {
        self.menu
            .lock()
            .expect("menu mutex poisoned")
            .as_ref()
            .and_then(|m| m.hit(row, col))
    }

    /// Ensure the active window carries a [`crate::menu::MenuView`]
    /// overlay (deduped by kind). The view reads the shared `menu`, so
    /// one instance suffices; it renders nothing while the menu is closed.
    fn ensure_menu_overlay(&mut self) {
        let menu = self.menu.clone();
        let win = self.active_window_mut();
        if !win.overlay_kinds().contains(&"context-menu") {
            win.push_overlay(Box::new(crate::menu::MenuView::new(menu)));
        }
    }

    // ---- in-buffer completion popup (Arc 1a, Q#C2/Q#C3) --------------------

    /// True while the in-buffer completion popup is open.
    #[must_use]
    pub fn completion_popup_is_open(&self) -> bool {
        self.completion_popup
            .lock()
            .expect("completion popup poisoned")
            .is_some()
    }

    /// Open (or replace) the completion popup session. Attaches the
    /// self-suppressing [`crate::completion::CompletionView`] overlay to
    /// the active window on first use (deduped by kind, like the menu).
    /// Emptiness is enforced upstream:
    /// [`crate::completion::CompletionPopupState::new`] refuses to build
    /// a candidate-less session.
    pub fn completion_popup_open(&mut self, mut state: crate::completion::CompletionPopupState) {
        // Stamp the owning window (Lua publishers don't know window
        // identity): only that window's overlay paints the popup, and
        // a focus change invalidates the session.
        state.window_id = Some(self.active_window_id());
        *self
            .completion_popup
            .lock()
            .expect("completion popup poisoned") = Some(state);
        self.ensure_completion_overlay();
    }

    /// Close the popup (the overlay then self-suppresses).
    pub fn completion_popup_close(&mut self) {
        *self
            .completion_popup
            .lock()
            .expect("completion popup poisoned") = None;
    }

    /// Move the popup highlight by `delta` (wrapping).
    pub fn completion_popup_step(&mut self, delta: isize) {
        if let Some(p) = self
            .completion_popup
            .lock()
            .expect("completion popup poisoned")
            .as_mut()
        {
            p.step(delta);
        }
    }

    /// Q#C3 session invariant: the popup only survives while the
    /// active buffer still matches, the cursor sits at or after the
    /// anchor, and every byte between them is a word byte (`[A-Za-z0-9_]`
    /// --- the same ASCII word definition the Lua driver uses). A
    /// trigger-character session (empty prefix, `cursor == anchor`)
    /// holds trivially. Returns the `(anchor, cursor)` pair while the
    /// invariant holds.
    #[must_use]
    fn completion_session_holds(&self) -> Option<(Position, Position)> {
        /// Longest byte run still plausibly a completion prefix; past
        /// this the session is stale, not a prefix.
        const MAX_PREFIX_BYTES: u64 = 512;

        let (buffer_id, window_id, anchor) = {
            let guard = self
                .completion_popup
                .lock()
                .expect("completion popup poisoned");
            let p = guard.as_ref()?;
            (p.buffer_id, p.window_id, p.anchor)
        };
        if window_id != Some(self.active_window_id()) {
            return None; // focus moved to another window/split
        }
        if self.active_buffer_id() != buffer_id {
            return None;
        }
        let cursor = self.active_window().cursor;
        if cursor < anchor || cursor - anchor > MAX_PREFIX_BYTES {
            return None;
        }
        let reg = self.registry.borrow();
        let buffer = reg.get(buffer_id).ok()?;
        if cursor > buffer.len() {
            return None;
        }
        let mut bytes = vec![0u8; (cursor - anchor) as usize];
        if !bytes.is_empty() {
            buffer.snapshot_rope().slice(anchor, cursor, &mut bytes);
        }
        bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
            .then_some((anchor, cursor))
    }

    /// Q#C3 post-dispatch validation: close the popup unless the
    /// session invariant still holds. Called by the dispatcher after
    /// every fallen-through key (motion, edits, buffer switches) and
    /// cheap enough to call unconditionally --- a closed popup is a
    /// single mutex peek.
    pub fn completion_popup_validate(&mut self) {
        if self.completion_popup_is_open() && self.completion_session_holds().is_none() {
            self.completion_popup_close();
        }
    }

    /// Q#C7 accept: re-validate the session at the moment of accept,
    /// close the popup, and --- only when the invariant still holds ---
    /// replace `[anchor .. cursor]` with the highlighted candidate's
    /// insert text as a **single** edit (one undo step, mirroring
    /// [`Self::insert_char_over_region`]). Returns `true` iff the
    /// buffer was edited (the dispatcher fires `buffer.after-edit`
    /// off that signal).
    pub fn completion_popup_accept(&mut self) -> bool {
        let holds = self.completion_session_holds();
        let snap = {
            let guard = self
                .completion_popup
                .lock()
                .expect("completion popup poisoned");
            guard
                .as_ref()
                .and_then(|p| p.selected_candidate().map(|c| c.insert_text.clone()))
        };
        self.completion_popup_close();
        let (Some((anchor, cursor)), Some(text)) = (holds, snap) else {
            return false;
        };
        self.active_window_mut().goal_col = None;
        // An empty range degenerates to a plain insert (the
        // trigger-character case, where nothing was typed yet).
        let result = if cursor > anchor {
            self.apply_active_edit(EditOp::Replace {
                range: Range {
                    start: anchor,
                    end: cursor,
                },
                bytes: text.as_bytes(),
            })
        } else {
            self.apply_active_edit(EditOp::Insert {
                pos: anchor,
                bytes: text.as_bytes(),
            })
        };
        if let Err(e) = result {
            self.status = format!("completion accept failed: {e}");
            return false;
        }
        let aw = self.active_window_mut();
        aw.cursor = anchor + text.len() as u64;
        aw.selection = None;
        true
    }

    // ---- round-trip input buffers (Arc 1b, Q#P6) ----------------------------

    /// Mark (or unmark) `buffer_id` as requiring round-trip input.
    /// See the field doc on `round_trip_buffers` for the semantics.
    pub fn set_round_trip_input(&mut self, buffer_id: BufferId, on: bool) {
        if on {
            self.round_trip_buffers.insert(buffer_id);
        } else {
            self.round_trip_buffers.remove(&buffer_id);
        }
    }

    /// True while the active buffer requires round-trip input (a
    /// panel or other buffer-local-keymap surface is focused).
    #[must_use]
    pub fn active_buffer_round_trips(&self) -> bool {
        self.round_trip_buffers.contains(&self.active_buffer_id())
    }

    /// Whether an explicit buffer requires daemon-owned round-trip input.
    #[must_use]
    pub fn buffer_round_trips(&self, buffer_id: BufferId) -> bool {
        self.round_trip_buffers.contains(&buffer_id)
    }

    /// Ensure the active window carries a
    /// [`crate::completion::CompletionView`] overlay (deduped by kind).
    /// The view reads the shared popup, so one instance suffices; it
    /// renders nothing while the popup is closed.
    fn ensure_completion_overlay(&mut self) {
        let popup = self.completion_popup.clone();
        let wid = self.active_window_id();
        let win = self.active_window_mut();
        if !win.overlay_kinds().contains(&"completion-popup") {
            win.push_overlay(Box::new(crate::completion::CompletionView::new(popup, wid)));
        }
    }

    /// Safely remove `buffer_id` from the registry. Any window that
    /// was displaying it is redirected to a fallback buffer (`*scratch*`,
    /// created on demand) so window state never refers to a missing id.
    ///
    /// # Errors
    ///
    /// Returns an error string when `buffer_id` is the only buffer in
    /// the registry (the registry must remain non-empty), or when the
    /// id doesn't resolve.
    pub fn kill_buffer(&mut self, buffer_id: BufferId) -> Result<(), String> {
        {
            let reg = self.registry.borrow();
            if !reg.contains(buffer_id) {
                return Err(format!("buffer {buffer_id:?} not found"));
            }
            if reg.len() <= 1 {
                return Err("cannot kill the last remaining buffer".into());
            }
        }
        self.round_trip_buffers.remove(&buffer_id);
        let fallback = {
            let mut reg = self.registry.borrow_mut();
            match reg.find_by_name("*scratch*") {
                Some(id) if id != buffer_id => id,
                _ => {
                    let candidate = reg.ids().iter().copied().find(|id| *id != buffer_id);
                    match candidate {
                        Some(id) => id,
                        None => reg.create("*scratch*"),
                    }
                }
            }
        };
        // Q#BP10a: a side window showing the victim is CLOSED, not
        // redirected to `*scratch*`. Redirecting would strand an
        // unrelated buffer in the panel slot; the wrapper collapse
        // restores the prior root, which by construction holds a leaf.
        let doomed_sides: Vec<(FrontendId, WindowId)> = self
            .views
            .iter()
            .filter_map(|(fid, view)| {
                let side = view.layout.side_leaf(|id| {
                    self.windows
                        .get(&id)
                        .is_some_and(crate::window::Window::is_side)
                })?;
                (self.windows.get(&side)?.buffer_id == buffer_id).then_some((*fid, side))
            })
            .collect();
        for (fid, side) in doomed_sides {
            self.remove_side_window(fid, side);
        }
        {
            let reg = self.registry.borrow();
            let buf = reg.get(fallback).map_err(|e| e.to_string())?;
            for win in self.windows.values_mut() {
                if win.buffer_id == buffer_id {
                    win.buffer_id = fallback;
                    win.text_view = TextView::new(buf);
                    win.overlays.clear();
                    win.cursor = 0;
                    win.selection = None;
                    win.view_top = 0;
                    win.goal_col = None;
                }
            }
        }
        self.registry
            .borrow_mut()
            .remove(buffer_id)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Rebind every buffer affected by a successful rename of `old` to
    /// `new` (dired Stage 2a, Q#DR14). Returns one
    /// [`RenameRebind`] per buffer moved.
    ///
    /// A rename is a **transaction across path owners**, not a field
    /// update. This method owns the two owners that live in the buffer:
    /// the stored path and — subject to the provenance rule below — the
    /// name. Everything else keyed by the path (URI-keyed LSP stores,
    /// diagnostic overlays, dired's pathless handles, a package's own
    /// URI table) reconciles off the `resource.renamed` hook that the
    /// caller fires, because no buffer-keyed rebind can reach them.
    ///
    /// Both rename paths call this — the drain harvest for
    /// `pmacs.fs.rename` and `apply_resource_op`'s rename arm — so the
    /// two cannot drift apart.
    ///
    /// # The name
    ///
    /// The name is rewritten only for a buffer whose name is
    /// [`crate::buffer::BufferNameOrigin::PathDerived`]. String
    /// inspection cannot substitute for that bit in either direction: a
    /// relative open is named `foo.rs` (so an equality test leaves it
    /// stale), and a user may name a buffer with a string that
    /// normalizes to its own path (so a path-equivalence test
    /// overwrites a chosen name). When it does fire, the new name is
    /// the **normalized** new path — a buffer opened relatively
    /// therefore acquires an absolute name, because no buffer records
    /// which base its name was relative to. Reconciliation re-records
    /// `PathDerived`, so a second rename still follows.
    pub fn reconcile_rename(&mut self, old: &Path, new: &Path) -> Vec<RenameRebind> {
        let old_n = normalize_buffer_path(old.to_path_buf());
        let new_n = normalize_buffer_path(new.to_path_buf());
        // A directory rename moves its whole subtree by construction,
        // so descendants are always in scope here.
        let affected = {
            let reg = self.registry.borrow();
            buffers_bound_under(&reg, &old_n, true)
        };
        let mut rebinds = Vec::with_capacity(affected.len());
        for (id, bound) in affected {
            // Rebuild the path under the new root. An exact match maps
            // to `new` itself; a descendant keeps its relative tail.
            let target = if bound == old_n {
                new_n.clone()
            } else {
                match bound.strip_prefix(&old_n) {
                    Ok(tail) => new_n.join(tail),
                    // Unreachable: `buffers_bound_under` matched on
                    // exactly this prefix. Skip rather than guess.
                    Err(_) => continue,
                }
            };
            let name_followed = {
                let mut reg = self.registry.borrow_mut();
                let Ok(buf) = reg.get_mut(id) else { continue };
                buf.set_file_path(Some(target.clone()));
                // The file behind this buffer moved, so metadata
                // captured against the old path no longer describes
                // it. Clearing is what `set_buffer_path`'s callers do
                // via `set_buffer_meta`; leaving it would make
                // external-change detection compare against a stat of
                // a path that is gone.
                buf.set_file_meta(None);
                if buf.name_origin() == crate::buffer::BufferNameOrigin::PathDerived {
                    buf.set_path_derived_name(target.display().to_string());
                    true
                } else {
                    false
                }
            };
            rebinds.push(RenameRebind {
                buffer_id: id,
                old_path: bound,
                new_path: target,
                name_followed,
            });
        }
        rebinds
    }

    /// Reconcile the buffers a successful delete of `path` orphaned
    /// (dired Stage 2a, Q#DR18).
    ///
    /// Walks the whole registry by normalized equality **or**
    /// component-aware prefix, so descendants of a deleted directory
    /// are included and a second buffer on one path is not missed.
    /// Descendants are unconditionally in scope here, unlike in
    /// `delete_verdict`: a recursive delete destroyed them, and a
    /// non-recursive one only succeeds on an *empty* directory, so a
    /// buffer still bound underneath it was already an orphan.
    ///
    /// Policy, per buffer:
    ///
    /// * **modified** — kept alive and reported. The buffer keeps its
    ///   contents; only the file is gone. This is the half of the
    ///   promise that is robust, because it runs at drain time against
    ///   whatever state exists then.
    /// * **mid-edit** — skipped entirely and reported in `refused`,
    ///   **preflighted** rather than discovered. A refusal from
    ///   `BufferRegistry::remove` is *not* inert: by the time it
    ///   returns `ConcurrentEdit`, [`Self::kill_buffer`] has already
    ///   dropped the id from `round_trip_buffers`, closed any side
    ///   window showing the buffer, and redirected every remaining
    ///   window onto a fallback with cursor, selection, overlays and
    ///   scroll position reset. The preflight is *sound*, not merely
    ///   cheap: phase 1 is entirely `EditorCore`, which holds no Lua
    ///   handle, so nothing between the check and the removal can
    ///   re-enter Lua and begin an edit.
    /// * otherwise — killed through the full phase 1 above.
    ///
    /// Neither refusal aborts the rest: a directory delete reaching
    /// twelve descendants must not stop at the one that is mid-edit.
    ///
    /// # Phase 2 is the caller's
    ///
    /// Buffer removal is two phases and the only place they are
    /// composed today is a Lua binding (`pmacs.buffer.kill`). Phase 2 —
    /// buffer-scoped keymaps, buffer-local config, folds, and the
    /// registered `on_removed` callbacks — lives in `lua_bindings` and
    /// needs `&Lua`, so this returns [`DeleteReconcile::killed`] and
    /// its caller runs phase 2 over those ids. `EditorCore` does not
    /// gain a Lua handle.
    pub fn reconcile_delete(&mut self, path: &Path) -> DeleteReconcile {
        let affected = {
            let reg = self.registry.borrow();
            buffers_bound_under(&reg, path, true)
        };
        let mut out = DeleteReconcile::default();
        for (id, _bound) in affected {
            let preflight = {
                let reg = self.registry.borrow();
                let Ok(buf) = reg.get(id) else { continue };
                let name = buf.name().to_owned();
                if buf.is_modified() {
                    Some(Err((true, name)))
                } else if buf.editing_in_progress() {
                    Some(Err((false, name)))
                } else {
                    Some(Ok(()))
                }
            };
            match preflight {
                Some(Ok(())) => {}
                Some(Err((true, name))) => {
                    out.kept_modified.push((id, name));
                    continue;
                }
                Some(Err((false, name))) => {
                    out.refused.push((
                        id,
                        format!("buffer {name:?} is mid-edit; finish the edit first"),
                    ));
                    continue;
                }
                None => continue,
            }
            match self.kill_buffer(id) {
                Ok(()) => out.killed.push(id),
                // Named, because the reason alone is not actionable:
                // `kill_buffer`'s "cannot kill the last remaining
                // buffer" says nothing about *which* buffer is now
                // bound to a path whose file is gone, and that buffer's
                // name is what the user needs in order to save it
                // somewhere else.
                Err(message) => {
                    let name = self
                        .registry
                        .borrow()
                        .get(id)
                        .map_or_else(|_| format!("{id:?}"), |b| b.name().to_owned());
                    out.refused
                        .push((id, format!("buffer {name:?}: {message}")));
                }
            }
        }
        out
    }

    /// Re-root every URI-keyed overlay in **every** window from
    /// `old_uri` to `new_uri` (dired Stage 2a, §5).
    ///
    /// The traversal mirrors overlay disposal's
    /// (`lua_bindings`'s `retain` over `overlay_identity`), with the
    /// `retain` replaced by [`View::rename_resource`]. That reaches
    /// passive windows as well as the active one — which the Lua attach
    /// path cannot, since `pmacs.diag._attach_view` takes the active
    /// window and errors otherwise — and preserves composition order,
    /// because nothing is removed or re-pushed.
    ///
    /// A window that never received the overlay still has none;
    /// renaming cannot re-root an overlay that was never attached.
    pub fn rename_resource_in_views(&mut self, old_uri: &str, new_uri: &str) {
        for win in self.windows.values_mut() {
            for overlay in &mut win.overlays {
                overlay.rename_resource(old_uri, new_uri);
            }
        }
    }

    /// Switch one frontend's active window to a different buffer, allocating
    /// a fresh [`TextView`] for it without changing global active state.
    pub fn switch_active_buffer_for(
        &mut self,
        frontend_id: FrontendId,
        buffer_id: BufferId,
    ) -> Result<(), String> {
        let text_view = {
            let reg = self.registry.borrow();
            let buf = reg.get(buffer_id).map_err(|e| e.to_string())?;
            TextView::new(buf)
        };
        let aw = self
            .active_window_mut_for(frontend_id)
            .ok_or_else(|| format!("frontend {frontend_id:?} has no active window"))?;
        aw.buffer_id = buffer_id;
        aw.text_view = text_view;
        // Overlays were keyed to the previous buffer's coordinates;
        // dropping them is safer than carrying through coordinates
        // that no longer mean anything. Callers that want to preserve
        // an overlay across buffer switches re-register after.
        aw.overlays.clear();
        aw.cursor = 0;
        aw.selection = None;
        aw.view_top = 0;
        aw.goal_col = None;
        Ok(())
    }

    /// Switch the globally active frontend's active window.
    pub fn switch_active_buffer(&mut self, buffer_id: BufferId) -> Result<(), String> {
        self.switch_active_buffer_for(self.active_frontend, buffer_id)
    }
}

// ---------------------------------------------------------------------------
// Codepoint navigation
// ---------------------------------------------------------------------------

/// Return the byte position of the codepoint immediately before `pos`.
fn prev_codepoint(buf: &Buffer, pos: Position) -> Position {
    if pos == 0 {
        return 0;
    }
    let rope = buf.snapshot_rope();
    let mut p = pos - 1;
    while p > 0 {
        let b = rope.byte_at(p).unwrap_or(0);
        if (b & 0xC0) != 0x80 {
            return p;
        }
        p -= 1;
    }
    0
}

/// Return the byte position of the codepoint immediately after `pos`.
fn next_codepoint(buf: &Buffer, pos: Position) -> Position {
    let len = buf.len();
    if pos >= len {
        return len;
    }
    let rope = buf.snapshot_rope();
    let lead = rope.byte_at(pos).unwrap_or(0);
    let advance = utf8_codepoint_len(lead);
    (pos + advance as u64).min(len)
}

fn utf8_codepoint_len(lead: u8) -> usize {
    if lead < 0xC0 {
        1
    } else if lead < 0xE0 {
        2
    } else if lead < 0xF0 {
        3
    } else {
        4
    }
}

/// Decode the codepoint starting at `pos`. Returns `(char, advance)`
/// where `advance` is the number of bytes the codepoint consumed.
/// `None` if `pos` is past the buffer end or the bytes there are not
/// valid UTF-8.
fn char_at(buf: &Buffer, pos: Position) -> Option<(char, u64)> {
    let rope = buf.snapshot_rope();
    if pos >= rope.len() {
        return None;
    }
    let lead = rope.byte_at(pos)?;
    let len = utf8_codepoint_len(lead);
    let mut bytes = [0u8; 4];
    for (i, slot) in bytes.iter_mut().take(len).enumerate() {
        *slot = rope.byte_at(pos + i as u64).unwrap_or(0);
    }
    let s = std::str::from_utf8(&bytes[..len]).ok()?;
    let ch = s.chars().next()?;
    Some((ch, len as u64))
}

/// Whether `c` counts as a word character. Matches the Emacs default:
/// alphanumerics plus underscore. Punctuation and whitespace are
/// separators.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Forward-word semantics: skip non-word characters, then skip word
/// characters, returning the resulting position.
fn forward_word(buf: &Buffer, mut pos: Position) -> Position {
    let len = buf.len();
    // Skip non-word.
    while pos < len {
        let Some((ch, advance)) = char_at(buf, pos) else {
            break;
        };
        if is_word_char(ch) {
            break;
        }
        pos += advance;
    }
    // Skip word.
    while pos < len {
        let Some((ch, advance)) = char_at(buf, pos) else {
            break;
        };
        if !is_word_char(ch) {
            break;
        }
        pos += advance;
    }
    pos
}

fn word_range_at(buf: &Buffer, pos: Position) -> Option<(Position, Position)> {
    let (ch, ch_len) = char_at(buf, pos)?;
    if !is_word_char(ch) {
        return None;
    }
    // Walk back from just *past* the char under the cursor, not from
    // `pos` itself: `backward_word(pos)` at a word's FIRST character
    // sees the non-word char before it, skips it, and crosses into
    // the previous word — double-clicking the 'w' of "llo world"
    // would select "llo world". From `pos + ch_len` the char behind
    // is this word's own first char, so the walk stops at its start.
    let start = backward_word(buf, pos.saturating_add(ch_len));
    let end = forward_word(buf, pos);
    (start < end).then_some((start, end))
}

/// True iff `line` is empty or contains only ASCII whitespace.
/// Used by paragraph motion: a blank line is a paragraph break.
fn line_is_blank(buf: &Buffer, view: &TextView, line: usize) -> bool {
    let Some(start) = view.line_offset(line) else {
        return true;
    };
    let Some(len) = view.line_len(buf, line) else {
        return true;
    };
    if len == 0 {
        return true;
    }
    let rope = buf.snapshot_rope();
    for chunk in rope.chunks(start, start + len) {
        if chunk.iter().any(|b| !b.is_ascii_whitespace()) {
            return false;
        }
    }
    true
}

/// Forward-paragraph: skip blank lines if currently on one, then
/// scan forward until the first blank line; return the position at
/// the start of that line, or the buffer end.
fn forward_paragraph(buf: &Buffer, view: &TextView, pos: Position) -> Position {
    let total = view.line_count();
    if total == 0 {
        return pos;
    }
    let cur_line = view.line_at_offset(pos);
    let starting_blank = line_is_blank(buf, view, cur_line);
    let mut line = cur_line.saturating_add(1);
    if starting_blank {
        while line < total && line_is_blank(buf, view, line) {
            line += 1;
        }
    }
    while line < total {
        if line_is_blank(buf, view, line) {
            return view.line_offset(line).unwrap_or(pos);
        }
        line += 1;
    }
    buf.len()
}

/// Backward-paragraph: mirror of [`forward_paragraph`].
fn backward_paragraph(buf: &Buffer, view: &TextView, pos: Position) -> Position {
    if pos == 0 {
        return 0;
    }
    let cur_line = view.line_at_offset(pos);
    if cur_line == 0 {
        return 0;
    }
    let starting_blank = line_is_blank(buf, view, cur_line);
    let mut line = cur_line - 1;
    if starting_blank {
        loop {
            if !line_is_blank(buf, view, line) {
                break;
            }
            if line == 0 {
                return view.line_offset(0).unwrap_or(0);
            }
            line -= 1;
        }
    }
    loop {
        if line_is_blank(buf, view, line) {
            return view.line_offset(line).unwrap_or(0);
        }
        if line == 0 {
            return 0;
        }
        line -= 1;
    }
}

/// Backward-word semantics: step back over non-word characters, then
/// step back over word characters.
fn backward_word(buf: &Buffer, mut pos: Position) -> Position {
    // Step back over non-word characters.
    while pos > 0 {
        let prev = prev_codepoint(buf, pos);
        let Some((ch, _)) = char_at(buf, prev) else {
            break;
        };
        if is_word_char(ch) {
            break;
        }
        pos = prev;
    }
    // Step back over word characters.
    while pos > 0 {
        let prev = prev_codepoint(buf, pos);
        let Some((ch, _)) = char_at(buf, prev) else {
            break;
        };
        if !is_word_char(ch) {
            break;
        }
        pos = prev;
    }
    pos
}

/// Every path-bound buffer an operation on `target` affects, paired
/// with its **normalized** stored path (dired Stage 2a; the shared walk
/// query #190 introduced for `delete_verdict`, lifted so rename
/// reconciliation and delete reconciliation cannot drift from it).
///
/// Three properties, each of which a naive lookup gets wrong:
///
/// * It scans **every** buffer.
///   [`crate::buffer_registry::BufferRegistry::find_by_path`] is
///   first-match-only, and duplicate path-bound buffers are reachable
///   from public Lua via `pmacs.buffer.from_file` — so a first match
///   can hide a second buffer on the same path, which then survives
///   pointing at a path that no longer exists.
/// * Both sides are normalized. Stored paths are normalized on write
///   (`set_buffer_path`) while an op names its target however the
///   caller spelled it, so a raw comparison misses the match entirely.
/// * Containment is **component-aware** ([`Path::starts_with`]), never
///   a string prefix: `/foo` is not an ancestor of `/foobar`.
///
/// `include_descendants` is the caller's decision because the two
/// consumers legitimately differ. A delete *guard* scopes descendants
/// to `recursive` (#190: a non-recursive delete destroys nothing
/// beneath the target, so a buffer under it must not refuse the op),
/// whereas a **rename** always moves its whole subtree and a
/// post-delete reconciliation is looking at a directory that is
/// already gone.
pub fn buffers_bound_under(
    reg: &crate::buffer_registry::BufferRegistry,
    target: &Path,
    include_descendants: bool,
) -> Vec<(BufferId, PathBuf)> {
    let target = normalize_buffer_path(target.to_path_buf());
    let mut out = Vec::new();
    for id in reg.ids() {
        let Ok(buf) = reg.get(*id) else { continue };
        let Some(bound) = buf.file_path() else {
            continue;
        };
        let bound = normalize_buffer_path(bound.to_path_buf());
        if bound == target || (include_descendants && bound.starts_with(&target)) {
            out.push((*id, bound));
        }
    }
    out
}

/// One buffer moved by [`EditorCore::reconcile_rename`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameRebind {
    /// The buffer that moved.
    pub buffer_id: BufferId,
    /// Its normalized path before the rename.
    pub old_path: PathBuf,
    /// Its normalized path after the rename.
    pub new_path: PathBuf,
    /// Whether the buffer's **name** followed the path, per
    /// [`crate::buffer::BufferNameOrigin`]. Reported rather than
    /// inferred so a consumer does not have to re-derive the
    /// provenance rule.
    pub name_followed: bool,
}

/// Outcome of [`EditorCore::reconcile_delete`].
///
/// Three lists rather than two, because "kept on purpose" and "could
/// not be removed" are different events: collapsing them makes a
/// failure look like a policy decision.
#[derive(Clone, Debug, Default)]
pub struct DeleteReconcile {
    /// Buffers whose phase 1 (core-side removal) completed. The
    /// caller **must** run phase 2 (`after_buffer_removed`) over
    /// these — `EditorCore` holds no Lua handle.
    pub killed: Vec<BufferId>,
    /// Modified buffers kept alive deliberately, with their names.
    pub kept_modified: Vec<(BufferId, String)>,
    /// Buffers that could not be removed, with the reason.
    pub refused: Vec<(BufferId, String)>,
}

/// Normalize a buffer path to an absolute, lexically-clean form:
///
/// 1. expand a leading `~` / `~/…` against `$HOME`,
/// 2. join onto the process cwd if still relative,
/// 3. fold `.` / `..` purely lexically.
///
/// No filesystem access and no symlink resolution (unlike
/// [`std::fs::canonicalize`]): the result is correct for a
/// not-yet-created "[new file]" buffer and never silently rewrites a
/// path's on-disk identity. Every step is best-effort — if `$HOME`
/// or the cwd is unavailable the path is returned as far as it could
/// be resolved rather than panicking.
///
/// Public because dired needs the *same* canonical form the buffer
/// registry keys on (Q#DR2): its buffer-per-directory naming and
/// `find_buffer_for_path`'s dedup have to agree, and a Lua-side mirror
/// of this function would be a second implementation of a canonical
/// form — the tab-width-constants class in miniature. `pmacs.path
/// .canonicalize` is this function, not a copy of it.
pub fn normalize_buffer_path(path: PathBuf) -> PathBuf {
    let path = expand_tilde(path);
    let abs = if path.is_absolute() {
        path
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.join(path)
    } else {
        path
    };
    lexical_normalize(&abs)
}

/// Expand a leading `~` (whole component only) using `$HOME`. A bare
/// `~` becomes `$HOME`; `~/x` becomes `$HOME/x`. `~user` is left
/// untouched (no passwd lookup). Returns the input unchanged if it
/// has no leading `~`, isn't valid UTF-8, or `$HOME` is unset.
pub fn expand_tilde(path: PathBuf) -> PathBuf {
    let Some(s) = path.to_str() else {
        return path;
    };
    if s == "~" {
        return std::env::var_os("HOME").map_or(path, PathBuf::from);
    }
    if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return Path::new(&home).join(rest);
    }
    path
}

/// Fold `.` and `..` components without touching the filesystem.
/// `..` pops a preceding normal segment; against the root (or a
/// Windows prefix) it is dropped, since you cannot ascend past it.
pub(crate) fn lexical_normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut stack: Vec<Component> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match stack.last() {
                Some(Component::Normal(_)) => {
                    stack.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => stack.push(Component::ParentDir),
            },
            c => stack.push(c),
        }
    }
    let mut out = PathBuf::new();
    for c in stack {
        out.push(c.as_os_str());
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn lexical_normalize_folds_dot_and_dotdot() {
        assert_eq!(
            lexical_normalize(Path::new("/a/./b/../c")),
            PathBuf::from("/a/c")
        );
        // `..` cannot ascend past the root.
        assert_eq!(
            lexical_normalize(Path::new("/../../x")),
            PathBuf::from("/x")
        );
        // Already clean ⇒ unchanged (keeps tempdir paths stable so
        // the LSP acceptance tests' exact-path asserts still hold).
        assert_eq!(
            lexical_normalize(Path::new("/tmp/quickshell/ipc.cpp")),
            PathBuf::from("/tmp/quickshell/ipc.cpp")
        );
    }

    #[test]
    fn expand_tilde_only_at_leading_component() {
        // `~user` (no passwd lookup) and a non-leading `~` are left
        // exactly as-is, independent of `$HOME`.
        assert_eq!(
            expand_tilde(PathBuf::from("~bob/x")),
            PathBuf::from("~bob/x")
        );
        assert_eq!(expand_tilde(PathBuf::from("a/~/b")), PathBuf::from("a/~/b"));
        // With `$HOME` set (the case in any normal test environment)
        // a leading `~` / `~/…` expands against its real value.
        if let Some(home) = std::env::var_os("HOME") {
            assert_eq!(expand_tilde(PathBuf::from("~")), PathBuf::from(&home));
            assert_eq!(
                expand_tilde(PathBuf::from("~/src/ipc.cpp")),
                Path::new(&home).join("src/ipc.cpp")
            );
        }
    }

    #[test]
    fn normalize_buffer_path_yields_absolute() {
        // A relative path becomes absolute (joined onto cwd) — this
        // is exactly what made clangd reject `file://ipc.cpp`.
        let p = normalize_buffer_path(PathBuf::from("ipc.cpp"));
        assert!(p.is_absolute(), "expected absolute, got {p:?}");
        assert!(p.ends_with("ipc.cpp"));
    }

    fn fresh() -> EditorCore {
        let reg: SharedRegistry =
            Rc::new(RefCell::new(crate::buffer_registry::BufferRegistry::new()));
        EditorCore::new(reg)
    }

    fn from_bytes(bytes: &[u8]) -> EditorCore {
        let reg: SharedRegistry =
            Rc::new(RefCell::new(crate::buffer_registry::BufferRegistry::new()));
        EditorCore::from_bytes(reg, "test", bytes)
    }

    /// Attach a second frontend `fid` with its own single-window layout,
    /// sharing the active buffer (mirrors `build_fresh_frontend_view`).
    fn attach_frontend(s: &mut EditorCore, fid: FrontendId) -> WindowId {
        let buffer_id = s.active_buffer_id();
        let text_view = {
            let reg = s.registry.borrow();
            crate::text_view::TextView::new(reg.get(buffer_id).expect("buffer present"))
        };
        let win_id = WindowId::next();
        s.windows
            .insert(win_id, Window::new(win_id, buffer_id, text_view));
        s.register_frontend_view(
            fid,
            FrontendView {
                layout: Layout::single(win_id),
                active: win_id,
                fold_projection: true,
                panel_capable: true,
                frame_geometry: None,
                panel_hidden: false,
            },
        );
        win_id
    }

    #[test]
    fn close_others_does_not_prune_other_frontends_windows() {
        // Multi-frontend crash regression: closing others from one
        // frontend must not delete another frontend's window (which would
        // leave its `view.active` dangling → `active_window()` panic).
        let mut s = from_bytes(b"hello\n");
        let local_win = s.active_window_id();
        let fid2 = FrontendId(42);
        let win2 = attach_frontend(&mut s, fid2);

        s.active_frontend = fid2;
        s.close_others().expect("document window may close others");

        assert!(
            s.windows.contains_key(&win2),
            "close_others keeps the active frontend's own window"
        );
        assert!(
            s.windows.contains_key(&local_win),
            "close_others must not remove another frontend's window"
        );
        // LOCAL's active window is intact — no panic.
        s.active_frontend = FrontendId::LOCAL;
        assert_eq!(s.active_window_id(), local_win);
        let _ = s.active_window();
    }

    #[test]
    fn close_active_refuses_the_frontends_last_window_even_with_others_attached() {
        // The "only one left" guard is per-frontend: two windows exist
        // globally (LOCAL + fid2), but fid2 has just one, so close must
        // refuse rather than empty fid2's layout and panic.
        let mut s = from_bytes(b"hello\n");
        let local_win = s.active_window_id();
        let fid2 = FrontendId(42);
        let win2 = attach_frontend(&mut s, fid2);

        s.active_frontend = fid2;
        assert!(
            !s.close_active(),
            "close_active refuses the active frontend's only window"
        );
        assert!(s.windows.contains_key(&win2));
        assert!(s.windows.contains_key(&local_win));
    }

    #[test]
    fn insert_advances_cursor() {
        let mut s = from_bytes(b"");
        s.insert_char('h');
        s.insert_char('i');
        assert_eq!(s.cursor(), 2);
        assert_eq!(s.active_buffer_len(), 2);
    }

    #[test]
    fn backspace_undoes_insertion() {
        let mut s = from_bytes(b"abc");
        s.active_window_mut().cursor = 3;
        s.backspace();
        assert_eq!(s.cursor(), 2);
        assert_eq!(s.active_buffer_len(), 2);
    }

    #[test]
    fn copy_captures_region_and_queues_publish() {
        let mut s = from_bytes(b"hello world");
        s.begin_selection(0);
        s.active_window_mut().cursor = 5; // region [0,5) = "hello"
        assert!(s.clipboard_copy());
        assert_eq!(s.clipboard_slot(), b"hello");
        let (fid, bytes) = s.take_pending_clipboard().expect("publish queued");
        assert_eq!(fid, s.active_frontend);
        assert_eq!(bytes, b"hello");
        // Drained: second take is None.
        assert!(s.take_pending_clipboard().is_none());
        // Copy does not mutate the buffer.
        assert_eq!(s.active_buffer_len(), 11);
    }

    #[test]
    fn copy_without_region_is_a_noop() {
        let mut s = from_bytes(b"abc");
        assert!(!s.clipboard_copy());
        assert!(s.clipboard_slot().is_empty());
        assert!(s.take_pending_clipboard().is_none());
    }

    #[test]
    fn cut_copies_then_deletes_region() {
        let mut s = from_bytes(b"hello world");
        s.begin_selection(6);
        s.active_window_mut().cursor = 11; // region [6,11) = "world"
        assert!(s.clipboard_cut().unwrap());
        assert_eq!(s.clipboard_slot(), b"world");
        assert_eq!(s.buffer_bytes(s.active_buffer_id()), b"hello ");
        assert_eq!(s.cursor(), 6);
    }

    #[test]
    fn paste_inserts_slot_at_cursor() {
        let mut s = from_bytes(b"ac");
        // Seed the slot via a copy.
        s.begin_selection(0);
        s.active_window_mut().cursor = 1; // "a"
        s.clipboard_copy();
        // Paste "a" between a and c.
        s.clear_selection();
        s.active_window_mut().cursor = 1;
        assert!(s.clipboard_paste().unwrap());
        assert_eq!(s.buffer_bytes(s.active_buffer_id()), b"aac");
        assert_eq!(s.cursor(), 2);
    }

    #[test]
    fn paste_replaces_active_region_as_one_step() {
        let mut s = from_bytes(b"hello world");
        // Copy "hello".
        s.begin_selection(0);
        s.active_window_mut().cursor = 5;
        s.clipboard_copy();
        // Select "world" and paste over it.
        s.begin_selection(6);
        s.active_window_mut().cursor = 11;
        assert!(s.clipboard_paste().unwrap());
        assert_eq!(s.buffer_bytes(s.active_buffer_id()), b"hello hello");
        assert!(s.active_region().is_none()); // selection cleared
    }

    #[test]
    fn paste_with_empty_slot_is_a_noop() {
        let mut s = from_bytes(b"abc");
        assert!(!s.clipboard_paste().unwrap());
        assert_eq!(s.active_buffer_len(), 3);
    }

    #[test]
    fn paste_inbound_inserts_and_refreshes_slot() {
        let mut s = from_bytes(b"ab");
        s.active_window_mut().cursor = 1;
        s.paste_inbound(b"XYZ").unwrap();
        assert_eq!(s.buffer_bytes(s.active_buffer_id()), b"aXYZb");
        assert_eq!(s.cursor(), 4);
        // Slot refreshed, so an in-app paste repeats the external text.
        assert_eq!(s.clipboard_slot(), b"XYZ");
        // Inbound paste does NOT queue an outbound publish (no echo loop).
        assert!(s.take_pending_clipboard().is_none());
    }

    #[test]
    fn select_all_spans_the_buffer() {
        let mut s = from_bytes(b"hello");
        s.active_window_mut().cursor = 2;
        s.select_all();
        assert_eq!(s.active_region(), Some((0, 5)));
        assert_eq!(s.region_bytes().unwrap(), b"hello");
    }

    #[test]
    fn word_at_cursor_reads_the_identifier() {
        let mut s = from_bytes(b"foo bar_baz qux");
        s.active_window_mut().cursor = 6; // inside "bar_baz"
        assert_eq!(s.word_at_cursor().as_deref(), Some("bar_baz"));
        s.active_window_mut().cursor = 0; // start of "foo"
        assert_eq!(s.word_at_cursor().as_deref(), Some("foo"));
        s.active_window_mut().cursor = 3; // just past "foo" → scans left
        assert_eq!(s.word_at_cursor().as_deref(), Some("foo"));
    }

    #[test]
    fn word_at_cursor_is_none_in_whitespace() {
        let mut s = from_bytes(b"a   b");
        s.active_window_mut().cursor = 2; // a run of spaces, none adjacent left
        assert_eq!(s.word_at_cursor(), None);
    }

    #[test]
    fn cursor_navigation_left_right() {
        let mut s = from_bytes(b"abc");
        s.active_window_mut().cursor = 0;
        s.move_right();
        assert_eq!(s.cursor(), 1);
        s.move_right();
        s.move_right();
        s.move_right();
        assert_eq!(s.cursor(), 3);
        s.move_left();
        s.move_left();
        s.move_left();
        s.move_left();
        assert_eq!(s.cursor(), 0);
    }

    #[test]
    fn cursor_navigation_up_down_preserves_column() {
        let mut s = from_bytes(b"abcdef\nghi\njklmno");
        s.active_window_mut().cursor = 4;
        s.move_down();
        assert_eq!(s.cursor(), 10);
        s.move_down();
        assert_eq!(s.cursor(), 15);
        s.move_up();
        s.move_up();
        assert_eq!(s.cursor(), 4);
    }

    #[test]
    fn line_start_and_end() {
        let mut s = from_bytes(b"hello\nworld");
        s.active_window_mut().cursor = 8;
        s.move_line_start();
        assert_eq!(s.cursor(), 6);
        s.move_line_end();
        assert_eq!(s.cursor(), 11);
    }

    #[test]
    fn undo_clamps_cursor_to_buffer_len() {
        let mut s = from_bytes(b"");
        s.insert_char('a');
        s.insert_char('b');
        assert_eq!(s.cursor(), 2);
        s.undo();
        assert_eq!(s.cursor(), 1);
        s.undo();
        assert_eq!(s.cursor(), 0);
    }

    #[test]
    fn delete_forward_at_end_is_noop() {
        let mut s = from_bytes(b"abc");
        s.active_window_mut().cursor = 3;
        s.delete_forward();
        assert_eq!(s.active_buffer_len(), 3);
    }

    #[test]
    fn delete_word_backward_removes_previous_word_to_cursor() {
        // Cursor sits at end-of-buffer; deletes back through "world".
        let mut s = from_bytes(b"hello world");
        s.active_window_mut().cursor = 11;
        s.delete_word_backward();
        // `backward_word` lands at the start of the word ("world"
        // begins at byte 6), so we delete bytes 6..11.
        assert_eq!(s.cursor(), 6);
        assert_eq!(s.active_buffer_len(), 6);
    }

    #[test]
    fn delete_word_backward_at_start_of_buffer_is_noop() {
        let mut s = from_bytes(b"hello");
        s.active_window_mut().cursor = 0;
        s.delete_word_backward();
        assert_eq!(s.cursor(), 0);
        assert_eq!(s.active_buffer_len(), 5);
    }

    #[test]
    fn delete_word_forward_removes_next_word_from_cursor() {
        let mut s = from_bytes(b"hello world");
        s.active_window_mut().cursor = 0;
        s.delete_word_forward();
        // `forward_word` lands at the end of the first word (byte 5);
        // delete bytes 0..5. Cursor stays where it was.
        assert_eq!(s.cursor(), 0);
        assert_eq!(s.active_buffer_len(), 6);
    }

    #[test]
    fn delete_word_forward_at_end_of_buffer_is_noop() {
        let mut s = from_bytes(b"hello");
        s.active_window_mut().cursor = 5;
        s.delete_word_forward();
        assert_eq!(s.cursor(), 5);
        assert_eq!(s.active_buffer_len(), 5);
    }

    #[test]
    fn multibyte_navigation() {
        let mut s = from_bytes("héllo".as_bytes());
        s.active_window_mut().cursor = 0;
        s.move_right();
        assert_eq!(s.cursor(), 1);
        s.move_right();
        assert_eq!(s.cursor(), 3);
        s.move_right();
        assert_eq!(s.cursor(), 4);
        s.move_left();
        s.move_left();
        assert_eq!(s.cursor(), 1);
    }

    #[test]
    fn save_with_no_path_announces_via_status() {
        let mut s = fresh();
        s.save();
        assert!(s.status.contains("no file"));
    }

    /// T M10.8 — pins the Day 2 transitional fallback behavior in
    /// [`EditorCore::active_view`].
    ///
    /// **Day 2 → Day 3 transition contract**: while the dispatcher
    /// thread is being wired (Day 3 work), the daemon may set
    /// `active_frontend` to a daemon-attached `FrontendId` whose
    /// `FrontendView` hasn't been registered yet. The fallback to
    /// `FrontendId::LOCAL`'s view keeps single-frontend behavior
    /// observable.
    ///
    /// **Day 3 cleanup**: once
    /// [`EditorCore::register_frontend_view`] is invariantly called
    /// before any event dispatch, this test flips to assert "every
    /// `active_frontend` has its own registered view, no fallback
    /// ever activates." Until then, the fallback is the bridge.
    #[test]
    fn active_view_falls_back_to_local_when_active_frontend_unregistered() {
        let mut s = fresh();
        // Default active_frontend is LOCAL → no fallback yet.
        assert_eq!(s.active_frontend, FrontendId::LOCAL);
        let local_active_window = s.active_view().active;

        // Simulate the Day 2 transitional state: a daemon-attached
        // frontend's id is set as active, but no FrontendView is
        // registered for it (Day 3 work).
        s.active_frontend = FrontendId(42);
        assert!(!s.views.contains_key(&FrontendId(42)));

        // Fallback activates: active_view() returns LOCAL's view.
        let fallback_view = s.active_view();
        assert_eq!(
            fallback_view.active, local_active_window,
            "Day 2 fallback: active_view() returns LOCAL's view when active_frontend has no entry"
        );

        // Same for active_window().
        let win = s.active_window();
        assert_eq!(win.id, local_active_window);
    }

    #[test]
    fn active_view_for_explicit_fid_returns_none_when_unregistered() {
        // T M10.8 — explicit-fid lookups don't fall back. Callers
        // explicitly asking about a specific frontend get a truthful
        // None when that frontend has no state, distinguishing
        // "active by default" from "actually has its own view."
        let s = fresh();
        assert!(s.active_window_for(FrontendId(42)).is_none());
        assert!(s.active_window_for(FrontendId::LOCAL).is_some());
    }

    /// Journey Stage 1a (Q#JR14f): the three decisive rows of the shared
    /// eligibility predicate.
    ///
    /// The `None` row is the one that exists for `commit_to`, and it is
    /// not a "don't care": a directory open validates its destination
    /// *before* the handler creates the buffer that will land there, so
    /// there is no incoming id to compare and a dedicated window must be
    /// refused. Approving it would let the handler claim and paint, and
    /// the display would refuse afterwards — after the mutations the
    /// preflight exists to prevent.
    #[test]
    fn window_accepts_buffer_matrix() {
        let mut s = fresh();
        let window = s.views[&FrontendId::LOCAL].active;
        let current = s.windows[&window].buffer_id;
        let other = s.registry.borrow_mut().create(String::from("other"));

        // Undedicated: accepts anything, including "not decided yet".
        assert!(s.window_accepts_buffer(window, Some(current)));
        assert!(s.window_accepts_buffer(window, Some(other)));
        assert!(s.window_accepts_buffer(window, None));

        s.windows.get_mut(&window).expect("live").params.dedicated = true;

        // Dedicated: only what it already shows.
        assert!(
            s.window_accepts_buffer(window, Some(current)),
            "a dedicated window still accepts the buffer it displays"
        );
        assert!(
            !s.window_accepts_buffer(window, Some(other)),
            "a dedicated window refuses a different buffer"
        );
        assert!(
            !s.window_accepts_buffer(window, None),
            "a dedicated window refuses an as-yet-unbuilt replacement"
        );
    }

    #[test]
    fn register_and_unregister_frontend_view() {
        // T M10.8 — the lifecycle API the dispatcher uses on attach
        // and detach. Wiring lives in `daemon.rs`; this test pins
        // the EditorCore-side semantics.
        let mut s = fresh();
        let fid = FrontendId(7);
        assert!(s.active_window_for(fid).is_none());

        // Build a view referencing the existing scratch window so
        // we don't need a fresh window allocation in this test.
        let local_view = s.views[&FrontendId::LOCAL].clone();
        s.register_frontend_view(fid, local_view);
        s.active_frontend = fid;
        assert!(s.active_window_for(fid).is_some());

        // Unregister drops the entry; explicit lookup returns None.
        s.unregister_frontend_view(fid);
        assert!(s.active_window_for(fid).is_none());

        // Removing the selected frontend restores the always-registered
        // LOCAL view as the ambient fallback.
        assert_eq!(s.active_frontend, FrontendId::LOCAL);
        assert!(s.views.contains_key(&FrontendId::LOCAL));
    }

    #[test]
    fn split_active_creates_a_second_window_on_same_buffer() {
        let mut s = fresh();
        let original = s.active_window_id();
        let new_id = s.split_active(Orientation::Vertical, true);
        assert_ne!(new_id, original);
        assert_eq!(s.windows.len(), 2);
        // Same buffer.
        assert_eq!(s.windows[&original].buffer_id, s.windows[&new_id].buffer_id);
    }

    #[test]
    fn edit_in_one_window_propagates_through_buffer_to_the_other() {
        let mut s = from_bytes(b"abc");
        let _new = s.split_active(Orientation::Vertical, true);
        // Insert via the active window.
        s.active_window_mut().cursor = 3;
        s.insert_char('X');
        // Buffer length is now 4; the *other* window shares the
        // same buffer, so its text view sees the same length.
        assert_eq!(s.active_buffer_len(), 4);
        // The other window's text_view has the same line count,
        // confirming on_edit fired.
        let active = s.active_window_id();
        let other = s.windows.keys().find(|id| **id != active).copied().unwrap();
        assert_eq!(s.windows[&other].text_view.line_count(), 1);
    }

    #[test]
    fn close_active_falls_back_to_remaining_window() {
        let mut s = fresh();
        s.split_active(Orientation::Horizontal, true);
        assert_eq!(s.windows.len(), 2);
        assert!(s.close_active());
        assert_eq!(s.windows.len(), 1);
    }

    #[test]
    fn close_active_refuses_when_only_one_window() {
        let mut s = fresh();
        assert!(!s.close_active());
        assert_eq!(s.windows.len(), 1);
    }

    #[test]
    fn focus_next_round_robins() {
        let mut s = fresh();
        let a = s.active_window_id();
        let _b = s.split_active(Orientation::Vertical, true);
        let _c = s.split_active(Orientation::Horizontal, true);
        // Splits don't move focus; `a` is still active.
        assert_eq!(s.active_window_id(), a);
        let order = s.active_layout().iter_ids();
        assert_eq!(order.len(), 3);
        // Walking N times wraps back to the original.
        for _ in 0..3 {
            s.focus_next();
        }
        assert_eq!(s.active_window_id(), a);
    }

    // ------------------------------------------------------------------
    // F27 / F28 (post-audit-round-5) — daemon-origin CRDT ops are
    // queued on `pending_crdt_ops` so they reach all replicas.
    // ------------------------------------------------------------------

    /// Helper: upgrade the active buffer to CRDT-backed under the
    /// LOCAL peer id (mirrors what the daemon does at attach time
    /// for replica sessions).
    #[cfg(feature = "crdt")]
    fn upgrade_active_to_crdt(s: &mut EditorCore) {
        let buffer_id = s.active_buffer_id();
        let mut reg = s.registry.borrow_mut();
        let buf = reg.get_mut(buffer_id).expect("active buffer present");
        buf.upgrade_to_crdt(crate::crdt::peer_id_from_frontend(
            crate::protocol::FrontendId::LOCAL,
        ))
        .expect("upgrade");
    }

    /// F27 — undo on a CRDT-backed buffer queues the resulting
    /// CRDT op for broadcast.
    #[cfg(feature = "crdt")]
    #[test]
    fn undo_on_crdt_buffer_queues_crdt_op_for_broadcast_f27() {
        let mut s = from_bytes(b"abc");
        upgrade_active_to_crdt(&mut s);
        // Apply an edit so there's something to undo. apply_active_edit
        // also pushes a DaemonKey-origin op.
        s.apply_active_edit(crate::buffer::EditOp::Insert {
            pos: 3,
            bytes: b"X",
        })
        .expect("edit");
        let queued_after_edit = s.pending_crdt_ops.len();
        assert!(queued_after_edit >= 1, "edit must queue a CRDT op");

        // Drain to isolate the undo's queueing.
        s.pending_crdt_ops.clear();
        s.undo();

        assert!(
            !s.pending_crdt_ops.is_empty(),
            "F27: undo on a CRDT-backed buffer must queue a CRDT op for broadcast"
        );
        // Origin must be DaemonKey (broadcast-to-all-replicas).
        let (origin, _, _) = &s.pending_crdt_ops[0];
        assert!(
            matches!(origin, CrdtOpOrigin::DaemonKey),
            "F27: undo's CRDT op must be queued with DaemonKey origin (broadcast to all replicas including active frontend)"
        );
    }

    /// F27 — redo on a CRDT-backed buffer queues the resulting
    /// CRDT op for broadcast.
    #[cfg(feature = "crdt")]
    #[test]
    fn redo_on_crdt_buffer_queues_crdt_op_for_broadcast_f27() {
        let mut s = from_bytes(b"abc");
        upgrade_active_to_crdt(&mut s);
        s.apply_active_edit(crate::buffer::EditOp::Insert {
            pos: 3,
            bytes: b"X",
        })
        .expect("edit");
        s.undo();
        s.pending_crdt_ops.clear();
        s.redo();
        assert!(
            !s.pending_crdt_ops.is_empty(),
            "F27: redo on a CRDT-backed buffer must queue a CRDT op for broadcast"
        );
        let (origin, _, _) = &s.pending_crdt_ops[0];
        assert!(matches!(origin, CrdtOpOrigin::DaemonKey));
    }

    /// F27 — undo on a non-CRDT buffer is a no-op for the broadcast
    /// queue (the buffer produced no `crdt_op` on the Edit).
    #[test]
    fn undo_on_non_crdt_buffer_does_not_queue_crdt_op_f27() {
        let mut s = from_bytes(b"abc");
        s.apply_active_edit(crate::buffer::EditOp::Insert {
            pos: 3,
            bytes: b"X",
        })
        .expect("edit");
        // Non-CRDT — apply_active_edit's pending push is a no-op
        // (Edit::crdt_op is None). Confirm precondition then undo.
        assert!(s.pending_crdt_ops.is_empty());
        s.undo();
        assert!(
            s.pending_crdt_ops.is_empty(),
            "F27: undo on a non-CRDT buffer must not produce a phantom queue entry"
        );
    }

    // ---- jump ring (T M4.5 L1) -----------------------------------------

    #[test]
    fn jump_back_returns_false_on_empty_ring() {
        let mut s = from_bytes(b"abc");
        s.active_window_mut().cursor = 2;
        assert!(!s.jump_back(), "empty ring must not move the cursor");
        assert_eq!(s.cursor(), 2);
    }

    #[test]
    fn push_then_jump_back_restores_cursor() {
        let mut s = from_bytes(b"line one\nline two\nline three");
        s.active_window_mut().cursor = 3;
        s.push_jump();
        s.active_window_mut().cursor = 20;
        assert!(s.jump_back());
        assert_eq!(s.cursor(), 3);
        // Ring is now empty; a second pop is a no-op.
        assert!(!s.jump_back());
    }

    #[test]
    fn jump_back_clamps_to_shortened_buffer() {
        let mut s = from_bytes(b"abcdefghij");
        s.active_window_mut().cursor = 9;
        s.push_jump();
        // Truncate the buffer so the recorded position is past EOF.
        s.apply_active_edit(crate::buffer::EditOp::Delete {
            range: Range::new(2, 10),
        })
        .expect("delete");
        assert!(s.jump_back());
        assert_eq!(
            s.cursor(),
            s.active_buffer_len(),
            "stale position must clamp to the current buffer length"
        );
    }

    #[test]
    fn jump_ring_is_bounded_and_evicts_oldest() {
        let mut s = from_bytes(b"0123456789");
        for i in 0..(EditorCore::JUMP_RING_CAP + 10) {
            s.active_window_mut().cursor = (i % 10) as u64;
            s.push_jump();
        }
        // Bottom-panel arc (Q#BP11c): the cap applies independently to
        // each frontend's own vector, with today's oldest-entry eviction.
        assert_eq!(
            s.jump_ring[&FrontendId::LOCAL].len(),
            EditorCore::JUMP_RING_CAP,
            "ring must stay bounded at JUMP_RING_CAP"
        );
    }

    #[test]
    fn jump_back_skips_removed_buffer() {
        let mut s = from_bytes(b"original");
        // Record a jump on a second buffer, then remove that buffer.
        let doomed = s.registry.borrow_mut().create_from_bytes("doomed", b"x");
        s.switch_active_buffer(doomed).expect("switch");
        s.active_window_mut().cursor = 1;
        s.push_jump();
        // Switch back and record a live origin too.
        let original = *s.registry.borrow().ids().first().expect("original id");
        s.switch_active_buffer(original).expect("switch back");
        s.active_window_mut().cursor = 4;
        s.push_jump();
        s.active_window_mut().cursor = 0;
        // Drop the doomed buffer: its ring entry is now stale.
        s.registry.borrow_mut().remove(doomed).expect("remove");
        // First pop lands on the live `original` origin.
        assert!(s.jump_back());
        assert_eq!(s.active_buffer_id(), original);
        assert_eq!(s.cursor(), 4);
        // Next pop would be the stale `doomed` entry — skipped, ring empties.
        assert!(!s.jump_back());
    }

    // ---- incremental search (Q#SR5) ------------------------------------

    fn type_query(s: &mut EditorCore, q: &str) {
        for ch in q.chars() {
            s.search_input_char(ch);
        }
    }

    #[test]
    fn search_begin_then_type_highlights_from_origin() {
        let mut s = from_bytes(b"foo bar foo baz foo");
        let bid = s.active_buffer_id();
        s.active_window_mut().cursor = 0;
        s.search_begin(true, false);
        assert!(s.search_active());
        type_query(&mut s, "foo");
        // Three matches: 0..3, 8..11, 16..19; first (at/after origin 0)
        // is active and the cursor sits on it.
        assert_eq!(s.search_match_summary(), (Some(0), 3));
        assert_eq!(s.cursor(), 0);
        let guard = s.search_store.lock().expect("store");
        assert!(!guard.is_stale(bid));
        assert_eq!(guard.for_buffer(bid).expect("entry").len(), 3);
    }

    #[test]
    fn search_step_walks_matches_and_wraps() {
        let mut s = from_bytes(b"foo bar foo baz foo");
        s.active_window_mut().cursor = 0;
        s.search_begin(true, false);
        type_query(&mut s, "foo");
        assert_eq!(s.cursor(), 0);
        s.search_step(true);
        assert_eq!((s.search_match_summary(), s.cursor()), ((Some(1), 3), 8));
        s.search_step(true);
        assert_eq!(s.cursor(), 16);
        s.search_step(true); // wraps to the first match
        assert_eq!(s.cursor(), 0);
        s.search_step(false); // backward wraps to the last
        assert_eq!(s.cursor(), 16);
    }

    #[test]
    fn search_focuses_first_match_at_or_after_origin() {
        let mut s = from_bytes(b"foo bar foo");
        s.active_window_mut().cursor = 5; // inside "bar"
        s.search_begin(true, false);
        type_query(&mut s, "foo");
        // First match with start >= 5 is the one at byte 8.
        assert_eq!(s.cursor(), 8);
        assert_eq!(s.search_match_summary(), (Some(1), 2));
    }

    #[test]
    fn search_cancel_restores_origin_and_clears_store() {
        let mut s = from_bytes(b"foo bar foo");
        let bid = s.active_buffer_id();
        s.active_window_mut().cursor = 5;
        s.search_begin(true, false);
        type_query(&mut s, "foo");
        assert_eq!(s.cursor(), 8);
        s.search_finish(false); // cancel
        assert!(!s.search_active());
        assert_eq!(s.cursor(), 5, "cancel restores the pre-search cursor");
        assert!(
            s.search_store
                .lock()
                .expect("store")
                .for_buffer(bid)
                .is_none(),
            "cancel clears the matches"
        );
    }

    #[test]
    fn search_accept_keeps_cursor_and_matches() {
        let mut s = from_bytes(b"foo bar foo");
        let bid = s.active_buffer_id();
        s.active_window_mut().cursor = 0;
        s.search_begin(true, false);
        type_query(&mut s, "foo");
        s.search_step(true); // focus the match at byte 8
        assert_eq!(s.cursor(), 8);
        s.search_finish(true); // accept
        assert!(!s.search_active());
        assert_eq!(s.cursor(), 8, "accept keeps the cursor on the match");
        assert!(
            s.search_store
                .lock()
                .expect("store")
                .for_buffer(bid)
                .is_some(),
            "accept keeps matches for highlight + navigation"
        );
    }

    #[test]
    fn stale_matches_fail_closed_for_step_and_summary() {
        // Q#AI8: once an edit marks matches stale, the highlights are
        // suppressed — stepping and the n/m prompt must fail closed
        // with them instead of navigating/advertising dead offsets.
        let mut s = from_bytes(b"foo bar foo");
        s.active_window_mut().cursor = 0;
        s.search_begin(true, false);
        type_query(&mut s, "foo");
        s.search_finish(true); // accept keeps the matches
        assert_eq!(s.search_match_summary(), (Some(0), 2));
        s.active_window_mut().cursor = 0;
        assert!(s.insert_char('x'), "plain insert lands");
        assert_eq!(
            s.search_match_summary(),
            (None, 0),
            "stale counts must not reach the prompt"
        );
        let before = s.cursor();
        s.search_step(true);
        assert_eq!(s.cursor(), before, "stale step is a no-op");
    }

    #[test]
    fn live_search_origin_translates_through_local_edits() {
        // Q#AI8: the session origin is a raw byte offset; an edit
        // before it must shift it (right-gravity) so cancel restores
        // the same TEXT position, not the same number.
        let mut s = from_bytes(b"foo bar foo");
        s.active_window_mut().cursor = 5;
        s.search_begin(true, false); // origin byte 5
        type_query(&mut s, "foo");
        assert_eq!(s.cursor(), 8, "focused the match after the origin");
        s.active_window_mut().cursor = 0;
        assert!(s.insert_char('x'));
        assert!(s.insert_char('y'));
        s.search_finish(false); // cancel
        assert_eq!(
            s.cursor(),
            7,
            "cancel restores the translated origin (5 + 2 inserted bytes)"
        );
    }

    #[test]
    fn live_search_recompute_focuses_from_the_translated_origin() {
        let mut s = from_bytes(b"foo bar foo");
        s.active_window_mut().cursor = 1;
        s.search_begin(true, false); // origin byte 1
        type_query(&mut s, "fo");
        assert_eq!(s.cursor(), 8, "first match at/after the origin");
        s.active_window_mut().cursor = 0;
        assert!(s.insert_char('x'));
        assert!(s.insert_char('y'));
        assert!(s.insert_char('z'));
        // "xyzfoo bar foo": origin 1 -> 4. Growing the query recomputes
        // and must focus from the TRANSLATED origin: the match at 11,
        // not the pre-edit offset 1's neighbor at 3.
        type_query(&mut s, "o");
        assert_eq!(
            s.cursor(),
            11,
            "recompute focuses the first match at/after the translated origin"
        );
    }

    #[test]
    fn notify_buffer_edit_marks_stale_and_translates_the_origin() {
        // Q#AI8 at the direct-edit seam (Lua mutators / applied CRDT
        // ops): notify_buffer_edit must invalidate matches and shift
        // the live origin exactly like apply_active_edit does.
        let mut s = from_bytes(b"foo bar foo");
        let bid = s.active_buffer_id();
        s.active_window_mut().cursor = 1;
        s.search_begin(true, false); // origin byte 1
        type_query(&mut s, "foo");
        let edit = {
            let mut reg = s.registry.borrow_mut();
            let buffer = reg.get_mut(bid).expect("buffer");
            buffer
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"zz",
                })
                .expect("direct insert")
        };
        s.notify_buffer_edit(bid, &edit);
        assert!(
            s.search_store.lock().expect("store").is_stale(bid),
            "direct edits mark the matches stale"
        );
        s.search_finish(false); // cancel
        assert_eq!(
            s.cursor(),
            3,
            "cancel restores the origin translated through the direct edit"
        );
    }

    #[test]
    fn undo_and_redo_stale_accepted_search_navigation() {
        // Q#AI8 (PR #109 round 1): history edits move bytes like any
        // other edit — undo/redo must invalidate search state.
        let mut s = from_bytes(b"foo bar foo");
        s.active_window_mut().cursor = 0;
        assert!(s.insert_char('x')); // the history entry: "xfoo bar foo"
        s.search_begin(true, false);
        type_query(&mut s, "foo");
        s.search_finish(true); // accept
        assert_eq!(s.search_match_summary(), (Some(0), 2));
        s.undo(); // back to "foo bar foo": offsets moved
        assert_eq!(
            s.search_match_summary(),
            (None, 0),
            "undo stales the accepted matches"
        );
        let before = s.cursor();
        s.search_step(true);
        assert_eq!(s.cursor(), before, "stale step is a no-op after undo");

        // Redo the same way: refresh the matches first (a fresh set
        // clears staleness), then redo must stale them again.
        s.search_begin(true, false);
        type_query(&mut s, "foo");
        s.search_finish(true);
        assert_eq!(s.search_match_summary().1, 2);
        s.redo(); // forward to "xfoo bar foo" again
        assert_eq!(
            s.search_match_summary(),
            (None, 0),
            "redo stales the accepted matches"
        );
    }

    #[test]
    fn undo_translates_the_live_search_origin() {
        let mut s = from_bytes(b"foo bar foo");
        s.active_window_mut().cursor = 0;
        assert!(s.insert_char('x')); // "xfoo bar foo"
        s.active_window_mut().cursor = 5;
        s.search_begin(true, false); // origin 5 ('b' of "bar")
        type_query(&mut s, "foo");
        s.undo(); // removes the 'x' at 0: origin must shift to 4
        s.search_finish(false); // cancel
        assert_eq!(
            s.cursor(),
            4,
            "cancel lands on the origin translated through the undo"
        );
    }

    #[test]
    fn origin_translates_through_deletes_on_both_paths() {
        // Dispatch path (apply_active_edit): backspace before the
        // origin.
        let mut s = from_bytes(b"foo bar foo");
        s.active_window_mut().cursor = 5;
        s.search_begin(true, false); // origin 5
        type_query(&mut s, "foo");
        s.active_window_mut().cursor = 2;
        s.backspace(); // deletes byte 1: origin 5 -> 4
        s.search_finish(false);
        assert_eq!(s.cursor(), 4, "origin shifted left by the deleted byte");

        // Direct-notification path: a delete edit through
        // notify_buffer_edit.
        let mut s = from_bytes(b"foo bar foo");
        let bid = s.active_buffer_id();
        s.active_window_mut().cursor = 5;
        s.search_begin(true, false); // origin 5
        type_query(&mut s, "foo");
        let edit = {
            let mut reg = s.registry.borrow_mut();
            let buffer = reg.get_mut(bid).expect("buffer");
            buffer
                .apply_edit(crate::buffer::EditOp::Delete {
                    range: Range::new(0, 2),
                })
                .expect("direct delete")
        };
        s.notify_buffer_edit(bid, &edit);
        s.search_finish(false);
        assert_eq!(
            s.cursor(),
            3,
            "origin shifted left by the two directly deleted bytes"
        );
    }

    #[test]
    fn active_search_fails_closed_while_stale_and_recovers_on_retype() {
        // Q#AI8 during a LIVE session: an external edit mid-search
        // makes step and summary fail closed; the next pattern
        // keystroke recomputes (set clears staleness) and resumes.
        let mut s = from_bytes(b"foo bar foo");
        let bid = s.active_buffer_id();
        s.active_window_mut().cursor = 0;
        s.search_begin(true, false);
        type_query(&mut s, "fo");
        assert_eq!(s.search_match_summary().1, 2);
        let edit = {
            let mut reg = s.registry.borrow_mut();
            let buffer = reg.get_mut(bid).expect("buffer");
            buffer
                .apply_edit(crate::buffer::EditOp::Insert {
                    pos: 0,
                    bytes: b"zz",
                })
                .expect("direct insert")
        };
        s.notify_buffer_edit(bid, &edit);
        assert_eq!(
            s.search_match_summary(),
            (None, 0),
            "summary fails closed mid-search"
        );
        let before = s.cursor();
        s.search_step(true);
        assert_eq!(s.cursor(), before, "step fails closed mid-search");
        // Growing the query recomputes against the current text.
        type_query(&mut s, "o");
        assert_eq!(
            s.search_match_summary().1,
            2,
            "the next pattern keystroke refreshes the match set"
        );
        assert_eq!(s.cursor(), 2, "focus lands from the translated origin");
        s.search_step(true);
        assert_eq!(s.cursor(), 10, "stepping resumes after the refresh");
    }

    #[test]
    fn search_backspace_widens_the_match_set() {
        let mut s = from_bytes(b"fo foo food");
        s.active_window_mut().cursor = 0;
        s.search_begin(true, false);
        type_query(&mut s, "foo"); // matches "foo" at 3..6, 7..10
        assert_eq!(s.search_match_summary().1, 2);
        s.search_backspace(); // query "fo"
        assert_eq!(s.search_query(), "fo");
        assert_eq!(s.search_match_summary().1, 3);
    }

    #[test]
    fn search_smart_case_is_case_sensitive_with_uppercase() {
        let mut s = from_bytes(b"Foo foo FOO");
        s.active_window_mut().cursor = 0;
        s.search_begin(true, false);
        type_query(&mut s, "Foo"); // uppercase => case-sensitive
        assert_eq!(s.search_match_summary().1, 1);
        s.search_backspace();
        s.search_backspace();
        s.search_backspace();
        type_query(&mut s, "foo"); // lowercase => smart-case folds all
        assert_eq!(s.search_match_summary().1, 3);
    }

    #[test]
    fn edit_marks_accepted_matches_stale() {
        let mut s = from_bytes(b"foo foo");
        let bid = s.active_buffer_id();
        s.active_window_mut().cursor = 0;
        s.search_begin(true, false);
        type_query(&mut s, "foo");
        s.search_finish(true); // matches persist after accept
        assert!(!s.search_store.lock().expect("store").is_stale(bid));
        s.insert_char('x'); // any edit invalidates the match offsets
        assert!(
            s.search_store.lock().expect("store").is_stale(bid),
            "an edit marks the buffer's matches stale (linger fix)"
        );
    }

    // ---- regex search (Q#RX3) ------------------------------------------

    #[test]
    fn regex_search_matches_pattern() {
        let mut s = from_bytes(b"a1 b2 c3");
        s.active_window_mut().cursor = 0;
        s.search_begin(true, true);
        assert!(s.search_is_regex());
        type_query(&mut s, r"\d");
        assert_eq!(s.search_match_summary().1, 3, "\\d matches 1, 2, 3");
        assert!(!s.search_is_invalid());
    }

    #[test]
    fn regex_invalid_pattern_flags_and_recovers() {
        let mut s = from_bytes(b"foo");
        s.active_window_mut().cursor = 0;
        s.search_begin(true, true);
        type_query(&mut s, "fo("); // unbalanced group mid-typing
        assert!(s.search_is_invalid(), "incomplete group is invalid");
        assert_eq!(s.search_match_summary().1, 0, "invalid ⇒ no matches");
        type_query(&mut s, "o)"); // completes the group: regex fo(o) → "foo"
        assert!(!s.search_is_invalid(), "valid pattern recovers");
        assert_eq!(s.search_match_summary().1, 1);
    }

    #[test]
    fn toggle_regex_reinterprets_the_query() {
        let mut s = from_bytes(b"a.b axb");
        s.active_window_mut().cursor = 0;
        s.search_begin(true, false); // literal
        type_query(&mut s, "a.b");
        assert!(!s.search_is_regex());
        assert_eq!(
            s.search_match_summary().1,
            1,
            "literal '.' matches only a.b"
        );
        s.search_toggle_regex(); // → regex
        assert!(s.search_is_regex());
        assert_eq!(s.search_match_summary().1, 2, "regex '.' also matches axb");
        s.search_toggle_regex(); // back to literal
        assert!(!s.search_is_regex());
        assert_eq!(s.search_match_summary().1, 1);
    }

    // ---- in-buffer completion popup (Arc 1a) --------------------------------

    fn text_of(s: &EditorCore) -> String {
        let id = s.active_buffer_id();
        let reg = s.registry.borrow();
        let buf = reg.get(id).expect("active buffer present");
        let mut bytes = vec![0u8; buf.len() as usize];
        if !bytes.is_empty() {
            buf.snapshot_rope().slice(0, buf.len(), &mut bytes);
        }
        String::from_utf8(bytes).expect("test buffers are UTF-8")
    }

    fn open_popup(s: &mut EditorCore, anchor: u64, prefix: &str, insert_text: &str) {
        let state = crate::completion::CompletionPopupState::new(
            s.active_buffer_id(),
            anchor,
            prefix.to_owned(),
            vec![crate::completion::PopupCandidate {
                label: insert_text.to_owned(),
                kind: crate::completion::CompletionItemKind::Text,
                detail: None,
                insert_text: insert_text.to_owned(),
            }],
            1,
        )
        .expect("non-empty candidate list");
        s.completion_popup_open(state);
    }

    #[test]
    fn completion_popup_open_attaches_self_suppressing_overlay() {
        let mut s = from_bytes(b"he\n");
        s.active_window_mut().cursor = 2;
        open_popup(&mut s, 0, "he", "hello");
        assert!(s.completion_popup_is_open());
        assert!(
            s.active_window()
                .overlay_kinds()
                .contains(&"completion-popup")
        );
        // Re-opening dedups the overlay by kind.
        open_popup(&mut s, 0, "he", "hello");
        let kinds = s.active_window().overlay_kinds();
        assert_eq!(
            kinds.iter().filter(|k| **k == "completion-popup").count(),
            1
        );
    }

    #[test]
    fn completion_popup_validate_survives_word_growth_and_empty_prefix() {
        let mut s = from_bytes(b"he world\n");
        s.active_window_mut().cursor = 2;
        open_popup(&mut s, 0, "he", "hello");
        s.completion_popup_validate();
        assert!(s.completion_popup_is_open(), "prefix `he` holds");
        // Typing extends the word: still valid.
        s.insert_char('l');
        s.completion_popup_validate();
        assert!(s.completion_popup_is_open(), "prefix `hel` holds");
        // Trigger-char shape (cursor == anchor, empty prefix) holds too.
        s.completion_popup_close();
        s.active_window_mut().cursor = 2;
        open_popup(&mut s, 2, "", "llo");
        s.completion_popup_validate();
        assert!(s.completion_popup_is_open(), "empty prefix at anchor holds");
    }

    #[test]
    fn completion_popup_validate_closes_when_invariant_breaks() {
        let mut s = from_bytes(b"he world\n");
        s.active_window_mut().cursor = 2;
        open_popup(&mut s, 0, "he", "hello");
        // Cursor moved past the word: `[anchor..cursor]` spans a space.
        s.active_window_mut().cursor = 4;
        s.completion_popup_validate();
        assert!(!s.completion_popup_is_open(), "non-word bytes close it");

        // Cursor moved before the anchor.
        s.active_window_mut().cursor = 2;
        open_popup(&mut s, 2, "", "x");
        s.active_window_mut().cursor = 1;
        s.completion_popup_validate();
        assert!(!s.completion_popup_is_open(), "cursor < anchor closes it");

        // Session bound to a buffer that is not the active one.
        let other = s.registry.borrow_mut().create("*other*");
        let state = crate::completion::CompletionPopupState::new(
            other,
            0,
            String::new(),
            vec![crate::completion::PopupCandidate {
                label: "x".into(),
                kind: crate::completion::CompletionItemKind::Text,
                detail: None,
                insert_text: "x".into(),
            }],
            1,
        )
        .unwrap();
        s.completion_popup_open(state);
        s.completion_popup_validate();
        assert!(!s.completion_popup_is_open(), "wrong buffer closes it");
    }

    #[test]
    fn completion_popup_accept_replaces_prefix_as_one_undo_step() {
        let mut s = from_bytes(b"he and more\n");
        s.active_window_mut().cursor = 2;
        open_popup(&mut s, 0, "he", "hello_world");
        assert!(s.completion_popup_accept());
        assert_eq!(text_of(&s), "hello_world and more\n");
        assert_eq!(s.active_window().cursor, 11);
        assert!(!s.completion_popup_is_open(), "accept closes the popup");
        // Q#C7: the replace is a single edit — one undo restores the
        // original text (not an intermediate delete-then-insert state).
        s.undo();
        assert_eq!(text_of(&s), "he and more\n");
    }

    #[test]
    fn completion_popup_accept_empty_prefix_inserts_at_anchor() {
        let mut s = from_bytes(b"x.\n");
        s.active_window_mut().cursor = 2;
        open_popup(&mut s, 2, "", "method");
        assert!(s.completion_popup_accept());
        assert_eq!(text_of(&s), "x.method\n");
        assert_eq!(s.active_window().cursor, 8);
    }

    #[test]
    fn completion_popup_accept_is_noop_when_session_stale() {
        let mut s = from_bytes(b"he world\n");
        s.active_window_mut().cursor = 2;
        open_popup(&mut s, 0, "he", "hello");
        // Simulate a race: the cursor left the word before accept ran.
        s.active_window_mut().cursor = 5;
        assert!(!s.completion_popup_accept());
        assert_eq!(text_of(&s), "he world\n", "buffer untouched");
        assert!(!s.completion_popup_is_open(), "stale accept still closes");
    }

    #[test]
    fn completion_popup_validate_closes_on_window_focus_change() {
        // Two splits on the SAME buffer: the session is window-scoped,
        // so moving focus (buffer unchanged!) must invalidate it ---
        // this is also what keeps the persistent overlay in the other
        // split from painting a popup it doesn't own.
        let mut s = from_bytes(b"he world\n");
        s.split_active(Orientation::Horizontal, true);
        s.active_window_mut().cursor = 2;
        open_popup(&mut s, 0, "he", "hello");
        s.completion_popup_validate();
        assert!(s.completion_popup_is_open(), "session holds in its window");
        s.focus_next();
        s.completion_popup_validate();
        assert!(
            !s.completion_popup_is_open(),
            "focus change closes the session even with the same buffer"
        );
    }

    // ---- query-replace core (Arc 2) ----------------------------------------

    #[test]
    fn query_replace_all_replaces_and_counts() {
        let mut s = from_bytes(b"foo foo foo\n");
        s.query_replace_begin("foo".into(), "bar".into(), false);
        assert!(s.query_replace_active(), "session opens on the first match");
        s.query_replace_all();
        assert_eq!(text_of(&s), "bar bar bar\n");
        assert!(!s.query_replace_active(), "! finishes the session");
        assert_eq!(s.status, "Replaced 3 occurrences");
    }

    #[test]
    fn query_replace_growing_replacement_does_not_loop() {
        // The a→aa shape: replacing must not re-match the inserted text.
        let mut s = from_bytes(b"a a a\n");
        s.query_replace_begin("a".into(), "aa".into(), false);
        s.query_replace_all();
        assert_eq!(text_of(&s), "aa aa aa\n", "each 'a' replaced exactly once");
        assert_eq!(s.status, "Replaced 3 occurrences");
    }

    #[test]
    fn query_replace_empty_to_deletes() {
        let mut s = from_bytes(b"a-b-c\n");
        s.query_replace_begin("-".into(), String::new(), false);
        s.query_replace_all();
        assert_eq!(text_of(&s), "abc\n", "empty replacement deletes matches");
    }

    #[test]
    fn query_replace_skip_then_replace_is_selective() {
        let mut s = from_bytes(b"x x x\n");
        s.query_replace_begin("x".into(), "y".into(), false);
        s.query_replace_skip(); // leave the first x
        s.query_replace_replace(); // replace the second x, advance to third
        s.query_replace_replace_and_quit(); // replace the third, quit
        assert_eq!(text_of(&s), "x y y\n", "first skipped, rest replaced");
        assert!(!s.query_replace_active());
    }

    #[test]
    fn query_replace_nothing_matched_restores_origin() {
        let mut s = from_bytes(b"hello world\n");
        s.active_window_mut().cursor = 6; // on "world"
        s.query_replace_begin("zzz".into(), "q".into(), false);
        assert!(
            !s.query_replace_active(),
            "no match → session never stays open"
        );
        assert_eq!(text_of(&s), "hello world\n", "buffer untouched");
        assert_eq!(s.active_window().cursor, 6, "origin cursor restored");
        assert_eq!(s.status, "No matches for 'zzz'");
    }

    #[test]
    fn query_replace_starts_from_cursor_forward() {
        let mut s = from_bytes(b"k _ k\n");
        s.active_window_mut().cursor = 2; // between the two k's
        s.query_replace_begin("k".into(), "K".into(), false);
        s.query_replace_all();
        assert_eq!(text_of(&s), "k _ K\n", "only the match at/after point");
    }

    #[test]
    fn query_replace_aborts_when_active_buffer_changes() {
        // The wrong-buffer merge-blocker: a session started in buffer X
        // must never apply its match to a buffer that became active
        // mid-session. Focus drifts (a click / cross-frontend key), then
        // the next replace key aborts safely instead of corrupting.
        let mut s = from_bytes(b"foo foo\n");
        let x = s.active_buffer_id();
        s.query_replace_begin("foo".into(), "bar".into(), false);
        assert!(s.query_replace_active());

        // Switch the active buffer to an unrelated one (focus drift).
        let y = s.registry.borrow_mut().create("*other*");
        {
            let reg = s.registry.borrow();
            let buf = reg.get(y).unwrap();
            let tv = crate::text_view::TextView::new(buf);
            drop(reg);
            let win = s.active_window_mut();
            win.buffer_id = y;
            win.text_view = tv;
            win.cursor = 0;
        }
        assert_eq!(s.active_buffer_id(), y);

        s.query_replace_replace(); // the y/replace key while drifted
        assert!(!s.query_replace_active(), "drift aborts the session");
        assert_eq!(s.status, "query-replace aborted: active buffer changed");
        // Neither buffer was mutated by the aborted replace.
        {
            let reg = s.registry.borrow();
            let bx = reg.get(x).unwrap();
            let mut xb = vec![0u8; bx.len() as usize];
            bx.snapshot_rope().slice(0, bx.len(), &mut xb);
            assert_eq!(&xb, b"foo foo\n", "origin buffer X untouched");
            let by = reg.get(y).unwrap();
            assert_eq!(by.len(), 0, "unrelated buffer Y untouched");
        }
    }

    #[test]
    fn query_replace_regex_replaces_and_invalid_refuses() {
        let mut s = from_bytes(b"a1 b2 c3\n");
        s.query_replace_begin("[0-9]".into(), "#".into(), true);
        s.query_replace_all();
        assert_eq!(text_of(&s), "a# b# c#\n", "regex matches digits");

        // Invalid regex refuses to start and leaves a status.
        let mut s2 = from_bytes(b"abc\n");
        s2.query_replace_begin("(unclosed".into(), "x".into(), true);
        assert!(
            !s2.query_replace_active(),
            "invalid regex never opens a session"
        );
        assert!(s2.status.starts_with("Invalid regex"));
    }
}
