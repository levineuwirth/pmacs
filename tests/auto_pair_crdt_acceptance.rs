// auto_pair_crdt_acceptance.rs --- auto-pairing over the wire.

//! Auto-pairing two-replica acceptance (docs/auto-pairing-framing.md):
//! a synthetic source replica plus a synthetic observer replica against
//! a real daemon subprocess.
//!
//! Dispatch route (built-in pair chars, Q#AP1): the source sends
//! round-tripped `Key` events; the daemon pairs/skips and broadcasts
//! `DaemonKey` ops to both replicas. Undo grain (Q#AP5) is pinned for
//! both routing models — the TUI's single-key optimistic undo is the
//! source replica's own peer-bound undo, `C-x u` is a round-tripped
//! daemon undo — as assertions of the named cross-peer substrate
//! limit, NOT frontend-equivalence claims.
//!
//! Optimistic route (custom pair char via user config, Q#AP1 cost
//! paragraph): the opener arrives as a `FrontendEvent::CrdtOp`; the
//! daemon's hook-queued closer is broadcast BEFORE the opener's
//! rebroadcast (the ordering quirk named in the framing), and the
//! observer must still converge. The source mirror's undo removes the
//! opener and leaves the closer — the pinned degraded undo.

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

/// One attached synthetic replica: stream + mirror + identity.
struct Replica {
    stream: std::os::unix::net::UnixStream,
    state: CrdtState,
    fid: FrontendId,
    buffer_id: pmacs::buffer::BufferId,
}

fn attach_replica(daemon: &TestDaemon) -> Replica {
    let (hello, mut stream) = attach_multi(daemon);
    let fid = hello.assigned_frontend_id;
    let (buffer_id, snap) = read_initial_snapshot(&mut stream);
    let state = CrdtState::new(fid.0).expect("CrdtState::new");
    state.import_snapshot(&snap).expect("import_snapshot");
    Replica {
        stream,
        state,
        fid,
        buffer_id,
    }
}

/// Mutate the local replica, export the delta, and ship it as an
/// optimistic `FrontendEvent::CrdtOp` (the `m10_11` idiom).
fn send_optimistic_op<F>(replica: &mut Replica, mutate: F)
where
    F: FnOnce(&CrdtState),
{
    let v = replica.state.version();
    mutate(&replica.state);
    let op_bytes = replica
        .state
        .export_updates_since(&v)
        .expect("export updates after local mutation");
    write_message(
        &mut replica.stream,
        &FrontendEvent::CrdtOp {
            frontend_id: replica.fid,
            buffer_id: replica.buffer_id,
            op: RopeCrdtOp {
                peer_id: replica.fid.0,
                bytes: op_bytes,
            },
        },
    )
    .expect("write CrdtOp");
}

fn send_key(replica: &mut Replica, key: Key, mods: Modifiers) {
    write_message(
        &mut replica.stream,
        &FrontendEvent::Key(KeyEvent {
            frontend_id: replica.fid,
            key,
            mods,
            timestamp_ns: 0,
        }),
    )
    .expect("send Key");
}

/// `C-x u` — the always-dispatched daemon undo.
fn send_daemon_undo(replica: &mut Replica) {
    send_key(replica, Key::Char('x'), Modifiers::CTRL);
    send_key(replica, Key::Char('u'), Modifiers::NONE);
}

/// `C-x r` — daemon redo.
fn send_daemon_redo(replica: &mut Replica) {
    send_key(replica, Key::Char('x'), Modifiers::CTRL);
    send_key(replica, Key::Char('r'), Modifiers::NONE);
}

/// What a pump observed so far: materialized text, the daemon's last
/// `CursorByte` for the shared buffer, ops imported this call.
struct Observed {
    text: String,
    cursor: Option<u64>,
    imported: usize,
}

