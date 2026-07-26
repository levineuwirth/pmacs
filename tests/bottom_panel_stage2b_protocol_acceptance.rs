//! Bottom-panel Stage 2B — the v21 protocol slice.
//!
//! Covers parent acceptance 37 (round-trip plus the two byte pins) and
//! the shared/terminal-only validator split of Q#BP15. The daemon
//! projection, the epoch state machine, and the GPU band are later
//! slices of this stage and are not exercised here.

use pmacs_protocol::cell::{Cell, CellCoord, CellSize, Glyph, Style};
use pmacs_protocol::message::{FrontendEvent, InstanceMessage, Modifiers, MouseButton, MouseKind};
use pmacs_protocol::panel::{PanelFrame, PanelFrameError, PanelFramePayload};
use pmacs_protocol::terminal::{
    MAX_TERMINAL_COLS, TerminalFrame, TerminalFrameError, TerminalProcessState,
};
use pmacs_protocol::{BufferId, FrontendId, PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS};

fn cell(ch: char) -> Cell {
    Cell {
        glyph: Glyph::Char(ch),
        style: Style::default(),
        attachment: None,
    }
}

fn panel_frame(rows: u32, cols: u32) -> PanelFrame {
    PanelFrame {
        buffer_id: BufferId::from_raw(9),
        panel_epoch: 3,
        geometry_epoch: 5,
        size: CellSize::new(rows, cols),
        cells: vec![cell(' '); (rows * cols) as usize],
        cursor: Some(CellCoord::new(0, 0)),
        focused: true,
    }
}

fn terminal_frame(rows: u32, cols: u32) -> TerminalFrame {
    TerminalFrame {
        buffer_id: BufferId::from_raw(9),
        size: CellSize::new(rows, cols),
        cells: vec![cell(' '); (rows * cols) as usize],
        cursor: Some(CellCoord::new(0, 0)),
        title: None,
        screen_generation: 1,
        selection: Vec::new(),
        scroll_offset: 0,
        at_bottom: true,
        pid: 1,
        process: TerminalProcessState::Running,
    }
}

// ---------------------------------------------------------------------------
// 37 — version and round-trip
// ---------------------------------------------------------------------------

#[test]
fn the_panel_stage_takes_protocol_v21() {
    assert_eq!(PROTOCOL_VERSION, 21);
    assert!(SUPPORTED_PROTOCOL_VERSIONS.contains(&21));
    // v20 stays supported: a v20 peer interoperates with panel traffic
    // simply absent rather than being refused the handshake.
    assert!(SUPPORTED_PROTOCOL_VERSIONS.contains(&20));
}

#[test]
fn a_present_panel_frame_round_trips_with_both_epochs() {
    let frame = panel_frame(2, 3);
    let msg = InstanceMessage::PanelFrame(PanelFramePayload::Present(frame.clone()));
    let bytes = postcard::to_allocvec(&msg).expect("encode");
    let decoded: InstanceMessage = postcard::from_bytes(&bytes).expect("decode");
    let InstanceMessage::PanelFrame(PanelFramePayload::Present(got)) = decoded else {
        panic!("expected a Present panel frame, got {decoded:?}");
    };
    // Both epochs must survive: they are the identities every later
    // panel event validates against, so a frame that round-trips its
    // cells but drops an epoch would silently accept stale input.
    assert_eq!(got.panel_epoch, frame.panel_epoch);
    assert_eq!(got.geometry_epoch, frame.geometry_epoch);
    assert_eq!(got.buffer_id, frame.buffer_id);
    assert_eq!(got.size, frame.size);
    assert_eq!(got.cells, frame.cells);
    assert_eq!(got.cursor, frame.cursor);
    assert_eq!(got.focused, frame.focused);
}

#[test]
fn an_absent_panel_payload_round_trips_as_its_own_state() {
    let msg = InstanceMessage::PanelFrame(PanelFramePayload::Absent);
    let bytes = postcard::to_allocvec(&msg).expect("encode");
    let decoded: InstanceMessage = postcard::from_bytes(&bytes).expect("decode");
    assert!(matches!(
        decoded,
        InstanceMessage::PanelFrame(PanelFramePayload::Absent)
    ));
    // Absent must be distinguishable from a Present frame carrying no
    // cells: it is authoritative, and conflating the two would make
    // "hide the band" indistinguishable from "paint an empty band".
    let empty_present = InstanceMessage::PanelFrame(PanelFramePayload::Present(panel_frame(1, 1)));
    assert_ne!(
        postcard::to_allocvec(&empty_present).expect("encode"),
        bytes
    );
}

