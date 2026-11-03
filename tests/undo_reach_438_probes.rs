// tests/undo_reach_438_probes.rs --- #438's reach: whether a user's
// undo meets loro's out-of-order undo of a span it splits into parts,
// and #444's fix: an update carrying several of a frontend's changes is
// recorded one span per change; #448's: every import is recorded between
// the daemon's own versions.

//! Probes through a real daemon and two synthetic replicas, each its
//! own loro peer (the frontend's id; the daemon writes as peer 1), as
//! `undo_across_peers_acceptance` drives them.
//!
//! loro 1.12.0's `undo_internal` splits a span at each op that depends
//! on another peer's and, with more than one part, can restore deleted
//! text out of order (#438, loro-dev/loro#1245). The arbiter reaches it
//! only through `undo_span_by_loro`, for a recorded span that
//! `deps_of_linear_span` finds non-linear (`src/crdt.rs`). A span is
//! recorded per forward edit (`spans_since` around one edit or, until
//! E10o, one import, `src/buffer.rs`), and both shipped frontends send
//! one `CrdtOp` per keystroke, exported right after that one edit. So:
//!
//! - two Backspaces with another peer's insert between them, as the
//!   frontends send them, are two updates, two one-op spans and two
//!   undo steps, and come back exactly;
//! - an undo group can hold several of a frontend's edits with another
//!   peer's op landing between them (a typed run), and each edit's span
//!   is undone on its own, so the group comes back exactly;
//! - the same two Backspaces in ONE update, which no shipped frontend
//!   sends and the daemon accepts, were one span with the other peer's
//!   op inside it until #444's fix, and its undo restored `Zabcdfe`.
//!   Since E10o the daemon records such an update one span per change,
//!   each undone between its own versions at the daemon
//!   (`CrdtState::changes_since`), and the undo restores `Zabcdef`.
//! - a keystroke the user made before another frontend's op reached it,
//!   which the daemon already held, is one update of one change, and was
//!   recorded as its span and undone in its own causal past, without
//!   that op, so the undo landed beside its place (#448: `Zabcdfe` for a
//!   Backspace, `Zabcdeg` for a typed `g`). Since E10o.4 every import is
//!   recorded change by change between the daemon's versions.
//!
//! A row whose update is made without a foreign edit the daemon already
//! held forces that order. The foreign edit and the user's update go out
//! on two sockets, and the daemon's reader threads race them into its
//! one channel (`src/daemon.rs`), so a row that only waits on the
//! sender's own replica leaves the order to the scheduler: pinned to one
//! CPU, #448's rows passed at `fc91a95`'s record 86 times in 90 (E10o
//! review 1). A third replica, the observer, sends nothing and pumps
//! the daemon's broadcasts until it shows the foreign edit. The daemon
//! broadcasts an op only after applying it and applies one event at a
//! time, so an update written after that wait is applied after the
//! foreign edit, on any schedule.
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

/// The route when a span was formed: the user's two Backspaces sent as
/// ONE update, the second made after the user imported the other's `Z`
/// (a frontend batching its keystrokes; neither shipped frontend does).
/// The daemon accepts it (every new op is the user's own peer's). Until
/// #444's fix it recorded one span whose second op depends on the
/// other's, and undo took it through loro's `undo_internal`, which
/// restored the two characters in part order, `Zabcdfe`. Now it records
/// one span per change, and one undo restores both in order.
#[test]
fn two_backspaces_in_one_update_around_anothers_insert_undo_in_order() {
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
        restored, "Zabcdef",
        "one undo restores both Backspaces of the update, in order"
    );
    pump_until_text(&mut user, "Zabcdef", "the user sees the same");
}

// ---------------------------------------------------------------------
// E10o.2: an update carrying several of a frontend's changes (#444).
// ---------------------------------------------------------------------

/// `C-x r`, `buffer.redo`.
fn send_redo(replica: &mut Replica) {
    send_key(replica, Key::Char('x'), Modifiers::CTRL);
    send_key(replica, Key::Char('r'), Modifiers::NONE);
}

/// Send everything the replica holds since `from` as one update: a
/// frontend that batches its changes.
fn send_since(replica: &mut Replica, from: &loro::VersionVector) -> Vec<u8> {
    let bytes = replica
        .state
        .export_updates_since(from)
        .expect("export the batch");
    send_op(replica, bytes.clone());
    bytes
}

