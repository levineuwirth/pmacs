//! Shared Stage 1 terminal registry, lifecycle, and read-only acceptance.

use std::time::{Duration, Instant};

use pmacs::ansi::AnsiEvent;
use pmacs::buffer::{Buffer, BufferError, BufferId, EditOp};
use pmacs::cell::{CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::process::ProcessState;
use pmacs::rope::Range;
use pmacs::terminal::screen::TerminalScreen;
use pmacs::terminal::{TerminalProcessState, TerminalSpec};

fn rope_bytes(buffer: &Buffer) -> Vec<u8> {
    let mut bytes = vec![0; buffer.len() as usize];
    if !bytes.is_empty() {
        buffer.snapshot_rope().slice(0, buffer.len(), &mut bytes);
    }
    bytes
}

fn screen_text(snapshot: &pmacs::terminal::TerminalSnapshot) -> String {
    let mut text = String::new();
    for (index, cell) in snapshot.cells.iter().enumerate() {
        if index > 0 && index % snapshot.size.cols as usize == 0 {
            text.push('\n');
        }
        match &cell.glyph {
            Glyph::Char(ch) => text.push(*ch),
            Glyph::Cluster(bytes) => text.push_str(&String::from_utf8_lossy(bytes)),
            Glyph::Continuation => {}
        }
    }
    text
}

fn tick_until(
    state: &mut EditorState,
    timeout: Duration,
    mut done: impl FnMut(&EditorState) -> bool,
) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        state.tick_processes();
        if done(state) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("terminal condition did not settle before {timeout:?}");
}

#[test]
fn terminal_cells_reject_child_control_characters() {
    let mut screen = TerminalScreen::new(CellSize::new(2, 4), 0).expect("valid screen");
    let before = screen.snapshot();
    screen.apply_event(AnsiEvent::Text("\u{9b}\n\0".into()));
    assert_eq!(screen.snapshot(), before);
}

#[test]
fn spawn_failure_is_transactional() {
    let mut state = EditorState::new_with_roots(&crate::iso::roots());
    let buffers_before = state.core.borrow().registry.borrow().len();
    let processes_before = state.process_supervisor.borrow().ids().count();
    state.process_supervisor.borrow_mut().shutdown();
    let result = state.open_terminal(TerminalSpec::new("/bin/sh"));

    assert!(result.is_err());
    assert_eq!(state.core.borrow().registry.borrow().len(), buffers_before);
    assert_eq!(state.terminal_manager.borrow().len(), 0);
    assert_eq!(
        state.process_supervisor.borrow().ids().count(),
        processes_before
    );
}

#[test]
fn strict_owned_spec_rejects_before_spawn_and_is_mutation_independent() {
    let mut state = EditorState::new_with_roots(&crate::iso::roots());
    let buffers_before = state.core.borrow().registry.borrow().len();
    let mut invalid = TerminalSpec::new("/bin/sh");
    invalid.rows = 0;
    assert!(state.open_terminal(invalid).is_err());
    assert_eq!(state.core.borrow().registry.borrow().len(), buffers_before);
    assert!(state.terminal_manager.borrow().is_empty());
    assert_eq!(state.process_supervisor.borrow().ids().count(), 0);

    let mut spec = TerminalSpec::new("/bin/sh");
    spec.args = vec!["-c".into(), "sleep 30".into()];
    spec.env = vec![("PMACS_VTERM_OWNED".into(), "original".into())];
    let mut caller_copy = spec.clone();
    let buffer_id = state.open_terminal(spec).expect("valid owned spec");
    caller_copy.command.clear();
    caller_copy.args.clear();
    caller_copy.env[0].1 = "mutated".into();
    let lua_processes: usize = state
        .lua_host
        .lua()
        .load("return #pmacs.process.list()")
        .eval()
        .expect("process list");
    assert_eq!(
        lua_processes, 0,
        "terminal-owned ProcessId must not be exposed through pmacs.process"
    );

    let process_id = state
        .terminal_manager
        .borrow()
        .process_id(buffer_id)
        .expect("terminal process");
    let supervisor = state.process_supervisor.borrow();
    let process_spec = supervisor.spec(process_id).expect("owned process spec");
    assert_eq!(process_spec.command, "/bin/sh");
    assert_eq!(
        process_spec.args,
        [String::from("-c"), String::from("sleep 30")]
    );
    assert_eq!(
        process_spec.env,
        [
            (String::from("PMACS_VTERM_OWNED"), String::from("original")),
            (String::from("TERM"), String::from("xterm-256color")),
        ]
    );
}

