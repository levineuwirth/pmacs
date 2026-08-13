//! GUI arc Stage 1a — the `TextInput` wire gate, against a real daemon.
//!
//! Separate from `gui_stage1a_acceptance` because these rows need a
//! live daemon and a negotiated session; that suite is in-process.
//!
//! **The claim under test is a REFUSAL**, which is the hardest kind to
//! witness honestly: "nothing happened" is also what a broken test,
//! a dead daemon or a dropped connection look like. Every row here
//! therefore pairs the refusal with a positive control on the same
//! session — something that *does* take effect — so silence can only
//! mean the gate fired.

#![cfg(feature = "crdt")]

use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use pmacs_protocol::{
    ADVERTISED_PROTOCOL_VERSION, AttachRequest, CellSize, FrontendCapabilities, FrontendEvent,
    Hello, InstanceMessage, Key, KeyEvent, Modifiers, PROTOCOL_VERSION, SessionBootstrapRequest,
    TEXT_INPUT_MIN_VERSION, read_message, write_message,
};

#[path = "common/mod.rs"]
mod common;

fn semantic_caps() -> FrontendCapabilities {
    FrontendCapabilities {
        synchronized_output: false,
        unicode_smp: true,
        true_color: true,
        mouse: false,
        bracketed_paste: false,
        terminal_kind: Some("stage1a".into()),
        multi_frontend: true,
        crdt_replica: true,
        semantic_render: true,
    }
}

/// Attach a semantic session that counter-offers `offer`.
fn attach_semantic(
    daemon: &common::daemon::TestDaemon,
    offer: u32,
) -> (UnixStream, pmacs_protocol::FrontendId) {
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set read timeout");
    let hello: Hello = read_message(&mut stream).expect("read daemon Hello");
    assert_eq!(
        hello.protocol_version, ADVERTISED_PROTOCOL_VERSION,
        "the server-first Hello stays at the compatibility baseline"
    );
    let fid = hello.assigned_frontend_id;
    write_message(
        &mut stream,
        &AttachRequest {
            protocol_version: offer,
            frontend_capabilities: semantic_caps(),
            initial_size: CellSize::new(24, 80),
        },
    )
    .expect("write AttachRequest");
    write_message(&mut stream, &SessionBootstrapRequest::default()).expect("write bootstrap");
    (stream, fid)
}

fn pump<T>(
    stream: &mut UnixStream,
    what: &str,
    mut want: impl FnMut(&InstanceMessage) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        match read_message::<InstanceMessage>(stream) {
            Ok(msg) => {
                if let Some(found) = want(&msg) {
                    return found;
                }
            }
            Err(error) => panic!("{what}: read stopped: {error}"),
        }
    }
    panic!("timed out waiting for {what}");
}

fn send_text_input(stream: &mut UnixStream, fid: pmacs_protocol::FrontendId, text: &str) {
    write_message(
        stream,
        &FrontendEvent::TextInput {
            frontend_id: fid,
            text: text.to_owned(),
        },
    )
    .expect("write TextInput");
}

fn send_key(stream: &mut UnixStream, fid: pmacs_protocol::FrontendId, key: Key) {
    write_message(
        stream,
        &FrontendEvent::Key(KeyEvent {
            frontend_id: fid,
            key,
            mods: Modifiers::NONE,
            timestamp_ns: 0,
        }),
    )
    .expect("write key");
}

/// **The discriminating witness for the inbound gate.** A session that
/// negotiated v23 can still ENCODE `TextInput` — it is built from this
/// same crate — so the daemon must refuse it on the authenticated
/// session's negotiated version rather than trusting the producer to
/// withhold.
///
/// The positive control is what makes the refusal legible: the SAME
/// session then sends an ordinary `Key`, and that must take effect. So
/// the session is alive, the stream is synchronized and the daemon is
/// listening — silence about the `TextInput` is the gate, not the
/// plumbing.
#[test]
fn a_v23_session_cannot_drive_an_edit_through_text_input() {
    assert_eq!(
        TEXT_INPUT_MIN_VERSION, 24,
        "this row is written against the v24 floor"
    );
    let daemon = common::daemon::TestDaemon::spawn();
    let (mut stream, fid) = attach_semantic(&daemon, TEXT_INPUT_MIN_VERSION - 1);

    let buffer_id = pump(&mut stream, "first BufferSnapshot", |msg| match msg {
        InstanceMessage::BufferSnapshot { buffer_id, .. } => Some(*buffer_id),
        _ => None,
    });

    // Refused: encoded by a peer that never declared v24.
    send_text_input(&mut stream, fid, "REFUSED");

    // The positive control, on the same session and after it.
    send_key(&mut stream, fid, Key::Char('k'));

    // The first edit that reaches this session must be the CONTROL's,
    // never the refused text. Ordering carries the proof: the daemon
    // processes a session's events in order, so the control's edit
    // arriving with no preceding `REFUSED` edit means the TextInput was
    // dropped rather than merely slow.
    let op = pump(&mut stream, "the control's edit", |msg| match msg {
        InstanceMessage::CrdtOp {
            buffer_id: b, op, ..
        } if *b == buffer_id => Some(op.bytes.clone()),
        _ => None,
    });
    let text = String::from_utf8_lossy(&op).into_owned();
    assert!(
        !text.contains("REFUSED"),
        "a v23 session must not be able to insert through TextInput; got {text:?}"
    );
}

/// The complement, so the row above cannot pass because `TextInput` is
/// broken outright: the SAME traffic on a v24 session **does** edit.
#[test]
fn a_v24_session_can_drive_an_edit_through_text_input() {
    let daemon = common::daemon::TestDaemon::spawn();
    let (mut stream, fid) = attach_semantic(&daemon, PROTOCOL_VERSION);

    let buffer_id = pump(&mut stream, "first BufferSnapshot", |msg| match msg {
        InstanceMessage::BufferSnapshot { buffer_id, .. } => Some(*buffer_id),
        _ => None,
    });

    send_text_input(&mut stream, fid, "ACCEPTED");

    let op = pump(&mut stream, "the TextInput edit", |msg| match msg {
        InstanceMessage::CrdtOp {
            buffer_id: b, op, ..
        } if *b == buffer_id => Some(op.bytes.clone()),
        _ => None,
    });
    let text = String::from_utf8_lossy(&op).into_owned();
    assert!(
        text.contains("ACCEPTED"),
        "a v24 session must be able to insert through TextInput; got {text:?}"
    );
}
