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
            // A2A-2: the semantic fan-out captures the primary document
            // AND the visible side window — two contexts, not one.
            assert_eq!(
                windows.len(),
                2,
                "the semantic-layout target must capture document + visible side window"
            );
            let side = windows
                .iter()
                .find(|w| w.context.window_id != document)
                .expect("a side-window context");
            assert!(
                side.context.active,
                "the focused panel's own context reports active = true"
            );
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

    // The gutter only paints when line numbers are on, so turn them on
    // rather than dropping the assertion.
    {
        let mut core = s.core.borrow_mut();
        let active = core.views[&FrontendId::LOCAL].active;
        core.windows.get_mut(&active).unwrap().line_numbers =
            pmacs::window::LineNumberMode::Absolute;
    }

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

    // Review round 1, finding 5: a fixed-point check alone is VACUOUS —
    // deleting `text_view.render` leaves it green. Assert the extracted
    // painter actually produced each of its four outputs.
    let rows = |cells: &[pmacs::cell::Cell]| -> Vec<String> {
        (0..ROWS as usize)
            .map(|r| {
                (0..COLS as usize)
                    .map(|c| match &cells[r * COLS as usize + c].glyph {
                        pmacs::cell::Glyph::Char(ch) => *ch,
                        _ => ' ',
                    })
                    .collect::<String>()
            })
            .collect()
    };
    let painted = rows(&cells_a);

    // TEXT: the buffer's content reached the grid.
    assert!(
        painted.iter().any(|row| row.contains("line")),
        "the extracted painter must paint buffer TEXT; got {painted:?}"
    );
    // GUTTER: line numbers were painted beside it.
    assert!(
        painted.iter().any(|row| row.trim_start().starts_with('1')),
        "the extracted painter must paint the line-number GUTTER"
    );
    // MODE LINE: the window's mode line names its buffer.
    assert!(
        painted.iter().any(|row| row.contains("*doc*")),
        "the extracted painter must paint the window MODE LINE"
    );
    // CURSOR: a real caret position came back, not None.
    assert!(
        cursor_a.is_some(),
        "the extraction must still return a caret position"
    );
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

// ---------------------------------------------------------------------------
// A2A-1 at the CONSUMER seam — review round 1, finding 3.
//
// The tests above assert the *authority* (`primary_document_window`).
// That is not sufficient: restoring a producer to `active_window_for`
// leaves every one of them green. These drive the real producers through
// `SemanticRenderState::render_frame` with a panel focused, so a reverted
// routing fails here.
// ---------------------------------------------------------------------------

/// A semantic frontend that CAN hold a panel. Stage 1 ships
/// `panel_capable = false` for semantic sessions and 2B flips it for a
/// v21-negotiated peer; until then the projection is only reachable with
/// a test-only capable view, which is exactly what the framing's §7.2
/// says 2B must replace with the real capability flip.
fn semantic_frontend_with_focused_panel(
    s: &EditorState,
) -> (FrontendId, WindowId, WindowId, pmacs::buffer::BufferId) {
    use pmacs::window::{FrontendView, Layout, LayoutNode, Orientation, Window, WindowParams};

    let fid = FrontendId(77);
    let (doc_win, panel_win, doc_buf) = {
        let mut core = s.core.borrow_mut();
        let doc_buf = core.active_window().buffer_id;
        let panel_buf = core.registry.borrow_mut().create("*panel*");

        let doc_win = WindowId::next();
        let panel_win = WindowId::next();
        let doc_view = {
            let reg = core.registry.borrow();
            pmacs::text_view::TextView::new(reg.get(doc_buf).expect("document buffer"))
        };
        let panel_view = {
            let reg = core.registry.borrow();
            pmacs::text_view::TextView::new(reg.get(panel_buf).expect("panel buffer"))
        };
        core.windows
            .insert(doc_win, Window::new(doc_win, doc_buf, doc_view));
        let mut panel = Window::new(panel_win, panel_buf, panel_view);
        let mut params = WindowParams::default();
        params.side = Some(Side::Bottom);
        params.fixed_rows = Some(4);
        panel.params = params;
        core.windows.insert(panel_win, panel);

        core.register_frontend_view(
            fid,
            FrontendView {
                layout: Layout {
                    root: LayoutNode::Split {
                        orientation: Orientation::Horizontal,
                        children: vec![LayoutNode::Leaf(doc_win), LayoutNode::Leaf(panel_win)],
                        weights: vec![1, 1],
                    },
                },
                // The panel owns focus; the document is the projection.
                active: panel_win,
                fold_projection: false,
                panel_capable: true,
                frame_geometry: None,
                panel_hidden: false,
            },
        );
        (doc_win, panel_win, doc_buf)
    };
    s.sync_frame_geometry(fid, CellSize::new(ROWS, COLS));
    (fid, doc_win, panel_win, doc_buf)
}

