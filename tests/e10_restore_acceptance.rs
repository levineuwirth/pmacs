//! E10 --- restore: the daemon keeps a desktop per daemon and frontend
//! kind (D16), written when a window detaches, for every attached window
//! at shutdown, and by `desktop-save` in a window, the last written
//! winning (E10.1); a GPU window that attaches naming no file, with no
//! view retained for it, opens on its kind's desktop, and
//! `desktop-restore` in a window of either kind restores its kind's
//! (E10.2).
//!
//! Every row drives a real daemon over its socket through
//! `common::wire::WireSession` and reads what the daemon wrote where it
//! writes it: the state store under the daemon's isolated state root
//! (`<tempdir>/pmacs/desktop/`), keyed by the daemon's identity, which
//! for a daemon on a socket path is its working directory, and the
//! window's kind.
#![cfg(feature = "crdt")]

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use pmacs::desktop::{FrontendKind, SavedDesktop, SavedNode};
use pmacs_protocol::BufferId;
use pmacs_protocol::cell::CellSize;
use pmacs_protocol::message::{InstanceMessage, Key, Modifiers, PROTOCOL_VERSION};

use common::daemon::TestDaemon;
use common::gpu_probe::{fact, gpu_probe_binary, probe_facts};
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

/// The key the daemon keeps `kind`'s desktop under: its identity on a
/// socket path is its working directory, which it shares with this test.
fn kind_key(kind: FrontendKind) -> String {
    let cwd = std::env::current_dir().expect("cwd");
    pmacs::desktop::frontend_session_key(
        &pmacs::desktop::session_key(None, &cwd.to_string_lossy()),
        kind,
    )
}

/// Where `kind`'s desktop lives under `state_root` (the daemon's
/// `PMACS_STATE_HOME`).
fn desktop_file(state_root: &Path, kind: FrontendKind) -> PathBuf {
    state_root
        .join("pmacs")
        .join(pmacs::desktop::desktop_state_key(&kind_key(kind)))
}

/// `kind`'s desktop as the daemon wrote it, if it did.
fn saved(state_root: &Path, kind: FrontendKind) -> Option<SavedDesktop> {
    let json = std::fs::read_to_string(desktop_file(state_root, kind)).ok()?;
    let desktop: SavedDesktop = serde_json::from_str(&json).expect("a desktop the daemon can read");
    assert_eq!(
        desktop.session_key,
        kind_key(kind),
        "keyed by daemon and kind"
    );
    Some(desktop)
}

