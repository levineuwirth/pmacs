// m11_5_semantic_acceptance.rs --- M11.5 acceptance: the semantic frontend↔instance glue.

//! T M11.5 acceptance suite for the semantic-frontend arc.
//!
//! Two paths, both exercising the headless [`SemanticClient`] — the
//! frontend↔instance glue the design note names as the bounded
//! testable surface (`docs/semantic-frontend-protocol.md`,
//! "Testability strategy"):
//!
//! - **Reconstruction-equivalence (instance-side, deterministic).**
//!   Drive a [`SemanticRenderState`] through a scripted sequence of
//!   viewport declarations and editor mutations, feed every emitted
//!   message into a `SemanticClient`, and assert the client's
//!   incrementally-reconstructed view is byte-for-byte identical to a
//!   *fresh full* projection of the same instant (the oracle). This
//!   is the golden discipline without a snapshot crate: the property
//!   asserted is "incremental ≡ from-scratch", which no incidental
//!   wire-shape churn can falsely pass.
//!
//! - **End-to-end daemon filter.** A real daemon, a semantic session
//!   (negotiating `semantic_render`, declaring a `Viewport`) and a
//!   grid session: prove the M11.2 per-session projection actually
//!   routes `StyleSpans`/`Decorations` to the semantic session and
//!   never to the grid one, and `CellDelta` vice versa.

#![cfg(feature = "crdt")]

use std::time::{Duration, Instant};

use pmacs::buffer::BufferId;
use pmacs::cell::CellSize;
use pmacs::editor::EditorState;
use pmacs::protocol::{
    AttachRequest, ByteRange, FrontendCapabilities, FrontendEvent, FrontendId, Hello,
    InstanceMessage,
};
use pmacs::semantic_client::SemanticClient;
use pmacs::semantic_render::SemanticRenderState;
use pmacs::transport::{read_message, write_message};

mod common;
use common::daemon::{TestDaemon, build_default_caps};

// ---------------------------------------------------------------------------
// Part A — reconstruction-equivalence (instance-side, no daemon)
// ---------------------------------------------------------------------------

const LOCAL: FrontendId = FrontendId::LOCAL;

fn active_buffer(state: &EditorState) -> BufferId {
    state.core.borrow().active_window().buffer_id
}

fn set_selection(state: &EditorState, anchor: u64, cursor: u64) {
    let mut core = state.core.borrow_mut();
    let win = core
        .active_window_mut_for(LOCAL)
        .expect("LOCAL always has a window");
    win.selection = Some(pmacs::window::Selection { anchor });
    win.cursor = cursor;
}

/// The authoritative reconstruction for this instant: a fresh
/// `SemanticRenderState` emits a `full` first frame carrying the
/// complete current scoped set; a fresh client consuming only that is
/// the oracle the incrementally-driven client must match.
fn oracle(state: &EditorState, buffer_id: BufferId, vp: ByteRange) -> SemanticClient {
    let mut o = SemanticRenderState::new(LOCAL);
    o.set_viewport(buffer_id, vp, 0);
    let mut oc = SemanticClient::new(LOCAL);
    for m in &o.render_frame(state) {
        oc.apply(m);
    }
    oc
}

fn assert_equiv(client: &SemanticClient, state: &EditorState, buffer_id: BufferId, vp: ByteRange) {
    let oc = oracle(state, buffer_id, vp);
    for b in vp.start..vp.end {
        assert_eq!(
            client.decoration_kinds_at(buffer_id, b),
            oc.decoration_kinds_at(buffer_id, b),
            "decoration mismatch at byte {b}"
        );
        assert_eq!(
            client.effective_style_at(buffer_id, b),
            oc.effective_style_at(buffer_id, b),
            "style mismatch at byte {b}"
        );
    }
}

fn decorations_full(msgs: &[InstanceMessage]) -> Option<bool> {
    msgs.iter().find_map(|m| match m {
        InstanceMessage::Decorations { full, .. } => Some(*full),
        _ => None,
    })
}

