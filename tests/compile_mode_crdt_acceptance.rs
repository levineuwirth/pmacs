// compile_mode_crdt_acceptance.rs --- compile-mode over the wire.

//! Compile-mode two-replica acceptance (docs/compile-mode-framing.md,
//! item 35): a full compile run's generated buffer converges
//! byte-identically on a mirror replica, and a synthetic accepted
//! replica edit to that buffer triggers the immediate recovery
//! marker (the `buffer.after-edit` path fires for accepted `CrdtOp`s)
//! and still converges on both replicas — even though the
//! hook-produced marker may queue before the source edit's
//! rebroadcast (the established causal-reordering seam).
//!
//! All compile-buffer writes are daemon-side Lua bypass edits —
//! ordinary daemon-peer CRDT ops with no optimistic involvement —
//! so convergence here pins the whole streaming pipeline (header,
//! parsed output, exit marker) as replicable state.

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

/// Read until a `BufferSnapshot` for a buffer other than the current
/// one arrives (the compile run creates *compilation* mid-session;
/// the daemon broadcasts a snapshot for the newly-CRDT-backed buffer
/// and via the active-buffer-follow path). Re-seats the replica's
/// mirror on that buffer.
fn adopt_next_buffer(replica: &mut Replica, what: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "timeout adopting the new buffer snapshot for {what}"
        );
        replica
            .stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .ok();
        match pmacs::transport::read_message::<pmacs::protocol::InstanceMessage>(
            &mut replica.stream,
        ) {
            Ok(pmacs::protocol::InstanceMessage::BufferSnapshot {
                buffer_id,
                crdt_snapshot,
            }) if buffer_id != replica.buffer_id => {
                let state = CrdtState::new(replica.fid.0).expect("CrdtState::new");
                state
                    .import_snapshot(&crdt_snapshot)
                    .expect("import new-buffer snapshot");
                replica.state = state;
                replica.buffer_id = buffer_id;
                return;
            }
            Ok(_) | Err(_) => {}
        }
    }
}

/// Pump broadcast messages, importing every `CrdtOp` for the tracked
/// buffer, until `pred(text)` holds.
fn pump_until_text<P: Fn(&str) -> bool>(
    replica: &mut Replica,
    timeout: Duration,
    what: &str,
    pred: P,
) -> String {
    let deadline = std::time::Instant::now() + timeout;
    let mut text = replica.state.materialize_string();
    loop {
        if pred(&text) {
            return text;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "pump timeout waiting for {what}; text={text:?}"
        );
        replica
            .stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .ok();
        match pmacs::transport::read_message::<pmacs::protocol::InstanceMessage>(
            &mut replica.stream,
        ) {
            Ok(pmacs::protocol::InstanceMessage::CrdtOp { buffer_id: b, op })
                if b == replica.buffer_id =>
            {
                let _ = replica.state.import_updates(&op.bytes);
                text = replica.state.materialize_string();
            }
            Ok(_) | Err(_) => {}
        }
    }
}

const DESYNC: &str = "[output desynced by external edit]";

