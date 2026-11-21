//! E10z.1 --- the answer: `InstanceMessage::InputHandled`, by which a
//! daemon says how many of a session's events its dispatcher has taken
//! up, through the production handshake and dispatch of a real daemon.
//!
//! The wire rows speak as the attached TUI does --- `Hello`,
//! `AttachRequest` with `crdt_replica` and no `semantic_render`, then
//! `FrontendEvent`s into the daemon's own dispatcher --- and read what
//! the daemon writes to the session, in order, the outermost seam a
//! frontend has. A reader thread feeds a channel, so a time-boxed drain
//! never cuts a frame in half. The last row is `pmacs-gpu` itself,
//! through `--headless-probe`: its handshake, outbox, reader and `State`.
//!
//! What a frontend does with the count is E10z.2's; nothing here reads a
//! frontend's gate.
#![cfg(feature = "crdt")]

mod common;

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pmacs_protocol::cell::CellSize;
use pmacs_protocol::message::{
    ADVERTISED_PROTOCOL_VERSION, AttachRequest, FrontendCapabilities, FrontendEvent, Hello,
    INPUT_HANDLED_MIN_VERSION, InstanceMessage, Key, KeyEvent, Modifiers, PROTOCOL_VERSION,
};
use pmacs_protocol::transport::{read_message, write_message};
use pmacs_protocol::{BufferId, FrontendId};

use common::daemon::{NO_REOPEN, TestDaemon, build_default_caps, multi_frontend_caps};
use common::gpu_probe::{fact, gpu_probe_binary, probe_facts};

/// One attached grid session: its writer, a channel of everything the
/// daemon wrote to it, its id, and how many events it has sent, which
/// is the count a frontend holds against the daemon's.
struct Tui {
    stream: UnixStream,
    rx: mpsc::Receiver<InstanceMessage>,
    fid: FrontendId,
    sent: u64,
}

/// A message's variant name, for a failure's list of what came.
fn name(msg: &InstanceMessage) -> String {
    match msg {
        InstanceMessage::InputHandled { events } => format!("InputHandled({events})"),
        InstanceMessage::DispatchIdle { idle } => format!("DispatchIdle({idle})"),
        InstanceMessage::CursorByte { byte_pos, .. } => format!("CursorByte({byte_pos})"),
        other => format!("{other:?}")
            .split(['(', ' ', '{'])
            .next()
            .unwrap_or("")
            .to_owned(),
    }
}