fn has_style_spans(msgs: &[InstanceMessage]) -> bool {
    msgs.iter()
        .any(|m| matches!(m, InstanceMessage::StyleSpans { .. }))
}

fn generation_of(msgs: &[InstanceMessage]) -> Option<u64> {
    msgs.iter().find_map(|m| match m {
        InstanceMessage::StyleSpans { generation, .. }
        | InstanceMessage::Decorations { generation, .. } => Some(*generation),
        _ => None,
    })
}

fn assert_disjoint_within(ranges: &[ByteRange], vp: ByteRange) {
    let mut sorted = ranges.to_vec();
    sorted.sort_by_key(|r| (r.start, r.end));
    let mut prev_end = vp.start;
    for r in &sorted {
        assert!(
            r.start >= vp.start && r.end <= vp.end,
            "tile {r:?} escapes the declared viewport {vp:?}"
        );
        assert!(
            r.start >= prev_end,
            "tiles overlap: {r:?} starts before previous end {prev_end}"
        );
        prev_end = r.end;
    }
}

#[test]
fn incremental_reconstruction_equals_fresh_full_projection() {
    let state = EditorState::new();
    let buffer_id = active_buffer(&state);
    let vp1 = ByteRange { start: 0, end: 64 };

    let mut sem = SemanticRenderState::new(LOCAL);
    sem.set_viewport(buffer_id, vp1, 0);
    let mut client = SemanticClient::new(LOCAL);
    let mut generations: Vec<u64> = Vec::new();

    // Frame 1 — first frame: a full resync for both families (empty
    // scratch, no selection → empty segments).
    let f1 = sem.render_frame(&state);
    assert_eq!(decorations_full(&f1), Some(true), "first frame full");
    assert!(has_style_spans(&f1), "first frame ships StyleSpans too");
    if let Some(g) = generation_of(&f1) {
        generations.push(g);
    }
    for m in &f1 {
        client.apply(m);
    }
    assert_equiv(&client, &state, buffer_id, vp1);

    // Unchanged → fully silent.
    assert!(
        sem.render_frame(&state).is_empty(),
        "an unchanged frame emits nothing"
    );

    // A selection appears → Decorations re-emits incrementally
    // (viewport region unchanged), styling stays suppressed.
    set_selection(&state, 2, 5);
    let f2 = sem.render_frame(&state);
    assert_eq!(decorations_full(&f2), Some(false), "incremental, not full");
    assert!(!has_style_spans(&f2), "styling unchanged → not re-sent");
    if let Some(g) = generation_of(&f2) {
        generations.push(g);
    }
    for m in &f2 {
        client.apply(m);
    }
    assert_equiv(&client, &state, buffer_id, vp1);

    // Selection jumps far away → two disjoint dirty intervals (old
    // cleared, new painted). The client must reconstruct both.
    set_selection(&state, 40, 42);
    let f3 = sem.render_frame(&state);
    for m in &f3 {
        client.apply(m);
    }
    if let Some(g) = generation_of(&f3) {
        generations.push(g);
    }
    assert_equiv(&client, &state, buffer_id, vp1);
    assert_disjoint_within(&client.decoration_tile_ranges(buffer_id), vp1);

    // Viewport region moves → a full resync. The selection at
    // [40,42) is outside the new window, so the reconstruction is
    // empty there — but only if the client correctly discarded the
    // old viewport's tiles on the `full` frame.
    let vp2 = ByteRange {
        start: 100,
        end: 200,
    };
    sem.set_viewport(buffer_id, vp2, 0);
    let f4 = sem.render_frame(&state);
    assert_eq!(
        decorations_full(&f4),
        Some(true),
        "viewport jump forces a full resync"
    );
    for m in &f4 {
        client.apply(m);
    }
    assert_equiv(&client, &state, buffer_id, vp2);

    // Generation is monotonic non-decreasing across the run.
    for w in generations.windows(2) {
        assert!(w[1] >= w[0], "generation went backwards: {generations:?}");
    }
}

// ---------------------------------------------------------------------------
// Part B — end-to-end daemon: per-session projection routing
// ---------------------------------------------------------------------------