#[test]
fn the_three_panel_events_round_trip() {
    let fid = FrontendId(4);
    let events = vec![
        FrontendEvent::FrontendCellGeometry {
            frontend_id: fid,
            geometry_epoch: 1,
            total: CellSize::new(40, 120),
        },
        FrontendEvent::PanelResizeRows {
            frontend_id: fid,
            geometry_epoch: 2,
            panel_epoch: 7,
            rows: 12,
        },
        FrontendEvent::PanelPointer {
            frontend_id: fid,
            geometry_epoch: 2,
            panel_epoch: 7,
            coord: CellCoord::new(3, 9),
            kind: MouseKind::Down(MouseButton::Left),
            mods: Modifiers::default(),
        },
    ];
    for event in events {
        let bytes = postcard::to_allocvec(&event).expect("encode");
        let decoded: FrontendEvent = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(decoded, event);
        assert_eq!(decoded.frontend_id(), fid);
    }
}

// ---------------------------------------------------------------------------
// 37 — byte pins on the previous final variant of each extended enum
// ---------------------------------------------------------------------------

#[test]
fn appending_panel_frame_does_not_move_the_previous_final_instance_discriminant() {
    // `InitialTargetResult` was the final v20 variant. Its encoding must
    // be byte-identical after `PanelFrame` is appended; if the new
    // variant were inserted anywhere earlier, this leading discriminant
    // byte would shift and every v20 peer would misread the wire.
    let msg = InstanceMessage::InitialTargetResult(
        pmacs_protocol::message::InitialTargetResult::Opened {
            buffer_id: BufferId::from_raw(1),
        },
    );
    let bytes = postcard::to_allocvec(&msg).expect("encode");
    assert_eq!(
        bytes[0], 27,
        "InitialTargetResult must stay discriminant 27; got {bytes:?}"
    );
    // And the appended variant must be the next one, not a reused slot.
    let panel = InstanceMessage::PanelFrame(PanelFramePayload::Absent);
    let panel_bytes = postcard::to_allocvec(&panel).expect("encode");
    assert_eq!(panel_bytes[0], 28);
}

#[test]
fn appending_panel_events_does_not_move_the_previous_final_event_discriminant() {
    // `TerminalPointer` was the final v19/v20 variant of `FrontendEvent`.
    let event = FrontendEvent::TerminalPointer {
        frontend_id: FrontendId(2),
        buffer_id: BufferId::from_raw(3),
        coord: CellCoord::new(1, 1),
        kind: MouseKind::Down(MouseButton::Left),
        mods: Modifiers::default(),
    };
    let bytes = postcard::to_allocvec(&event).expect("encode");
    assert_eq!(
        bytes[0], 12,
        "TerminalPointer must stay discriminant 12; got {bytes:?}"
    );
    // The three appended events take the next three slots, in order.
    let fid = FrontendId(2);
    for (expected, event) in [
        (
            13u8,
            FrontendEvent::FrontendCellGeometry {
                frontend_id: fid,
                geometry_epoch: 1,
                total: CellSize::new(1, 1),
            },
        ),
        (
            14,
            FrontendEvent::PanelResizeRows {
                frontend_id: fid,
                geometry_epoch: 1,
                panel_epoch: 1,
                rows: 1,
            },
        ),
        (
            15,
            FrontendEvent::PanelPointer {
                frontend_id: fid,
                geometry_epoch: 1,
                panel_epoch: 1,
                coord: CellCoord::new(0, 0),
                kind: MouseKind::Down(MouseButton::Left),
                mods: Modifiers::default(),
            },
        ),
    ] {
        let bytes = postcard::to_allocvec(&event).expect("encode");
        assert_eq!(bytes[0], expected, "wrong discriminant for {event:?}");
    }
}

// ---------------------------------------------------------------------------
// 39 — the shared/terminal-only validator split
// ---------------------------------------------------------------------------