/// The one leaf of a single-window desktop: its file and cursor.
fn only_leaf(desktop: &SavedDesktop) -> (String, u64) {
    match &desktop.root {
        SavedNode::Leaf(leaf) => (leaf.path.clone(), leaf.cursor),
        split @ SavedNode::Split { .. } => panic!("expected one window, got {split:?}"),
    }
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

/// `pmacs --daemon --stop --socket <path>`, then the daemon's exit.
fn stop(daemon: &mut TestDaemon) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pmacs"))
        .arg("--daemon")
        .arg("--stop")
        .arg("--socket")
        .arg(daemon.socket_path())
        .output()
        .expect("run pmacs --daemon --stop");
    assert!(
        output.status.success(),
        "--stop: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status = daemon.wait_for_exit().expect("the daemon exits");
    assert!(status.success(), "the daemon stops cleanly: {status:?}");
}

#[test]
fn e10_1_a_departing_semantic_window_writes_what_it_had_under_its_kind() {
    let daemon = daemon();
    let state = home(&daemon);
    let note_path = state.join("note.txt").display().to_string();
    let (mut window, scratch) = semantic(&daemon);
    let note = open(&mut window, scratch, "e10.note");
    next_line(&mut window);
    window.cursor_at(note, 4);
    assert!(
        saved(&state, FrontendKind::Semantic).is_none(),
        "nothing is written while the window is attached"
    );

    window.depart();
    wait_for_detach(&daemon, &window);

    let desktop = saved(&state, FrontendKind::Semantic).expect("the departure wrote the desktop");
    assert_eq!(only_leaf(&desktop), (note_path.clone(), 4));
    assert!(
        desktop
            .buffers
            .iter()
            .any(|b| b.path == note_path && !b.modified),
        "every open file buffer is kept: {:?}",
        desktop.buffers
    );
    assert!(
        saved(&state, FrontendKind::Grid).is_none(),
        "a semantic window writes nothing under the grid kind"
    );
}

#[test]
fn e10_1_a_lost_window_writes_what_it_had_too() {
    let daemon = daemon();
    let state = home(&daemon);
    let (mut window, scratch) = semantic(&daemon);
    let note = open(&mut window, scratch, "e10.note");
    next_line(&mut window);
    window.cursor_at(note, 4);

    window.lose();
    wait_for_detach(&daemon, &window);

    let desktop = saved(&state, FrontendKind::Semantic).expect("the loss wrote the desktop");
    assert_eq!(
        only_leaf(&desktop),
        (state.join("note.txt").display().to_string(), 4)
    );
}

#[test]
fn e10_1_a_grid_window_writes_under_the_grid_kind() {
    let daemon = daemon();
    let state = home(&daemon);
    let mut grid = WireSession::grid(&daemon, CellSize::new(24, 80));
    grid.run_command("e10.other");
    next_line(&mut grid);
    grid.wait("the grid window a line into the other file", |m| {
        matches!(m, InstanceMessage::CursorByte { byte_pos: 6, .. }).then_some(())
    });

    grid.depart();
    wait_for_detach(&daemon, &grid);

    let desktop = saved(&state, FrontendKind::Grid).expect("the grid window's desktop");
    assert_eq!(
        only_leaf(&desktop),
        (state.join("other.txt").display().to_string(), 6)
    );
    assert!(
        saved(&state, FrontendKind::Semantic).is_none(),
        "a grid window writes nothing under the semantic kind"
    );
}

#[test]
fn e10_1_the_window_detached_last_wins_whichever_it_is() {
    let daemon = daemon();
    let state = home(&daemon);
    let (mut a, scratch) = semantic(&daemon);
    let note = open(&mut a, scratch, "e10.note");
    a.cursor_at(note, 0);
    let (mut b, scratch_b) = semantic(&daemon);
    let other = open(&mut b, scratch_b, "e10.other");
    next_line(&mut b);
    b.cursor_at(other, 6);

    // The later window detaches first, then the earlier one: each
    // detach overwrites, so the order of detaching decides, not the
    // order of attaching.
    b.depart();
    wait_for_detach(&daemon, &b);
    let first = saved(&state, FrontendKind::Semantic).expect("B's desktop");
    assert_eq!(
        only_leaf(&first),
        (state.join("other.txt").display().to_string(), 6)
    );

    a.depart();
    wait_for_detach(&daemon, &a);
    let last = saved(&state, FrontendKind::Semantic).expect("A's desktop");
    assert_eq!(
        only_leaf(&last),
        (state.join("note.txt").display().to_string(), 0)
    );
}

#[test]
fn e10_1_a_window_leaving_nothing_to_save_clears_its_kinds_desktop() {
    let daemon = daemon();
    let state = home(&daemon);
    let (mut a, scratch) = semantic(&daemon);
    let note = open(&mut a, scratch, "e10.note");
    a.cursor_at(note, 0);
    let (mut b, on) = semantic(&daemon);
    assert_eq!(on, scratch, "B shows the scratch, no file");
    a.depart();
    wait_for_detach(&daemon, &a);
    assert!(saved(&state, FrontendKind::Semantic).is_some());

    // B, on `*scratch*` alone, detaches last: what it had is a fresh
    // window, so the kind's desktop goes.
    b.depart();
    wait_for_detach(&daemon, &b);
    assert!(
        !desktop_file(&state, FrontendKind::Semantic).exists(),
        "the last window detached left nothing, and nothing is kept"
    );
}

/// Two semantic windows and a grid window, all attached, the earlier
/// semantic window A on the note and the later B on the other file, and
/// A the last of the two to give input. Returns `(a, b, grid)`.
fn three_windows(daemon: &TestDaemon) -> (WireSession, WireSession, WireSession) {
    let (mut a, scratch) = semantic(daemon);
    let note = open(&mut a, scratch, "e10.note");
    a.cursor_at(note, 0);
    let (mut b, scratch_b) = semantic(daemon);
    let other = open(&mut b, scratch_b, "e10.other");
    b.cursor_at(other, 0);
    let mut grid = WireSession::grid(daemon, CellSize::new(24, 80));
    grid.run_command("e10.other");
    grid.wait("the grid window on the other file", |m| match m {
        InstanceMessage::CursorByte { buffer_id, .. } if *buffer_id == other => Some(()),
        _ => None,
    });
    next_line(&mut b);
    b.cursor_at(other, 6);
    next_line(&mut a);
    a.cursor_at(note, 4);
    (a, b, grid)
}

#[test]
fn e10_1_a_signal_writes_every_attached_window_the_last_to_give_input_last() {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;
    let mut daemon = daemon();
    let state = home(&daemon);
    let _windows = three_windows(&daemon);

    kill(
        Pid::from_raw(i32::try_from(daemon.pid()).expect("pid")),
        Signal::SIGTERM,
    )
    .expect("SIGTERM");
    let status = daemon.wait_for_exit().expect("the daemon exits");
    assert!(status.success(), "the daemon stops cleanly: {status:?}");

    // A attached first and gave input last: written last, it is what
    // the semantic kind keeps, not the later B.
    let semantic = saved(&state, FrontendKind::Semantic).expect("the semantic windows' desktop");
    assert_eq!(
        only_leaf(&semantic),
        (state.join("note.txt").display().to_string(), 4)
    );
    let grid = saved(&state, FrontendKind::Grid).expect("the grid window's desktop");
    assert_eq!(
        only_leaf(&grid),
        (state.join("other.txt").display().to_string(), 0)
    );
}

#[test]
fn e10_1_stop_writes_every_window_and_not_its_own_session() {
    let mut daemon = daemon();
    let state = home(&daemon);
    let _windows = three_windows(&daemon);

    stop(&mut daemon);

    // `--stop` attaches a grid session of its own and gives the last
    // input; it keeps no desktop, so the windows are written in attach
    // order and the later B is what the semantic kind keeps.
    let semantic = saved(&state, FrontendKind::Semantic).expect("the semantic windows' desktop");
    assert_eq!(
        only_leaf(&semantic),
        (state.join("other.txt").display().to_string(), 6)
    );
    // Its own view of the scratch would have cleared the grid kind's.
    let grid = saved(&state, FrontendKind::Grid).expect("the grid window's desktop survives");
    assert_eq!(
        only_leaf(&grid),
        (state.join("other.txt").display().to_string(), 0)
    );
}

#[test]
fn e10_1_desktop_save_in_a_window_writes_that_window_under_its_kind() {
    let daemon = daemon();
    let state = home(&daemon);
    let (mut window, scratch) = semantic(&daemon);
    let note = open(&mut window, scratch, "e10.note");
    next_line(&mut window);
    window.cursor_at(note, 4);

    window.run_command("desktop-save");
    window.wait("the command's own word", |m| match m {
        InstanceMessage::StatusFacts {
            message: Some(message),
            ..
        } if message == "desktop saved" => Some(()),
        _ => None,
    });

    let desktop = saved(&state, FrontendKind::Semantic).expect("the command wrote the desktop");
    assert_eq!(
        only_leaf(&desktop),
        (state.join("note.txt").display().to_string(), 4)
    );
}

// ---------------------------------------------------------------------
// E10.2 --- a fresh semantic window that names no target is built from
// its kind's desktop; a target wins, a retained view wins, a grid window
// is built as before, and `desktop-restore` in a window restores it.
// ---------------------------------------------------------------------

/// The init with `extra` after it.
fn daemon_with(extra: &str) -> TestDaemon {
    TestDaemon::spawn_with_env_and_init(&[("PMACS_ATTACH_DEBUG", "1")], &format!("{INIT}{extra}"))
}

/// A window on the note a line down (byte 4) that visited the other
/// file after it, so the registry lists the note before the other file
/// and an all-buffer sweep's last snapshot is not the note's. Returns
/// `(window, scratch, note, other)`.
fn on_the_note_after_the_other(daemon: &TestDaemon) -> (WireSession, BufferId, BufferId, BufferId) {
    let (mut window, scratch) = semantic(daemon);
    let note = open(&mut window, scratch, "e10.note");
    let other = open(&mut window, note, "e10.other");
    window.run_command("e10.note");
    window.cursor_at(note, 0);
    window.declare_viewport(note);
    next_line(&mut window);
    window.cursor_at(note, 4);
    (window, scratch, note, other)
}

/// Answer every snapshot of the bootstrap with a viewport on its buffer,
/// in order, as `pmacs-gpu` does: a viewport moves the window to the
/// buffer it names, so after this the window is where the last snapshot
/// put it.
fn answer_the_bootstrap(window: &mut WireSession) {
    for buffer in window.bootstrap_snapshots() {
        window.declare_viewport(buffer);
    }
}

#[test]
fn e10_2_a_fresh_semantic_window_reopens_what_the_last_one_had() {
    let daemon = daemon();
    let (mut a, _scratch, note, _other) = on_the_note_after_the_other(&daemon);
    a.depart();
    wait_for_detach(&daemon, &a);

    let mut b = WireSession::semantic(&daemon, PROTOCOL_VERSION, None, None);
    // Bootstrapped on the one buffer its window shows, as a resumed
    // window is (E9): an all-buffer sweep would leave it on the other
    // file, the sweep's last.
    assert_eq!(
        b.bootstrap_snapshots(),
        vec![note],
        "one snapshot, the note's"
    );
    answer_the_bootstrap(&mut b);
    assert_eq!(b.where_am_i(), (note, 4), "the note, a line down");
    b.type_str("x");
    b.cursor_at(note, 5);
}

#[test]
fn e10_2_a_restarted_daemon_reopens_what_the_last_window_had() {
    let shared = tempfile::TempDir::new().expect("a shared state root");
    let root = shared.path().to_str().expect("utf-8 path");
    let env = [("PMACS_ATTACH_DEBUG", "1"), ("PMACS_STATE_HOME", root)];
    let mut first = TestDaemon::spawn_with_env_and_init(&env, INIT);
    let (mut a, scratch) = semantic(&first);
    let note = open(&mut a, scratch, "e10.note");
    next_line(&mut a);
    a.cursor_at(note, 4);
    stop(&mut first);

    // The same identity (this working directory) and state root, a new
    // process: nothing of the first daemon's is in it but the desktop.
    let second = TestDaemon::spawn_with_env_and_init(&env, INIT);
    let mut b = WireSession::semantic(&second, PROTOCOL_VERSION, None, None);
    let snapshots = b.bootstrap_snapshots();
    assert_eq!(snapshots.len(), 1, "one snapshot: {snapshots:?}");
    let reopened = snapshots[0];
    b.declare_viewport(reopened);
    let name = b.wait("the reopened buffer's facts", |m| match m {
        InstanceMessage::StatusFacts {
            buffer_id, name, ..
        } if *buffer_id == reopened => Some(name.clone()),
        _ => None,
    });
    assert_eq!(
        name,
        home(&first).join("note.txt").display().to_string(),
        "the first daemon's note"
    );
    b.cursor_at(reopened, 4);
    b.type_str("x");
    b.cursor_at(reopened, 5);
}

#[test]
fn e10_2_a_target_wins_over_the_desktop() {
    use std::os::unix::ffi::OsStrExt;
    let daemon = daemon_with(LAYOUT);
    let (note, other) = split_desktop(&daemon);

    let target = pmacs_protocol::message::InitialTarget {
        cwd: home(&daemon).as_os_str().as_bytes().to_vec(),
        path: b"other.txt".to_vec(),
    };
    let mut b = WireSession::semantic(&daemon, PROTOCOL_VERSION, Some(target), None);
    let opened = b
        .bootstrap
        .iter()
        .find_map(|m| match m {
            InstanceMessage::InitialTargetResult(
                pmacs_protocol::message::InitialTargetResult::Opened { buffer_id },
            ) => Some(*buffer_id),
            _ => None,
        })
        .expect("the target opened");
    assert_eq!(opened, other);
    assert_eq!(b.bootstrap_snapshots(), vec![other]);
    answer_the_bootstrap(&mut b);
    b.type_str("x");
    b.cursor_at(other, 1);
    // One window on the target: none of the desktop's split, whose
    // focused window holds the target's file too, so only the layout
    // tells a restored view from a fresh one.
    b.run_command("e10.layout");
    let other_path = home(&daemon).join("other.txt").display().to_string();
    common::ready::expect_true(
        "the target's own layout",
        Duration::from_secs(10),
        || lines(&daemon, "layout.log") == vec![other_path.clone()],
        || format!("{:?} (the note is {note:?})", lines(&daemon, "layout.log")),
    );
}

#[test]
fn e10_2_a_retained_view_wins_over_the_desktop() {
    let daemon = daemon();
    let (mut a, _scratch, note, other) = on_the_note_after_the_other(&daemon);
    let token = a.token.expect("a token");
    a.lose();
    wait_for_detach(&daemon, &a);

    // A later window leaves the other file, a line down, as the
    // desktop: the retained view is the note's, the desktop is not.
    let mut b = WireSession::semantic(&daemon, PROTOCOL_VERSION, None, None);
    answer_the_bootstrap(&mut b);
    b.cursor_at(note, 4);
    b.run_command("e10.other");
    b.cursor_at(other, 0);
    b.declare_viewport(other);
    next_line(&mut b);
    b.cursor_at(other, 6);
    b.depart();
    wait_for_detach(&daemon, &b);

    let mut resumed = WireSession::semantic(&daemon, PROTOCOL_VERSION, None, Some(token));
    assert_eq!(resumed.bootstrap_snapshots(), vec![note]);
    answer_the_bootstrap(&mut resumed);
    assert_eq!(
        resumed.where_am_i(),
        (note, 4),
        "its own view, not the desktop"
    );
}

#[test]
fn e10_2_a_window_whose_view_is_gone_reopens_the_desktop_and_says_so() {
    let daemon = daemon_with("pmacs.config.set('session.view-retention-secs', 0)\n");
    let (mut a, _scratch, note, _other) = on_the_note_after_the_other(&daemon);
    let token = a.token.expect("a token");
    a.lose();
    wait_for_detach(&daemon, &a);

    // Nothing is retained at zero, so the token names nothing: a fresh
    // view, which is the kind's desktop, the one this window left.
    let mut back = WireSession::semantic(&daemon, PROTOCOL_VERSION, None, Some(token));
    assert_eq!(back.bootstrap_snapshots(), vec![note]);
    answer_the_bootstrap(&mut back);
    let said = back.wait("the status saying the view is gone", |m| match m {
        InstanceMessage::StatusFacts {
            message: Some(message),
            ..
        } if message.contains("earlier view is gone") => Some(message.clone()),
        _ => None,
    });
    assert!(
        said.contains("reopening what the last window of its kind had"),
        "{said:?}"
    );
    assert_eq!(back.where_am_i(), (note, 4));
}

#[test]
fn e10_2_a_grid_window_is_built_as_before_and_no_kind_reads_the_others() {
    let daemon = daemon();
    let mut grid = WireSession::grid(&daemon, CellSize::new(24, 80));
    let (scratch, _) = grid.where_am_i();
    grid.run_command("e10.other");
    next_line(&mut grid);
    grid.wait("the grid window a line into the other file", |m| {
        matches!(m, InstanceMessage::CursorByte { byte_pos: 6, .. }).then_some(())
    });
    grid.depart();
    wait_for_detach(&daemon, &grid);
    assert!(saved(&home(&daemon), FrontendKind::Grid).is_some());

    // The grid kind's desktop is no semantic window's to reopen.
    let (_semantic, on) = semantic(&daemon);
    assert_eq!(on, scratch, "a semantic window does not reopen the grid's");
    // And a grid window is built on LOCAL's document as before E10
    // (`docs/divergences.md`).
    let mut again = WireSession::grid(&daemon, CellSize::new(24, 80));
    assert_eq!(again.where_am_i(), (scratch, 0));
}

#[test]
fn e10_2_desktop_restore_in_a_window_brings_its_kinds_desktop_back() {
    let daemon = daemon();
    let (mut window, scratch) = semantic(&daemon);
    let note = open(&mut window, scratch, "e10.note");
    next_line(&mut window);
    window.cursor_at(note, 4);
    window.run_command("desktop-save");
    window.wait("desktop saved", |m| match m {
        InstanceMessage::StatusFacts {
            message: Some(message),
            ..
        } if message == "desktop saved" => Some(()),
        _ => None,
    });
    let other = open(&mut window, note, "e10.other");
    window.cursor_at(other, 0);

    window.run_command("desktop-restore");
    window.cursor_at(note, 4);
    window.declare_viewport(note);
    window.type_str("x");
    window.cursor_at(note, 5);
}

/// The restored windows' buffers, read from the daemon by a command in
/// the window (its layout is not on the semantic wire before E11).
const LAYOUT: &str = "\
pmacs.command.define { name = 'e10.layout', description = 'log the layout',
  fn = function()
    local f = assert(io.open(home .. '/layout.log', 'w'))
    for _, id in ipairs(pmacs.window.list()) do
      f:write(pmacs.window.buffer(id):name(), '\\n')
    end
    f:close()
  end }