fn semantic_caps() -> FrontendCapabilities {
    // semantic_render requires crdt_replica (negotiation dependency
    // rule); a semantic session is also a text replica.
    FrontendCapabilities {
        multi_frontend: true,
        crdt_replica: true,
        semantic_render: true,
        ..build_default_caps()
    }
}

/// Read messages until `deadline`, classifying what arrives. Returns
/// `(saw_cell_delta, saw_semantic, first_buffer_id)`.
fn drain_kinds(
    stream: &mut std::os::unix::net::UnixStream,
    deadline: Instant,
    mut on_snapshot: impl FnMut(BufferId),
) -> (bool, bool) {
    let mut saw_cell = false;
    let mut saw_semantic = false;
    while Instant::now() < deadline {
        match read_message::<InstanceMessage>(stream) {
            Ok(InstanceMessage::CellDelta { .. }) => saw_cell = true,
            Ok(InstanceMessage::StyleSpans { .. } | InstanceMessage::Decorations { .. }) => {
                saw_semantic = true;
            }
            Ok(InstanceMessage::BufferSnapshot { buffer_id, .. }) => on_snapshot(buffer_id),
            // Other variants are irrelevant here; `Err` is a
            // read-timeout slice — both just keep polling.
            Ok(_) | Err(_) => {}
        }
    }
    (saw_cell, saw_semantic)
}

#[test]
fn daemon_routes_semantic_family_to_semantic_session_only() {
    let daemon = TestDaemon::spawn();

    // --- Semantic session ---
    let mut sem = daemon.connect();
    sem.set_read_timeout(Some(Duration::from_millis(250)))
        .unwrap();
    let hello: Hello = read_message(&mut sem).expect("semantic read Hello");
    let sem_fid = hello.assigned_frontend_id;
    write_message(
        &mut sem,
        &AttachRequest {
            protocol_version: hello.protocol_version,
            frontend_capabilities: semantic_caps(),
            initial_size: CellSize::new(24, 80),
        },
    )
    .expect("semantic write AttachRequest");

    // Learn a buffer id from the bootstrap snapshot, then declare a
    // viewport — the daemon emits nothing semantic until it does
    // (M11.2), so this also exercises the Viewport intercept e2e.
    let mut buf: Option<BufferId> = None;
    let by = Instant::now() + Duration::from_secs(5);
    let _ = drain_kinds(&mut sem, Instant::now() + Duration::from_secs(2), |b| {
        buf.get_or_insert(b);
    });
    let buffer_id = buf.expect("semantic session received a BufferSnapshot");
    write_message(
        &mut sem,
        &FrontendEvent::Viewport {
            frontend_id: sem_fid,
            buffer_id,
            visible: ByteRange {
                start: 0,
                end: 4096,
            },
            generation: 0,
        },
    )
    .expect("semantic write Viewport");
    let (sem_saw_cell, sem_saw_semantic) = drain_kinds(&mut sem, by, |_| {});
    assert!(
        sem_saw_semantic,
        "semantic session must receive StyleSpans/Decorations after declaring a viewport"
    );
    assert!(
        !sem_saw_cell,
        "semantic session must NOT receive grid CellDelta (it lays out locally)"
    );

    // --- Grid session (same daemon) ---
    let mut grid = daemon.connect();
    grid.set_read_timeout(Some(Duration::from_millis(250)))
        .unwrap();
    let ghello: Hello = read_message(&mut grid).expect("grid read Hello");
    write_message(
        &mut grid,
        &AttachRequest {
            protocol_version: ghello.protocol_version,
            frontend_capabilities: build_default_caps(),
            initial_size: CellSize::new(24, 80),
        },
    )
    .expect("grid write AttachRequest");
    let (grid_saw_cell, grid_saw_semantic) =
        drain_kinds(&mut grid, Instant::now() + Duration::from_secs(3), |_| {});
    assert!(
        grid_saw_cell,
        "grid session must receive CellDelta (the M5 projection)"
    );
    assert!(
        !grid_saw_semantic,
        "grid session must NOT receive the semantic family"
    );
}
