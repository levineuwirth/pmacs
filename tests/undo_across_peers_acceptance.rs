// tests/undo_across_peers_acceptance.rs --- E6c: undo across peers.

//! The cross-peer undo arbiter at the daemon, witnessed through the
//! routes a user's keystrokes take. A GPU user's plain characters are
//! optimistic `FrontendEvent::CrdtOp`s on the GPU's own loro peer; a
//! TUI user's round-trip as `Key` events and land on the daemon's
//! peer. Every undo chord round-trips, and the daemon pops the acting
//! source's own most recent command-boundary group whichever peer
//! carried it, compensating on its own peer so both replicas converge.
//!
//! Two synthetic replicas stand for the two frontends: `gpu` types
//! optimistically, `tui` types through keys; both are attached
//! replicas of one daemon, as `auto_pair_crdt_acceptance` has them.
//! The last row drives a real `pmacs-gpu --headless-probe` through its
//! production `App` dispatch: `hello`, then `C-x U` as the owner
//! presses it, and hello goes.

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

/// Pump broadcast messages into the replica until its text is
/// `expected` or the deadline passes.
fn pump_until_text(replica: &mut Replica, expected: &str, what: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if replica.state.materialize_string() == expected {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "pump timeout waiting for {what}: expected {expected:?}, text={:?}",
            replica.state.materialize_string()
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

/// `hello` on the GPU route, then undo: hello goes, on both replicas,
/// and a second undo finds nothing.
#[test]
fn hello_then_undo_on_the_gpu_route_empties_the_buffer() {
    let daemon = TestDaemon::spawn();
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_optimistic(&mut gpu, 0, "hello");
    both_see(&mut gpu, &mut tui, "hello", "hello");
    send_undo(&mut gpu);
    both_see(&mut gpu, &mut tui, "", "empty after undo");
    send_undo(&mut gpu);
    assert_text_stays(&mut gpu, "", Duration::from_millis(500));
}

/// `hello` on the TUI route, then undo: hello goes, on both replicas,
/// and a second undo finds nothing.
#[test]
fn hello_then_undo_on_the_tui_route_empties_the_buffer() {
    let daemon = TestDaemon::spawn();
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_keys(&mut tui, "hello");
    both_see(&mut gpu, &mut tui, "hello", "hello");
    send_undo(&mut tui);
    both_see(&mut gpu, &mut tui, "", "empty after undo");
    send_undo(&mut tui);
    assert_text_stays(&mut tui, "", Duration::from_millis(500));
}

/// `hello world`, eleven characters under the default amalgamation
/// limit of twenty, undoes as one step on both routes.
#[test]
fn hello_world_undoes_as_one_step_on_both_routes() {
    let daemon = TestDaemon::spawn();
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_optimistic(&mut gpu, 0, "hello world");
    both_see(&mut gpu, &mut tui, "hello world", "hello world (gpu)");
    send_undo(&mut gpu);
    both_see(&mut gpu, &mut tui, "", "empty after the gpu's undo");

    type_keys(&mut tui, "hello world");
    both_see(&mut gpu, &mut tui, "hello world", "hello world (tui)");
    send_undo(&mut tui);
    both_see(&mut gpu, &mut tui, "", "empty after the tui's undo");
}

/// GPU and TUI typing interleaved into one buffer: the TUI types
/// `xyz` through keys, the GPU types `abc` optimistically in front of
/// it. The GPU's undo removes `abc` and leaves `xyz`; the TUI's undo
/// then removes `xyz`. Each source undoes its own last group and never
/// the other's.
#[test]
fn interleaved_typing_each_source_undoes_its_own_last_group_gpu_first() {
    let daemon = TestDaemon::spawn();
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_keys(&mut tui, "xyz");
    both_see(&mut gpu, &mut tui, "xyz", "xyz");
    type_optimistic(&mut gpu, 0, "abc");
    both_see(&mut gpu, &mut tui, "abcxyz", "abcxyz");

    send_undo(&mut gpu);
    both_see(
        &mut gpu,
        &mut tui,
        "xyz",
        "the gpu's abc gone, the tui's xyz kept",
    );
    send_undo(&mut tui);
    both_see(&mut gpu, &mut tui, "", "the tui's xyz gone");
}

/// The same interleaving, the older source undoing first: the TUI's
/// undo removes its `xyz` from behind the GPU's later `abc`, which
/// stays; the GPU's undo then removes `abc`.
#[test]
fn interleaved_typing_each_source_undoes_its_own_last_group_tui_first() {
    let daemon = TestDaemon::spawn();
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_keys(&mut tui, "xyz");
    both_see(&mut gpu, &mut tui, "xyz", "xyz");
    type_optimistic(&mut gpu, 0, "abc");
    both_see(&mut gpu, &mut tui, "abcxyz", "abcxyz");

    send_undo(&mut tui);
    both_see(
        &mut gpu,
        &mut tui,
        "abc",
        "the tui's xyz gone, the gpu's abc kept",
    );
    send_undo(&mut gpu);
    both_see(&mut gpu, &mut tui, "", "the gpu's abc gone");
}

/// Foreign text inside an older group's text, the newer group undone
/// first: the TUI types `abcd`, steps back two and types `XY` (a
/// second group, inside the first's text), the GPU drops `Z` between
/// `c` and `d`. The TUI's first undo removes `XY` around the foreign
/// text; its second removes `abcd` around it, leaving `Z`. Undoing the
/// older group needs its base --- the `Z` insert --- re-expressed in
/// the coordinates the newer group's undo left, which is what a stale
/// base gets wrong (it would remove `abcZ` and leave `d`).
#[test]
fn a_foreign_edit_inside_an_older_group_survives_both_undos() {
    let daemon = TestDaemon::spawn();
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_keys(&mut tui, "abcd");
    both_see(&mut gpu, &mut tui, "abcd", "abcd");
    send_key(&mut tui, Key::Left, Modifiers::NONE);
    send_key(&mut tui, Key::Left, Modifiers::NONE);
    type_keys(&mut tui, "XY");
    both_see(&mut gpu, &mut tui, "abXYcd", "abXYcd");
    type_optimistic(&mut gpu, 5, "Z");
    both_see(&mut gpu, &mut tui, "abXYcZd", "abXYcZd");

    send_undo(&mut tui);
    both_see(&mut gpu, &mut tui, "abcZd", "XY gone around the foreign Z");
    send_undo(&mut tui);
    both_see(&mut gpu, &mut tui, "Z", "abcd gone around the foreign Z");
}

/// A command after optimistic typing is its own step: the GPU types
/// `ab` optimistically, then round-trips Backspace (a command chord,
/// dispatched at the daemon). The first undo restores `b`; the second
/// removes `ab`. The command's edit must not join the still-open
/// typed run, on this route exactly as on the dispatch route.
#[test]
fn a_command_after_optimistic_typing_is_its_own_undo_step() {
    let daemon = TestDaemon::spawn();
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_optimistic(&mut gpu, 0, "ab");
    both_see(&mut gpu, &mut tui, "ab", "ab");
    send_key(&mut gpu, Key::End, Modifiers::NONE);
    send_key(&mut gpu, Key::Backspace, Modifiers::NONE);
    both_see(&mut gpu, &mut tui, "a", "a after Backspace");

    send_undo(&mut gpu);
    both_see(&mut gpu, &mut tui, "ab", "the Backspace undone alone");
    send_undo(&mut gpu);
    both_see(&mut gpu, &mut tui, "", "the typed run undone");
}

/// The knob at zero on the GPU route: `undo.amalgamate = 0` makes
/// every keystroke its own step, and a keystroke's command stays
/// whole. The GPU types `a(` optimistically and the daemon's hook
/// closes the pair; `End`, then Backspace takes the closer. The first
/// undo restores the closer alone (a command chord after zero-limit
/// typing is its own step, as after a run); the second removes `(`
/// with the closer it came with, one step; the third removes `a`.
/// Before the fix round the arbiter opened no group at zero, so the
/// opener and the closer stood as two steps, and the settle path
/// registered no run at zero, so a following command chord joined the
/// group left open.
#[test]
fn amalgamate_zero_keeps_a_keystrokes_command_whole_on_the_gpu_route() {
    let daemon = TestDaemon::spawn_with_config("pmacs.config.set('undo.amalgamate', 0)\n");
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_optimistic(&mut gpu, 0, "a(");
    both_see(&mut gpu, &mut tui, "a()", "a( with its closer");
    send_key(&mut gpu, Key::End, Modifiers::NONE);
    send_key(&mut gpu, Key::Backspace, Modifiers::NONE);
    both_see(&mut gpu, &mut tui, "a(", "the closer deleted");

    send_undo(&mut gpu);
    both_see(&mut gpu, &mut tui, "a()", "the Backspace undone alone");
    send_undo(&mut gpu);
    both_see(
        &mut gpu,
        &mut tui,
        "a",
        "the opener and its closer, one step",
    );
    send_undo(&mut gpu);
    both_see(&mut gpu, &mut tui, "", "a, its own step");
    send_undo(&mut gpu);
    assert_text_stays(&mut gpu, "", Duration::from_millis(500));
}

/// Undo, type, undo again, on the GPU route: `ab`, undo, `cd`, undo
/// leaves the buffer empty at each undo; and the forward edit cleared
/// the redo, so `C-x r` after it restores nothing.
#[test]
fn undo_type_undo_again_on_the_gpu_route() {
    let daemon = TestDaemon::spawn();
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_optimistic(&mut gpu, 0, "ab");
    both_see(&mut gpu, &mut tui, "ab", "ab");
    send_undo(&mut gpu);
    both_see(&mut gpu, &mut tui, "", "empty after the first undo");
    type_optimistic(&mut gpu, 0, "cd");
    both_see(&mut gpu, &mut tui, "cd", "cd");
    send_key(&mut gpu, Key::Char('x'), Modifiers::CTRL);
    send_key(&mut gpu, Key::Char('r'), Modifiers::NONE);
    assert_text_stays(&mut gpu, "cd", Duration::from_millis(500));
    send_undo(&mut gpu);
    both_see(&mut gpu, &mut tui, "", "empty after the second undo");
}

/// A `frontend.detached` hook that logs the departed id into the
/// daemon's isolated home, so a test can wait for a detach to have
/// been handled --- the hook runs last in the daemon's detach arm ---
/// without editing any buffer.
const DETACH_LOG_CONFIG: &str = "\
pmacs.hook.add('frontend.detached', function(fid)
  local f = assert(io.open(os.getenv('HOME') .. '/detached.log', 'a'))
  f:write(tostring(fid), '\\n')
  f:close()
end)
";

/// The same, also inserting `X` at the start of `*scratch*`: a daemon
/// edit, nobody's, landing after the detach arm's own work.
const DETACH_LOG_AND_INSERT_CONFIG: &str = "\
pmacs.hook.add('frontend.detached', function(fid)
  local f = assert(io.open(os.getenv('HOME') .. '/detached.log', 'a'))
  f:write(tostring(fid), '\\n')
  f:close()
  for _, id in ipairs(pmacs.buffer.list()) do
    local ok, described = pcall(pmacs.describe.buffer, id)
    if ok and described and described.name == '*scratch*' then
      id:insert(0, 'X')
    end
  end
end)
";