#[test]
fn read_only_guard_covers_direct_skip_undo_and_redo_without_state_change() {
    let mut buffer = Buffer::from_bytes(BufferId::next(), "*protected*", b"abc");
    buffer
        .apply_edit(EditOp::Insert {
            pos: 3,
            bytes: b"d",
        })
        .expect("seed undo");
    buffer.undo().expect("seed redo");
    buffer.set_read_only(true);
    let before = (rope_bytes(&buffer), buffer.revision(), buffer.is_modified());
    assert!(matches!(
        buffer.begin_edit(),
        Err(BufferError::ReadOnly { .. })
    ));
    assert!(!buffer.editing_in_progress());

    let results = [
        buffer.apply_edit(EditOp::Insert {
            pos: 0,
            bytes: b"x",
        }),
        buffer.apply_edit_skip_intercepts(EditOp::Replace {
            range: Range::new(0, 1),
            bytes: b"y",
        }),
        buffer.undo(),
        buffer.redo(),
    ];

    assert!(
        results
            .iter()
            .all(|result| matches!(result, Err(BufferError::ReadOnly { .. })))
    );
    assert_eq!(
        before,
        (rope_bytes(&buffer), buffer.revision(), buffer.is_modified())
    );
}

#[cfg(feature = "crdt")]
#[test]
fn read_only_empty_crdt_bootstrap_is_immutable_against_remote_content() {
    let mut buffer = Buffer::new(BufferId::next(), "*terminal*");
    buffer.set_read_only(true);
    buffer
        .upgrade_to_crdt(1)
        .expect("empty immutable bootstrap is allowed");

    let donor = pmacs::crdt::CrdtState::new(2).expect("donor");
    let version = donor.version();
    donor.insert(0, "forged").expect("donor edit");
    let update = donor.export_updates_since(&version).expect("update");

    assert!(matches!(
        buffer.apply_remote_crdt_op(&update),
        Err(BufferError::ReadOnly { .. })
    ));
    assert!(buffer.is_empty());
    assert_eq!(buffer.revision(), 0);
    assert!(!buffer.is_modified());
}

#[test]
fn final_output_precedes_exact_nonzero_annotation_and_buffer_is_retained() {
    let mut state = EditorState::new_with_roots(&crate::iso::roots());
    let mut spec = TerminalSpec::new("/bin/sh");
    spec.args = vec![
        "-c".into(),
        concat!(
            "printf 'main-home'; ",
            "printf '\\033'; sleep 0.03; printf '[?1049h'; ",
            "printf '\\033[2;'; sleep 0.03; printf '4HALT'; ",
            "IFS= read -r gate; ",
            "printf '\\033[?1049l'; ",
            "printf '\\033[2;'; sleep 0.03; printf '3Hfinal-'; ",
            "sleep 0.03; printf 'output'; exit 7"
        )
        .into(),
    ];
    spec.rows = 8;
    spec.cols = 80;
    let buffer_id = state.open_terminal(spec).expect("open terminal");

    tick_until(&mut state, Duration::from_secs(5), |state| {
        state
            .terminal_manager
            .borrow()
            .snapshot(buffer_id)
            .is_some_and(|snapshot| {
                matches!(snapshot.process, TerminalProcessState::Running)
                    && screen_text(&snapshot)
                        .lines()
                        .nth(1)
                        .is_some_and(|row| row.starts_with("   ALT"))
            })
    });
    let alternate = state
        .terminal_manager
        .borrow()
        .snapshot(buffer_id)
        .expect("running alternate-screen snapshot");
    let alternate_text = screen_text(&alternate);
    assert!(alternate_text.contains("ALT"));
    assert!(
        !alternate_text.contains("main-home"),
        "alternate screen must not expose the preserved main grid"
    );
    {
        let manager = state.terminal_manager.borrow();
        let mut supervisor = state.process_supervisor.borrow_mut();
        manager
            .send(buffer_id, b"\n", &mut supervisor)
            .expect("raw stdin unblocks child");
    }

    tick_until(&mut state, Duration::from_secs(5), |state| {
        state
            .terminal_manager
            .borrow()
            .snapshot(buffer_id)
            .is_some_and(|snapshot| matches!(snapshot.process, TerminalProcessState::Exited(7)))
    });

    let snapshot = state
        .terminal_manager
        .borrow()
        .snapshot(buffer_id)
        .expect("retained terminal snapshot");
    let text = screen_text(&snapshot);
    assert!(
        text.contains("main-home"),
        "leaving alternate screen must restore the main grid"
    );
    assert!(
        !text.contains("ALT"),
        "alternate-screen output must not enter the retained main grid"
    );
    assert!(
        text.lines()
            .nth(1)
            .is_some_and(|row| row.starts_with("  final-output")),
        "FullScreen parser/profile must honor CSI cursor addressing"
    );
    let output_at = text
        .find("final-output")
        .expect("final child output visible");
    let annotation = format!("Process {} exited abnormally with code 7", snapshot.pid);
    let annotation_at = text
        .find(&annotation)
        .expect("exact exit annotation visible");
    assert!(
        output_at < annotation_at,
        "final output must precede annotation"
    );
    assert!(state.core.borrow().registry.borrow().contains(buffer_id));
    let core = state.core.borrow();
    let registry = core.registry.borrow();
    let buffer = registry.get(buffer_id).expect("identity buffer retained");
    assert!(buffer.is_read_only());
    assert!(buffer.is_empty());
    assert!(!buffer.is_modified());
}

