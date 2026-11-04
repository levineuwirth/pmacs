// tests/e10o_review2_probes.rs --- E10o review 2's behavioral probes,
// committed as witnesses at its fix round.

//! States neither E10o's rows nor review 1's probes chose, and E10l's
//! whole-file summary driven across E10o's undo.
//!
//! Part A: more of the crossings a shipped frontend reaches. Part B: a
//! randomized scenario against an oracle that does not know how the record
//! works. Part C: the integration: a GPU peer, its minimap and a crossing
//! key's undo, with the whole-file read failing or not.
//!
//! Every row passes at the head the review read: it found no product
//! defect. Each row's doc names what it fails at: `fc91a95`'s record
//! (`07ff8e5`'s `src/buffer.rs`), `a70487f`'s (that commit's), or plant G,
//! which stops `compensate`'s `broadcast_on_edit` (the integration rows). A
//! row that passes at a record says so, and a row passing at both is a
//! guard.
//!
//! A `Replica` here reads its socket on a thread into a queue and imports
//! only when pumped, so holding an edit never holds up the daemon, whose
//! writes to every session are synchronous from its one dispatcher thread.

#![cfg(feature = "crdt")]

use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use pmacs::crdt::CrdtState;
use pmacs::protocol::{FrontendEvent, FrontendId, Key, KeyEvent, Modifiers};
use pmacs::rope::CrdtOp as RopeCrdtOp;
use pmacs::transport::write_message;

mod common;
use common::daemon::{TestDaemon, attach_multi};

fn read_initial_snapshot(stream: &mut UnixStream) -> (pmacs::buffer::BufferId, Vec<u8>) {
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

/// One attached synthetic replica. A thread reads its socket into a queue
/// from the attach on, and it imports only when a pump takes from the
/// queue: a replica "holding" its edit has not imported anything, and the
/// daemon, which writes to every session synchronously from its one
/// dispatcher thread (#390), is never held up by it. Without the thread a
/// replica that does not read for about a second stalls the daemon, and a
/// wait on any other replica then times out (the paused row in
/// `undo_reach_438_probes`).
struct Replica {
    stream: UnixStream,
    state: CrdtState,
    fid: FrontendId,
    buffer_id: pmacs::buffer::BufferId,
    rx: std::sync::mpsc::Receiver<pmacs::protocol::InstanceMessage>,
}

impl Drop for Replica {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

fn attach_replica(daemon: &TestDaemon) -> Replica {
    let (hello, mut stream) = attach_multi(daemon);
    let fid = hello.assigned_frontend_id;
    let (buffer_id, snap) = read_initial_snapshot(&mut stream);
    let state = CrdtState::new(fid.0).expect("CrdtState::new");
    state.import_snapshot(&snap).expect("import_snapshot");
    stream.set_read_timeout(None).expect("blocking reads");
    let rx = common::wire::read_into_queue(&stream);
    Replica {
        stream,
        state,
        fid,
        buffer_id,
        rx,
    }
}

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

fn send_since(replica: &mut Replica, from: &loro::VersionVector) {
    let bytes = replica
        .state
        .export_updates_since(from)
        .expect("export the update");
    send_op(replica, bytes);
}

fn type_optimistic(replica: &mut Replica, at: usize, text: &str) {
    let mut at = at;
    for ch in text.chars() {
        let v = replica.state.version();
        replica
            .state
            .insert(at, &ch.to_string())
            .expect("mirror insert");
        at += ch.len_utf8();
        send_since(replica, &v);
    }
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

fn send_undo(replica: &mut Replica) {
    send_key(replica, Key::Char('x'), Modifiers::CTRL);
    send_key(replica, Key::Char('u'), Modifiers::NONE);
}

fn send_redo(replica: &mut Replica) {
    send_key(replica, Key::Char('x'), Modifiers::CTRL);
    send_key(replica, Key::Char('r'), Modifiers::NONE);
}

/// Import one broadcast into the replica, if one comes within 100 ms.
fn pump_once(replica: &mut Replica) {
    if let Ok(pmacs::protocol::InstanceMessage::CrdtOp { buffer_id: b, op }) =
        replica.rx.recv_timeout(Duration::from_millis(100))
        && b == replica.buffer_id
    {
        let _ = replica.state.import_updates(&op.bytes);
    }
}

fn pump_until(replica: &mut Replica, what: &str, accept: impl Fn(&Replica) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if accept(replica) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "pump timeout waiting for {what}: text={:?}",
            replica.state.materialize_string()
        );
        pump_once(replica);
    }
}

fn pump_until_text(replica: &mut Replica, expected: &str, what: &str) {
    pump_until(replica, &format!("{what} (expected {expected:?})"), |r| {
        r.state.materialize_string() == expected
    });
}

fn all_see(r: &mut [&mut Replica; 3], expected: &str, what: &str) {
    for (who, replica) in ["user", "other", "observer"].iter().zip(r.iter_mut()) {
        pump_until_text(replica, expected, &format!("{who} {what}"));
    }
}

fn three_on(daemon: &TestDaemon, text: &str) -> (Replica, Replica, Replica) {
    let mut user = attach_replica(daemon);
    let mut other = attach_replica(daemon);
    let mut observer = attach_replica(daemon);
    type_optimistic(&mut other, 0, text);
    all_see(&mut [&mut user, &mut other, &mut observer], text, text);
    (user, other, observer)
}

// ---------------------------------------------------------------------
// Part A: crossings the rows and review 1's probes did not build.
// ---------------------------------------------------------------------

/// A key made before the user's own undo reached it: the shipped route
/// for `C-x u` and then a quick key. `g` is typed and seen, the undo sent,
/// and `h` made on a mirror still holding the `g` and sent right behind
/// it on the same socket, so the daemon undoes first and imports `h`,
/// made without the compensation it already holds. No other frontend is
/// involved: the crossing is with the daemon's own op, on any schedule.
/// Fails at `fc91a95`'s and `a70487f`'s records with `abcdefh`: the undo of
/// the `h` deletes nothing.
#[test]
fn r2_p1_a_key_made_before_its_senders_undo_arrived_undoes_at_its_place() {
    let daemon = TestDaemon::spawn();
    let (mut user, mut other, mut observer) = three_on(&daemon, "abcdef");
    type_optimistic(&mut user, 6, "g");
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "abcdefg",
        "the g",
    );
    send_undo(&mut user);
    let v = user.state.version();
    user.state
        .insert(7, "h")
        .expect("h, on a mirror with the g");
    send_since(&mut user, &v);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "abcdefh",
        "the g undone, the h kept",
    );
    send_undo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "abcdef",
        "the h undone and nothing beside it",
    );
    send_redo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "abcdefh",
        "the h redone",
    );
}

/// The same for a Backspace: `f` deleted and seen, the undo sent, and a
/// second Backspace, of `e`, made on the mirror still lacking the `f`. The
/// daemon restores the `f` and then deletes the `e`: `abcdf`. Its undo
/// restores `e` before the restored `f`. A guard: it passes at `fc91a95`'s
/// and `a70487f`'s records, since what the Backspace was made without, the
/// restored `f`, lies after the `e`, so its own coordinates still find the
/// place.
#[test]
fn r2_p1b_a_backspace_made_before_its_senders_undo_arrived_undoes_at_its_place() {
    let daemon = TestDaemon::spawn();
    let (mut user, mut other, mut observer) = three_on(&daemon, "abcdef");
    let v = user.state.version();
    user.state.delete(5, 1).expect("the f");
    send_since(&mut user, &v);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "abcde",
        "the f deleted",
    );
    send_undo(&mut user);
    let v = user.state.version();
    user.state
        .delete(4, 1)
        .expect("the e, on a mirror without f");
    send_since(&mut user, &v);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "abcdf",
        "the f restored, the e deleted",
    );
    send_undo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "abcdef",
        "the e restored at its place",
    );
}