/// Drop `replica` and wait until the daemon has run the detach hook
/// for it, which is after the detach arm handed its undo history on.
fn detach_and_wait(daemon: &TestDaemon, replica: Replica) {
    let fid = replica.fid;
    drop(replica);
    let log = daemon
        .socket_path()
        .parent()
        .expect("socket parent")
        .join("detached.log");
    let seen = || std::fs::read_to_string(&log).unwrap_or_default();
    common::ready::expect_true(
        &format!("the daemon's detach hook for frontend {}", fid.0),
        Duration::from_secs(5),
        || seen().lines().any(|line| line == fid.0.to_string()),
        || format!("detached.log: {:?}", seen()),
    );
}

/// A departed user's word goes to whoever undoes next, when it is the
/// newest thing and not before: the GPU types `hello`, the TUI types
/// ` world` after it, the GPU detaches. The TUI's first undo takes
/// its own ` world`, the newer group; its second takes `hello`, now
/// the newest unowned group, as it would take a script's insert; a
/// third finds nothing. Before the fix round the detach arm left the
/// GPU's stack under its id, reachable by nobody, and `hello` stayed.
#[test]
fn a_departed_users_word_goes_to_whoever_undoes_next_when_newest() {
    let daemon = TestDaemon::spawn_with_config(DETACH_LOG_CONFIG);
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_optimistic(&mut gpu, 0, "hello");
    both_see(&mut gpu, &mut tui, "hello", "hello");
    send_key(&mut tui, Key::End, Modifiers::NONE);
    type_keys(&mut tui, " world");
    both_see(&mut gpu, &mut tui, "hello world", "hello world");
    detach_and_wait(&daemon, gpu);

    send_undo(&mut tui);
    pump_until_text(&mut tui, "hello", "the tui's own world, the newest");
    send_undo(&mut tui);
    pump_until_text(&mut tui, "", "the departed gpu's hello, next newest");
    send_undo(&mut tui);
    assert_text_stays(&mut tui, "", Duration::from_millis(500));
}

