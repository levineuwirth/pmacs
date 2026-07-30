//! Wire message envelopes — moved from `pmacs::protocol` in session 1
//! of the `pmacs-gpu` arc. Contains the input event types
//! (`Key`/`Modifiers`/`KeyEvent`, mouse types, `FrontendEvent`), the
//! instance-side message types (`InstanceMessage`, `CursorState`,
//! `InstanceSignal`, `GoodbyeReason`, `SelectionSnapshot`), the
//! `SemanticFrame` family (`StyleSpan`/`StyleSegment`, `Decoration`,
//! `Adornment*`, `ResourceBody`), and the attach handshake
//! (`Hello`/`AttachRequest`/`InstanceCapabilities`/`FrontendCapabilities`/
//! `NegotiatedCapabilities`/`InstanceIdentity` + `PROTOCOL_VERSION` /
//! `SUPPORTED_PROTOCOL_VERSIONS`).
//!
//! The original `pmacs::protocol` module keeps internal CLI / binding
//! types (`AttachTarget`, `AttachError`, `AttachmentHandle`, the
//! `crossterm_translate` submodule) plus the existing wire-format
//! roundtrip tests; it re-exports everything below so existing
//! `crate::protocol::*` imports stay stable.

use crate::cell::{Cell, CellCoord, CellSize, DiffSpan};
use crate::ids::{ByteRange, FrontendId};

// ---------------------------------------------------------------------------
// Key encoding
// ---------------------------------------------------------------------------

/// Key code, normalized away from any specific terminal protocol.
///
/// `Char` covers printable input. The named variants cover the keys
/// terminals report distinctly (arrows, function keys, etc.). `Unknown`
/// is the escape hatch: a key the protocol layer cannot encode in
/// any of the named variants is preserved as a u32 sentinel so it
/// can round-trip through serialization without becoming an error.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Key {
    /// A printable character. The character is the user-visible
    /// codepoint after layout / IME processing.
    Char(char),
    /// A function key. `n` is 1-based: `F(1)` is F1.
    F(u8),
    /// Backspace / `^H`.
    Backspace,
    /// Enter / Return / `^M`.
    Enter,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Home key.
    Home,
    /// End key.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Tab.
    Tab,
    /// Shift-Tab.
    BackTab,
    /// Forward delete.
    Delete,
    /// Insert.
    Insert,
    /// Escape.
    Escape,
    /// Caps Lock.
    CapsLock,
    /// Scroll Lock.
    ScrollLock,
    /// Num Lock.
    NumLock,
    /// Print Screen.
    PrintScreen,
    /// Pause / Break.
    Pause,
    /// Menu / context-menu key.
    Menu,
    /// Numeric-keypad center key.
    KeypadBegin,
    /// The "null" keycode (terminal-protocol artifact).
    Null,
    /// A key the protocol layer does not recognize. The `u32`
    /// preserves whatever sentinel value the upstream layer attached
    /// (e.g. a media-key code from kitty's keyboard protocol). Round-trips
    /// through serialization but is not actionable by commands.
    Unknown(u32),
}

/// The nine built-in auto-pair characters (docs/auto-pairing-framing.md
/// Q#AP1). Both frontends' optimistic classifiers exclude these so they
/// always round-trip through daemon dispatch: the typed opener and the
/// pairing hook's closer then land as adjacent daemon-peer undo units,
/// dispatch-path CUA type-over applies, and skip-over-close never
/// paints a transient duplicate. Shared here — not duplicated per
/// frontend — because a frontend that drifts from this set silently
/// re-degrades pair undo to the cross-peer mixed-history case.
pub const BUILTIN_PAIR_CHARS: [char; 9] = ['(', ')', '[', ']', '{', '}', '"', '\'', '`'];

/// True when `c` is one of [`BUILTIN_PAIR_CHARS`].
#[must_use]
pub fn is_builtin_pair_char(c: char) -> bool {
    BUILTIN_PAIR_CHARS.contains(&c)
}
/// Maximum number of live statusline providers and wire segments.
pub const MAX_STATUSLINE_PROVIDERS: usize = 64;

/// Maximum UTF-8 byte length of a statusline provider's display name.
pub const MAX_STATUSLINE_PROVIDER_NAME_BYTES: usize = 256;

/// Maximum UTF-8 byte length of a statusline segment face name.
pub const MAX_STATUSLINE_FACE_BYTES: usize = 256;

/// Maximum UTF-8 byte length of one statusline segment's text.
pub const MAX_STATUSLINE_SEGMENT_BYTES: usize = 1024;

/// Maximum aggregate UTF-8 text bytes in one statusline payload.
pub const MAX_STATUSLINE_TOTAL_TEXT_BYTES: usize = 64 * 1024;

/// True when `name` belongs to the reserved UI-face namespace.
#[must_use]
pub fn is_ui_face_name(name: &str) -> bool {
    name == "ui" || name.starts_with("ui.")
}

/// True when `name` is the modeline face or one of its children.
#[must_use]
pub fn is_modeline_face_name(name: &str) -> bool {
    name == "ui.modeline" || name.starts_with("ui.modeline.")
}

/// Modifier-key set. Bit-flag encoding for compact wire shape.
///
/// `META` corresponds to the "logo" / "super" key on most keyboards.
/// `HYPER` is reserved for the rare keyboards that distinguish it
/// from `META` (kitty's keyboard protocol surfaces both).
#[derive(
    Copy, Clone, Eq, PartialEq, Hash, Debug, Default, serde::Serialize, serde::Deserialize,
)]
pub struct Modifiers(u8);

impl Modifiers {
    /// Empty set: no modifiers held.
    pub const NONE: Modifiers = Modifiers(0);
    /// Shift.
    pub const SHIFT: Modifiers = Modifiers(1 << 0);
    /// Control.
    pub const CTRL: Modifiers = Modifiers(1 << 1);
    /// Alt / Option.
    pub const ALT: Modifiers = Modifiers(1 << 2);
    /// Meta / Super / Logo / Command.
    pub const META: Modifiers = Modifiers(1 << 3);
    /// Hyper. Distinguished from `META` only on keyboards that
    /// surface both (kitty's keyboard protocol).
    pub const HYPER: Modifiers = Modifiers(1 << 4);

    /// Construct from a raw bit set. Bits outside the defined range
    /// are silently masked off so a future-extended wire cannot smuggle
    /// undefined bits past current decoders.
    #[must_use]
    pub const fn from_bits_truncate(bits: u8) -> Self {
        Self(bits & 0b0001_1111)
    }

    /// Raw bit set.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether `self` includes every bit set in `other`.
    #[must_use]
    pub const fn contains(self, other: Modifiers) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Whether no modifiers are held.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Modifiers;
    fn bitor(self, rhs: Modifiers) -> Modifiers {
        Modifiers(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, rhs: Modifiers) {
        self.0 |= rhs.0;
    }
}

/// A keyboard event.
#[derive(Copy, Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KeyEvent {
    /// Frontend that produced the event.
    pub frontend_id: FrontendId,
    /// The key code.
    pub key: Key,
    /// Modifier set held when the key was pressed.
    pub mods: Modifiers,
    /// Monotonic timestamp at which the frontend captured the event.
    /// Zero means "no timestamp available" (e.g. test-synthesized
    /// events).
    pub timestamp_ns: u64,
}

// ---------------------------------------------------------------------------
// Mouse encoding
// ---------------------------------------------------------------------------

/// Mouse button.
#[derive(Copy, Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MouseButton {
    /// Left button.
    Left,
    /// Right button.
    Right,
    /// Middle button.
    Middle,
}

/// Kind of mouse interaction.
#[derive(Copy, Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MouseKind {
    /// Button pressed.
    Down(MouseButton),
    /// Button released.
    Up(MouseButton),
    /// Drag with the named button held.
    Drag(MouseButton),
    /// Pointer moved with no button held.
    Move,
    /// Wheel scrolled up.
    ScrollUp,
    /// Wheel scrolled down.
    ScrollDown,
    /// Wheel scrolled left.
    ScrollLeft,
    /// Wheel scrolled right.
    ScrollRight,
}

/// A mouse event.
#[derive(Copy, Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MouseEvent {
    /// Frontend that produced the event.
    pub frontend_id: FrontendId,
    /// Kind of mouse interaction.
    pub kind: MouseKind,
    /// Cell-grid coordinate of the pointer at the moment of the event.
    pub coord: CellCoord,
    /// Modifiers held during the event.
    pub mods: Modifiers,
}

// ---------------------------------------------------------------------------
// Frontend → Instance events
// ---------------------------------------------------------------------------

