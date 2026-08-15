// bottom_panel_stage2b_daemon_acceptance.rs --- bottom-panel Stage 2B-2
// (docs/bottom-panel-stage2-framing.md §7.2.2; parent acceptance 38, 39
// receiver half, 40, 41 daemon half, 42, 45, 49, 51, 52, plus A2B-1).

//! The daemon panel projection and the epoch machine.
//!
//! Everything here runs through an explicitly panel-capable semantic
//! view constructed by the fixture. When this suite was written that was
//! a *test-only* configuration — production negotiation set
//! `panel_capable = false` for every semantic session. **Stage 2B-3 made
//! it production-reachable**: a semantic session that negotiates
//! `PANEL_MIN_VERSION` is panel-capable, activated by the frontend's
//! `AttachRequest` counter-offer rather than by moving the advertised
//! `Hello` version. These assertions did not change, and that is the
//! point — the projection they pin is the one production now drives.
//! `bottom_panel_stage2b_gpu_acceptance` covers the negotiation itself.
//!
//! Two disciplines the framing is explicit about:
//!
//! * **Every geometry claim is asserted against the frame the producer
//!   actually shipped**, never against `panel_grid_size` alone — the
//!   grid the daemon derives is only meaningful if it reaches the wire.
//! * **Every drop is paired with its accepted counterpart in the same
//!   fixture.** A suite that only proved "stale events are dropped"
//!   would pass against a producer that ships nothing at all.

use std::collections::HashMap;

