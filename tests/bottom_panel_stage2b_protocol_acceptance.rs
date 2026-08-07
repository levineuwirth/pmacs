//! Bottom-panel Stage 2B — the v21 protocol slice.
//!
//! Covers parent acceptance 37 (round-trip plus the two byte pins) and
//! the shared/terminal-only validator split of Q#BP15. The daemon
//! projection, the epoch state machine, and the GPU band are later
//! slices of this stage and are not exercised here.

mod common;

use std::time::Duration;

use pmacs_protocol::cell::{Cell, CellCoord, CellSize, Color, Glyph, Style, UnderlineStyle};
use pmacs_protocol::message::{
    AttachRequest, FrontendEvent, Hello, InstanceMessage, Modifiers, MouseButton, MouseKind,
};
use pmacs_protocol::panel::{
    MAX_PANEL_VISIBLE_CELLS, PANEL_MIN_VERSION, PanelFrame, PanelFrameError, PanelFramePayload,
};
use pmacs_protocol::terminal::{
    MAX_TERMINAL_COLS, TerminalFrame, TerminalFrameError, TerminalProcessState,
};
use pmacs_protocol::transport::{MAX_FRAME_BYTES, read_message, write_message};
use pmacs_protocol::wire_grid::{MAX_WIRE_GRID_GLYPH_BYTES, MAX_WIRE_GRID_GRAPHEME_BYTES};
use pmacs_protocol::{
    ADVERTISED_PROTOCOL_VERSION, BufferId, FrontendId, PROTOCOL_VERSION,
    SUPPORTED_PROTOCOL_VERSIONS,
};

use common::daemon::{TestDaemon, build_default_caps};

fn cell(ch: char) -> Cell {
    Cell {
        glyph: Glyph::Char(ch),
        style: Style::default(),
        attachment: None,
    }
}

/// The style whose postcard encoding is as long as a legal `Style` gets.
fn maximal_style() -> Style {
    Style {
        fg: Color::Rgb(0xff, 0xee, 0xdd),
        bg: Color::Rgb(0x11, 0x22, 0x33),
        bold: true,
        italic: true,
        underline: UnderlineStyle::Dashed,
        reverse: true,
        underline_color: Color::Rgb(0x44, 0x55, 0x66),
    }
}

fn maximal_cell(glyph: Glyph) -> Cell {
    Cell {
        glyph,
        style: maximal_style(),
        attachment: None,
    }
}

/// A single-column cluster of exactly `len` UTF-8 bytes.
fn cluster_of_len(len: usize) -> Vec<u8> {
    assert!((1..=MAX_WIRE_GRID_GRAPHEME_BYTES).contains(&len));
    let mut text = String::with_capacity(len);
    if len % 2 == 1 {
        text.push(' ');
    } else {
        text.push('\u{e9}');
    }
    while text.len() < len {
        text.push('\u{301}');
    }
    assert_eq!(text.len(), len);
    text.into_bytes()
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
    // The panel stage's own version, which does not move when a later
    // feature appends to the wire.
    //
    // This was `assert_eq!(PROTOCOL_VERSION, 21)` — the CURRENT wire
    // used as a proxy for the panel stage's version. The two were equal
    // only until the next feature landed (v22, `LineWrapFacts`), and the
    // proxy then failed in a test whose own name says what it means to
    // pin. `PANEL_MIN_VERSION` is that constant.
    assert_eq!(PANEL_MIN_VERSION, 21);
    assert!(SUPPORTED_PROTOCOL_VERSIONS.contains(&21));
    // This binary must be able to speak the stage it implements.
    const { assert!(PROTOCOL_VERSION >= PANEL_MIN_VERSION) };
    // The advertised version is a compatibility BASELINE, and Stage 2B-3
    // made that permanent rather than temporary: the server-first Hello
    // reaches an already-shipped frontend before that frontend can send
    // anything, so it must stay at a version none of them has to reject.
    // v21 is activated by the frontend's AttachRequest counter-offer
    // instead, which is why this stays 20 even though the panel wire is
    // now live in production.
    assert_eq!(ADVERTISED_PROTOCOL_VERSION, 20);
    assert!(SUPPORTED_PROTOCOL_VERSIONS.contains(&20));
}