#[test]
fn consumer_line_numbers_follow_the_document_not_the_focused_panel() {
    use pmacs::protocol::{ByteRange, InstanceMessage};
    use pmacs::semantic_render::SemanticRenderState;
    use pmacs::window::LineNumberMode;

    let s = editor();
    let (fid, doc_win, panel_win, doc_buf) = semantic_frontend_with_focused_panel(&s);

    // Make the two windows DISAGREE, so the emitted mode identifies
    // which window the producer read (§1.3 #4).
    {
        let mut core = s.core.borrow_mut();
        core.windows.get_mut(&doc_win).unwrap().line_numbers = LineNumberMode::Absolute;
        core.windows.get_mut(&panel_win).unwrap().line_numbers = LineNumberMode::Off;
    }

    let mut sem = SemanticRenderState::new(fid);
    sem.set_viewport(doc_buf, ByteRange { start: 0, end: 0 }, 0);
    let msgs = sem.render_frame(&s);

    let mode = msgs.iter().find_map(|m| match m {
        InstanceMessage::LineNumbers { mode, .. } => Some(*mode),
        _ => None,
    });
    assert_eq!(
        mode,
        Some(pmacs::protocol::LineNumberMode::Absolute),
        "LineNumbers must describe the DOCUMENT window's mode, not the focused panel's"
    );
}

#[test]
fn consumer_statusline_segments_name_the_document_window() {
    use pmacs::protocol::ByteRange;
    use pmacs::semantic_render::SemanticRenderState;

    // §1.3 #12 at the producer: the wire segments must be selected by
    // the DOCUMENT window even though the fan-out now also evaluates the
    // visible side window (A2A-2).
    let s = editor();
    let (fid, doc_win, _panel_win, doc_buf) = semantic_frontend_with_focused_panel(&s);

    let mut sem = SemanticRenderState::new(fid);
    sem.set_viewport(doc_buf, ByteRange { start: 0, end: 0 }, 0);
    // Not asserting on message presence (a peer that never negotiated
    // v18 emits none); asserting the routing input the producer uses.
    let _ = sem.render_frame(&s);

    assert_eq!(
        s.core.borrow().primary_document_window(fid),
        Some(doc_win),
        "the producer's document-window selector must name the document"
    );
}

#[test]
fn consumer_terminal_declaration_cannot_be_claimed_by_a_focused_panel() {
    // §1.3 #6/#10/#11 through the real guard: with the panel focused,
    // a declaration naming the PANEL's buffer must be refused, because
    // the full-window terminal surface is the document window.
    let s = editor();
    let (fid, _doc_win, panel_win, doc_buf) = semantic_frontend_with_focused_panel(&s);
    let panel_buf = s.core.borrow().windows[&panel_win].buffer_id;

    assert!(
        !s.semantic_terminal_declaration_is_active(fid, panel_buf),
        "a focused panel's buffer must not become the document terminal declaration"
    );
    // Non-vacuity: the document buffer is not a terminal either, so pin
    // that the guard resolves the DOCUMENT window by asserting the
    // window identity the resolver used.
    assert_eq!(
        s.core.borrow().primary_document_buffer(fid),
        Some(doc_buf),
        "the terminal resolver's window must be the document window"
    );
}