/// Input event from frontend to instance.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum FrontendEvent {
    /// A key event.
    Key(KeyEvent),
    /// A mouse event.
    Mouse(MouseEvent),
    /// Frontend's terminal resized.
    Resize {
        /// Frontend that resized.
        frontend_id: FrontendId,
        /// New size, in cells.
        size: CellSize,
    },
    /// Bracketed-paste payload from the frontend.
    Paste {
        /// Frontend that produced the paste.
        frontend_id: FrontendId,
        /// Raw bytes pasted (the instance decodes as UTF-8 if relevant).
        data: Vec<u8>,
    },
    /// Frontend gained input focus.
    FocusGained(FrontendId),
    /// Frontend lost input focus.
    FocusLost(FrontendId),
    /// Frontend is going away. Instance treats this as immediate
    /// detach; no acknowledgement required.
    Detach(FrontendId),
    /// T M10.5: CRDT operation produced by this frontend's local
    /// edit, sent to the instance for broadcast to the other
    /// attached frontends. The actual flow that produces these
    /// (frontend maintaining a local CRDT state, applying edits
    /// optimistically, sending the resulting op) is wired in M10.8
    /// + M10.10; M10.5 declares the wire shape so the protocol
    ///   version bump (1 → 2) covers it.
    ///
    /// Only sent by v1.0 frontends (`protocol_version = 2`); v0.1
    /// frontends never emit this variant. Sessions negotiated at
    /// protocol version 1 must NOT receive this on the
    /// instance-side dispatcher (the daemon filters per-session;
    /// the editor-core treats it as an unknown frontend event if
    /// it ever arrives from a v1 session, which it shouldn't).
    CrdtOp {
        /// Which attached frontend produced this op. The instance
        /// uses this to avoid echoing the op back to its sender.
        frontend_id: FrontendId,
        /// Which buffer this op affects. The instance routes the
        /// op to that buffer's CRDT state.
        buffer_id: crate::BufferId,
        /// The CRDT operation payload — `peer_id` + opaque wire bytes
        /// loro's `import_updates` decodes.
        op: crate::CrdtOp,
    },
    /// T M11.1: the buffer byte range a semantic frontend currently
    /// has on screen, in buffer coordinates. Replaces the
    /// instance-derived grid viewport for `semantic_render` sessions:
    /// the instance scopes its `StyleSpans` / `Decorations` / … to
    /// this range rather than shipping a whole file's styling.
    ///
    /// **No pixels.** This carries a byte range, never viewport pixel
    /// size, DPI, font metrics, or glyph advances — the contract
    /// boundary invariant from the semantic-frontend design note. The
    /// frontend owns all visual-motion semantics and resolves
    /// pixel→offset locally; there is deliberately no hit-test
    /// request variant and no `SemanticResize`, both of which would
    /// leak pixels across the boundary.
    ///
    /// `generation` ties the declared range to a CRDT version so the
    /// instance can ignore a viewport that races a not-yet-applied
    /// edit (symmetric with `StyleSpans::generation`).
    ///
    /// Only emitted by sessions that negotiated `semantic_render`;
    /// a non-semantic session never sends it. M11.1 declares the
    /// wire shape; the instance-side consumer is wired with the
    /// projection seam in M11.2.
    Viewport {
        /// Which frontend's viewport this is.
        frontend_id: FrontendId,
        /// Which buffer the visible range indexes into.
        buffer_id: crate::BufferId,
        /// Half-open byte range currently on screen.
        visible: ByteRange,
        /// CRDT generation the frontend computed `visible` against.
        generation: u64,
    },
    /// Q#M1 (protocol v5): a pointer gesture a *semantic* frontend
    /// hit-tested locally to a **source byte offset**. The pixel→byte
    /// resolution happens entirely frontend-side (the frontend owns
    /// layout: fonts, inline adornments, scroll) — consistent with
    /// `Viewport`'s no-pixels contract; the instance replays its
    /// existing mouse gesture semantics in byte space, so selection
    /// behavior stays single-sourced with the grid path.
    ///
    /// Cell-grid frontends keep using [`FrontendEvent::Mouse`]; the
    /// two variants are per-session-kind, never mixed. Only sent when
    /// the instance's `Hello.protocol_version >= 5` (an older
    /// instance cannot decode the variant).
    Pointer {
        /// Which frontend produced the gesture (untrusted; the
        /// instance routes by the authenticated session, matching
        /// the `CrdtOp` / `Viewport` source-trust rule).
        frontend_id: FrontendId,
        /// Buffer the frontend was displaying when it hit-tested.
        buffer_id: crate::BufferId,
        /// Source byte offset of the hit (frontend-local hit test;
        /// adornment runs already snapped to their anchors).
        byte: u64,
        /// Which gesture step this is.
        kind: PointerKind,
        /// Modifiers held during the gesture. Carried for future
        /// Shift-click extension; the v5 instance ignores them.
        mods: Modifiers,
    },
    /// Navigate an open context menu (Q#CM1, protocol v11). A semantic
    /// frontend hit-tests the popup it drew locally and reports the row
    /// the pointer is over, so the daemon never needs the frontend's
    /// pixels — symmetric with how the GPU owns its viewport. Sent only
    /// while the daemon's menu is open.
    MenuPointer {
        /// Which frontend produced the gesture.
        frontend_id: FrontendId,
        /// Row index the pointer is over, or `None` when it left the
        /// popup (a hover off the menu, or a click outside).
        index: Option<u32>,
        /// `true` on a click/release: invoke `index`'s item, or dismiss
        /// the menu when `index` is `None`. `false` is a hover that only
        /// moves the highlight.
        invoke: bool,
    },
    /// Vterm Stage 3 (protocol v19): the terminal-cell geometry a
    /// semantic frontend has on screen for `buffer_id`.
    ///
    /// This is the terminal twin of [`Self::Viewport`] and keeps the
    /// same no-pixels contract: it carries a CELL size, never pixel
    /// extent, DPI, or glyph advances. The frontend divides its own
    /// drawable rectangle by its own metrics.
    ///
    /// A v19 frontend sends both this and `Viewport` after every
    /// `BufferSnapshot`; the daemon accepts only the declaration
    /// matching the authenticated active buffer's kind and drops the
    /// other. That dual declaration is what removes the otherwise
    /// circular dependency where a frontend would need a terminal frame
    /// before it knew to ask for one.
    ///
    /// Recording the size is not claiming control: a passive view's
    /// declaration produces its own clipped/padded projection, while
    /// only the durable controller changes the shared PTY geometry.
    /// Sent only to a `>= 19` daemon.
    TerminalResize {
        /// Which frontend declared the geometry (untrusted; the daemon
        /// routes by the authenticated session, matching the
        /// `CrdtOp` / `Viewport` / `Pointer` source-trust rule).
        frontend_id: FrontendId,
        /// Terminal identity buffer the frontend was displaying.
        buffer_id: crate::BufferId,
        /// Content rectangle size in terminal cells.
        size: CellSize,
    },
    /// Vterm Stage 3 (protocol v19): a pointer gesture a semantic
    /// frontend hit-tested to a terminal CELL.
    ///
    /// Terminal windows have no source bytes to hit-test against, so
    /// this replaces [`Self::Pointer`] inside the terminal clip. Once
    /// accepted it follows the landed Stage 2 pointer path: child SGR
    /// mouse reporting when eligible, otherwise per-view scroll,
    /// selection, or context menu. Sent only to a `>= 19` daemon.
    TerminalPointer {
        /// Which frontend produced the gesture (untrusted, as above).
        frontend_id: FrontendId,
        /// Terminal identity buffer the frontend was displaying.
        buffer_id: crate::BufferId,
        /// Cell the pointer is over, within the last declared viewport.
        coord: CellCoord,
        /// Which gesture step this is.
        kind: MouseKind,
        /// Modifiers held during the gesture.
        mods: Modifiers,
    },
    /// Bottom panel Stage 2 (protocol v21): the frontend's authoritative
    /// cell-equivalent layout capacity (Q#BP15a).
    ///
    /// Valid **without** a side window — the daemon needs columns before
    /// it can paint a first panel frame, so gating this on panel
    /// presence would deadlock the first open. "Without" refers to
    /// side-window presence only; the protocol and session gates still
    /// apply, and the event is accepted only from an authenticated,
    /// negotiated panel-capable semantic session.
    ///
    /// Sent immediately after attach acceptance and refreshed on window
    /// resize, font change, and scale change. `geometry_epoch` is
    /// frontend-owned because a font or scale change can invalidate an
    /// old panel frame while `total` is **identical**, which daemon-side
    /// value dedup cannot detect.
    FrontendCellGeometry {
        /// Which frontend declared this (untrusted; checked against the
        /// transport source).
        frontend_id: FrontendId,
        /// Monotonic frontend-owned declaration id; `0` is reserved for
        /// "never declared" and is rejected on the wire.
        geometry_epoch: u64,
        /// Whole-cell capacity of the frontend's frame.
        total: CellSize,
    },
    /// Bottom panel Stage 2 (protocol v21): requested fixed panel rows
    /// from a divider drag (Q#BP15a).
    ///
    /// Rows are the only size component; the epochs are identities, not
    /// geometry. Accepted only for the currently visible `Present` panel
    /// matching both the latest geometry declaration and the current
    /// presentation epoch, then clamped by Q#BP2's interactive
    /// preference.
    PanelResizeRows {
        /// Which frontend produced the drag (untrusted, as above).
        frontend_id: FrontendId,
        /// Geometry declaration this request is measured against.
        geometry_epoch: u64,
        /// Presentation identity this request addresses.
        panel_epoch: u64,
        /// Requested fixed panel rows.
        rows: u32,
    },
    /// Bottom panel Stage 2 (protocol v21): a pointer gesture a semantic
    /// frontend hit-tested to a panel CELL (Q#BP16).
    ///
    /// Carries both epochs so a gesture aimed at a panel that has since
    /// been replaced or reopened cannot be applied to its successor.
    /// Unlike [`Self::Pointer`], accepting this **activates the panel**.
    ///
    /// `buffer_id` and `panel_epoch` close different holes and neither
    /// subsumes the other: `buffer_id` catches an A→B buffer
    /// replacement, while `panel_epoch` catches close/hide/reopen of the
    /// **same** persistent buffer — which a buffer id alone cannot
    /// distinguish — without putting a `WindowId` on the wire.
    PanelPointer {
        /// Which frontend produced the gesture (untrusted, as above).
        frontend_id: FrontendId,
        /// Geometry declaration this gesture was hit-tested against.
        geometry_epoch: u64,
        /// Presentation identity this gesture addresses.
        panel_epoch: u64,
        /// Buffer the frontend believed the panel was displaying.
        buffer_id: crate::BufferId,
        /// Cell the pointer is over, within the declared panel grid.
        coord: CellCoord,
        /// Which gesture step this is.
        kind: MouseKind,
        /// Modifiers held during the gesture.
        mods: Modifiers,
    },
}

/// Gesture step for [`FrontendEvent::Pointer`]. Double-click
/// detection is frontend-side (`DoubleDown` instead of a second
/// `Down`): only the frontend knows pixel proximity and its own
/// double-click interval.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    /// Primary button pressed at `byte`.
    Down,
    /// Pointer moved to `byte` with the primary button held.
    Drag,
    /// Primary button released at `byte`.
    Up,
    /// Second press at the same hit within the frontend's
    /// double-click window — selects the word at `byte`.
    DoubleDown,
    /// Third press at the same hit within the frontend's
    /// multi-click window — selects the whole line at `byte`,
    /// trailing newline included (Q#M4, protocol v7). The frontend
    /// sends this only to a `>= 7` instance; against an older one
    /// the third click restarts the chain as a plain `Down`.
    TripleDown,
    /// Secondary (right) button pressed at `byte` — opens the context
    /// menu there (Q#CM1, protocol v11). The frontend sends this only to
    /// a `>= 11` instance.
    Context,
}

impl FrontendEvent {
    /// The frontend that produced this event.
    #[must_use]
    pub fn frontend_id(&self) -> FrontendId {
        match self {
            Self::Key(e) => e.frontend_id,
            Self::Mouse(e) => e.frontend_id,
            Self::Resize { frontend_id, .. }
            | Self::Paste { frontend_id, .. }
            | Self::FocusGained(frontend_id)
            | Self::FocusLost(frontend_id)
            | Self::Detach(frontend_id)
            | Self::CrdtOp { frontend_id, .. }
            | Self::Viewport { frontend_id, .. }
            | Self::Pointer { frontend_id, .. }
            | Self::MenuPointer { frontend_id, .. }
            | Self::TerminalResize { frontend_id, .. }
            | Self::TerminalPointer { frontend_id, .. }
            | Self::FrontendCellGeometry { frontend_id, .. }
            | Self::PanelResizeRows { frontend_id, .. }
            | Self::PanelPointer { frontend_id, .. } => *frontend_id,
        }
    }
}

// ---------------------------------------------------------------------------
// Instance → Frontend messages
// ---------------------------------------------------------------------------

/// Cursor position and visibility.
#[derive(Copy, Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CursorState {
    /// Cell where the cursor should be drawn.
    pub coord: CellCoord,
    /// Whether the cursor is visible at all.
    pub visible: bool,
}

/// Instance-level signal that is not a render message.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum InstanceSignal {
    /// Terminal bell.
    Bell,
    /// Window-title change request.
    Title(String),
    /// Clipboard set request (OSC 52).
    Clipboard(Vec<u8>),
}

/// Reason an instance terminates an attachment.
///
/// Only the four variants the v0.1 daemon actually emits or rejects on.
/// `Evicted` (multi-frontend takeover) and similar will land alongside
/// the v0.3 multi-frontend work; until then `AlreadyAttached` covers
/// the single-slot equivalent.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum GoodbyeReason {
    /// Instance is shutting down (SIGTERM / SIGINT or clean exit).
    ShuttingDown,
    /// Frontend's `protocol_version` does not match the instance's.
    /// The handshake fails before any further messages.
    VersionMismatch {
        /// The instance's [`PROTOCOL_VERSION`] — the highest wire it can
        /// speak, **not** the [`ADVERTISED_PROTOCOL_VERSION`] baseline it put
        /// in [`Hello`]. Since those diverged (the baseline is a permanent
        /// compatibility floor), reporting the baseline here would understate
        /// the daemon's ceiling and invert the upgrade advice.
        ///
        /// A *frontend* raising this locally can only report the baseline it
        /// was handed, because that is all the daemon told it.
        server: u32,
        /// The version the frontend announced in its `AttachRequest`.
        client: u32,
    },
    /// Another frontend is currently attached. v0.1 rejects concurrent
    /// attaches; v0.3 will replace this with eviction or multiplexing.
    AlreadyAttached,
    /// Frontend sent a malformed message or otherwise violated the
    /// protocol. The connection is closed without further dialogue.
    ProtocolError,
    /// T M10.7: frontend declared one or more negotiated capability
    /// bits that the instance cannot honor. The handshake fails after
    /// the version check but before any further messages.
    ///
    /// `missing` lists the capability *field names* (e.g.,
    /// `"multi_frontend"`, `"crdt_replica"`) the frontend requested
    /// (`true`) that the instance reports as `false`. These strings
    /// are stable wire-format identifiers: they are exactly the
    /// `FrontendCapabilities` / `InstanceCapabilities` field names,
    /// not human-readable descriptions. The frontend translates them
    /// for display via [`AttachError`]'s formatting. Renaming a
    /// capability bit requires changing both the field name AND the
    /// missing-string emission in `negotiate_capabilities` in
    /// lockstep — see the M10.7 audit's wire-format-stability
    /// section.
    CapabilityMismatch {
        /// The capability bit names the frontend asked for that the
        /// instance does not support. Each entry is a verbatim
        /// `FrontendCapabilities` field name.
        missing: Vec<String>,
    },
}