/// The other frontend's `abcdef`, seen by both.
fn start_with_abcdef(user: &mut Replica, other: &mut Replica) {
    assert_distinct_peers(user, other);
    type_optimistic(other, 0, "abcdef");
    both_see(user, other, "abcdef", "abcdef");
}

/// The daemon has applied a foreign edit: `observer`, a replica that
/// sends nothing, pumps the daemon's broadcasts until it shows
/// `at_daemon`. The daemon broadcasts an op after applying it
/// (`handle_remote_crdt_op`, then `broadcast_pending_crdt_ops`) and its
/// dispatcher takes one event at a time, so an update written after this
/// returns is applied after that edit (the module's note).
fn await_daemon_shows(observer: &mut Replica, at_daemon: &str, what: &str) {
    pump_until_text(
        observer,
        at_daemon,
        &format!("the observer: the daemon applied {what}"),
    );
}

/// The batched probe's update: on `abcdef`, the user's Backspace held,
/// the other's `Z` at 0 imported over it, a second Backspace, both sent
/// as one update. Leaves both replicas at `Zabcd`.
fn send_the_batched_backspaces(user: &mut Replica, other: &mut Replica) {
    let before = user.state.version();
    user.state.delete(5, 1).expect("the first Backspace, held");
    type_optimistic(other, 0, "Z");
    pump_until_text(other, "Zabcdef", "the other's Z");
    pump_until_text(user, "Zabcde", "the user imports Z over its held Backspace");
    user.state.delete(5, 1).expect("the second Backspace");
    send_since(user, &before);
    both_see(user, other, "Zabcd", "both Backspaces in one update");
}

/// Redo of the batched update: undo restores both Backspaces' text in
/// order, and redo deletes it again, on both replicas. Fails at the base
/// at the undo (`Zabcdfe`).
#[test]
fn redo_of_the_batched_backspaces_restores_zabcd() {
    let daemon = TestDaemon::spawn();
    let mut user = attach_replica(&daemon);
    let mut other = attach_replica(&daemon);
    start_with_abcdef(&mut user, &mut other);
    send_the_batched_backspaces(&mut user, &mut other);

    send_undo(&mut user);
    both_see(
        &mut user,
        &mut other,
        "Zabcdef",
        "the batch undone in order",
    );
    send_redo(&mut user);
    both_see(&mut user, &mut other, "Zabcd", "the batch redone");
    send_undo(&mut user);
    both_see(&mut user, &mut other, "Zabcdef", "the redo undone");
}

/// A batched typed run around the other frontend's insert: on `xy`, the
/// user types `ab` at the end and holds it, imports the other's `Z` at
/// 0, types `cd`, and sends all four characters as one update (two
/// changes, the other's `Z` between them). One undo takes the whole run
/// around the `Z`, and redo puts it back, on both replicas.
#[test]
fn a_batched_typed_run_around_anothers_insert_undoes_and_redoes() {
    let daemon = TestDaemon::spawn();
    let mut user = attach_replica(&daemon);
    let mut other = attach_replica(&daemon);
    assert_distinct_peers(&user, &other);
    type_optimistic(&mut other, 0, "xy");
    both_see(&mut user, &mut other, "xy", "xy");

    let before = user.state.version();
    user.state.insert(2, "ab").expect("ab, held");
    type_optimistic(&mut other, 0, "Z");
    pump_until_text(&mut other, "Zxy", "the other's Z");
    pump_until_text(&mut user, "Zxyab", "the user imports Z over its held ab");
    user.state.insert(5, "cd").expect("cd");
    send_since(&mut user, &before);
    both_see(&mut user, &mut other, "Zxyabcd", "the run in one update");

    send_undo(&mut user);
    both_see(
        &mut user,
        &mut other,
        "Zxy",
        "the whole run undone around Z",
    );
    send_redo(&mut user);
    both_see(&mut user, &mut other, "Zxyabcd", "the run redone");
}