impl Tui {
    /// The handshake as `pmacs --attach` performs it, at `offer` with
    /// `caps`; reads nothing past it, the reader thread does.
    fn attach(daemon: &TestDaemon, offer: u32, caps: FrontendCapabilities) -> Self {
        let mut stream = daemon.connect();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("read timeout");
        let hello: Hello = read_message(&mut stream).expect("read Hello");
        assert_eq!(hello.protocol_version, ADVERTISED_PROTOCOL_VERSION);
        let fid = hello.assigned_frontend_id;
        write_message(
            &mut stream,
            &AttachRequest {
                protocol_version: offer,
                frontend_capabilities: caps,
                initial_size: CellSize::new(24, 80),
            },
        )
        .expect("AttachRequest");
        let mut reader = stream.try_clone().expect("clone stream");
        reader.set_read_timeout(None).expect("no read timeout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(msg) = read_message::<InstanceMessage>(&mut reader) {
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        Self {
            stream,
            rx,
            fid,
            sent: 0,
        }
    }

    /// A replica at `offer`, read up to its first `DispatchIdle { idle:
    /// true }` and the `CursorByte` after it: the state a TUI types from.
    fn replica(daemon: &TestDaemon, offer: u32) -> (Self, BufferId, u64) {
        let mut tui = Self::attach(daemon, offer, multi_frontend_caps());
        tui.read_until("the first DispatchIdle { idle: true }", |m| {
            matches!(m, InstanceMessage::DispatchIdle { idle: true }).then_some(())
        });
        let (buffer, byte) = tui.cursor("the first CursorByte");
        (tui, buffer, byte)
    }

    fn send(&mut self, event: &FrontendEvent) {
        write_message(&mut self.stream, event).expect("send an event");
        self.sent += 1;
    }

    fn key(&mut self, key: Key, mods: Modifiers) {
        let event = FrontendEvent::Key(KeyEvent {
            frontend_id: self.fid,
            key,
            mods,
            timestamp_ns: 0,
        });
        self.send(&event);
    }

    /// Everything the daemon writes up to and including the first
    /// message `pick` accepts, with what it returned; ten seconds, and
    /// the timeout names every message that came meanwhile.
    fn read_until<T>(
        &mut self,
        what: &str,
        mut pick: impl FnMut(&InstanceMessage) -> Option<T>,
    ) -> (Vec<InstanceMessage>, T) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut read = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting for {what}; the daemon wrote {:?}",
                read.iter().map(name).collect::<Vec<_>>()
            );
            match self
                .rx
                .recv_timeout(remaining.min(Duration::from_millis(200)))
            {
                Ok(msg) => {
                    let found = pick(&msg);
                    read.push(msg);
                    if let Some(found) = found {
                        return (read, found);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("the daemon closed the session while waiting for {what}");
                }
            }
        }
    }

    /// The buffer and byte of the next `CursorByte`.
    fn cursor(&mut self, what: &str) -> (BufferId, u64) {
        self.read_until(what, |m| match m {
            InstanceMessage::CursorByte {
                buffer_id,
                byte_pos,
            } => Some((*buffer_id, *byte_pos)),
            _ => None,
        })
        .1
    }

    /// Everything the daemon writes in `window`: the window before a
    /// negative assertion, named with its length where it is used.
    fn drain_for(&mut self, window: Duration) -> Vec<InstanceMessage> {
        let deadline = Instant::now() + window;
        let mut read = Vec::new();
        while let Ok(msg) = self
            .rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            read.push(msg);
            if Instant::now() >= deadline {
                break;
            }
        }
        read
    }

    /// Read up to the answer that covers everything sent so far.
    fn read_to_answer(&mut self) -> Vec<InstanceMessage> {
        let sent = self.sent;
        self.read_until(&format!("InputHandled {{ events: {sent} }}"), |m| {
            matches!(m, InstanceMessage::InputHandled { events } if *events == sent).then_some(())
        })
        .0
    }
}

/// The counts of every `InputHandled` in `read`, in order.
fn answers(read: &[InstanceMessage]) -> Vec<u64> {
    read.iter()
        .filter_map(|m| match m {
            InstanceMessage::InputHandled { events } => Some(*events),
            _ => None,
        })
        .collect()
}

fn position(read: &[InstanceMessage], want: impl Fn(&InstanceMessage) -> bool) -> Option<usize> {
    read.iter().position(want)
}

/// E10z.1 --- a key that opens a prefix is answered after the flip it
/// caused, so a frontend that has read the answer has read the flip; and
/// the key that closes it likewise. This is what the row's "after the
/// tick's `DispatchIdle`" is for: an answer ahead of the flip would tell
/// a frontend its `C-x` was handled while it still believed the daemon
/// idle, and the next key would go as text (#470).
#[test]
fn a_key_that_flips_the_dispatcher_is_answered_after_the_flip() {
    let daemon = TestDaemon::spawn_with_config(NO_REOPEN);
    let (mut tui, _, _) = Tui::replica(&daemon, PROTOCOL_VERSION);

    tui.key(Key::Char('x'), Modifiers::CTRL);
    let read = tui.read_to_answer();
    let names: Vec<String> = read.iter().map(name).collect();
    let flip = position(&read, |m| {
        matches!(m, InstanceMessage::DispatchIdle { idle: false })
    })
    .unwrap_or_else(|| panic!("`C-x` must flip the dispatcher before its answer: {names:?}"));
    let answer = position(&read, |m| matches!(m, InstanceMessage::InputHandled { .. }))
        .expect("read_to_answer ends on one");
    assert!(
        flip < answer,
        "no answer may come before the flip its key caused: {names:?}"
    );

    tui.key(Key::Char('g'), Modifiers::CTRL);
    let read = tui.read_to_answer();
    let names: Vec<String> = read.iter().map(name).collect();
    let flip = position(&read, |m| {
        matches!(m, InstanceMessage::DispatchIdle { idle: true })
    })
    .unwrap_or_else(|| panic!("`C-g` must flip the dispatcher back before its answer: {names:?}"));
    let answer = position(&read, |m| matches!(m, InstanceMessage::InputHandled { .. }))
        .expect("read_to_answer ends on one");
    assert!(
        flip < answer,
        "no answer may come before the flip its key caused: {names:?}"
    );
}

