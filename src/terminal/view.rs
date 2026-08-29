//! Per-frontend terminal viewport, selection, copy, and bell projection.
//!
//! A view stores only logical anchors into one [`TerminalScreen`]. Cells,
//! modes, history, and process state remain session-owned.

use crate::buffer::BufferId;
use crate::cell::{Cell, CellCoord, CellSize, Glyph, Style};
use crate::protocol::FrontendId;
use crate::terminal::screen::{BorrowedScreenProjection, TerminalModes, TerminalRow};
use crate::terminal::session::{TerminalManager, TerminalSnapshot};
use crate::terminal::{
    MAX_TERMINAL_COLS, MAX_TERMINAL_ROWS, MAX_TERMINAL_VISIBLE_CELLS, TerminalProcessState,
    TerminalSelectionSpan,
};
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

/// Fresh context metadata for Lua/statusline consumers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalViewStatus {
    /// Geometric live-tail visibility.
    pub at_bottom: bool,
    /// Physical retained rows between this viewport and the live tail.
    pub scroll_offset: u32,
    /// Whether the view owns a nonempty selection.
    pub selection: bool,
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
    pub(super) alternate_active: Option<bool>,
    pub(super) last_bell_count: u64,
    pub(super) viewport_size: Option<CellSize>,
    pub(super) selection_froze_top: bool,
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

impl TerminalManager {
    /// Project one exact view into an owned viewport-sized snapshot.
    ///
    /// Zero or out-of-range viewports and unknown sessions return `None`
    /// without registering view state.
    #[must_use]
    pub fn snapshot_for_view(
        &mut self,
        key: TerminalViewKey,
        viewport_size: CellSize,
    ) -> Option<TerminalSnapshot> {
        if !valid_viewport(viewport_size) {
            return None;
        }
        let session = self.sessions.get(&key.buffer_id)?;
        let projection = session.screen.projection_ref();
        let pid = session.pid;
        let process = session.process.clone();
        let bell_count = session.screen.bell_count();

        let state = self.views.entry(key).or_insert_with(|| TerminalViewState {
            alternate_active: Some(projection.alternate_active),
            last_bell_count: bell_count,
            ..TerminalViewState::default()
        });
        declare_view_size(state, projection, viewport_size);
        Some(project_snapshot(
            key.buffer_id,
            viewport_size,
            projection,
            state,
            pid,
            process,
        ))
    }

    /// Scroll one view by physical retained rows.
    ///
    /// Positive values move toward older rows and negative values move toward
    /// the live tail. Returns whether the top anchor changed.
    pub fn scroll_view(
        &mut self,
        key: TerminalViewKey,
        viewport_size: CellSize,
        lines: i32,
    ) -> bool {
        if lines == 0 || !valid_viewport(viewport_size) {
            return false;
        }
        let Some(session) = self.sessions.get(&key.buffer_id) else {
            return false;
        };
        let projection = session.screen.projection_ref();
        let bell_count = session.screen.bell_count();
        let state = self.views.entry(key).or_insert_with(|| TerminalViewState {
            alternate_active: Some(projection.alternate_active),
            last_bell_count: bell_count,
            ..TerminalViewState::default()
        });
        normalize_state(state, projection);
        state.viewport_size = Some(viewport_size);
        let rows = retained_rows(projection);
        if rows.is_empty() {
            return false;
        }
        let geometry = view_geometry(&rows, state, viewport_size.rows);
        let tail_start = rows.len().saturating_sub(viewport_size.rows as usize);
        let magnitude = lines.unsigned_abs() as usize;
        let next = if lines > 0 {
            geometry.start.saturating_sub(magnitude)
        } else {
            geometry.start.saturating_add(magnitude).min(tail_start)
        };
        if next == geometry.start {
            return false;
        }
        state.top = if next == tail_start && state.selection.is_none() {
            None
        } else {
            Some(row_lead(rows.get(next).expect("bounded retained row")))
        };
        true
    }

    /// Scroll by `lines` using the last nonzero rendered viewport.
    pub fn scroll_lines(&mut self, key: TerminalViewKey, lines: i32) -> bool {
        if lines == 0 {
            return false;
        }
        let Some(size) = self.views.get(&key).and_then(|state| state.viewport_size) else {
            return false;
        };
        self.scroll_view(key, size, lines)
    }

    /// Scroll by one last-rendered page in `direction` (`1` older, `-1` tail).
    pub fn scroll_page(&mut self, key: TerminalViewKey, direction: i32) -> bool {
        if direction == 0 {
            return false;
        }
        let Some(size) = self.views.get(&key).and_then(|state| state.viewport_size) else {
            return false;
        };
        let rows = i32::try_from(size.rows).unwrap_or(i32::MAX);
        self.scroll_view(key, size, rows.saturating_mul(direction.signum()))
    }

    /// Register or refresh one view at `viewport_size` and return its geometry
    /// without allocating an owned cell snapshot.
    #[must_use]
    pub(crate) fn view_status_for_size(
        &mut self,
        key: TerminalViewKey,
        viewport_size: CellSize,
    ) -> Option<TerminalViewStatus> {
        if !valid_viewport(viewport_size) {
            return None;
        }
        let session = self.sessions.get(&key.buffer_id)?;
        let projection = session.screen.projection_ref();
        let bell_count = session.screen.bell_count();
        let state = self.views.entry(key).or_insert_with(|| TerminalViewState {
            alternate_active: Some(projection.alternate_active),
            last_bell_count: bell_count,
            ..TerminalViewState::default()
        });
        declare_view_size(state, projection, viewport_size);
        let rows = retained_rows(projection);
        let geometry = view_geometry(&rows, state, viewport_size.rows);
        Some(TerminalViewStatus {
            at_bottom: geometry.scroll_offset == 0,
            scroll_offset: geometry.scroll_offset,
            selection: state.selection.is_some(),
        })
    }