/// The overlap: the user's first Backspace delivered alone; then, after
/// the other's `Z` and `Y`, an update carrying all three of the user's
/// Backspaces, the first again. The daemon integrates only the two new
/// ones, one span each, and records nothing for the repeated one: one
/// undo restores the two (`de`, in order) and leaves the first deleted,
/// and the next undo restores it, its own earlier step.
#[test]
fn an_update_repeating_a_delivered_prefix_records_only_what_is_new() {
    let daemon = TestDaemon::spawn();
    let mut user = attach_replica(&daemon);
    let mut other = attach_replica(&daemon);
    start_with_abcdef(&mut user, &mut other);

    let before = user.state.version();
    user.state.delete(5, 1).expect("the first Backspace");
    send_since(&mut user, &before);
    both_see(&mut user, &mut other, "abcde", "the first Backspace, alone");
    type_optimistic(&mut other, 0, "Z");
    both_see(&mut user, &mut other, "Zabcde", "the other's Z");
    user.state.delete(5, 1).expect("the second Backspace, held");
    type_optimistic(&mut other, 0, "Y");
    pump_until_text(&mut other, "YZabcde", "the other's Y");
    pump_until_text(
        &mut user,
        "YZabcd",
        "the user imports Y over its held Backspace",
    );
    user.state.delete(5, 1).expect("the third Backspace");
    send_since(&mut user, &before);
    both_see(&mut user, &mut other, "YZabc", "all three in one update");

    send_undo(&mut user);
    both_see(
        &mut user,
        &mut other,
        "YZabcde",
        "the two new Backspaces undone, the first kept",
    );
    send_undo(&mut user);
    both_see(
        &mut user,
        &mut other,
        "YZabcdef",
        "the first undone on its own",
    );
    send_undo(&mut user);
    assert_text_stays(&mut user, "YZabcdef", Duration::from_millis(500));
}

/// Replaying identical bytes adds no undo history: after the batched
/// run is undone, the same update sent again integrates nothing, and the
/// redo it would have cleared still redoes. A guard: at the base too an
/// import that integrates nothing changes no text and is not recorded
/// (`apply_remote_crdt_op` returns before its stash).
#[test]
fn replaying_an_update_adds_no_undo_history() {
    let daemon = TestDaemon::spawn();
    let mut user = attach_replica(&daemon);
    let mut other = attach_replica(&daemon);
    assert_distinct_peers(&user, &other);
    type_optimistic(&mut other, 0, "xy");
    both_see(&mut user, &mut other, "xy", "xy");

    let before = user.state.version();
    user.state.insert(2, "ab").expect("ab, held");
    type_optimistic(&mut other, 0, "Z");
    pump_until_text(&mut other, "Zxy", "the other's Z");
    pump_until_text(&mut user, "Zxyab", "the user imports Z over its held ab");
    user.state.insert(5, "cd").expect("cd");
    let batch = send_since(&mut user, &before);
    both_see(&mut user, &mut other, "Zxyabcd", "the run in one update");

    send_undo(&mut user);
    both_see(&mut user, &mut other, "Zxy", "the run undone");
    send_op(&mut user, batch);
    send_redo(&mut user);
    both_see(
        &mut user,
        &mut other,
        "Zxyabcd",
        "redo still redoes after the replay",
    );
    send_undo(&mut user);
    both_see(&mut user, &mut other, "Zxy", "the redo undone");
    send_undo(&mut user);
    assert_text_stays(&mut user, "Zxy", Duration::from_millis(500));
}

/// The delayed batch: the other's `Y` reaches the daemon after the user
/// last imported, so the user's update carries changes made without it;
/// then the other's `W` lands after the update. Undo and redo are exact,
/// `Y` and `W` kept. The observer shows `Y` at the daemon before the
/// update is sent, so the daemon holds it when the update arrives.
#[test]
fn a_batch_made_without_a_foreign_edit_the_daemon_held_undoes_exactly() {
    let daemon = TestDaemon::spawn();
    let mut user = attach_replica(&daemon);
    let mut other = attach_replica(&daemon);
    let mut observer = attach_replica(&daemon);
    start_with_abcdef(&mut user, &mut other);

    let before = user.state.version();
    user.state.delete(5, 1).expect("the first Backspace, held");
    type_optimistic(&mut other, 0, "Z");
    pump_until_text(&mut other, "Zabcdef", "the other's Z");
    pump_until_text(
        &mut user,
        "Zabcde",
        "the user imports Z over its held Backspace",
    );
    user.state.delete(5, 1).expect("the second Backspace, held");
    type_optimistic(&mut other, 0, "Y");
    await_daemon_shows(&mut observer, "YZabcdef", "the other's Y");
    assert_eq!(
        user.state.materialize_string(),
        "Zabcd",
        "the user has not received Y when it sends its batch"
    );
    send_since(&mut user, &before);
    pump_until_text(&mut other, "YZabcd", "the other sees both Backspaces");
    type_optimistic(&mut other, 0, "W");
    both_see(
        &mut user,
        &mut other,
        "WYZabcd",
        "the other's W after the update",
    );

    send_undo(&mut user);
    both_see(
        &mut user,
        &mut other,
        "WYZabcdef",
        "the batch undone, Y and W kept",
    );
    send_redo(&mut user);
    both_see(
        &mut user,
        &mut other,
        "WYZabcd",
        "the batch redone, Y and W kept",
    );
}

