// tests/undo_reach_438_probes.rs --- #438's reach: whether a user's
// undo meets loro's out-of-order undo of a span it splits into parts.

//! Probes through a real daemon and two synthetic replicas, each its
//! own loro peer (the frontend's id; the daemon writes as peer 1), as
//! `undo_across_peers_acceptance` drives them.
//!
//! loro 1.12.0's `undo_internal` splits a span at each op that depends
//! on another peer's and, with more than one part, can restore deleted
//! text out of order (#438, loro-dev/loro#1245). The arbiter reaches it
//! only through `undo_span_by_loro`, for a recorded span that
//! `deps_of_linear_span` finds non-linear (`src/crdt.rs`). A span is
//! recorded per forward edit (`spans_since` around one edit or one
//! import, `src/buffer.rs`), and both shipped frontends send one
//! `CrdtOp` per keystroke, exported right after that one edit. So:
//!
//! - two Backspaces with another peer's insert between them, as the
//!   frontends send them, are two updates, two one-op spans and two
//!   undo steps, and come back exactly;
//! - an undo group can hold several of a frontend's edits with another
//!   peer's op landing between them (a typed run), and each edit's span
//!   is undone on its own, so the group comes back exactly;
//! - the same two Backspaces in ONE update, which no shipped frontend
//!   sends and the daemon accepts, are one span with the other peer's
//!   op inside it, and its undo restores `Zabcdfe`.
//!
//! Helpers are the acceptance suite's, copied so the probe stands
//! alone, plus an optimistic delete and a raw update send.

#![cfg(feature = "crdt")]

use std::time::Duration;

use pmacs::crdt::CrdtState;
use pmacs::protocol::{FrontendEvent, FrontendId, Key, KeyEvent, Modifiers};
use pmacs::rope::CrdtOp as RopeCrdtOp;
use pmacs::transport::write_message;

mod common;
use common::daemon::{TestDaemon, attach_multi};

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

/// Send `bytes` as one `CrdtOp` on the replica's own peer.
fn send_op(replica: &mut Replica, bytes: Vec<u8>) {
    write_message(
        &mut replica.stream,
        &FrontendEvent::CrdtOp {
            frontend_id: replica.fid,
            buffer_id: replica.buffer_id,
            op: RopeCrdtOp {
                peer_id: replica.fid.0,
                bytes,
            },
        },
    )
    .expect("write CrdtOp");
}

/// The optimistic route: one op per character, each exported right
/// after it and sent alone, as both frontends do.
fn type_optimistic(replica: &mut Replica, at: usize, text: &str) {
    for (i, ch) in text.chars().enumerate() {
        let v = replica.state.version();
        replica
            .state
            .insert(at + i, &ch.to_string())
            .expect("mirror insert");
        let bytes = replica
            .state
            .export_updates_since(&v)
            .expect("export updates after local mutation");
        send_op(replica, bytes);
    }
}

/// An optimistic Backspace before byte `at + 1`: deleted in the mirror,
/// exported and sent alone (`BufferMirror::apply_local_delete`,
/// `pmacs-gpu`'s `optimistic_crdt_delete`).
fn backspace_optimistic(replica: &mut Replica, at: usize) {
    let v = replica.state.version();
    replica.state.delete(at, 1).expect("mirror delete");
    let bytes = replica
        .state
        .export_updates_since(&v)
        .expect("export updates after local mutation");
    send_op(replica, bytes);
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

/// `C-x u`, the prefix chord that round-trips on every frontend.
fn send_undo(replica: &mut Replica) {
    send_key(replica, Key::Char('x'), Modifiers::CTRL);
    send_key(replica, Key::Char('u'), Modifiers::NONE);
}

/// Pump broadcast messages into the replica until `accept` holds of
/// its text, or the deadline passes; returns the text.
fn pump_until(replica: &mut Replica, what: &str, accept: impl Fn(&str) -> bool) -> String {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let now = replica.state.materialize_string();
        if accept(&now) {
            return now;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "pump timeout waiting for {what}: text={now:?}"
        );
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        replica
            .stream
            .set_read_timeout(Some(remaining.min(Duration::from_millis(100))))
            .ok();
        if let Ok(pmacs::protocol::InstanceMessage::CrdtOp { buffer_id: b, op }) =
            pmacs::transport::read_message::<pmacs::protocol::InstanceMessage>(&mut replica.stream)
            && b == replica.buffer_id
        {
            let _ = replica.state.import_updates(&op.bytes);
        }
    }
}

fn pump_until_text(replica: &mut Replica, expected: &str, what: &str) {
    pump_until(replica, &format!("{what} (expected {expected:?})"), |t| {
        t == expected
    });
}

/// Pump for `window` expecting the text to stay `expected`.
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

fn both_see(user: &mut Replica, other: &mut Replica, expected: &str, what: &str) {
    pump_until_text(user, expected, &format!("user {what}"));
    pump_until_text(other, expected, &format!("other {what}"));
}

/// Two frontends of one daemon, and the daemon itself, are three loro
/// peers.
fn assert_distinct_peers(user: &Replica, other: &Replica) {
    let daemon_peer = pmacs::crdt::peer_id_from_frontend(FrontendId::LOCAL);
    assert_ne!(user.state.peer_id(), other.state.peer_id());
    assert_ne!(user.state.peer_id(), daemon_peer);
    assert_ne!(other.state.peer_id(), daemon_peer);
}