#[test]
fn a_new_daemon_keeps_an_existing_v20_client_attachable() {
    let daemon = TestDaemon::spawn();
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set handshake timeout");

    // This is the rejection point in an already-shipped client: it reads the
    // daemon's unsolicited Hello before it is able to identify its own
    // supported range or send AttachRequest.
    let hello: Hello = read_message(&mut stream).expect("read daemon Hello");
    let v20_client_supported_versions = 6..=20;
    assert_eq!(hello.protocol_version, ADVERTISED_PROTOCOL_VERSION);
    assert!(
        v20_client_supported_versions.contains(&hello.protocol_version),
        "an existing v20 client would reject the server-first Hello"
    );

    write_message(
        &mut stream,
        &AttachRequest {
            protocol_version: hello.protocol_version,
            frontend_capabilities: build_default_caps(),
            initial_size: CellSize::new(24, 80),
        },
    )
    .expect("write v20 AttachRequest");

    assert!(
        matches!(
            read_message::<InstanceMessage>(&mut stream).expect("read initial grid"),
            InstanceMessage::CellDelta {
                full_grid: true,
                ..
            }
        ),
        "the daemon must establish the v20 session, not merely send an acceptable Hello"
    );
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
            buffer_id: BufferId::from_raw(21),
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

#[test]
fn panel_pointer_carries_buffer_id_distinctly_from_panel_epoch() {
    // The two fields close different holes and neither subsumes the
    // other: `buffer_id` catches an A->B buffer replacement, while
    // `panel_epoch` catches close/hide/reopen of the SAME buffer, which
    // a buffer id alone cannot see. So each must independently reach the
    // wire — a field silently dropped from the encoding would let one of
    // those two stale gestures through.
    let base = |buffer: u64, panel_epoch: u64| FrontendEvent::PanelPointer {
        frontend_id: FrontendId(4),
        geometry_epoch: 2,
        panel_epoch,
        buffer_id: BufferId::from_raw(buffer),
        coord: CellCoord::new(1, 1),
        kind: MouseKind::Down(MouseButton::Left),
        mods: Modifiers::default(),
    };
    let encode = |e: &FrontendEvent| postcard::to_allocvec(e).expect("encode");

    // Same panel epoch, different buffer: must differ on the wire.
    assert_ne!(encode(&base(1, 7)), encode(&base(2, 7)));
    // Same buffer, different panel epoch: must also differ.
    assert_ne!(encode(&base(1, 7)), encode(&base(1, 8)));

    // And both survive decode rather than being defaulted.
    let event = base(31, 7);
    let decoded: FrontendEvent = postcard::from_bytes(&encode(&event)).expect("decode");
    let FrontendEvent::PanelPointer {
        buffer_id,
        panel_epoch,
        ..
    } = decoded
    else {
        panic!("expected a PanelPointer, got {decoded:?}");
    };
    assert_eq!(buffer_id, BufferId::from_raw(31));
    assert_eq!(panel_epoch, 7);
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
                buffer_id: BufferId::from_raw(1),
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
    let wide = u32::from(MAX_TERMINAL_COLS) + 1;

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
            if cols == wide && max_cols == u32::from(MAX_TERMINAL_COLS)
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

// ---------------------------------------------------------------------------
// 39 — the transport-safety ratchet
// ---------------------------------------------------------------------------

/// The largest legal panel frame, plus the same frame one glyph byte over.
///
/// Deliberately shaped `1 x MAX_PANEL_VISIBLE_CELLS`: a panel carries no
/// per-axis cap, so this is a legal panel geometry a terminal frame
/// cannot express, and it is therefore the worst case the terminal's own
/// ratchet never measured.
fn panel_budget_boundary_frames() -> (PanelFrame, PanelFrame) {
    /// Shortest cluster length postcard encodes with a two-byte length
    /// prefix, which is what makes a cluster cell maximally expensive.
    const WIDE_PREFIX_LEN: usize = 128;
    let area = MAX_PANEL_VISIBLE_CELLS;

    // Every cell owes at least one glyph byte; the rest of the budget is
    // spent on as many two-byte-prefix clusters as it affords.
    let spare = MAX_WIRE_GRID_GLYPH_BYTES - area;
    let wide_cells = spare / (WIDE_PREFIX_LEN - 1);
    let remainder = spare % (WIDE_PREFIX_LEN - 1);
    assert!(wide_cells + usize::from(remainder > 0) <= area);

    let wide = cluster_of_len(WIDE_PREFIX_LEN).into_boxed_slice();
    let single = cluster_of_len(1).into_boxed_slice();
    let mut cells = Vec::with_capacity(area);
    for index in 0..area {
        let glyph = if index < wide_cells {
            Glyph::Cluster(wide.clone())
        } else if index == wide_cells && remainder > 0 {
            Glyph::Cluster(cluster_of_len(remainder + 1).into_boxed_slice())
        } else {
            Glyph::Cluster(single.clone())
        };
        cells.push(maximal_cell(glyph));
    }

    let cols = u32::try_from(area).expect("area fits u32");
    let exact = PanelFrame {
        buffer_id: BufferId::from_raw(u64::MAX),
        panel_epoch: u64::MAX,
        geometry_epoch: u64::MAX,
        size: CellSize::new(1, cols),
        cells,
        cursor: Some(CellCoord::new(0, cols - 1)),
        focused: true,
    };

    let mut over = exact.clone();
    // One more byte of glyph, nothing else changed.
    let last = over.cells.len() - 1;
    over.cells[last] = maximal_cell(Glyph::Cluster(cluster_of_len(2).into_boxed_slice()));

    (exact, over)
}

#[test]
fn maximum_legal_panel_frame_encodes_below_the_transport_cap() {
    let (exact, over) = panel_budget_boundary_frames();
    assert_eq!(exact.validate(), Ok(()));

    // The fixture must actually sit ON the boundary, or the ratchet
    // below measures something smaller than the worst case and would
    // stay green while a real maximum frame overran the transport.
    let mut glyph_bytes = 0usize;
    for cell in &exact.cells {
        glyph_bytes += match &cell.glyph {
            Glyph::Char(ch) => ch.len_utf8(),
            Glyph::Cluster(bytes) => bytes.len(),
            Glyph::Continuation => 0,
        };
    }
    assert_eq!(
        glyph_bytes, MAX_WIRE_GRID_GLYPH_BYTES,
        "the measured fixture must spend the whole aggregate budget"
    );
    let over_glyph_bytes = over
        .cells
        .iter()
        .map(|cell| match &cell.glyph {
            Glyph::Char(ch) => ch.len_utf8(),
            Glyph::Cluster(bytes) => bytes.len(),
            Glyph::Continuation => 0,
        })
        .sum::<usize>();
    assert_eq!(
        over_glyph_bytes,
        MAX_WIRE_GRID_GLYPH_BYTES + 1,
        "the rejecting twin must be exactly one byte over the aggregate budget"
    );

    // One byte over is rejected, which is what makes `exact` maximal.
    assert!(matches!(
        over.validate(),
        Err(PanelFrameError::GlyphBudget { .. })
    ));

    let msg = InstanceMessage::PanelFrame(PanelFramePayload::Present(exact));
    let bytes = postcard::to_allocvec(&msg).expect("encode");
    assert!(
        bytes.len() < MAX_FRAME_BYTES,
        "largest legal panel frame encodes to {} bytes, at or above the \
         {MAX_FRAME_BYTES}-byte transport cap; the aggregate glyph bound no \
         longer keeps panel traffic inside the existing transport limit",
        bytes.len()
    );
}
