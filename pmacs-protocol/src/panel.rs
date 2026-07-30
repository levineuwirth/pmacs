//! Bottom-panel wire types (Q#BP15, Q#BP15a, Q#BP16).
//!
//! A panel frame is the daemon's painted projection of one side window.
//! It shares [`crate::wire_grid`]'s cell rules with
//! [`crate::terminal::TerminalFrame`] but not its per-axis PTY caps: a
//! 4K surface at a small font is legitimately wider than 512 columns,
//! and the area bound is what keeps the encoding inside the transport
//! budget.
//!
//! Presence is explicit. [`PanelFramePayload::Absent`] is authoritative
//! and must be sent on close *and* on hide, because the receiver
//! retains its last valid frame: silence would leave a stale band on
//! screen indefinitely.

use crate::cell::{Cell, CellCoord, CellSize};
use crate::ids::BufferId;
use crate::wire_grid::{
    MAX_WIRE_GRID_GLYPH_BYTES, WireGridError, WireGridLimits, validate_wire_grid,
};

/// Lowest negotiated protocol version that carries the bottom-panel wire
/// family (Q#BP9): [`crate::InstanceMessage::PanelFrame`] daemon→frontend,
/// and `FrontendEvent::{FrontendCellGeometry, PanelResizeRows,
/// PanelPointer}` frontend→daemon.
///
/// One constant rather than a literal at each gate, because the panel bump
/// gates in **both** directions: the daemon's send filter, the producer's
/// peer flag, the three inbound event gates, and — since Stage 2B-3 — the
/// GPU frontend's own declaration and paint gates must move together, or
/// one side starts trusting a wire the other never negotiated. It lives in
/// the protocol crate precisely so the frontend aliases this definition
/// rather than restating the number.
///
/// Distinct from [`crate::ADVERTISED_PROTOCOL_VERSION`], which stays at the
/// compatibility baseline permanently: a session reaches this version by
/// the frontend's `AttachRequest` counter-offer, never by the daemon
/// advertising it.
pub const PANEL_MIN_VERSION: u32 = 21;

/// Shared visible-cell ceiling for a panel grid.
///
/// Identical to the terminal bound: it is the transport-safety limit,
/// not a PTY policy, so both messages answer to it.
pub const MAX_PANEL_VISIBLE_CELLS: usize = crate::wire_grid::MAX_WIRE_GRID_VISIBLE_CELLS;

/// Bounds a panel frame enforces on its cell grid.
///
/// The per-axis ceilings are the area bound itself rather than 512: any
/// axis larger than the area bound is already rejected by the area
/// check, so this expresses "no independent per-axis policy" without
/// leaving the multiplication unchecked.
const PANEL_GRID_LIMITS: WireGridLimits = WireGridLimits {
    max_rows: MAX_PANEL_VISIBLE_CELLS as u32,
    max_cols: MAX_PANEL_VISIBLE_CELLS as u32,
    max_visible_cells: MAX_PANEL_VISIBLE_CELLS,
    max_glyph_bytes: MAX_WIRE_GRID_GLYPH_BYTES,
};

/// The daemon's painted projection of one side window.
///
/// `panel_epoch` is opaque and monotonic per frontend: stable across
/// ordinary frames of one continuously present window/buffer, and
/// changed on buffer replacement, new side-window creation, and every
/// `Absent` → `Present` transition. That is what stops a stale
/// `PanelPointer` from addressing a reopened panel as if it were the
/// old one (Q#BP16).
///
/// `geometry_epoch` answers a *frontend* declaration and moves whenever
/// the frontend declares new effective cell geometry — including a font
/// or scale change that leaves [`CellSize`] identical, which is exactly
/// the case daemon-side value dedup cannot see (Q#BP2S1).
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PanelFrame {
    /// Buffer this frame projects.
    pub buffer_id: BufferId,
    /// Presentation identity, monotonic per frontend.
    pub panel_epoch: u64,
    /// The frontend geometry declaration this frame answers.
    pub geometry_epoch: u64,
    /// Panel grid dimensions in cells.
    pub size: CellSize,
    /// Row-major cells; exactly `size.area()` entries.
    pub cells: Vec<Cell>,
    /// Panel caret, or `None` when the panel shows no cursor.
    ///
    /// `paint_frame` returns the cursor separately from the cells, so a
    /// frame carrying cells alone would lose the caret.
    pub cursor: Option<CellCoord>,
    /// Whether the panel owns focus.
    ///
    /// Presentation and focus-chrome routing only (Q#BP14b) — the
    /// *keys* decision is `DispatchIdle` (Q#BP14a).
    pub focused: bool,
}

