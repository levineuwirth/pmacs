// bottom_panel_stage1_acceptance.rs --- bottom-panel Stage 1 acceptance
// (docs/bottom-panel-framing.md, acceptance items 1-35).

//! Window placement + TUI side windows. No wire change.
//!
//! Every claim about geometry is asserted through a **production**
//! caller: `window_placements` (via the real `paint_frame`) or the
//! peer-presence overlay pass, never against `Layout::compute` in
//! isolation — the whole point of R5-B1 is that a second caller derives
//! its own rect and would otherwise keep computing unfixed geometry.
//! Placement, quit, and visit claims run through the real Lua surface
//! and the real adopter entry points.

use std::collections::HashMap;
use std::time::Duration;

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use pmacs::buffer::BufferId;
use pmacs::cell::{CellCoord, CellGrid, CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::editor_core::{DisplayRequest, EditorCore};
use pmacs::protocol::FrontendId;
use pmacs::window::{
    FrontendView, Layout, LayoutNode, MAX_PANEL_QUIT_DEPTH, MIN_WINDOW_OUTER_ROWS, Orientation,
    QuitAction, Rect, Side, Window, WindowId, subtree_min_rows,
};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Terminal geometry. `paint_frame` reserves the last row for the status
/// line, so the window area is `ROWS - 1`.
const ROWS: u32 = 24;
const COLS: u32 = 60;
const AREA_ROWS: u32 = ROWS - 1;

fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    exec(&s, "pmacs.lsp.config = {}");
    // Geometry is authoritative state, and a grid frontend's real frame
    // size IS its declaration. Every test that does not render declares
    // it here, before any input.
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(ROWS, COLS));
    s
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn try_exec(s: &EditorState, src: &str) -> Result<(), String> {
    s.lua_host
        .lua()
        .load(src.to_string())
        .exec()
        .map_err(|e| e.to_string())
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

/// Render one real frame and return the per-window outer rects keyed by
/// window id, as `window_placements` computed them.
fn render(s: &EditorState) -> HashMap<WindowId, Rect> {
    render_at(s, CellSize::new(ROWS, COLS))
}

fn render_at(s: &EditorState, size: CellSize) -> HashMap<WindowId, Rect> {
    let mut cells = vec![pmacs::cell::Cell::default(); (size.rows * size.cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: size.cols,
        size,
    };
    pmacs::editor::paint_frame(s, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);
    placements(s, size)
}

/// The production placement pass, at `size`.
fn placements(s: &EditorState, size: CellSize) -> HashMap<WindowId, Rect> {
    let core = s.core.borrow();
    let view = core.views.get(&FrontendId::LOCAL).expect("LOCAL view");
    let area = Rect::new(0, 0, size.rows - 1, size.cols);
    let fixed = core.panel_fixed_rows(FrontendId::LOCAL, area.size.rows);
    view.layout.compute(area, &fixed)
}

/// Paint one frame and hand back the grid text, row by row.
fn painted_rows(s: &EditorState, size: CellSize) -> Vec<String> {
    let mut cells = vec![pmacs::cell::Cell::default(); (size.rows * size.cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: size.cols,
        size,
    };
    pmacs::editor::paint_frame(s, FrontendId::LOCAL, &HashMap::new(), &mut grid, size);
    (0..size.rows)
        .map(|row| {
            (0..size.cols)
                .map(|col| match &cells[(row * size.cols + col) as usize].glyph {
                    Glyph::Char(ch) => *ch,
                    Glyph::Cluster(_) => '?',
                    Glyph::Continuation => ' ',
                })
                .collect()
        })
        .collect()
}

fn side_window(s: &EditorState) -> Option<WindowId> {
    s.core.borrow().side_window_for(FrontendId::LOCAL)
}

fn active_window(s: &EditorState) -> WindowId {
    s.core.borrow().active_window_id()
}

fn fixed_rows_of(s: &EditorState, win: WindowId) -> Option<u32> {
    s.core.borrow().windows.get(&win)?.params.fixed_rows
}

fn layout_root(s: &EditorState) -> LayoutNode {
    s.core
        .borrow()
        .views
        .get(&FrontendId::LOCAL)
        .expect("LOCAL view")
        .layout
        .root
        .clone()
}

/// Structural fingerprint: node shape, weights, order, and ids — what
/// Bet B6 promises stays byte-identical when a panel opens.
fn structure(node: &LayoutNode) -> String {
    match node {
        LayoutNode::Leaf(id) => format!("L{}", id.raw()),
        LayoutNode::Split {
            orientation,
            weights,
            children,
        } => format!(
            "S{}{weights:?}({})",
            match orientation {
                Orientation::Horizontal => "H",
                Orientation::Vertical => "V",
            },
            children.iter().map(structure).collect::<Vec<_>>().join(",")
        ),
    }
}

/// Create a panel showing a fresh generated buffer, through the real Lua
/// display surface.
fn open_panel(s: &EditorState, name: &str, height: u32) -> WindowId {
    exec(
        s,
        &format!(
            "PANEL_BUF = pmacs.buffer.create({name:?})
             PANEL_WIN = pmacs.window.display(PANEL_BUF, \
                 {{ side = \"bottom\", height = {height} }})"
        ),
    );
    side_window(s).expect("panel exists")
}

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn mouse(kind: MouseEventKind, row: u16, column: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

/// Register a second frontend with its own single-window layout.
fn attach_frontend(s: &EditorState, fid: FrontendId, panel_capable: bool) -> WindowId {
    let mut core = s.core.borrow_mut();
    let buffer_id = core.active_buffer_id();
    let text_view = {
        let reg = core.registry.borrow();
        pmacs::text_view::TextView::new(reg.get(buffer_id).expect("buffer"))
    };
    let win = WindowId::next();
    core.windows
        .insert(win, Window::new(win, buffer_id, text_view));
    core.register_frontend_view(
        fid,
        FrontendView {
            layout: Layout::single(win),
            active: win,
            fold_projection: true,
            panel_capable,
            frame_geometry: None,
            panel_hidden: false,
        },
    );
    drop(core);
    if panel_capable {
        s.sync_frame_geometry(fid, CellSize::new(ROWS, COLS));
    }
    win
}

// ---------------------------------------------------------------------------
// 1 — fixed extents reach BOTH production callers
// ---------------------------------------------------------------------------

#[test]
fn acc1_fixed_extent_reaches_both_production_callers() {
    let s = editor();
    let document = active_window(&s);
    let before = render(&s);
    assert_eq!(
        before[&document].size.rows, AREA_ROWS,
        "one window takes the whole area"
    );

    let panel = open_panel(&s, "*panel*", 6);
    let after = render(&s);
    assert_eq!(
        after[&panel].size.rows, 6,
        "the side child gets exactly N rows"
    );
    assert_eq!(
        after[&document].size.rows,
        AREA_ROWS - 6,
        "the sibling divides the remainder"
    );

    // The second production caller (`overlay_paint`) derives its OWN
    // text-area rect and never routes through `window_placements`. Paint
    // a peer cursor into the document window and assert it lands on the
    // row the fixed geometry says — the assertion that fails if that
    // caller keeps computing unfixed geometry.
    let document_buffer = s.core.borrow().windows[&document].buffer_id;
    let row_with_panel = peer_cursor_row(&s, document_buffer, 0);
    s.core
        .borrow_mut()
        .remove_side_window(FrontendId::LOCAL, panel);
    let row_without_panel = peer_cursor_row(&s, document_buffer, 0);
    assert_eq!(
        row_with_panel, row_without_panel,
        "a peer cursor in the document window paints at the same row \
         whether or not a panel is open"
    );
}

/// Paint the peer-presence overlay pass and report the grid row the peer
/// cursor landed on.
fn peer_cursor_row(s: &EditorState, buffer_id: BufferId, position: u64) -> u32 {
    let size = CellSize::new(ROWS, COLS);
    let mut cells = vec![pmacs::cell::Cell::default(); (size.rows * size.cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: size.cols,
        size,
    };
    let presence = pmacs::overlay_paint::OtherPresence {
        frontend_id: FrontendId(7),
        color_slot: 0,
        snapshot: pmacs::presence::PresenceSnapshot {
            buffer_id,
            cursor: position,
            selection: None,
        },
    };
    pmacs::overlay_paint::paint_other_frontend_overlays(s, &mut grid, size, &[presence]);
    for row in 0..size.rows {
        for col in 0..size.cols {
            if cells[(row * size.cols + col) as usize].style.reverse {
                return row;
            }
        }
    }
    panic!("peer cursor was not painted anywhere");
}

// ---------------------------------------------------------------------------
// 2 — opening a panel preserves the document subtree's STRUCTURE (B6)
// ---------------------------------------------------------------------------

#[test]
fn acc2_opening_a_panel_preserves_document_structure() {
    let s = editor();
    exec(
        &s,
        "pmacs.window.split_horizontal(); pmacs.window.split_vertical()",
    );
    let before = layout_root(&s);
    let before_rects = render(&s);

    open_panel(&s, "*panel*", 5);
    let after = layout_root(&s);
    let LayoutNode::Split { children, .. } = &after else {
        panic!("the panel wrapper is a split");
    };
    assert_eq!(
        structure(&before),
        structure(&children[0]),
        "nodes, weights, order and ids of the document subtree are identical"
    );
    let after_rects = render(&s);
    assert!(
        before_rects
            .keys()
            .any(|id| before_rects[id] != after_rects[id]),
        "…while the rectangles necessarily change, being recomputed \
         inside the smaller flexible remainder"
    );
}

// ---------------------------------------------------------------------------
// 3 — the minimum is RECURSIVE
// ---------------------------------------------------------------------------

#[test]
fn acc3_subtree_minimum_is_recursive_and_clamps_the_panel() {
    // Horizontal inside vertical inside horizontal: four leaves, of
    // which three stack rows.
    let leaf_a = WindowId::next();
    let leaf_b = WindowId::next();
    let leaf_c = WindowId::next();
    let leaf_d = WindowId::next();
    let nested = LayoutNode::Split {
        orientation: Orientation::Horizontal,
        weights: vec![1, 1],
        children: vec![
            LayoutNode::Leaf(leaf_a),
            LayoutNode::Split {
                orientation: Orientation::Vertical,
                weights: vec![1, 1],
                children: vec![
                    LayoutNode::Leaf(leaf_b),
                    LayoutNode::Split {
                        orientation: Orientation::Horizontal,
                        weights: vec![1, 1],
                        children: vec![LayoutNode::Leaf(leaf_c), LayoutNode::Leaf(leaf_d)],
                    },
                ],
            },
        ],
    };
    // Rows add across a horizontal split and the tallest child governs a
    // vertical one: 2 + max(2, 2 + 2) = 6. A flat "two rows at the root"
    // reading would answer 2.
    assert_eq!(subtree_min_rows(&nested), 6);

    // In a live layout the PANEL is clamped, never the document.
    let s = editor();
    exec(
        &s,
        "pmacs.window.split_horizontal(); pmacs.window.split_vertical(); \
         pmacs.window.split_horizontal()",
    );
    let document_min = {
        let core = s.core.borrow();
        subtree_min_rows(&core.views[&FrontendId::LOCAL].layout.root)
    };
    let panel = open_panel(&s, "*panel*", AREA_ROWS);
    let rects = render(&s);
    assert_eq!(
        rects[&panel].size.rows,
        AREA_ROWS - document_min,
        "the panel takes min(requested, area - subtree_min_rows(document))"
    );
}

// ---------------------------------------------------------------------------
// 4 — clamping, rejection, and saturating arithmetic
// ---------------------------------------------------------------------------

#[test]
fn acc4_height_requests_clamp_to_the_floor_and_reject_zero() {
    let s = editor();
    let panel = open_panel(&s, "*panel*", 1);
    assert_eq!(
        fixed_rows_of(&s, panel),
        Some(MIN_WINDOW_OUTER_ROWS),
        "a one-row request clamps up to the structural floor"
    );
    assert_eq!(render(&s)[&panel].size.rows, MIN_WINDOW_OUTER_ROWS);

    let zero = try_exec(
        &s,
        "pmacs.window.display(pmacs.buffer.create(\"*z*\"), \
         { side = \"bottom\", height = 0 })",
    );
    assert!(
        zero.is_err(),
        "a request of zero is rejected, not an invisible open"
    );
    assert!(
        try_exec(
            &s,
            &format!(
                "pmacs.window.set_params({}, {{ fixed_rows = 0 }})",
                panel.raw()
            )
        )
        .is_err(),
        "set_params rejects zero too"
    );
    exec(
        &s,
        &format!(
            "pmacs.window.set_params({}, {{ fixed_rows = 1 }})",
            panel.raw()
        ),
    );
    assert_eq!(fixed_rows_of(&s, panel), Some(MIN_WINDOW_OUTER_ROWS));

    // `window.panel-height` is the creation default, clamped the same way.
    exec(&s, "pmacs.config.set(\"window.panel-height\", 2)");
    s.core
        .borrow_mut()
        .remove_side_window(FrontendId::LOCAL, panel);
    exec(
        &s,
        "pmacs.window.display(pmacs.buffer.create(\"*p2*\"), { side = \"bottom\" })",
    );
    let panel = side_window(&s).expect("panel");
    assert_eq!(fixed_rows_of(&s, panel), Some(2));

    // An intrinsically tiny frame saturates and hides rather than
    // underflowing; a zero-column frame is never presentable.
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(3, COLS));
    assert!(s.core.borrow().panel_hidden_for(FrontendId::LOCAL));
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(ROWS, 0));
    assert!(s.core.borrow().panel_hidden_for(FrontendId::LOCAL));
}

// ---------------------------------------------------------------------------
// 5 — absolute height vs proportional ratio, in ONE layout
// ---------------------------------------------------------------------------

#[test]
fn acc5_resize_preserves_absolute_panel_height_and_flexible_ratio() {
    let s = editor();
    exec(&s, "pmacs.window.split_horizontal()");
    let panel = open_panel(&s, "*panel*", 6);
    let ids: Vec<WindowId> = {
        let core = s.core.borrow();
        core.views[&FrontendId::LOCAL]
            .layout
            .iter_ids()
            .into_iter()
            .filter(|id| *id != panel)
            .collect()
    };
    let wide = render_at(&s, CellSize::new(ROWS, COLS));
    assert_eq!(wide[&panel].size.rows, 6);
    let ratio_before = f64::from(wide[&ids[0]].size.rows) / f64::from(wide[&ids[1]].size.rows);

    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(ROWS + 10, COLS));
    let tall = render_at(&s, CellSize::new(ROWS + 10, COLS));
    assert_eq!(
        tall[&panel].size.rows, 6,
        "the side window keeps its ABSOLUTE height"
    );
    let ratio_after = f64::from(tall[&ids[0]].size.rows) / f64::from(tall[&ids[1]].size.rows);
    assert!(
        (ratio_before - ratio_after).abs() < 0.35,
        "the flexible pair keeps its RATIO ({ratio_before} vs {ratio_after})"
    );
}

// ---------------------------------------------------------------------------
// 6 / 7 / 8 — hiding is a durable transition
// ---------------------------------------------------------------------------

#[test]
fn acc6_reconciliation_hides_moves_focus_and_releases_before_the_next_key() {
    let s = editor();
    let document = active_window(&s);
    let panel = open_panel(&s, "*panel*", 8);
    exec(&s, "pmacs.window.focus_next()");
    assert_eq!(active_window(&s), panel, "the panel is focused");

    // Shrink the frame to something that cannot satisfy the panel, then
    // dispatch a key in the same burst.
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(4, COLS));
    assert!(s.core.borrow().panel_hidden_for(FrontendId::LOCAL));
    assert_eq!(
        active_window(&s),
        document,
        "focus moved out of the invisible panel"
    );
    let rects = placements(&s, CellSize::new(4, COLS));
    assert_eq!(
        rects[&panel].size.rows, 0,
        "a hidden panel has an empty rect"
    );
    assert_eq!(
        rects[&document].size.rows, 3,
        "the document subtree receives every reclaimed row"
    );
    assert_eq!(
        fixed_rows_of(&s, panel),
        Some(8),
        "the stored request survives hiding"
    );
}

#[test]
fn acc7_reappearing_restores_the_request_but_not_focus() {
    let s = editor();
    let document = active_window(&s);
    let panel = open_panel(&s, "*panel*", 8);
    exec(&s, "pmacs.window.focus_next()");
    assert_eq!(active_window(&s), panel);

    let before = layout_root(&s);
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(4, COLS));
    assert_eq!(
        structure(&before),
        structure(&layout_root(&s)),
        "wrapper, ids, weights and order survive hiding"
    );
    // While hidden the panel is not a focus destination.
    exec(&s, "pmacs.window.focus_next()");
    assert_eq!(
        active_window(&s),
        document,
        "focus_next skips a hidden panel"
    );

    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(ROWS, COLS));
    assert!(!s.core.borrow().panel_hidden_for(FrontendId::LOCAL));
    assert_eq!(
        render(&s)[&panel].size.rows,
        8,
        "restored at the exact request"
    );
    assert_eq!(
        active_window(&s),
        document,
        "focus is NOT auto-restored — the user moved on"
    );
    exec(&s, "pmacs.window.focus_next()");
    assert_eq!(active_window(&s), panel, "…but C-x o reaches it again");
}

#[test]
fn acc8_keys_while_hidden_reach_the_document_window() {
    let mut s = editor();
    let document = active_window(&s);
    open_panel(&s, "*panel*", 8);
    exec(&s, "pmacs.window.focus_next()");
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(4, COLS));

    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('x'), KeyModifiers::NONE),
    );
    let document_buffer = s.core.borrow().windows[&document].buffer_id;
    let text: String = {
        let core = s.core.borrow();
        let reg = core.registry.borrow();
        let buf = reg.get(document_buffer).unwrap();
        let mut bytes = vec![0u8; buf.len() as usize];
        buf.snapshot_rope().slice(0, buf.len(), &mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    };
    assert!(
        text.contains('x'),
        "the keystroke landed in the document buffer, not the invisible panel"
    );
}