pmacs.hook.add('buffer.after-load', function()
  local f = assert(io.open(home .. '/after-load.log', 'a'))
  f:write(pmacs.window.buffer():name(), '\\n')
  f:close()
end)
";

fn lines(daemon: &TestDaemon, file: &str) -> Vec<String> {
    std::fs::read_to_string(home(daemon).join(file))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Under [`LAYOUT`]'s init: a window splits beside itself (`C-x 3`),
/// keeping the note on the left, takes the other file into the right
/// window (`C-x o`), reads its layout as `[note, other]`, and departs,
/// so the semantic desktop is that split with the right window focused.
/// Returns `(note, other)`.
fn split_desktop(daemon: &TestDaemon) -> (BufferId, BufferId) {
    let note_path = home(daemon).join("note.txt").display().to_string();
    let other_path = home(daemon).join("other.txt").display().to_string();
    let (mut a, scratch) = semantic(daemon);
    let note = open(&mut a, scratch, "e10.note");
    a.key(Key::Char('x'), Modifiers::CTRL);
    a.key(Key::Char('3'), Modifiers::NONE);
    a.key(Key::Char('x'), Modifiers::CTRL);
    a.key(Key::Char('o'), Modifiers::NONE);
    let other = open(&mut a, note, "e10.other");
    a.cursor_at(other, 0);
    a.run_command("e10.layout");
    common::ready::expect_true(
        "the window's own layout",
        Duration::from_secs(10),
        || lines(daemon, "layout.log") == vec![note_path.clone(), other_path.clone()],
        || format!("{:?}", lines(daemon, "layout.log")),
    );
    a.depart();
    wait_for_detach(daemon, &a);
    std::fs::remove_file(home(daemon).join("layout.log")).expect("the layout read");
    (note, other)
}

#[test]
fn e10_2_a_split_desktop_comes_back_whole_each_window_loaded_in_turn() {
    let daemon = daemon_with(LAYOUT);
    let (_note, other) = split_desktop(&daemon);
    let note_path = home(&daemon).join("note.txt").display().to_string();
    let other_path = home(&daemon).join("other.txt").display().to_string();
    let before = lines(&daemon, "after-load.log").len();

    let mut b = WireSession::semantic(&daemon, PROTOCOL_VERSION, None, None);
    assert_eq!(
        b.bootstrap_snapshots(),
        vec![other],
        "the focused window's buffer"
    );
    answer_the_bootstrap(&mut b);
    assert_eq!(b.where_am_i(), (other, 0));
    // `buffer.after-load` ran once a leaf, each with its leaf active.
    let after = lines(&daemon, "after-load.log");
    assert_eq!(
        after[before..].to_vec(),
        vec![note_path.clone(), other_path.clone()],
        "{after:?}"
    );
    b.run_command("e10.layout");
    common::ready::expect_true(
        "the restored layout",
        Duration::from_secs(10),
        || lines(&daemon, "layout.log") == vec![note_path.clone(), other_path.clone()],
        || format!("{:?}", lines(&daemon, "layout.log")),
    );
}

#[test]
fn e10_2_with_reopen_desktop_off_a_fresh_window_starts_where_it_did_before() {
    let daemon = daemon_with("pmacs.config.set('session.reopen-desktop', false)\n");
    let state = home(&daemon);
    let (mut a, scratch, _note, _other) = on_the_note_after_the_other(&daemon);
    a.depart();
    wait_for_detach(&daemon, &a);
    assert!(
        saved(&state, FrontendKind::Semantic).is_some(),
        "the desktop is written whatever the knob says"
    );

    let (mut b, on) = semantic(&daemon);
    assert_eq!(on, scratch, "LOCAL's document, as before E10");
    // And by hand it comes back.
    b.run_command("desktop-restore");
    b.wait("the desktop restored by hand", |m| {
        matches!(m, InstanceMessage::CursorByte { byte_pos: 4, buffer_id } if *buffer_id != scratch)
            .then_some(())
    });
}

#[test]
fn e10_2_desktop_restore_in_a_grid_window_brings_the_grid_desktop_back() {
    let daemon = daemon();
    let mut grid = WireSession::grid(&daemon, CellSize::new(24, 80));
    let (scratch, _) = grid.where_am_i();
    grid.run_command("e10.other");
    next_line(&mut grid);
    let other = grid.wait("the grid window a line into the other file", |m| match m {
        InstanceMessage::CursorByte {
            buffer_id,
            byte_pos: 6,
        } => Some(*buffer_id),
        _ => None,
    });
    grid.depart();
    wait_for_detach(&daemon, &grid);

    let mut again = WireSession::grid(&daemon, CellSize::new(24, 80));
    assert_eq!(again.where_am_i(), (scratch, 0), "not at the attach");
    again.run_command("desktop-restore");
    again.cursor_at(other, 6);
}

/// The status a fresh window's first facts carry for `buffer`, once it
/// declares a viewport there.
fn first_status(window: &mut WireSession, buffer: BufferId) -> Option<String> {
    window.declare_viewport(buffer);
    window.wait("the buffer's facts", |m| match m {
        InstanceMessage::StatusFacts {
            buffer_id, message, ..
        } if *buffer_id == buffer => Some(message.clone()),
        _ => None,
    })
}

#[test]
fn e10_2_unsaved_changes_are_told_only_where_the_reopened_file_lost_them() {
    let shared = tempfile::TempDir::new().expect("a shared state root");
    let root = shared.path().to_str().expect("utf-8 path");
    let env = [("PMACS_ATTACH_DEBUG", "1"), ("PMACS_STATE_HOME", root)];
    // No recovery copy: the stop's last sweep would write one, and the
    // restarted daemon then says so instead (autosave's own report,
    // which comes after the restore's).
    let init = format!("{INIT}pmacs.autosave.enable(false)\n");
    let mut first = TestDaemon::spawn_with_env_and_init(&env, &init);
    let (mut a, scratch) = semantic(&first);
    let note = open(&mut a, scratch, "e10.note");
    a.type_str("z");
    a.cursor_at(note, 1);
    a.depart();
    wait_for_detach(&first, &a);

    // The same daemon still holds the note with its edit: reopening it
    // loses nothing, and says nothing of it.
    let mut b = WireSession::semantic(&first, PROTOCOL_VERSION, None, None);
    assert_eq!(b.bootstrap_snapshots(), vec![note]);
    let said = first_status(&mut b, note);
    assert!(
        !said.as_deref().unwrap_or("").contains("unsaved changes"),
        "{said:?}"
    );
    b.depart();
    wait_for_detach(&first, &b);
    stop(&mut first);

    // A restarted daemon opens the file from disk, without the edit.
    let second = TestDaemon::spawn_with_env_and_init(&env, &init);
    let mut c = WireSession::semantic(&second, PROTOCOL_VERSION, None, None);
    let reopened = c.bootstrap_snapshots();
    assert_eq!(reopened.len(), 1, "{reopened:?}");
    let said = first_status(&mut c, reopened[0]);
    assert_eq!(
        said.as_deref(),
        Some("desktop restored; 1 buffer(s) had unsaved changes when saved")
    );
}

/// `pmacs-gpu --headless-probe` against `daemon` with its reconnect
/// action and no cut: the production `App` attaches with no file,
/// answers its bootstrap as a window does, types `x` once its caret is
/// the daemon's, and reports its own mirror (`buffer.before`,
/// `cursor.before`, `text.before`) when the deadline ends it, which is
/// its only end without a cut. Fifteen seconds: CI's `Test (crdt)` leg
/// (job 119271402563) did not reach the typing inside six in the row's
/// control arm, since dropped. `None` when skipped (no binary or
/// adapter), which panics under `PMACS_REQUIRE_GPU`.
fn gpu_window_types_x(daemon: &TestDaemon) -> Option<std::collections::HashMap<String, String>> {
    let binary = gpu_probe_binary()?;
    let dir = tempfile::TempDir::new().expect("tempdir");
    let report = dir.path().join("restore.report");
    let output = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(daemon.socket_path())
        .arg(&report)
        .env("PMACS_GPU_PROBE_TYPE_TEXT", "x")
        .env("PMACS_GPU_PROBE_ACTION", "reconnect")
        .env("PMACS_GPU_PROBE_COMMAND", "")
        .env("PMACS_GPU_PROBE_DEADLINE_MS", "15000")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("run the headless GPU probe");
    probe_facts(&output, &report, "no cut")
}

/// The GUI itself, not a client standing in for it: `pmacs-gpu`'s
/// production `App` attaches with no file to a daemon whose semantic
/// desktop is the note a line down, and its own mirror holds the note
/// with its `x` typed at byte 4, the caret after it. The desktop is
/// asserted first, the row's precondition. A window that is not
/// reopened does not come back to that caret: with the reopening off
/// at the attach the row fails (the bite in the handoff), the window
/// ending where the all-buffer sweep and its viewport answers leave it
/// (seen once on the note at byte 0 with its `x` typed elsewhere, once
/// on the other file with the `x` at its start). That control ran here
/// as a second arm until CI's `Test (crdt)` leg timed it out (job
/// 119271402563); each arm runs to the probe's deadline, and that leg
/// is near its ceiling (#393).
#[test]
fn e10_2_the_gpu_window_itself_reopens_the_note_where_it_was_left() {
    let daemon = daemon();
    let (mut a, _scratch, note, _other) = on_the_note_after_the_other(&daemon);
    a.depart();
    wait_for_detach(&daemon, &a);
    let desktop = saved(&home(&daemon), FrontendKind::Semantic).expect("the desktop A left");
    assert_eq!(
        only_leaf(&desktop),
        (home(&daemon).join("note.txt").display().to_string(), 4)
    );

    let Some(facts) = gpu_window_types_x(&daemon) else {
        return;
    };
    assert_eq!(
        fact(&facts, "buffer.before"),
        note.raw().to_string(),
        "{facts:?}"
    );
    assert_eq!(fact(&facts, "text.before"), r#""one\nxtwo\nthree\n""#);
    assert_eq!(fact(&facts, "cursor.before"), "5");
}