/// The foreign op crossing the key is another frontend's undo, a
/// compensation on the daemon's peer: the other's `Z` is seen by all, the
/// user makes `g` with it, the other undoes `Z` and the observer shows it,
/// and only then the `g` is sent. Undo and redo of the `g` are exact, and
/// the other's redo of `Z` still lands at 0. Fails at `fc91a95`'s and
/// `a70487f`'s records with `g` after the undo.
#[test]
fn r2_p2_a_key_crossing_anothers_undo_undoes_at_its_place() {
    let daemon = TestDaemon::spawn();
    let (mut user, mut other, mut observer) = three_on(&daemon, "abcdef");
    type_optimistic(&mut other, 0, "Z");
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "Zabcdef",
        "the Z",
    );
    // The seed run is `abcdef` plus `Z`: one typed run of the other's.
    let v = user.state.version();
    user.state.insert(7, "g").expect("g, held");
    send_undo(&mut other);
    pump_until_text(&mut observer, "", "the observer: the other's run undone");
    assert_eq!(user.state.materialize_string(), "Zabcdefg");
    send_since(&mut user, &v);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "g",
        "the g alone",
    );
    send_undo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "",
        "the g undone",
    );
    send_redo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "g",
        "the g redone",
    );
    send_redo(&mut other);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "Zabcdefg",
        "the other's run redone before the g",
    );
}

/// Two keys at one place, each made without the other: a concurrent
/// insert at the end. Whichever order loro gives them, each frontend's
/// undo removes its own key and nothing else. A guard: it passes at
/// `fc91a95`'s and `a70487f`'s records.
#[test]
fn r2_p3_concurrent_keys_at_one_place_undo_each_alone() {
    let daemon = TestDaemon::spawn();
    let mut user = attach_replica(&daemon);
    let mut other = attach_replica(&daemon);
    let mut observer = attach_replica(&daemon);
    type_optimistic(&mut observer, 0, "abcdef");
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "abcdef",
        "abcdef",
    );
    let v = user.state.version();
    user.state.insert(6, "g").expect("g, held");
    type_optimistic(&mut other, 6, "h");
    pump_until(&mut observer, "the observer: h", |r| {
        r.state.materialize_string().contains('h')
    });
    send_since(&mut user, &v);
    pump_until(&mut observer, "the observer: g", |r| {
        r.state.materialize_string().contains('g')
    });
    let both = observer.state.materialize_string();
    all_see(&mut [&mut user, &mut other, &mut observer], &both, "both");
    send_undo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "abcdefh",
        "the g undone",
    );
    send_undo(&mut other);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "abcdef",
        "the h undone",
    );
}

/// A crossing key whose sender has left: its group becomes nobody's. The
/// `Z`'s run closed by a cursor key before the detach closes the `g`'s, so
/// the `g`'s group is the newer and the other's undo takes it, at its place
/// and with the `Z` kept; then its own `Z`. Fails at `fc91a95`'s and
/// `a70487f`'s records with `Zabcdeg`.
#[test]
fn r2_p4_a_departed_senders_crossing_key_undoes_at_its_place() {
    let daemon = TestDaemon::spawn();
    let (mut user, mut other, mut observer) = three_on(&daemon, "abcdef");
    // A cursor key closes the other's seed run, so `Z` is its own group.
    send_key(&mut other, Key::Home, Modifiers::NONE);
    let v = user.state.version();
    user.state.insert(6, "g").expect("g, held");
    type_optimistic(&mut other, 0, "Z");
    pump_until_text(&mut observer, "Zabcdef", "the observer: Z");
    send_since(&mut user, &v);
    pump_until_text(&mut observer, "Zabcdefg", "the observer: g");
    send_key(&mut other, Key::Home, Modifiers::NONE);
    std::thread::sleep(Duration::from_millis(200));
    drop(user);
    // The detach has no broadcast to wait on.
    std::thread::sleep(Duration::from_millis(500));
    send_undo(&mut other);
    pump_until_text(&mut other, "Zabcdef", "the departed g undone");
    pump_until_text(&mut observer, "Zabcdef", "the departed g undone");
    send_undo(&mut other);
    pump_until_text(&mut other, "abcdef", "then the other's own Z");
}

/// The same with the `Z`'s run still open at the detach: a group's place in
/// the recency pick is when it closed (`push_group`), so the other's own
/// `Z`, closed by the undo itself, is the newer: the `Z` goes first, and the
/// departed `g` after it, still at its place. This records E6c's pick by
/// close order as it stands, not that the order is ruled right: the older
/// `Z` goes before the newer `g`. What it witnesses is the `g`'s place,
/// failing at `fc91a95`'s and `a70487f`'s records with `abcdeg`.
#[test]
fn r2_p4b_a_departed_senders_crossing_key_undoes_after_the_others_open_run() {
    let daemon = TestDaemon::spawn();
    let (mut user, mut other, mut observer) = three_on(&daemon, "abcdef");
    send_key(&mut other, Key::Home, Modifiers::NONE);
    let v = user.state.version();
    user.state.insert(6, "g").expect("g, held");
    type_optimistic(&mut other, 0, "Z");
    pump_until_text(&mut observer, "Zabcdef", "the observer: Z");
    send_since(&mut user, &v);
    pump_until_text(&mut observer, "Zabcdefg", "the observer: g");
    drop(user);
    std::thread::sleep(Duration::from_millis(500));
    send_undo(&mut other);
    pump_until_text(&mut other, "abcdefg", "the other's own Z, the newer");
    send_undo(&mut other);
    pump_until_text(&mut other, "abcdef", "the departed g, at its place");
    pump_until_text(&mut observer, "abcdef", "the departed g, at its place");
}

/// Crossing keys over characters of every width: `é`, `日` and `𝄞`, a
/// Backspace of the four-byte character and a typed three-byte one, each
/// made without a three-byte insert at 0. Fails at `fc91a95`'s and
/// `a70487f`'s records with `語aé𝄞日f`.
#[test]
fn r2_p5_crossings_over_wide_characters_undo_at_their_place() {
    let daemon = TestDaemon::spawn();
    let (mut user, mut other, mut observer) = three_on(&daemon, "aé日𝄞f");
    let v = user.state.version();
    let at = "aé日".len();
    user.state.delete(at, "𝄞".len()).expect("the 𝄞, held");
    type_optimistic(&mut other, 0, "語");
    pump_until_text(&mut observer, "語aé日𝄞f", "the observer: 語");
    send_since(&mut user, &v);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "語aé日f",
        "the 𝄞 deleted",
    );
    send_undo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "語aé日𝄞f",
        "the 𝄞 restored at its place",
    );

    let v = user.state.version();
    let end = user.state.len_utf8();
    user.state.insert(end, "本").expect("the 本, held");
    type_optimistic(&mut other, 0, "字");
    pump_until_text(&mut observer, "字語aé日𝄞f", "the observer: 字");
    send_since(&mut user, &v);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "字語aé日𝄞f本",
        "the 本 typed",
    );
    send_undo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "字語aé日𝄞f",
        "the 本 undone and nothing beside it",
    );
}