/// E10z.1 --- after a key that flips nothing, the answer comes before
/// the first `CursorByte` that reflects the key: it goes with the early
/// confirmation (`confirm_cursor_early`) and does not wait for the tick.
/// A `CursorByte` at the old byte may still stand between the key and its
/// answer, written before the daemon read the key; none at the new byte
/// does. Three characters are typed first so that `C-b` has somewhere to
/// go.
#[test]
fn the_answer_comes_before_the_first_cursor_its_key_moved() {
    let daemon = TestDaemon::spawn_with_config(NO_REOPEN);
    let (mut tui, buffer, start) = Tui::replica(&daemon, PROTOCOL_VERSION);
    for ch in ['a', 'b', 'c'] {
        tui.key(Key::Char(ch), Modifiers::NONE);
    }
    // The answer first: it is written ahead of the cursor it goes with,
    // so reading to the cursor first would read past it.
    tui.read_to_answer();
    tui.read_until("the caret after three characters", |m| {
        matches!(m, InstanceMessage::CursorByte { buffer_id, byte_pos }
            if *buffer_id == buffer && *byte_pos == start + 3)
        .then_some(())
    });
    // The ticks' cursors at the old byte, so the next read starts quiet.
    let quiet = tui.drain_for(Duration::from_millis(150));
    assert!(
        answers(&quiet).is_empty(),
        "premise: nothing is owed an answer before `C-b`: {:?}",
        quiet.iter().map(name).collect::<Vec<_>>()
    );

    tui.key(Key::Char('b'), Modifiers::CTRL);
    let sent = tui.sent;
    let (read, ()) = tui.read_until("the caret one character back", |m| {
        matches!(m, InstanceMessage::CursorByte { buffer_id, byte_pos }
            if *buffer_id == buffer && *byte_pos == start + 2)
        .then_some(())
    });
    assert_eq!(
        answers(&read),
        vec![sent],
        "the answer to `C-b` is written before the first cursor it moved, once: {:?}",
        read.iter().map(name).collect::<Vec<_>>()
    );
}