/// Pump broadcast messages into the replica until `pred` holds or the
/// deadline passes. Imports every `CrdtOp` for the shared buffer and
/// tracks the latest `CursorByte`.
fn pump_until<P: Fn(&Observed) -> bool>(
    replica: &mut Replica,
    timeout: Duration,
    what: &str,
    pred: P,
) -> Observed {
    let deadline = std::time::Instant::now() + timeout;
    let mut obs = Observed {
        text: replica.state.materialize_string(),
        cursor: None,
        imported: 0,
    };
    loop {
        if pred(&obs) {
            return obs;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "pump timeout waiting for {what}; text={:?} cursor={:?} imported={}",
            obs.text,
            obs.cursor,
            obs.imported
        );
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        replica
            .stream
            .set_read_timeout(Some(remaining.min(Duration::from_millis(100))))
            .ok();
        match pmacs::transport::read_message::<pmacs::protocol::InstanceMessage>(
            &mut replica.stream,
        ) {
            Ok(pmacs::protocol::InstanceMessage::CrdtOp { buffer_id: b, op })
                if b == replica.buffer_id =>
            {
                let _ = replica.state.import_updates(&op.bytes);
                obs.imported += 1;
                obs.text = replica.state.materialize_string();
            }
            Ok(pmacs::protocol::InstanceMessage::CursorByte {
                buffer_id: b,
                byte_pos,
            }) if b == replica.buffer_id => {
                obs.cursor = Some(byte_pos);
            }
            Ok(_) | Err(_) => {}
        }
    }
}

/// Pump for `window` expecting NO text change — the negative
/// assertion for "a further daemon undo cannot reach source-peer
/// history". Ops are still imported (there should be none that change
/// text); panics if the text leaves `expected`.
fn assert_text_stays(replica: &mut Replica, expected: &str, window: Duration) {
    let deadline = std::time::Instant::now() + window;
    while std::time::Instant::now() < deadline {
        replica
            .stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .ok();
        if let Ok(pmacs::protocol::InstanceMessage::CrdtOp { buffer_id: b, op }) =
            pmacs::transport::read_message::<pmacs::protocol::InstanceMessage>(&mut replica.stream)
            && b == replica.buffer_id
        {
            let _ = replica.state.import_updates(&op.bytes);
            assert_eq!(
                replica.state.materialize_string(),
                expected,
                "text must not change during the negative window"
            );
        }
    }
    assert_eq!(replica.state.materialize_string(), expected);
}

// ---------------------------------------------------------------------------
// Dispatch route (built-in chars)
// ---------------------------------------------------------------------------

/// Round-tripped `(` pairs daemon-side and both replicas converge to
/// `()` with the daemon cursor between the pair; a round-tripped `)`
/// then skips (insert + swallow-delete, two more ops) and the daemon
/// cursor steps over the closer.
#[test]
fn dispatch_route_pair_and_skip_converge_on_both_replicas() {
    let daemon = TestDaemon::spawn();
    let mut source = attach_replica(&daemon);
    let mut observer = attach_replica(&daemon);

    send_key(&mut source, Key::Char('('), Modifiers::NONE);
    pump_until(&mut observer, Duration::from_secs(5), "observer ()", |o| {
        o.text == "()"
    });
    pump_until(
        &mut source,
        Duration::from_secs(5),
        "source () with cursor between",
        |o| o.text == "()" && o.cursor == Some(1),
    );

    // Skip: the typed `)` inserts then swallows the duplicate — two
    // ops that leave the text identical, so convergence is detected
    // by the op count plus the daemon cursor stepping to 2.
    send_key(&mut source, Key::Char(')'), Modifiers::NONE);
    pump_until(
        &mut source,
        Duration::from_secs(5),
        "source skip (two ops, cursor after the closer)",
        |o| o.text == "()" && o.imported >= 2 && o.cursor == Some(2),
    );
    pump_until(
        &mut observer,
        Duration::from_secs(5),
        "observer skip (two ops, text still ())",
        |o| o.text == "()" && o.imported >= 2,
    );
}