/// A departed user's word is taken at once when it is the newest: the
/// GPU types `hello` and detaches; the TUI, which typed nothing,
/// undoes and `hello` goes.
#[test]
fn a_departed_users_word_is_taken_at_once_when_it_is_the_newest() {
    let daemon = TestDaemon::spawn_with_config(DETACH_LOG_CONFIG);
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_optimistic(&mut gpu, 0, "hello");
    both_see(&mut gpu, &mut tui, "hello", "hello");
    detach_and_wait(&daemon, gpu);

    send_undo(&mut tui);
    pump_until_text(&mut tui, "", "the departed gpu's hello");
    send_undo(&mut tui);
    assert_text_stays(&mut tui, "", Duration::from_millis(500));
}

/// A departed user's groups keep their bases: the GPU types `ab`; a
/// script inserts `X` in front of it (a passerby's detach hook does),
/// so the GPU's group now carries that foreign insert in its base;
/// the GPU detaches and the same hook inserts a second `X`. The TUI
/// undoes three times: the newer `X`, the older `X`, then `ab` ---
/// whole, at its own position. Handing the groups to the daemon's
/// stack instead would leave the older `X` in `ab`'s base after that
/// `X` had been undone from the same stack, and the third undo would
/// miss.
#[test]
fn a_departed_users_groups_keep_their_bases_beside_script_edits() {
    let daemon = TestDaemon::spawn_with_config(DETACH_LOG_AND_INSERT_CONFIG);
    let mut gpu = attach_replica(&daemon);
    let mut tui = attach_replica(&daemon);

    type_optimistic(&mut gpu, 0, "ab");
    both_see(&mut gpu, &mut tui, "ab", "ab");
    let passerby = attach_replica(&daemon);
    detach_and_wait(&daemon, passerby);
    both_see(
        &mut gpu,
        &mut tui,
        "Xab",
        "the passerby's detach inserted X",
    );
    detach_and_wait(&daemon, gpu);
    pump_until_text(&mut tui, "XXab", "the gpu's detach inserted a second X");

    send_undo(&mut tui);
    pump_until_text(&mut tui, "Xab", "the newer X");
    send_undo(&mut tui);
    pump_until_text(&mut tui, "ab", "the older X");
    send_undo(&mut tui);
    pump_until_text(&mut tui, "", "the departed gpu's ab, whole");
    send_undo(&mut tui);
    assert_text_stays(&mut tui, "", Duration::from_millis(500));
}