/// One change of a delete and an insert, a replacement, made without the
/// other's `Z`. Fails at `fc91a95`'s and `a70487f`'s records with
/// `ZacdeYf`.
#[test]
fn r2_p6_a_crossing_replacement_undoes_at_its_place() {
    let daemon = TestDaemon::spawn();
    let (mut user, mut other, mut observer) = three_on(&daemon, "abcdef");
    let v = user.state.version();
    user.state.delete(2, 3).expect("cde, held");
    user.state.insert(2, "XY").expect("XY, held");
    type_optimistic(&mut other, 0, "Z");
    pump_until_text(&mut observer, "Zabcdef", "the observer: Z");
    send_since(&mut user, &v);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "ZabXYf",
        "the replacement",
    );
    send_undo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "Zabcdef",
        "the replacement undone",
    );
    send_redo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "ZabXYf",
        "the replacement redone",
    );
}

/// A typed run, then a crossing batch that closes it: the batch undoes
/// whole, then the run whole, each exact, the foreign `Z` and `Y` kept.
/// Fails at `a70487f`'s record with `YZadcbefgh`; it passes at `fc91a95`'s,
/// which already records a batch change by change.
#[test]
fn r2_p7_a_crossing_batch_after_a_typed_run_undoes_apart_from_it() {
    let daemon = TestDaemon::spawn();
    let (mut user, mut other, mut observer) = three_on(&daemon, "abcdef");
    type_optimistic(&mut user, 6, "gh");
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "abcdefgh",
        "the run",
    );
    let v = user.state.version();
    user.state.delete(3, 1).expect("the d, held");
    type_optimistic(&mut other, 0, "Z");
    pump_until_text(&mut observer, "Zabcdefgh", "the observer: Z");
    pump_until_text(&mut user, "Zabcefgh", "the user imports Z");
    user.state.delete(3, 1).expect("the c, held");
    type_optimistic(&mut other, 0, "Y");
    pump_until_text(&mut observer, "YZabcdefgh", "the observer: Y");
    send_since(&mut user, &v);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "YZabefgh",
        "the batch",
    );
    send_undo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "YZabcdefgh",
        "the batch undone",
    );
    send_undo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "YZabcdef",
        "the run undone",
    );
}

/// A batch whose net effect is one typed character joins the sender's open
/// run: the PR's Not done says "by reading, not run". `g` typed, then an
/// update of `h` and `x` with `x` deleted again and the other's `Z` imported
/// between, which nets one character; then `i`. One undo takes the run.
/// Fails at `a70487f`'s record with `Zabcdefx`; it passes at `fc91a95`'s,
/// which already records a batch change by change.
#[test]
fn r2_p9_a_batch_netting_one_character_joins_the_open_run() {
    let daemon = TestDaemon::spawn();
    let (mut user, mut other, mut observer) = three_on(&daemon, "abcdef");
    send_key(&mut user, Key::End, Modifiers::NONE);
    type_optimistic(&mut user, 6, "g");
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "abcdefg",
        "the g",
    );
    let v = user.state.version();
    user.state.insert(7, "h").expect("h");
    user.state.insert(8, "x").expect("x");
    type_optimistic(&mut other, 0, "Z");
    pump_until_text(&mut observer, "Zabcdefg", "the observer: Z");
    pump_until_text(&mut user, "Zabcdefghx", "the user imports Z");
    user.state.delete(9, 1).expect("x again");
    send_since(&mut user, &v);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "Zabcdefgh",
        "the batch, one character net",
    );
    type_optimistic(&mut user, 9, "i");
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "Zabcdefghi",
        "the i",
    );
    send_undo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "Zabcdef",
        "the whole run undone, the Z kept",
    );
    send_redo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "Zabcdefghi",
        "the whole run redone",
    );
}

/// One typed run holding both kinds of record: `g` sent as a key, dispatched
/// at the daemon and recorded as a span, then `h`, made without the other's
/// `Z` the daemon held and recorded as a change. One undo takes the run
/// around the `Z`, each record in its own coordinates, and redo restores it.
/// A guard: it passes at `fc91a95`'s and `a70487f`'s records, the crossing
/// key being the run's last (`undo_reach_438_probes`' typed-run row says why
/// that shows nothing there).
#[test]
fn r2_p10_a_run_of_a_key_and_a_crossing_import_undoes_as_one_step() {
    let daemon = TestDaemon::spawn();
    let (mut user, mut other, mut observer) = three_on(&daemon, "abcdef");
    send_key(&mut user, Key::End, Modifiers::NONE);
    send_key(&mut user, Key::Char('g'), Modifiers::NONE);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "abcdefg",
        "the key's g",
    );
    let v = user.state.version();
    user.state.insert(7, "h").expect("h, held");
    type_optimistic(&mut other, 0, "Z");
    pump_until_text(&mut observer, "Zabcdefg", "the observer: Z");
    send_since(&mut user, &v);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "Zabcdefgh",
        "the crossing h",
    );
    send_undo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "Zabcdef",
        "the run undone around the Z",
    );
    send_redo(&mut user);
    all_see(
        &mut [&mut user, &mut other, &mut observer],
        "Zabcdefgh",
        "the run redone",
    );
}

// ---------------------------------------------------------------------
// Part B: random crossings against an oracle that does not know the record.
// ---------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() >> 33).expect("31 bits") % n.max(1)
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

/// Everything the other frontend types is katakana, so a text's foreign
/// part and its own are told apart by the characters alone.
fn is_foreign(c: char) -> bool {
    ('\u{30A0}'..='\u{30FF}').contains(&c)
}

fn own_part(text: &str) -> String {
    text.chars().filter(|c| !is_foreign(*c)).collect()
}

fn foreign_part(text: &str) -> String {
    text.chars().filter(|c| is_foreign(*c)).collect()
}

/// The `n`th character the user types, all distinct, of every width.
fn user_char(n: usize, narrow: bool) -> char {
    let cp = if narrow {
        0x4E00 + n as u32 // distinct, one width
    } else {
        match n % 4 {
            0 => 0x0100 + n as u32,
            1 => 0x4E00 + n as u32,
            2 => 0x1F300 + (n as u32 % 0x200),
            _ => 0x0400 + n as u32,
        }
    };
    char::from_u32(cp).expect("a character")
}

fn is_single_insert(before: &str, after: &str) -> bool {
    let b: Vec<char> = before.chars().collect();
    let a: Vec<char> = after.chars().collect();
    if a.len() != b.len() + 1 {
        return false;
    }
    (0..a.len()).any(|skip| {
        a.iter()
            .enumerate()
            .filter(|(i, _)| *i != skip)
            .map(|(_, c)| *c)
            .eq(b.iter().copied())
    })
}

/// A forward step's own part, and whether it was a typed key's shape.
struct Checkpoint {
    own: String,
    typed: bool,
}

struct Scenario {
    rng: Rng,
    user: Replica,
    other: Replica,
    observer: Replica,
    daemon_peer: u64,
    /// Own text after each forward update, `[0]` the file's.
    checkpoints: Vec<Checkpoint>,
    at: usize,
    redo: Vec<usize>,
    next_char: usize,
    next_foreign: u32,
    other_redo: usize,
    narrow: bool,
    prefix_only: bool,
    trace: Vec<String>,
}

impl Scenario {
    fn includes(have: &loro::VersionVector, want: &loro::VersionVector) -> bool {
        want.iter()
            .all(|(peer, end)| have.get(peer).copied().unwrap_or(0) >= *end)
    }

    fn daemon_ops(&self, r: &Replica) -> i32 {
        r.state
            .version()
            .get(&self.daemon_peer)
            .copied()
            .unwrap_or(0)
    }

