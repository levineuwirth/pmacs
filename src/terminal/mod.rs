//! Stateful terminal core and process-session ownership.
//!
//! Terminal buffers are identity/lifecycle anchors. Visible contents live in
//! [`screen::TerminalScreen`] and are exposed as owned session snapshots.

/// Terminal input byte encoders.
pub mod input;
/// Stateful terminal screen model.
pub mod screen;
pub mod session;

pub use session::{
    SharedTerminalManager, TerminalError, TerminalManager, TerminalProcessState,
    TerminalSelectionSpan, TerminalSnapshot, TerminalSpec,
};

/// Maximum terminal rows accepted at creation or resize.
pub const MAX_TERMINAL_ROWS: u16 = 512;
/// Maximum terminal columns accepted at creation or resize.
pub const MAX_TERMINAL_COLS: u16 = 512;
/// Maximum visible terminal cells accepted at creation or resize.
pub const MAX_TERMINAL_VISIBLE_CELLS: usize = 262_144;
/// Maximum UTF-8 bytes retained in one terminal grapheme cluster.
pub const MAX_TERMINAL_GRAPHEME_BYTES: usize = 256;
/// Default retained main-screen scrollback rows.
pub const DEFAULT_TERMINAL_SCROLLBACK_ROWS: usize = 10_000;
/// Maximum retained main-screen history cells.
pub const MAX_TERMINAL_HISTORY_CELLS: usize = 4_000_000;
/// Shared cap for terminal title and process-outcome metadata.
pub const MAX_TERMINAL_METADATA_BYTES: usize = 1_024;
