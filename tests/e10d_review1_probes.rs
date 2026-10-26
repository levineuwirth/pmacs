//! E10d review 1 --- behavioral probes against E10d's rows, in states a
//! user reaches that the phase's witnesses did not choose, committed as
//! witnesses at fix round 1.
//!
//! P1 and P1a (Medium 1): a message the transport refuses before writing
//! a byte (`TransportError::FrameTooLarge`, here a clipboard past the
//! 16 MiB frame) reaches `write_to_session`, which ended the session's
//! output and, at `WRITE_FAILURE_DRAIN`, the session, though its socket
//! is whole. At `73bc1e6` that route discarded the error and the session
//! went on. Since fix round 1 only a failure of the socket
//! (`TransportError::Io`) begins the drain (the owner's ruling of
//! 2026-10-26); a refusal is logged with the route's line and the session
//! kept.
//!
//! The review's P2 (Low 1) is committed beside E10d.1's control, in
//! `tests/e9_session_acceptance.rs`'s drain rows, as the ruling put it.
//!
//! Fix round 1 adds one row for each other route a message past the
//! frame reaches: the tick's frame (a status message), which drained on
//! a refusal at `73bc1e6` too and now keeps the session, and the CRDT op
//! (a Lua insert), whose refusal `73bc1e6` dropped silently and which is
//! now logged; the replica's mirror goes without that op either way.
//!
//! Every daemon writes a `CursorByte` to every replica on every pass, so
//! each row waits for a position its window has not had before, never
//! for one it may already be sending.
//!
//! Linux alone, as E9's drain rows are (a write to a peer that shut its
//! read side succeeds on macOS, E8d.4's finding), and `crdt`.
#![cfg(all(feature = "crdt", target_os = "linux"))]

mod common;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use pmacs_protocol::cell::CellSize;
use pmacs_protocol::message::{InstanceMessage, Key, Modifiers, PROTOCOL_VERSION};
use pmacs_protocol::{BufferId, FrontendId};

use common::daemon::{NO_REOPEN, TestDaemon};
use common::wire::WireSession;

/// The drain's bound, `pmacs::daemon::WRITE_FAILURE_DRAIN`.
const DRAIN: Duration = Duration::from_secs(2);

/// A `frontend.detached` hook logging the departed id into the daemon's
/// home (its `HOME` is the socket's directory), as E9's suite does.
const DETACH_LOG: &str = "\
pmacs.hook.add('frontend.detached', function(fid)
  local f = assert(io.open(os.getenv('HOME') .. '/detached.log', 'a'))
  f:write(tostring(fid), '\\n')
  f:close()
end)
";

fn home(daemon: &TestDaemon) -> PathBuf {
    daemon
        .socket_path()
        .parent()
        .expect("the socket's directory")
        .to_path_buf()
}

fn daemon_log(daemon: &TestDaemon) -> String {
    std::fs::read_to_string(format!("{}.stderr.log", daemon.socket_path().display()))
        .unwrap_or_default()
}