/// Rendering and signals from instance to frontend.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum InstanceMessage {
    /// Cell deltas. `full_grid = true` is the initial sync sent on
    /// fresh attach (or after a resize where the previous grid is no
    /// longer applicable); `full_grid = false` is a differential
    /// frame.
    CellDelta {
        /// One run of changed cells per `DiffSpan`.
        spans: Vec<DiffSpan>,
        /// Whether `spans` represents a full-grid resync (true on
        /// fresh attach or post-resize) versus an incremental frame.
        full_grid: bool,
    },
    /// Cursor position and visibility update.
    Cursor(Option<CursorState>),
    /// Modeline cells. Reserved for v0.3 GUI use; v0.1 ships modeline
    /// inside [`InstanceMessage::CellDelta`]. The variant exists in
    /// the protocol from day one so adding the discrete channel later
    /// is not a breaking change.
    ModeLine(Vec<Cell>),
    /// Side-channel signal (bell, title, clipboard).
    Signal(InstanceSignal),
    /// Instance is terminating the attachment.
    Goodbye(GoodbyeReason),
    /// T M10.5: CRDT operation broadcast from the instance to all
    /// attached frontends. The originating frontend produced this op
    /// (via `FrontendEvent::CrdtOp` or via a local editor-core edit
    /// that synthesizes one); the instance fans it out so every
    /// attached frontend can apply the op to its local CRDT state.
    ///
    /// Only sent to v1.0 frontends — sessions negotiated at
    /// `protocol_version = 1` never receive this variant, per
    /// `§sec:m10-backward-compat`. The daemon filters at the
    /// outgoing-message path; this variant simply existing in the
    /// enum is not a wire-compat issue for v1 sessions because the
    /// daemon never emits it to them.
    ///
    /// M10.5 declares the wire shape. M10.8 wires the editor-core →
    /// daemon → frontend flow that actually emits these.
    CrdtOp {
        /// Which buffer this op affects. v1.0 frontends maintain
        /// a per-buffer local CRDT state; this routes to the right
        /// one.
        buffer_id: crate::BufferId,
        /// The CRDT operation payload — `peer_id` + opaque wire bytes
        /// loro's `import_updates` decodes.
        op: crate::CrdtOp,
    },
    /// T M10.6: cursor + selection state of one attached frontend,
    /// broadcast to the other v1.0 frontends so they can render
    /// peer-presence overlays. Coalesced at the daemon: rapid cursor
    /// movement produces one `PresenceUpdate` per tick per source
    /// frontend, carrying the *final* state, not intermediate values.
    ///
    /// Sender exclusion: the source frontend never receives its own
    /// `PresenceUpdate`. v0.1 sessions (negotiated `protocol_version =
    /// 1`) are filtered out at the daemon's outgoing-message path.
    ///
    /// M10.6 declares the wire shape AND wires the daemon-side
    /// sweep with per-session filter. In single-frontend deployments
    /// the recipient list is structurally empty (sender exclusion
    /// with no other v2 sessions); M10.8 enables the multi-frontend
    /// case where this message actually crosses the wire. The
    /// frontend's renderer for peer-cursor overlays is also M10.8.
    PresenceUpdate {
        /// Which attached frontend this presence belongs to. v1.0
        /// frontends use this to label the peer-cursor overlay
        /// ("user 4 is editing here").
        frontend_id: FrontendId,
        /// Which buffer the source frontend's cursor is in.
        buffer_id: crate::BufferId,
        /// Byte offset of the source frontend's cursor within
        /// `buffer_id`. Frontends convert to line/column at render
        /// time via the rope's coord-mapping; the wire carries the
        /// canonical byte offset to avoid encoding-vs-rendering
        /// drift across frontends.
        cursor: crate::Position,
        /// Active selection range, if any.
        selection: Option<SelectionSnapshot>,
    },
    /// T M10.10: bootstrap a frontend's local CRDT replica with the
    /// instance's current authoritative state. Sent once per active
    /// buffer at `SessionEstablished` time (and on subsequent
    /// buffer-creation events) to frontends that negotiated
    /// `crdt_replica: true`. Frontends that didn't negotiate the
    /// capability never receive this variant — the daemon's
    /// outgoing-message filter gates the send on
    /// `NegotiatedCapabilities::crdt_replica`.
    ///
    /// `crdt_snapshot` carries loro's run-encoded snapshot
    /// (`CrdtState::export_snapshot()`) — the CRDT-internal state
    /// including peer IDs, version vectors, and op-history structure.
    /// Raw byte contents are insufficient because a fresh CRDT replica
    /// initialized from bytes alone diverges on the first concurrent
    /// edit.
    ///
    /// Cursor position is intentionally absent: cursor is per-frontend
    /// window state (M10.8 `FrontendView`), not per-buffer CRDT
    /// state. The same buffer can appear in multiple windows on one
    /// frontend with different cursors; coupling cursor to
    /// `BufferSnapshot` would break this model.
    BufferSnapshot {
        /// Which buffer's CRDT state this snapshot represents.
        buffer_id: crate::BufferId,
        /// `loro::LoroDoc::export(ExportMode::Snapshot)` output. Applied
        /// to a fresh `CrdtState::new(peer_id_from_frontend(my_id))`
        /// via `import_snapshot(bytes)` on the receiving frontend.
        crdt_snapshot: Vec<u8>,
    },
    /// T M10.10: the active buffer for a replica frontend, with the
    /// cursor position within it.
    ///
    /// # Semantics (Day 3 step 3b composition-check broadened
    /// contract)
    ///
    /// `CursorByte` represents "the active buffer for this frontend
    /// is `buffer_id`; the cursor in that buffer is at `byte_pos`."
    /// Not just "the cursor moved." This contract matters: a narrow
    /// "cursor moved" emission would miss active-buffer-changed-
    /// without-cursor-motion events (Lua-driven buffer switch
    /// landing at the same byte position), and the frontend's
    /// active-buffer tracking would go stale.
    ///
    /// Daemon emits `CursorByte` on every per-tick render frame for
    /// replica frontends, derived fresh from `active_window_for(fid)`.
    /// Cursor move, active-buffer change, and active-window change
    /// all produce a new emission carrying the current `(buffer_id,
    /// byte_pos)`. The per-tick rate (16ms at 60Hz) is the same as
    /// `Cursor`'s grid-coord variant.
    ///
    /// # Why a separate variant from `Cursor`
    ///
    /// `Cursor` carries grid coordinates (row/col cells) — the
    /// frontend uses them to paint the cursor. The optimistic-apply
    /// path needs byte position (CRDT insert/delete is byte-indexed),
    /// which the grid coordinate can't recover without duplicating
    /// the daemon's view-layout logic (tab expansion, line wrap,
    /// double-width chars, viewport offset). `CursorByte` is the
    /// authoritative byte position for the active buffer.
    ///
    /// # Atomicity with `Cursor`
    ///
    /// The daemon emits `Cursor` and `CursorByte` together for
    /// replica frontends — both derived from the same render-frame
    /// iteration so they describe the cursor in the same instant in
    /// two reference frames. Non-replica frontends receive only
    /// `Cursor` (existing behavior). The replica frontend that sees
    /// `Cursor` without a paired `CursorByte` would interpret stale
    /// byte position; the daemon guarantees both emit together by
    /// derivation, not by message-protocol atomicity.
    ///
    /// # Wire-format compatibility
    ///
    /// New variant in v2; receivers without M10.10 hard-error on
    /// decode (postcard does not gracefully degrade unknown variants,
    /// per M10.10-FRAMING.md Refinement 3). Capability-gated: daemon
    /// sends only to frontends that negotiated `crdt_replica: true`.
    /// `PROTOCOL_VERSION` stays at 2.
    CursorByte {
        /// The buffer the cursor is in. A replica frontend tracks
        /// per-buffer cursors; this routes the update to the right
        /// entry.
        buffer_id: crate::BufferId,
        /// Byte offset of the cursor within `buffer_id`. Source of
        /// truth for the optimistic-apply path's insert / delete
        /// position arguments. Wire type matches
        /// `PresenceUpdate::cursor` (`u64`) for consistency; frontend
        /// converts to `usize` for the loro API.
        byte_pos: crate::Position,
    },
    /// T M11.1 — syntax + face styling over the semantic frontend's
    /// current viewport range. `generation` ties the spans to a CRDT
    /// version so the frontend can discard styling that predates an
    /// edit it has already applied optimistically. Ships **no text** —
    /// the frontend holds the rope via the `crdt_replica` machinery
    /// and interprets these spans over it.
    ///
    /// # Diff shape (T M11.4)
    ///
    /// Mirrors `CellDelta`'s `full_grid` + changed-runs structure,
    /// lifted from positional cells to byte-anchored ranges. `full =
    /// true` is a resync: the frontend discards all prior styling for
    /// `buffer_id` and the `segments` are authoritative for the whole
    /// declared viewport (first frame after a `Viewport`, a viewport
    /// jump, or a generation discontinuity). `full = false` is
    /// incremental: each [`StyleSegment`] replaces styling **only**
    /// within its `range`; bytes covered by no segment keep their
    /// previously-applied style. A frame whose styling is unchanged
    /// ships no `StyleSpans` at all.
    ///
    /// Because byte offsets cascade on edits (an insert shifts every
    /// later span), an incremental frame after an edit dirties
    /// `[edit, viewport_end)` — still bounded, and no-edit frames
    /// (cursor move, scroll within the declared viewport, selection)
    /// cost nothing. Each segment carries *all* current spans
    /// intersecting its range (clipped), not only changed ones, so an
    /// unchanged span overlapping a dirty range is faithfully
    /// reconstructed.
    ///
    /// Gated on negotiated `semantic_render`; never sent to a grid
    /// session (the daemon's per-session outgoing filter — wired with
    /// the producer in M11.2 — never emits it there, so postcard's
    /// hard-error on unknown variants is mooted exactly as it is for
    /// `CursorByte`).
    StyleSpans {
        /// Buffer these spans interpret.
        buffer_id: crate::BufferId,
        /// CRDT generation the spans were computed against.
        generation: u64,
        /// `true` → discard all prior styling for `buffer_id` first;
        /// `segments` are authoritative for the declared viewport.
        full: bool,
        /// Dirty byte regions and the styling now covering them.
        segments: Vec<StyleSegment>,
    },
    /// T M11.1 — diagnostics, search hits, current-line, and any
    /// other "this region means something" overlay, as offset ranges
    /// plus a kind. Peer selection is **not** here — it stays on the
    /// existing `PresenceUpdate` path. Gated on `semantic_render`.
    ///
    /// T M11.4 — same `full` + segment diff shape as `StyleSpans`
    /// (see its docs), and gains `generation` for parity: a frontend
    /// wants the CRDT version decorations were computed against for
    /// the same optimistic-edit race reason styling does.
    Decorations {
        /// Buffer these decorations apply to.
        buffer_id: crate::BufferId,
        /// CRDT generation the decorations were computed against.
        generation: u64,
        /// `true` → discard all prior decorations for `buffer_id`
        /// first; `segments` are authoritative for the viewport.
        full: bool,
        /// Dirty byte regions and the decorations now covering them.
        segments: Vec<DecorationSegment>,
    },
    /// T M11.1 — inlay hints, blame, lens, virtual text. Anchored at
    /// a single offset with a placement; occupies no document bytes —
    /// the frontend interleaves it at layout time. Gated on
    /// `semantic_render`.
    InlineAdornments {
        /// Buffer these adornments annotate.
        buffer_id: crate::BufferId,
        /// The adornment items for the declared viewport.
        items: Vec<InlineAdornment>,
    },
    /// Coarse whole-file styling summary for a minimap / scrollbar
    /// overview, resolving design-note Open Q#2. One [`Style`] per
    /// source line (the *dominant* style for that line by byte count
    /// across the producer's current spans); the frontend maps minimap
    /// rows to one or more of these. Unlike [`Self::StyleSpans`], this
    /// is **not** viewport-scoped — the minimap shows the whole file.
    ///
    /// Recomputed when the buffer's CRDT `generation` advances; an
    /// unchanged buffer ships no further summary. A coarser
    /// representation (fixed bands) or finer (run-length style runs)
    /// is recorded in `docs/semantic-frontend-protocol.md` as future
    /// refinements; per-line dominant style is the v1 choice because
    /// minimap rows naturally correspond to code lines.
    ///
    /// Gated on negotiated `semantic_render`; the daemon emits it only
    /// for sessions that have a [`crate::semantic_render::SemanticRenderState`]
    /// (structural gating, same as every other semantic family).
    FileStyleSummary {
        /// Buffer this summary describes.
        buffer_id: crate::BufferId,
        /// CRDT generation the summary was computed against. The
        /// frontend can discard a summary that predates an edit it
        /// has already applied optimistically.
        generation: u64,
        /// One [`crate::cell::Style`] per source line, in line order
        /// from line 0. Empty when the buffer is empty.
        lines: Vec<crate::cell::Style>,
    },
    /// Q#S1 (status band, protocol v8) — instance-authoritative
    /// status facts a semantic frontend cannot derive locally:
    /// buffer name, modified flag, whole-file diagnostic counts.
    /// Cursor position and scroll stay frontend-derived (the
    /// optimistic caret must not lag a round trip). Emitted by the
    /// semantic producer when any fact changes; kept off wires
    /// negotiated `< 8` (additive variant — an older peer would
    /// hard-error decoding it).
    StatusFacts {
        /// Buffer these facts describe.
        buffer_id: crate::BufferId,
        /// Buffer display name.
        name: String,
        /// Unsaved-changes flag.
        modified: bool,
        /// Whole-file `Error`-severity diagnostic count.
        diag_errors: u32,
        /// Whole-file `Warning`-severity diagnostic count.
        diag_warnings: u32,
        /// The core's transient status message (`pmacs.editor.
        /// set_status` — LSP command summaries like "12 references",
        /// error reports, ...), or `None` when clear. Added in v15:
        /// the attached TUI gets the message for free through the
        /// rendered cell grid's bottom row, but a semantic frontend
        /// only sees what's on this wire — without it every modeline
        /// summary was TUI-only. Encoding change to this variant; its
        /// daemon gate moved `>= 8` → `>= 15` (the v10 `SearchPrompt`
        /// / v14 `LineNumbers` precedent).
        message: Option<String>,
    },
    /// T M11.1 — diff zones, folded-region placeholders, anything
    /// occupying its own vertical band. Anchored to the offset of the
    /// line it precedes or replaces; the frontend allocates the
    /// vertical space. Gated on `semantic_render`.
    BlockAdornments {
        /// Buffer these adornments annotate.
        buffer_id: crate::BufferId,
        /// The block items for the declared viewport.
        items: Vec<BlockAdornment>,
    },
    /// T M11.1 — the instance's authoritative fold set as document
    /// facts. Folding is an instance command-semantics concern (Lua
    /// can fold); the visual collapse is a frontend layout concern —
    /// the frontend renders the placeholder and adjusts its own
    /// layout. Gated on `semantic_render`.
    FoldState {
        /// Buffer whose fold set this is.
        buffer_id: crate::BufferId,
        /// Folded byte ranges.
        folds: Vec<ByteRange>,
    },
    /// T M11.1 — out-of-band content an adornment refers to (images,
    /// blame avatars). Sent once, referenced by `handle`, so it is
    /// not re-shipped per frame. Gated on `semantic_render`.
    ResourceOffer {
        /// Stable handle adornments reference via
        /// [`AdornmentContent::Resource`].
        handle: u64,
        /// MIME type of `body`.
        mime: String,
        /// Inline bytes or a URI the frontend resolves itself.
        body: ResourceBody,
    },
    /// Daemon-side input-dispatcher idleness signal. Tells a
    /// `crdt_replica` frontend whether the next key event would be
    /// **intercepted** by the daemon (minibuffer prompt active or
    /// dispatcher holds a pending multi-key prefix) versus would
    /// self-insert into the active buffer.
    ///
    /// Frontends running the optimistic-apply layer (`src/optimistic.rs`)
    /// gate the local apply on `idle == true`: when the daemon is not
    /// idle, plain-char keystrokes must round-trip as
    /// [`FrontendEvent::Key`] so the daemon's minibuffer / prefix
    /// dispatcher receives them. Without this signal, characters typed
    /// into a minibuffer prompt would be applied as `CrdtOp`s to the
    /// previously-active document instead — the surfacing of which
    /// motivated this wire addition.
    ///
    /// Emission contract (daemon side):
    ///
    /// - Once after `AttachRequest` is accepted, before any other
    ///   non-handshake message, so a fresh frontend starts from a
    ///   known idle state regardless of the daemon's current input
    ///   condition.
    /// - At every transition: minibuffer begin / dismiss; dispatcher
    ///   `pending` empty↔non-empty.
    /// - Coalesced where consecutive transitions land on the same
    ///   value (no spurious back-to-back identical signals).
    ///
    /// Gated on negotiated `crdt_replica` — frontends without the
    /// optimistic-apply layer have no use for this signal and the
    /// daemon's per-session filter drops it for them.
    DispatchIdle {
        /// `true` → the next key event would self-insert into the
        /// active buffer (no minibuffer, no pending prefix); optimistic
        /// apply is correct.
        /// `false` → the daemon would intercept the next key; the
        /// frontend must round-trip via [`FrontendEvent::Key`].
        idle: bool,
    },
    /// Q#SR5 (incremental search, protocol v9) — the live isearch
    /// prompt for a semantic frontend that cannot host a minibuffer.
    /// Carries the query as typed and the match readout so the
    /// frontend can render an `I-search: <query> (n/m)` band; the
    /// matches themselves arrive as [`DecorationKind::SearchMatch`] /
    /// [`DecorationKind::SearchMatchActive`] decorations. A `query` of
    /// `None` means no search is running — the frontend hides the band.
    ///
    /// Emitted by the semantic producer when the search state changes
    /// (cached-compare suppressed, like [`Self::StatusFacts`]). The
    /// `regex` / `invalid` fields (Q#RX6) changed this variant's
    /// encoding, so the daemon's per-session filter now keeps it off
    /// wires negotiated `< 10` (was `< 9` for the original four-field
    /// shape — see `PROTOCOL_VERSION`).
    SearchPrompt {
        /// Buffer the search is anchored in (the active buffer).
        buffer_id: crate::BufferId,
        /// The query as typed so far, or `None` when no search runs.
        /// `Some("")` is a freshly-started search with an empty query.
        query: Option<String>,
        /// 0-based index of the active match, or `None` when the query
        /// has no matches (a failing search).
        active: Option<u32>,
        /// Total number of matches for the current query.
        total: u32,
        /// `true` when the search is in regex mode (Q#RX3) — the
        /// frontend prefixes the prompt with `Regex `.
        regex: bool,
        /// `true` when a regex pattern failed to compile — the frontend
        /// shows `[invalid]` instead of a match count. Always `false`
        /// in literal mode.
        invalid: bool,
    },
    /// Open-menu contents for a semantic frontend (Q#CM1, protocol v11).
    /// The daemon resolves the visible items (predicates / context tags)
    /// and ships the rendered rows + highlight; the frontend draws the
    /// popup at the pixel it remembered from the right-click and reports
    /// navigation via [`FrontendEvent::MenuPointer`]. An empty `rows`
    /// closes the menu. Cached-compare suppressed like `SearchPrompt`.
    MenuPrompt {
        /// Buffer the menu is anchored in (the active buffer).
        buffer_id: crate::BufferId,
        /// Rows top-to-bottom (items + separators); empty = closed.
        rows: Vec<MenuPromptRow>,
        /// Index into `rows` of the highlighted item, or `None` when
        /// closed.
        active: Option<u32>,
    },
    /// Minibuffer prompt state for a semantic frontend (Q#MB1, protocol
    /// v12). The minibuffer is a single *global* core instance, so this
    /// is bufferless; the producer still emits it from the active-buffer
    /// viewport. `prompt: None` clears the GUI. Cached-compare
    /// suppressed like `SearchPrompt`; daemon-gated `>= 12`.
    MinibufferPrompt {
        /// The prompt string (e.g. `"M-x "`), or `None` when no
        /// minibuffer is open.
        prompt: Option<String>,
        /// The text typed so far.
        input: String,
        /// Codepoints before the cursor within `input` (the caret
        /// position).
        cursor: u32,
        /// A windowed slice of the completion candidates (best-first,
        /// already filtered/sorted by the core), `<= MB_VISIBLE`.
        candidates: Vec<String>,
        /// Highlighted row *within* `candidates`, or `None`.
        selected: Option<u32>,
        /// Total candidate count (the window is a slice of this).
        total: u32,
    },
    /// UX gutter — the per-window line-number gutter *mode* for the
    /// frontend's active window. A semantic frontend renders line numbers
    /// *locally* (it owns the text + its own cursor line, so it can draw
    /// relative/hybrid without a round trip), but the mode toggle lives
    /// daemon-side, so the daemon ships which mode. Bumped 13 → 14: the
    /// v13 shape carried a bare `enabled: bool` (off/absolute only); v14
    /// carries the full [`LineNumberMode`] so relative/hybrid can ride the
    /// same message. Daemon-gated `>= 14` — a v13 peer negotiates v13 and
    /// receives no `LineNumbers` (its gutter stays off) rather than
    /// mis-decoding the wider shape.
    LineNumbers {
        /// Buffer the active window shows (routing/consistency; the mode
        /// is a window property, not a buffer one).
        buffer_id: crate::BufferId,
        /// The line-number gutter mode for that window.
        mode: LineNumberMode,
    },
    /// In-buffer completion popup state for a semantic frontend
    /// (Arc 1a Q#C5, protocol v15). Unlike the band-anchored
    /// [`Self::MinibufferPrompt`], the popup is anchored *at a byte*
    /// (the typed prefix's start) — the frontend maps byte → glyph
    /// rect locally, exactly as it does for the caret, so the
    /// instance never learns a pixel. Rows are display-only: accept
    /// is a daemon round-trip (`dispatch_completion_key`), so insert
    /// text never ships. `anchor: None` clears the popup.
    /// Cached-compare suppressed like `SearchPrompt`; daemon-gated
    /// `>= 15`.
    CompletionPopup {
        /// Buffer the popup targets.
        buffer_id: crate::BufferId,
        /// Byte offset of the prefix start, or `None` when closed.
        anchor: Option<u64>,
        /// Bytes of typed prefix at `anchor` (a frontend may embolden
        /// the matched prefix within each label).
        prefix_len: u32,
        /// A windowed slice of the candidates (best-first, already
        /// scored/filtered by the core), `<= POPUP_VISIBLE`.
        rows: Vec<CompletionPopupRow>,
        /// Highlighted row *within* `rows`, or `None`.
        selected: Option<u32>,
        /// Total candidate count (the window is a slice of this).
        total: u32,
    },
    /// Themes arc (Q#TH7, protocol v16). The daemon-resolved UI faces.
    /// The theme is one global instance, so this is bufferless (the
    /// [`Self::MinibufferPrompt`] shape). Complete replacement each
    /// send: a face absent from `faces` is unset, and the frontend
    /// uses its own default for that surface. Every attachment
    /// receives exactly one authoritative table — the empty table
    /// included — with its first emission after viewport declaration;
    /// cached-compare suppressed thereafter. Daemon-gated `>= 16`.
    ///
    /// Appended as the final v16 variant deliberately: postcard
    /// discriminants are ordinal, so inserting earlier would shift
    /// every later variant's tag and corrupt v15 peers on ungated
    /// channels. The `CompletionPopup` byte pin in `src/protocol.rs`
    /// guards this placement; the `ThemeFacts` byte pin there guards
    /// the v17 `FontFacts` placement after it in turn.
    ThemeFacts {
        /// Every stage-1 face that resolves to a style (the Q#TH4
        /// dotted-prefix walk, resolved daemon-side — frontends do
        /// exact-name lookup, no walk), full names, sorted by name
        /// for deterministic comparison.
        faces: Vec<ThemeFace>,
    },
    /// Themes arc stage 2 (Q#F4, protocol v17). The daemon-relayed
    /// GPU font preference. One global instance ⇒ bufferless (the
    /// [`Self::MinibufferPrompt`] shape). Complete replacement each
    /// send; `None` means the frontend's built-in default for that
    /// axis. The daemon relays a PREFERENCE — it never learns
    /// metrics, advances, or what resolves; the frontend owns
    /// resolution and every pixel consequence (the no-pixels
    /// invariant). Every attachment receives exactly one
    /// authoritative preference — the all-default `(None, None)`
    /// included — with its first emission after viewport
    /// declaration; cached-compare suppressed thereafter.
    /// Daemon-gated `>= 17`.
    ///
    /// Appended as the FINAL variant deliberately: postcard
    /// discriminants are ordinal, so inserting earlier would shift
    /// every later variant's tag and corrupt v16 peers on ungated
    /// channels. The `ThemeFacts` byte pin in `src/protocol.rs`
    /// guards this placement.
    FontFacts {
        /// Font family name to resolve frontend-locally, or `None`
        /// for the frontend's default family query.
        family: Option<String>,
        /// Font size in HUNDREDTHS of a logical pixel (1600 =
        /// today's 16.0) — an integer because this enum derives
        /// `Eq`, which `f32` cannot satisfy, and because cosmic-text
        /// metrics are logical pixels, not typographic points.
        /// Valid range 600..=7200; frontends validate and fail
        /// closed (deserialized protocol input is untrusted).
        size_centi_px: Option<u32>,
    },
    /// Statusline segments (Q#SL7, protocol v18). Custom provider output
    /// for the semantic frontend's current buffer. This is a complete
    /// replacement: empty vectors authoritatively mean no custom segments.
    /// Daemon-gated `>= 18`.
    ///
    /// Appended after [`Self::FontFacts`], the final v17 variant, so no
    /// existing postcard discriminant moves.
    StatuslineSegments {
        /// Buffer whose modeline the segments describe.
        buffer_id: crate::BufferId,
        /// Left-side custom segments in display order.
        left: Vec<StatuslineSegment>,
        /// Right-side custom segments in display order.
        right: Vec<StatuslineSegment>,
    },
    /// Vterm Stage 3 (protocol v19). The complete visible terminal grid
    /// for the receiving frontend's active terminal window.
    ///
    /// A terminal identity buffer's `BufferSnapshot` is an empty CRDT
    /// anchor, so a semantic frontend has no text to lay out; this
    /// carries the cells instead. It is a whole-grid replacement, never
    /// a delta, and it is validated by
    /// [`crate::terminal::TerminalFrame::validate`] both before the
    /// daemon emits it and after the frontend decodes it.
    ///
    /// Suppression compares the COMPLETE ordered payload, not
    /// `screen_generation`: selection, scroll, viewport, and process
    /// state all change without advancing that counter, and a
    /// generation-keyed producer would go silent on exactly those
    /// view-only updates. Daemon-gated `>= 19`.
    ///
    /// Appended after [`Self::StatuslineSegments`], the final v18
    /// variant, so no existing postcard discriminant moves.
    TerminalFrame(crate::terminal::TerminalFrame),
    /// GPU initial-target bootstrap result (protocol v20). Sent only to a
    /// semantic session that supplied [`SessionBootstrapRequest::initial_target`].
    ///
    /// Appended after [`Self::TerminalFrame`], the final v19 variant, so no
    /// legacy postcard discriminant moves.
    InitialTargetResult(InitialTargetResult),
    /// Bottom panel Stage 2 (protocol v21): the daemon's painted
    /// projection of one side window, or its authoritative absence
    /// (Q#BP15).
    ///
    /// `Absent` is sent on close **and** on hide: the receiver retains
    /// its last valid frame, so silence would leave a stale band on
    /// screen indefinitely. `Absent` is duplicate-suppressed like any
    /// payload, and applying it clears the last declared panel size and
    /// presentation epoch before any later event can validate against
    /// them.
    ///
    /// Appended after [`Self::InitialTargetResult`], the final v20
    /// variant, so no existing postcard discriminant moves.
    PanelFrame(crate::panel::PanelFramePayload),
}