#[test]
fn normal_and_signal_annotations_use_exact_pid_and_outcome() {
    for (script, expected, annotation_tail) in [
        (
            "printf normal-output; exit 0",
            TerminalProcessState::Exited(0),
            "exited normally with code 0",
        ),
        (
            "printf signal-output; kill -TERM $$",
            TerminalProcessState::Signaled("SIGTERM".into()),
            "exited abnormally with signal SIGTERM",
        ),
    ] {
        let mut state = EditorState::new_with_roots(&crate::iso::roots());
        let mut spec = TerminalSpec::new("/bin/sh");
        spec.args = vec!["-c".into(), script.into()];
        spec.rows = 6;
        spec.cols = 80;
        let buffer_id = state.open_terminal(spec).expect("open terminal");
        tick_until(&mut state, Duration::from_secs(5), |state| {
            state
                .terminal_manager
                .borrow()
                .snapshot(buffer_id)
                .is_some_and(|snapshot| snapshot.process == expected)
        });
        let snapshot = state
            .terminal_manager
            .borrow()
            .snapshot(buffer_id)
            .expect("snapshot retained");
        assert!(
            screen_text(&snapshot).contains(&format!("Process {} {annotation_tail}", snapshot.pid)),
            "missing exact annotation for {:?}",
            snapshot.process
        );
    }
}

#[test]
fn killing_terminal_buffer_prunes_session_and_reaps_owned_process() {
    let mut state = EditorState::new_with_roots(&crate::iso::roots());
    let mut spec = TerminalSpec::new("/bin/sh");
    spec.args = vec!["-c".into(), "sleep 30".into()];
    let buffer_id = state.open_terminal(spec).expect("open terminal");
    let process_id = state
        .terminal_manager
        .borrow()
        .process_id(buffer_id)
        .expect("owned process");

    state
        .core
        .borrow_mut()
        .kill_buffer(buffer_id)
        .expect("kill identity buffer");
    state.tick_processes();
    assert!(!state.terminal_manager.borrow().is_terminal(buffer_id));

    tick_until(&mut state, Duration::from_secs(5), |state| {
        state
            .process_supervisor
            .borrow()
            .state(process_id)
            .is_none()
    });
}

#[test]
fn editor_shutdown_kills_term_ignoring_terminal_child() {
    let pid = {
        let mut state = EditorState::new_with_roots(&crate::iso::roots());
        state
            .process_supervisor
            .borrow_mut()
            .set_grace_period(Duration::from_millis(50));
        let mut spec = TerminalSpec::new("/bin/sh");
        spec.args = vec![
            "-c".into(),
            "trap '' TERM; while :; do sleep 1; done".into(),
        ];
        let buffer_id = state.open_terminal(spec).expect("open terminal");
        state
            .terminal_manager
            .borrow()
            .snapshot(buffer_id)
            .expect("snapshot")
            .pid
    };

    let pid = nix::unistd::Pid::from_raw(i32::try_from(pid).expect("pid fits i32"));
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && nix::sys::signal::kill(pid, None).is_ok() {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        nix::sys::signal::kill(pid, None),
        Err(nix::errno::Errno::ESRCH),
        "terminal child {pid} survived EditorState shutdown"
    );
}

#[test]
fn terminal_tick_does_not_take_non_terminal_process_events() {
    let mut state = EditorState::new_with_roots(&crate::iso::roots());
    let mut process = pmacs::process::ProcessSpec::new("ordinary", "/bin/sh");
    process.args = vec!["-c".into(), "printf ordinary".into()];
    let ordinary_id = state
        .process_supervisor
        .borrow_mut()
        .spawn(process)
        .expect("ordinary process");

    tick_until(&mut state, Duration::from_secs(5), |state| {
        matches!(
            state.process_supervisor.borrow().state(ordinary_id),
            Some(ProcessState::Terminated(_))
        )
    });
    let events = state
        .process_supervisor
        .borrow_mut()
        .take_events(ordinary_id);
    assert!(
        events.iter().any(|event| matches!(
            &event.kind,
            pmacs::process::ProcessEventKind::Stdout(bytes) if bytes == b"ordinary"
        )),
        "TerminalManager must not steal ordinary process output"
    );
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
