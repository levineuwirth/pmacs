//! Per-frontend terminal viewport and selection identities.
//!
//! These types identify projections over one [`super::screen::TerminalScreen`].
//! They never own or mirror terminal cells.

use crate::buffer::BufferId;
use crate::protocol::FrontendId;
use crate::window::WindowId;

/// One frontend/window projection of a terminal session.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TerminalViewKey {
    /// Authenticated frontend that owns this view state.
    pub frontend_id: FrontendId,
    /// Stable editor window showing the terminal.
    pub window_id: WindowId,
    /// Identity buffer whose session is projected.
    pub buffer_id: BufferId,
}

impl TerminalViewKey {
    /// Construct an exact terminal view identity.
    #[must_use]
    pub const fn new(frontend_id: FrontendId, window_id: WindowId, buffer_id: BufferId) -> Self {
        Self {
            frontend_id,
            window_id,
            buffer_id,
        }
    }
}

/// Leading display-cell offset within one retained logical line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogicalCellAnchor {
    /// Stable logical line identity preserved by main-screen reflow.
    pub logical_line_id: u64,
    /// Leading display-cell offset within that logical line.
    pub cell_offset: u32,
}

/// Inclusive terminal selection endpoints.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalSelection {
    /// Fixed endpoint where the drag began.
    pub anchor: LogicalCellAnchor,
    /// Moving endpoint under the pointer.
    pub head: LogicalCellAnchor,
}

/// Mutable state for one [`TerminalViewKey`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerminalViewState {
    /// First visible retained logical cell, or live-tail following when absent.
    pub top: Option<LogicalCellAnchor>,
    /// Inclusive logical-cell selection.
    pub selection: Option<TerminalSelection>,
    /// Current editor-owned drag endpoint; cleared on release.
    pub drag: Option<LogicalCellAnchor>,
}

/// The one authenticated frontend/window allowed to control a session's PTY.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalController {
    /// Authenticated controlling frontend.
    pub frontend_id: FrontendId,
    /// Active terminal window on that frontend.
    pub window_id: WindowId,
}

impl TerminalController {
    /// Construct a controller from an exact view identity.
    #[must_use]
    pub const fn from_view(key: TerminalViewKey) -> Self {
        Self {
            frontend_id: key.frontend_id,
            window_id: key.window_id,
        }
    }

    /// Whether this controller names `key`'s frontend and window.
    #[must_use]
    pub fn matches(self, key: TerminalViewKey) -> bool {
        self.frontend_id == key.frontend_id && self.window_id == key.window_id
    }
}