/// One resolved UI face for [`InstanceMessage::ThemeFacts`]: a full
/// face name (e.g. `"ui.modeline"`) and the style the daemon resolved
/// for it. The face's component *mask* (which components a frontend
/// may read) is a stage-1 contract documented per face in the themes
/// framing; out-of-mask components are never read by either frontend.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ThemeFace {
    /// Full face name (`ui` or `ui.`-prefixed).
    pub name: String,
    /// The daemon-resolved style for this face.
    pub style: crate::cell::Style,
}

/// One daemon-produced custom modeline segment.
///
/// `text` has already been sanitized to one line. `face` is
/// `ui.modeline` or one of its child names; a missing exact entry in
/// [`InstanceMessage::ThemeFacts`] means the base modeline text color.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StatuslineSegment {
    /// Non-empty, single-line segment text.
    pub text: String,
    /// Static modeline face name selected at provider registration.
    pub face: String,
}

/// Line-number gutter mode for a window (UX gutter arc). Shared across the
/// wire, the daemon, and both frontends so the *number rule* — what value
/// each line shows — is identical everywhere (Q#UX7). `pmacs` re-exports
/// this as `crate::window::LineNumberMode`.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineNumberMode {
    /// No gutter; zero layout change (the default, Emacs tradition).
    #[default]
    Off,
    /// Absolute 1-based line numbers.
    Absolute,
    /// Distance from the cursor line (the cursor line shows `0`).
    Relative,
    /// Like `Relative`, but the cursor line shows its absolute 1-based
    /// number instead of `0` (Vim `number` + `relativenumber`).
    Hybrid,
}

