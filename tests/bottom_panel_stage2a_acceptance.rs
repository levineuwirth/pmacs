// bottom_panel_stage2a_acceptance.rs --- bottom-panel Stage 2A
// (docs/bottom-panel-stage2-framing.md, criteria A2A-1 / A2A-2 / A2A-3).

//! Classified §1.3 census routing + the per-window painter extraction.
//! No wire change.
//!
//! **The negative half is the load-bearing half.** A suite that only
//! proved "the document surface is used" would pass with the focus,
//! focus-chrome, and focus/session consumers *wrongly* rerouted to the
//! document — which is the defect the framing spent three review rounds
//! eliminating, and which would break remote-op validation,
//! `DispatchIdle`, presence, focused search/menu/completion routing, and
//! terminal bell ownership. So every Projection assertion here is paired
//! with a focus-class assertion taken in the *same* state.

use pmacs::cell::{CellGrid, CellSize};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;
use pmacs::window::{Side, WindowId};

const ROWS: u32 = 24;
const COLS: u32 = 60;

fn editor() -> EditorState {
    let s = EditorState::new();
    exec(&s, "pmacs.lsp.config = {}");
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(ROWS, COLS));
    s
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn side_window(s: &EditorState) -> Option<WindowId> {
    let core = s.core.borrow();
    core.views[&FrontendId::LOCAL]
        .layout
        .iter_ids()
        .into_iter()
        .find(|id| {
            core.windows
                .get(id)
                .is_some_and(|w| w.params.side.is_some())
        })
}