#[test]
fn compile_run_converges_and_replica_edit_triggers_recovery() {
    // Fixture: the compile command lives in a shared tempdir; the
    // init.lua binds a chord that runs it (typing an M-x prompt over
    // the wire would test the minibuffer, not compile-mode).
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("fix.sh");
    std::fs::write(&script, "printf 'x.c:1:1: error: boom\\ndone\\n'\n").unwrap();
    let init = format!(
        r#"
        pmacs.command.define {{
            name = "test.compile",
            description = "compile-mode CRDT fixture trigger",
            fn = function()
                pmacs.compile.run("sh {script}", {{ cwd = "{dir}" }})
            end,
        }}
        pmacs.keymap.bind {{ scope = "global", sequence = "C-c 9", command = "test.compile" }}
        "#,
        script = script.display(),
        dir = dir.path().display(),
    );
    let daemon = TestDaemon::spawn_with_config(&init);
    let mut source = attach_replica(&daemon);
    let mut observer = attach_replica(&daemon);
    let initial = source.buffer_id;

    // Trigger the run from the source replica (round-tripped keys).
    send_key(&mut source, Key::Char('c'), Modifiers::CTRL);
    send_key(&mut source, Key::Char('9'), Modifiers::NONE);

    // Both replicas adopt the freshly-created *compilation* buffer.
    adopt_next_buffer(&mut source, "source");
    adopt_next_buffer(&mut observer, "observer");
    assert_ne!(source.buffer_id, initial, "a new buffer was created");
    assert_eq!(
        source.buffer_id, observer.buffer_id,
        "both replicas mirror the same generated buffer"
    );

    // The full run — header, streamed output, exit marker — reaches
    // both mirrors byte-identically.
    let done = |t: &str| t.contains("[compile exited with code 0]");
    let src_text = pump_until_text(&mut source, Duration::from_secs(15), "source run", done);
    let obs_text = pump_until_text(&mut observer, Duration::from_secs(15), "observer run", done);
    assert_eq!(src_text, obs_text, "byte-identical convergence");
    assert!(
        src_text.contains("x.c:1:1: error: boom"),
        "output replicated"
    );
    assert!(src_text.starts_with("$ sh "), "header replicated");

    // Synthetic accepted replica edit to the generated buffer: the
    // daemon applies it, buffer.after-edit fires, and compile.lua's
    // revision guard appends the recovery marker immediately. The
    // marker (a daemon-peer op) may broadcast before the source
    // edit's own rebroadcast — the causal-reordering seam — and both
    // replicas must still converge.
    send_optimistic_op(&mut source, |r| {
        r.insert(0, "Z").expect("replica edit");
    });
    let recovered = |t: &str| t.contains(DESYNC) && t.starts_with('Z');
    let src_text = pump_until_text(
        &mut source,
        Duration::from_secs(10),
        "source recovery marker",
        recovered,
    );
    let obs_text = pump_until_text(
        &mut observer,
        Duration::from_secs(10),
        "observer recovery marker",
        recovered,
    );
    assert_eq!(
        src_text, obs_text,
        "post-recovery convergence across the reorder seam"
    );
}

#[test]
fn r3f1_unicode_cr_backspace_survive_crdt_replication() {
    // PR #113 round-3 finding 1, CRDT twin: pre-fix the byte-counted
    // overwrite split a 2-byte é mid-codepoint; the byte-native
    // UTF-8 CRDT edit REJECTS that range, the pump callback aborts
    // after events_take, the run never reaches its exit marker, and
    // the process record leaks. Post-fix the whole-codepoint atomic
    // replace applies cleanly and both replicas converge.
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("uni.sh");
    std::fs::write(
        &script,
        "printf '\\303\\251\\rX\\n'\nprintf 'X\\r\\303\\251\\n'\nprintf '\\303\\251\\bX\\n'\n",
    )
    .unwrap();
    let init = format!(
        r#"
        pmacs.command.define {{
            name = "test.compile-unicode",
            description = "round-3 unicode fixture trigger",
            fn = function()
                pmacs.compile.run("sh {script}", {{ cwd = "{dir}" }})
            end,
        }}
        pmacs.keymap.bind {{ scope = "global", sequence = "C-c 8", command = "test.compile-unicode" }}
        "#,
        script = script.display(),
        dir = dir.path().display(),
    );
    let daemon = TestDaemon::spawn_with_config(&init);
    let mut source = attach_replica(&daemon);
    let mut observer = attach_replica(&daemon);

    send_key(&mut source, Key::Char('c'), Modifiers::CTRL);
    send_key(&mut source, Key::Char('8'), Modifiers::NONE);

    adopt_next_buffer(&mut source, "source");
    adopt_next_buffer(&mut observer, "observer");

    let done = |t: &str| t.contains("[compile exited with code 0]");
    let src_text = pump_until_text(&mut source, Duration::from_secs(15), "source run", done);
    let obs_text = pump_until_text(&mut observer, Duration::from_secs(15), "observer run", done);
    assert_eq!(src_text, obs_text, "byte-identical convergence");
    assert!(
        src_text.contains("\nX\n\u{e9}\nX\n"),
        "whole-codepoint overwrites replicate as valid UTF-8; got:\n{src_text:?}"
    );
}