/// #438's case as a user makes it: on `abcdef`, the user's optimistic
/// Backspace, the other frontend's `Z` at 0, the user's second
/// Backspace. Each Backspace is its own update, its own one-op span and
/// its own undo step: the first undo restores `e` alone, the second
/// `f`, each exactly and after the other's `Z`, and the user has
/// nothing more to undo (the other's typing is not theirs).
#[test]
fn two_backspaces_around_anothers_insert_undo_apart_and_exactly() {
    let daemon = TestDaemon::spawn();
    let mut user = attach_replica(&daemon);
    let mut other = attach_replica(&daemon);
    assert_distinct_peers(&user, &other);

    type_optimistic(&mut other, 0, "abcdef");
    both_see(&mut user, &mut other, "abcdef", "abcdef");
    backspace_optimistic(&mut user, 5);
    both_see(&mut user, &mut other, "abcde", "the first Backspace");
    type_optimistic(&mut other, 0, "Z");
    both_see(&mut user, &mut other, "Zabcde", "the other's Z");
    backspace_optimistic(&mut user, 5);
    both_see(&mut user, &mut other, "Zabcd", "the second Backspace");

    send_undo(&mut user);
    both_see(
        &mut user,
        &mut other,
        "Zabcde",
        "the second Backspace undone alone",
    );
    send_undo(&mut user);
    both_see(
        &mut user,
        &mut other,
        "Zabcdef",
        "the first Backspace undone, in order",
    );
    send_undo(&mut user);
    assert_text_stays(&mut user, "Zabcdef", Duration::from_millis(500));
}

/// The same on the key route, where the user's Backspaces are commands
/// dispatched at the daemon and written on the daemon's peer: still one
/// span per command and one undo step each, restored exactly.
#[test]
fn two_backspace_commands_around_anothers_insert_undo_apart_and_exactly() {
    let daemon = TestDaemon::spawn();
    let mut user = attach_replica(&daemon);
    let mut other = attach_replica(&daemon);
    assert_distinct_peers(&user, &other);

    type_optimistic(&mut other, 0, "abcdef");
    both_see(&mut user, &mut other, "abcdef", "abcdef");
    send_key(&mut user, Key::End, Modifiers::NONE);
    send_key(&mut user, Key::Backspace, Modifiers::NONE);
    both_see(&mut user, &mut other, "abcde", "the first Backspace");
    type_optimistic(&mut other, 0, "Z");
    both_see(&mut user, &mut other, "Zabcde", "the other's Z");
    send_key(&mut user, Key::Backspace, Modifiers::NONE);
    both_see(&mut user, &mut other, "Zabcd", "the second Backspace");

    send_undo(&mut user);
    both_see(
        &mut user,
        &mut other,
        "Zabcde",
        "the second Backspace undone alone",
    );
    send_undo(&mut user);
    both_see(
        &mut user,
        &mut other,
        "Zabcdef",
        "the first Backspace undone, in order",
    );
}

/// One undo unit holding several of the user's edits with the other's
/// op landing between them: the user types `ab` and, after the other's
/// `Z` at 0, `cd`, one typed run (four spans, one group, under the
/// default limit of twenty). One undo takes all four around the `Z`;
/// the next finds nothing of the user's.
#[test]
fn a_typed_run_with_anothers_insert_inside_it_undoes_as_one_step_exactly() {
    let daemon = TestDaemon::spawn();
    let mut user = attach_replica(&daemon);
    let mut other = attach_replica(&daemon);
    assert_distinct_peers(&user, &other);

    type_optimistic(&mut user, 0, "ab");
    both_see(&mut user, &mut other, "ab", "ab");
    type_optimistic(&mut other, 0, "Z");
    both_see(&mut user, &mut other, "Zab", "the other's Z inside the run");
    type_optimistic(&mut user, 3, "cd");
    both_see(&mut user, &mut other, "Zabcd", "cd");

    send_undo(&mut user);
    both_see(&mut user, &mut other, "Z", "the whole run undone around Z");
    send_undo(&mut user);
    assert_text_stays(&mut user, "Z", Duration::from_millis(500));
}

/// The route when a span is formed: the user's two Backspaces sent as
/// ONE update, the second made after the user imported the other's `Z`
/// (a frontend batching its keystrokes; neither shipped frontend does).
/// The daemon accepts it (every new op is the user's own peer's) and
/// records one span whose second op depends on the other's; undo takes
/// it through loro's `undo_internal`, which restores the two characters
/// in part order. Pinned at loro 1.12.0's `Zabcdfe`; `Zabcdef` here
/// means loro's fix (#1245) or a change in how the daemon records or
/// undoes such an update.
#[test]
fn two_backspaces_in_one_update_around_anothers_insert_undo_out_of_order() {
    let daemon = TestDaemon::spawn();
    let mut user = attach_replica(&daemon);
    let mut other = attach_replica(&daemon);
    assert_distinct_peers(&user, &other);

    type_optimistic(&mut other, 0, "abcdef");
    both_see(&mut user, &mut other, "abcdef", "abcdef");

    let before = user.state.version();
    user.state.delete(5, 1).expect("the first Backspace, held");
    type_optimistic(&mut other, 0, "Z");
    pump_until_text(&mut other, "Zabcdef", "the other's Z");
    pump_until_text(
        &mut user,
        "Zabcde",
        "the user imports Z over its held Backspace",
    );
    user.state.delete(5, 1).expect("the second Backspace");
    let bytes = user
        .state
        .export_updates_since(&before)
        .expect("export both Backspaces");
    send_op(&mut user, bytes);
    pump_until_text(&mut other, "Zabcd", "the other sees both Backspaces");

    send_undo(&mut user);
    let restored = pump_until(&mut other, "the user's undo of the batch", |t| t.len() == 7);
    eprintln!("undo of two Backspaces sent as one update: {restored:?}");
    assert_eq!(
        restored, "Zabcdfe",
        "loro's multi-part undo restores the Backspaces in part order"
    );
    pump_until_text(&mut user, "Zabcdfe", "the user sees the same");
}