/// Open a bottom panel and leave it FOCUSED — the state in which every
/// classification difference becomes observable.
fn focused_panel(s: &EditorState) -> (WindowId, WindowId) {
    let document = s.core.borrow().views[&FrontendId::LOCAL].active;
    exec(
        s,
        "PANEL_BUF = pmacs.buffer.create(\"*panel*\")
         PANEL_WIN = pmacs.window.display(PANEL_BUF, \
             { side = \"bottom\", height = 4 })",
    );
    let panel = side_window(s).expect("panel exists");
    s.core.borrow_mut().focus_window(FrontendId::LOCAL, panel);
    assert_eq!(
        s.core.borrow().views[&FrontendId::LOCAL].active,
        panel,
        "fixture precondition: the panel must own focus"
    );
    (document, panel)
}

fn render(s: &EditorState) {
    let size = CellSize::new(ROWS, COLS);
    let mut cells = vec![pmacs::cell::Cell::default(); (ROWS * COLS) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: size.cols,
        size,
    };
    let _ = pmacs::editor::paint_frame(
        s,
        FrontendId::LOCAL,
        &std::collections::HashMap::new(),
        &mut grid,
        size,
    );
}

// ---------------------------------------------------------------------------
// A2A-1 — the Projection class resolves the document surface
// ---------------------------------------------------------------------------

#[test]
fn projection_resolves_the_document_window_while_a_panel_is_focused() {
    let s = editor();
    let (document, panel) = focused_panel(&s);
    let core = s.core.borrow();

    assert_eq!(
        core.primary_document_window(FrontendId::LOCAL),
        Some(document),
        "Projection consumers must resolve the document window, not the focused panel"
    );
    assert_ne!(document, panel);
}

#[test]
fn projection_buffer_is_the_document_buffer_not_the_panel_buffer() {
    let s = editor();
    let (document, _panel) = focused_panel(&s);
    let core = s.core.borrow();

    let document_buffer = core.windows[&document].buffer_id;
    assert_eq!(
        core.primary_document_buffer(FrontendId::LOCAL),
        Some(document_buffer),
        "the replica's document mirror must not follow panel focus"
    );
    assert_ne!(
        core.primary_document_buffer(FrontendId::LOCAL),
        Some(core.windows[&core.views[&FrontendId::LOCAL].active].buffer_id),
        "non-vacuity: the focused window's buffer differs, so this test can fail"
    );
}

// ---------------------------------------------------------------------------
// A2A-1 — the NEGATIVE half: focus classes still resolve focus
// ---------------------------------------------------------------------------

#[test]
fn focus_class_dispatch_idle_still_tracks_the_focused_window() {
    let s = editor();
    let (_document, _panel) = focused_panel(&s);

    // §1.3 #14 — Focus. Q#BP14a: optimistic input is gated per WINDOW.
    // A panel that owns focus must suppress `DispatchIdle` even though
    // the *document* projection is unaffected.
    assert!(
        !s.dispatch_idle_for(FrontendId::LOCAL),
        "a focused side window must gate optimistic input off (#14)"
    );
}

#[test]
fn focus_class_gate_lifts_when_focus_returns_to_the_document() {
    let s = editor();
    let (document, _panel) = focused_panel(&s);
    s.core
        .borrow_mut()
        .focus_window(FrontendId::LOCAL, document);

    assert!(
        s.dispatch_idle_for(FrontendId::LOCAL),
        "non-vacuity: the gate must lift with focus, or the test above proves nothing"
    );
}

#[test]
fn focus_and_projection_disagree_in_the_same_state() {
    // The single most important assertion in this suite: in ONE state,
    // the two classes must resolve DIFFERENT windows. If a future change
    // routes the focus class through `primary_document_window`, this
    // fails even though every Projection test above still passes.
    let s = editor();
    let (document, panel) = focused_panel(&s);
    let core = s.core.borrow();

    let focused = core.views[&FrontendId::LOCAL].active;
    let projected = core
        .primary_document_window(FrontendId::LOCAL)
        .expect("a document window exists");

    assert_eq!(focused, panel, "focus authority must name the panel");
    assert_eq!(
        projected, document,
        "projection authority must name the document"
    );
    assert_ne!(
        focused, projected,
        "the two authorities must be genuinely distinct in this state"
    );
}

// ---------------------------------------------------------------------------
// A2A-2 — the statusline split: lookup reroutes, `active` does not
// ---------------------------------------------------------------------------

#[test]
fn statusline_document_context_reports_active_false_under_a_focused_panel() {
    use pmacs::statusline::{
        StatuslineEvaluationOutcome, StatuslineEvaluationTarget, evaluate_statusline,
    };

    let s = editor();
    let (document, _panel) = focused_panel(&s);
    let declared = s.core.borrow().windows[&document].buffer_id;

    let evaluation = evaluate_statusline(
        s.lua_host.lua(),
        &s.core,
        &s.statusline_registry,
        StatuslineEvaluationTarget::Semantic {
            frontend_id: FrontendId::LOCAL,
            declared_buffer: declared,
        },
    );

    match evaluation.outcome {
        StatuslineEvaluationOutcome::Ready(windows) => {
            let context = windows
                .first()
                .map(|segments| segments.context)
                .expect("one document context");
            // The LOOKUP rerouted: it resolved the document window even
            // though the panel is focused (§1.3 #12).
            assert_eq!(
                context.window_id, document,
                "the semantic target must resolve the primary document window"
            );
            // `active` did NOT reroute (parent acceptance 42): a document
            // provider observes the truth, that it is not focused.
            assert!(
                !context.active,
                "a document provider must observe active = false while the panel owns focus"
            );
        }
        other => panic!("expected a ready evaluation, got {other:?}"),
    }
}

#[test]
fn statusline_document_context_is_active_when_the_document_is_focused() {
    use pmacs::statusline::{
        StatuslineEvaluationOutcome, StatuslineEvaluationTarget, evaluate_statusline,
    };

    // Non-vacuity for the assertion above: with focus on the document,
    // the same context must report `active = true`.
    let s = editor();
    let (document, _panel) = focused_panel(&s);
    s.core
        .borrow_mut()
        .focus_window(FrontendId::LOCAL, document);
    let declared = s.core.borrow().windows[&document].buffer_id;

    let evaluation = evaluate_statusline(
        s.lua_host.lua(),
        &s.core,
        &s.statusline_registry,
        StatuslineEvaluationTarget::Semantic {
            frontend_id: FrontendId::LOCAL,
            declared_buffer: declared,
        },
    );

    match evaluation.outcome {
        StatuslineEvaluationOutcome::Ready(windows) => {
            let context = windows.first().map(|s| s.context).expect("one context");
            assert!(context.active, "a focused document context must be active");
        }
        other => panic!("expected a ready evaluation, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// A2A-3 — the painter extraction preserves grid behavior
// ---------------------------------------------------------------------------

#[test]
fn extraction_preserves_cells_cursor_and_focused_view_top() {
    // The extraction must preserve four things, not just cells: a clamp
    // that silently moved to the WRONG window would leave the painted
    // cells identical on a single-window frame.
    let s = editor();
    exec(
        &s,
        "local b = pmacs.buffer.create(\"*doc*\")
         b:insert(0, string.rep(\"line\\n\", 200))
         pmacs.window.display(b, {})",
    );

    let size = CellSize::new(ROWS, COLS);
    let mut cells_a = vec![pmacs::cell::Cell::default(); (ROWS * COLS) as usize];
    let mut grid_a = CellGrid {
        cells: &mut cells_a,
        stride: size.cols,
        size,
    };
    let cursor_a = pmacs::editor::paint_frame(
        &s,
        FrontendId::LOCAL,
        &std::collections::HashMap::new(),
        &mut grid_a,
        size,
    );
    let active = s.core.borrow().views[&FrontendId::LOCAL].active;
    let view_top_a = s.core.borrow().windows[&active].view_top;

    // A second identical paint is a fixed point: same cells, same
    // returned cursor, same `view_top`.
    let mut cells_b = vec![pmacs::cell::Cell::default(); (ROWS * COLS) as usize];
    let mut grid_b = CellGrid {
        cells: &mut cells_b,
        stride: size.cols,
        size,
    };
    let cursor_b = pmacs::editor::paint_frame(
        &s,
        FrontendId::LOCAL,
        &std::collections::HashMap::new(),
        &mut grid_b,
        size,
    );
    let view_top_b = s.core.borrow().windows[&active].view_top;

    assert_eq!(cells_a, cells_b, "painted cells must be stable");
    assert_eq!(cursor_a, cursor_b, "the returned cursor must be stable");
    assert_eq!(view_top_a, view_top_b, "focused view_top must be stable");
}

#[test]
fn extraction_leaves_a_passive_window_view_top_untouched() {
    // The auto-scroll clamp runs for the FOCUSED window only. A passive
    // window's scroll state must survive a frame it did not own.
    let s = editor();
    exec(
        &s,
        "local b = pmacs.buffer.create(\"*doc*\")
         b:insert(0, string.rep(\"line\\n\", 200))
         pmacs.window.display(b, {})
         pmacs.window.split_horizontal()",
    );
    render(&s);

    let (passive, before) = {
        let core = s.core.borrow();
        let view = &core.views[&FrontendId::LOCAL];
        let passive = view
            .layout
            .iter_ids()
            .into_iter()
            .find(|id| *id != view.active)
            .expect("a second window exists");
        (passive, core.windows[&passive].view_top)
    };

    // Scroll the passive window somewhere the clamp would "fix" if it
    // ever ran against the wrong window.
    s.core
        .borrow_mut()
        .windows
        .get_mut(&passive)
        .unwrap()
        .view_top = 120;
    render(&s);

    assert_eq!(
        s.core.borrow().windows[&passive].view_top,
        120,
        "a passive window's view_top must not be clamped by another window's frame"
    );
    assert_ne!(before, 120, "non-vacuity: the value actually changed");
}

// ---------------------------------------------------------------------------
// Fixture integrity
// ---------------------------------------------------------------------------

#[test]
fn the_panel_fixture_really_builds_a_side_window() {
    // Every test above is worthless if `focused_panel` silently produced
    // an ordinary split, so pin the fixture's own precondition.
    let s = editor();
    let (_document, panel) = focused_panel(&s);
    let core = s.core.borrow();
    assert_eq!(
        core.windows[&panel].params.side,
        Some(Side::Bottom),
        "the fixture must produce a real bottom side window"
    );
}