impl LineNumberMode {
    /// Whether this mode reserves a gutter at all (everything but `Off`).
    #[must_use]
    pub fn is_on(self) -> bool {
        !matches!(self, Self::Off)
    }

    /// The displayed number for a 0-based buffer `line` given the cursor's
    /// 0-based buffer line, or `None` in `Off`. `Relative`/`Hybrid` depend
    /// on `cursor_line`; `Absolute` ignores it.
    #[must_use]
    pub fn number_for(self, line: usize, cursor_line: usize) -> Option<usize> {
        match self {
            Self::Off => None,
            Self::Absolute => Some(line + 1),
            Self::Relative => Some(line.abs_diff(cursor_line)),
            Self::Hybrid => Some(if line == cursor_line {
                line + 1
            } else {
                line.abs_diff(cursor_line)
            }),
        }
    }
}

/// One row of an open menu on the wire ([`InstanceMessage::MenuPrompt`]).
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MenuPromptRow {
    /// Display label (empty and ignored when `separator`).
    pub label: String,
    /// `true` for a non-selectable group divider.
    pub separator: bool,
}

/// One row of the in-buffer completion popup on the wire
/// ([`InstanceMessage::CompletionPopup`]). Display fields only ---
/// accept resolves daemon-side against the core session, so the
/// insert text stays off the wire.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CompletionPopupRow {
    /// Display label.
    pub label: String,
    /// LSP `CompletionItemKind` numeric code (1..=25; frontends map
    /// unknown codes to a plain-text glyph, per the LSP contract).
    pub kind: u8,
    /// Optional one-line detail rendered after the label.
    pub detail: Option<String>,
}

/// Flat selection state for the wire.
///
/// Mirrors [`crate::window::Selection`] but as a self-contained pair
/// of byte offsets — `anchor` is where the selection began,
/// `active` is the current selection cursor. Either may be the
/// numerically larger value; callers wanting `(lo, hi)` order
/// compute it locally.
///
/// Kept flat (no nested types) so [`PartialEq`] equality is exactly
/// wire-representation equality: two `SelectionSnapshot`s compare
/// equal iff they serialize to identical bytes. The presence-diff
/// sweep relies on this — see [`crate::presence::SessionRegistry`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SelectionSnapshot {
    /// Where the selection began.
    pub anchor: crate::Position,
    /// The active end (typically the cursor at the moment of the
    /// snapshot).
    pub active: crate::Position,
}

// ---------------------------------------------------------------------------
// T M11.1 — Semantic-frontend projection types
//
// The payloads of the `InstanceMessage::StyleSpans` … `ResourceOffer`
// family and `FrontendEvent::Viewport`. Everything is anchored in
// **byte offsets** (consistent with `CursorByte`): line/col is a
// frontend rendering concern, CRDT position is replica-internal. The
// instance never learns a pixel — see the contract boundary in
// `docs/semantic-frontend-protocol.md`.
//
// The variant/kind sets here are provisional and co-evolve within the
// M11 arc behind the `semantic_render` capability + protocol v3,
// exactly as the CRDT op shape evolved M10.5→M10.10 behind
// `crdt_replica`. They are not a wire-compat hazard for non-semantic
// sessions: the daemon's per-session outgoing filter (wired with the
// producer in M11.2) never emits the family to a session that didn't
// negotiate `semantic_render`, so postcard's hard-error on unknown
// variants is mooted exactly as it is for `CursorByte`.
// ---------------------------------------------------------------------------

// `ByteRange` moved to `pmacs-protocol` (see the re-export near the
// top of this file).

/// One run of buffer bytes carrying a resolved visual style. The
/// instance is the single syntax/face authority; the frontend lays
/// the style out locally over rope text it already holds. Reuses
/// [`crate::cell::Style`] so the grid and semantic projections share
/// one style vocabulary.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StyleSpan {
    /// Byte range this style covers.
    pub range: ByteRange,
    /// The resolved style (syntax highlight ∘ faces ∘ theme).
    pub style: crate::cell::Style,
}

/// T M11.4 — one dirty byte region of an `InstanceMessage::StyleSpans`
/// frame and the styling now covering it. The semantic analog of a
/// `CellDelta` changed-run: the frontend clears styling within
/// `range` and applies `spans` (already clipped to `range`). `spans`
/// is every current span intersecting `range`, not only changed ones,
/// so an unchanged span overlapping the dirty region is preserved.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StyleSegment {
    /// The byte region the frontend should clear and repaint.
    pub range: ByteRange,
    /// Spans intersecting `range`, each clipped to it.
    pub spans: Vec<StyleSpan>,
}

/// What a [`Decoration`] region *means*. Provisional variant set (see
/// the module-section note above). Peer selection is deliberately
/// absent — it stays on the `PresenceUpdate` path.
#[derive(Copy, Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DecorationKind {
    /// LSP diagnostic, error severity.
    DiagnosticError,
    /// LSP diagnostic, warning severity.
    DiagnosticWarning,
    /// LSP diagnostic, information severity.
    DiagnosticInfo,
    /// LSP diagnostic, hint severity.
    DiagnosticHint,
    /// The local selection region.
    Selection,
    /// A non-active search match.
    SearchMatch,
    /// The currently-focused search match.
    SearchMatchActive,
    /// The line containing the cursor.
    CurrentLine,
}

/// A byte range tagged with what it means. The frontend decides how
/// to paint each [`DecorationKind`] (squiggle, highlight, gutter
/// mark) — the instance only states the fact.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Decoration {
    /// Byte range the decoration covers.
    pub range: ByteRange,
    /// What the region signifies.
    pub kind: DecorationKind,
}

/// T M11.4 — `StyleSegment`'s analog for the `Decorations` family:
/// one dirty byte region and the decorations now covering it (every
/// current decoration intersecting `range`, clipped to it).
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DecorationSegment {
    /// The byte region the frontend should clear and repaint.
    pub range: ByteRange,
    /// Decorations intersecting `range`, each clipped to it.
    pub decorations: Vec<Decoration>,
}

/// Where an [`InlineAdornment`] sits relative to its anchor offset.
#[derive(Copy, Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum AdornmentPlacement {
    /// On its own, before the line containing `at`.
    BeforeLine,
    /// At the end of the line containing `at`.
    EndOfLine,
    /// Inline, exactly at the byte offset `at`.
    AtOffset,
}

/// Adornment payload: either inline styled text, or a handle into a
/// previously-sent [`InstanceMessage::ResourceOffer`] so out-of-band
/// content (images, blame avatars) is shipped once, not per frame.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum AdornmentContent {
    /// Styled virtual text.
    Text {
        /// The virtual text to display.
        text: String,
        /// Its style.
        style: crate::cell::Style,
    },
    /// A handle into a `ResourceOffer`.
    Resource {
        /// The offered resource's handle.
        handle: u64,
    },
}

/// Virtual text occupying no document bytes (inlay hints, blame,
/// lens). Anchored at a single offset; the frontend interleaves it
/// at layout time.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InlineAdornment {
    /// Buffer byte offset this adornment anchors to.
    pub at: u64,
    /// Placement relative to `at`.
    pub placement: AdornmentPlacement,
    /// What to render.
    pub content: AdornmentContent,
}

/// Content occupying its own vertical band (diff zones, folded-region
/// placeholders). Anchored to the offset of the line it precedes or
/// replaces. `replaces` is `Some` when the band stands in for a
/// collapsed region (the frontend renders the placeholder instead of
/// that range), `None` for an additive band. The frontend allocates
/// the vertical space — the instance never dictates pixel height.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BlockAdornment {
    /// Buffer byte offset of the line this band precedes/replaces.
    pub at: u64,
    /// The byte range this band stands in for, if it replaces one.
    pub replaces: Option<ByteRange>,
    /// What to render in the band.
    pub content: AdornmentContent,
}

