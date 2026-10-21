//! E10 --- restore: the daemon keeps a desktop per daemon and frontend
//! kind (D16), written when a window detaches, for every attached window
//! at shutdown, and by `desktop-save` in a window, the last written
//! winning (E10.1).
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