/// Explicit panel presence.
///
/// `Absent` is authoritative rather than implied by silence, and is
/// duplicate-suppressed like any other payload.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PanelFramePayload {
    /// A panel is visible and this is its current frame.
    Present(PanelFrame),
    /// No panel is visible; clear any retained frame.
    Absent,
}

/// Why a [`PanelFrame`] is not structurally valid.
///
/// Validation is atomic: the frame is rejected whole and the receiver
/// retains its previous valid frame.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PanelFrameError {
    /// Rows or columns are zero or above the area-derived bounds.
    #[error("panel size {rows}x{cols} is outside 1..={max_rows}x1..={max_cols}")]
    Size {
        /// Declared rows.
        rows: u32,
        /// Declared columns.
        cols: u32,
        /// Row bound in force.
        max_rows: u32,
        /// Column bound in force.
        max_cols: u32,
    },
    /// The checked area exceeds the shared visible-cell bound.
    #[error("panel area {area} exceeds the visible-cell bound {max}")]
    Area {
        /// Checked `rows * cols`.
        area: usize,
        /// Shared visible-cell bound.
        max: usize,
    },
    /// `cells.len()` disagrees with the declared area.
    #[error("panel frame carries {actual} cells for a {expected}-cell area")]
    CellCount {
        /// Declared area.
        expected: usize,
        /// Supplied cell count.
        actual: usize,
    },
    /// The cursor lies outside the declared grid.
    #[error("panel cursor ({row},{col}) is outside the {rows}x{cols} grid")]
    Cursor {
        /// Cursor row.
        row: u32,
        /// Cursor column.
        col: u32,
        /// Declared rows.
        rows: u32,
        /// Declared columns.
        cols: u32,
    },
    /// A cell's glyph is not a legal wire glyph.
    #[error("panel cell {index} has an invalid glyph: {reason}")]
    Glyph {
        /// Row-major cell index.
        index: usize,
        /// Why the glyph failed.
        reason: &'static str,
    },
    /// A cell carries a frontend attachment, which panels never use.
    #[error("panel cell {index} carries an attachment")]
    Attachment {
        /// Row-major cell index.
        index: usize,
    },
    /// Aggregate glyph bytes exceed the shared budget.
    #[error("panel frame glyph bytes exceed the aggregate bound {max}")]
    GlyphBudget {
        /// Shared aggregate bound.
        max: usize,
    },
    /// An epoch is zero, which is reserved for "never declared".
    #[error("panel {field} epoch is zero, which is reserved for 'never declared'")]
    ZeroEpoch {
        /// Which epoch was zero.
        field: &'static str,
    },
}

impl PanelFrame {
    /// Check every structural rule a panel frame must satisfy.
    ///
    /// Pure: a rejected frame mutates nothing, so callers get atomic
    /// rejection for free.
    pub fn validate(&self) -> Result<(), PanelFrameError> {
        if self.panel_epoch == 0 {
            return Err(PanelFrameError::ZeroEpoch { field: "panel" });
        }
        if self.geometry_epoch == 0 {
            return Err(PanelFrameError::ZeroEpoch { field: "geometry" });
        }
        validate_wire_grid(self.size, &self.cells, self.cursor, PANEL_GRID_LIMITS)
            .map_err(panel_grid_error)
    }
}

/// Map a shared wire-grid failure onto this message's error type.
fn panel_grid_error(error: WireGridError) -> PanelFrameError {
    match error {
        WireGridError::Size {
            rows,
            cols,
            max_rows,
            max_cols,
        } => PanelFrameError::Size {
            rows,
            cols,
            max_rows,
            max_cols,
        },
        WireGridError::Area { area, max } => PanelFrameError::Area { area, max },
        WireGridError::CellCount { expected, actual } => {
            PanelFrameError::CellCount { expected, actual }
        }
        WireGridError::Cursor {
            row,
            col,
            rows,
            cols,
        } => PanelFrameError::Cursor {
            row,
            col,
            rows,
            cols,
        },
        WireGridError::Glyph { index, reason } => PanelFrameError::Glyph { index, reason },
        WireGridError::Attachment { index } => PanelFrameError::Attachment { index },
        WireGridError::GlyphBudget { max } => PanelFrameError::GlyphBudget { max },
    }
}