    /// Pump the observer until it holds `want`: the daemon applied it.
    fn observer_holds(&mut self, want: &loro::VersionVector) {
        pump_until(&mut self.observer, "the observer to catch up", |r| {
            Self::includes(&r.state.version(), want)
        });
    }

    /// Pump `user` or `other` up to what the daemon has applied.
    fn user_catches_up(&mut self) {
        let want = self.observer.state.version();
        pump_until(&mut self.user, "the user to catch up", |r| {
            Self::includes(&r.state.version(), &want)
        });
    }

    fn other_catches_up(&mut self) {
        let want = self.observer.state.version();
        pump_until(&mut self.other, "the other to catch up", |r| {
            Self::includes(&r.state.version(), &want)
        });
    }

    /// One step of the other frontend's, forced at the daemon by the
    /// observer before anything else is sent.
    fn foreign_step(&mut self) {
        if self.rng.chance(70) {
            self.other_catches_up();
        }
        let foreign_chars = foreign_part(&self.observer.state.materialize_string());
        let roll = self.rng.below(100);
        if roll < 15 {
            // The key route: dispatched at the daemon, on its own peer.
            let c = char::from_u32(0x30A1 + (self.next_foreign % 0x50)).expect("katakana");
            self.next_foreign += 1;
            self.trace.push(format!("other keys {c}"));
            let ops = self.daemon_ops(&self.observer);
            let home = if self.prefix_only {
                Key::Home
            } else {
                Key::End
            };
            send_key(&mut self.other, home, Modifiers::NONE);
            send_key(&mut self.other, Key::Char(c), Modifiers::NONE);
            self.other_redo = 0;
            let peer = self.daemon_peer;
            pump_until(&mut self.observer, "the other's key", |r| {
                r.state.version().get(&peer).copied().unwrap_or(0) > ops
            });
        } else if roll < 75 || self.prefix_only && roll < 85 {
            let text = self.other.state.materialize_string();
            let boundaries: Vec<usize> = text
                .char_indices()
                .map(|(i, _)| i)
                .chain([text.len()])
                .collect();
            let at = if self.prefix_only {
                0
            } else {
                boundaries[self.rng.below(boundaries.len())]
            };
            let c = char::from_u32(0x30A1 + (self.next_foreign % 0x50)).expect("katakana");
            self.next_foreign += 1;
            self.trace.push(format!("other types {c} at {at}"));
            type_optimistic(&mut self.other, at, &c.to_string());
            self.other_redo = 0;
            let want = self.other.state.version();
            self.observer_holds(&want);
        } else if roll < 90 && !foreign_chars.is_empty() {
            self.trace.push("other undoes".to_owned());
            let ops = self.daemon_ops(&self.observer);
            send_undo(&mut self.other);
            let peer = self.daemon_peer;
            pump_until(&mut self.observer, "the other's undo", |r| {
                r.state.version().get(&peer).copied().unwrap_or(0) > ops
            });
            self.other_redo += 1;
        } else if self.other_redo > 0 {
            self.trace.push("other redoes".to_owned());
            let ops = self.daemon_ops(&self.observer);
            send_redo(&mut self.other);
            let peer = self.daemon_peer;
            pump_until(&mut self.observer, "the other's redo", |r| {
                r.state.version().get(&peer).copied().unwrap_or(0) > ops
            });
            self.other_redo -= 1;
        }
    }

    /// One edit on the user's mirror, never of the first character, which
    /// keeps every foreign insert in prefix mode before the user's text.
    fn user_edit(&mut self) {
        let text = self.user.state.materialize_string();
        let first_own = text
            .char_indices()
            .find(|(_, c)| !is_foreign(*c))
            .map_or(text.len(), |(i, c)| i + c.len_utf8());
        let boundaries: Vec<usize> = text[first_own..]
            .char_indices()
            .map(|(i, _)| i + first_own)
            .chain([text.len()])
            .collect();
        let deletable: Vec<(usize, char)> = text[first_own..]
            .char_indices()
            .map(|(i, c)| (i + first_own, c))
            .filter(|(_, c)| !is_foreign(*c))
            .collect();
        let roll = self.rng.below(100);
        if roll < 30 && !deletable.is_empty() {
            let (at, c) = deletable[self.rng.below(deletable.len())];
            self.trace.push(format!("user deletes {c} at {at}"));
            self.user
                .state
                .delete(at, c.len_utf8())
                .expect("a user delete");
        } else if roll < 40 {
            let at = boundaries[self.rng.below(boundaries.len())];
            let mut s = String::new();
            for _ in 0..2 + self.rng.below(3) {
                s.push(user_char(self.next_char, self.narrow));
                self.next_char += 1;
            }
            self.trace.push(format!("user pastes {s} at {at}"));
            self.user.state.insert(at, &s).expect("a user paste");
        } else {
            let at = boundaries[self.rng.below(boundaries.len())];
            let c = user_char(self.next_char, self.narrow);
            self.next_char += 1;
            self.trace.push(format!("user types {c} at {at}"));
            self.user
                .state
                .insert(at, &c.to_string())
                .expect("a user key");
        }
    }

    fn own_now(&self) -> String {
        own_part(&self.user.state.materialize_string())
    }

    fn record_forward(&mut self) {
        let own = self.own_now();
        if own == self.checkpoints[self.at].own {
            return; // Net nothing: the daemon records nothing.
        }
        self.checkpoints.truncate(self.at + 1);
        let typed = is_single_insert(&self.checkpoints[self.at].own, &own);
        self.checkpoints.push(Checkpoint { own, typed });
        self.at += 1;
        self.redo.clear();
    }

    /// One update of the user's: one edit or a batch, around foreign steps
    /// it may or may not have imported.
    fn forward_step(&mut self) {
        if self.rng.chance(50) {
            self.user_catches_up();
        }
        let before = self.user.state.version();
        let edits = if self.rng.chance(60) {
            1
        } else {
            2 + self.rng.below(3)
        };
        for i in 0..edits {
            self.user_edit();
            if i + 1 < edits {
                if self.rng.chance(40) {
                    self.foreign_step();
                }
                if self.rng.chance(40) {
                    self.user_catches_up();
                }
            }
        }
        if self.rng.chance(30) {
            self.foreign_step();
        }
        self.trace.push(format!(
            "user sends, its mirror {:?}, the daemon {:?}",
            self.user.state.materialize_string(),
            self.observer.state.materialize_string()
        ));
        send_since(&mut self.user, &before);
        let want = self.user.state.version();
        self.observer_holds(&want);
        self.record_forward();
    }

    /// A key the user sends rather than makes: dispatched at the daemon on
    /// its own peer and recorded as a span, beside the changes the user's
    /// imports are recorded as (`RecordedOps`), joining the same typed run
    /// where the keys are consecutive. No cursor key before it when the run
    /// should stay open: the caret stays where the daemon's last edit left
    /// it, and only the first of these moves it to the end of the line.
    fn key_step(&mut self, move_caret: bool) {
        let c = user_char(self.next_char, self.narrow);
        self.next_char += 1;
        self.trace.push(format!("user keys {c}"));
        let ops = self.daemon_ops(&self.observer);
        if move_caret {
            send_key(&mut self.user, Key::End, Modifiers::NONE);
        }
        send_key(&mut self.user, Key::Char(c), Modifiers::NONE);
        let peer = self.daemon_peer;
        pump_until(&mut self.observer, "the user's key", |r| {
            r.state.version().get(&peer).copied().unwrap_or(0) > ops
        });
        self.user_catches_up();
        self.record_forward();
    }