    /// Return the publication-consistent child grid size for one view.
    #[must_use]
    pub(crate) fn screen_size_for_view(&self, key: TerminalViewKey) -> Option<CellSize> {
        self.screen_size(key.buffer_id)
    }

    /// Apply one parsed event to a session's screen, for tests.
    ///
    /// Terminal output normally arrives on the PTY reader thread, which
    /// no daemon-level test can drive deterministically. §5b's terminal
    /// rows must nevertheless be witnessed **across the seam** — the
    /// screen counter and the daemon's key are separately provable, and
    /// a `view_mapping_identity` returning a constant would leave both
    /// green — so this exists to join them.
    ///
    /// `#[doc(hidden)]` rather than `#[cfg(test)]`, because the rows
    /// that need it are integration tests and those link the library
    /// without `cfg(test)`.
    #[doc(hidden)]
    pub fn apply_event_for_test(
        &mut self,
        buffer_id: BufferId,
        event: crate::ansi::AnsiEvent,
    ) -> bool {
        match self.sessions.get_mut(&buffer_id) {
            Some(session) => {
                session.screen.apply_event(event);
                session.screen.publish_for_test();
                true
            }
            None => false,
        }
    }

    /// §5b — the terminal's **mapping revision** plus its per-view
    /// scroll anchor: together, the identity of what a coordinate in
    /// this view denotes.
    ///
    /// The anchor is part of it because the same coordinate names a
    /// different retained row once the view scrolls, even with the
    /// child's screen untouched.
    #[must_use]
    pub fn view_mapping_identity(
        &self,
        key: TerminalViewKey,
    ) -> Option<(u64, Option<LogicalCellAnchor>)> {
        let session = self.sessions.get(&key.buffer_id)?;
        // `top` IS the anchor: `None` means following the live tail,
        // which is itself a distinct state from any pinned row.
        let anchor = self.views.get(&key).and_then(|view| view.top);
        // The PUBLISHED revision, not the live one. While synchronized
        // output is held, `projection_ref` keeps returning the last
        // published cells while the screen races ahead — reading
        // `screen.mapping_revision()` there would stamp displayed cells
        // with authority they were never painted under, and the frontend
        // would echo a generation matching nothing it can see.
        let published = session.screen.projection_ref().mapping_revision;
        Some((published, anchor))
    }

    /// The shared screen's current size, read from the borrowed
    /// projection.
    ///
    /// Deliberately not `snapshot(..).size`: that clones the whole
    /// visible cell grid, and geometry comparison runs on every
    /// dispatcher tick for every frontend with a declared terminal.
    #[must_use]
    pub fn screen_size(&self, buffer_id: BufferId) -> Option<CellSize> {
        self.sessions
            .get(&buffer_id)
            .map(|session| session.screen.projection_ref().size)
    }

    /// Record an exact view's declared viewport size without projecting.
    ///
    /// Vterm Stage 3: a semantic frontend declares terminal geometry
    /// through its own message rather than through a layout pass, and a
    /// PASSIVE view must still record its size — that is what lets it
    /// receive its own clipped/padded projection instead of the
    /// controller's. Recording a size is deliberately not claiming
    /// control; the caller decides whether to resize the PTY.
    ///
    /// Returns `false` for an unknown session or an out-of-range size,
    /// leaving prior geometry untouched.
    pub fn record_view_size(&mut self, key: TerminalViewKey, viewport_size: CellSize) -> bool {
        if !valid_viewport(viewport_size) {
            return false;
        }
        let Some(session) = self.sessions.get(&key.buffer_id) else {
            return false;
        };
        let projection = session.screen.projection_ref();
        let bell_count = session.screen.bell_count();
        let state = self.views.entry(key).or_insert_with(|| TerminalViewState {
            alternate_active: Some(projection.alternate_active),
            last_bell_count: bell_count,
            ..TerminalViewState::default()
        });
        declare_view_size(state, projection, viewport_size);
        true
    }

    /// The viewport size an exact view last declared or rendered at.
    #[must_use]
    pub fn declared_view_size(&self, key: TerminalViewKey) -> Option<CellSize> {
        self.views.get(&key).and_then(|state| state.viewport_size)
    }

    /// Return fresh geometric status for one registered view.
    #[must_use]
    pub fn view_status(&mut self, key: TerminalViewKey) -> Option<TerminalViewStatus> {
        let size = self.views.get(&key)?.viewport_size?;
        self.view_status_for_size(key, size)
    }

    /// Clear selection and resume live-tail following for one view.
    pub fn scroll_to_bottom(&mut self, key: TerminalViewKey) -> bool {
        let Some(state) = self.views.get_mut(&key) else {
            return false;
        };
        let changed = state.top.is_some() || state.selection.is_some() || state.drag.is_some();
        state.top = None;
        state.selection = None;
        state.drag = None;
        state.selection_froze_top = false;
        changed
    }