/// The user's earlier step after the delayed batch: the user's typed `x`
/// is its own group before the batch, and the other's `Z` and `Y` land
/// after it, `Y` unseen by the user when it sends the batch and shown at
/// the daemon by the observer before it does. Undoing the batch and then
/// the `x` is exact: the batch's undo leaves the `x`'s record holding `Z`
/// and `Y` once, so the second undo removes the `x` and nothing beside
/// it.
#[test]
fn the_users_earlier_step_undoes_exactly_after_a_delayed_batch() {
    let daemon = TestDaemon::spawn();
    let mut user = attach_replica(&daemon);
    let mut other = attach_replica(&daemon);
    let mut observer = attach_replica(&daemon);
    start_with_abcdef(&mut user, &mut other);
    type_optimistic(&mut user, 0, "x");
    both_see(&mut user, &mut other, "xabcdef", "the user's x");

    let before = user.state.version();
    user.state.delete(6, 1).expect("the first Backspace, held");
    type_optimistic(&mut other, 0, "Z");
    pump_until_text(&mut other, "Zxabcdef", "the other's Z");
    pump_until_text(
        &mut user,
        "Zxabcde",
        "the user imports Z over its held Backspace",
    );
    user.state.delete(6, 1).expect("the second Backspace, held");
    type_optimistic(&mut other, 0, "Y");
    await_daemon_shows(&mut observer, "YZxabcdef", "the other's Y");
    assert_eq!(
        user.state.materialize_string(),
        "Zxabcd",
        "the user has not received Y when it sends its batch"
    );
    send_since(&mut user, &before);
    both_see(
        &mut user,
        &mut other,
        "YZxabcd",
        "the batch, Y imported after",
    );

    send_undo(&mut user);
    both_see(&mut user, &mut other, "YZxabcdef", "the batch undone");
    send_undo(&mut user);
    both_see(
        &mut user,
        &mut other,
        "YZabcdef",
        "the x undone, nothing beside it",
    );
}

/// A one-change update with several ops undoes as one step: two
/// Backspaces with nothing between them merge into one change on the
/// user's replica (loro merges consecutive local changes), and one undo
/// restores both. A guard: until E10o.4 such an update was recorded as
/// its span, since E10o.4 as its one change between the daemon's
/// versions, and either undoes it exactly, nothing having landed inside
/// or beside it.
#[test]
fn a_one_change_update_of_two_backspaces_undoes_as_one_step() {
    let daemon = TestDaemon::spawn();
    let mut user = attach_replica(&daemon);
    let mut other = attach_replica(&daemon);
    start_with_abcdef(&mut user, &mut other);

    let before = user.state.version();
    user.state.delete(5, 1).expect("the first Backspace");
    user.state.delete(4, 1).expect("the second Backspace");
    send_since(&mut user, &before);
    both_see(
        &mut user,
        &mut other,
        "abcd",
        "both Backspaces in one update",
    );

    send_undo(&mut user);
    both_see(&mut user, &mut other, "abcdef", "both restored in one step");
    send_undo(&mut user);
    assert_text_stays(&mut user, "abcdef", Duration::from_millis(500));
}

// ---------------------------------------------------------------------
// E10o.4: every import between the daemon's versions (#448).
// ---------------------------------------------------------------------