/// Q#AP5 undo grain over the wire: the pair is two adjacent
/// daemon-peer units. Two `C-x u` restore `(` then empty on BOTH
/// replicas; two `C-x r` restore `(` then `()` in order.
#[test]
fn dispatch_route_daemon_undo_redo_walk_the_pair_on_both_replicas() {
    let daemon = TestDaemon::spawn();
    let mut source = attach_replica(&daemon);
    let mut observer = attach_replica(&daemon);

    send_key(&mut source, Key::Char('('), Modifiers::NONE);
    pump_until(&mut source, Duration::from_secs(5), "source ()", |o| {
        o.text == "()"
    });
    pump_until(&mut observer, Duration::from_secs(5), "observer ()", |o| {
        o.text == "()"
    });

    send_daemon_undo(&mut source);
    pump_until(&mut source, Duration::from_secs(5), "source (", |o| {
        o.text == "("
    });
    pump_until(&mut observer, Duration::from_secs(5), "observer (", |o| {
        o.text == "("
    });

    send_daemon_undo(&mut source);
    pump_until(&mut source, Duration::from_secs(5), "source empty", |o| {
        o.text.is_empty()
    });
    pump_until(
        &mut observer,
        Duration::from_secs(5),
        "observer empty",
        |o| o.text.is_empty(),
    );

    send_daemon_redo(&mut source);
    pump_until(
        &mut source,
        Duration::from_secs(5),
        "source ( redone",
        |o| o.text == "(",
    );
    pump_until(
        &mut observer,
        Duration::from_secs(5),
        "observer ( redone",
        |o| o.text == "(",
    );

    send_daemon_redo(&mut source);
    pump_until(
        &mut source,
        Duration::from_secs(5),
        "source () redone",
        |o| o.text == "()",
    );
    pump_until(
        &mut observer,
        Duration::from_secs(5),
        "observer () redone",
        |o| o.text == "()",
    );
}

// ---------------------------------------------------------------------------
// Mixed source/daemon history (the named substrate limit, pinned)
// ---------------------------------------------------------------------------

/// TUI routing model: with optimistic `a` already in the source
/// mirror, the single-key optimistic undo (the mirror's own peer-bound
/// undo) removes `a` — NOT the daemon-peer closer — leaving `()`.
#[test]
fn mixed_history_source_mirror_undo_removes_the_optimistic_char_first() {
    let daemon = TestDaemon::spawn();
    let mut source = attach_replica(&daemon);
    let mut observer = attach_replica(&daemon);

    send_optimistic_op(&mut source, |r| {
        r.insert(0, "a").expect("insert a");
    });
    send_key(&mut source, Key::Char('('), Modifiers::NONE);
    pump_until(&mut source, Duration::from_secs(5), "source a()", |o| {
        o.text == "a()"
    });
    pump_until(&mut observer, Duration::from_secs(5), "observer a()", |o| {
        o.text == "a()"
    });

    // The TUI's single-key undo: mirror-local, peer-bound.
    send_optimistic_op(&mut source, |r| {
        r.undo().expect("mirror undo");
    });
    assert_eq!(
        source.state.materialize_string(),
        "()",
        "the mirror undo removed source-peer `a`, not the adjacent daemon closer"
    );
    pump_until(&mut observer, Duration::from_secs(5), "observer ()", |o| {
        o.text == "()"
    });
}