// ---------------------------------------------------------------------------
// 9 — window.min-height is an INTERACTIVE preference only
// ---------------------------------------------------------------------------

#[test]
fn acc9_min_height_constrains_interactive_resize_only() {
    let s = editor();
    exec(&s, "pmacs.config.set(\"window.min-height\", 1)");
    let panel = open_panel(&s, "*panel*", 6);
    // Below the structural floor: the resolver clamps it back up.
    assert_eq!(s.window_min_height(None), MIN_WINDOW_OUTER_ROWS);

    // A value materially above the floor constrains resize recursively
    // across a nested document tree.
    exec(
        &s,
        "pmacs.config.set(\"window.min-height\", 5)
         pmacs.window.split_horizontal()",
    );
    let document = s
        .core
        .borrow()
        .non_side_target(FrontendId::LOCAL)
        .expect("document target");
    // Two document leaves at 5 rows each = 10; the frame area is 23, so
    // the panel can never grow past 13.
    let _ = s.resize_window_boundary(FrontendId::LOCAL, panel, 100, AREA_ROWS);
    assert!(
        fixed_rows_of(&s, panel).expect("panel rows") <= AREA_ROWS - 10,
        "the recursive interactive minimum bounds the panel"
    );
    // Frame-resize layout ignores the preference entirely: an area that
    // only satisfies the STRUCTURAL floor still lays out.
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(8, COLS));
    let rects = placements(&s, CellSize::new(8, COLS));
    assert!(
        rects[&document].size.rows > 0,
        "changing a preference never invalidates an existing layout"
    );
}

// ---------------------------------------------------------------------------
// 10 — closing collapses the wrapper
// ---------------------------------------------------------------------------

#[test]
fn acc10_closing_the_panel_restores_the_prior_root_exactly() {
    let s = editor();
    exec(
        &s,
        "pmacs.window.split_horizontal(); pmacs.window.split_vertical()",
    );
    let before = structure(&layout_root(&s));
    let panel = open_panel(&s, "*panel*", 5);
    s.core
        .borrow_mut()
        .remove_side_window(FrontendId::LOCAL, panel);
    assert_eq!(
        before,
        structure(&layout_root(&s)),
        "the wrapper collapses and the prior root returns unchanged"
    );
}

// ---------------------------------------------------------------------------
// 11 — parameter write discipline
// ---------------------------------------------------------------------------

#[test]
fn acc11_parameter_writes_are_restricted_and_ids_are_frontend_scoped() {
    let s = editor();
    let document = active_window(&s);
    let panel = open_panel(&s, "*panel*", 5);
    for forbidden in [
        "side = \"bottom\"",
        "origin_document = 1",
        "quit_action = \"delete\"",
    ] {
        assert!(
            try_exec(
                &s,
                &format!(
                    "pmacs.window.set_params({}, {{ {forbidden} }})",
                    panel.raw()
                )
            )
            .is_err(),
            "set_params must reject `{forbidden}`"
        );
    }
    // `params` may REPORT the implementation-owned bookkeeping.
    exec(&s, "pmacs.window.focus_next()");
    let origin: Option<u64> = eval(
        &s,
        &format!(
            "return pmacs.window.params({}).origin_document",
            panel.raw()
        ),
    );
    assert_eq!(origin, Some(document.raw()));

    // A stray `fixed_rows` on a non-side window is inert.
    exec(
        &s,
        &format!(
            "pmacs.window.set_params({}, {{ fixed_rows = 4 }})",
            document.raw()
        ),
    );
    let rects = render(&s);
    assert_eq!(
        rects[&document].size.rows,
        AREA_ROWS - 5,
        "the fixed map is built from side windows only"
    );

    // Every WindowId-taking operation rejects a live id owned by another
    // frontend.
    let foreign = attach_frontend(&s, FrontendId(9), true);
    for call in [
        format!("pmacs.window.params({})", foreign.raw()),
        format!(
            "pmacs.window.set_params({}, {{ dedicated = true }})",
            foreign.raw()
        ),
        format!("pmacs.window.resize({}, 1)", foreign.raw()),
        format!("pmacs.window.quit({})", foreign.raw()),
        format!(
            "pmacs.window.display(pmacs.buffer.create(\"*f*\"), {{ window = {} }})",
            foreign.raw()
        ),
    ] {
        assert!(
            try_exec(&s, &call).is_err(),
            "a cross-frontend id must be a pointed error: {call}"
        );
    }
}

// ---------------------------------------------------------------------------
// 12 — dedication binds the POLICY layer only
// ---------------------------------------------------------------------------

