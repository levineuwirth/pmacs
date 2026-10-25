//! E10 review 1: default-on desktop reopening must not disguise a
//! retained token after deliberate departure. A competing desktop makes
//! the two outcomes distinct. The second probe exposes #395's existing
//! message-ordering assumption through a daemon acknowledgment barrier.
#![cfg(feature = "crdt")]

mod common;

use std::path::PathBuf;
use std::time::Duration;

use pmacs_protocol::BufferId;
use pmacs_protocol::message::{InstanceMessage, Key, Modifiers, PROTOCOL_VERSION};

use common::daemon::TestDaemon;
use common::wire::WireSession;

/// The daemon's init: no language servers, a teardown log, two files in
/// its home and a command opening each in the acting window.
const INIT: &str = "\
pmacs.lsp.config = {}
local home = os.getenv('HOME')
pmacs.hook.add('frontend.detached', function(fid)
  local f = assert(io.open(home .. '/detached.log', 'a'))
  f:write(tostring(fid), '\\n')
  f:close()
end)
local function put(name, body)
  local f = assert(io.open(home .. '/' .. name, 'w')); f:write(body); f:close()
end
put('note.txt', 'one\\ntwo\\nthree\\n')
put('other.txt', 'alpha\\nbeta\\n')
pmacs.command.define { name = 'e10.note', description = 'open the note',
  fn = function() pmacs.buffer.find_or_open(home .. '/note.txt') end }
pmacs.command.define { name = 'e10.other', description = 'open the other file',
  fn = function() pmacs.buffer.find_or_open(home .. '/other.txt') end }
";

fn daemon() -> TestDaemon {
    TestDaemon::spawn_with_env_and_init(&[("PMACS_ATTACH_DEBUG", "1")], INIT)
}

/// The daemon's home, which is its socket's directory.
fn home(daemon: &TestDaemon) -> PathBuf {
    daemon
        .socket_path()
        .parent()
        .expect("socket has a parent")
        .to_path_buf()
}

/// The ids the teardown hook has logged, in order.
fn detached(daemon: &TestDaemon) -> Vec<String> {
    std::fs::read_to_string(home(daemon).join("detached.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Wait until the daemon has torn `session` down (its hook ran).
fn wait_for_detach(daemon: &TestDaemon, session: &WireSession) {
    let fid = session.fid.0.to_string();
    common::ready::expect_true(
        "the teardown hook",
        Duration::from_secs(10),
        || detached(daemon).contains(&fid),
        || format!("{:?}", detached(daemon)),
    );
}

/// A semantic window attached the way `pmacs-gpu` attaches with no file,
/// looking at the buffer the daemon put it on.
fn semantic(daemon: &TestDaemon) -> (WireSession, BufferId) {
    let mut window = WireSession::semantic(daemon, PROTOCOL_VERSION, None, None);
    let (buffer, _) = window.where_am_i();
    window.declare_viewport(buffer);
    (window, buffer)
}

/// Run `command` (one of the init's two) in `window`, which is on
/// `current`, and return the buffer it reached.
fn open(window: &mut WireSession, current: BufferId, command: &str) -> BufferId {
    window.run_command(command);
    let reached = window.wait("the window to reach the file", |m| match m {
        InstanceMessage::CursorByte { buffer_id, .. } if *buffer_id != current => Some(*buffer_id),
        _ => None,
    });
    window.declare_viewport(reached);
    reached
}

/// `C-n`: one line down, which is four bytes from the start of the note
/// ("one\n") and six of the other file ("alpha\n").
fn next_line(window: &mut WireSession) {
    window.key(Key::Char('n'), Modifiers::CTRL);
}

/// Distinct desktops make token resumption observable: A left the note,
/// B subsequently saved the other file. Reusing A's discarded token must
/// restore B, while a lost socket must resume A. Reopening stays at default.
#[test]
fn e10_review1_departure_and_loss_choose_different_views_at_default() {
    for departure in ["detach", "quit-with-peer", "last-quit-n", "loss"] {
        let daemon = daemon();
        let (mut a, scratch) = semantic(&daemon);
        let note = open(&mut a, scratch, "e10.note");
        next_line(&mut a);
        a.cursor_at(note, 4);
        let token = a.token.expect("A's token");
        let peer = if departure == "quit-with-peer" {
            Some(semantic(&daemon).0)
        } else {
            None
        };
        match departure {
            "detach" => a.depart(),
            "loss" => a.lose(),
            _ => {
                a.key(Key::Char('x'), Modifiers::CTRL);
                a.key(Key::Char('c'), Modifiers::CTRL);
                if departure == "last-quit-n" {
                    a.wait("the last-window question", |m| match m {
                        InstanceMessage::MinibufferPromptRows {
                            prompt: Some(p), ..
                        } if p.contains("stop the daemon") => Some(()),
                        _ => None,
                    });
                    a.key(Key::Char('n'), Modifiers::NONE);
                }
                a.wait("the quitter's goodbye", |m| {
                    matches!(
                        m,
                        InstanceMessage::Goodbye(pmacs_protocol::message::GoodbyeReason::Detached)
                    )
                    .then_some(())
                });
            }
        }
        wait_for_detach(&daemon, &a);
        // Leave a peer attached, if there is one, until all assertions are
        // done, so its later scratch departure cannot overwrite B's file.
        let (mut b, on) = semantic(&daemon);
        assert_eq!(on, note);
        let other = open(&mut b, on, "e10.other");
        next_line(&mut b);
        b.cursor_at(other, 6);
        b.depart();
        wait_for_detach(&daemon, &b);
        let mut back = WireSession::semantic(&daemon, PROTOCOL_VERSION, None, Some(token));
        let expected = if departure == "loss" {
            (note, 4)
        } else {
            (other, 6)
        };
        assert_eq!(back.bootstrap_snapshots(), vec![expected.0], "{departure}");
        assert_eq!(back.where_am_i(), expected, "{departure}");
        eprintln!("{departure}: A={note:?}/4; B={other:?}/6; return={expected:?}");
        drop(peer);
    }
}

/// #395's helper waits for `CursorByte` and discards preceding messages.
/// Hold M-x until the daemon acknowledges non-idle dispatch, then capture
/// the ordering the helper loses. This is a witness to the race, not its fix.
#[test]
fn e10_review1_snapshot_can_precede_the_cursor_the_e9_helper_waits_for() {
    let daemon =
        TestDaemon::spawn_with_env_and_init(&[], &format!("{INIT}{}", common::daemon::NO_REOPEN));
    let (mut first, scratch) = semantic(&daemon);
    let note = open(&mut first, scratch, "e10.note");
    first.cursor_at(note, 0);
    let (mut reader, on) = semantic(&daemon);
    assert_eq!(on, scratch);
    reader.key(Key::Char('x'), Modifiers::ALT);
    reader.wait("M-x dispatch is non-idle", |m| {
        matches!(m, InstanceMessage::DispatchIdle { idle: false }).then_some(())
    });
    reader.type_str("e10.note");
    reader.key(Key::Enter, Modifiers::NONE);
    let mut snapshots = Vec::new();
    reader.wait("the cursor open_note waits for", |m| {
        match m {
            InstanceMessage::BufferSnapshot { buffer_id, .. } => snapshots.push(*buffer_id),
            InstanceMessage::CursorByte { buffer_id, .. } if *buffer_id != scratch => {
                return Some(());
            }
            _ => {}
        }
        None
    });
    assert!(
        snapshots.contains(&note),
        "the schedule must expose a snapshot discarded by open_note: {snapshots:?}"
    );
    eprintln!("snapshot {note:?} preceded the cursor; open_note discards it");
}