/// E10z fix round 3 (review 3's Low 1) --- the premise the terminal pump's
/// caret rests on since fix round 2, which no row asserted: every `Cursor`
/// and every `CursorByte` the daemon writes to a session is its caret after
/// exactly the events the last `InputHandled` before it counts. The pump
/// moves each frame's `Cursor` by the ops that count does not cover; a count
/// written behind its frame would move a caret the frame already shows.
///
/// Both of the daemon's writers are met, which is the row's premise. Forty
/// characters go as keys in one burst at the end of an empty line, each
/// moving the caret a column and a byte: every key is answered by the early
/// confirmation (`confirm_cursor_early`), its count straight ahead of its
/// `CursorByte`, with the ticks' frames among them. Then a paste of two
/// characters, alone: a paste gets no early confirmation, so its count is
/// the tick's, written after any `DispatchIdle` and ahead of the frame that
/// shows the paste. With the tick's count moved behind its frame, a defect by
/// this premise, that frame's `Cursor` is two columns past what the count
/// read before it allows, and the row fails on it.
#[test]
fn a_cursor_is_the_caret_after_exactly_the_count_read_before_it() {
    let daemon = TestDaemon::spawn_with_config(NO_REOPEN);
    let mut tui = Tui::attach(&daemon, PROTOCOL_VERSION, multi_frontend_caps());
    // The first tick: the frame's `Cursor`, then its `CursorByte`, nothing sent yet.
    let (first, (buffer, byte0)) = tui.read_until("the first CursorByte", |m| match m {
        InstanceMessage::CursorByte {
            buffer_id,
            byte_pos,
        } => Some((*buffer_id, *byte_pos)),
        _ => None,
    });
    let (row0, col0) = first
        .iter()
        .rev()
        .find_map(|m| match m {
            InstanceMessage::Cursor(Some(c)) if c.visible => Some((c.coord.row, c.coord.col)),
            _ => None,
        })
        .expect("premise: the first frame's Cursor comes before the first CursorByte");
    assert!(
        first
            .iter()
            .any(|m| matches!(m, InstanceMessage::DispatchIdle { idle: true })),
        "premise: the dispatcher is idle before the burst: {:?}",
        first.iter().map(name).collect::<Vec<_>>()
    );

    for i in 0..40u8 {
        tui.key(Key::Char(char::from(b'a' + i % 26)), Modifiers::NONE);
    }
    let mut read = tui.read_to_answer();
    // The burst's last cursors and a tick or two with nothing owed, so that the
    // paste's answer is the tick's alone.
    read.extend(tui.drain_for(Duration::from_millis(150)));
    let fid = tui.fid;
    tui.send(&FrontendEvent::Paste {
        frontend_id: fid,
        data: b"PQ".to_vec(),
    });
    read.extend(tui.read_to_answer());
    read.extend(tui.drain_for(Duration::from_millis(300)));

    // The columns, and bytes, the caret has moved after `n` events.
    let moved = |n: u64| n.min(40) + if n > 40 { 2 } else { 0 };
    let names = || read.iter().map(name).collect::<Vec<_>>();
    let mut count = 0u64;
    let (mut cursors, mut bytes, mut early, mut by_the_tick) = (0, 0, 0, 0);
    for (at, m) in read.iter().enumerate() {
        match m {
            InstanceMessage::InputHandled { events } => {
                assert!(
                    *events > count,
                    "a count never goes back or repeats: {events} after {count}"
                );
                count = *events;
                match read.get(at + 1) {
                    Some(InstanceMessage::CursorByte { .. }) => early += 1,
                    Some(InstanceMessage::CellDelta { .. }) => by_the_tick += 1,
                    _ => {}
                }
            }
            InstanceMessage::Cursor(Some(c)) if c.visible => {
                cursors += 1;
                assert_eq!(
                    (c.coord.row, u64::from(c.coord.col)),
                    (row0, u64::from(col0) + moved(count)),
                    "a frame's Cursor read with the count at {count} is the caret after {count} \
                     events: {:?}",
                    names()
                );
            }
            InstanceMessage::CursorByte {
                buffer_id,
                byte_pos,
            } => {
                bytes += 1;
                assert_eq!(
                    (*buffer_id, *byte_pos),
                    (buffer, byte0 + moved(count)),
                    "a CursorByte read with the count at {count} is the caret after {count} \
                     events: {:?}",
                    names()
                );
            }
            _ => {}
        }
    }
    assert_eq!(
        count, 41,
        "premise: the forty keys and the paste were answered"
    );
    assert!(
        early >= 1 && by_the_tick >= 1 && cursors >= 2 && bytes >= 3,
        "premise: the burst met both writers, a count straight ahead of an early CursorByte \
         ({early}) and a count the tick wrote ahead of its frame ({by_the_tick}), with \
         {cursors} Cursor and {bytes} CursorByte read: {:?}",
        names()
    );
}