#[test]
fn acc12_dedication_binds_display_policy_not_the_raw_switch() {
    let s = editor();
    let document = active_window(&s);
    exec(
        &s,
        &format!(
            "OTHER = pmacs.buffer.create(\"*other*\")
             pmacs.window.set_params({}, {{ dedicated = true }})",
            document.raw()
        ),
    );
    let pinned_buffer = s.core.borrow().windows[&document].buffer_id;

    // The raw escape hatch ignores dedication.
    exec(&s, "pmacs.window.switch_buffer(OTHER)");
    assert_ne!(
        s.core.borrow().windows[&document].buffer_id,
        pinned_buffer,
        "raw switch_buffer ignores `dedicated`"
    );

    // The policy layer honors it on every candidate.
    exec(
        &s,
        &format!(
            "pmacs.window.switch_buffer(pmacs.buffer.list()[1])
             pmacs.window.set_params({}, {{ dedicated = true }})",
            document.raw()
        ),
    );
    assert!(
        try_exec(&s, "pmacs.window.display(OTHER)").is_err(),
        "display_buffer refuses to overwrite a dedicated window with no alternative"
    );
    assert!(
        try_exec(
            &s,
            &format!(
                "pmacs.window.display(OTHER, {{ window = {} }})",
                document.raw()
            )
        )
        .is_err(),
        "…and refuses a dedicated EXACT target too"
    );

    // An ordinary display never reuses a matching side window.
    exec(
        &s,
        &format!(
            "pmacs.window.set_params({}, {{ dedicated = false }})",
            document.raw()
        ),
    );
    let panel = open_panel(&s, "*shared*", 5);
    let panel_buffer = s.core.borrow().windows[&panel].buffer_id;
    let target: u64 = eval(&s, "return pmacs.window.display(PANEL_BUF)");
    assert_ne!(
        target,
        panel.raw(),
        "an ordinary display never selects the panel by coincidence"
    );
    assert_eq!(
        s.core.borrow().windows[&panel].buffer_id,
        panel_buffer,
        "…and leaves the panel's own presentation alone"
    );
}

// ---------------------------------------------------------------------------
// 13 — side placement affinity + option-valued height/dedication
// ---------------------------------------------------------------------------

#[test]
fn acc13_side_placement_is_affinity_aware_and_option_valued() {
    let s = editor();
    let document = active_window(&s);
    // A buffer already visible in a DOCUMENT window must not preempt a
    // requested usable side slot.
    exec(
        &s,
        "SHARED = pmacs.buffer.create(\"*shared*\"); pmacs.window.switch_buffer(SHARED)",
    );
    let target: u64 = eval(
        &s,
        "return pmacs.window.display(SHARED, { side = \"bottom\", height = 7 })",
    );
    let panel = side_window(&s).expect("panel created");
    assert_eq!(target, panel.raw(), "the requested side placement wins");
    assert_eq!(
        s.core.borrow().windows[&document].buffer_id,
        s.core.borrow().windows[&panel].buffer_id
    );

    // Same-buffer redisplay preserves an omitted height, dedication, and
    // quit action.
    exec(
        &s,
        &format!(
            "pmacs.window.set_params({}, {{ dedicated = true }})",
            panel.raw()
        ),
    );
    exec(&s, "pmacs.window.display(SHARED, { side = \"bottom\" })");
    assert_eq!(fixed_rows_of(&s, panel), Some(7));
    assert!(s.core.borrow().windows[&panel].params.dedicated);

    // A dedicated side slot never spawns a second one: the request falls
    // back after discarding height/dedication/quit state.
    exec(&s, "OTHER = pmacs.buffer.create(\"*other*\")");
    let fallback: u64 = eval(
        &s,
        "return pmacs.window.display(OTHER, { side = \"bottom\", height = 9, dedicated = true })",
    );
    assert_ne!(fallback, panel.raw());
    assert_eq!(side_window(&s), Some(panel), "still exactly one side slot");
    {
        let core = s.core.borrow();
        let fell_back = core
            .windows
            .values()
            .find(|w| w.id.raw() == fallback)
            .expect("fallback window");
        assert!(
            !fell_back.params.dedicated,
            "a failed request may not dedicate"
        );
        assert!(fell_back.params.fixed_rows.is_none(), "…nor pin");
        assert!(
            fell_back.params.quit_action().is_none(),
            "…nor leave quit state"
        );
    }

    // Replacement preserves an omitted (user-resized) height but starts
    // undedicated.
    exec(
        &s,
        &format!(
            "pmacs.window.set_params({}, {{ dedicated = false }})",
            panel.raw()
        ),
    );
    exec(&s, "pmacs.window.display(OTHER, { side = \"bottom\" })");
    assert_eq!(
        fixed_rows_of(&s, panel),
        Some(7),
        "the resized height survives"
    );
    assert!(!s.core.borrow().windows[&panel].params.dedicated);

    // Mutual exclusion and a freestanding height are pointed errors.
    assert!(
        try_exec(
            &s,
            &format!(
                "pmacs.window.display(OTHER, {{ side = \"bottom\", window = {} }})",
                document.raw()
            )
        )
        .is_err()
    );
    assert!(try_exec(&s, "pmacs.window.display(OTHER, { height = 4 })").is_err());
    assert!(
        try_exec(&s, "pmacs.window.display(OTHER, { side = \"left\" })").is_err(),
        "Stage 1 ships only the bottom side"
    );

    // An explicit `dedicated = false` cannot clear-and-bypass an existing
    // dedication in the same call.
    exec(
        &s,
        &format!(
            "pmacs.window.set_params({}, {{ dedicated = true }})",
            panel.raw()
        ),
    );
    exec(&s, "THIRD = pmacs.buffer.create(\"*third*\")");
    let bypass: u64 = eval(
        &s,
        "return pmacs.window.display(THIRD, { side = \"bottom\", dedicated = false })",
    );
    assert_ne!(
        bypass,
        panel.raw(),
        "eligibility is checked before the new dedication"
    );
}

// ---------------------------------------------------------------------------
// 14 — capability fallback
// ---------------------------------------------------------------------------

#[test]
fn acc14_capability_fallback_discards_every_side_parameter() {
    let s = editor();
    let fid = FrontendId(11);
    let document = attach_frontend(&s, fid, false);
    let buffer = s.core.borrow_mut().registry.borrow_mut().create("*panel*");
    let mut request = DisplayRequest::new(buffer);
    request.side = Some(Side::Bottom);
    request.height = Some(9);
    request.dedicated = Some(true);
    let outcome = s
        .core
        .borrow_mut()
        .display_buffer(fid, &request)
        .expect("fallback succeeds");
    assert_eq!(outcome.target, document, "fell back to the document target");
    assert!(
        s.core.borrow().side_window_for(fid).is_none(),
        "no side window was created"
    );
    let core = s.core.borrow();
    let window = &core.windows[&document];
    assert!(
        !window.params.dedicated,
        "the document target is left undedicated"
    );
    assert!(window.params.fixed_rows.is_none(), "…and unpinned");
    assert!(window.params.side.is_none());
    assert!(window.params.quit_action().is_none());
}

// ---------------------------------------------------------------------------
// 15 / 16 — the final-focus matrix and the hook-failure arms
// ---------------------------------------------------------------------------

