//! Stateful terminal core and process-session ownership.
//!
//! Terminal buffers are identity/lifecycle anchors. Visible contents live in
//! [`screen::TerminalScreen`] and are exposed as owned session snapshots.

/// Terminal input byte encoders.
pub mod input;
/// Stateful terminal screen model.
pub mod screen;
pub mod session;
/// Per-context terminal viewport, selection, and controller identities.
pub mod view;

pub use session::{
    SharedTerminalManager, TerminalError, TerminalManager, TerminalSnapshot, TerminalSpec,
};
pub use view::{
    LogicalCellAnchor, TerminalController, TerminalSelection, TerminalViewKey, TerminalViewState,
};

// Vterm Stage 3: the screen bounds and the process/selection payload
// types moved to `pmacs-protocol` so the daemon's pre-emission check and
// a frontend's post-decode check run the SAME policy. Re-exported here
// so Stage 1/2 callers keep their `crate::terminal::…` paths and no
// duplicate type appears in the tree.
pub use pmacs_protocol::terminal::{
    MAX_TERMINAL_COLS, MAX_TERMINAL_FRAME_GLYPH_BYTES, MAX_TERMINAL_GRAPHEME_BYTES,
    MAX_TERMINAL_METADATA_BYTES, MAX_TERMINAL_ROWS, MAX_TERMINAL_VISIBLE_CELLS, TerminalFrame,
    TerminalFrameError, TerminalProcessState, TerminalSelectionSpan,
};

/// Default retained main-screen scrollback rows.
///
/// Configuration-time, not a wire bound: history never crosses the
/// protocol, so this stays core-owned.
pub const DEFAULT_TERMINAL_SCROLLBACK_ROWS: usize = 10_000;
/// Maximum retained main-screen history cells. Core-owned for the same
/// reason as [`DEFAULT_TERMINAL_SCROLLBACK_ROWS`].
pub const MAX_TERMINAL_HISTORY_CELLS: usize = 4_000_000;