    /// Where the user's undo may land from `at`: one step back from a
    /// non-typed update, back through its run from a typed one.
    fn undo_floor(&self) -> usize {
        let mut k = self.at;
        if !self.checkpoints[k].typed {
            return k - 1;
        }
        while k > 0 && self.checkpoints[k].typed {
            k -= 1;
        }
        k
    }

    fn fail(&self, what: &str) -> ! {
        panic!("{what}\ntrace:\n  {}", self.trace.join("\n  "));
    }

    /// The user's history step `send`, its effect awaited: the text every
    /// replica has afterward, the foreign part untouched.
    fn step_and_settle(&mut self, send: fn(&mut Replica), what: &str) {
        let foreign = foreign_part(&self.observer.state.materialize_string());
        let ops = self.daemon_ops(&self.observer);
        send(&mut self.user);
        let peer = self.daemon_peer;
        pump_until(&mut self.user, what, |r| {
            r.state.version().get(&peer).copied().unwrap_or(0) > ops
        });
        let want = self.user.state.version();
        self.observer_holds(&want);
        self.user_catches_up();
        let after = foreign_part(&self.user.state.materialize_string());
        if after != foreign {
            let msg = format!(
                "{what}: the foreign part went from {foreign:?} to {after:?}; text {:?}",
                self.user.state.materialize_string()
            );
            self.fail(&msg);
        }
    }

    fn undo_step(&mut self) {
        if self.at == 0 {
            return;
        }
        let from = self.at;
        self.trace.push(format!(
            "user undoes from step {from}, daemon {:?}",
            self.observer.state.materialize_string()
        ));
        self.step_and_settle(send_undo, "the user's undo");
        let own = self.own_now();
        let floor = self.undo_floor();
        let found = (floor..from)
            .rev()
            .find(|j| self.checkpoints[*j].own == own);
        let Some(j) = found else {
            let cands: Vec<&str> = self.checkpoints[floor..from]
                .iter()
                .map(|c| c.own.as_str())
                .collect();
            let msg = format!(
                "undo from step {from} (own {:?}) gave {own:?}, text {:?}; expected one of \
                 {cands:?}",
                self.checkpoints[from].own,
                self.user.state.materialize_string()
            );
            self.fail(&msg);
        };
        self.redo.push(from);
        self.at = j;
        self.prefix_holds();
    }

    fn redo_step(&mut self) {
        let Some(to) = self.redo.pop() else {
            return;
        };
        self.trace.push(format!("user redoes to step {to}"));
        self.step_and_settle(send_redo, "the user's redo");
        let own = self.own_now();
        if own != self.checkpoints[to].own {
            let msg = format!(
                "redo to step {to} gave {own:?}, text {:?}; expected {:?}",
                self.user.state.materialize_string(),
                self.checkpoints[to].own
            );
            self.fail(&msg);
        }
        self.at = to;
        self.prefix_holds();
    }

    /// `C-x u` and a key right behind it, made on the mirror before the
    /// compensation arrives. Its model is the undo's result with the one
    /// new character.
    fn undo_with_a_key_behind_it(&mut self) {
        if self.at == 0 {
            return;
        }
        let from = self.at;
        let floor = self.undo_floor();
        let foreign_before = foreign_part(&self.observer.state.materialize_string());
        let ops = self.daemon_ops(&self.observer);
        send_undo(&mut self.user);
        let before = self.user.state.version();
        let text = self.user.state.materialize_string();
        let c = user_char(self.next_char, self.narrow);
        self.next_char += 1;
        let at = text.len();
        self.trace
            .push(format!("user undoes from step {from}, {c} right behind it"));
        self.user
            .state
            .insert(at, &c.to_string())
            .expect("a key behind the undo");
        send_since(&mut self.user, &before);
        let peer = self.daemon_peer;
        pump_until(&mut self.user, "the undo behind a key", |r| {
            r.state.version().get(&peer).copied().unwrap_or(0) > ops
        });
        let want = self.user.state.version();
        self.observer_holds(&want);
        self.user_catches_up();
        let own = self.own_now();
        let without: String = own.chars().filter(|x| *x != c).collect();
        let found = (floor..from)
            .rev()
            .find(|j| self.checkpoints[*j].own == without);
        let Some(j) = found else {
            let msg = format!(
                "the undo with {c} behind it, from step {from}, gave {own:?}, text {:?}",
                self.user.state.materialize_string()
            );
            self.fail(&msg);
        };
        let after = foreign_part(&self.user.state.materialize_string());
        if after != foreign_before {
            let msg = format!(
                "the undo with a key behind it: the foreign part went from {foreign_before:?} \
                 to {after:?}"
            );
            self.fail(&msg);
        }
        self.checkpoints.truncate(j + 1);
        self.checkpoints.push(Checkpoint { own, typed: true });
        self.at = j + 1;
        self.redo.clear();
    }

    fn prefix_holds(&self) {
        if self.prefix_only {
            let text = self.user.state.materialize_string();
            let want = format!("{}{}", foreign_part(&text), self.checkpoints[self.at].own);
            if text != want {
                self.fail(&format!("the text is {text:?}, not {want:?}"));
            }
        }
    }

    fn converge(&mut self) {
        let want = self.observer.state.version();
        pump_until(&mut self.user, "convergence", |r| {
            Self::includes(&r.state.version(), &want)
        });
        pump_until(&mut self.other, "convergence", |r| {
            Self::includes(&r.state.version(), &want)
        });
        let (u, o, b) = (
            self.user.state.materialize_string(),
            self.other.state.materialize_string(),
            self.observer.state.materialize_string(),
        );
        if u != o || o != b {
            self.fail(&format!("no convergence: {u:?} {o:?} {b:?}"));
        }
    }
}

fn run_scenario(seed: u64, trace_out: &std::sync::Mutex<Vec<String>>) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let file = dir.path().join("seed.txt");
    let narrow = seed.is_multiple_of(2);
    let prefix_only = seed.is_multiple_of(3);
    let seed_text = if narrow {
        "0123456789"
    } else {
        "0é2日4𝄞6789"
    };
    std::fs::write(&file, seed_text).expect("write the file");
    let daemon = TestDaemon::holding(&file);
    let user = attach_replica(&daemon);
    let other = attach_replica(&daemon);
    let observer = attach_replica(&daemon);
    let mut s = Scenario {
        rng: Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1),
        user,
        other,
        observer,
        daemon_peer: pmacs::crdt::peer_id_from_frontend(FrontendId::LOCAL),
        checkpoints: vec![Checkpoint {
            own: seed_text.to_owned(),
            typed: false,
        }],
        at: 0,
        redo: Vec::new(),
        next_char: 0,
        next_foreign: 0,
        other_redo: 0,
        narrow,
        prefix_only,
        trace: vec![format!("seed {seed} narrow {narrow} prefix {prefix_only}")],
    };
    let steps = 12 + s.rng.below(25);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for _ in 0..steps {
            match s.rng.below(100) {
                0..=19 => s.foreign_step(),
                20..=59 => s.forward_step(),
                60..=69 => s.key_step(true),
                70..=84 => s.undo_step(),
                85..=94 => s.redo_step(),
                _ => s.undo_with_a_key_behind_it(),
            }
        }
        while s.at > 0 {
            s.undo_step();
        }
        s.converge();
        let own = s.own_now();
        if own != seed_text {
            s.fail(&format!(
                "undoing everything left {own:?}, not {seed_text:?}"
            ));
        }
        while !s.redo.is_empty() {
            s.redo_step();
        }
        s.converge();
    }));
    trace_out.lock().expect("trace").clone_from(&s.trace);
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