use pmacs::cell::{CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::editor_core::GeometryUpdate;
use pmacs::protocol::{FrontendId, InstanceMessage, PROTOCOL_VERSION};
use pmacs::semantic_render::SemanticRenderState;
use pmacs::window::{FrontendView, Layout, Window, WindowId};
use pmacs_protocol::panel::{PANEL_MIN_VERSION, PanelFrame, PanelFramePayload};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

const FID: FrontendId = FrontendId(64);
/// Deliberately NOT `24x80`: parent acceptance 40 requires the first
/// open to be sized from the frontend's own declaration, and a fixture
/// that happened to match the attach placeholder could not tell the two
/// apart.
const ROWS: u32 = 40;
const COLS: u32 = 120;

struct Session {
    state: EditorState,
    render: SemanticRenderState,
    document: WindowId,
}

impl Session {
    /// A semantic, panel-capable frontend with one document window and
    /// no geometry declared yet.
    fn new() -> Self {
        let state = EditorState::new_with_roots(&crate::iso::roots());
        exec(&state, "pmacs.lsp.config = {}");
        let document = {
            let mut core = state.core.borrow_mut();
            let buffer_id = core.active_buffer_id();
            let text_view = {
                let reg = core.registry.borrow();
                pmacs::text_view::TextView::new(reg.get(buffer_id).expect("buffer"))
            };
            let window = WindowId::next();
            core.windows
                .insert(window, Window::new(window, buffer_id, text_view));
            core.register_frontend_view(
                FID,
                FrontendView {
                    layout: Layout::single(window),
                    active: window,
                    // Semantic sessions do not project folds (Q#FD21);
                    // parent acceptance 52 depends on it.
                    fold_projection: false,
                    // Test-only. Production negotiation still says false.
                    panel_capable: true,
                    // Q#BP15a: UNKNOWN, never the attach placeholder.
                    frame_geometry: None,
                    panel_hidden: false,
                },
            );
            // Programmatic Lua calls act for the ambient active frontend,
            // so every `pmacs.window.*` call below targets this session.
            core.active_frontend = FID;
            window
        };
        Self {
            state,
            render: SemanticRenderState::for_peer(FID, PROTOCOL_VERSION),
            document,
        }
    }

    fn declare(&self, epoch: u64, rows: u32, cols: u32) -> GeometryUpdate {
        self.state
            .accept_semantic_frame_geometry(FID, epoch, CellSize::new(rows, cols))
    }

    /// Project one frame and return the panel payload it carried, or
    /// `None` when the frame said nothing about the band (which is what
    /// duplicate suppression looks like from the wire).
    fn frame(&mut self) -> Option<PanelFramePayload> {
        let messages = self.render.render_frame(&self.state);
        let mut panels = messages.into_iter().filter_map(|message| match message {
            InstanceMessage::PanelFrame(payload) => Some(payload),
            _ => None,
        });
        let first = panels.next();
        assert!(
            panels.next().is_none(),
            "one frame ships at most one panel payload"
        );
        first
    }

    fn present(&mut self) -> PanelFrame {
        match self.frame() {
            Some(PanelFramePayload::Present(frame)) => frame,
            other => panic!("expected a Present panel payload, got {other:?}"),
        }
    }

    fn side_window(&self) -> Option<WindowId> {
        self.state.core.borrow().side_window_for(FID)
    }
}

fn exec(state: &EditorState, src: &str) {
    state.lua_host.lua().load(src.to_string()).exec().unwrap();
}

/// The panel grid's text as rows of strings, mode line included.
fn rows_of(frame: &PanelFrame) -> Vec<String> {
    frame
        .cells
        .chunks(frame.size.cols as usize)
        .map(|row| {
            row.iter()
                .map(|cell| match &cell.glyph {
                    Glyph::Char(ch) => ch.to_string(),
                    Glyph::Cluster(bytes) => String::from_utf8_lossy(bytes).into_owned(),
                    Glyph::Continuation => String::new(),
                })
                .collect::<String>()
        })
        .collect()
}

/// Replace the panel buffer's contents through the real Lua data API.
fn set_panel_text(session: &Session, text: &str) {
    exec(
        &session.state,
        &format!(
            "local len = PANEL_BUF:len()
             if len > 0 then PANEL_BUF:delete(0, len) end
             PANEL_BUF:insert(0, {text:?})"
        ),
    );
}

/// Close the panel: focus it, then take the real `close_active` path.
fn close_panel(session: &Session) {
    let panel = session.side_window().expect("side window");
    session.state.core.borrow_mut().focus_window(FID, panel);
    exec(&session.state, "pmacs.window.close()");
    session.state.reconcile_panel_layout(FID);
}

fn open_panel(session: &Session, name: &str, rows: u32) {
    exec(
        &session.state,
        &format!(
            "PANEL_BUF = pmacs.buffer.create(\"{name}\")
             pmacs.window.display(PANEL_BUF, {{ side = \"bottom\", height = {rows} }})"
        ),
    );
}

// ---------------------------------------------------------------------------
// 40 + A2B-1 — unknown geometry is first-class; the placeholder is never
// consulted
// ---------------------------------------------------------------------------

#[test]
fn acc40_a_panel_opened_before_any_declaration_stays_absent() {
    let mut session = Session::new();
    open_panel(&session, "*panel*", 4);
    assert!(session.side_window().is_some(), "the side window exists");

    assert_eq!(
        session.frame(),
        None,
        "with geometry UNKNOWN the band is non-presentable, and the seeded \
         Absent baseline means there is nothing new to say"
    );
    assert_eq!(
        session.state.core.borrow().panel_grid_size(FID),
        None,
        "no grid is derivable before a real declaration"
    );
}

#[test]
fn acc40_the_first_frame_is_sized_from_the_declaration_not_the_placeholder() {
    let mut session = Session::new();
    open_panel(&session, "*panel*", 4);
    assert_eq!(session.declare(1, ROWS, COLS), GeometryUpdate::Advanced);

    let frame = session.present();
    assert_eq!(
        frame.size.cols, COLS,
        "columns come from the frontend's declaration; the permanent 24x80 \
         attach placeholder would have produced 80"
    );
    assert_eq!(
        frame.size.rows, 4,
        "rows are the clamped fixed_rows request"
    );
    assert_eq!(
        frame.geometry_epoch, 1,
        "the frame echoes the declaration it answers"
    );
    assert!(frame.panel_epoch >= 1, "presentation epochs start at 1");
}

// ---------------------------------------------------------------------------
// 38 / 49 — the full lifecycle, and a new epoch at every identity change
// ---------------------------------------------------------------------------

#[test]
fn acc38_open_replace_hide_reappear_close() {
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*first*", 4);

    // --- open ---------------------------------------------------------
    let opened = session.present();
    let first_buffer = opened.buffer_id;

    // --- replace the panel's buffer -----------------------------------
    exec(
        &session.state,
        "SECOND = pmacs.buffer.create(\"*second*\")
         pmacs.window.display(SECOND, { side = \"bottom\" })",
    );
    let replaced = session.present();
    assert_ne!(
        replaced.buffer_id, first_buffer,
        "the panel really is showing another buffer"
    );
    assert!(
        replaced.panel_epoch > opened.panel_epoch,
        "49: buffer replacement must move the presentation epoch \
         ({} -> {})",
        opened.panel_epoch,
        replaced.panel_epoch
    );

    // --- hidden by a frame too small to satisfy it --------------------
    // Q#BP2b: hiding is a durable transition, and the band must be
    // cleared AUTHORITATIVELY. Silence would leave the retained frame
    // on the peer's screen indefinitely.
    session.declare(2, 4, COLS);
    assert_eq!(
        session.frame(),
        Some(PanelFramePayload::Absent),
        "38: hiding sends an authoritative Absent"
    );
    assert!(
        session.side_window().is_some(),
        "…while the side window itself survives, with its request intact"
    );

    // --- reappear -----------------------------------------------------
    session.declare(3, ROWS, COLS);
    let reappeared = session.present();
    assert!(
        reappeared.panel_epoch > replaced.panel_epoch,
        "49: every Absent -> Present transition takes a fresh epoch \
         ({} -> {})",
        replaced.panel_epoch,
        reappeared.panel_epoch
    );
    assert_eq!(
        reappeared.buffer_id, replaced.buffer_id,
        "…even though the SAME persistent buffer came back, which is exactly \
         the hole a buffer id alone cannot close"
    );

    // --- close --------------------------------------------------------
    close_panel(&session);
    assert_eq!(session.side_window(), None, "the side window is gone");
    assert_eq!(
        session.frame(),
        Some(PanelFramePayload::Absent),
        "38: closing sends an authoritative Absent"
    );
}

#[test]
fn acc49_close_and_reopen_of_the_same_buffer_takes_a_new_epoch() {
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    exec(
        &session.state,
        "PERSISTENT = pmacs.buffer.create(\"*persistent*\")
         pmacs.window.display(PERSISTENT, { side = \"bottom\", height = 4 })",
    );
    let first = session.present();

    close_panel(&session);
    assert_eq!(session.frame(), Some(PanelFramePayload::Absent));

    exec(
        &session.state,
        "pmacs.window.display(PERSISTENT, { side = \"bottom\", height = 4 })",
    );
    let second = session.present();

    assert_eq!(
        second.buffer_id, first.buffer_id,
        "the SAME persistent buffer is back — a buffer id alone cannot \
         distinguish this from the original presentation"
    );
    assert!(
        second.panel_epoch > first.panel_epoch,
        "49: the presentation epoch must have moved ({} -> {})",
        first.panel_epoch,
        second.panel_epoch
    );
}

// ---------------------------------------------------------------------------
// 39 (receiver half) — duplicates do no work; an invalid frame is
// rejected atomically and the previous valid frame is retained
// ---------------------------------------------------------------------------

#[test]
fn acc39_a_duplicate_frame_does_no_work() {
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*panel*", 4);
    let _first = session.present();

    assert_eq!(
        session.frame(),
        None,
        "39: nothing changed, so the second frame ships no panel payload"
    );
    assert_eq!(
        session.frame(),
        None,
        "…and it stays quiet, rather than alternating"
    );
}

#[test]
fn acc39_a_duplicate_absent_does_no_work() {
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*panel*", 4);
    let _present = session.present();

    close_panel(&session);
    assert_eq!(
        session.frame(),
        Some(PanelFramePayload::Absent),
        "the first Absent is authoritative and must be sent"
    );
    assert_eq!(
        session.frame(),
        None,
        "39: a duplicate Absent is suppressed exactly like any other payload"
    );
}

/// 39, receiver half: rejection is atomic and the peer keeps its last
/// valid frame.
///
/// The producer-reachable route to an invalid frame runs through the
/// **mode line**, not the text: `TextView::render` drops zero-width marks
/// and never emits a cluster, and the terminal screen caps its clusters
/// at `MAX_TERMINAL_GRAPHEME_BYTES` — but `prepare_mode_line_runs` emits
/// whole grapheme clusters verbatim, and one of its inputs is the buffer
/// **name**. A name carrying a cluster past
/// `MAX_WIRE_GRID_GRAPHEME_BYTES` therefore produces a structurally
/// invalid panel frame through the ordinary display path.
#[test]
fn acc39_an_invalid_frame_is_rejected_and_the_previous_one_is_retained() {
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*panel*", 4);
    set_panel_text(&session, "before");
    let valid = session.present();
    assert!(
        rows_of(&valid)[0].starts_with("before"),
        "fixture precondition: the retained frame really shows the old text"
    );

    // One grapheme cluster far past the shared per-cell byte ceiling.
    let monster = format!("x{}", "\u{301}".repeat(300));
    assert!(
        monster.len() > pmacs_protocol::wire_grid::MAX_WIRE_GRID_GRAPHEME_BYTES,
        "fixture precondition: the cluster really exceeds the shared ceiling"
    );
    exec(
        &session.state,
        &format!(
            "BAD = pmacs.buffer.create({monster:?})
             pmacs.window.display(BAD, {{ side = \"bottom\" }})"
        ),
    );

    assert_eq!(
        session.frame(),
        None,
        "39: an invalid frame is not shipped, whole or in part"
    );
    let retained = session
        .render
        .panel_declaration()
        .expect("the previous valid frame is retained");
    assert_eq!(
        retained, &valid,
        "39: the receiver's authority is unchanged — same cells, same epochs"
    );

    // The rejection also burned no presentation identity: nothing the
    // peer ever saw carried the epoch the rejected frame would have used.
    exec(
        &session.state,
        "GOOD = pmacs.buffer.create(\"*good*\")
         pmacs.window.display(GOOD, { side = \"bottom\" })",
    );
    let recovered = session.present();
    assert_eq!(
        recovered.panel_epoch,
        valid.panel_epoch + 1,
        "the next shipped identity is the one the rejected frame did not \
         consume"
    );
}

// ---------------------------------------------------------------------------
// 41 (daemon half) — the daemon alone derives the grid
// ---------------------------------------------------------------------------

#[test]
fn acc41_row_clamping_preserves_the_stored_request() {
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*panel*", 12);
    assert_eq!(session.present().size.rows, 12, "the request fits at first");

    let panel = session.side_window().expect("side window");
    // A frame with room for the document minimum plus three panel rows.
    session.declare(2, 6, COLS);
    let clamped = session.present();
    assert_eq!(
        clamped.size.rows, 3,
        "the panel is clamped, not the document subtree"
    );
    assert_eq!(
        session.state.core.borrow().windows[&panel]
            .params
            .fixed_rows,
        Some(12),
        "41: the STORED request survives the clamp, so a later wider frame \
         can restore it"
    );

    session.declare(3, ROWS, COLS);
    assert_eq!(
        session.present().size.rows,
        12,
        "41: and it is restored exactly"
    );
}

#[test]
fn acc41_the_wire_area_budget_clamps_rows_and_can_hide_the_panel() {
    let max_cells = pmacs_protocol::panel::MAX_PANEL_VISIBLE_CELLS as u32;

    // Wide enough that only four rows fit inside the shared area bound.
    let mut session = Session::new();
    let cols = max_cells / 4;
    session.declare(1, 60, cols);
    open_panel(&session, "*panel*", 20);
    let frame = session.present();
    assert_eq!(frame.size.cols, cols);
    assert_eq!(
        frame.size.rows, 4,
        "41: rows are clamped by the shared wire-area budget, not only by \
         the layout"
    );
    frame
        .validate()
        .expect("41: a clamped frame is one the shared validator accepts");

    // Wide enough that not even the structural two-row floor fits.
    session.declare(2, 60, max_cells);
    assert_eq!(
        session.frame(),
        Some(PanelFramePayload::Absent),
        "41: when even two rows exceed the budget the panel follows the \
         Q#BP2b hidden arm"
    );
}

#[test]
fn acc41_degenerate_geometry_fails_closed_to_zero_usable_grid() {
    for (label, rows, cols) in [
        ("zero columns", ROWS, 0),
        ("zero rows", 0, COLS),
        ("a frame shorter than its own status row", 1, COLS),
    ] {
        let mut session = Session::new();
        session.declare(1, ROWS, COLS);
        open_panel(&session, "*panel*", 4);
        assert!(
            matches!(session.frame(), Some(PanelFramePayload::Present(_))),
            "{label}: fixture precondition — a band was visible first"
        );

        session.declare(2, rows, cols);
        assert_eq!(
            session.frame(),
            Some(PanelFramePayload::Absent),
            "{label}: declares zero usable geometry and hides, without \
             overflow or an oversized allocation"
        );
    }
}

// ---------------------------------------------------------------------------
// A2B-1 — the epoch state machine, row by row
// ---------------------------------------------------------------------------

#[test]
fn a2b1_the_semantic_acceptance_table_holds_row_by_row() {
    let session = Session::new();
    let total = CellSize::new(ROWS, COLS);

    assert_eq!(
        session.declare(0, ROWS, COLS),
        GeometryUpdate::Rejected,
        "epoch 0 is reserved for 'never declared' and is rejected on the wire"
    );
    assert_eq!(
        session.state.core.borrow().frame_geometry_for(FID),
        None,
        "…and stores nothing"
    );

    assert_eq!(session.declare(5, ROWS, COLS), GeometryUpdate::Advanced);
    assert_eq!(session.declare(5, ROWS, COLS), GeometryUpdate::Duplicate);
    assert_eq!(
        session.declare(5, ROWS, COLS + 1),
        GeometryUpdate::Rejected,
        "the same epoch with a different total is conflicting"
    );
    assert_eq!(
        session.declare(4, ROWS, COLS),
        GeometryUpdate::Rejected,
        "a LOWER epoch carrying identical data is still stale"
    );
    assert_eq!(
        session
            .state
            .core
            .borrow()
            .frame_geometry_for(FID)
            .map(|geometry| (geometry.geometry_epoch, geometry.total)),
        Some((5, total)),
        "every rejection left the stored declaration untouched"
    );

    assert_eq!(
        session.declare(6, ROWS, COLS),
        GeometryUpdate::Advanced,
        "Q#BP2S1: a greater epoch is accepted even when the total is \
         IDENTICAL — the font/scale case daemon-side value dedup cannot see"
    );
    assert_eq!(
        session
            .state
            .core
            .borrow()
            .frame_geometry_for(FID)
            .map(|geometry| geometry.geometry_epoch),
        Some(6),
        "…and it is stored verbatim"
    );
}

#[test]
fn a2b1_a_duplicate_reconciles_nothing_while_an_advance_does() {
    let session = Session::new();
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*panel*", 4);

    // Force the derived visibility cache to a value only reconciliation
    // can correct. `Duplicate` must leave it alone; `Advanced` must not.
    let force_hidden = |session: &Session| {
        session
            .state
            .core
            .borrow_mut()
            .views
            .get_mut(&FID)
            .expect("view")
            .panel_hidden = true;
    };

    force_hidden(&session);
    assert_eq!(session.declare(1, ROWS, COLS), GeometryUpdate::Duplicate);
    assert!(
        session.state.core.borrow().panel_hidden_for(FID),
        "a Duplicate returns without touching panel state"
    );

    assert_eq!(
        session.declare(2, ROWS, COLS),
        GeometryUpdate::Advanced,
        "…while an advance with the same total still reconciles"
    );
    assert!(
        !session.state.core.borrow().panel_hidden_for(FID),
        "Advanced ran the reconciliation the Duplicate skipped"
    );
}

#[test]
fn a2b1_a_rejected_declaration_reconciles_nothing() {
    let session = Session::new();
    session.declare(4, ROWS, COLS);
    open_panel(&session, "*panel*", 4);
    session
        .state
        .core
        .borrow_mut()
        .views
        .get_mut(&FID)
        .expect("view")
        .panel_hidden = true;

    assert_eq!(session.declare(3, ROWS, COLS), GeometryUpdate::Rejected);
    assert!(
        session.state.core.borrow().panel_hidden_for(FID),
        "a Rejected declaration is dropped BEFORE any reconciliation"
    );
}

#[test]
fn a2b1_grid_allocator_exhaustion_clears_the_declaration_and_hides() {
    // The grid/LOCAL allocator, which mints its own epochs. `LOCAL` is
    // panel-capable, so this is the production path for a TUI.
    let state = EditorState::new_with_roots(&crate::iso::roots());
    exec(&state, "pmacs.lsp.config = {}");
    state.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(ROWS, COLS));
    exec(
        &state,
        "pmacs.window.display(pmacs.buffer.create(\"*panel*\"), \
         { side = \"bottom\", height = 4 })",
    );
    assert!(
        !state.core.borrow().panel_hidden_for(FrontendId::LOCAL),
        "fixture precondition: the panel is visible before exhaustion"
    );

    // Drive the allocator to its last id.
    state
        .core
        .borrow_mut()
        .views
        .get_mut(&FrontendId::LOCAL)
        .expect("view")
        .frame_geometry = Some(pmacs::window::DeclaredFrameGeometry {
        geometry_epoch: u64::MAX,
        total: CellSize::new(ROWS, COLS),
    });

    // A REAL resize now needs an id the allocator cannot mint.
    assert_eq!(
        state.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(ROWS + 1, COLS)),
        GeometryUpdate::Rejected,
        "checked allocation refuses rather than pinning at u64::MAX, where \
         two different geometries would share one id"
    );
    assert_eq!(
        state.core.borrow().frame_geometry_for(FrontendId::LOCAL),
        None,
        "A2B-1: exhaustion CLEARS the authoritative declaration; retaining \
         the old one would keep painting a panel sized to a frame that no \
         longer exists"
    );
    assert!(
        state.core.borrow().panel_hidden_for(FrontendId::LOCAL),
        "A2B-1: unknown geometry is non-presentable, so the panel hides"
    );
    assert_eq!(
        state.core.borrow().panel_grid_size(FrontendId::LOCAL),
        None,
        "…and no stale-geometry grid is derivable afterwards"
    );
}

// ---------------------------------------------------------------------------
// 51 — a non-panel-capable semantic frontend gets no band at all
// ---------------------------------------------------------------------------

#[test]
fn acc51_a_pre_panel_semantic_frontend_is_never_sent_a_panel_frame() {
    let mut session = Session::new();
    session
        .state
        .core
        .borrow_mut()
        .views
        .get_mut(&FID)
        .expect("view")
        .panel_capable = false;
    session.declare(1, ROWS, COLS);
    // The Stage 1 fallback discards `side`, so this lands in the document
    // window; assert that directly rather than assuming it.
    open_panel(&session, "*panel*", 4);
    assert_eq!(
        session.side_window(),
        None,
        "51: capability fallback placed the buffer in a document window"
    );
    assert_eq!(
        session.frame(),
        None,
        "51: and no band message is produced for it in any case"
    );
}

/// A peer below [`PANEL_MIN_VERSION`] gets no `PanelFrame` even when the
/// daemon could build one for it.
///
/// **Anchored on `PANEL_MIN_VERSION - 1`, not `PROTOCOL_VERSION - 1`.**
/// The original spelling was the latter, which expressed an *absolute*
/// contract — "older than the version that introduced panel frames" —
/// as arithmetic on a *moving* constant. It held only while
/// `PROTOCOL_VERSION` happened to equal `PANEL_MIN_VERSION`, and the
/// long-lines lane's bump to v22 made `PROTOCOL_VERSION - 1` equal
/// `PANEL_MIN_VERSION` exactly: the fixture's "old" peer became
/// panel-capable, so the daemon correctly sent a frame and the test
/// correctly failed. The production code was never wrong.
///
/// `src/daemon.rs` and `pmacs-gpu/src/main.rs` already spell this
/// `PANEL_MIN_VERSION - 1` in five places; this was the one outlier.
#[test]
fn acc51_a_sub_panel_version_peer_is_sent_no_panel_frame_even_when_capable() {
    let mut session = Session::new();
    session.render = SemanticRenderState::for_peer(FID, PANEL_MIN_VERSION - 1);
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*panel*", 4);

    assert!(
        session.state.core.borrow().panel_grid_size(FID).is_some(),
        "fixture precondition: the daemon CAN derive a grid here"
    );
    assert_eq!(
        session.frame(),
        None,
        "51: a peer below the panel version receives no PanelFrame"
    );
}

// ---------------------------------------------------------------------------
// 42 — focusing the panel does not disturb the document projection
// ---------------------------------------------------------------------------

#[test]
fn acc42_focusing_the_panel_leaves_the_document_surface_alone() {
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*panel*", 4);
    let panel = session.side_window().expect("side window");
    let document_buffer = session.state.core.borrow().windows[&session.document].buffer_id;

    let unfocused = session.present();
    assert!(!unfocused.focused, "the panel does not own focus yet");

    session.state.core.borrow_mut().focus_window(FID, panel);
    let focused = session.present();
    assert!(focused.focused, "the frame reports the focus transition");

    let core = session.state.core.borrow();
    assert_eq!(
        core.primary_document_window(FID),
        Some(session.document),
        "42: the document surface is unchanged while the panel is focused"
    );
    assert_eq!(
        core.primary_document_buffer(FID),
        Some(document_buffer),
        "42: and still names the document buffer, not the panel's"
    );
}

// ---------------------------------------------------------------------------
// 52 — the panel honors the OWNING frontend's fold projection
// ---------------------------------------------------------------------------

#[test]
fn acc52_a_non_projecting_frontend_sees_every_source_line_in_its_panel() {
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*panel*", 6);
    set_panel_text(&session, "alpha\nbravo\ncharlie\ndelta");
    exec(
        &session.state,
        "assert(pmacs.fold.fold(PANEL_BUF, { start = 0, ['end'] = 17 }))",
    );

    let rows = rows_of(&session.present());
    let painted = rows.join("\n");
    for line in ["alpha", "bravo", "charlie"] {
        assert!(
            painted.contains(line),
            "52: fold_projection = false means the panel collapses nothing; \
             {line:?} is missing from\n{painted}"
        );
    }

    // The discriminating half: the same fold DOES collapse for a
    // projecting frontend, so the assertion above is not vacuous.
    session
        .state
        .core
        .borrow_mut()
        .views
        .get_mut(&FID)
        .expect("view")
        .fold_projection = true;
    let projected = rows_of(&session.present()).join("\n");
    assert!(
        !projected.contains("bravo"),
        "sanity: with projection on, the folded lines really do disappear \
         from\n{projected}"
    );
}

// ---------------------------------------------------------------------------
// 45 — one provider invocation supplies both surfaces
// ---------------------------------------------------------------------------

#[test]
fn acc45_one_statusline_invocation_serves_the_document_and_the_panel() {
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*panel*", 4);
    let panel_buffer = {
        let core = session.state.core.borrow();
        let side = core.side_window_for(FID).expect("side window");
        core.windows[&side].buffer_id
    };
    // The semantic fan-out is keyed on the DECLARED viewport buffer, so
    // the document half only runs once the frontend has declared one.
    let document_buffer = session.state.core.borrow().windows[&session.document].buffer_id;
    session.render.set_viewport(
        document_buffer,
        pmacs::protocol::ByteRange { start: 0, end: 0 },
        0,
    );

    exec(
        &session.state,
        "CALLS = 0
         pmacs.statusline.register {
           name = 'probe', side = 'left',
           fn = function(ctx) CALLS = CALLS + 1; return 'W' .. tostring(ctx.window) end,
         }",
    );

    let messages = session.render.render_frame(&session.state);
    let calls: u32 = session
        .state
        .lua_host
        .lua()
        .load("return CALLS")
        .eval()
        .expect("counter");
    assert_eq!(
        calls, 2,
        "45: exactly one invocation per visible context — the primary \
         document and the visible side window — and no second evaluation \
         for the band"
    );

    let panel_rows = messages
        .iter()
        .find_map(|message| match message {
            InstanceMessage::PanelFrame(PanelFramePayload::Present(frame)) => Some(rows_of(frame)),
            _ => None,
        })
        .expect("a panel frame");
    let mode_line = panel_rows.last().expect("mode line").clone();
    let panel_name = {
        let core = session.state.core.borrow();
        let reg = core.registry.borrow();
        reg.get(panel_buffer)
            .expect("panel buffer")
            .name()
            .to_owned()
    };
    assert!(
        mode_line.contains(&panel_name),
        "45: the band's mode line carries the SIDE window's provider text, \
         not the document's; got {mode_line:?}"
    );
}

// ---------------------------------------------------------------------------
// The producer never touches a passive panel's scroll state
// ---------------------------------------------------------------------------

#[test]
fn a_passive_panel_keeps_its_view_top_while_a_focused_one_scrolls_to_its_caret() {
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*panel*", 4);
    let panel = session.side_window().expect("side window");
    let body = vec!["line-of-text"; 40].join("\n");
    let last_byte = body.len() as u64;
    set_panel_text(&session, &body);

    // Put the panel's caret far below its viewport while it is PASSIVE.
    {
        let mut core = session.state.core.borrow_mut();
        let window = core.windows.get_mut(&panel).expect("panel window");
        window.cursor = last_byte;
        window.view_top = 0;
    }
    let _ = session.present();
    assert_eq!(
        session.state.core.borrow().windows[&panel].view_top,
        0,
        "a passive panel's view_top is not moved by the projection"
    );

    session.state.core.borrow_mut().focus_window(FID, panel);
    let _ = session.present();
    assert!(
        session.state.core.borrow().windows[&panel].view_top > 0,
        "…while a FOCUSED panel runs the shared auto-scroll clamp"
    );
}

// ---------------------------------------------------------------------------
// The band rides the terminal document path too
// ---------------------------------------------------------------------------

#[test]
fn the_band_is_projected_even_when_the_document_surface_is_a_terminal() {
    // A frontend with no declared byte viewport takes neither the
    // document nor the terminal pass; the band must still be produced,
    // or the first panel would be unpaintable.
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*panel*", 4);
    assert!(
        session.render.panel_declaration().is_none(),
        "fixture precondition: nothing shipped yet"
    );
    let frame = session.present();
    assert_eq!(frame.size.cols, COLS);
    assert!(
        session.render.panel_declaration().is_some(),
        "the declaration is recorded for the inbound validation ladder"
    );
}

// ---------------------------------------------------------------------------
// Sanity: the projected cells really are the panel window's content
// ---------------------------------------------------------------------------

#[test]
fn the_projection_paints_the_side_windows_own_buffer() {
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*panel*", 4);
    set_panel_text(&session, "panel-content");
    let frame = session.present();
    let rows = rows_of(&frame);
    assert_eq!(rows.len(), frame.size.rows as usize);
    assert!(
        rows[0].starts_with("panel-content"),
        "the first row is the panel buffer's first line, got {:?}",
        rows[0]
    );
    assert_eq!(
        frame.cells.len(),
        (frame.size.rows * frame.size.cols) as usize,
        "exactly size.area() cells"
    );
    frame.validate().expect("a produced frame is a valid frame");
    let _ = HashMap::<u32, u32>::new();
}

// ---------------------------------------------------------------------------
// Review round 1 — R1-1 and R2-4
// ---------------------------------------------------------------------------

/// R1-1: exhausting the wire-area budget must be a **durable** hide, not
/// a per-frame one.
///
/// Q#BP2b is explicit that hiding is a durable state transition: a
/// render-time dodge still routes keys to an invisible window and leaves
/// the terminal controller claimed. `panel_grid_size` gained a budget
/// clamp that `reconcile_panel_layout_core` did not share, so the band
/// went `Absent` on the wire while `panel_hidden` stayed false — the
/// exact per-frame-effect shape the Stage 1 record warns about.
#[test]
fn r1_1_wire_area_exhaustion_is_a_durable_hide_not_a_blank_frame() {
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*panel*", 4);
    let panel = session.side_window().expect("side window");
    session.state.core.borrow_mut().focus_window(FID, panel);
    assert!(
        session.present().focused,
        "fixture precondition: the panel owns focus while it is presentable"
    );

    // Wide enough that not even the structural two-row floor fits inside
    // the shared area bound.
    let max_cells = u32::try_from(pmacs_protocol::panel::MAX_PANEL_VISIBLE_CELLS).expect("bound");
    session.declare(2, ROWS, max_cells);

    assert_eq!(
        session.frame(),
        Some(PanelFramePayload::Absent),
        "the band is cleared authoritatively"
    );
    let core = session.state.core.borrow();
    assert!(
        core.panel_hidden_for(FID),
        "R1-1: …and the DURABLE hidden state must move with it, or keys \
         keep reaching an invisible window"
    );
    assert_ne!(
        core.views[&FID].active, panel,
        "R1-1: focus must leave a panel that can no longer be presented"
    );
}

/// R1-1's controller half: a panel terminal that stops being presentable
/// must have its child released, because the resize path merely returns
/// on zero content without releasing anything.
#[test]
fn r1_1_wire_area_exhaustion_releases_a_panel_terminals_controller() {
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    exec(
        &session.state,
        "TERM_BUF = pmacs.terminal.open { command = \"/bin/sh\", \
           args = { \"-c\", \"sleep 30\" }, display = \"panel\" }",
    );
    let panel = session.side_window().expect("terminal panel");
    let buffer: pmacs::lua_bindings::BufferIdLua = session
        .state
        .lua_host
        .lua()
        .load("return TERM_BUF")
        .eval()
        .unwrap();
    session.state.core.borrow_mut().focus_window(FID, panel);
    let _ = session.present();
    let key = pmacs::terminal::TerminalViewKey::new(FID, panel, buffer.0);
    assert!(
        session
            .state
            .terminal_manager
            .borrow_mut()
            .claim_controller(key),
        "fixture precondition: the panel view controls the child"
    );

    let max_cells = u32::try_from(pmacs_protocol::panel::MAX_PANEL_VISIBLE_CELLS).expect("bound");
    session.declare(2, ROWS, max_cells);
    assert_eq!(session.frame(), Some(PanelFramePayload::Absent));

    assert!(
        session
            .state
            .terminal_manager
            .borrow()
            .controller(buffer.0)
            .is_none(),
        "R1-1: the child's controller is released with the durable hide"
    );
    exec(&session.state, "pmacs.terminal.terminate(TERM_BUF)");
}

/// R2-4: `NoMessage` means publish **nothing**, so the band keeps the
/// text it last published. Treating it like `Invalidated` *removes*
/// provider text on a transient buffer-follow mismatch.
#[test]
fn r2_4_nomessage_retains_the_bands_published_segments() {
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*panel*", 4);
    let panel = session.side_window().expect("side window");
    let document_buffer = session.state.core.borrow().windows[&session.document].buffer_id;
    session.render.set_viewport(
        document_buffer,
        pmacs::protocol::ByteRange { start: 0, end: 0 },
        0,
    );
    exec(
        &session.state,
        "pmacs.statusline.register {
           name = 'probe', side = 'left',
           fn = function(ctx) return 'W' .. tostring(ctx.window) end,
         }",
    );

    let marker = format!("W{}", panel.raw());
    let mode_line = |session: &mut Session| {
        rows_of(&session.present())
            .last()
            .expect("mode line")
            .clone()
    };
    assert!(
        mode_line(&mut session).contains(&marker),
        "fixture precondition: a Ready evaluation paints the band's own \
         provider text"
    );

    // A buffer-follow mismatch: the primary document window moves off the
    // buffer the frontend declared, so phase 1 is already stale and the
    // evaluator returns NoMessage.
    exec(
        &session.state,
        "pmacs.window.switch_buffer(pmacs.buffer.create(\"*elsewhere*\"))",
    );
    // Force a repaint: without a content change the payload would be
    // duplicate-suppressed and this would assert nothing.
    set_panel_text(&session, "changed");
    assert!(
        mode_line(&mut session).contains(&marker),
        "R2-4: NoMessage publishes nothing, so the band keeps its last \
         published segments"
    );

    // The discriminating half: `Invalidated` DOES clear them, because a
    // callback that mutated the registry invalidates all evaluated text.
    // The evaluation has to reach the callback phase first, so re-declare
    // the viewport onto the buffer the document window now shows —
    // otherwise phase 1 stays stale and this would still be NoMessage.
    let elsewhere = session.state.core.borrow().windows[&session.document].buffer_id;
    session.render.set_viewport(
        elsewhere,
        pmacs::protocol::ByteRange { start: 0, end: 0 },
        0,
    );
    exec(
        &session.state,
        "SELF = pmacs.statusline.register {
           name = 'self-unregistering', side = 'right',
           fn = function() pmacs.statusline.unregister(SELF); return 'STALE' end,
         }",
    );
    set_panel_text(&session, "changed again");
    let after = mode_line(&mut session);
    assert!(
        !after.contains(&marker) && !after.contains("STALE"),
        "an Invalidated evaluation discards ALL callback text, got {after:?}"
    );
}

/// R2-4's retained baseline follows one window-and-buffer presentation,
/// not merely the side `WindowId`. Side affinity reuses the window when it
/// replaces the buffer, so `NoMessage` must not carry provider text from
/// the predecessor into the replacement.
#[test]
fn r2_4_nomessage_does_not_cross_a_same_window_buffer_replacement() {
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*first-panel*", 4);
    let panel = session.side_window().expect("side window");
    let document_buffer = session.state.core.borrow().windows[&session.document].buffer_id;
    session.render.set_viewport(
        document_buffer,
        pmacs::protocol::ByteRange { start: 0, end: 0 },
        0,
    );
    exec(
        &session.state,
        "pmacs.statusline.register {
           name = 'buffer-probe', side = 'left',
           fn = function(ctx) return 'CTX:' .. ctx.buffer:name() end,
         }",
    );

    let first = rows_of(&session.present())
        .last()
        .expect("mode line")
        .clone();
    assert!(
        first.contains("CTX:*first-panel*"),
        "fixture precondition: the first panel publishes its buffer-scoped \
         segment; got {first:?}"
    );

    // Replace the buffer in the existing side WindowId, then move the
    // document off its declared viewport in the same transaction so phase
    // 1 returns NoMessage before it can publish the replacement context.
    exec(
        &session.state,
        "SECOND = pmacs.buffer.create('*second-panel*')
         pmacs.window.display(SECOND, { side = 'bottom' })
         pmacs.window.switch_buffer(pmacs.buffer.create('*elsewhere*'))",
    );
    assert_eq!(
        session.side_window(),
        Some(panel),
        "fixture precondition: side-buffer replacement reuses the WindowId"
    );

    let replaced = rows_of(&session.present())
        .last()
        .expect("mode line")
        .clone();
    assert!(
        replaced.contains("*second-panel*"),
        "fixture precondition: the replacement panel was painted; got {replaced:?}"
    );
    assert!(
        !replaced.contains("CTX:*first-panel*"),
        "NoMessage must not carry buffer-scoped segments across a new panel \
         presentation; got {replaced:?}"
    );
}

/// An authoritative `Absent` clears the peer's retained band, including
/// its mode line. Reopening the same panel under `NoMessage` therefore
/// starts with no provider segments to retain.
#[test]
fn r2_4_nomessage_does_not_resurrect_segments_after_absent() {
    let mut session = Session::new();
    session.declare(1, ROWS, COLS);
    open_panel(&session, "*panel*", 4);
    let document_buffer = session.state.core.borrow().windows[&session.document].buffer_id;
    session.render.set_viewport(
        document_buffer,
        pmacs::protocol::ByteRange { start: 0, end: 0 },
        0,
    );
    exec(
        &session.state,
        "pmacs.statusline.register {
           name = 'absence-probe', side = 'left',
           fn = function() return 'BEFORE-ABSENT' end,
         }",
    );
    assert!(
        rows_of(&session.present())
            .last()
            .expect("mode line")
            .contains("BEFORE-ABSENT"),
        "fixture precondition: the segment was published"
    );

    exec(
        &session.state,
        "pmacs.window.switch_buffer(pmacs.buffer.create('*elsewhere*'))",
    );
    session.declare(2, 3, COLS);
    assert_eq!(
        session.frame(),
        Some(PanelFramePayload::Absent),
        "fixture precondition: the peer's panel state was cleared"
    );

    session.declare(3, ROWS, COLS);
    let reappeared = rows_of(&session.present())
        .last()
        .expect("mode line")
        .clone();
    assert!(
        !reappeared.contains("BEFORE-ABSENT"),
        "NoMessage cannot retain across Absent because the peer has no panel \
         statusline state left to retain; got {reappeared:?}"
    );
}

// ---------------------------------------------------------------------------
// Review round 1 sweep — the same defect shape, found elsewhere
// ---------------------------------------------------------------------------

/// Sweep result: a panel **wider than the terminal subsystem's per-axis
/// cap** hosting a terminal reproduced R1-1's shape all over again.
///
/// Bet B5' makes a panel wider than 512 columns legal on the wire — a 4K
/// surface at a small font is ordinary, and the frontend declares that
/// width itself. But the terminal screen keeps its own PTY policy
/// (`MAX_TERMINAL_COLS`), so `snapshot_for_view` refused the panel's
/// content rect, the projection returned `None`, and the band went
/// **per-frame `Absent` while `panel_hidden` stayed false** — keys still
/// reaching an invisible window, controller still claimed.
///
/// The band is legitimately that wide, so hiding it would be the wrong
/// answer: the terminal projects into the columns it can occupy and the
/// remainder is band background, exactly as a snapshot narrower than its
/// window already paints.
#[test]
fn sweep_a_panel_wider_than_the_terminal_cap_still_presents_its_terminal() {
    let wide = u32::from(pmacs::terminal::MAX_TERMINAL_COLS) + 88;
    let mut session = Session::new();
    session.declare(1, ROWS, wide);
    exec(
        &session.state,
        "TERM_BUF = pmacs.terminal.open { command = \"/bin/sh\", \
           args = { \"-c\", \"sleep 30\" }, display = \"panel\" }",
    );
    let panel = session.side_window().expect("terminal panel");
    session.state.core.borrow_mut().focus_window(FID, panel);

    let frame = match session.frame() {
        Some(PanelFramePayload::Present(frame)) => frame,
        other => panic!(
            "a legally wide panel must still present its terminal; got {other:?} \
             — and the durable state says hidden={}",
            session.state.core.borrow().panel_hidden_for(FID)
        ),
    };
    assert_eq!(
        frame.size.cols, wide,
        "the band keeps the width the frontend declared (Bet B5')"
    );
    frame
        .validate()
        .expect("and it is a frame the shared validator accepts");

    // The durable state and the wire agree — which is the property R1-1
    // was really about.
    assert!(
        !session.state.core.borrow().panel_hidden_for(FID),
        "a presented band is not durably hidden"
    );
    assert_eq!(
        session.state.core.borrow().views[&FID].active,
        panel,
        "…and focus is still legitimately in it"
    );
    exec(&session.state, "pmacs.terminal.terminate(TERM_BUF)");
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;

// ---------------------------------------------------------------------------
// §5b G1–G4 — the authoritative cell-mapping key
//
// The key is derived from a FINGERPRINT of the inverse mapping's inputs,
// so the changing/stable split is structural: an input that is hashed
// moves the key by construction, and one that is not cannot. These rows
// pin each input individually, because a single "it changed" row cannot
// show WHICH input moved it — and a key that silently ignored, say,
// `view_left` would pass every row that only scrolls vertically.
// ---------------------------------------------------------------------------

/// Edit the panel's buffer from OUTSIDE any gesture — the "foreign
/// edit" the ladder cannot see. Done in Rust rather than Lua because it
/// must be a plain content mutation with no view, cursor or command
/// state attached to it.
fn foreign_edit(session: &Session, text: &str) {
    let core = session.state.core.borrow();
    let side = core.side_window_for(FID).expect("a side window");
    let buffer_id = core.windows[&side].buffer_id;
    let registry = core.registry.clone();
    let mut reg = registry.borrow_mut();
    let buffer = reg.get_mut(buffer_id).expect("the panel's buffer");
    buffer
        .set_generated_contents(text.as_bytes())
        .expect("a generated-contents write is a plain content change");
}

/// The key as the daemon would compute it for `FID`, advancing on change.
fn mapping_generation(session: &mut Session) -> Option<u64> {
    let fingerprint = session.state.panel_mapping_fingerprint(FID);
    session.render.panel_mapping_generation(fingerprint)
}

/// §5b G1 — a **foreign** edit before the next render moves the key.
///
/// This is the case the whole slice exists for: the epoch ladder cannot
/// see it. No buffer is replaced, no panel reopens, no geometry is
/// re-declared — every epoch holds — and yet the byte under a cell has
/// changed. It is also why the key must be derived on demand rather than
/// from the last emitted frame: nothing has rendered here.
#[test]
fn g1_a_foreign_edit_moves_the_mapping_key_before_anything_renders() {
    let mut session = Session::new();
    open_panel(&session, "g1", 4);
    session.declare(1, 24, 80);
    let _ = session.present();

    let before = mapping_generation(&mut session).expect("a presentable panel has a key");
    assert!(
        before >= 1,
        "a live key is never zero — zero is the wire's invalid value"
    );

    // An edit from somewhere other than the gesture's frontend, with no
    // render in between.
    foreign_edit(&session, "foreign edit\n");

    let after = mapping_generation(&mut session).expect("still presentable");
    assert!(
        after > before,
        "a foreign edit changes which byte a cell means, and no epoch \
         moves with it — this is the hole the ladder cannot close"
    );
}

/// §5b G3 — the **stable** inputs, one row each.
///
/// Every entry here is something that repaints a panel without changing
/// which byte a cell denotes. A drag provokes selection repaints on every
/// motion, so a key that moved with them would cancel the gesture it
/// exists to protect after a single step.
#[test]
fn g3_repaints_that_cannot_move_a_byte_leave_the_key_alone() {
    let mut session = Session::new();
    open_panel(&session, "g3", 4);
    session.declare(1, 24, 80);
    let _ = session.present();

    let baseline = mapping_generation(&mut session).expect("a key");

    // Re-reading with nothing changed at all.
    assert_eq!(
        mapping_generation(&mut session),
        Some(baseline),
        "an idle re-read must not advance the key, or every frame would \
         cancel every gesture"
    );

    // Cursor motion the follow rules absorb: the caret moves inside the
    // viewport, so no origin moves with it. Set directly, so nothing but
    // the cursor changes.
    {
        let mut core = session.state.core.borrow_mut();
        let side = core.side_window_for(FID).expect("a side window");
        let window = core.windows.get_mut(&side).expect("the side window");
        window.cursor = 0;
    }
    assert_eq!(
        mapping_generation(&mut session),
        Some(baseline),
        "cursor motion that moves no origin is not a mapping change"
    );
}

/// §5b G4a — a **selection-only** repaint preserves the key.
///
/// Split from G3 because it is the one the lifecycle depends on: G4b —
/// that an in-flight drag then continues through real replay — is owed by
/// the rebased replay lane, which is the only branch where replay exists.
#[test]
fn g4a_a_selection_only_repaint_preserves_the_mapping_key() {
    let mut session = Session::new();
    open_panel(&session, "g4a", 4);
    session.declare(1, 24, 80);
    let _ = session.present();

    let baseline = mapping_generation(&mut session).expect("a key");
    foreign_edit(&session, "alpha beta\n");
    let after_edit = mapping_generation(&mut session).expect("a key");
    assert!(after_edit > baseline, "the edit itself is a mapping change");

    // Now a selection, with no content or viewport change.
    {
        let mut core = session.state.core.borrow_mut();
        let side = core.side_window_for(FID).expect("a side window");
        let window = core.windows.get_mut(&side).expect("the side window");
        window.selection = Some(pmacs::window::Selection { anchor: 0 });
        window.cursor = 5;
    }
    assert_eq!(
        mapping_generation(&mut session),
        Some(after_edit),
        "a selection changes what is HIGHLIGHTED, never what a cell \
         denotes — and a drag repaints the selection on every motion"
    );
}

/// §5b — the key is a **high-water mark** and survives `Absent`.
///
/// Hiding the band clears input authority, but it must not reset the
/// generation: a frame delayed across the hide would otherwise return
/// with a lower value and be believed.
#[test]
fn the_mapping_key_never_moves_backward_across_a_hidden_panel() {
    let mut session = Session::new();
    open_panel(&session, "hw", 4);
    session.declare(1, 24, 80);
    let _ = session.present();

    let before = mapping_generation(&mut session).expect("a key");
    foreign_edit(&session, "one\n");
    let peak = mapping_generation(&mut session).expect("a key");
    assert!(peak > before);

    // Hide it: no fingerprint, so no advance — and no reset either.
    {
        let mut core = session.state.core.borrow_mut();
        core.views
            .get_mut(&FID)
            .expect("the frontend's view")
            .panel_hidden = true;
    }
    assert_eq!(
        mapping_generation(&mut session),
        None,
        "no presentable panel means no key to stamp, which is not the \
         same as a key of zero"
    );
    assert_eq!(
        session.render.panel_mapping_generation_peek(),
        Some(peak),
        "the high-water mark SURVIVES the hide — clearing it would let a \
         delayed frame roll the producer's authority backward"
    );
}

/// §5b G2 — **every changing input, one leg each.**
///
/// Enumerated rather than asserted in aggregate, and mutation testing is
/// what forced it: with only the content-edit row present, dropping
/// `view_left` from the key and collapsing the grid to `rows * cols`
/// both stayed GREEN. A single "the key moved" row cannot show *which*
/// input moved it, and a key that ignores horizontal scrolling passes
/// every row that only scrolls vertically.
#[test]
fn g2_each_input_of_the_inverse_mapping_moves_the_key_on_its_own() {
    // Each leg names one input and touches only that input.
    type Leg = (&'static str, fn(&Session));

    let legs: &[Leg] = &[
        ("view_top", |session| {
            let mut core = session.state.core.borrow_mut();
            let side = core.side_window_for(FID).expect("side");
            core.windows.get_mut(&side).expect("win").view_top += 1;
        }),
        ("view_left — GUI arc 1b makes this real", |session| {
            let mut core = session.state.core.borrow_mut();
            let side = core.side_window_for(FID).expect("side");
            core.windows.get_mut(&side).expect("win").view_left += 1;
        }),
        ("wrap mode", |session| {
            let mut core = session.state.core.borrow_mut();
            let side = core.side_window_for(FID).expect("side");
            let window = core.windows.get_mut(&side).expect("win");
            window.last_wrap = match window.last_wrap {
                pmacs::view::WrapMode::Wrap => pmacs::view::WrapMode::Truncate,
                pmacs::view::WrapMode::Truncate => pmacs::view::WrapMode::Wrap,
            };
        }),
        (
            "content columns — the gutter is subtracted here",
            |session| {
                let mut core = session.state.core.borrow_mut();
                let side = core.side_window_for(FID).expect("side");
                core.windows.get_mut(&side).expect("win").last_content_cols += 1;
            },
        ),
        ("fold PROJECTION POLICY, owned by the view", |session| {
            let mut core = session.state.core.borrow_mut();
            let view = core.views.get_mut(&FID).expect("view");
            view.fold_projection = !view.fold_projection;
        }),
        ("fold CONTENT, owned by the buffer", |session| {
            let core = session.state.core.borrow();
            let side = core.side_window_for(FID).expect("side");
            let buffer_id = core.windows[&side].buffer_id;
            let registry = core.registry.clone();
            let mut reg = registry.borrow_mut();
            let buffer = reg.get_mut(buffer_id).expect("buffer");
            let store = core.fold_registry.store_or_attach(buffer);
            store
                .lock()
                .expect("fold store mutex")
                .insert(pmacs_protocol::ByteRange { start: 0, end: 1 });
        }),
    ];

    for (name, mutate) in legs {
        let mut session = Session::new();
        open_panel(&session, "g2", 4);
        session.declare(1, 24, 80);
        let _ = session.present();

        let before = mapping_generation(&mut session).expect("a key");
        mutate(&session);
        let after = mapping_generation(&mut session).expect("a key");
        assert!(
            after > before,
            "changing {name} changes which byte a cell means, so the key \
             must move; it did not ({before} → {after})"
        );
    }
}

/// §5b G2 — grid **rows** and **columns** each move the key.
///
/// **Honest limit, recorded because mutation testing found it:** these
/// two legs do NOT discriminate `rows`/`cols` from their product.
/// Collapsing the key to `rows * cols` leaves both GREEN, because
/// `last_content_cols` co-varies with a column change and the panel's
/// row count co-varies with a resize — the key still moves, by another
/// input. Only a **transposition** (2×6 → 6×2, identical product) would
/// isolate it, and no production path reaches one: rows come from the
/// band's height and columns from the frame declaration, and nothing
/// swaps them.
///
/// The key hashes them separately anyway. That is cheap and correct,
/// and the alternative — hashing a product because no test can currently
/// tell the difference — would be choosing the weaker construction for
/// the convenience of the test suite. What these legs *do* pin is that
/// each dimension moves the key at all, which is what the rest of the
/// slice depends on.
#[test]
fn g2_grid_rows_and_columns_are_independent_inputs() {
    // ROWS come from the band's own height, not the frame's total rows —
    // declaring a shorter frame leaves a 4-row panel a 4-row panel. The
    // resize path is the one that actually changes them.
    {
        let mut session = Session::new();
        open_panel(&session, "g2rows", 4);
        session.declare(1, 24, 80);
        let _ = session.present();

        let before = mapping_generation(&mut session).expect("a key");
        assert!(
            session.state.apply_panel_resize_rows(FID, 6),
            "the resize must be accepted, or this leg proves nothing"
        );
        let after = mapping_generation(&mut session).expect("a key");
        assert!(
            after > before,
            "changing grid ROWS alone must move the key — an area product \
             would miss a transposition"
        );
    }

    // COLUMNS come from the declaration.
    {
        let mut session = Session::new();
        open_panel(&session, "g2cols", 4);
        session.declare(1, 24, 80);
        let _ = session.present();

        let before = mapping_generation(&mut session).expect("a key");
        session.declare(2, 24, 40);
        let after = mapping_generation(&mut session).expect("a key");
        assert!(
            after > before,
            "changing grid COLUMNS alone must move the key"
        );
    }
}