/// The body of an [`InstanceMessage::ResourceOffer`] — inline bytes
/// for small payloads, or a URI the frontend resolves itself for
/// large or remote resources.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ResourceBody {
    /// The resource bytes, carried inline.
    Inline(Vec<u8>),
    /// A URI the frontend fetches/resolves on its own.
    Uri(String),
}
// ---------------------------------------------------------------------------
// Handshake — version, identity, capabilities
// ---------------------------------------------------------------------------

/// Wire-protocol version. Bumped on any breaking change to the
/// `Hello` / `AttachRequest` / event-message shapes.
///
/// The handshake compares against [`SUPPORTED_PROTOCOL_VERSIONS`];
/// mismatches close the connection with
/// [`GoodbyeReason::VersionMismatch`]. v1.0 servers and clients accept
/// either the v0.1 wire (version 1) or the v1.0 wire (version 2) per
/// `§sec:m10-backward-compat` — both directions of the version
/// asymmetry need symmetric relaxation so v0.1-era binaries connect
/// to v1.0-era binaries (and vice versa) once both have shipped.
///
/// T M10.5: bumped from 1 to 2. The v0.1 wire (version 1) remains
/// accepted by v1.0 binaries; CRDT-only message variants
/// (`InstanceMessage::CrdtOp`, `FrontendEvent::CrdtOp`) are filtered
/// per-session for v1 negotiated sessions.
///
/// T M11.1: bumped from 2 to 3. The v1.0 wire (version 2) remains
/// accepted; the semantic-frontend variant family
/// (`InstanceMessage::StyleSpans` … `ResourceOffer`,
/// `FrontendEvent::Viewport`) is filtered per-session for sessions
/// that did not negotiate `semantic_render`. Mechanically identical
/// to the M10.5 bump: the slice-membership handshake check (not
/// strict equality) means v0.1/v1.0 binaries keep connecting
/// unchanged, and the new variants simply existing in the enums is
/// not a wire-compat issue for non-semantic sessions because the
/// daemon never emits them to those sessions.
///
/// T M11.6: bumped from 3 to 4. v1, v2, v3 wires remain accepted;
/// [`InstanceMessage::DispatchIdle`] is filtered per-session for
/// sessions whose negotiated wire is `<= 3`. Same gating shape as the
/// `CrdtOp` / semantic-frontend bumps: the daemon's per-tick emission
/// checks the session's negotiated version and skips the variant for
/// older peers. An old peer would hard-error on decode of an unknown
/// postcard variant; gating prevents that.
///
/// Q#M1 (mouse framing): bumped from 4 to 5 for
/// [`FrontendEvent::Pointer`]. The gate runs in the *frontend* this
/// time (the new variant travels frontend→instance): a semantic
/// frontend sends `Pointer` only when the instance's
/// `Hello.protocol_version >= 5`, because an older instance would
/// hard-error decoding the unknown variant.
///
/// T M4.6 (diagnostic surface): bumped from 5 to 6 for
/// `Style::underline_color`. Unlike every previous bump, this one
/// changes the *encoding of an existing struct* — `Style` rides
/// inside `Cell` / `CellDelta` / `Snapshot` / `StyleSpans`, messages
/// every session receives — so per-session send gating cannot
/// preserve compatibility (postcard is not self-describing; a v5
/// decoder mis-reads any v6 `Style`). v6 binaries therefore accept
/// only v6 peers: a version-mismatched pair fails the handshake with
/// [`GoodbyeReason::VersionMismatch`] instead of garbling cell
/// traffic mid-session.
///
/// Q#M4 (mouse deferred set): bumped from 6 to 7 for
/// [`PointerKind::TripleDown`]. Back to the cheap additive shape:
/// a new variant on a frontend→instance enum, gated in the frontend
/// (sent only when the instance's `Hello.protocol_version >= 7`),
/// so the compat ladder restarts on the v6 encoding floor —
/// `SUPPORTED_PROTOCOL_VERSIONS` grows to `[6, 7]`.
///
/// Q#S1 (status band): bumped from 7 to 8 for
/// [`InstanceMessage::StatusFacts`]. Additive again, gated in the
/// *daemon* this time (the variant travels instance→frontend): the
/// per-session filter keeps it off wires negotiated `< 8`, the same
/// shape as the `DispatchIdle` (v4) gate.
///
/// Q#SR5 (incremental search): bumped from 8 to 9 for
/// [`InstanceMessage::SearchPrompt`]. Additive and daemon-gated per
/// session, identical shape to the `StatusFacts` (v8) bump.
///
/// Q#RX6 (regex search): bumped from 9 to 10 — `SearchPrompt` gained
/// `regex` / `invalid` fields, changing that variant's postcard
/// encoding. Still daemon-gated per session (now at `< 10`); a v9 peer
/// negotiates v9 and simply receives no `SearchPrompt` (the decorations
/// still highlight), rather than mis-decoding the wider shape.
///
/// UX gutter: bumped 12 → 13 for [`InstanceMessage::LineNumbers`] — a new
/// additive variant carrying the per-window line-number gutter mode.
/// Daemon-gated `< 13`; a v12 peer negotiates v12 and receives no
/// `LineNumbers` (its gutter simply stays off), like every prior additive
/// bump.
///
/// UX gutter modes: bumped 13 → 14 — `LineNumbers` swapped its `enabled:
/// bool` for a [`LineNumberMode`] enum so relative/hybrid ride the same
/// message. Encoding change to that variant; daemon-gated `< 14` (a v13
/// peer negotiates v13 and receives no `LineNumbers` rather than
/// mis-decoding the wider shape), same shape as the v10 `SearchPrompt` bump.
///
/// Completion popup (Arc 1a Q#C5): bumped 14 → 15 for
/// [`InstanceMessage::CompletionPopup`] — a new additive variant
/// carrying the byte-anchored in-buffer completion dropdown.
/// Daemon-gated `< 15`; a v14 peer negotiates v14 and simply receives
/// no `CompletionPopup` (completion still works via the daemon's TUI
/// rendering and the key round-trip), like every prior additive bump.
/// v15 also widened `StatusFacts` with the transient status `message`
/// (encoding change to that variant; its gate moved `>= 8` → `>= 15`,
/// so a v14 peer's status band goes dark rather than mis-decoding —
/// the v10 `SearchPrompt` / v14 `LineNumbers` shape).
///
/// Theme faces (Q#TH7): bumped 15 → 16 for
/// [`InstanceMessage::ThemeFacts`] — a new additive variant carrying
/// the daemon-resolved UI face table. Daemon-gated `< 16`; a v15 peer
/// negotiates v15 and simply receives no `ThemeFacts` (its chrome
/// stays on the frontend defaults), like every prior additive bump.
/// The variant is appended after `CompletionPopup` — the final v15
/// variant — because postcard discriminants are ordinal and an
/// earlier insertion would shift existing tags under v15 peers.
///
/// GPU font preference (Q#F4): bumped 16 → 17 for
/// [`InstanceMessage::FontFacts`] — a new additive variant relaying
/// the global font preference to GPU-capable peers. Daemon-gated
/// `< 17`; a v16 peer negotiates v16 and simply keeps its built-in
/// font. Appended after `ThemeFacts` — the final v16 variant —
/// same ordinal-discriminant reasoning as every additive bump.
///
/// Statusline segments (Q#SL7): bumped 17 → 18 for
/// [`InstanceMessage::StatuslineSegments`] — a new additive variant
/// carrying custom modeline provider output. Daemon-gated `< 18`; a
/// v17 peer keeps the built-in status band. Appended after `FontFacts`
/// so the final v17 discriminant remains stable.
///
/// Vterm Stage 3: bumped 18 → 19 for the terminal family —
/// [`InstanceMessage::TerminalFrame`] (daemon-gated `< 19`) plus
/// [`FrontendEvent::TerminalResize`] and
/// [`FrontendEvent::TerminalPointer`] (frontend-gated, sent only to a
/// `>= 19` instance). All three are appended after their enum's final
/// v18 variant, so the ladder resumes on the v6 encoding floor: a v18
/// grid peer keeps receiving Stage 2's composed `CellDelta` terminal
/// windows, and a v18 semantic peer keeps ordinary document editing
/// with no terminal surface at all. This is the first bump to gate in
/// BOTH directions at once, which is why criterion 28 pins the two
/// send filters independently.
///
/// GPU initial target (Q#GT4): bumped 19 → 20 for the semantic-session
/// bootstrap envelope and [`InstanceMessage::InitialTargetResult`]. The
/// handshake extension is read only from v20 semantic sessions; the result is
/// sent only when such a session requested a target. v6–v19 handshakes and
/// message discriminants remain unchanged.
///
/// Bottom panel Stage 2 (Q#BP9): bumped 20 → 21 for
/// [`InstanceMessage::PanelFrame`] and
/// [`FrontendEvent::{FrontendCellGeometry, PanelResizeRows, PanelPointer}`].
/// All four are appended after their enum's previous final variant, so
/// no v6–v20 discriminant moves and the encoding of every existing
/// message is byte-identical. The new traffic is gated in both
/// directions: a v20 peer neither receives `PanelFrame` nor is placed in
/// a side window, because denying only the events would leave its
/// window invisible.
pub const PROTOCOL_VERSION: u32 = 21;

/// Protocol version placed in the daemon's server-first [`Hello`].
///
/// **This is a compatibility *baseline*, not a ceiling, and Stage 2B-3
/// makes that permanent.** The handshake is server-first: the daemon
/// writes [`Hello`] before the frontend has said anything at all, and a
/// frontend rejects an unrecognized `protocol_version` *before* it can
/// send [`AttachRequest`]. Advertising [`PROTOCOL_VERSION`] here would
/// therefore lock out every already-shipped frontend whose supported
/// range ends lower — an incompatible act on its own, independent of
/// whether a single new message is ever exchanged.
///
/// So the baseline stays at the highest version every shipped frontend
/// is known to accept, and the session's actual version is settled by
/// the frontend's [`AttachRequest`] instead:
///
/// 1. the daemon advertises this baseline;
/// 2. the frontend answers with [`requested_protocol_version`] — its own
///    [`PROTOCOL_VERSION`] when the baseline is this constant, and a
///    verbatim echo of anything older;
/// 3. the daemon negotiates [`negotiated_session_version`] of that offer.
///
/// A shipped baseline-version frontend echoes the baseline and gets a
/// baseline session, exactly as before. A current frontend offers up and
/// gets the current wire. Nothing about the `Hello` encoding or its
/// value changes, which is why the old frontend never sees a version it
/// must reject.
///
/// Moving this constant is therefore a **deliberately incompatible**
/// act, reserved for a wire change that cannot be expressed additively.
/// An additive family — like the bottom panel's v21 shapes — never needs
/// it.
pub const ADVERTISED_PROTOCOL_VERSION: u32 = 20;

/// The version a frontend puts in its [`AttachRequest`], given the
/// server-first [`Hello`] baseline it just read.
///
/// The counter-offer is confined to the *current* baseline on purpose. A
/// daemon advertising anything other than [`ADVERTISED_PROTOCOL_VERSION`]
/// is genuinely older than this ladder rung, so its baseline is echoed
/// verbatim and that attachment takes byte-for-byte the pre-Stage-2B-3
/// path. Only the current baseline — the one every daemon built from
/// this ladder sends — is answered with this binary's own
/// [`PROTOCOL_VERSION`].
///
/// The offer is never *lower* than the baseline: a frontend that
/// supported less than the daemon advertised would already have rejected
/// the `Hello` via [`is_supported_protocol_version`].
///
/// # The one-way window this leaves open
///
/// A daemon whose own `PROTOCOL_VERSION` *equals* the baseline also
/// advertises the baseline, and rejects an offer above its supported
/// range with [`GoodbyeReason::VersionMismatch`]. That is the price of a
/// server-first handshake with no client-first hint: compatibility can
/// be preserved for old *frontends* (the direction that matters, since
/// the daemon is what a user leaves running) or for old *daemons*, but a
/// single `AttachRequest` cannot mean both "I want 21" and "≤ 20" at
/// once. The window closes as soon as the running daemon is restarted on
/// a binary from this ladder rung or later, and it is one-connection
/// visible — [`GoodbyeReason::VersionMismatch`] names both versions.
#[must_use]
pub fn requested_protocol_version(server_baseline: u32) -> u32 {
    if server_baseline == ADVERTISED_PROTOCOL_VERSION {
        PROTOCOL_VERSION
    } else {
        server_baseline
    }
}