    /// Serialize one view's current selection from retained terminal rows.
    #[must_use]
    pub fn copy_selection(&mut self, key: TerminalViewKey) -> Option<Vec<u8>> {
        let session = self.sessions.get(&key.buffer_id)?;
        let projection = session.screen.projection_ref();
        let state = self.views.get_mut(&key)?;
        normalize_state(state, projection);
        let selection = state.selection?;
        let rows = retained_rows(projection);
        copy_selection_bytes(&rows, selection)
    }

    /// Serialize a session's ENTIRE retained range — scrollback plus the
    /// visible screen — through the same path [`copy_selection`] uses.
    ///
    /// Q#TC7. This deliberately builds a whole-range *selection* and hands
    /// it to the existing serializer rather than walking the rows itself.
    /// Soft-wrap joining, wide-glyph continuation, cluster bytes, and
    /// per-row trailing-blank trimming are Vterm Stage 2 criterion 21's
    /// pinned behavior; a second walk would re-derive all four and the two
    /// would drift. That inheritance is what acceptance 13 asserts, by
    /// comparing this against a full-range `copy_selection` rather than
    /// against a literal.
    ///
    /// Returns `None` for a non-terminal buffer and for a session whose
    /// retained rows are all empty — there is no cell to anchor to.
    /// Unlike `copy_selection` this needs no registered view, so copy mode
    /// does not depend on the terminal being currently displayed.
    #[must_use]
    pub fn copy_retained(&self, buffer_id: BufferId) -> Option<Vec<u8>> {
        let session = self.sessions.get(&buffer_id)?;
        let projection = session.screen.projection_ref();
        retained_bytes(&retained_rows(projection))
    }

    /// Start an editor-owned primary selection at a viewport coordinate.
    pub fn begin_selection(
        &mut self,
        key: TerminalViewKey,
        viewport_size: CellSize,
        coord: CellCoord,
    ) -> bool {
        let Some(session) = self.sessions.get(&key.buffer_id) else {
            return false;
        };
        let projection = session.screen.projection_ref();
        let bell_count = session.screen.bell_count();
        let state = self.views.entry(key).or_insert_with(|| TerminalViewState {
            alternate_active: Some(projection.alternate_active),
            last_bell_count: bell_count,
            ..TerminalViewState::default()
        });
        normalize_state(state, projection);
        let rows = retained_rows(projection);
        let geometry = view_geometry(&rows, state, viewport_size.rows);
        let Some(anchor) = anchor_at(&rows, &geometry, viewport_size, coord) else {
            state.selection = None;
            state.drag = None;
            return false;
        };
        state.selection_froze_top = state.top.is_none();
        if state.top.is_none() {
            state.top = rows.get(geometry.start).map(row_lead);
        }
        state.selection = Some(TerminalSelection {
            anchor,
            head: anchor,
        });
        state.drag = Some(anchor);
        true
    }

    /// Move an active editor-owned terminal selection.
    pub fn update_selection(
        &mut self,
        key: TerminalViewKey,
        viewport_size: CellSize,
        coord: CellCoord,
    ) -> bool {
        let Some(session) = self.sessions.get(&key.buffer_id) else {
            return false;
        };
        let projection = session.screen.projection_ref();
        let Some(state) = self.views.get_mut(&key) else {
            return false;
        };
        normalize_state(state, projection);
        if state.drag.is_none() {
            return false;
        }
        let rows = retained_rows(projection);
        let geometry = view_geometry(&rows, state, viewport_size.rows);
        let Some(head) = anchor_at(&rows, &geometry, viewport_size, coord) else {
            return false;
        };
        let changed = state
            .selection
            .is_some_and(|selection| selection.head != head);
        if let Some(selection) = state.selection.as_mut() {
            selection.head = head;
        }
        state.drag = Some(head);
        changed
    }

    /// Finish an editor-owned terminal selection.
    pub fn finish_selection(
        &mut self,
        key: TerminalViewKey,
        viewport_size: CellSize,
        coord: CellCoord,
    ) -> bool {
        let moved = self.update_selection(key, viewport_size, coord);
        let Some(state) = self.views.get_mut(&key) else {
            return false;
        };
        let was_dragging = state.drag.take().is_some();
        if state
            .selection
            .is_some_and(|selection| selection.anchor == selection.head)
        {
            state.selection = None;
            if state.selection_froze_top {
                state.top = None;
            }
            state.selection_froze_top = false;
        }
        moved || was_dragging
    }

    /// Whether a view is mid-drag, for parent 48 Q#BP-R4's completion
    /// witnesses.
    ///
    /// A local terminal gesture is "finished" exactly when
    /// `finish_selection` takes `drag`, so this is the observable that
    /// separates a delivered completion from a latch that merely
    /// emptied — which is the distinction the framing requires those
    /// rows to assert.
    #[doc(hidden)]
    #[must_use]
    pub fn view_is_dragging_for_test(&self, key: TerminalViewKey) -> bool {
        self.views
            .get(&key)
            .is_some_and(|state| state.drag.is_some())
    }

    /// Clear one view's terminal selection without changing its scroll anchor.
    pub fn clear_selection(&mut self, key: TerminalViewKey) -> bool {
        let Some(state) = self.views.get_mut(&key) else {
            return false;
        };
        state.selection.take().is_some() || state.drag.take().is_some()
    }

    /// Turn SGR mouse reporting on or off for one session (G5k).
    #[doc(hidden)]
    pub fn set_mouse_reporting_for_test(&mut self, buffer_id: BufferId, enabled: bool) {
        if let Some(session) = self.sessions.get_mut(&buffer_id) {
            session.screen.set_mouse_reporting_for_test(enabled);
        }
    }