/// The crossing, forced (E10o review 1's `cross`): the user's `edit`
/// held; `foreign` run by the other (or anything else that changes the
/// daemon's text); the observer pumped until it shows `at_daemon`, so the
/// daemon applied the foreign edit; the user, still showing `held`, sends
/// its edit alone, one change, as both shipped frontends send a keystroke
/// that crossed another frontend's op in flight. That the user shows
/// `held` holds by construction (its replica has read nothing since);
/// the observer's wait is what makes the update cross the foreign edit
/// at the daemon.
fn cross(
    user: &mut Replica,
    other: &mut Replica,
    observer: &mut Replica,
    edit: impl FnOnce(&CrdtState),
    foreign: impl FnOnce(&mut Replica),
    held: &str,
    at_daemon: &str,
) {
    let before = user.state.version();
    edit(&user.state);
    foreign(other);
    await_daemon_shows(observer, at_daemon, "the foreign edit");
    assert_eq!(
        user.state.materialize_string(),
        held,
        "the user has not received the foreign edit"
    );
    send_since(user, &before);
}

/// The user's held edit, made on `abcdef` before the other frontend's
/// `Z` at 0 reaches it, crossing it at the daemon ([`cross`]). Returns
/// once both replicas show `expected`.
fn send_one_edit_made_without_anothers_z(
    user: &mut Replica,
    other: &mut Replica,
    observer: &mut Replica,
    edit: impl FnOnce(&CrdtState),
    held: &str,
    expected: &str,
) {
    cross(
        user,
        other,
        observer,
        edit,
        |o| type_optimistic(o, 0, "Z"),
        held,
        "Zabcdef",
    );
    both_see(user, other, expected, "the user's edit after Z");
}

/// #448's first probe: a Backspace made without the other's `Z`, which
/// the daemon held. Undone between the daemon's versions around it, the
/// undo restores the `f` at its place, and redo deletes it again. Fails
/// at `fc91a95` with `Zabcdfe` on any schedule: the change was recorded
/// as its span with an empty base and undone in its own causal past,
/// which lacks `Z`.
#[test]
fn a_backspace_made_without_anothers_insert_the_daemon_held_undoes_at_its_place() {
    let daemon = TestDaemon::spawn();
    let mut user = attach_replica(&daemon);
    let mut other = attach_replica(&daemon);
    let mut observer = attach_replica(&daemon);
    start_with_abcdef(&mut user, &mut other);
    send_one_edit_made_without_anothers_z(
        &mut user,
        &mut other,
        &mut observer,
        |s| s.delete(5, 1).expect("one Backspace, held"),
        "abcde",
        "Zabcde",
    );

    send_undo(&mut user);
    both_see(
        &mut user,
        &mut other,
        "Zabcdef",
        "the Backspace undone at its place",
    );
    send_redo(&mut user);
    both_see(&mut user, &mut other, "Zabcde", "the Backspace redone");
}

/// #448's second probe: a typed `g` at the end, made without the other's
/// `Z`. The undo removes the `g` and nothing else, and redo puts it back.
/// Fails at `fc91a95` with `Zabcdeg` on any schedule: the undo deleted
/// the `f`. Without the crossing it passes there
/// (`r1_p5_the_448_typed_char_row_without_its_crossing_passes_either_way`).
#[test]
fn a_typed_char_made_without_anothers_insert_the_daemon_held_undoes_at_its_place() {
    let daemon = TestDaemon::spawn();
    let mut user = attach_replica(&daemon);
    let mut other = attach_replica(&daemon);
    let mut observer = attach_replica(&daemon);
    start_with_abcdef(&mut user, &mut other);
    send_one_edit_made_without_anothers_z(
        &mut user,
        &mut other,
        &mut observer,
        |s| s.insert(6, "g").expect("one typed g, held"),
        "abcdefg",
        "Zabcdefg",
    );

    send_undo(&mut user);
    both_see(&mut user, &mut other, "Zabcdef", "the g undone, the f kept");
    send_redo(&mut user);
    both_see(&mut user, &mut other, "Zabcdefg", "the g redone");
}

