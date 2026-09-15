// tests/e6c_review1_undo_across_peers_probes.rs --- E6c review 1: the
// arbiter across peers, in the states the phase's witnesses left out.

//! Behavioral probes against E6c.2 and E6c.3 through a real daemon and
//! two synthetic replicas, as `undo_across_peers_acceptance` has them:
//! `gpu` types optimistically on its own peer, `tui` types through
//! round-tripped keys onto the daemon's peer, and every undo and redo
//! chord round-trips.
//!
//! The states: the everyday sequence the net-diff base got wrong, on
//! the GPU route; a foreign edit landing between an undo and its redo
//! (what redo means then); foreign text inside the newest group's own
//! span before that group is undone (the absorb direction the
//! handoff names as unwitnessed); redo stacks per source; and a
//! sourceless edit --- a script's insert at the daemon, outside any
//! command --- followed by the GPU user's undo.
//!
//! Helpers are the acceptance suite's, copied so the probe stands
//! alone.

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

/// The GPU route: one optimistic op per character, on the replica's
/// own peer, inserted at `at` in the mirror.
fn type_optimistic(replica: &mut Replica, at: usize, text: &str) {
    for (i, ch) in text.chars().enumerate() {
        let v = replica.state.version();
        replica
            .state
            .insert(at + i, &ch.to_string())
            .expect("mirror insert");
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

/// The TUI route: every character a round-tripped key, landing at the
/// daemon's cursor for this frontend.
fn type_keys(replica: &mut Replica, text: &str) {
    for ch in text.chars() {
        send_key(replica, Key::Char(ch), Modifiers::NONE);
    }
}

/// `C-x u`, the prefix chord that round-trips on every frontend.
fn send_undo(replica: &mut Replica) {
    send_key(replica, Key::Char('x'), Modifiers::CTRL);
    send_key(replica, Key::Char('u'), Modifiers::NONE);
}

/// `C-x r`, `buffer.redo`.
fn send_redo(replica: &mut Replica) {
    send_key(replica, Key::Char('x'), Modifiers::CTRL);
    send_key(replica, Key::Char('r'), Modifiers::NONE);
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

fn both_see(gpu: &mut Replica, tui: &mut Replica, expected: &str, what: &str) {
    pump_until_text(gpu, expected, &format!("gpu {what}"));
    pump_until_text(tui, expected, &format!("tui {what}"));
}

/// The everyday sequence on the GPU route: `hello` optimistically,
/// `End`, Backspace twice (command chords, each its own step), then
/// undo until nothing is left: `hell`, `hello`, empty. The last undo
/// takes the run whose last two characters are now the daemon-peer
/// text the Backspace undos reinserted; a net-diff base kept them and
/// left `lo`.
#[test]
fn review_hello_backspace_twice_then_undo_to_nothing_on_the_gpu_route() {
    let daemon = TestDaemon::spawn();
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_optimistic(&mut gpu, 0, "hello");
    both_see(&mut gpu, &mut tui, "hello", "hello");
    send_key(&mut gpu, Key::End, Modifiers::NONE);
    send_key(&mut gpu, Key::Backspace, Modifiers::NONE);
    send_key(&mut gpu, Key::Backspace, Modifiers::NONE);
    both_see(&mut gpu, &mut tui, "hel", "hel after two Backspaces");

    send_undo(&mut gpu);
    both_see(&mut gpu, &mut tui, "hell", "the second Backspace undone");
    send_undo(&mut gpu);
    both_see(&mut gpu, &mut tui, "hello", "the first Backspace undone");
    send_undo(&mut gpu);
    both_see(
        &mut gpu,
        &mut tui,
        "",
        "hello gone whole, the reinserted lo with it",
    );
    send_undo(&mut gpu);
    assert_text_stays(&mut gpu, "", Duration::from_millis(500));
}

/// A foreign edit between an undo and its redo: the TUI types `abc`
/// and undoes it; the GPU types `xy`; the TUI redoes. Redo is undoing
/// the compensation against what landed since, so `abc` comes back and
/// `xy` survives --- the order of the two is the transform's and is
/// recorded here, not designed. Then the TUI's undo takes `abc` back
/// out around `xy`, and the GPU's undo takes `xy`.
#[test]
fn review_redo_after_a_foreign_edit_between_undo_and_redo() {
    let daemon = TestDaemon::spawn();
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_keys(&mut tui, "abc");
    both_see(&mut gpu, &mut tui, "abc", "abc");
    send_undo(&mut tui);
    both_see(&mut gpu, &mut tui, "", "abc undone");
    type_optimistic(&mut gpu, 0, "xy");
    both_see(&mut gpu, &mut tui, "xy", "xy from the gpu");

    send_redo(&mut tui);
    let redone = pump_until(&mut tui, "the tui's redo after the gpu's xy", |t| {
        t.contains("abc")
    });
    assert!(
        redone.contains("xy"),
        "the gpu's xy must survive the tui's redo: {redone:?}"
    );
    assert_eq!(redone.len(), 5, "abc and xy, nothing else: {redone:?}");
    eprintln!("after redo with a foreign xy between undo and redo: {redone:?}");
    pump_until_text(&mut gpu, &redone, "gpu sees the redo");

    send_undo(&mut tui);
    both_see(
        &mut gpu,
        &mut tui,
        "xy",
        "the tui's undo of its redo leaves xy",
    );
    send_undo(&mut gpu);
    both_see(&mut gpu, &mut tui, "", "the gpu's undo takes xy");
}

/// Foreign text inside the newest group's own span before that group
/// is undone: the TUI types `abcd` (one run), the GPU drops `Z` into
/// the middle of it, the TUI undoes. `abcd` goes around the foreign
/// `Z`, which stays --- the absorb direction the interleaving rows do
/// not reach, since their foreign text lands beside a group, not in it.
#[test]
fn review_foreign_text_inside_the_newest_group_survives_its_undo() {
    let daemon = TestDaemon::spawn();
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_keys(&mut tui, "abcd");
    both_see(&mut gpu, &mut tui, "abcd", "abcd");
    type_optimistic(&mut gpu, 2, "Z");
    both_see(&mut gpu, &mut tui, "abZcd", "Z inside abcd");

    send_undo(&mut tui);
    both_see(&mut gpu, &mut tui, "Z", "abcd gone around the foreign Z");
    send_undo(&mut gpu);
    both_see(&mut gpu, &mut tui, "", "Z gone");
}

/// Redo stacks are per source: the GPU types `ab`, the TUI types `xy`
/// after it, the GPU undoes `ab`. The TUI's redo finds nothing (it
/// undid nothing) and `xy` stays; the GPU's redo brings `ab` back
/// beside `xy`.
#[test]
fn review_redo_is_per_source_and_finds_only_the_sources_own_undos() {
    let daemon = TestDaemon::spawn();
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_optimistic(&mut gpu, 0, "ab");
    both_see(&mut gpu, &mut tui, "ab", "ab");
    send_key(&mut tui, Key::End, Modifiers::NONE);
    type_keys(&mut tui, "xy");
    both_see(&mut gpu, &mut tui, "abxy", "abxy");

    send_undo(&mut gpu);
    both_see(&mut gpu, &mut tui, "xy", "the gpu's ab gone");
    send_redo(&mut tui);
    assert_text_stays(&mut tui, "xy", Duration::from_millis(500));

    send_redo(&mut gpu);
    let redone = pump_until(&mut gpu, "the gpu's redo", |t| t.contains("ab"));
    assert!(
        redone.contains("xy"),
        "xy survives the gpu's redo: {redone:?}"
    );
    assert_eq!(redone.len(), 4, "ab and xy, nothing else: {redone:?}");
    eprintln!("after the gpu's redo beside the tui's xy: {redone:?}");
    pump_until_text(&mut tui, &redone, "tui sees the gpu's redo");
}

/// A `frontend.detached` hook that inserts `X` at the end of every
/// buffer: the hook runs at the daemon outside any interactive
/// command, so its insert is a daemon edit --- nobody's --- exactly as
/// a script's or a server's is.
const DETACH_INSERT_CONFIG: &str = "\
pmacs.hook.add('frontend.detached', function()
  for _, id in ipairs(pmacs.buffer.list()) do
    local ok, described = pcall(pmacs.describe.buffer, id)
    if ok and described and described.name == '*scratch*' then
      id:insert(id:len(), 'X')
    end
  end
end)
";

/// A sourceless edit, then the GPU user's undo: the GPU types
/// `hello`; a script inserts `X` at the daemon (a third frontend's
/// detach fires a hook that does it); the GPU undoes. The script's
/// insert is the newest thing, so the GPU's first undo removes `X`
/// and leaves `hello`; its second removes `hello`. The design's rule
/// --- a sourceless edit goes to whoever undoes next when it is the
/// newest --- witnessed on the route the owner uses.
#[test]
fn review_script_insert_then_gpu_undo_takes_the_script_first() {
    let daemon = TestDaemon::spawn_with_config(DETACH_INSERT_CONFIG);
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_optimistic(&mut gpu, 0, "hello");
    both_see(&mut gpu, &mut tui, "hello", "hello");
    {
        let passerby = attach_replica(&daemon);
        drop(passerby);
    }
    both_see(
        &mut gpu,
        &mut tui,
        "helloX",
        "the detach hook's script insert",
    );

    send_undo(&mut gpu);
    both_see(
        &mut gpu,
        &mut tui,
        "hello",
        "the script's X gone, hello kept",
    );
    send_undo(&mut gpu);
    both_see(&mut gpu, &mut tui, "", "hello gone");
    send_undo(&mut gpu);
    assert_text_stays(&mut gpu, "", Duration::from_millis(500));
}

/// The same sourceless edit, with the other user undoing first: the
/// GPU types `hello`, the script inserts `X`, the TUI --- which typed
/// nothing --- undoes. The TUI takes the script's `X`; a second TUI
/// undo finds nothing, `hello` being the GPU's; the GPU's undo then
/// takes `hello`. Whoever undoes next takes the sourceless edit, and
/// only that.
#[test]
fn review_script_insert_then_the_other_users_undo_takes_only_the_script() {
    let daemon = TestDaemon::spawn_with_config(DETACH_INSERT_CONFIG);
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_optimistic(&mut gpu, 0, "hello");
    both_see(&mut gpu, &mut tui, "hello", "hello");
    {
        let passerby = attach_replica(&daemon);
        drop(passerby);
    }
    both_see(
        &mut gpu,
        &mut tui,
        "helloX",
        "the detach hook's script insert",
    );

    send_undo(&mut tui);
    both_see(&mut gpu, &mut tui, "hello", "the tui took the script's X");
    send_undo(&mut tui);
    assert_text_stays(&mut tui, "hello", Duration::from_millis(500));
    send_undo(&mut gpu);
    both_see(&mut gpu, &mut tui, "", "the gpu's own hello");
}