    /// Current child input modes for one session.
    #[must_use]
    pub fn modes_for_view(&self, key: TerminalViewKey) -> Option<TerminalModes> {
        self.sessions
            .get(&key.buffer_id)
            .map(|session| session.screen.modes())
    }

    /// Exact controlled view for one frontend, if it still exists.
    #[must_use]
    pub fn controller_view_for_frontend(&self, frontend_id: FrontendId) -> Option<TerminalViewKey> {
        self.controllers.iter().find_map(|(buffer_id, controller)| {
            (controller.frontend_id == frontend_id)
                .then(|| TerminalViewKey::new(frontend_id, controller.window_id, *buffer_id))
        })
    }

    /// Observe BEL counters for every live view on one frontend.
    ///
    /// Returns `true` only for a new bell in `active`; all other counters are
    /// advanced so historical bells cannot replay after a later activation.
    pub fn take_bell_for_frontend(
        &mut self,
        frontend_id: FrontendId,
        active: Option<TerminalViewKey>,
    ) -> bool {
        let mut ring = false;
        for (key, state) in &mut self.views {
            if key.frontend_id != frontend_id {
                continue;
            }
            let Some(session) = self.sessions.get(&key.buffer_id) else {
                continue;
            };
            let current = session.screen.bell_count();
            if Some(*key) == active && current > state.last_bell_count {
                ring = true;
            }
            state.last_bell_count = current;
        }
        ring
    }
}

#[derive(Clone, Copy)]
struct ResolvedCell {
    row: usize,
    col: usize,
}

struct ViewGeometry {
    start: usize,
    top_padding: usize,
    scroll_offset: u32,
}

#[derive(Clone, Copy)]
struct RetainedRows<'a> {
    projection: BorrowedScreenProjection<'a>,
}

impl<'a> RetainedRows<'a> {
    fn len(self) -> usize {
        self.projection.history_len() + self.projection.visible_rows.len()
    }

    fn is_empty(self) -> bool {
        self.len() == 0
    }

    fn get(self, index: usize) -> Option<&'a TerminalRow> {
        let head_len = self.projection.history_head.len();
        if index < head_len {
            return self.projection.history_head.get(index);
        }
        let index = index - head_len;
        let tail_len = self.projection.history_tail.len();
        if index < tail_len {
            return self.projection.history_tail.get(index);
        }
        self.projection.visible_rows.get(index - tail_len)
    }

    fn first(self) -> Option<&'a TerminalRow> {
        self.get(0)
    }

    fn iter(self) -> impl Iterator<Item = &'a TerminalRow> {
        self.projection
            .history_head
            .iter()
            .chain(self.projection.history_tail)
            .chain(self.projection.visible_rows)
    }
}

fn valid_viewport(size: CellSize) -> bool {
    size.rows > 0
        && size.cols > 0
        && size.rows <= u32::from(MAX_TERMINAL_ROWS)
        && size.cols <= u32::from(MAX_TERMINAL_COLS)
        && size.area() as usize <= MAX_TERMINAL_VISIBLE_CELLS
}