/// E10z.1 --- the count is of every event the dispatcher takes up from
/// the session, whatever its kind: a focus change, a resize, a paste, a
/// mouse press and a key each move it by one, and it never passes what
/// the session has sent. (The owner's ruling at C10z.1 is on this row's
/// sentence; under "keys alone" the first four would move nothing.)
#[test]
fn every_event_the_dispatcher_takes_up_is_counted() {
    let daemon = TestDaemon::spawn_with_config(NO_REOPEN);
    let (mut tui, _, _) = Tui::replica(&daemon, PROTOCOL_VERSION);
    let fid = tui.fid;
    let events = [
        FrontendEvent::FocusLost(fid),
        FrontendEvent::FocusGained(fid),
        FrontendEvent::Resize {
            frontend_id: fid,
            size: CellSize::new(30, 100),
        },
        FrontendEvent::Paste {
            frontend_id: fid,
            data: b"pasted".to_vec(),
        },
        FrontendEvent::Mouse(pmacs_protocol::MouseEvent {
            frontend_id: fid,
            kind: pmacs_protocol::MouseKind::Down(pmacs_protocol::MouseButton::Left),
            coord: pmacs_protocol::CellCoord { row: 0, col: 0 },
            mods: Modifiers::NONE,
        }),
        FrontendEvent::Key(KeyEvent {
            frontend_id: fid,
            key: Key::Char('f'),
            mods: Modifiers::CTRL,
            timestamp_ns: 0,
        }),
    ];
    let mut all = Vec::new();
    for event in &events {
        tui.send(event);
        let read = tui.read_to_answer();
        assert!(
            answers(&read).iter().all(|n| *n <= tui.sent),
            "an answer never passes what was sent ({}): {:?}",
            tui.sent,
            answers(&read)
        );
        all.extend(answers(&read));
    }
    assert_eq!(tui.sent, 6, "premise: six events were sent");
    assert_eq!(
        all,
        vec![1, 2, 3, 4, 5, 6],
        "each event, sent after the last was answered, is answered by its own count"
    );
}

/// E10z.1 --- the count is written when it has changed and not again:
/// the ticks that follow an answered key repeat its `CursorByte` and not
/// its answer. 300 ms is about eighteen ticks at the default frame.
#[test]
fn an_answer_is_written_once() {
    let daemon = TestDaemon::spawn_with_config(NO_REOPEN);
    let (mut tui, _, _) = Tui::replica(&daemon, PROTOCOL_VERSION);
    tui.key(Key::Char('f'), Modifiers::CTRL);
    let read = tui.read_to_answer();
    assert_eq!(answers(&read), vec![1]);
    let after = tui.drain_for(Duration::from_millis(300));
    let cursors = after
        .iter()
        .filter(|m| matches!(m, InstanceMessage::CursorByte { .. }))
        .count();
    assert!(
        cursors >= 3,
        "premise: the ticks went on writing to the session for 300 ms ({cursors} cursors)"
    );
    assert!(
        answers(&after).is_empty(),
        "an unchanged count is not written again: {:?}",
        after.iter().map(name).collect::<Vec<_>>()
    );
}

/// E10z.1 --- a session below the version is told nothing: it negotiated
/// a wire without the variant and keeps today's rule. Its keys are still
/// handled, which the two flips show, so the silence is the gate's and
/// not a dead session's. 300 ms follows the second flip.
#[test]
fn a_session_below_the_version_is_told_nothing() {
    assert_eq!(INPUT_HANDLED_MIN_VERSION, 28);
    let daemon = TestDaemon::spawn_with_config(NO_REOPEN);
    let (mut tui, _, _) = Tui::replica(&daemon, INPUT_HANDLED_MIN_VERSION - 1);
    let mut read = Vec::new();
    tui.key(Key::Char('x'), Modifiers::CTRL);
    read.extend(
        tui.read_until("the flip `C-x` causes", |m| {
            matches!(m, InstanceMessage::DispatchIdle { idle: false }).then_some(())
        })
        .0,
    );
    tui.key(Key::Char('g'), Modifiers::CTRL);
    read.extend(
        tui.read_until("the flip `C-g` causes", |m| {
            matches!(m, InstanceMessage::DispatchIdle { idle: true }).then_some(())
        })
        .0,
    );
    read.extend(tui.drain_for(Duration::from_millis(300)));
    assert!(
        answers(&read).is_empty(),
        "a v{} session must not be sent InputHandled: {:?}",
        INPUT_HANDLED_MIN_VERSION - 1,
        read.iter().map(name).collect::<Vec<_>>()
    );
}