/// Random crossings of one user and one other frontend, single keys,
/// pastes and batches, with undo and redo, against an oracle that knows
/// nothing of the record: with the foreign characters filtered out, the
/// user's undo goes back to a text the user had (exactly one step from
/// anything but a typed run) and its redo forward again, the foreign part
/// is untouched, every replica converges, and undoing everything gives
/// the file. `PMACS_R2_SEEDS=from..to` sets the seeds, 40 by default. Fails
/// at `fc91a95`'s and `a70487f`'s records on seeds 1, 3, 4, 5 and 6, where
/// it stops at its fifth failure.
#[test]
fn r2_p8_random_crossings_keep_the_users_own_text_on_its_steps() {
    let spec = std::env::var("PMACS_R2_SEEDS").unwrap_or_else(|_| "1..41".to_owned());
    let (from, to) = spec.split_once("..").expect("from..to");
    let (from, to): (u64, u64) = (from.parse().expect("from"), to.parse().expect("to"));
    let mut failures = Vec::new();
    for seed in from..to {
        let trace = std::sync::Mutex::new(Vec::new());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_scenario(seed, &trace);
        }));
        if let Err(panic) = result {
            let msg = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "a panic".to_owned());
            failures.push((seed, msg));
            if failures.len() >= 5 {
                break;
            }
        }
    }
    eprintln!(
        "r2_p8: seeds {from}..{to}: {} failed: {:?}",
        failures.len(),
        failures.iter().map(|(s, _)| *s).collect::<Vec<_>>()
    );
    for (seed, msg) in &failures {
        eprintln!("--- seed {seed}\n{msg}");
    }
    assert!(failures.is_empty(), "{} seeds failed", failures.len());
}

// ---------------------------------------------------------------------
// Part C: the integration. E10l's minimap across E10o's undo, a GPU peer
// as `pmacs-gpu` attaches, sending what it sends.
// ---------------------------------------------------------------------

mod integration {
    use super::*;
    use common::wire::WireSession;
    use pmacs::cell::Style;
    use pmacs_protocol::message::{InstanceMessage, PROTOCOL_VERSION, StyleSegment};

    const DEADLINE: Duration = Duration::from_secs(30);

