// auto_indent_crdt_acceptance.rs --- RET's daemon CRDT round trip.

//! Auto-indent daemon-side wire acceptance (Q#AI6, the second of the
//! two named GPU seams in docs/auto-indent-framing.md): a synthetic
//! attached replica sends pending optimistic self-inserts followed by
//! a round-tripped Enter, and the daemon must dispatch
//! `edit.newline-and-indent` and broadcast the resulting multi-byte
//! CRDT op back to the source replica. This is the daemon side of the
//! wire path the GPU frontend takes now that plain Enter is no longer
//! optimistic-eligible; the in-crate classifier test in `pmacs-gpu`
//! covers the frontend side of the seam.

#![cfg(feature = "crdt")]

use std::time::Duration;

use pmacs::crdt::CrdtState;
use pmacs::protocol::{FrontendEvent, FrontendId, Key, KeyEvent, Modifiers};
use pmacs::rope::CrdtOp as RopeCrdtOp;
use pmacs::transport::write_message;

mod common;
use common::daemon::{TestDaemon, attach_multi};

/// Read the daemon's initial `BufferSnapshot` for a freshly-attached
/// replica stream (the daemon always emits it first).
fn read_initial_snapshot(
    stream: &mut std::os::unix::net::UnixStream,
) -> (pmacs::buffer::BufferId, Vec<u8>) {
    match pmacs::transport::read_message::<pmacs::protocol::InstanceMessage>(stream)
        .expect("read initial BufferSnapshot")
    {
        pmacs::protocol::InstanceMessage::BufferSnapshot {
            buffer_id,
            crdt_snapshot,
        } => (buffer_id, crdt_snapshot),
        other => panic!("expected initial BufferSnapshot, got {other:?}"),
    }
}

/// Mutate the local replica, export the delta, and ship it as an
/// optimistic `FrontendEvent::CrdtOp` (the `m10_11` idiom).
fn send_optimistic_op_from<F>(
    stream: &mut std::os::unix::net::UnixStream,
    replica: &CrdtState,
    frontend_id: FrontendId,
    buffer_id: pmacs::buffer::BufferId,
    mutate: F,
) where
    F: FnOnce(&CrdtState),
{
    let v = replica.version();
    mutate(replica);
    let op_bytes = replica
        .export_updates_since(&v)
        .expect("export updates after local mutation");
    write_message(
        stream,
        &FrontendEvent::CrdtOp {
            frontend_id,
            buffer_id,
            op: RopeCrdtOp {
                peer_id: frontend_id.0,
                bytes: op_bytes,
            },
        },
    )
    .expect("write CrdtOp");
}

/// Pump broadcast messages into the replica until it materializes
/// `expected` or the deadline passes.
fn pump_until(
    stream: &mut std::os::unix::net::UnixStream,
    replica: &CrdtState,
    buffer_id: pmacs::buffer::BufferId,
    expected: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if replica.materialize_string() == expected {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        stream
            .set_read_timeout(Some(remaining.min(Duration::from_millis(100))))
            .ok();
        match pmacs::transport::read_message::<pmacs::protocol::InstanceMessage>(stream) {
            Ok(pmacs::protocol::InstanceMessage::CrdtOp { buffer_id: b, op }) if b == buffer_id => {
                let _ = replica.import_updates(&op.bytes);
            }
            Ok(_) | Err(_) => {}
        }
    }
    Err(format!(
        "expected materialize {expected:?}, got {observed:?} after {timeout:?}",
        observed = replica.materialize_string()
    ))
}

/// Pending optimistic self-inserts (`"··x"`, one op per keystroke,
/// mirroring GPU typing), then Enter as a round-tripped Key. The
/// daemon's dispatch must run `edit.newline-and-indent` — carrying
/// the two-space indent — and the multi-byte op must come back to the
/// source replica. A plain-newline dispatch would converge to
/// `"  x\n"` instead and fail the assertion.
#[test]
fn round_tripped_enter_after_pending_optimistic_input_auto_indents() {
    let daemon = TestDaemon::spawn();
    let (hello, mut stream) = attach_multi(&daemon);
    let fid = hello.assigned_frontend_id;

    let (buffer_id, snap) = read_initial_snapshot(&mut stream);
    let replica = CrdtState::new(fid.0).expect("CrdtState::new");
    replica.import_snapshot(&snap).expect("import_snapshot");

    // Three pending optimistic self-inserts, ahead of the Enter.
    send_optimistic_op_from(&mut stream, &replica, fid, buffer_id, |r| {
        r.insert(0, " ").expect("insert space");
    });
    send_optimistic_op_from(&mut stream, &replica, fid, buffer_id, |r| {
        r.insert(1, " ").expect("insert space");
    });
    send_optimistic_op_from(&mut stream, &replica, fid, buffer_id, |r| {
        r.insert(2, "x").expect("insert x");
    });

    // Enter round-trips (never optimistic since Q#AI1): the daemon
    // applies the pending ops first — its cursor for this frontend
    // tracks the optimistic post-edit position — then dispatches the
    // keymap's RET binding.
    write_message(
        &mut stream,
        &FrontendEvent::Key(KeyEvent {
            frontend_id: fid,
            key: Key::Enter,
            mods: Modifiers::NONE,
            timestamp_ns: 0,
        }),
    )
    .expect("send Enter");

    pump_until(
        &mut stream,
        &replica,
        buffer_id,
        "  x\n  ",
        Duration::from_secs(5),
    )
    .expect("replica converges to the auto-indented text");
}