/// The version a daemon records for a session, given the frontend's
/// [`AttachRequest`] offer.
///
/// The offer has already passed [`is_supported_protocol_version`], so
/// this clamp cannot bind today; it is here because "the session speaks
/// the lower of the two ceilings" is the *rule*, and leaving it implicit
/// in a membership test is how a future ladder widening (accepting a
/// version this binary cannot itself produce) would silently ship an
/// over-negotiated session.
#[must_use]
pub fn negotiated_session_version(frontend_offer: u32) -> u32 {
    frontend_offer.min(PROTOCOL_VERSION)
}

/// T M10.5: the set of protocol versions a v1.0 binary accepts on
/// the wire. v0.1 binaries only accepted `[1]`; v1.0 binaries accept
/// `[1, 2]` so the version asymmetry the §sec:m10-backward-compat
/// spec section describes is handled symmetrically on both sides.
///
/// T M11.1: extended to `[1, 2, 3]`. v1.1 binaries accept the v0.1
/// (1), v1.0 (2), and semantic-frontend (3) wires. The check remains
/// slice membership — "is the peer's `protocol_version` present in
/// this slice?" — not strict equality on `PROTOCOL_VERSION`. The
/// session's negotiated version (the peer's) is recorded for
/// downstream filtering: v1 sessions don't receive
/// `InstanceMessage::CrdtOp` / `PresenceUpdate` messages even from
/// a v3 daemon, and only sessions that negotiated `semantic_render`
/// receive the `SemanticFrame` variant family.
///
/// T M11.6: extended to `[1, 2, 3, 4]`. v4 sessions receive
/// `InstanceMessage::DispatchIdle`; older sessions are filtered out
/// of that emission.
///
/// Mouse framing Q#M1: extended to `[1, 2, 3, 4, 5]`. v5 peers may
/// send `FrontendEvent::Pointer`; the frontend-side gate (see
/// [`PROTOCOL_VERSION`]) keeps the variant off wires negotiated `< 5`.
///
/// T M4.6: narrowed to `[6]`. `Style::underline_color` changed the
/// postcard encoding of every cell-carrying message, so v6 binaries
/// cannot exchange cell traffic with any earlier wire. The v1–v5
/// compat ladder (additive variants, per-session filtering) assumed
/// shared-struct encodings never changed; this bump is the first
/// that breaks that assumption, and slice membership is how the
/// handshake communicates it.
///
/// Q#M4: extended to `[6, 7]`. `PointerKind::TripleDown` is additive
/// and frontend-gated (like `Pointer` itself at v5), so the ladder
/// resumes: v6 and v7 binaries interoperate, with the new variant
/// kept off wires whose instance negotiated `< 7`.
///
/// Q#S1: extended to `[6, 7, 8]`. `InstanceMessage::StatusFacts` is
/// additive and daemon-gated per session.
///
/// Q#SR5: extended to `[6, 7, 8, 9]`. `InstanceMessage::SearchPrompt`
/// is additive and daemon-gated per session.
///
/// Q#RX6: extended to `[6, 7, 8, 9, 10]`. `SearchPrompt` gained
/// `regex` / `invalid` (encoding change to that variant); v9 and v10
/// interoperate because the variant is daemon-gated per session, so a
/// v9 peer is simply never sent the wider shape.
///
/// Q#CM1: extended to `[6, 7, 8, 9, 10, 11]`. The context menu adds
/// `PointerKind::Context` (frontend-gated like `Pointer`/`TripleDown`),
/// `FrontendEvent::MenuPointer`, and `InstanceMessage::MenuPrompt`
/// (daemon-gated per session) — all additive, so the ladder resumes.
///
/// Q#MB1: extended to `[6, 7, 8, 9, 10, 11, 12]`.
/// `InstanceMessage::MinibufferPrompt` is additive and daemon-gated per
/// session, so the ladder resumes again.
///
/// Q#C5: extended to `[6, ..., 15]`. `InstanceMessage::CompletionPopup`
/// is additive and daemon-gated per session.
///
/// Q#TH7: extended to `[6, ..., 16]`. `InstanceMessage::ThemeFacts`
/// is additive and daemon-gated per session, so the ladder resumes.
///
/// Q#F4: extended to `[6, ..., 17]`. `InstanceMessage::FontFacts`
/// is additive and daemon-gated per session, so the ladder resumes.
///
/// Q#SL7: extended to `[6, ..., 18]`.
/// [`InstanceMessage::StatuslineSegments`] is additive and daemon-gated.
///
/// Vterm Stage 3: extended to `[6, ..., 19]`. The terminal family is
/// additive in both directions — `TerminalFrame` is daemon-gated,
/// `TerminalResize` / `TerminalPointer` are frontend-gated — so v18 and
/// v19 binaries interoperate with terminal traffic simply absent.
///
/// GPU initial target (Q#GT4): extended to `[6, ..., 20]`. v20 semantic
/// sessions send a bounded bootstrap envelope after `AttachRequest`; legacy
/// and non-semantic sessions retain their existing handshake shape.
///
/// Bottom panel Stage 2 (Q#BP9): extended to `[6, ..., 21]`. Stage 2B-1
/// reserves and validates the v21 wire while production daemons continue
/// to send [`ADVERTISED_PROTOCOL_VERSION`] in their server-first
/// [`Hello`]. The later capability-activation slice owns moving production
/// negotiation to v21 without making existing v20 frontends reject the
/// handshake.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[u32] =
    &[6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21];

/// T M10.5: predicate for the handshake check. Returns `true` if
/// `peer_version` is in [`SUPPORTED_PROTOCOL_VERSIONS`].
#[must_use]
pub fn is_supported_protocol_version(peer_version: u32) -> bool {
    SUPPORTED_PROTOCOL_VERSIONS.contains(&peer_version)
}

/// Identifies an instance for client-side display.
///
/// Sent inside [`Hello`] from instance to frontend. Use of `uptime_secs`
/// instead of an absolute start time is deliberate: instance and
/// frontend may run on machines whose clocks disagree, so the frontend
/// computes "instance has been running N seconds" using only the
/// instance's view of time.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InstanceIdentity {
    /// Pmacs version string (`env!("CARGO_PKG_VERSION")`).
    pub pmacs_version: String,
    /// Short git hash if the build embedded one. `None` for releases or
    /// source-tarball builds where no git checkout was available.
    pub build_hash: Option<String>,
    /// The name the instance was launched under (`--socket NAME`).
    /// `None` for the default daemon (no `--socket` argument).
    pub instance_name: Option<String>,
    /// Seconds since the instance started, from the instance's clock.
    /// Frontend displays "running 47m" by interpreting this against
    /// its own notion of "now," avoiding cross-machine clock skew.
    pub uptime_secs: u64,
    /// Working directory the instance is running in. Encoded as a
    /// UTF-8 string; non-UTF-8 paths are rejected at the boundary.
    pub working_directory: String,
}

impl InstanceIdentity {
    /// Build an identity for the running pmacs process.
    ///
    /// `instance_name` is the user-facing name (typically the
    /// `--socket NAME` value for the daemon path; `None` for the
    /// in-process Local mode and the unnamed default daemon).
    /// `started` is the wall-clock anchor used to compute
    /// [`Self::uptime_secs`]; the elapsed seconds are evaluated at the
    /// call site, so calling twice on different days surfaces different
    /// uptimes from the same anchor.
    ///
    /// The version comes from `CARGO_PKG_VERSION` and the build hash
    /// from the optional `PMACS_GIT_HASH` environment variable populated
    /// by the build script.
    #[must_use]
    pub fn for_running_process(instance_name: Option<String>, started: std::time::Instant) -> Self {
        Self {
            pmacs_version: env!("CARGO_PKG_VERSION").into(),
            build_hash: option_env!("PMACS_GIT_HASH").map(String::from),
            instance_name,
            uptime_secs: started.elapsed().as_secs(),
            working_directory: std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }
}

/// Capabilities the instance advertises to attaching frontends.
///
/// Empty for v0.1; the type exists so that adding capabilities in v0.2+
/// is not a breaking-change. Symmetric with [`FrontendCapabilities`].
///
/// T M10.5: added `multi_frontend` and `crdt_replica` bits with
/// `#[serde(default)]` so v1 wire bytes still deserialize. The
/// negotiation logic (which side advertises what, and what the
/// instance does with mismatches) is M10.7 scope; M10.5 just makes
/// the bit positions stable in the wire format.
///
/// T M10.5/8: bit defaults evolve with the substrate.
///
/// - M10.5 declared the bits with `#[serde(default)]` so v1 wire
///   bytes deserialize forward-compatibly. M10.5–M10.7 set both bits
///   to `false` so a frontend declaring `multi_frontend: true` got
///   `Goodbye(CapabilityMismatch)` — the multi-frontend path
///   wasn't actually wired yet.
/// - **T M10.8 Day 4 flip**: the instance's `multi_frontend` and
///   `crdt_replica` defaults flip to `true`. This is the "M10.8 enables
///   multi-frontend" moment — the underlying dispatcher (Day 3) and
///   broadcast routing (Day 4) support both capabilities, so the
///   instance advertises them.
///
/// The frontend-side defaults remain `false` (a frontend that omits
/// the field is conservatively treated as not supporting the
/// capability; matches v0.1 wire-format semantics).
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InstanceCapabilities {
    /// T M10.5: instance can host multi-frontend sessions on the
    /// same buffer (per `§sec:m10-collab`). T M10.8 Day 4: default
    /// flipped to `true` — the dispatcher supports multiple
    /// attached frontends.
    #[serde(default = "default_true")]
    pub multi_frontend: bool,
    /// T M10.5: instance can broadcast `InstanceMessage::CrdtOp`
    /// messages. T M10.8 Day 4: default flipped to `true` — the
    /// broadcast routing for CRDT ops wires up in this milestone.
    #[serde(default = "default_true")]
    pub crdt_replica: bool,
    /// T M11.1: instance can produce the semantic-frontend variant
    /// family (`InstanceMessage::StyleSpans` … `ResourceOffer`) and
    /// consume `FrontendEvent::Viewport`.
    ///
    /// T M11.1 declared the bit position + negotiation mechanics with
    /// the default `false` (no producer yet). T M11.2 landed the
    /// instance-side projection seam (`SemanticRenderState`) and
    /// flipped the default to `cfg!(feature = "crdt")`: the instance
    /// now advertises `semantic_render` on CRDT builds. It tracks the
    /// `crdt` feature rather than being unconditional because the
    /// negotiation dependency rule makes a semantic session
    /// necessarily a text replica — a non-CRDT build can host
    /// neither. See [`Default`] impl below.
    #[serde(default)]
    pub semantic_render: bool,
}

// Clippy in non-CRDT builds notes that `cfg!(feature = "crdt")`
// evaluates to `false`, making this impl derivable. In CRDT builds
// the values are `true`, so the impl is genuinely manual. Allow.
#[allow(clippy::derivable_impls)]
impl Default for InstanceCapabilities {
    fn default() -> Self {
        // T M10.10 — the `crdt_replica` default tracks the `crdt`
        // Cargo feature. A daemon built without the `crdt` feature
        // can't honor a `crdt_replica: true` negotiation (the
        // CRDT-handling code paths are conditionally compiled out
        // — `send_buffer_snapshots`, `apply_remote_crdt_op`, the
        // dispatcher's CursorByte emit). Advertising `true`
        // unconditionally would be wire-protocol false advertising.
        //
        // `multi_frontend` is conceptually independent of CRDT but
        // in M10.10's architecture every multi-frontend participant
        // is also a CRDT replica; gating both on the same feature
        // keeps the daemon's advertised capabilities consistent
        // with what it can actually do.
        //
        // T M11.1 declared `semantic_render` defaulting to `false`
        // unconditionally — no projection-seam producer existed, so
        // advertising it would have been wire-protocol false
        // advertising (the M10.5→M10.7 "bits false until the path is
        // wired" discipline).
        //
        // T M11.2 — **the flip**: the instance-side projection seam
        // (`SemanticRenderState`, the producer) has landed and the
        // dispatcher selects it per session, so the instance now
        // advertises `semantic_render`. It tracks `cfg!(feature =
        // "crdt")` like `crdt_replica` because the negotiation
        // dependency rule makes a semantic session necessarily a
        // text replica; a non-CRDT build can host neither. This is
        // the "M11.2 enables semantic" moment, exactly analogous to
        // the M10.8 Day-4 multi_frontend/crdt_replica flip.
        Self {
            multi_frontend: cfg!(feature = "crdt"),
            crdt_replica: cfg!(feature = "crdt"),
            semantic_render: cfg!(feature = "crdt"),
        }
    }
}

#[allow(clippy::missing_const_for_fn)]
fn default_true() -> bool {
    true
}

/// Capabilities the frontend advertises to the instance.
///
/// All bools default to `false` so a frontend that omits a field via an
/// older `AttachRequest` is conservatively treated as not supporting
/// the capability. New capabilities added in v0.2+ get
/// `#[serde(default)]` so old wire bytes still deserialize.
// A capability set is exactly the case `struct_excessive_bools` warns
// against — but each flag is independent and the alternative (an enum
// or bitset) loses the per-field `#[serde(default)]` semantics that
// make schema evolution work.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FrontendCapabilities {
    /// Frontend understands DEC 2026 `BeginSynchronizedUpdate` /
    /// `EndSynchronizedUpdate` markers. Instance strips them when false.
    #[serde(default)]
    pub synchronized_output: bool,
    /// Frontend can render Unicode beyond the Basic Multilingual Plane.
    /// Instance can substitute a fallback glyph when false.
    #[serde(default)]
    pub unicode_smp: bool,
    /// Frontend supports 24-bit color (truecolor SGR sequences).
    /// Instance maps to the 256-color palette when false.
    #[serde(default)]
    pub true_color: bool,
    /// Frontend captures and forwards mouse events.
    #[serde(default)]
    pub mouse: bool,
    /// Frontend supports bracketed paste — distinguishes pasted bytes
    /// from typed bytes. Instance treats all input as keystrokes when false.
    #[serde(default)]
    pub bracketed_paste: bool,
    /// Optional human-readable terminal identifier for logs and
    /// debugging only. The instance does not branch on this value;
    /// branching is done on the explicit capability bits above.
    #[serde(default)]
    pub terminal_kind: Option<String>,
    /// T M10.5: frontend can participate in multi-frontend sessions
    /// (per `§sec:m10-collab`). false for v0.1 frontends — they
    /// attach as single-frontend and never receive `CrdtOp` /
    /// `PresenceUpdate` broadcasts. v1.0 frontends opt in via M10.7's
    /// negotiation handshake. M10.5 declares the bit position; M10.7
    /// wires the negotiation.
    ///
    /// Default is `false` — v1 frontends are treated as not
    /// supporting this feature, which matches reality (v1 frontends
    /// have no local CRDT state). A `true` default would have v1
    /// frontends claimed to support features they don't.
    #[serde(default)]
    pub multi_frontend: bool,
    /// T M10.5: frontend can apply incoming `CrdtOp` messages to a
    /// local CRDT state. false for v0.1; v1.0 opts in. M10.7 wires
    /// negotiation; M10.5 declares the bit position.
    #[serde(default)]
    pub crdt_replica: bool,
    /// T M11.1: frontend is a semantic (layout-local) renderer — it
    /// consumes the `InstanceMessage::StyleSpans` … `ResourceOffer`
    /// family and emits `FrontendEvent::Viewport`. false for v0.1 and
    /// v1.0 grid/TUI frontends; a future GPU/GUI frontend opts in.
    ///
    /// A semantic frontend is *required* to also be a text replica:
    /// the semantic frame ships no text, so the frontend must hold
    /// the rope locally via the `crdt_replica` machinery. This
    /// dependency is enforced in [`negotiate_capabilities`], not just
    /// documented — declaring `semantic_render: true` without
    /// `crdt_replica: true` is a capability mismatch, never a silent
    /// degrade.
    #[serde(default)]
    pub semantic_render: bool,
}