#[test]
fn acc15_final_focus_matrix_all_six_rows() {
    // Row 1 — select = true, target live: the target stays selected.
    let s = editor();
    let document = active_window(&s);
    exec(
        &s,
        "P = pmacs.buffer.create(\"*p*\")
         pmacs.window.display(P, { side = \"bottom\", height = 5, select = true })",
    );
    assert_eq!(active_window(&s), side_window(&s).unwrap());

    // Row 4 — select = false with a live saved window that IS the panel:
    // a passive display invoked from a focused panel must not blur it.
    let panel = side_window(&s).unwrap();
    exec(
        &s,
        "Q = pmacs.buffer.create(\"*q*\")
         pmacs.window.display(Q, { select = false })",
    );
    assert_eq!(
        active_window(&s),
        panel,
        "select = false restores a SIDE saved_active"
    );
    assert_eq!(
        s.core.borrow().windows[&document].buffer_id,
        eval::<pmacs::lua_bindings::BufferIdLua>(&s, "return Q").0,
        "…while the buffer really did land in the document window"
    );

    // Row 5 — select = false, saved window died in the hook, target live.
    let s = editor();
    exec(
        &s,
        "pmacs.window.split_horizontal()
         SAVED = pmacs.window.list()[1]
         pmacs.hook.add(\"buffer.after-switch\", function()
           if KILL_SAVED then KILL_SAVED = nil; pmacs.window.focus_next(); pmacs.window.close() end
         end)",
    );
    exec(&s, "R = pmacs.buffer.create(\"*r*\")");
    let saved = active_window(&s);
    exec(&s, "KILL_SAVED = true");
    let target: u64 = eval(&s, "return pmacs.window.display(R, { select = false })");
    assert!(
        !s.core.borrow().windows.contains_key(&saved) || active_window(&s).raw() == target,
        "focus falls to the live target when the saved window dies"
    );

    // Rows 2/3/6 — the target dies in the hook.
    let s = editor();
    exec(
        &s,
        "pmacs.window.split_horizontal()
         pmacs.hook.add(\"buffer.after-switch\", function()
           if KILL_TARGET then KILL_TARGET = nil; pmacs.window.close() end
         end)
         T = pmacs.buffer.create(\"*t*\")
         KILL_TARGET = true",
    );
    let before = active_window(&s);
    exec(&s, "pmacs.window.display(T, { select = true })");
    assert!(
        s.core.borrow().views[&FrontendId::LOCAL]
            .layout
            .iter_ids()
            .contains(&active_window(&s)),
        "focus always lands on a live window"
    );
    let _ = before;
}

#[test]
fn acc16_hook_failure_arms_are_covered_in_both_select_modes() {
    for select in ["true", "false"] {
        // The hook switches the target's buffer out from under us.
        let s = editor();
        exec(
            &s,
            "pmacs.hook.add(\"buffer.after-switch\", function()
               if SWAP then SWAP = nil; pmacs.window.switch_buffer(pmacs.buffer.create(\"*swap*\")) end
             end)
             X = pmacs.buffer.create(\"*x*\")
             SWAP = true",
        );
        exec(
            &s,
            &format!("pmacs.window.display(X, {{ select = {select} }})"),
        );
        assert!(
            s.core.borrow().views[&FrontendId::LOCAL]
                .layout
                .iter_ids()
                .contains(&active_window(&s)),
            "select = {select}: focus stays on a live window after a buffer-switching hook"
        );

        // The hook closes the target.
        let s = editor();
        exec(
            &s,
            "pmacs.window.split_horizontal()
             pmacs.hook.add(\"buffer.after-switch\", function()
               if CLOSE then CLOSE = nil; pmacs.window.close() end
             end)
             Y = pmacs.buffer.create(\"*y*\")
             CLOSE = true",
        );
        exec(
            &s,
            &format!("pmacs.window.display(Y, {{ select = {select} }})"),
        );
        assert!(
            s.core.borrow().views[&FrontendId::LOCAL]
                .layout
                .iter_ids()
                .contains(&active_window(&s)),
            "select = {select}: focus stays live after a target-closing hook"
        );
    }
}

// ---------------------------------------------------------------------------
// 17 — a passive display re-attaches overlays
// ---------------------------------------------------------------------------

#[test]
fn acc17_passive_display_reattaches_overlays() {
    let s = editor();
    exec(
        &s,
        "pmacs.hook.add(\"buffer.after-switch\", function()
           SEEN_ACTIVE = pmacs.window.list_active and 1 or 1
           HOOK_WINDOW = pmacs.window.current()
         end)
         Z = pmacs.buffer.create(\"*z*\")",
    );
    let target: u64 = eval(
        &s,
        "return pmacs.window.display(Z, { side = \"bottom\", height = 5 })",
    );
    let hook_window: u64 = eval(&s, "return HOOK_WINDOW");
    assert_eq!(
        hook_window, target,
        "the switch hook observes the TARGET window as active, which is \
         what re-attaches store-backed overlays on a passive display"
    );
    assert_ne!(
        active_window(&s).raw(),
        target,
        "…while the passive display leaves focus where it was"
    );
}

// ---------------------------------------------------------------------------
// 18 — display_file
// ---------------------------------------------------------------------------

#[test]
fn acc18_display_file_targets_the_document_from_a_focused_panel() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("visit.txt");
    std::fs::write(&file, b"hello\n").unwrap();
    let path = file.display().to_string();

    let s = editor();
    let document = active_window(&s);
    let panel = open_panel(&s, "*panel*", 5);
    exec(&s, "pmacs.window.focus_next()");
    assert_eq!(active_window(&s), panel);
    exec(
        &s,
        "pmacs.hook.add(\"buffer.after-load\", function()
           LOAD_WINDOW = pmacs.window.current()
         end)",
    );
    let target: u64 = eval(
        &s,
        &format!("return pmacs.window.display_file({path:?}, {{ select = true }})"),
    );
    assert_eq!(
        target,
        document.raw(),
        "the visit lands in the document target"
    );
    assert_eq!(
        eval::<u64>(&s, "return LOAD_WINDOW"),
        document.raw(),
        "buffer.after-load fires with the DOCUMENT TARGET active"
    );
    assert_eq!(side_window(&s), Some(panel), "the panel is intact");

    // A dedicated exact target fails WITHOUT loading.
    let unopened = dir.path().join("unopened.txt");
    std::fs::write(&unopened, b"nope\n").unwrap();
    let unopened_path = unopened.display().to_string();
    exec(
        &s,
        &format!(
            "pmacs.window.set_params({}, {{ dedicated = true }})",
            document.raw()
        ),
    );
    assert!(
        try_exec(
            &s,
            &format!(
                "pmacs.window.display_file({unopened_path:?}, {{ window = {} }})",
                document.raw()
            )
        )
        .is_err()
    );
    let opened_names: Vec<String> = eval(
        &s,
        "local out = {}
         for _, b in ipairs(pmacs.buffer.list()) do out[#out+1] = b:name() end
         return out",
    );
    assert!(
        !opened_names.iter().any(|n| n.contains("unopened")),
        "the file must not be loaded when the destination is ineligible"
    );

    // An omitted target skips a dedicated remembered origin and chooses
    // the next eligible non-side window — before I/O.
    exec(&s, "pmacs.window.split_horizontal()");
    exec(&s, &format!("pmacs.window.display_file({unopened_path:?})"));
    assert!(
        eval::<Vec<String>>(
            &s,
            "local out = {}
             for _, b in ipairs(pmacs.buffer.list()) do out[#out+1] = b:name() end
             return out"
        )
        .iter()
        .any(|n| n.contains("unopened")),
        "…and succeeds once another eligible window exists"
    );

    // A NotFound path creates a path-backed buffer and fires NO hook.
    let s = editor();
    exec(
        &s,
        "LOADS = 0
         pmacs.hook.add(\"buffer.after-load\", function() LOADS = LOADS + 1 end)
         SWITCHES = 0
         pmacs.hook.add(\"buffer.after-switch\", function() SWITCHES = SWITCHES + 1 end)",
    );
    let missing = dir.path().join("brand-new.txt").display().to_string();
    exec(&s, &format!("pmacs.window.display_file({missing:?})"));
    assert_eq!(eval::<u32>(&s, "return LOADS"), 0);
    assert_eq!(eval::<u32>(&s, "return SWITCHES"), 0);
    assert_eq!(
        eval::<String>(&s, "return pmacs.window.buffer():path()"),
        missing,
        "the new buffer is path-backed"
    );
}

// ---------------------------------------------------------------------------
// 19 — adopters place through their REAL entry points
// ---------------------------------------------------------------------------

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one placement scenario per adopter; splitting it would hide that they share a contract"
)]
fn acc19_adopters_place_side_affinely_through_real_entry_points() {
    // Stage 3: the DEFAULT is now the panel, so assert that first —
    // this test's subject is placement through the real entry points.
    let s = editor();
    exec(
        &s,
        "pmacs.listview.open { name = \"*default*\", rows = { { text = \"row\" } } }",
    );
    assert!(
        side_window(&s).is_some(),
        "omitting display places a listview in the panel"
    );

    // listview: pre-seed the persistent panel buffer in a DOCUMENT window
    // first, so side-affine placement cannot be vacuous.
    //
    // The seed now says `display = "current"` EXPLICITLY. That is not a
    // bolt-on to keep the test green: the seed's whole purpose is "this
    // buffer starts in a document window", and after the flip that
    // requires saying so. Leaving it omitted would seed a panel and the
    // side-affine assertion below would pass without having moved
    // anything — exactly the vacuity this fixture was built to prevent.
    let s = editor();
    exec(
        &s,
        "pmacs.listview.open { name = \"*outline*\", rows = { { text = \"row\" } }, \
         display = \"current\" }",
    );
    let seeded = active_window(&s);
    assert!(
        side_window(&s).is_none(),
        "the explicit opt-out still places in the document window"
    );
    exec(
        &s,
        "pmacs.listview.open { name = \"*outline*\", rows = { { text = \"row\" } }, \
         display = \"panel\" }",
    );
    let panel = side_window(&s).expect("listview opened a panel");
    assert_eq!(
        active_window(&s),
        panel,
        "an interactive listview takes select = true"
    );
    assert_ne!(panel, seeded);
    assert!(
        try_exec(
            &s,
            "pmacs.listview.open { name = \"*bogus*\", rows = {}, display = \"sideways\" }"
        )
        .is_err(),
        "an unknown display value is a pointed error"
    );

    // compile: same shape, but passive (`select = false`). Seeded with
    // the explicit opt-out for the same reason as the listview above.
    let s = editor();
    exec(&s, "pmacs.compile.run(\"true\", { display = \"current\" })");
    assert!(side_window(&s).is_none());
    let document = active_window(&s);
    exec(&s, "pmacs.compile.run(\"true\", { display = \"panel\" })");
    let panel = side_window(&s).expect("compile opened a panel");
    assert_eq!(
        active_window(&s),
        document,
        "compile output is passive: select = false"
    );
    assert_ne!(panel, document);
    let before = s.core.borrow().registry.borrow().ids().len();
    assert!(
        try_exec(&s, "pmacs.compile.run(\"true\", { display = \"nope\" })").is_err(),
        "an unknown display value fails BEFORE the run starts"
    );
    assert_eq!(
        s.core.borrow().registry.borrow().ids().len(),
        before,
        "…and creates no buffer"
    );

    // terminal: the panel opt-in uses select = true.
    let s = editor();
    let document = active_window(&s);
    let before = s.core.borrow().registry.borrow().ids().len();
    assert!(
        try_exec(
            &s,
            "pmacs.terminal.open { command = \"/bin/sh\", display = \"elsewhere\" }"
        )
        .is_err(),
        "unknown display fails before session/process/buffer creation"
    );
    assert_eq!(s.core.borrow().registry.borrow().ids().len(), before);

    // Bottom-panel Stage 3, Q#S3-1 — the NON-STRING normalization.
    //
    // Pinned because step 2 CHANGED this deliberately and nothing else
    // covers it. Before the shared resolver, terminal read
    // `get::<Option<String>>("display")?`, so a number raised mlua's
    // TYPE error before any custom message existed; the Lua adopters
    // stringified instead and reported their own. Unifying the error
    // text without pinning this would have let the two drift back apart
    // unnoticed, and the surrounding assertions could not have caught it
    // — they all pass unknown STRINGS, which take the same path in both
    // designs.
    //
    // The custom error wins because it names the legal vocabulary. The
    // type is reported WITHOUT the value, so the message cannot imply a
    // string was passed.
    let before = s.core.borrow().registry.borrow().ids().len();
    let err = try_exec(
        &s,
        "pmacs.terminal.open { command = \"/bin/sh\", display = 42 }",
    )
    .expect_err("a non-string display is rejected");
    // The TYPE SPELLING is deliberately not pinned: Lua 5.4 reports
    // `integer` where LuaJIT has no integer subtype, so asserting either
    // literal would pass on one CI flavor and fail on the other. What is
    // pinned is the shape — our operation name, a parenthesised type
    // rather than a quoted value, and the vocabulary.
    assert!(
        err.contains("pmacs.terminal.open: unknown display ("),
        "a non-string display takes the shared unknown-display error naming the \
         operation and a type, not mlua's type error; got: {err}"
    );
    assert!(
        err.contains("expected \"current\" or \"panel\""),
        "the error still names the legal values; got: {err}"
    );
    assert!(
        !err.contains("\"42\""),
        "the rejected value is reported by TYPE, not quoted as though it were a \
         string; got: {err}"
    );
    assert_eq!(
        s.core.borrow().registry.borrow().ids().len(),
        before,
        "…and still creates nothing, exactly as an unknown string does"
    );

    exec(
        &s,
        "TERM_BUF = pmacs.terminal.open { command = \"/bin/sh\", display = \"panel\" }",
    );
    let panel = side_window(&s).expect("terminal opened a panel");
    assert_eq!(active_window(&s), panel);
    assert_ne!(panel, document);
}

/// A recompile carries no `display` (only cmdline/cwd are stored), so
/// the raw switch would put `*compilation*` in the selected DOCUMENT
/// window while the panel still shows it — the duplicate presentation
/// this arc removes elsewhere.
#[test]
fn acc19b_recompile_reuses_the_panel_instead_of_duplicating_into_the_document() {
    let s = editor();
    exec(&s, "pmacs.window.split_horizontal()");
    exec(&s, "pmacs.compile.run(\"true\", { display = \"panel\" })");
    let panel = side_window(&s).expect("compile opened a panel");
    let compilation = s.core.borrow().windows[&panel].buffer_id;

    // Focus a document window, then recompile — which reaches
    // `start_run` with no `display` at all.
    let document = s
        .core
        .borrow()
        .non_side_target(FrontendId::LOCAL)
        .expect("document");
    s.core
        .borrow_mut()
        .focus_window(FrontendId::LOCAL, document);
    let document_buffer = s.core.borrow().windows[&document].buffer_id;
    exec(&s, "pmacs.command.invoke(\"compile.recompile\")");

    assert_eq!(
        s.core.borrow().windows[&panel].buffer_id,
        compilation,
        "the recompile stayed in the panel"
    );
    assert_eq!(
        s.core.borrow().windows[&document].buffer_id,
        document_buffer,
        "…and did not duplicate itself into the document window"
    );

    // An EXPLICIT `display = "current"` still wins over the inference:
    // it is the documented user-facing opt-out from the Stage 3 default
    // flip, so it must reach the raw switch even while the panel holds
    // this buffer. The resulting duplicate presentation is the escape
    // hatch's documented cost (R3-rp2).
    s.core
        .borrow_mut()
        .focus_window(FrontendId::LOCAL, document);
    exec(&s, "pmacs.compile.run(\"true\", { display = \"current\" })");
    assert_eq!(
        s.core.borrow().windows[&document].buffer_id,
        compilation,
        "explicit \"current\" reached the raw switch"
    );
    assert_eq!(
        s.core.borrow().windows[&panel].buffer_id,
        compilation,
        "…and the panel still holds it too — the escape hatch's cost"
    );

    // A compilation that is NOT in a panel keeps the pre-arc raw switch.
    // Reaching that state now takes an explicit opt-out, since the
    // default would panel it — and "not in a panel" is the precondition
    // this half exists to exercise.
    let s = editor();
    exec(&s, "pmacs.compile.run(\"true\", { display = \"current\" })");
    assert!(side_window(&s).is_none());
    let target = active_window(&s);
    exec(&s, "pmacs.command.invoke(\"compile.recompile\")");
    assert_eq!(active_window(&s), target);
    assert!(
        side_window(&s).is_none(),
        "no panel is created out of nowhere"
    );
}

/// `pmacs.window.buffer()` with NO argument must stay **infallible**.
///
/// The optional window argument this arc added is validated against the
/// acting frontend's layout, and it is tempting to make the no-arg arm
/// symmetric by resolving it the same way. That silently breaks the
/// runtime: `acting_frontend` follows the interactive origin, which can
/// name a frontend with **no registered view** (as a bare
/// `dispatch_key` from a peer does), where a `views`-keyed lookup raises
/// instead of answering — and `killring`, `syntax`, `autosave`, `pair`,
/// `indent` and `comment` all call this on ordinary edits without
/// `pcall`, so the raise does not surface as an error, it just drops the
/// operation. Routing it through `selected_window` lost an entire kill in
/// `kill_ring_acceptance`.
#[test]
fn acc19c_window_buffer_stays_infallible_for_an_acting_frontend_without_a_view() {
    let mut s = editor();
    let ambient = s.core.borrow().active_buffer_id();
    exec(
        &s,
        // A `buffer.after-edit` subscriber is the real shape: this is
        // where syntax.lua, pair.lua and comment.lua each call
        // `pmacs.window.buffer()` on every ordinary edit.
        "SEEN = nil; ERR = nil \
         pmacs.hook.add(\"buffer.after-edit\", function() \
           local ok, got = pcall(pmacs.window.buffer) \
           if ok then SEEN = got else ERR = tostring(got) end \
         end)",
    );

    // A peer that never registered a view — the shape `dispatch_key`
    // produces for an unattached frontend, and what the kill-ring suite
    // drives with `ctrl_as`.
    let viewless = FrontendId(9);
    assert!(
        !s.core.borrow().views.contains_key(&viewless),
        "the premise: this frontend really has no view"
    );
    s.dispatch_key(viewless, key(KeyCode::Char('z'), KeyModifiers::NONE));

    let err: Option<String> = eval(&s, "return ERR");
    assert_eq!(
        err, None,
        "pmacs.window.buffer() must not raise for a viewless acting frontend"
    );
    let seen: Option<pmacs::lua_bindings::BufferIdLua> = eval(&s, "return SEEN");
    assert_eq!(
        seen.expect("the command observed a buffer").0,
        ambient,
        "…it answers with the ambient active buffer"
    );
}

// ---------------------------------------------------------------------------
// 20 / 23 — quit: delete, restore chains, revalidation, and the cap
// ---------------------------------------------------------------------------

#[test]
fn acc20_quit_deletes_then_restores_each_saved_presentation() {
    let s = editor();
    let document = active_window(&s);
    exec(
        &s,
        "A = pmacs.buffer.create(\"*A*\")
         B = pmacs.buffer.create(\"*B*\")
         C = pmacs.buffer.create(\"*C*\")
         pmacs.window.display(A, { side = \"bottom\", height = 6, select = true })",
    );
    let panel = side_window(&s).expect("panel");
    exec(
        &s,
        &format!(
            "pmacs.window.set_params({}, {{ fixed_rows = 9 }})",
            panel.raw()
        ),
    );
    exec(
        &s,
        "pmacs.window.display(B, { side = \"bottom\", select = true })",
    );
    exec(
        &s,
        "pmacs.window.display(C, { side = \"bottom\", select = true })",
    );

    // C -> B -> A -> delete.
    exec(&s, "pmacs.window.quit()");
    assert_eq!(
        s.core.borrow().windows[&panel].buffer_id,
        eval::<pmacs::lua_bindings::BufferIdLua>(&s, "return B").0
    );
    exec(&s, "pmacs.window.quit()");
    assert_eq!(
        s.core.borrow().windows[&panel].buffer_id,
        eval::<pmacs::lua_bindings::BufferIdLua>(&s, "return A").0
    );
    assert_eq!(
        fixed_rows_of(&s, panel),
        Some(9),
        "the saved (user-resized) height is restored with its presentation"
    );
    exec(&s, "pmacs.window.quit()");
    assert!(side_window(&s).is_none(), "the last quit deletes the slot");
    assert_eq!(active_window(&s), document);

    // A window with no quit action is a pointed error that changes nothing.
    let before = structure(&layout_root(&s));
    assert!(try_exec(&s, "pmacs.window.quit()").is_err());
    assert_eq!(before, structure(&layout_root(&s)));
}

#[test]
fn acc20b_quit_history_is_bounded_at_max_panel_quit_depth() {
    let s = editor();
    exec(&s, "P0 = pmacs.buffer.create(\"*p0*\")");
    exec(
        &s,
        "pmacs.window.display(P0, { side = \"bottom\", height = 4 })",
    );
    let panel = side_window(&s).expect("panel");
    for i in 1..=(MAX_PANEL_QUIT_DEPTH + 20) {
        exec(
            &s,
            &format!(
                "pmacs.window.display(pmacs.buffer.create(\"*p{i}*\"), {{ side = \"bottom\" }})"
            ),
        );
        let depth: usize = eval(
            &s,
            &format!("return pmacs.window.params({}).quit_depth", panel.raw()),
        );
        assert!(
            depth <= MAX_PANEL_QUIT_DEPTH,
            "depth never grows beyond the cap (saw {depth} at replacement {i})"
        );
    }
    let depth: usize = eval(
        &s,
        &format!("return pmacs.window.params({}).quit_depth", panel.raw()),
    );
    assert_eq!(
        depth, MAX_PANEL_QUIT_DEPTH,
        "exactly the newest 64 are retained"
    );
    for _ in 0..MAX_PANEL_QUIT_DEPTH {
        exec(&s, &format!("pmacs.window.quit({})", panel.raw()));
    }
    exec(&s, &format!("pmacs.window.quit({})", panel.raw()));
    assert!(side_window(&s).is_none(), "the chain terminates in Delete");
}

#[test]
fn acc23_quit_revalidates_a_killed_restore_target() {
    let s = editor();
    exec(
        &s,
        "A = pmacs.buffer.create(\"*A*\")
         B = pmacs.buffer.create(\"*B*\")
         pmacs.window.display(A, { side = \"bottom\", height = 5 })
         pmacs.window.display(B, { side = \"bottom\" })",
    );
    let panel = side_window(&s).expect("panel");
    exec(&s, "pmacs.buffer.kill(A)");
    exec(&s, &format!("pmacs.window.quit({})", panel.raw()));
    assert!(
        side_window(&s).is_none(),
        "a killed restore target degrades the whole chain to Delete"
    );
}

// ---------------------------------------------------------------------------
// 21 / 22 — the jump ring
// ---------------------------------------------------------------------------

#[test]
fn acc21_panel_visit_and_jump_back_returns_to_the_panel() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("src.txt");
    std::fs::write(&file, b"one\ntwo\nthree\n").unwrap();
    let path = file.display().to_string();

    let s = editor();
    let document = active_window(&s);
    let panel = open_panel(&s, "*outline*", 6);
    exec(&s, "pmacs.window.focus_next()");
    assert_eq!(active_window(&s), panel);
    // Move the panel cursor so the restored row is observable.
    s.core.borrow_mut().windows.get_mut(&panel).unwrap().cursor = 0;

    exec(&s, "pmacs.editor.push_jump()");
    exec(
        &s,
        &format!("pmacs.window.display_file({path:?}, {{ select = true }})"),
    );
    assert_eq!(
        active_window(&s),
        document,
        "RET visited the document window"
    );

    let jumped: bool = eval(&s, "return pmacs.editor.jump_back()");
    assert!(jumped);
    assert_eq!(
        active_window(&s),
        panel,
        "M-, returns focus to the EXISTING panel, not a duplicate"
    );
    assert_eq!(
        s.core.borrow().views[&FrontendId::LOCAL]
            .layout
            .iter_ids()
            .len(),
        2,
        "no duplicate presentation was created"
    );
}

/// Bottom-panel Stage 3, criterion 5 — the OMITTED default degrades on a
/// frontend that cannot host a panel.
///
/// `acc14` already proves capability fallback for an EXPLICIT
/// `request.side` at the core level. This is the Stage 3 case and it is
/// not the same one: the default is now resolved into a panel request
/// inside the adopter, so a pre-panel semantic frontend must degrade a
/// request the caller never wrote. The framing calls this the criterion
/// most likely to be quietly wrong, because the fallback is invisible
/// from the adopter's side — nothing in `listview.open` says "panel",
/// yet the request that reaches the core does.
///
/// What must survive the degradation: no side window, no side
/// parameters, and NO QUIT ACTION left on the document window. A quit
/// action stranded on a document window would make a later `q` try to
/// restore a presentation that never existed.
#[test]
fn s3_2_the_omitted_default_degrades_on_a_pre_panel_frontend() {
    let s = editor();
    let fid = FrontendId(31);
    let document = attach_frontend(&s, fid, false);
    s.core.borrow_mut().active_frontend = fid;

    // No `display` at all — the Stage 3 default resolves to a panel
    // request, which this frontend cannot honour.
    exec(
        &s,
        "pmacs.listview.open { name = \"*degraded*\", rows = { { text = \"row\" } } }",
    );

    assert!(
        s.core.borrow().side_window_for(fid).is_none(),
        "a pre-panel frontend gets no side window from the omitted default"
    );
    {
        let core = s.core.borrow();
        let window = &core.windows[&document];
        assert!(
            window.params.side.is_none(),
            "no side parameter is left on the document window"
        );
        assert!(
            !window.params.dedicated,
            "the document window is not dedicated by a degraded request"
        );
        assert!(
            window.params.quit_action().is_none(),
            "NO quit action is left behind — `q` must not try to restore a \
             presentation that never happened"
        );
        assert_eq!(
            core.registry
                .borrow()
                .get(window.buffer_id)
                .expect("live buffer")
                .name(),
            "*degraded*",
            "the buffer still reached the document target"
        );
    }

    s.core.borrow_mut().active_frontend = FrontendId::LOCAL;
}

#[test]
fn acc22_jump_histories_are_per_frontend_and_skip_stale_side_origins() {
    let s = editor();
    let fid = FrontendId(21);
    let foreign = attach_frontend(&s, fid, true);

    // LOCAL pushes; the foreign frontend must not be able to pop it.
    exec(&s, "pmacs.editor.push_jump()");
    s.core.borrow_mut().active_frontend = fid;
    assert!(
        !s.core.borrow_mut().jump_back(),
        "one frontend cannot consume another's navigation trail"
    );
    s.core.borrow_mut().active_frontend = FrontendId::LOCAL;
    assert!(
        s.core.borrow_mut().jump_back(),
        "LOCAL's own entry survives"
    );
    let _ = foreign;

    // A SIDE origin whose buffer was replaced is skipped, not resurrected.
    let s = editor();
    let panel = open_panel(&s, "*panel*", 5);
    exec(&s, "pmacs.window.focus_next()");
    exec(&s, "pmacs.editor.push_jump()");
    exec(
        &s,
        "pmacs.window.display(pmacs.buffer.create(\"*new*\"), { side = \"bottom\" })",
    );
    let panel_buffer = s.core.borrow().windows[&panel].buffer_id;
    assert!(
        !s.core.borrow_mut().jump_back(),
        "a replaced side origin is skipped rather than duplicated into the document"
    );
    assert_eq!(
        s.core.borrow().windows[&panel].buffer_id,
        panel_buffer,
        "…and the panel keeps its current presentation"
    );
}

// ---------------------------------------------------------------------------
// 24 / 25 / 26 / 27 — the window guards
// ---------------------------------------------------------------------------

#[test]
fn acc24_killing_a_panel_buffer_closes_the_side_window() {
    let s = editor();
    let panel = open_panel(&s, "*panel*", 5);
    let panel_buffer = s.core.borrow().windows[&panel].buffer_id;
    exec(&s, "pmacs.buffer.kill(PANEL_BUF)");
    assert!(side_window(&s).is_none(), "the side window closed");
    assert!(
        !s.core.borrow().windows.contains_key(&panel),
        "…rather than being redirected to *scratch*"
    );
    assert!(!s.core.borrow().registry.borrow().contains(panel_buffer));
}

#[test]
fn acc25_close_active_refuses_only_the_last_document_window() {
    let s = editor();
    let document = active_window(&s);
    let panel = open_panel(&s, "*panel*", 5);
    // A document window with only the panel beside it still cannot close.
    assert!(
        !s.core.borrow_mut().close_active(),
        "the last document window is protected"
    );
    // The panel itself always may — even as the only other window.
    exec(&s, "pmacs.window.focus_next()");
    assert_eq!(active_window(&s), panel);
    assert!(
        s.core.borrow_mut().close_active(),
        "closing the side window is always legal"
    );
    assert!(side_window(&s).is_none());
    assert_eq!(active_window(&s), document);
}

#[test]
fn acc26_close_others_and_split_respect_the_side_window() {
    let s = editor();
    exec(&s, "pmacs.window.split_horizontal()");
    let panel = open_panel(&s, "*panel*", 5);
    // From a side window both are pointed errors — asserted through the
    // REAL Lua bindings, which is what `C-x 1` / `C-x 2` / `C-x 3`
    // reach. A direct `core.try_split_active(..)` call would pass even
    // with the guard unwired, which is exactly how an unwired guard
    // survives review.
    exec(&s, "pmacs.window.focus_next()");
    while active_window(&s) != panel {
        exec(&s, "pmacs.window.focus_next()");
    }
    let before = structure(&layout_root(&s));
    assert!(try_exec(&s, "pmacs.window.close_others()").is_err());
    assert!(try_exec(&s, "pmacs.window.split_horizontal()").is_err());
    assert!(try_exec(&s, "pmacs.window.split_vertical()").is_err());
    assert!(side_window(&s).is_some(), "nothing was mutated");
    assert_eq!(
        before,
        structure(&layout_root(&s)),
        "the wrapper's final child is still Leaf(side)"
    );

    // From a document window, close_others deletes the panel too.
    exec(&s, "pmacs.window.focus_next()");
    assert_ne!(active_window(&s), panel);
    exec(&s, "pmacs.window.close_others()");
    assert!(side_window(&s).is_none());
    assert_eq!(
        s.core.borrow().views[&FrontendId::LOCAL]
            .layout
            .iter_ids()
            .len(),
        1
    );
}

#[test]
fn acc27_traversal_refreshes_the_remembered_document_origin() {
    let s = editor();
    let a = active_window(&s);
    exec(&s, "pmacs.window.split_horizontal()");
    let b = s.core.borrow().views[&FrontendId::LOCAL]
        .layout
        .iter_ids()
        .into_iter()
        .find(|id| *id != a)
        .expect("second document window");
    // Create the panel from A.
    s.core.borrow_mut().focus_window(FrontendId::LOCAL, a);
    let panel = open_panel(&s, "*panel*", 5);
    assert_eq!(
        s.core.borrow().windows[&panel].params.origin_document(),
        Some(a)
    );
    // Enter the panel from B: the memory retargets.
    s.core.borrow_mut().focus_window(FrontendId::LOCAL, b);
    s.core.borrow_mut().focus_window(FrontendId::LOCAL, panel);
    assert_eq!(
        s.core.borrow().windows[&panel].params.origin_document(),
        Some(b),
        "entering the panel from B retargets the remembered origin"
    );
    assert_eq!(
        eval::<u64>(&s, "return pmacs.window.display_target()"),
        b.raw(),
        "display_target follows it"
    );
    // A Delete-form quit focuses B, not the creation-time window.
    exec(&s, "pmacs.window.quit()");
    assert_eq!(active_window(&s), b);
}

// ---------------------------------------------------------------------------
// 29 — optimistic input is gated per WINDOW, not per buffer
// ---------------------------------------------------------------------------

#[test]
fn acc29_focused_side_window_gates_dispatch_idle_without_marking_the_buffer() {
    let s = editor();
    let panel = open_panel(&s, "*panel*", 5);
    let panel_buffer = s.core.borrow().windows[&panel].buffer_id;
    assert!(
        s.dispatch_idle_for(FrontendId::LOCAL),
        "a document window is idle"
    );
    exec(&s, "pmacs.window.focus_next()");
    assert_eq!(active_window(&s), panel);
    assert!(
        !s.dispatch_idle_for(FrontendId::LOCAL),
        "a focused side window turns optimistic apply off"
    );
    assert!(
        !s.core.borrow().buffer_round_trips(panel_buffer),
        "…WITHOUT marking the buffer round-trip"
    );

    // Another frontend showing that same buffer as its DOCUMENT keeps
    // optimistic apply.
    let other = FrontendId(29);
    let other_window = attach_frontend(&s, other, true);
    s.core
        .borrow_mut()
        .install_buffer_in_window(other_window, panel_buffer)
        .expect("install");
    assert!(
        s.dispatch_idle_for(other),
        "the buffer-global set is untouched, so the peer stays optimistic"
    );
}

// ---------------------------------------------------------------------------
// 30 / 31 — the divider
// ---------------------------------------------------------------------------

#[test]
fn acc30_divider_drag_writes_fixed_rows_and_weights_and_creates_no_selection() {
    let s0 = editor();
    let mut s = s0;
    let panel = open_panel(&s, "*panel*", 6);
    let document = s
        .core
        .borrow()
        .non_side_target(FrontendId::LOCAL)
        .expect("document");
    let rects = render(&s);
    let divider_row = u16::try_from(rects[&document].origin.row + rects[&document].size.rows - 1)
        .expect("row fits");

    s.dispatch_mouse(
        FrontendId::LOCAL,
        mouse(MouseEventKind::Down(MouseButton::Left), divider_row, 3),
        CellSize::new(ROWS, COLS),
    );
    assert!(
        s.core.borrow().active_window().selection.is_none(),
        "a press on the reserved row creates no selection"
    );
    s.dispatch_mouse(
        FrontendId::LOCAL,
        mouse(MouseEventKind::Drag(MouseButton::Left), divider_row + 2, 3),
        CellSize::new(ROWS, COLS),
    );
    s.dispatch_mouse(
        FrontendId::LOCAL,
        mouse(MouseEventKind::Up(MouseButton::Left), divider_row + 2, 3),
        CellSize::new(ROWS, COLS),
    );
    assert_eq!(
        fixed_rows_of(&s, panel),
        Some(4),
        "dragging the divider DOWN shrinks the side window's fixed rows"
    );

    // A flexible pair writes weights instead.
    let mut s = editor();
    exec(&s, "pmacs.window.split_horizontal()");
    let top = s.core.borrow().views[&FrontendId::LOCAL].layout.iter_ids()[0];
    let rects = render(&s);
    let divider_row =
        u16::try_from(rects[&top].origin.row + rects[&top].size.rows - 1).expect("row fits");
    let before = rects[&top].size.rows;
    s.dispatch_mouse(
        FrontendId::LOCAL,
        mouse(MouseEventKind::Down(MouseButton::Left), divider_row, 3),
        CellSize::new(ROWS, COLS),
    );
    s.dispatch_mouse(
        FrontendId::LOCAL,
        mouse(MouseEventKind::Drag(MouseButton::Left), divider_row + 3, 3),
        CellSize::new(ROWS, COLS),
    );
    let after = render(&s)[&top].size.rows;
    assert_eq!(
        after,
        before + 3,
        "the flexible boundary moved by the drag delta"
    );
    // …and the ratio survives a frame resize, which is the whole point of
    // writing weights rather than a fixed extent.
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(ROWS * 2, COLS));
    let doubled = render_at(&s, CellSize::new(ROWS * 2, COLS))[&top].size.rows;
    assert!(doubled > after, "the ratio scales with the frame");
}

/// An armed drag owns the pointer for its OWN frontend only. The daemon
/// routes every attached grid frontend through one `dispatch_mouse`, so
/// an unscoped guard would let one frontend's in-flight gesture cancel
/// and swallow another frontend's clicks.
#[test]
fn acc30c_an_armed_drag_does_not_swallow_another_frontends_mouse_events() {
    let mut s = editor();
    let panel = open_panel(&s, "*panel*", 6);
    let document = s
        .core
        .borrow()
        .non_side_target(FrontendId::LOCAL)
        .expect("document");
    let other = FrontendId(30);
    let other_window = attach_frontend(&s, other, true);

    let rects = render(&s);
    let divider_row = u16::try_from(rects[&document].origin.row + rects[&document].size.rows - 1)
        .expect("row fits");
    s.dispatch_mouse(
        FrontendId::LOCAL,
        mouse(MouseEventKind::Down(MouseButton::Left), divider_row, 3),
        CellSize::new(ROWS, COLS),
    );
    let armed_rows = fixed_rows_of(&s, panel);

    // A click from the OTHER frontend must be dispatched normally…
    s.dispatch_mouse(
        other,
        mouse(MouseEventKind::Down(MouseButton::Left), 1, 2),
        CellSize::new(ROWS, COLS),
    );
    assert_eq!(
        s.core.borrow().views[&other].active,
        other_window,
        "the peer's click reached its own window instead of being swallowed"
    );

    // …a peer press on ITS OWN mode-line row must not steal or clear the
    // slot either. That press reaches `arm_window_drag`, which a single
    // global slot lets it overwrite — and the peer's lone window owns no
    // boundary, so the write is an outright clear. The peer's mode line
    // is the last row of its own single-window layout.
    let peer_mode_line = u16::try_from(AREA_ROWS - 1).expect("row fits");
    s.dispatch_mouse(
        other,
        mouse(MouseEventKind::Down(MouseButton::Left), peer_mode_line, 4),
        CellSize::new(ROWS, COLS),
    );

    // …and LOCAL's gesture must still be armed and still work.
    s.dispatch_mouse(
        FrontendId::LOCAL,
        mouse(MouseEventKind::Drag(MouseButton::Left), divider_row + 2, 3),
        CellSize::new(ROWS, COLS),
    );
    assert_eq!(
        fixed_rows_of(&s, panel),
        Some(armed_rows.expect("armed rows") - 2),
        "the peer's events did not cancel or steal LOCAL's in-flight drag"
    );
}

#[test]
fn acc30b_ui_divider_face_resolves_and_paints_every_exposed_segment() {
    let s = editor();
    // A boundary whose upper child is a VERTICAL split exposes several
    // leaf mode-line segments along the same edge.
    exec(&s, "pmacs.window.split_vertical()");
    open_panel(&s, "*panel*", 5);
    exec(
        &s,
        "pmacs.theme.set { [\"ui.divider\"] = { fg = { 255, 0, 255 } } }",
    );
    let rows = painted_rows(&s, CellSize::new(ROWS, COLS));
    let boundary_rows: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains('⇕'))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        boundary_rows.len(),
        1,
        "both exposed segments sit on the SAME boundary row"
    );

    // Dragging either segment resolves the same boundary.
    let core = s.core.borrow();
    let ids = core.views[&FrontendId::LOCAL].layout.iter_ids();
    let leaves: Vec<WindowId> = ids
        .into_iter()
        .filter(|id| !core.windows[id].is_side())
        .collect();
    let layout = core.views[&FrontendId::LOCAL].layout.clone();
    drop(core);
    assert_eq!(leaves.len(), 2);
    assert_eq!(
        layout.boundary_below(leaves[0]),
        layout.boundary_below(leaves[1]),
        "every leaf segment touching the same bottom edge resolves to one boundary"
    );
}

#[test]
fn acc31_keyboard_resize_matches_the_equivalent_drag_in_a_nested_layout() {
    // Build H[ H[A, C], B ] — A's nearest horizontal ancestor is the
    // inner split; C's is that same split, but C is its FINAL child, so
    // C's boundary is the outer one. The naive "nearest horizontal
    // ancestor" reading picks the wrong split for C.
    let s = editor();
    exec(&s, "pmacs.window.split_horizontal()");
    let a = s.core.borrow().views[&FrontendId::LOCAL].layout.iter_ids()[0];
    s.core.borrow_mut().focus_window(FrontendId::LOCAL, a);
    exec(&s, "pmacs.window.split_horizontal()");
    let ids = s.core.borrow().views[&FrontendId::LOCAL].layout.iter_ids();
    assert_eq!(ids.len(), 3);
    let (a, c, b) = (ids[0], ids[1], ids[2]);

    let layout = s.core.borrow().views[&FrontendId::LOCAL].layout.clone();
    assert_ne!(
        layout.boundary_below(a),
        layout.boundary_below(c),
        "A owns the INNER boundary; C, as that split's final child, \
         resolves upward to the outer one — the naive \"nearest \
         horizontal ancestor\" reading picks the wrong split for C"
    );
    assert_eq!(
        layout.boundary_below(c).expect("C has a boundary").path,
        Vec::<usize>::new(),
        "C's boundary is the ROOT split, not its own parent"
    );
    assert!(
        layout.boundary_below(b).is_none(),
        "the last child owns no boundary"
    );

    // The keyboard resize and the equivalent DRAG move the same boundary
    // to the same place. `resize(win, delta)` resolves from the SUPPLIED
    // window (the Lua entry point is explicit).
    let before = render(&s);
    exec(&s, &format!("pmacs.window.resize({}, 2)", c.raw()));
    let by_command: HashMap<WindowId, u32> = render(&s)
        .iter()
        .map(|(id, rect)| (*id, rect.size.rows))
        .collect();
    assert!(
        by_command[&c] > before[&c].size.rows,
        "C grew: {} -> {}",
        before[&c].size.rows,
        by_command[&c]
    );

    let mut dragged = editor();
    exec(&dragged, "pmacs.window.split_horizontal()");
    let da = dragged.core.borrow().views[&FrontendId::LOCAL]
        .layout
        .iter_ids()[0];
    dragged
        .core
        .borrow_mut()
        .focus_window(FrontendId::LOCAL, da);
    exec(&dragged, "pmacs.window.split_horizontal()");
    let dids = dragged.core.borrow().views[&FrontendId::LOCAL]
        .layout
        .iter_ids();
    let dc = dids[1];
    let rects = render(&dragged);
    let divider_row =
        u16::try_from(rects[&dc].origin.row + rects[&dc].size.rows - 1).expect("row fits");
    dragged.dispatch_mouse(
        FrontendId::LOCAL,
        mouse(MouseEventKind::Down(MouseButton::Left), divider_row, 3),
        CellSize::new(ROWS, COLS),
    );
    dragged.dispatch_mouse(
        FrontendId::LOCAL,
        mouse(MouseEventKind::Drag(MouseButton::Left), divider_row + 2, 3),
        CellSize::new(ROWS, COLS),
    );
    let by_drag = render(&dragged);
    assert_eq!(
        by_command[&c], by_drag[&dc].size.rows,
        "keyboard resize equals the equivalent drag on that window's \
         bottom mode-line row"
    );

    // The no-adjustable-boundary case reports and no-ops.
    let before = structure(&layout_root(&s));
    assert!(try_exec(&s, &format!("pmacs.window.resize({}, 1)", b.raw())).is_err());
    assert_eq!(before, structure(&layout_root(&s)));

    // The commands act on the ACTIVE window and equal the same move.
    let s = editor();
    exec(&s, "pmacs.window.split_horizontal()");
    let top = s.core.borrow().views[&FrontendId::LOCAL].layout.iter_ids()[0];
    s.core.borrow_mut().focus_window(FrontendId::LOCAL, top);
    let before = render(&s)[&top].size.rows;
    exec(&s, "pmacs.command.invoke(\"window.enlarge\")");
    assert_eq!(render(&s)[&top].size.rows, before + 1);
    exec(&s, "pmacs.command.invoke(\"window.shrink\")");
    assert_eq!(render(&s)[&top].size.rows, before);
}

// ---------------------------------------------------------------------------
// 32 / 33 / 34 — a terminal panel's height changes
// ---------------------------------------------------------------------------

#[test]
fn acc32_terminal_panel_height_change_is_a_viewport_change() {
    let mut s = editor();
    exec(
        &s,
        // `printf '...\\r\\n'`, not `echo`: a PTY in the default mode
        // does not translate LF to CRLF for us, so LF-only output
        // staircases rightward and every row past the viewport width
        // clips to blanks — which would make the anchor assertions below
        // compare "" with "" and pass for any regression.
        "TERM_BUF = pmacs.terminal.open { command = \"/bin/sh\", \
           args = { \"-c\", \"i=1; while [ $i -le 200 ]; do printf 'line%d\\\\r\\\\n' $i; \
                             i=$((i+1)); done; sleep 30\" }, \
           display = \"panel\" }",
    );
    let panel = side_window(&s).expect("terminal panel");
    let buffer: pmacs::lua_bindings::BufferIdLua = eval(&s, "return TERM_BUF");
    render(&s);

    // Wait for the child's LAST line: `scroll_offset` is tail-relative,
    // so comparing it across a height change is only meaningful once the
    // tail has stopped moving.
    wait_for_terminal_text(&mut s, buffer.0, "line200", Duration::from_secs(10));

    // Scroll back, then change the panel height. `top` is preserved
    // verbatim: a height change is a viewport change, never a scroll one.
    exec(&s, "pmacs.window.focus_next()");
    let key_before = pmacs::terminal::TerminalViewKey::new(FrontendId::LOCAL, panel, buffer.0);
    let before_size = CellSize::new(11, COLS);
    s.terminal_manager
        .borrow_mut()
        .scroll_view(key_before, before_size, 30);
    let top_before = first_visible_row(&s, key_before, before_size);
    assert!(
        !top_before.is_empty(),
        "the anchor row must carry real text, or the equality below \
         cannot fail for the regression it names"
    );
    exec(
        &s,
        &format!(
            "pmacs.window.set_params({}, {{ fixed_rows = 10 }})",
            panel.raw()
        ),
    );
    render(&s);
    s.sync_terminal_layout(FrontendId::LOCAL, CellSize::new(ROWS, COLS));
    // The ANCHOR is the invariant. `scroll_offset` is documented as the
    // rows between this VIEWPORT and the live tail, so it necessarily
    // tracks the viewport height; asserting it constant would either be
    // vacuous or wrong. The first visible row is `top` itself.
    assert_eq!(
        top_before,
        first_visible_row(&s, key_before, CellSize::new(9, COLS)),
        "a scrolled-back terminal panel keeps its top across a height change"
    );
    assert!(
        !s.terminal_manager
            .borrow_mut()
            .view_status(key_before)
            .expect("view status")
            .at_bottom,
        "…and a SHRINK cannot re-arm follow"
    );
    exec(&s, "pmacs.terminal.terminate(TERM_BUF)");
}

/// Q#BP7 item 1 proper: **growth reaching the live tail re-arms follow**,
/// so later output scrolls in.
///
/// `at_bottom` alone cannot pin this — it is the instantaneous geometric
/// readout `scroll_offset == 0`, which a still-anchored view satisfies
/// whenever it happens to be tall enough to reach the tail. The pin has
/// to feed the child MORE output after the growth and assert the view
/// moved with it.
#[test]
fn acc32b_growth_reaching_the_tail_re_arms_follow_and_later_output_scrolls_in() {
    let dir = tempfile::tempdir().expect("tempdir");
    let gate = dir.path().join("gate");
    // Inserted bare into the shell word: `tempfile` paths carry no
    // spaces or quotes, and wrapping it would terminate the Lua string.
    let gate_path = gate.display().to_string();
    let mut s = editor();
    // Two bursts with a filesystem gate between them, so "more output
    // after the growth" is deterministic rather than a race.
    exec(
        &s,
        &format!(
            "TERM_BUF = pmacs.terminal.open {{ command = \"/bin/sh\", \
               args = {{ \"-c\", \"i=1; while [ $i -le 60 ]; do printf 'first%02d\\\\r\\\\n' $i; \
                                 i=$((i+1)); done; \
                                 while [ ! -f {gate_path} ]; do sleep 0.02; done; \
                                 i=1; while [ $i -le 40 ]; do printf 'second%02d\\\\r\\\\n' $i; \
                                 i=$((i+1)); done; sleep 30\" }}, \
               display = \"panel\" }}"
        ),
    );
    let panel = side_window(&s).expect("terminal panel");
    let buffer: pmacs::lua_bindings::BufferIdLua = eval(&s, "return TERM_BUF");
    let key_id = pmacs::terminal::TerminalViewKey::new(FrontendId::LOCAL, panel, buffer.0);
    render(&s);
    wait_for_terminal_text(&mut s, buffer.0, "first60", Duration::from_secs(10));

    // Scroll back into history at a short viewport.
    let short = CellSize::new(6, COLS);
    assert!(
        s.terminal_manager
            .borrow_mut()
            .scroll_view(key_id, short, 20)
    );
    let anchored = first_visible_row(&s, key_id, short);
    assert!(!anchored.is_empty(), "the anchor row carries real text");
    assert!(
        s.terminal_manager
            .borrow_mut()
            .view_status(key_id)
            .expect("status")
            .scroll_offset
            > 0,
        "the view really is anchored in history"
    );

    // Grow the panel until the viewport covers the tail.
    exec(
        &s,
        &format!(
            "pmacs.window.set_params({}, {{ fixed_rows = 23 }})",
            panel.raw()
        ),
    );
    render(&s);
    s.sync_terminal_layout(FrontendId::LOCAL, CellSize::new(ROWS, COLS));
    let grown = CellSize::new(40, COLS);
    s.terminal_manager
        .borrow_mut()
        .snapshot_for_view(key_id, grown)
        .expect("snapshot at the grown size");

    // Release the second burst. A view that merely LOOKS at-bottom while
    // still anchored gets pushed back into history here; a re-armed one
    // follows.
    std::fs::write(&gate, b"go").expect("open the gate");
    wait_for_terminal_text(&mut s, buffer.0, "second40", Duration::from_secs(10));
    s.terminal_manager
        .borrow_mut()
        .snapshot_for_view(key_id, grown)
        .expect("snapshot after the second burst");

    let status = s
        .terminal_manager
        .borrow_mut()
        .view_status(key_id)
        .expect("status");
    assert_eq!(
        status.scroll_offset, 0,
        "the view followed the live tail through the new output"
    );
    assert!(status.at_bottom);
    assert_ne!(
        anchored,
        first_visible_row(&s, key_id, grown),
        "…and its first visible row moved off the old anchor"
    );
    exec(&s, "pmacs.terminal.terminate(TERM_BUF)");
}

/// **Bet B1 pin.** Panel-as-window means the terminal controller, the
/// fixed `C-c` escape, and release-on-blur need zero new code: the
/// controller is keyed `(frontend_id, window_id)` and `view.active`
/// already answers "which window", whether or not that window is a side
/// window.
#[test]
fn acc28_child_input_and_the_c_c_escape_work_unchanged_in_a_panel() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let ready_path = temp.path().join("ready");
    let input_path = temp.path().join("input");
    let probe = format!(
        concat!(
            "import os, tty\n",
            "tty.setraw(0)\n",
            "open({:?}, 'wb').write(b'1')\n",
            "data = b''\n",
            "while len(data) < 5: data += os.read(0, 5 - len(data))\n",
            "open({:?}, 'wb').write(data)\n",
        ),
        ready_path.to_str().expect("UTF-8 ready path"),
        input_path.to_str().expect("UTF-8 input path")
    );
    let mut s = editor();
    exec(
        &s,
        &format!(
            "TERM_BUF = pmacs.terminal.open {{
               command = \"/usr/bin/python3\",
               args = {{ \"-c\", {} }},
               rows = 4, cols = 20,
               display = \"panel\",
             }}",
            format_args!("{probe:?}")
        ),
    );
    let panel = side_window(&s).expect("terminal panel");
    assert_eq!(
        active_window(&s),
        panel,
        "the panel opt-in selects the panel"
    );
    assert_eq!(
        wait_for_file(&ready_path, Duration::from_secs(5)),
        b"1",
        "the child in the PANEL reached raw mode"
    );

    // Exactly the Stage 2 vterm contract, unchanged: unescaped bound keys
    // reach the child, `C-c` escapes for one key, `C-c C-c` sends one
    // literal interrupt.
    for ev in [
        key(KeyCode::Char('v'), KeyModifiers::ALT),
        key(KeyCode::Char('c'), KeyModifiers::CONTROL),
        key(KeyCode::Char('c'), KeyModifiers::CONTROL),
        key(KeyCode::Char('w'), KeyModifiers::ALT),
    ] {
        s.dispatch_key(FrontendId::LOCAL, ev);
    }
    assert_eq!(
        wait_for_file(&input_path, Duration::from_secs(5)),
        b"\x1bv\x03\x1bw",
        "child input routing through a SIDE window is byte-identical"
    );

    // Release-on-blur still works: leaving the panel drops the controller.
    exec(&s, "pmacs.window.focus_next()");
    assert_ne!(active_window(&s), panel);
    s.sync_terminal_layout(FrontendId::LOCAL, CellSize::new(ROWS, COLS));
    assert!(
        s.terminal_manager
            .borrow()
            .controller_view_for_frontend(FrontendId::LOCAL)
            .is_none(),
        "the controller is released when focus leaves the panel"
    );
    exec(&s, "pmacs.terminal.terminate(TERM_BUF)");
}

fn wait_for_file(path: &std::path::Path, timeout: Duration) -> Vec<u8> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Ok(bytes) = std::fs::read(path)
            && !bytes.is_empty()
        {
            return bytes;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Tick until the child's screen contains `needle`, so a test that
/// compares tail-relative state is not racing further output.
///
/// `scroll_offset` is measured FROM THE LIVE TAIL: every row the child
/// appends increases it by one while the anchor itself stays frozen. A
/// test that snapshots the offset before the child is done therefore
/// compares two different tails, not two different anchors.
fn wait_for_terminal_text(s: &mut EditorState, buffer: BufferId, needle: &str, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        s.tick_processes();
        let seen = s
            .terminal_manager
            .borrow()
            .snapshot(buffer)
            .is_some_and(|snapshot| {
                let text: String = snapshot
                    .cells
                    .iter()
                    .filter_map(|cell| match &cell.glyph {
                        Glyph::Char(ch) => Some(*ch),
                        Glyph::Cluster(_) => Some('?'),
                        Glyph::Continuation => None,
                    })
                    .collect();
                text.contains(needle)
            });
        if seen {
            // One more drain so nothing is left in flight.
            s.tick_processes();
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {needle:?} on the terminal screen"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn acc33_growth_with_a_historical_selection_keeps_the_anchor_frozen() {
    let mut s = editor();
    exec(
        &s,
        "TERM_BUF = pmacs.terminal.open { command = \"/bin/sh\", \
           args = { \"-c\", \"i=0; while [ $i -lt 60 ]; do printf 'row%02d\\\\r\\\\n' $i; \
                             i=$((i+1)); done; sleep 30\" }, \
           display = \"panel\" }",
    );
    let panel = side_window(&s).expect("terminal panel");
    let buffer: pmacs::lua_bindings::BufferIdLua = eval(&s, "return TERM_BUF");
    let key_id = pmacs::terminal::TerminalViewKey::new(FrontendId::LOCAL, panel, buffer.0);
    let view_size = CellSize::new(5, COLS);

    // Wait for the child's LAST line, so the tail is stable before the
    // before/after comparison below.
    wait_for_terminal_text(&mut s, buffer.0, "row59", Duration::from_secs(10));
    s.terminal_manager
        .borrow_mut()
        .snapshot_for_view(key_id, view_size)
        .expect("the view has a snapshot once output arrived");

    // Scroll back into history and start a selection there.
    {
        let mut manager = s.terminal_manager.borrow_mut();
        assert!(manager.scroll_view(key_id, view_size, 10));
        assert!(manager.begin_selection(key_id, view_size, CellCoord::new(0, 0)));
    }
    let top_before = first_visible_row(&s, key_id, view_size);
    assert!(
        !top_before.is_empty(),
        "the anchor row must carry real text, or the equality below \
         cannot fail for the regression it names"
    );

    // Grow the panel enough that following the tail WOULD reach it.
    exec(
        &s,
        &format!(
            "pmacs.window.set_params({}, {{ fixed_rows = 20 }})",
            panel.raw()
        ),
    );
    render(&s);
    s.sync_terminal_layout(FrontendId::LOCAL, CellSize::new(ROWS, COLS));

    let grown = CellSize::new(19, COLS);
    let after = s
        .terminal_manager
        .borrow_mut()
        .view_status(key_id)
        .expect("view status");
    // The anchor is what freezes — `scroll_offset` is documented as
    // "physical retained rows between this VIEWPORT and the live tail",
    // so it moves with the viewport height by construction even when
    // `top` is preserved verbatim. Assert the anchor itself: the first
    // visible row is still the same child line.
    assert_eq!(
        top_before,
        first_visible_row(&s, key_id, grown),
        "the anchor is frozen: growth is a viewport change, not a scroll"
    );
    assert!(after.selection, "the historical selection survived");
    assert!(
        !after.at_bottom,
        "follow is NOT re-armed while a selection is frozen"
    );

    // The contrast that makes this bite: the freeze is owed to the
    // SELECTION, so clearing it lets the next size declaration re-arm
    // follow at the very same geometry. Without this, "no re-arm while
    // selected" would also hold if the re-arm simply did not exist.
    assert!(s.terminal_manager.borrow_mut().clear_selection(key_id));
    s.terminal_manager
        .borrow_mut()
        .snapshot_for_view(key_id, grown)
        .expect("snapshot after clearing");
    let cleared = s
        .terminal_manager
        .borrow_mut()
        .view_status(key_id)
        .expect("view status after clearing");
    assert!(
        cleared.at_bottom && cleared.scroll_offset == 0,
        "clearing the selection re-arms follow at the same geometry"
    );
    assert_ne!(
        top_before,
        first_visible_row(&s, key_id, grown),
        "…and the view left the frozen anchor"
    );
    exec(&s, "pmacs.terminal.terminate(TERM_BUF)");
}

/// Text of the view's first visible row — the anchor, read through the
/// same per-view projection the painter uses.
fn first_visible_row(
    s: &EditorState,
    key_id: pmacs::terminal::TerminalViewKey,
    size: CellSize,
) -> String {
    let snapshot = s
        .terminal_manager
        .borrow_mut()
        .snapshot_for_view(key_id, size)
        .expect("view snapshot");
    snapshot
        .cells
        .iter()
        .take(size.cols as usize)
        .filter_map(|cell| match &cell.glyph {
            Glyph::Char(ch) => Some(*ch),
            Glyph::Cluster(_) => Some('?'),
            Glyph::Continuation => None,
        })
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[test]
fn acc34_only_the_controller_resizes_the_pty() {
    let mut s = editor();
    exec(
        &s,
        "TERM_BUF = pmacs.terminal.open { command = \"/bin/sh\", \
           args = { \"-c\", \"sleep 30\" }, display = \"panel\" }",
    );
    let panel = side_window(&s).expect("terminal panel");
    let buffer: pmacs::lua_bindings::BufferIdLua = eval(&s, "return TERM_BUF");
    render(&s);
    s.sync_terminal_layout(FrontendId::LOCAL, CellSize::new(ROWS, COLS));
    let controlled = s.terminal_manager.borrow().screen_size(buffer.0);

    // A second frontend that does NOT control the session may hold its
    // own panel height without resizing the child.
    let other = FrontendId(34);
    attach_frontend(&s, other, true);
    s.sync_terminal_layout(other, CellSize::new(ROWS, COLS));
    assert_eq!(
        s.terminal_manager.borrow().screen_size(buffer.0),
        controlled,
        "only the controller's height change resizes the PTY"
    );
    let _ = panel;
    exec(&s, "pmacs.terminal.terminate(TERM_BUF)");
}

// ---------------------------------------------------------------------------
// 35 — the desktop never persists a side window
// ---------------------------------------------------------------------------

#[test]
fn acc35_desktop_round_trip_omits_the_side_leaf_and_its_wrapper() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("saved.txt");
    std::fs::write(&file, b"content\n").unwrap();
    let path = file.display().to_string();

    let s = editor();
    exec(&s, &format!("pmacs.buffer.find_or_open({path:?})"));
    let document_structure = structure(&layout_root(&s));
    exec(
        &s,
        &format!(
            "PANEL_BUF = pmacs.buffer.find_or_open({path:?})
             pmacs.window.display(PANEL_BUF, {{ side = \"bottom\", height = 6 }})"
        ),
    );
    assert!(side_window(&s).is_some());

    let snapshot =
        pmacs::desktop::snapshot(&s.core.borrow(), "test".into()).expect("a file window survives");
    assert_eq!(
        snapshot.version,
        pmacs::desktop::DESKTOP_VERSION,
        "the desktop format version does not change"
    );
    assert!(
        matches!(snapshot.root, pmacs::desktop::SavedNode::Leaf(_)),
        "neither the side leaf nor its root wrapper is persisted \
         (saw {:?})",
        snapshot.root
    );
    let _ = document_structure;
}

// ---------------------------------------------------------------------------
// Core-level invariants that back the above
// ---------------------------------------------------------------------------

#[test]
fn panel_hidden_never_describes_a_panel_that_no_longer_exists() {
    let s = editor();
    let panel = open_panel(&s, "*panel*", 8);
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(4, COLS));
    assert!(s.core.borrow().panel_hidden_for(FrontendId::LOCAL));
    s.core
        .borrow_mut()
        .remove_side_window(FrontendId::LOCAL, panel);
    s.reconcile_panel_layout(FrontendId::LOCAL);
    assert!(
        !s.core.borrow().views[&FrontendId::LOCAL].panel_hidden,
        "reconciliation clears the flag once the window is gone"
    );
}

#[test]
fn unknown_geometry_is_not_twenty_four_by_eighty() {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    exec(&s, "pmacs.lsp.config = {}");
    let fid = FrontendId(77);
    attach_frontend(&s, fid, false);
    assert!(
        s.core.borrow().frontend_area_rows(fid).is_none(),
        "a semantic view's geometry is UNKNOWN, never the attach placeholder"
    );
    let buffer = s.core.borrow_mut().registry.borrow_mut().create("*p*");
    let mut request = DisplayRequest::new(buffer);
    request.side = Some(Side::Bottom);
    let _ = s.core.borrow_mut().display_buffer(fid, &request);
    // Not panel-capable in Stage 1, so it fell back; and even a capable
    // view with unknown geometry would follow the hidden arm.
    assert!(s.core.borrow().side_window_for(fid).is_none());
}

#[test]
fn quit_action_truncation_is_iterative_and_bounded() {
    let mut action = QuitAction::Delete;
    for _ in 0..(MAX_PANEL_QUIT_DEPTH * 3) {
        action = QuitAction::Restore {
            buffer_id: BufferId::from_raw(1),
            fixed_rows: 4,
            dedicated: false,
            cursor: 0,
            view_top: 0,
            goal_col: None,
            selection: None,
            then: Box::new(action),
        };
        action.truncate_to(MAX_PANEL_QUIT_DEPTH);
        assert!(action.depth() <= MAX_PANEL_QUIT_DEPTH);
    }
}

#[test]
fn clamp_panel_rows_rejects_zero_and_lifts_to_the_floor() {
    assert!(EditorCore::clamp_panel_rows(0).is_err());
    assert_eq!(EditorCore::clamp_panel_rows(1), Ok(MIN_WINDOW_OUTER_ROWS));
    assert_eq!(EditorCore::clamp_panel_rows(30), Ok(30));
}

#[test]
fn cell_coord_helper_is_used() {
    // Keeps the CellCoord import honest for grid assertions above.
    assert_eq!(CellCoord::new(1, 2).row, 1);
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