fn retained_rows(projection: BorrowedScreenProjection<'_>) -> RetainedRows<'_> {
    RetainedRows { projection }
}

/// Serialize every retained cell, through the selection-copy serializer.
///
/// Split out from [`TerminalManager::copy_retained`] so the fidelity
/// claims — soft-wrap joining, per-row trailing-blank trimming, wide-glyph
/// continuation, cluster bytes — are testable against the same projection
/// fixtures that pin `copy_selection_bytes` itself. Those four are exactly
/// what a second, independently written walk would get wrong.
fn retained_bytes(rows: &RetainedRows<'_>) -> Option<Vec<u8>> {
    copy_selection_bytes(rows, full_retained_selection(rows)?)
}

/// The selection spanning every retained cell.
///
/// Rows with no cells are skipped at both ends rather than clamped: an
/// anchor into a zero-width row cannot resolve (`resolve_anchor` requires
/// `cell_offset` to fall inside `cell_offset .. cell_offset + len`), so
/// including one would make the whole range unresolvable and silently
/// yield nothing. Interior empty rows are untouched, because trailing- and
/// interior-blank handling belongs to the serializer.
fn full_retained_selection(rows: &RetainedRows<'_>) -> Option<TerminalSelection> {
    let mut occupied = rows.iter().filter(|row| !row.cells.is_empty());
    let first = occupied.next()?;
    // `RetainedRows::iter` is a chain of slice iterators exposed as
    // `impl Iterator`, so it is not double-ended; scan forward.
    let last = occupied.last().unwrap_or(first);
    Some(TerminalSelection {
        anchor: row_lead(first),
        head: LogicalCellAnchor {
            logical_line_id: last.logical_line_id,
            cell_offset: last.cell_offset.saturating_add(last.cells.len() as u32 - 1),
        },
    })
}

fn row_lead(row: &TerminalRow) -> LogicalCellAnchor {
    LogicalCellAnchor {
        logical_line_id: row.logical_line_id,
        cell_offset: row.cell_offset,
    }
}

fn resolve_anchor(rows: &RetainedRows<'_>, anchor: LogicalCellAnchor) -> Option<ResolvedCell> {
    rows.iter().enumerate().find_map(|(row_index, row)| {
        if row.logical_line_id != anchor.logical_line_id {
            return None;
        }
        let start = row.cell_offset;
        let end = start.saturating_add(row.cells.len() as u32);
        if anchor.cell_offset < start || anchor.cell_offset >= end {
            return None;
        }
        let col = (anchor.cell_offset - start) as usize;
        Some(ResolvedCell {
            row: row_index,
            col: canonical_col(row, col),
        })
    })
}

fn canonical_col(row: &TerminalRow, mut col: usize) -> usize {
    col = col.min(row.cells.len().saturating_sub(1));
    while col > 0 && matches!(row.cells[col].glyph, Glyph::Continuation) {
        col -= 1;
    }
    col
}

fn anchor_for(rows: &RetainedRows<'_>, resolved: ResolvedCell) -> LogicalCellAnchor {
    let row = rows.get(resolved.row).expect("resolved retained row");
    LogicalCellAnchor {
        logical_line_id: row.logical_line_id,
        cell_offset: row.cell_offset.saturating_add(resolved.col as u32),
    }
}

fn clamp_or_clear(rows: &RetainedRows<'_>, anchor: LogicalCellAnchor) -> Option<LogicalCellAnchor> {
    if let Some(resolved) = resolve_anchor(rows, anchor) {
        return Some(anchor_for(rows, resolved));
    }
    let first = rows.first()?;
    (anchor.logical_line_id < first.logical_line_id
        || (anchor.logical_line_id == first.logical_line_id
            && anchor.cell_offset < first.cell_offset))
        .then(|| row_lead(first))
}

/// The shared viewport-size declaration path (bottom-panel arc, Q#BP7).
///
/// Normalize, then re-arm live-tail following when the newly declared
/// viewport reaches the tail, then record the size. Every path that
/// *declares* a size routes through here so grid and semantic
/// declarations cannot disagree; `scroll_view` and `begin_selection`
/// deliberately do not, because they write `top` themselves.
fn declare_view_size(
    state: &mut TerminalViewState,
    projection: BorrowedScreenProjection<'_>,
    viewport_size: CellSize,
) {
    normalize_state(state, projection);
    rearm_follow_on_growth(state, projection, viewport_size.rows);
    state.viewport_size = Some(viewport_size);
}

/// Q#BP7 item 1: **growth reaching the live tail re-arms follow.**
///
/// A height change is a viewport change, never a scroll change — `top`
/// is preserved verbatim — but once a taller viewport covers the tail,
/// staying anchored would leave the view frozen just short of the live
/// output while `at_bottom` reported `true`: `at_bottom` is the
/// instantaneous geometric readout `scroll_offset == 0`, so it cannot
/// distinguish "following" from "anchored, and currently tall enough to
/// reach". The next rows the child prints would then push the anchored
/// view back into history with nothing to explain it.
///
/// **Only when no selection is active** (R1-8): a historical selection
/// froze this anchor on purpose, and growth must not yank the user's
/// region out from under them. `scroll_view` already handles the
/// scroll-driven arm (`next == tail_start`), so during ordinary
/// scrolling `scroll_offset == 0` implies follow is already armed —
/// which makes this rule fire on exactly the growth (and shrink-back)
/// case it names, and be idempotent everywhere else.
fn rearm_follow_on_growth(
    state: &mut TerminalViewState,
    projection: BorrowedScreenProjection<'_>,
    viewport_rows: u32,
) {
    if state.top.is_none() || state.selection.is_some() || viewport_rows == 0 {
        return;
    }
    let rows = retained_rows(projection);
    if view_geometry(&rows, state, viewport_rows).scroll_offset == 0 {
        state.top = None;
        state.selection_froze_top = false;
    }
}

fn normalize_state(state: &mut TerminalViewState, projection: BorrowedScreenProjection<'_>) {
    if state
        .alternate_active
        .is_some_and(|active| active != projection.alternate_active)
    {
        state.top = None;
        state.selection = None;
        state.drag = None;
        state.selection_froze_top = false;
    }
    state.alternate_active = Some(projection.alternate_active);
    let rows = retained_rows(projection);
    state.top = state.top.and_then(|anchor| clamp_or_clear(&rows, anchor));
    state.selection = state.selection.and_then(|selection| {
        let anchor = clamp_or_clear(&rows, selection.anchor)?;
        let head = clamp_or_clear(&rows, selection.head)?;
        let collapsed_by_clamp =
            anchor == head && (anchor != selection.anchor || head != selection.head);
        (!collapsed_by_clamp).then_some(TerminalSelection { anchor, head })
    });
    state.drag = state.drag.and_then(|anchor| clamp_or_clear(&rows, anchor));
    if state.selection.is_none() {
        state.drag = None;
        state.selection_froze_top = false;
    }
}

fn view_geometry(
    rows: &RetainedRows<'_>,
    state: &TerminalViewState,
    viewport_rows: u32,
) -> ViewGeometry {
    let viewport_rows = viewport_rows as usize;
    let follow = state.top.is_none() && state.selection.is_none();
    let tail_start = rows.len().saturating_sub(viewport_rows);
    let start = if follow {
        tail_start
    } else {
        state
            .top
            .and_then(|anchor| resolve_anchor(rows, anchor))
            .map_or(tail_start, |resolved| resolved.row)
    };
    let top_padding = if follow && rows.len() < viewport_rows {
        viewport_rows - rows.len()
    } else {
        0
    };
    let rows_after_view = rows
        .len()
        .saturating_sub(start.saturating_add(viewport_rows));
    ViewGeometry {
        start,
        top_padding,
        scroll_offset: u32::try_from(rows_after_view).unwrap_or(u32::MAX),
    }
}

fn viewport_row(
    geometry: &ViewGeometry,
    viewport_rows: usize,
    retained_row: usize,
) -> Option<usize> {
    if retained_row < geometry.start {
        return None;
    }
    let row = geometry
        .top_padding
        .saturating_add(retained_row - geometry.start);
    (row < viewport_rows).then_some(row)
}

fn anchor_at(
    rows: &RetainedRows<'_>,
    geometry: &ViewGeometry,
    viewport_size: CellSize,
    coord: CellCoord,
) -> Option<LogicalCellAnchor> {
    if coord.row >= viewport_size.rows || coord.col >= viewport_size.cols {
        return None;
    }
    let viewport_row = coord.row as usize;
    if viewport_row < geometry.top_padding {
        return None;
    }
    let retained_row = geometry
        .start
        .saturating_add(viewport_row - geometry.top_padding);
    let row = rows.get(retained_row)?;
    if coord.col as usize >= row.cells.len() {
        return None;
    }
    let col = canonical_col(row, coord.col as usize);
    Some(LogicalCellAnchor {
        logical_line_id: row.logical_line_id,
        cell_offset: row.cell_offset.saturating_add(col as u32),
    })
}

fn normalized_selection(
    rows: &RetainedRows<'_>,
    selection: TerminalSelection,
) -> Option<(ResolvedCell, ResolvedCell)> {
    let mut start = resolve_anchor(rows, selection.anchor)?;
    let mut end = resolve_anchor(rows, selection.head)?;
    if (start.row, start.col) > (end.row, end.col) {
        std::mem::swap(&mut start, &mut end);
    }
    Some((start, end))
}

fn glyph_width(row: &TerminalRow, col: usize) -> usize {
    if col + 1 < row.cells.len() && matches!(row.cells[col + 1].glyph, Glyph::Continuation) {
        2
    } else {
        1
    }
}

fn project_snapshot(
    buffer_id: BufferId,
    viewport_size: CellSize,
    projection: BorrowedScreenProjection<'_>,
    state: &TerminalViewState,
    pid: u32,
    process: TerminalProcessState,
) -> TerminalSnapshot {
    let rows = retained_rows(projection);
    let geometry = view_geometry(&rows, state, viewport_size.rows);
    let mut cells = vec![Cell::default(); viewport_size.area() as usize];
    for (retained_row, row) in rows.iter().enumerate().skip(geometry.start) {
        let Some(target_row) = viewport_row(&geometry, viewport_size.rows as usize, retained_row)
        else {
            continue;
        };
        let copy_cols = row.cells.len().min(viewport_size.cols as usize);
        let target = target_row * viewport_size.cols as usize;
        cells[target..target + copy_cols].clone_from_slice(&row.cells[..copy_cols]);
    }

    let selection = state
        .selection
        .and_then(|selection| normalized_selection(&rows, selection))
        .map_or_else(Vec::new, |(start, end)| {
            let mut spans = Vec::new();
            for (retained_row, row) in rows.iter().enumerate().take(end.row + 1).skip(start.row) {
                let Some(target_row) =
                    viewport_row(&geometry, viewport_size.rows as usize, retained_row)
                else {
                    continue;
                };
                let start_col = if retained_row == start.row {
                    start.col
                } else {
                    0
                };
                let end_col = if retained_row == end.row {
                    end.col.saturating_add(glyph_width(row, end.col))
                } else {
                    row.cells.len()
                }
                .min(viewport_size.cols as usize);
                if start_col < end_col && start_col < viewport_size.cols as usize {
                    spans.push(TerminalSelectionSpan {
                        row: target_row as u32,
                        start_col: start_col as u32,
                        end_col: end_col as u32,
                    });
                }
            }
            spans
        });

    let cursor = if geometry.scroll_offset == 0 {
        projection.cursor.and_then(|cursor| {
            let retained_row = projection.history_len().saturating_add(cursor.row as usize);
            let row = viewport_row(&geometry, viewport_size.rows as usize, retained_row)?;
            (cursor.col < viewport_size.cols).then(|| CellCoord::new(row as u32, cursor.col))
        })
    } else {
        None
    };

    TerminalSnapshot {
        buffer_id,
        size: viewport_size,
        cells,
        cursor,
        title: projection.title.map(str::to_owned),
        screen_generation: projection.generation,
        selection,
        scroll_offset: geometry.scroll_offset,
        at_bottom: geometry.scroll_offset == 0,
        pid,
        process,
    }
}

fn is_default_blank(cell: &Cell) -> bool {
    matches!(cell.glyph, Glyph::Char(' '))
        && cell.style == Style::default()
        && cell.attachment.is_none()
}

fn copy_selection_bytes(rows: &RetainedRows<'_>, selection: TerminalSelection) -> Option<Vec<u8>> {
    let (start, end) = normalized_selection(rows, selection)?;
    let mut out = Vec::new();
    for (row_index, row) in rows.iter().enumerate().take(end.row + 1).skip(start.row) {
        let from = if row_index == start.row { start.col } else { 0 };
        let mut to = if row_index == end.row {
            end.col.saturating_add(glyph_width(row, end.col))
        } else {
            row.cells.len()
        };
        while to > from && is_default_blank(&row.cells[to - 1]) {
            to -= 1;
        }
        for cell in &row.cells[from..to] {
            match &cell.glyph {
                Glyph::Char(ch) => {
                    let mut bytes = [0; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut bytes).as_bytes());
                }
                Glyph::Cluster(bytes) => out.extend_from_slice(bytes),
                Glyph::Continuation => {}
            }
        }
        if row_index < end.row && !row.soft_wrapped {
            out.push(b'\n');
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::TerminalProcessState;
    use crate::terminal::screen::ScreenProjection;

    fn row(id: u64, offset: u32, text: &str, soft_wrapped: bool) -> TerminalRow {
        TerminalRow {
            cells: text
                .chars()
                .map(|ch| Cell {
                    glyph: Glyph::Char(ch),
                    style: Style::default(),
                    attachment: None,
                })
                .collect(),
            logical_line_id: id,
            cell_offset: offset,
            soft_wrapped,
        }
    }

    fn projection(history: Vec<TerminalRow>, visible_rows: Vec<TerminalRow>) -> ScreenProjection {
        let cols = visible_rows.first().map_or(1, |row| row.cells.len() as u32);
        ScreenProjection {
            size: CellSize::new(visible_rows.len() as u32, cols),
            alternate_active: false,
            history,
            visible_rows,
            cursor: None,
            title: Some("shell".into()),
            generation: 7,
            mapping_revision: 0,
        }
    }

    #[test]
    fn tail_projection_pads_above_and_right_and_translates_cursor() {
        let mut source = projection(
            Vec::new(),
            vec![row(1, 0, "abc", false), row(2, 0, "def", false)],
        );
        source.cursor = Some(CellCoord::new(1, 2));
        let snapshot = project_snapshot(
            BufferId::next(),
            CellSize::new(4, 5),
            source.as_borrowed(),
            &TerminalViewState::default(),
            42,
            TerminalProcessState::Running,
        );
        assert_eq!(snapshot.cells.len(), 20);
        assert!(
            snapshot.cells[..10]
                .iter()
                .all(|cell| *cell == Cell::default())
        );
        assert_eq!(snapshot.cells[10].glyph, Glyph::Char('a'));
        assert_eq!(snapshot.cells[13], Cell::default());
        assert_eq!(snapshot.cells[15].glyph, Glyph::Char('d'));
        assert_eq!(snapshot.cursor, Some(CellCoord::new(3, 2)));
        assert!(snapshot.at_bottom);
        assert_eq!(snapshot.scroll_offset, 0);
    }

    #[test]
    fn frozen_top_is_geometrically_at_bottom_when_view_still_reaches_tail() {
        let source = projection(
            vec![row(1, 0, "aaa", false)],
            vec![row(2, 0, "bbb", false), row(3, 0, "ccc", false)],
        );
        let state = TerminalViewState {
            top: Some(LogicalCellAnchor {
                logical_line_id: 1,
                cell_offset: 0,
            }),
            selection: Some(TerminalSelection {
                anchor: LogicalCellAnchor {
                    logical_line_id: 2,
                    cell_offset: 0,
                },
                head: LogicalCellAnchor {
                    logical_line_id: 2,
                    cell_offset: 1,
                },
            }),
            ..TerminalViewState::default()
        };
        let snapshot = project_snapshot(
            BufferId::next(),
            CellSize::new(3, 3),
            source.as_borrowed(),
            &state,
            1,
            TerminalProcessState::Running,
        );
        assert!(snapshot.at_bottom);
        assert_eq!(snapshot.scroll_offset, 0);
    }

    #[test]
    fn copy_joins_soft_wraps_trims_default_blanks_and_separates_hard_rows() {
        let rows = [
            row(1, 0, "ab ", true),
            row(1, 3, "cd ", false),
            row(2, 0, "e  ", false),
        ];
        let source = projection(Vec::new(), rows.into());
        let retained = retained_rows(source.as_borrowed());
        let bytes = copy_selection_bytes(
            &retained,
            TerminalSelection {
                anchor: LogicalCellAnchor {
                    logical_line_id: 1,
                    cell_offset: 0,
                },
                head: LogicalCellAnchor {
                    logical_line_id: 2,
                    cell_offset: 2,
                },
            },
        )
        .expect("selection resolves");
        assert_eq!(bytes, b"abcd\ne");
    }

    /// Stage 2 criteria 13 and 14. Every property here is one a second,
    /// independently written whole-range walk would get wrong: a naive
    /// walk emits a newline per physical row (breaking the soft wrap),
    /// keeps trailing default blanks, and has to rediscover that history
    /// precedes the visible screen. Asserting exact bytes is what makes
    /// "it reuses the serializer" falsifiable.
    #[test]
    fn retained_copy_spans_history_joins_soft_wraps_and_trims_blanks() {
        let source = projection(
            vec![row(1, 0, "ab ", true), row(1, 3, "cd ", false)],
            vec![row(2, 0, "e  ", false), row(3, 0, "   ", false)],
        );
        let retained = retained_rows(source.as_borrowed());
        let bytes = retained_bytes(&retained).expect("whole range resolves");
        // `ab`+`cd` joined across the soft wrap; `e` on its own hard row;
        // the all-blank final row trimmed to nothing but still separated.
        assert_eq!(bytes, b"abcd\ne\n");
    }

    /// The whole-range selection must not depend on a view existing, and
    /// must agree with an explicit full-span selection through the public
    /// serializer — the anti-drift half of criterion 13.
    #[test]
    fn retained_copy_agrees_with_an_explicit_full_span_selection() {
        let source = projection(
            vec![row(1, 0, "aaa", false)],
            vec![row(2, 0, "bbb", false), row(3, 0, "ccc", false)],
        );
        let retained = retained_rows(source.as_borrowed());
        let explicit = copy_selection_bytes(
            &retained,
            TerminalSelection {
                anchor: LogicalCellAnchor {
                    logical_line_id: 1,
                    cell_offset: 0,
                },
                head: LogicalCellAnchor {
                    logical_line_id: 3,
                    cell_offset: 2,
                },
            },
        )
        .expect("explicit selection resolves");
        assert_eq!(retained_bytes(&retained).expect("whole range"), explicit);
        assert_eq!(explicit, b"aaa\nbbb\nccc");
    }

    /// A wide glyph must be copied once across the whole range too, not
    /// once per cell it occupies.
    #[test]
    fn retained_copy_emits_a_wide_glyph_once() {
        let wide = TerminalRow {
            cells: vec![
                Cell {
                    glyph: Glyph::Char('界'),
                    style: Style::default(),
                    attachment: None,
                },
                Cell {
                    glyph: Glyph::Continuation,
                    style: Style::default(),
                    attachment: None,
                },
                Cell::default(),
            ],
            logical_line_id: 9,
            cell_offset: 0,
            soft_wrapped: false,
        };
        let source = projection(Vec::new(), vec![wide]);
        let retained = retained_rows(source.as_borrowed());
        assert_eq!(
            retained_bytes(&retained).expect("whole range"),
            "界".as_bytes()
        );
    }

    /// A session with nothing retained yields `None` rather than an empty
    /// string, so the caller can tell "no terminal" from "empty terminal".
    #[test]
    fn retained_copy_of_zero_width_rows_is_none() {
        let source = projection(Vec::new(), vec![row(1, 0, "", false)]);
        let retained = retained_rows(source.as_borrowed());
        assert!(retained_bytes(&retained).is_none());
    }

    #[test]
    fn wide_continuation_canonicalizes_to_lead_and_copies_once() {
        let wide = TerminalRow {
            cells: vec![
                Cell {
                    glyph: Glyph::Char('界'),
                    style: Style::default(),
                    attachment: None,
                },
                Cell {
                    glyph: Glyph::Continuation,
                    style: Style::default(),
                    attachment: None,
                },
                Cell::default(),
            ],
            logical_line_id: 9,
            cell_offset: 0,
            soft_wrapped: false,
        };
        let source = projection(Vec::new(), vec![wide]);
        let retained = retained_rows(source.as_borrowed());
        let continuation = resolve_anchor(
            &retained,
            LogicalCellAnchor {
                logical_line_id: 9,
                cell_offset: 1,
            },
        )
        .expect("continuation resolves");
        assert_eq!(continuation.col, 0);
        let bytes = copy_selection_bytes(
            &retained,
            TerminalSelection {
                anchor: LogicalCellAnchor {
                    logical_line_id: 9,
                    cell_offset: 0,
                },
                head: LogicalCellAnchor {
                    logical_line_id: 9,
                    cell_offset: 1,
                },
            },
        )
        .expect("wide selection resolves");
        assert_eq!(bytes, "界".as_bytes());
    }

    #[test]
    fn partially_evicted_wrapped_anchor_clamps_to_first_surviving_cell() {
        let source = projection(
            vec![row(7, 4, "tail", true)],
            vec![row(8, 0, "next", false)],
        );
        let first_survivor = LogicalCellAnchor {
            logical_line_id: 7,
            cell_offset: 4,
        };
        let mut state = TerminalViewState {
            top: Some(LogicalCellAnchor {
                logical_line_id: 7,
                cell_offset: 1,
            }),
            selection: Some(TerminalSelection {
                anchor: LogicalCellAnchor {
                    logical_line_id: 7,
                    cell_offset: 2,
                },
                head: LogicalCellAnchor {
                    logical_line_id: 8,
                    cell_offset: 1,
                },
            }),
            ..TerminalViewState::default()
        };

        normalize_state(&mut state, source.as_borrowed());

        assert_eq!(state.top, Some(first_survivor));
        assert_eq!(
            state.selection,
            Some(TerminalSelection {
                anchor: first_survivor,
                head: LogicalCellAnchor {
                    logical_line_id: 8,
                    cell_offset: 1,
                },
            })
        );
    }

    #[test]
    fn alternate_switch_clears_view_anchors_and_selection() {
        let source = ScreenProjection {
            size: CellSize::new(1, 3),
            alternate_active: true,
            history: Vec::new(),
            visible_rows: vec![row(10, 0, "alt", false)],
            cursor: None,
            title: None,
            generation: 2,
            mapping_revision: 0,
        };
        let mut state = TerminalViewState {
            top: Some(LogicalCellAnchor {
                logical_line_id: 1,
                cell_offset: 0,
            }),
            selection: Some(TerminalSelection {
                anchor: LogicalCellAnchor {
                    logical_line_id: 1,
                    cell_offset: 0,
                },
                head: LogicalCellAnchor {
                    logical_line_id: 1,
                    cell_offset: 1,
                },
            }),
            alternate_active: Some(false),
            ..TerminalViewState::default()
        };
        normalize_state(&mut state, source.as_borrowed());
        assert_eq!(state.top, None);
        assert_eq!(state.selection, None);
        assert_eq!(state.alternate_active, Some(true));
    }
}