    fn issue_439_json(entries: usize) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        out.push('[');
        for i in 0..entries {
            out.push_str(if i == 0 { "\n" } else { ",\n" });
            write!(
                out,
                " {{\n  \"id\": {i},\n  \"name\": \"item {i}\",\n  \"tags\": [\n   \"a\",\n   \"b\"\n  ],\n  \"value\": {:?}\n }}",
                f64::from(u32::try_from(i).expect("small")) * 0.5
            )
            .expect("write");
        }
        out.push_str("\n]\n");
        out
    }

    fn holding_traced(file: &std::path::Path, trace: &std::path::Path) -> TestDaemon {
        let init_lua = format!(
            "pmacs.lsp.config = {{}}\npmacs.buffer.find_or_open({:?})\nfor _, b in ipairs(pmacs.buffer.list()) do\n  if b:name() == '*scratch*' then pcall(pmacs.buffer.kill, b) end\nend\n",
            file.display().to_string()
        );
        TestDaemon::spawn_with_env_and_init(
            &[("PMACS_E7I_TRACE", trace.to_str().expect("a UTF-8 path"))],
            &init_lua,
        )
    }

    fn parsed_units(trace: &std::path::Path) -> Vec<u32> {
        std::fs::read_to_string(trace)
            .unwrap_or_default()
            .lines()
            .filter(|line| line.contains("\"event\":\"parsed\""))
            .filter_map(|line| {
                line.split("\"unit\":\"pid ")
                    .nth(1)?
                    .split('"')
                    .next()?
                    .parse()
                    .ok()
            })
            .collect()
    }

    fn signal(sig: &str, pid: u32) -> bool {
        std::process::Command::new("kill")
            .args([sig, &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    /// A GPU peer with its mirror: everything it was sent since the last
    /// look, the CRDT ops imported, the last summary kept.
    struct Gpu {
        session: WireSession,
        mirror: CrdtState,
        buffer: pmacs_protocol::BufferId,
        summaries: Vec<(u64, Vec<Style>)>,
        /// The viewport's spans as the last `full` frame and the partial
        /// ones after it describe them, with the generation they name.
        spans: Vec<StyleSegment>,
        spans_generation: u64,
    }

    impl Gpu {
        fn attach(daemon: &TestDaemon) -> Self {
            let mut session =
                WireSession::semantic_with_patience(daemon, PROTOCOL_VERSION, None, None, DEADLINE);
            let (buffer, snap) = session
                .bootstrap
                .iter()
                .find_map(|m| match m {
                    InstanceMessage::BufferSnapshot {
                        buffer_id,
                        crdt_snapshot,
                    } => Some((*buffer_id, crdt_snapshot.clone())),
                    _ => None,
                })
                .expect("the bootstrap snapshots the file");
            session.bootstrap.clear();
            let mirror = CrdtState::new(session.fid.0).expect("mirror");
            mirror.import_snapshot(&snap).expect("snapshot");
            session.declare_viewport(buffer);
            Self {
                session,
                mirror,
                buffer,
                summaries: Vec::new(),
                spans: Vec::new(),
                spans_generation: 0,
            }
        }

        fn take(&mut self, msg: InstanceMessage) {
            match msg {
                InstanceMessage::CrdtOp { buffer_id, op } if buffer_id == self.buffer => {
                    self.mirror.import_updates(&op.bytes).expect("import");
                }
                InstanceMessage::FileStyleSummary {
                    buffer_id,
                    generation,
                    lines,
                } if buffer_id == self.buffer => self.summaries.push((generation, lines)),
                InstanceMessage::StyleSpans {
                    buffer_id,
                    generation,
                    full,
                    segments,
                } if buffer_id == self.buffer => {
                    if full {
                        self.spans = segments;
                    } else {
                        self.spans.extend(segments);
                    }
                    self.spans_generation = generation;
                }
                _ => {}
            }
        }

        /// Read until `accept` holds or the deadline.
        fn until(&mut self, what: &str, accept: impl Fn(&Self) -> bool) {
            let started = Instant::now();
            while !accept(self) {
                assert!(
                    started.elapsed() < DEADLINE,
                    "{what}: not within {DEADLINE:?}; mirror {} bytes, {} summaries",
                    self.mirror.len_utf8(),
                    self.summaries.len(),
                );
                match self.session.rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(msg) => self.take(msg),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        panic!("{what}: the daemon closed the session")
                    }
                }
            }
        }

        /// Read for `wait`, whatever comes.
        fn for_a_while(&mut self, wait: Duration) {
            let started = Instant::now();
            while started.elapsed() < wait {
                if let Ok(msg) = self.session.rx.recv_timeout(Duration::from_millis(20)) {
                    self.take(msg);
                }
            }
        }

        fn send_since(&mut self, from: &loro::VersionVector) {
            let bytes = self.mirror.export_updates_since(from).expect("export");
            write_message(
                &mut self.session.stream,
                &FrontendEvent::CrdtOp {
                    frontend_id: self.session.fid,
                    buffer_id: self.buffer,
                    op: RopeCrdtOp {
                        peer_id: self.session.fid.0,
                        bytes,
                    },
                },
            )
            .expect("write CrdtOp");
        }

        fn undo(&mut self) {
            self.session.key(Key::Char('x'), Modifiers::CTRL);
            self.session.key(Key::Char('u'), Modifiers::NONE);
        }

        fn redo(&mut self) {
            self.session.key(Key::Char('x'), Modifiers::CTRL);
            self.session.key(Key::Char('r'), Modifiers::NONE);
        }

        /// The last summary, once it is for `lines` lines; the summaries
        /// come late by the quiet window.
        fn summary_of(&mut self, lines: usize, what: &str) -> Vec<Style> {
            self.until(what, |g| {
                g.summaries.last().is_some_and(|(_, s)| s.len() == lines)
            });
            self.summaries.last().expect("checked").1.clone()
        }
    }

    /// What a fresh daemon's first summary and viewport spans say of
    /// `text`: the file opened anew, the reference for the live one.
    fn fresh_reading(text: &str) -> (Vec<Style>, Vec<(u64, u64, Style)>) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = dir.path().join("fresh.json");
        let trace = dir.path().join("fresh-trace.jsonl");
        std::fs::write(&file, text).expect("write");
        let daemon = holding_traced(&file, &trace);
        let mut gpu = Gpu::attach(&daemon);
        let lines = text.bytes().filter(|b| *b == b'\n').count() + 1;
        let summary = gpu.summary_of(lines, "the fresh file's summary");
        gpu.until("the fresh file's spans", |g| !g.spans.is_empty());
        gpu.for_a_while(Duration::from_millis(300));
        let spans = flat_spans(&gpu.spans);
        (summary, spans)
    }

    /// The styled runs inside the first 4096 bytes, by byte, the later
    /// segment winning: what a frontend paints whichever way it was cut.
    fn flat_spans(segments: &[StyleSegment]) -> Vec<(u64, u64, Style)> {
        let mut by_byte: Vec<Option<Style>> = vec![None; 4096];
        for segment in segments {
            for byte in segment.range.start..segment.range.end.min(4096) {
                by_byte[byte as usize] = None;
            }
            for span in &segment.spans {
                for byte in span.range.start..span.range.end.min(4096) {
                    by_byte[byte as usize] = Some(span.style);
                }
            }
        }
        let mut out: Vec<(u64, u64, Style)> = Vec::new();
        for (byte, style) in by_byte.iter().enumerate() {
            let Some(style) = style else { continue };
            let byte = byte as u64;
            match out.last_mut() {
                Some((_, end, last)) if *end == byte && last == style => *end = byte + 1,
                _ => out.push((byte, byte + 1, *style)),
            }
        }
        out
    }

    fn styled(summary: &[Style]) -> usize {
        summary.iter().filter(|s| **s != Style::default()).count()
    }

    /// Compare what the live GPU peer holds with a fresh daemon's reading
    /// of the same text.
    fn holds_the_files_own(gpu: &mut Gpu, text: &str, what: &str) {
        let lines = text.bytes().filter(|b| *b == b'\n').count() + 1;
        gpu.until(&format!("{what}: the text"), |g| {
            g.mirror.materialize_string() == text
        });
        gpu.summary_of(lines, &format!("{what}: the summary"));
        // Spans follow the parse; let the frame that names this text come,
        // and any later summary with it.
        gpu.for_a_while(Duration::from_secs(2));
        let live = gpu.summaries.last().expect("a summary").1.clone();
        let (fresh, fresh_spans) = fresh_reading(text);
        assert_eq!(
            live.len(),
            fresh.len(),
            "{what}: a summary line per line of the text"
        );
        let wrong = live
            .iter()
            .zip(fresh.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        assert!(
            wrong.is_empty(),
            "{what}: {} of {} lines styled unlike the file opened anew, first {:?} ({} styled \
             live, {} fresh)",
            wrong.len(),
            live.len(),
            wrong.first(),
            styled(&live),
            styled(&fresh)
        );
        let live_spans = flat_spans(&gpu.spans);
        assert_eq!(
            live_spans.len(),
            fresh_spans.len(),
            "{what}: the viewport's runs, live and on a file opened anew; first difference {:?}",
            live_spans
                .iter()
                .zip(fresh_spans.iter())
                .find(|(a, b)| a != b)
        );
        assert_eq!(live_spans, fresh_spans, "{what}: the viewport's runs");
    }

    fn setup(entries: usize) -> (tempfile::TempDir, std::path::PathBuf, String, TestDaemon) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let file = dir.path().join("small.json");
        let trace = dir.path().join("trace.jsonl");
        let text = issue_439_json(entries);
        std::fs::write(&file, &text).expect("write the file");
        let daemon = holding_traced(&file, &trace);
        (dir, trace, text, daemon)
    }

    /// The offset just before the `n`th newline.
    fn before_newline(text: &str, n: usize) -> usize {
        text.match_indices('\n').nth(n).expect("that many lines").0
    }

    /// E10l and E10o together on the shipped route: the GPU peer's key,
    /// a line break made without the other frontend's two line breaks at
    /// 0, which the daemon held. The minimap and the viewport's spans, in
    /// the crossing, after its undo and after the redo, are each what the
    /// same text opened anew gives. Fails at the text at `fc91a95`'s and
    /// `a70487f`'s records, and on the viewport's runs at plant G.
    #[test]
    fn r2_i1_the_minimap_across_a_crossing_keys_undo_and_redo_is_the_files_own() {
        let (_dir, _trace, text, daemon) = setup(300);
        let mut gpu = Gpu::attach(&daemon);
        let mut other = attach_replica(&daemon);
        let mut observer = attach_replica(&daemon);
        let lines = text.bytes().filter(|b| *b == b'\n').count() + 1;
        let first = gpu.summary_of(lines, "the first summary");
        assert!(styled(&first) > lines / 2, "the first summary is styled");

        let at = before_newline(&text, 3);
        let v = gpu.mirror.version();
        gpu.mirror.insert(at, "\n").expect("the line break, held");
        let foreign = format!("\n\n{text}");
        {
            let vv = other.state.version();
            other.state.insert(0, "\n\n").expect("two line breaks at 0");
            send_since(&mut other, &vv);
        }
        pump_until_text(&mut observer, &foreign, "the observer: the line breaks");
        gpu.send_since(&v);
        let crossed = format!("\n\n{}\n{}", &text[..at], &text[at..]);
        holds_the_files_own(&mut gpu, &crossed, "the crossing");

        gpu.undo();
        holds_the_files_own(&mut gpu, &foreign, "the undo");

        gpu.redo();
        holds_the_files_own(&mut gpu, &crossed, "the redo");
    }

    /// The same keys with no pause: the undo lands inside the summary's
    /// quiet window, so no summary describes the crossing; the one that
    /// comes describes the text after the undo. Fails at the text at
    /// `fc91a95`'s and `a70487f`'s records, and on the viewport's runs at
    /// plant G.
    #[test]
    fn r2_i2_an_undo_inside_the_quiet_window_leaves_the_files_own_minimap() {
        let (_dir, _trace, text, daemon) = setup(300);
        let mut gpu = Gpu::attach(&daemon);
        let mut other = attach_replica(&daemon);
        let mut observer = attach_replica(&daemon);
        let lines = text.bytes().filter(|b| *b == b'\n').count() + 1;
        gpu.summary_of(lines, "the first summary");

        let at = before_newline(&text, 3);
        let v = gpu.mirror.version();
        gpu.mirror.insert(at, "\n").expect("the line break, held");
        let foreign = format!("\n\n{text}");
        {
            let vv = other.state.version();
            other.state.insert(0, "\n\n").expect("two line breaks at 0");
            send_since(&mut other, &vv);
        }
        pump_until_text(&mut observer, &foreign, "the observer: the line breaks");
        gpu.send_since(&v);
        gpu.undo();
        holds_the_files_own(&mut gpu, &foreign, "the undo in the quiet window");
    }

    /// E10l fix round 2's failed read inside E10o's undo: the crossing
    /// key's parse in a worker stopped before the summary reads it, the
    /// read past `READ_HARD`, the worker discarded, nothing sent; then the
    /// undo is the next edit, and what comes is the file's own. Fails at the
    /// text at `fc91a95`'s and `a70487f`'s records, and on the viewport's
    /// runs at plant G.
    #[test]
    fn r2_i3_a_failed_whole_file_read_then_the_crossing_keys_undo_is_the_files_own() {
        let (_dir, trace, text, daemon) = setup(300);
        let mut gpu = Gpu::attach(&daemon);
        let mut other = attach_replica(&daemon);
        let mut observer = attach_replica(&daemon);
        let lines = text.bytes().filter(|b| *b == b'\n').count() + 1;
        let first = gpu.summary_of(lines, "the first summary");
        let sent = gpu.summaries.len();
        let parses = parsed_units(&trace).len();

        let at = before_newline(&text, 3);
        let v = gpu.mirror.version();
        gpu.mirror.insert(at, "\n").expect("the line break, held");
        let foreign = format!("\n\n{text}");
        {
            let vv = other.state.version();
            other.state.insert(0, "\n\n").expect("two line breaks at 0");
            send_since(&mut other, &vv);
        }
        pump_until_text(&mut observer, &foreign, "the observer: the line breaks");
        gpu.send_since(&v);
        let crossed = format!("\n\n{}\n{}", &text[..at], &text[at..]);
        gpu.until("the crossing", |g| g.mirror.materialize_string() == crossed);

        // The crossing's parse: the last worker to have answered once the
        // trace stops growing for 200 ms, well inside the summary's quiet
        // second, then the stop.
        let asked = Instant::now();
        let worker = loop {
            let units = parsed_units(&trace);
            std::thread::sleep(Duration::from_millis(200));
            let later = parsed_units(&trace);
            if later.len() > parses && later.len() == units.len() {
                break *later.last().expect("a parse");
            }
            assert!(
                asked.elapsed() < Duration::from_secs(20),
                "no parse of the crossing in the trace ({later:?})"
            );
        };
        assert!(signal("-STOP", worker), "stop the worker {worker}");
        let stopped = Instant::now();
        let gone = loop {
            gpu.for_a_while(Duration::from_millis(100));
            if !signal("-0", worker) {
                break true;
            }
            if stopped.elapsed() > Duration::from_secs(20) {
                break false;
            }
        };
        if !gone {
            signal("-CONT", worker);
        }
        assert!(gone, "the read never reached the stopped worker");
        gpu.for_a_while(Duration::from_secs(2));
        assert_eq!(
            gpu.summaries.len(),
            sent,
            "a failed read sends nothing over the good summary ({} styled)",
            styled(&first)
        );

        gpu.undo();
        holds_the_files_own(&mut gpu, &foreign, "the undo after the failed read");
    }

    /// With no foreign edit the undo gives back the text of the last
    /// summary sent, and the read failed in between: the summary is
    /// suppressed as unchanged, which is right only because the frontend
    /// still holds it. A guard at `fc91a95`'s and `a70487f`'s records, where
    /// it passes since nothing crosses; it fails on the viewport's runs at
    /// plant G, the undo's edit never reaching the parse.
    #[test]
    fn r2_i4_a_failed_read_then_an_undo_to_the_last_sent_text_keeps_that_summary() {
        let (_dir, trace, text, daemon) = setup(300);
        let mut gpu = Gpu::attach(&daemon);
        let lines = text.bytes().filter(|b| *b == b'\n').count() + 1;
        let first = gpu.summary_of(lines, "the first summary");
        let sent = gpu.summaries.len();
        let parses = parsed_units(&trace).len();

        let at = before_newline(&text, 3);
        let v = gpu.mirror.version();
        gpu.mirror.insert(at, "\n").expect("a line break");
        gpu.send_since(&v);
        let asked = Instant::now();
        let worker = loop {
            if let Some(&pid) = parsed_units(&trace).get(parses) {
                break pid;
            }
            assert!(asked.elapsed() < Duration::from_secs(20), "no parse");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(signal("-STOP", worker), "stop the worker {worker}");
        let stopped = Instant::now();
        let gone = loop {
            gpu.for_a_while(Duration::from_millis(100));
            if !signal("-0", worker) {
                break true;
            }
            if stopped.elapsed() > Duration::from_secs(20) {
                break false;
            }
        };
        if !gone {
            signal("-CONT", worker);
        }
        assert!(gone, "the read never reached the stopped worker");
        gpu.for_a_while(Duration::from_secs(2));
        assert_eq!(gpu.summaries.len(), sent, "a failed read sends nothing");

        gpu.undo();
        gpu.until("the undo", |g| g.mirror.materialize_string() == text);
        gpu.for_a_while(Duration::from_secs(4));
        let last = gpu.summaries.last().expect("the first").1.clone();
        assert_eq!(last, first, "the frontend still holds the file's summary");
        holds_the_files_own(&mut gpu, &text, "the undo to the last sent text");
    }

    /// E10o.1's batch through the GPU route: two line breaks in one update,
    /// the user having imported the other's edit between them and not its
    /// next, undone and redone; the minimap each time is the file's own.
    /// Fails at the text at `a70487f`'s record, and at plant G on the minimap
    /// (598 of its 2,705 lines); it passes at `fc91a95`'s, which already
    /// records a batch change by change.
    #[test]
    fn r2_i5_a_batched_crossing_updates_the_minimap_at_each_step() {
        let (_dir, _trace, text, daemon) = setup(300);
        let mut gpu = Gpu::attach(&daemon);
        let mut other = attach_replica(&daemon);
        let mut observer = attach_replica(&daemon);
        let lines = text.bytes().filter(|b| *b == b'\n').count() + 1;
        gpu.summary_of(lines, "the first summary");

        let first_at = before_newline(&text, 3);
        let second_at = before_newline(&text, 8);
        let v = gpu.mirror.version();
        gpu.mirror
            .insert(second_at, "\n")
            .expect("the later line break, held");
        {
            let vv = other.state.version();
            other.state.insert(0, "\n").expect("a line break at 0");
            send_since(&mut other, &vv);
        }
        let one = format!("\n{text}");
        pump_until_text(&mut observer, &one, "the observer: the first");
        gpu.until("the mirror imports the first", |g| {
            g.mirror.materialize_string().starts_with("\n[")
        });
        gpu.mirror
            .insert(first_at + 1, "\n")
            .expect("the earlier line break, held");
        {
            let vv = other.state.version();
            other.state.insert(0, "\n").expect("a second at 0");
            send_since(&mut other, &vv);
        }
        let two = format!("\n\n{text}");
        pump_until_text(&mut observer, &two, "the observer: the second");
        gpu.send_since(&v);
        let crossed = format!(
            "\n\n{}\n{}\n{}",
            &text[..first_at],
            &text[first_at..second_at],
            &text[second_at..]
        );
        holds_the_files_own(&mut gpu, &crossed, "the batch");
        gpu.undo();
        holds_the_files_own(&mut gpu, &two, "the batch undone");
        gpu.redo();
        holds_the_files_own(&mut gpu, &crossed, "the batch redone");
    }
}
