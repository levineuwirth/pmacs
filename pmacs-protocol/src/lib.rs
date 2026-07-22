//! Wire types for the pmacs daemon ↔ frontend protocol.
//!
//! Session 1 of the pmacs-gpu arc — see `docs/pmacs-gpu-design.md` in
//! the workspace root. This crate owns every type that appears on the
//! `InstanceMessage` / `FrontendEvent` wire so a future `pmacs-gpu`
//! frontend can depend on it directly without pulling in the `pmacs`
//! main crate (and its Lua / tree-sitter / process-supervisor surface).
//!
//! What lives here:
//! - The `SemanticFrame` family: `StyleSpans`, `Decorations`,
//!   `InlineAdornments`, `BlockAdornments`, `FoldState`,
//!   `ResourceOffer`, `FileStyleSummary`.
//! - The grid-rendering family: `CellDelta`, plus `Cell`, `Glyph`,
//!   `Style`, `Color`, `UnderlineStyle`, `CellCoord`, `CellSize`,
//!   `DiffSpan`, `Attachment`.
//! - Identity types: `BufferId`, `FrontendId`, `Position`, `ByteRange`.
//! - The full message envelopes: `InstanceMessage`, `FrontendEvent`,
//!   `GoodbyeReason`, capability structs, `PresenceUpdate`, etc.
//! - The optional `CrdtOp` wire variant (feature-gated on `crdt`).
//! - [`TAB_STOP_COLUMNS`], the shared logical width used when frontends
//!   project raw buffer tabs for display.
//!
//! What does NOT live here:
//! - `crate::cell::CellGrid` and `crate::cell::diff()` (rendering
//!   helpers, not wire types — stay in the `pmacs` crate).
//! - `Buffer` / `BufferRegistry` / `Rope` / `Edit` / `Range`
//!   (instance-side editor machinery).
//! - `AttachTarget` and the attach-CLI binding error types
//!   (`pmacs`-binary-only logic; `pmacs-gpu` builds its own attach
//!   client).
//! - Lua / tree-sitter / process-supervisor everything.
//!
//! The `pmacs` crate re-exports back through its existing module paths
//! (`crate::cell::Style`, `crate::buffer::BufferId`, etc.) so internal
//! pmacs code doesn't churn its imports. New consumers
//! (`pmacs-gpu`, debug tools, future ports) depend on this crate
//! directly.

pub mod cell;
pub mod crdt;
pub mod ids;
pub mod message;
pub mod transport;

/// Logical display columns between fixed buffer-text tab stops.
///
/// Semantic frames keep tabs as source bytes; every frontend expands them
/// only in its display projection so protocol byte ranges remain unchanged.
pub const TAB_STOP_COLUMNS: u32 = 8;

pub use cell::{
    Attachment, Cell, CellCoord, CellSize, Color, DiffSpan, Glyph, Style, UnderlineStyle,
};
pub use crdt::CrdtOp;
pub use ids::{BufferId, ByteRange, FrontendId, Position};
pub use message::{
    AdornmentContent, AdornmentPlacement, AttachRequest, BUILTIN_PAIR_CHARS, BlockAdornment,
    CompletionPopupRow, CursorState, Decoration, DecorationKind, DecorationSegment,
    FrontendCapabilities, FrontendEvent, GoodbyeReason, Hello, InlineAdornment,
    InstanceCapabilities, InstanceIdentity, InstanceMessage, InstanceSignal, Key, KeyEvent,
    LineNumberMode, MAX_STATUSLINE_FACE_BYTES, MAX_STATUSLINE_PROVIDER_NAME_BYTES,
    MAX_STATUSLINE_PROVIDERS, MAX_STATUSLINE_SEGMENT_BYTES, MAX_STATUSLINE_TOTAL_TEXT_BYTES,
    MenuPromptRow, Modifiers, MouseButton, MouseEvent, MouseKind, NegotiatedCapabilities,
    PROTOCOL_VERSION, PointerKind, ResourceBody, SUPPORTED_PROTOCOL_VERSIONS, SelectionSnapshot,
    StatuslineSegment, StyleSegment, StyleSpan, ThemeFace, is_builtin_pair_char,
    is_modeline_face_name, is_supported_protocol_version, is_ui_face_name, negotiate_capabilities,
};
pub use transport::{MAX_FRAME_BYTES, TransportError, read_message, write_message};