/// T M10.7 — the negotiated capability bits for one attached session.
///
/// Computed by [`negotiate_capabilities`] from the frontend's
/// [`FrontendCapabilities`] and the instance's [`InstanceCapabilities`].
/// Each negotiated bit is the AND of the two declared bits. Fields
/// added here in future milestones append at the end with sensible
/// defaults so existing call sites stay valid.
///
/// This is a daemon-internal struct (not on the wire); the
/// negotiation result is communicated to the frontend via the
/// success of the handshake (no capability-mismatch `Goodbye`) and
/// the instance's behavior thereafter.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct NegotiatedCapabilities {
    /// Session is eligible for multi-frontend operation. True iff
    /// both the frontend and the instance declared `multi_frontend =
    /// true`. v0.1 frontends always end up here as `false` (the v0.1
    /// wire format does not carry the field; `#[serde(default)]`
    /// makes the deserialized value `false`).
    pub multi_frontend: bool,
    /// Session can produce/consume `InstanceMessage::CrdtOp` /
    /// `FrontendEvent::CrdtOp`. True iff both sides declared
    /// `crdt_replica = true`. The daemon's outgoing-message filter for
    /// `CrdtOp` consults this in M10.8.
    pub crdt_replica: bool,
    /// T M11.1 — session uses the semantic projection: it
    /// produces/consumes the `InstanceMessage::StyleSpans` …
    /// `ResourceOffer` family and `FrontendEvent::Viewport`. True iff
    /// both sides declared `semantic_render = true` *and* the session
    /// also negotiated `crdt_replica = true` (a semantic session is a
    /// text replica; see [`negotiate_capabilities`]). The daemon's
    /// per-session outgoing filter gates the entire semantic family
    /// on this bit — wired with the producer in M11.2.
    pub semantic_render: bool,
}

/// T M10.7 — pure-function capability negotiation.
///
/// For each negotiated bit (`multi_frontend`, `crdt_replica`,
/// `semantic_render`):
///
/// | Frontend wants | Instance has | Result |
/// |----------------|--------------|--------|
/// | `false`        | `false`      | bit `false`, no error |
/// | `false`        | `true`       | bit `false`, no error |
/// | `true`         | `true`       | bit `true`,  no error |
/// | `true`         | `false`      | bit appears in `missing` |
///
/// If any bit ends up in `missing`, the negotiation fails as a whole
/// (returns `Err`). Otherwise the negotiated bits are returned as
/// [`NegotiatedCapabilities`]. The `Err` form gathers ALL missing
/// bits into one `CapabilityMismatch` — one round-trip carries the
/// complete picture rather than serial rejections. Missing bits are
/// ordered `multi_frontend`, `crdt_replica`, `semantic_render` for
/// deterministic wire output.
///
/// # T M11.1 — the `semantic_render ⇒ crdt_replica` dependency
///
/// A semantic-render session ships no text on the semantic frame;
/// the frontend holds the rope locally via the `crdt_replica`
/// machinery (`BufferSnapshot` to bootstrap, `CrdtOp` to stay live).
/// So `semantic_render` is only coherent on a session that also
/// negotiated `crdt_replica`. When the AND-rule would yield
/// `semantic_render = true` but the session did not also negotiate
/// `crdt_replica = true`, this function rejects with
/// `"semantic_render"` in `missing` rather than silently degrading
/// the session to a text-only replica. The rejected identifier is
/// `"semantic_render"` (the capability whose precondition is unmet),
/// not `"crdt_replica"`.
///
/// # Wire-format stability
///
/// The strings emitted into `missing` are exactly the
/// `FrontendCapabilities` field names (`"multi_frontend"`,
/// `"crdt_replica"`). These are stable wire-format identifiers, not
/// human-readable descriptions. User-facing translation is the
/// frontend's responsibility (see [`AttachError`]'s `Display` impl).
/// Renaming a capability bit requires updating both the field name
/// and the missing-string emission here in lockstep.
pub fn negotiate_capabilities(
    frontend: &FrontendCapabilities,
    instance: &InstanceCapabilities,
) -> Result<NegotiatedCapabilities, GoodbyeReason> {
    let mut missing = Vec::new();
    let multi_frontend = match (frontend.multi_frontend, instance.multi_frontend) {
        (true, false) => {
            missing.push("multi_frontend".to_string());
            false
        }
        (a, b) => a && b,
    };
    let crdt_replica = match (frontend.crdt_replica, instance.crdt_replica) {
        (true, false) => {
            missing.push("crdt_replica".to_string());
            false
        }
        (a, b) => a && b,
    };
    let semantic_render = match (frontend.semantic_render, instance.semantic_render) {
        (true, false) => {
            missing.push("semantic_render".to_string());
            false
        }
        (a, b) => a && b,
    };
    // T M11.1 — dependency rule. A semantic session is a text replica
    // (the semantic frame carries no text). If both sides declared
    // `semantic_render` but the session did not also negotiate
    // `crdt_replica`, reject rather than silently degrade. Guard
    // against a duplicate push: the only path where `semantic_render`
    // is already in `missing` is the `(true, false)` arm above, which
    // also sets the local `semantic_render` to false, so the
    // condition below cannot re-fire for that case — but the explicit
    // membership check keeps this robust against future reordering.
    if semantic_render && !crdt_replica && !missing.iter().any(|m| m == "semantic_render") {
        missing.push("semantic_render".to_string());
    }
    let semantic_render = semantic_render && crdt_replica;
    if missing.is_empty() {
        Ok(NegotiatedCapabilities {
            multi_frontend,
            crdt_replica,
            semantic_render,
        })
    } else {
        Err(GoodbyeReason::CapabilityMismatch { missing })
    }
}

/// First message sent by the instance to a freshly-attached frontend.
///
/// Sent immediately after the connection is accepted, before reading
/// the frontend's [`AttachRequest`]. The frontend uses
/// `instance_identity` for status display and `protocol_version` /
/// `instance_capabilities` for compatibility decisions.
///
/// The instance also stamps the `assigned_frontend_id` which the
/// frontend will use as the `FrontendId` on every event it sends.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Hello {
    /// The protocol version this attachment should use.
    ///
    /// This can deliberately trail [`PROTOCOL_VERSION`] while an additive
    /// wire family is reserved but not yet activated in production.
    pub protocol_version: u32,
    /// `FrontendId` assigned to this attachment by the instance. The
    /// frontend stamps this onto subsequent events. v0.1 daemons start
    /// allocation at `FrontendId(2)` (1 reserved for the in-process TUI).
    pub assigned_frontend_id: FrontendId,
    /// Instance self-identification (version, name, uptime, cwd).
    pub instance_identity: InstanceIdentity,
    /// Instance capabilities. Empty for v0.1.
    pub instance_capabilities: InstanceCapabilities,
}

/// First message sent by a frontend after receiving [`Hello`].
///
/// Carries the frontend's view of the protocol version, the
/// capabilities it can support, and its initial terminal size. On
/// version mismatch the instance closes with
/// [`GoodbyeReason::VersionMismatch`] and no further messages flow.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AttachRequest {
    /// The frontend's `PROTOCOL_VERSION`.
    pub protocol_version: u32,
    /// Frontend capabilities. Defaults to all-false if omitted.
    #[serde(default)]
    pub frontend_capabilities: FrontendCapabilities,
    /// The frontend's terminal size at attach time. Authoritative
    /// until the frontend sends a [`FrontendEvent::Resize`]. The
    /// instance uses this for the initial full-grid render.
    pub initial_size: CellSize,
}

/// Maximum byte length of either raw Unix path in an initial-target request.
pub const MAX_INITIAL_TARGET_PATH_BYTES: usize = 32 * 1024;

/// Maximum UTF-8 byte length of a daemon-produced initial-target error.
pub const MAX_INITIAL_TARGET_ERROR_BYTES: usize = 4 * 1024;

/// Raw local paths for a semantic session's pre-window initial target.
///
/// Both fields are Unix path bytes rather than display text. The daemon
/// validates the byte bounds, absolute `cwd`, nonempty fields, and embedded
/// NULs before constructing paths.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InitialTarget {
    /// Absolute launcher working-directory bytes.
    pub cwd: Vec<u8>,
    /// Launcher-expanded target path bytes, absolute or relative to `cwd`.
    pub path: Vec<u8>,
}

/// Protocol-v20 semantic-session bootstrap extension.
///
/// A v20 semantic frontend sends this immediately after [`AttachRequest`].
/// `None` preserves ordinary attach behavior.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionBootstrapRequest {
    /// Optional file that must be ready before the frontend creates a window.
    pub initial_target: Option<InitialTarget>,
}

/// Pre-window outcome for a requested initial target.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum InitialTargetResult {
    /// The target snapshot was written and the session is ready.
    Opened {
        /// Buffer identified by the immediately preceding target snapshot.
        buffer_id: crate::BufferId,
    },
    /// Bootstrap failed; the provisional session has been removed.
    Failed {
        /// Bounded user-facing daemon detail.
        message: String,
    },
}
