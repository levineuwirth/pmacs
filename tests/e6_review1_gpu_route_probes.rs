// tests/e6_review1_gpu_route_probes.rs --- E6 review 1: E6.4 on the
// route a GPU user's plain characters actually take.

//! The handoff's premise for E6.4 is that "the GPU types through
//! `TextInput`, the daemon's own edit". It does not: `text_input_payload`
//! returns `None` for a single scalar (`pmacs-gpu/src/main.rs`), so
//! every plain character a GPU user presses is an optimistic
//! `FrontendEvent::CrdtOp` on the GPU's own loro peer --- "the bulk of
//! plain-char typing" in the daemon's own words --- and the daemon's
//! peer-bound `UndoManager` never records it. This probe drives that
//! route the way `auto_pair_crdt_acceptance` does, one op per keystroke
//! as the GPU sends them, and asks what the bound undo does afterward:
//! nothing. The amalgamation E6.4 delivers is unreachable from here.
//!
//! Helpers are the CRDT auto-pair suite's, copied so the probe stands
//! alone until the fix round folds it in.

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

struct Observed {
    text: String,
    cursor: Option<u64>,
    imported: usize,
}

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

/// Five plain characters typed the way the GPU sends them --- one
/// optimistic op each, on the frontend's own peer --- then a
/// round-tripped `(` as the positive control (a pair char round-trips
/// and its closer is a daemon-peer step), then the bound undo three
/// times, forwarded as the GPU forwards every command chord. The first
/// two undos peel `)` and `(`, proving the key reaches the daemon's
/// history; the third finds nothing: the daemon classified each of the
/// five as `buffer.self-insert` and its history holds none of them. A
/// TUI user's `hello C-/` empties the buffer
/// (`undo_amalgamation_acceptance::typing_a_word_undoes_as_one_step`).
#[test]
fn review_gpu_route_plain_typing_is_beyond_the_daemon_undo_and_so_beyond_e6_4() {
    let daemon = TestDaemon::spawn();
    let mut source = attach_replica(&daemon);
    let mut observer = attach_replica(&daemon);

    for (i, ch) in "hello".chars().enumerate() {
        send_optimistic_op(&mut source, |r| {
            r.insert(i, &ch.to_string()).expect("insert");
        });
    }
    pump_until(&mut observer, Duration::from_secs(5), "observer hello", |o| {
        o.text == "hello"
    });
    // Move the daemon's cursor for `source` to the end, as the GPU's
    // own CursorByte round-trips would: End is a command chord.
    send_key(&mut source, Key::End, Modifiers::NONE);
    send_key(&mut source, Key::Char('('), Modifiers::NONE);
    pump_until(&mut source, Duration::from_secs(5), "source hello()", |o| {
        o.text == "hello()"
    });
    pump_until(&mut observer, Duration::from_secs(5), "observer hello()", |o| {
        o.text == "hello()"
    });

    send_key(&mut source, Key::Char('/'), Modifiers::CTRL);
    pump_until(&mut source, Duration::from_secs(5), "source hello(", |o| {
        o.text == "hello("
    });
    send_key(&mut source, Key::Char('/'), Modifiers::CTRL);
    pump_until(&mut source, Duration::from_secs(5), "source hello", |o| {
        o.text == "hello"
    });
    pump_until(&mut observer, Duration::from_secs(5), "observer hello", |o| {
        o.text == "hello"
    });

    // The third undo: the word is source-peer history, beyond reach.
    send_key(&mut source, Key::Char('/'), Modifiers::CTRL);
    assert_text_stays(&mut observer, "hello", Duration::from_millis(800));
    assert_text_stays(&mut source, "hello", Duration::from_millis(200));
}
