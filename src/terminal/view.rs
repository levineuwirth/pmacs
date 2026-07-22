//! Per-frontend terminal viewport, selection, copy, and bell projection.
//!
//! A view stores only logical anchors into one [`TerminalScreen`]. Cells,
//! modes, history, and process state remain session-owned.

use crate::buffer::BufferId;
use crate::cell::{Cell, CellCoord, CellSize, Glyph, Style};
use crate::protocol::FrontendId;
use crate::terminal::screen::{BorrowedScreenProjection, TerminalModes, TerminalRow};
use crate::terminal::session::{TerminalManager, TerminalSelectionSpan, TerminalSnapshot};
use crate::terminal::{MAX_TERMINAL_COLS, MAX_TERMINAL_ROWS, MAX_TERMINAL_VISIBLE_CELLS};
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
        normalize_state(state, projection);
        state.viewport_size = Some(viewport_size);
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

    /// Return fresh geometric status for one registered view.
    #[must_use]
    pub fn view_status(&mut self, key: TerminalViewKey) -> Option<TerminalViewStatus> {
        let projection = self.sessions.get(&key.buffer_id)?.screen.projection_ref();
        let state = self.views.get_mut(&key)?;
        normalize_state(state, projection);
        let rows = retained_rows(projection);
        let size = state.viewport_size?;
        let geometry = view_geometry(&rows, state, size.rows);
        Some(TerminalViewStatus {
            at_bottom: geometry.scroll_offset == 0,
            scroll_offset: geometry.scroll_offset,
            selection: state.selection.is_some(),
        })
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

    /// Clear one view's terminal selection without changing its scroll anchor.
    pub fn clear_selection(&mut self, key: TerminalViewKey) -> bool {
        let Some(state) = self.views.get_mut(&key) else {
            return false;
        };
        state.selection.take().is_some() || state.drag.take().is_some()
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
    (anchor.logical_line_id < first.logical_line_id).then(|| row_lead(first))
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
    process: crate::terminal::session::TerminalProcessState,
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
    use crate::terminal::screen::ScreenProjection;
    use crate::terminal::session::TerminalProcessState;

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
    fn alternate_switch_clears_view_anchors_and_selection() {
        let source = ScreenProjection {
            size: CellSize::new(1, 3),
            alternate_active: true,
            history: Vec::new(),
            visible_rows: vec![row(10, 0, "alt", false)],
            cursor: None,
            title: None,
            generation: 2,
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