/// The phase's acceptance through the production GPU dispatch: a real
/// `pmacs-gpu --headless-probe` types `hello` as optimistic ops
/// through `App::apply_keyboard`, presses `C-x` and, once the daemon
/// reports the pending prefix, `U` shifted --- the owner's keystroke,
/// which the dispatcher lowers to `C-x u` --- and the mirror's text
/// loses `hello` when the daemon's compensation arrives. Skips without
/// the binary or an adapter unless `PMACS_REQUIRE_GPU` is set.
#[test]
fn hello_on_the_gpu_then_c_x_shift_u_and_hello_goes() {
    use std::path::{Path, PathBuf};

    fn gpu_binary() -> PathBuf {
        Path::new(env!("CARGO_BIN_EXE_pmacs"))
            .parent()
            .expect("test binary directory")
            .join("pmacs-gpu")
    }

    let required = std::env::var_os("PMACS_REQUIRE_GPU").is_some();
    let binary = gpu_binary();
    if !binary.exists() {
        assert!(
            !required,
            "PMACS_REQUIRE_GPU is set but {} is not built; build the workspace first",
            binary.display()
        );
        eprintln!(
            "skipping the GPU undo probe: {} is not built",
            binary.display()
        );
        return;
    }
    let daemon = TestDaemon::spawn_with_env(&[
        ("PMACS_INSTANCE_SEMANTIC_RENDER", "1"),
        ("PMACS_INSTANCE_MULTI_FRONTEND", "1"),
    ]);
    let report = daemon
        .socket_path()
        .parent()
        .expect("socket parent")
        .join("gpu-undo.txt");
    let output = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(daemon.socket_path())
        .arg(&report)
        .env("PMACS_GPU_PROBE_TYPE_TEXT", "hello")
        .env("PMACS_GPU_PROBE_ACTION", "undo")
        .output()
        .expect("run the headless GPU undo probe");
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let no_adapter = output.status.code() == Some(3);
        assert!(
            no_adapter && !required,
            "headless GPU undo probe failed (status {:?}):\n{stderr}",
            output.status.code()
        );
        eprintln!("skipping the GPU undo probe: no wgpu adapter available");
        return;
    }
    let text = std::fs::read_to_string(&report).expect("probe report");
    eprintln!("--- probe report ---\n{text}--- end ---");
    let facts: std::collections::HashMap<String, String> = text
        .lines()
        .filter_map(|line| {
            let (k, v) = line.split_once('=')?;
            Some((k.to_owned(), v.to_owned()))
        })
        .collect();
    let fact = |key: &str| -> &str {
        facts
            .get(key)
            .map_or_else(|| panic!("report carries no {key}"), String::as_str)
    };
    assert_eq!(fact("disconnect"), "", "the probe stayed attached");
    assert!(
        fact("text_after_typing").contains("hello"),
        "the probe typed hello optimistically: {}",
        fact("text_after_typing")
    );
    assert_ne!(
        fact("prefix_pending_at_ms"),
        "none",
        "the daemon reported the C-x prefix pending before U was pressed"
    );
    assert_eq!(
        fact("undone"),
        "true",
        "hello did not go: {}",
        fact("text_after_undo")
    );
    assert!(
        !fact("text_after_undo").contains("hello"),
        "hello still in the mirror after C-x U: {}",
        fact("text_after_undo")
    );
}