/// E10z.1 --- a session that is no replica is told nothing either, at
/// the current version: it has no optimistic path to gate, as it has no
/// `DispatchIdle`. Its key is still handled, which the frame that paints
/// the typed character shows. 300 ms follows that frame.
#[test]
fn a_session_that_is_no_replica_is_told_nothing() {
    let daemon = TestDaemon::spawn_with_config(NO_REOPEN);
    let mut tui = Tui::attach(&daemon, PROTOCOL_VERSION, build_default_caps());
    let mut read = tui
        .read_until("the first frame", |m| {
            matches!(m, InstanceMessage::CellDelta { .. }).then_some(())
        })
        .0;
    read.extend(tui.drain_for(Duration::from_millis(150)));
    tui.key(Key::Char('q'), Modifiers::NONE);
    read.extend(
        tui.read_until("the frame that paints the typed character", |m| {
            matches!(m, InstanceMessage::CellDelta { .. }).then_some(())
        })
        .0,
    );
    read.extend(tui.drain_for(Duration::from_millis(300)));
    assert!(
        answers(&read).is_empty()
            && !read
                .iter()
                .any(|m| matches!(m, InstanceMessage::DispatchIdle { .. })),
        "a session without `crdt_replica` is sent neither signal: {:?}",
        read.iter().map(name).collect::<Vec<_>>()
    );
}

/// Run the popup probe's script against `daemon`; `None` when skipped.
fn run_probe(daemon: &TestDaemon, script: &str, label: &str) -> Option<HashMap<String, String>> {
    let binary = gpu_probe_binary()?;
    let report = daemon
        .socket_path()
        .parent()
        .expect("socket parent")
        .join(label);
    let output = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(daemon.socket_path())
        .arg(&report)
        .env("PMACS_GPU_PROBE_TYPE_TEXT", script)
        .env("PMACS_GPU_PROBE_ACTION", "popup")
        .env("PMACS_GPU_PROBE_DEADLINE_MS", "40000")
        .output()
        .expect("run the headless GPU probe");
    probe_facts(&output, &report, label)
}

/// E10z.1 --- `pmacs-gpu` takes the answer: through its own handshake at
/// the current version, its outbox, its reader and its `State`, the
/// count it holds after two keys covers them, and after a third has
/// grown by at least that one. (It also sends events of its own, a
/// `Viewport` after each snapshot among them, so the count is at least
/// the keys and not exactly them; holding its gate on the count is
/// E10z.2's.)
#[test]
fn the_gpu_frontend_holds_the_count_the_daemon_sends() {
    let daemon = TestDaemon::spawn_with_config(NO_REOPEN);
    let Some(facts) = run_probe(
        &daemon,
        "key:C-f key:C-b answer:two key:C-f answer:three",
        "e10z-gpu-answer.report",
    ) else {
        return;
    };
    assert_eq!(fact(&facts, "ready"), "true");
    assert_eq!(
        fact(&facts, "session_protocol_version"),
        PROTOCOL_VERSION.to_string()
    );
    let two: u64 = fact(&facts, "two.input_handled").parse().expect("a count");
    let three: u64 = fact(&facts, "three.input_handled")
        .parse()
        .expect("a count");
    assert!(
        two >= 2,
        "after two keys the window holds a count that covers them: {two}"
    );
    assert!(three > two, "and a third key moves it: {two} then {three}");
}