fn detached(daemon: &TestDaemon) -> Vec<String> {
    std::fs::read_to_string(home(daemon).join("detached.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Whether the one path has logged that it ended `fid`'s output.
fn drain_announced(daemon: &TestDaemon, fid: FrontendId) -> bool {
    daemon_log(daemon).contains(&format!("{fid:?}'s write failed; nothing more is written"))
}

/// A daemon with `init` after the shared lines (no desktop reopening, no
/// language servers, the detach log), its handshake breadcrumbs on.
fn daemon_with(init: &str) -> TestDaemon {
    let init = format!("{NO_REOPEN}pmacs.lsp.config = {{}}\n{DETACH_LOG}{init}");
    TestDaemon::spawn_with_env_and_init(&[("PMACS_ATTACH_DEBUG", "1")], &init)
}

/// Whether the daemon writes `session` `times` `CursorByte`s at `buffer`
/// and `byte` within ten seconds; `false` on the deadline or when the
/// daemon closed the session, so the row can say why with the daemon's
/// log. Twice is a pass's: the first may be `confirm_cursor_early`'s,
/// written while the key's event is handled, but a second comes only
/// from a pass, and every pass comes after the top of its loop, where a
/// copy's clipboard is written.
fn served_at(session: &mut WireSession, buffer: BufferId, byte: u64, times: usize) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = 0;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        match session.rx.recv_timeout(remaining) {
            Ok(InstanceMessage::CursorByte {
                buffer_id,
                byte_pos,
            }) if buffer_id == buffer && byte_pos == byte => {
                seen += 1;
                if seen == times {
                    return true;
                }
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                return false;
            }
        }
    }
}

/// P1 (E10d.2, Medium 1) --- the attached TUI copies a region past the
/// frame: `C-x h M-w` over a 17 MiB log, as a user copies a large log or
/// a minified file whole, queues an OS-clipboard publish of 17 MiB to the
/// frontend that copied, which `write_message` refuses as
/// `FrameTooLarge` before writing a byte; the socket is whole. At
/// `fd38a8a` the one path reads that refusal as a failed write: the
/// TUI's output ends at the next loop and its session is torn down at
/// the drain's deadline, so a copy disconnects the attached TUI. At
/// `73bc1e6` the clipboard's error was discarded (no OS clipboard; the
/// session goes on), which is what this row asserts: the keys after the
/// copy are answered, no drain is announced, and the session is the
/// TUI's past the deadline.
#[test]
fn e10d_review1_p1_the_attached_tui_copying_past_the_frame_stays_attached() {
    let dir = tempfile::TempDir::new().expect("tempdir for the big log");
    let big = dir.path().join("big.log");
    let line = format!("{}\n", "x".repeat(79));
    std::fs::write(&big, line.repeat((17 << 20) / line.len() + 1)).expect("write the big log");
    let daemon = daemon_with(&format!(
        "pmacs.command.define {{ name = 'r1.open-big', description = 'open the big log',\n\
           fn = function() pmacs.buffer.find_or_open({big:?}) end }}\n",
        big = big.to_str().expect("utf-8 path"),
    ));
    let mut tui = WireSession::grid(&daemon, CellSize::new(24, 80));
    let fid = tui.fid;
    let id = fid.0.to_string();
    let (scratch, _) = tui.where_am_i();
    tui.run_command("r1.open-big");
    let log = tui.wait("the TUI's window on the big log", |m| match m {
        InstanceMessage::CursorByte { buffer_id, .. } if *buffer_id != scratch => Some(*buffer_id),
        _ => None,
    });

    // `C-x h` (edit.select-all), `M-w` (edit.copy), then `M-<` `C-n`,
    // which put the caret where it has not been: the second line.
    tui.key(Key::Char('x'), Modifiers::CTRL);
    tui.key(Key::Char('h'), Modifiers::NONE);
    tui.key(Key::Char('w'), Modifiers::ALT);
    tui.key(Key::Char('<'), Modifiers::ALT);
    tui.key(Key::Char('n'), Modifiers::CTRL);
    assert!(
        served_at(&mut tui, log, 80, 2),
        "the TUI was not answered after its copy (drain announced: {}): {}",
        drain_announced(&daemon, fid),
        daemon_log(&daemon)
    );

    // Past the drain's deadline, as a drain begun at the clipboard's
    // write would have ended the session by now.
    std::thread::sleep(DRAIN + Duration::from_millis(500));
    assert!(
        !drain_announced(&daemon, fid),
        "a clipboard refused before a byte was written ended the TUI's output: {}",
        daemon_log(&daemon)
    );
    assert!(
        !detached(&daemon).contains(&id),
        "the TUI was torn down after its copy: {}",
        daemon_log(&daemon)
    );
    tui.key(Key::Char('n'), Modifiers::CTRL);
    assert!(
        served_at(&mut tui, log, 160, 2),
        "the TUI was not answered past the deadline: {}",
        daemon_log(&daemon)
    );
    std::mem::forget(dir);
}

/// P1a (E10d.2, Medium 1) --- the same refusal from a GPU-shaped window,
/// through the kill ring's own call (`pmacs.editor.clipboard_set`, which
/// `M-w`, `C-w` and `C-k` run), with no big buffer: a fast witness of the
/// route. The window types after the copy and is echoed, and again past
/// the drain's deadline.
#[test]
fn e10d_review1_p1a_a_clipboard_past_the_frame_ends_nothing() {
    let daemon = daemon_with(
        "pmacs.command.define { name = 'r1.copy-big', description = 'a 17 MiB kill',\n\
           fn = function() pmacs.editor.clipboard_set(string.rep('x', 17 * 1024 * 1024)) end }\n",
    );
    let mut window = WireSession::semantic(&daemon, PROTOCOL_VERSION, None, None);
    let (document, at) = window.where_am_i();
    window.declare_viewport(document);
    let fid = window.fid;
    let id = fid.0.to_string();
    window.run_command("r1.copy-big");
    window.type_str("k");
    assert!(
        served_at(&mut window, document, at + 1, 2),
        "the window was not echoed after its copy (drain announced: {}): {}",
        drain_announced(&daemon, fid),
        daemon_log(&daemon)
    );
    std::thread::sleep(DRAIN + Duration::from_millis(500));
    assert!(
        !drain_announced(&daemon, fid),
        "a clipboard refused before a byte was written ended the window's output: {}",
        daemon_log(&daemon)
    );
    assert!(
        !detached(&daemon).contains(&id),
        "the window was torn down after its copy: {}",
        daemon_log(&daemon)
    );
    window.type_str("k");
    assert!(
        served_at(&mut window, document, at + 2, 2),
        "the window was not echoed past the deadline: {}",
        daemon_log(&daemon)
    );
}

/// Wait until the daemon's log holds `needle`, ten seconds at most: a
/// row's premise, that the route it names met the refusal.
fn logged(daemon: &TestDaemon, what: &str, needle: &str) {
    common::ready::expect_true(
        what,
        Duration::from_secs(10),
        || daemon_log(daemon).contains(needle),
        || daemon_log(daemon),
    );
}

/// Fix round 1 (E10d.2, Medium 1's ruling) --- the tick's frame route
/// meets a refusal: a status message past the frame (a plugin setting
/// the whole of a command's output with `pmacs.editor.set_status`, here
/// 17 MiB) rides the window's next `StatusFacts`, which `write_message`
/// refuses before writing a byte. At `73bc1e6` and at `fd38a8a` the frame
/// route drained on a refusal as on a failed socket, and the window was
/// torn down at the deadline; since fix round 1 the refusal is logged
/// with the frame route's line, the message is left out, the rest of the
/// frame written and the session kept, its band without that message.
/// The key goes only once the refusal is logged, since a key clears the
/// status.
#[test]
fn e10d_fixes1_a_status_past_the_frame_ends_nothing() {
    let daemon = daemon_with(
        "pmacs.command.define { name = 'f1.status-big', description = 'a 17 MiB status',\n\
           fn = function() pmacs.editor.set_status(string.rep('x', 17 * 1024 * 1024)) end }\n",
    );
    let mut window = WireSession::semantic(&daemon, PROTOCOL_VERSION, None, None);
    let (document, at) = window.where_am_i();
    window.declare_viewport(document);
    let fid = window.fid;
    let id = fid.0.to_string();
    window.run_command("f1.status-big");
    logged(
        &daemon,
        "premise: the frame route's refusal of the status",
        &format!("pmacs: write failed for {fid:?} in dispatcher: frame length "),
    );
    window.type_str("k");
    assert!(
        served_at(&mut window, document, at + 1, 2),
        "the window was not echoed after its status (drain announced: {}): {}",
        drain_announced(&daemon, fid),
        daemon_log(&daemon)
    );
    std::thread::sleep(DRAIN + Duration::from_millis(500));
    assert!(
        !drain_announced(&daemon, fid),
        "a frame message refused before a byte was written ended the window's output: {}",
        daemon_log(&daemon)
    );
    assert!(
        !detached(&daemon).contains(&id),
        "the window was torn down after its status: {}",
        daemon_log(&daemon)
    );
    window.type_str("k");
    assert!(
        served_at(&mut window, document, at + 2, 2),
        "the window was not echoed past the deadline: {}",
        daemon_log(&daemon)
    );
}

/// Fix round 1 (E10d.2, Medium 1's ruling on the op route) --- a CRDT op
/// past the frame: a Lua insert of 17 MiB into the buffer the window
/// mirrors (a plugin's, or an inserted file's) is broadcast to the
/// window as one `CrdtOp`, which `write_message` refuses before writing a
/// byte. At `73bc1e6` the refusal was dropped with nothing logged, and at
/// `fd38a8a` it began the drain and the window was torn down; since fix
/// round 1 it is logged with the op route's line and the session kept.
/// Either way the window's mirror goes without the insert, the replica
/// falling behind the daemon, which is filed for E11's subscriptions.
#[test]
fn e10d_fixes1_an_op_past_the_frame_ends_nothing() {
    let daemon = daemon_with(
        "pmacs.command.define { name = 'f1.insert-big', description = 'a 17 MiB insert',\n\
           fn = function()\n\
             local buf = pmacs.window.buffer()\n\
             buf:insert(buf:len(), string.rep('x', 17 * 1024 * 1024))\n\
             pmacs.command.invoke('cursor.buffer-start')\n\
           end }\n",
    );
    let mut window = WireSession::semantic(&daemon, PROTOCOL_VERSION, None, None);
    let (document, _) = window.where_am_i();
    window.declare_viewport(document);
    let fid = window.fid;
    let id = fid.0.to_string();
    window.run_command("f1.insert-big");
    logged(
        &daemon,
        "premise: the op route's refusal of the insert",
        &format!("pmacs: write CrdtOp for {fid:?} failed: frame length "),
    );
    window.type_str("k");
    assert!(
        served_at(&mut window, document, 1, 2),
        "the window was not echoed after the insert (drain announced: {}): {}",
        drain_announced(&daemon, fid),
        daemon_log(&daemon)
    );
    std::thread::sleep(DRAIN + Duration::from_millis(500));
    assert!(
        !drain_announced(&daemon, fid),
        "an op refused before a byte was written ended the window's output: {}",
        daemon_log(&daemon)
    );
    assert!(
        !detached(&daemon).contains(&id),
        "the window was torn down after the insert: {}",
        daemon_log(&daemon)
    );
    window.type_str("k");
    assert!(
        served_at(&mut window, document, 2, 2),
        "the window was not echoed past the deadline: {}",
        daemon_log(&daemon)
    );
}