/// The shipped route when typing goes on: the user's `g` crosses the
/// other's `Z` in flight as in the probe above, and the user, having
/// received `Z`, types `h`, each key sent alone, one typed run. One undo
/// takes the run around the `Z`, and redo restores it. Fails at
/// `fc91a95` with `Zabcdeg`: the `h` is undone at its place, its change
/// made after `Z`, and the `g`'s undo, in coordinates without `Z`,
/// deletes the `f`. (With the crossing key last instead, its misplaced
/// delete takes the run's own `g` and the `g`'s then the `h`, so the
/// text comes out right at `fc91a95` too and shows nothing.) The `g`
/// crosses `Z` at the daemon on any schedule ([`cross`]).
#[test]
fn a_typed_run_opened_by_a_key_made_without_anothers_insert_undoes_exactly() {
    let daemon = TestDaemon::spawn();
    let mut user = attach_replica(&daemon);
    let mut other = attach_replica(&daemon);
    let mut observer = attach_replica(&daemon);
    start_with_abcdef(&mut user, &mut other);
    send_one_edit_made_without_anothers_z(
        &mut user,
        &mut other,
        &mut observer,
        |s| s.insert(6, "g").expect("one typed g, held"),
        "abcdefg",
        "Zabcdefg",
    );
    type_optimistic(&mut user, 8, "h");
    both_see(&mut user, &mut other, "Zabcdefgh", "the user's h after Z");

    send_undo(&mut user);
    both_see(&mut user, &mut other, "Zabcdef", "the run undone around Z");
    send_redo(&mut user);
    both_see(&mut user, &mut other, "Zabcdefgh", "the run redone");
    send_undo(&mut user);
    both_see(&mut user, &mut other, "Zabcdef", "the redo undone");
    send_undo(&mut user);
    assert_text_stays(&mut user, "Zabcdef", Duration::from_millis(500));
}

// ---------------------------------------------------------------------
// E10o review 1's probes: states the phase's rows did not choose, each
// crossing forced by the observer.
// ---------------------------------------------------------------------

/// The user, the other frontend and the observer all show `expected`.
fn all_see(r: &mut [&mut Replica; 3], expected: &str, what: &str) {
    for (who, replica) in ["user", "other", "observer"].iter().zip(r.iter_mut()) {
        pump_until_text(replica, expected, &format!("{who} {what}"));
    }
}

/// Three frontends of one daemon: the user, the other, and an observer
/// that only watches. The other types `abcdef` and then a dispatched
/// `Home`, which ends its typed run, so its next key is a group of its
/// own.
fn three_on_abcdef(daemon: &TestDaemon) -> (Replica, Replica, Replica) {
    let mut user = attach_replica(daemon);
    let mut other = attach_replica(daemon);
    let mut observer = attach_replica(daemon);
    let daemon_peer = pmacs::crdt::peer_id_from_frontend(FrontendId::LOCAL);
    for r in [&user, &other, &observer] {
        assert_ne!(r.state.peer_id(), daemon_peer);
    }
    assert_ne!(user.state.peer_id(), other.state.peer_id());
    type_optimistic(&mut other, 0, "abcdef");
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "abcdef",
        "abcdef",
    );
    send_key(&mut other, Key::Home, Modifiers::NONE);
    (user, other, observer)
}

/// R1-P5, a guard: the other interleaving of
/// `a_typed_char_made_without_anothers_insert_the_daemon_held_undoes_at_its_place`.
/// The user's held `g` reaches the daemon BEFORE the other's `Z` (the
/// observer shows `abcdefg` before `Z` is typed): the same keys, the
/// same texts on every replica, the same undo, and `Zabcdef` at the head
/// and at `fc91a95`'s record alike. With the daemon applying the user's
/// update first nothing crosses, so this is what that row read at the
/// base whenever its order inverted, before it forced the crossing.
#[test]
fn r1_p5_the_448_typed_char_row_without_its_crossing_passes_either_way() {
    let daemon = TestDaemon::spawn();
    let (mut user, mut other, mut observer) = three_on_abcdef(&daemon);
    let before = user.state.version();
    user.state.insert(6, "g").expect("g, held");
    send_since(&mut user, &before);
    await_daemon_shows(&mut observer, "abcdefg", "the user's g, first");
    type_optimistic(&mut other, 0, "Z");
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "Zabcdefg",
        "g, then Z",
    );
    send_undo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "Zabcdef",
        "the g undone",
    );
}