#[test]
fn a_panel_wider_than_512_columns_is_legal_while_a_terminal_is_not() {
    let wide = MAX_TERMINAL_COLS as u32 + 1;

    // The panel does not inherit the PTY per-axis cap: a 4K surface at a
    // small font is legitimately this wide, and the area bound is what
    // keeps the encoding inside the transport budget.
    let panel = panel_frame(1, wide);
    assert_eq!(panel.validate(), Ok(()));

    // The terminal keeps it, and reports the axis that failed.
    let terminal = terminal_frame(1, wide);
    assert!(matches!(
        terminal.validate(),
        Err(TerminalFrameError::Size { cols, max_cols, .. })
            if cols == wide && max_cols == MAX_TERMINAL_COLS as u32
    ));
}

#[test]
fn a_panel_still_answers_to_the_shared_area_bound() {
    // Removing the per-axis cap must not remove the area bound: that is
    // the check that actually bounds the encoded size.
    let huge = panel_frame(1, 1);
    let mut huge = huge;
    huge.size = CellSize::new(1024, 1024);
    huge.cells = vec![cell(' '); 1];
    assert!(matches!(huge.validate(), Err(PanelFrameError::Area { .. })));
}

#[test]
fn a_panel_cell_carrying_an_attachment_is_rejected() {
    // The attachment rejection is SHARED, not terminal-only, even though
    // the terminal-side message says "which terminals never use":
    // panels render no attachments either, so a shared rejection fails
    // closed for both.
    let mut frame = panel_frame(1, 2);
    frame.cells[1].attachment = Some(pmacs_protocol::cell::Attachment::ImageCell {
        image_id: 1,
        sub_x: 0,
        sub_y: 0,
    });
    assert!(matches!(
        frame.validate(),
        Err(PanelFrameError::Attachment { index: 1 })
    ));
}

#[test]
fn panel_glyph_topology_matches_the_terminal_rules() {
    // A wide lead with no continuation column on its row is rejected the
    // same way for both messages — the topology rule is shared.
    let mut frame = panel_frame(1, 1);
    frame.cells[0] = cell('\u{4e00}');
    assert!(matches!(
        frame.validate(),
        Err(PanelFrameError::Glyph { .. })
    ));

    let mut terminal = terminal_frame(1, 1);
    terminal.cells[0] = cell('\u{4e00}');
    assert!(matches!(
        terminal.validate(),
        Err(TerminalFrameError::Glyph { .. })
    ));
}

#[test]
fn a_panel_cursor_outside_its_grid_is_rejected() {
    let mut frame = panel_frame(2, 2);
    frame.cursor = Some(CellCoord::new(2, 0));
    assert!(matches!(
        frame.validate(),
        Err(PanelFrameError::Cursor {
            row: 2,
            rows: 2,
            ..
        })
    ));
}

#[test]
fn a_zero_epoch_panel_frame_is_rejected_on_the_wire() {
    // Epoch 0 is reserved for "never declared" (Q#BP2S1), so a frame
    // carrying it could otherwise match a receiver that has declared
    // nothing yet.
    let mut frame = panel_frame(1, 1);
    frame.panel_epoch = 0;
    assert!(matches!(
        frame.validate(),
        Err(PanelFrameError::ZeroEpoch { field: "panel" })
    ));

    let mut frame = panel_frame(1, 1);
    frame.geometry_epoch = 0;
    assert!(matches!(
        frame.validate(),
        Err(PanelFrameError::ZeroEpoch { field: "geometry" })
    ));
}

#[test]
fn terminal_frames_are_unchanged_by_the_factoring() {
    // The shared validator must not have altered terminal acceptance:
    // a valid frame still validates, and each terminal-only rule still
    // reports its own variant.
    assert_eq!(terminal_frame(3, 4).validate(), Ok(()));

    let mut bad_bottom = terminal_frame(1, 1);
    bad_bottom.at_bottom = false;
    bad_bottom.scroll_offset = 0;
    assert!(matches!(
        bad_bottom.validate(),
        Err(TerminalFrameError::BottomState { .. })
    ));

    let mut bad_meta = terminal_frame(1, 1);
    bad_meta.title = Some("\u{7}".into());
    assert!(matches!(
        bad_meta.validate(),
        Err(TerminalFrameError::Metadata { field: "title", .. })
    ));

    let mut bad_count = terminal_frame(2, 2);
    bad_count.cells.pop();
    assert!(matches!(
        bad_count.validate(),
        Err(TerminalFrameError::CellCount {
            expected: 4,
            actual: 3
        })
    ));
}