/// `C-x u` routing model (and the GPU model, which reaches the daemon
/// the same way): daemon undos peel the pair — closer, then opener —
/// and a FURTHER daemon undo cannot reach the source-peer `a`.
#[test]
fn mixed_history_daemon_undo_peels_the_pair_but_cannot_reach_source_history() {
    let daemon = TestDaemon::spawn();
    let mut source = attach_replica(&daemon);
    let mut observer = attach_replica(&daemon);

    send_optimistic_op(&mut source, |r| {
        r.insert(0, "a").expect("insert a");
    });
    send_key(&mut source, Key::Char('('), Modifiers::NONE);
    pump_until(&mut source, Duration::from_secs(5), "source a()", |o| {
        o.text == "a()"
    });

    send_daemon_undo(&mut source);
    pump_until(&mut source, Duration::from_secs(5), "source a(", |o| {
        o.text == "a("
    });
    pump_until(&mut observer, Duration::from_secs(5), "observer a(", |o| {
        o.text == "a("
    });

    send_daemon_undo(&mut source);
    pump_until(&mut source, Duration::from_secs(5), "source a", |o| {
        o.text == "a"
    });

    // The named limit: daemon undo is peer-bound too — source-peer
    // `a` is beyond its reach. (Cross-peer chronological arbitration
    // is deferred substrate work, not pair.lua's claim.)
    send_daemon_undo(&mut source);
    assert_text_stays(&mut source, "a", Duration::from_millis(800));
}

// ---------------------------------------------------------------------------
// Optimistic route (custom pair char from user config)
// ---------------------------------------------------------------------------

const CUSTOM_PAIR_CONFIG: &str = "table.insert(pmacs.pair.sets.default, \"<>\")\n";

/// A user-extended pair char still arrives optimistically: the opener
/// is a source-peer op, the daemon's hook-queued `>` closer is
/// broadcast BEFORE the opener's rebroadcast (the framing's ordering
/// quirk — the observer receives the causally dependent closer
/// first), and both replicas must still converge. The skip route
/// converges likewise.
#[test]
fn optimistic_route_custom_char_pairs_and_skips_despite_closer_first_broadcast() {
    let daemon = TestDaemon::spawn_with_config(CUSTOM_PAIR_CONFIG);
    let mut source = attach_replica(&daemon);
    let mut observer = attach_replica(&daemon);

    send_optimistic_op(&mut source, |r| {
        r.insert(0, "<").expect("insert <");
    });
    pump_until(&mut observer, Duration::from_secs(5), "observer <>", |o| {
        o.text == "<>"
    });
    pump_until(&mut source, Duration::from_secs(5), "source <>", |o| {
        o.text == "<>"
    });

    // Skip: the source optimistically types the closer before the
    // existing `>`; the daemon swallows the duplicate. Text returns
    // to `<>`; the extra daemon delete op must reach both replicas.
    send_optimistic_op(&mut source, |r| {
        r.insert(1, ">").expect("insert >");
    });
    pump_until(
        &mut source,
        Duration::from_secs(5),
        "source skip converged",
        |o| o.text == "<>" && o.imported >= 1,
    );
    pump_until(
        &mut observer,
        Duration::from_secs(5),
        "observer skip converged",
        |o| o.text == "<>" && o.imported >= 2,
    );
}

/// The pinned degraded undo for optimistic pair chars: the opener and
/// closer live on DIFFERENT peers, so the source mirror's undo removes
/// its own opener and leaves the daemon's closer behind.
#[test]
fn optimistic_route_mirror_undo_removes_the_opener_leaving_the_closer() {
    let daemon = TestDaemon::spawn_with_config(CUSTOM_PAIR_CONFIG);
    let mut source = attach_replica(&daemon);
    let mut observer = attach_replica(&daemon);

    send_optimistic_op(&mut source, |r| {
        r.insert(0, "<").expect("insert <");
    });
    pump_until(&mut source, Duration::from_secs(5), "source <>", |o| {
        o.text == "<>"
    });
    pump_until(&mut observer, Duration::from_secs(5), "observer <>", |o| {
        o.text == "<>"
    });

    send_optimistic_op(&mut source, |r| {
        r.undo().expect("mirror undo");
    });
    assert_eq!(
        source.state.materialize_string(),
        ">",
        "peer-bound mirror undo removes the opener; the daemon-peer closer stays"
    );
    pump_until(&mut observer, Duration::from_secs(5), "observer >", |o| {
        o.text == ">"
    });
}
