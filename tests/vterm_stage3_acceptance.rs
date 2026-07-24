//! Stage 3 terminal/GPU protocol acceptance (criteria 28–37).
//!
//! Focused wire and paint assertions live in `pmacs-protocol` and
//! `pmacs-gpu`; this suite owns the cross-surface behavior — the
//! semantic producer's baselines, the authenticated per-view routing,
//! and the real-daemon path.

mod common;

use std::thread;
use std::time::{Duration, Instant};

use pmacs::cell::{CellCoord, CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::protocol::{
    FrontendId, InstanceMessage, Modifiers, MouseButton, MouseKind, TerminalFrame,
};
use pmacs::semantic_render::SemanticRenderState;
use pmacs::terminal::{TerminalProcessState, TerminalSpec, TerminalViewKey};
use pmacs::window::{FrontendView, Layout, Window, WindowId};

fn tick_until(
    state: &mut EditorState,
    timeout: Duration,
    mut done: impl FnMut(&EditorState) -> bool,
) {
    let deadline = Instant::now() + timeout;
    loop {
        state.tick_processes();
        if done(state) {
            return;
        }
        assert!(Instant::now() < deadline, "terminal condition timed out");
        thread::sleep(Duration::from_millis(10));
    }
}

fn snapshot_text(snapshot: &pmacs::terminal::TerminalSnapshot) -> String {
    let mut text = String::new();
    for cell in &snapshot.cells {
        match &cell.glyph {
            Glyph::Char(ch) => text.push(*ch),
            Glyph::Cluster(bytes) => text.push_str(&String::from_utf8_lossy(bytes)),
            Glyph::Continuation => {}
        }
    }
    text
}

fn frame_text(frame: &TerminalFrame) -> String {
    let mut text = String::new();
    for cell in &frame.cells {
        match &cell.glyph {
            Glyph::Char(ch) => text.push(*ch),
            Glyph::Cluster(bytes) => text.push_str(&String::from_utf8_lossy(bytes)),
            Glyph::Continuation => {}
        }
    }
    text
}

/// Give `frontend_id` its own window showing `buffer_id`, the way the
/// daemon's session bootstrap and buffer-follow do.
fn attach_view(
    state: &EditorState,
    frontend_id: FrontendId,
    buffer_id: pmacs::buffer::BufferId,
) -> WindowId {
    let mut core = state.core.borrow_mut();
    let text_view = {
        let registry = core.registry.clone();
        let registry = registry.borrow();
        let buffer = registry.get(buffer_id).expect("buffer present");
        pmacs::text_view::TextView::new(buffer)
    };
    let window_id = WindowId::next();
    let window = Window::new(window_id, buffer_id, text_view);
    core.windows.insert(window_id, window);
    core.register_frontend_view(
        frontend_id,
        FrontendView {
            layout: Layout::single(window_id),
            active: window_id,
            fold_projection: true,
        },
    );
    window_id
}

/// Point an already-registered view at another buffer, the way a
/// daemon-side buffer switch does.
fn switch_view(state: &EditorState, frontend_id: FrontendId, buffer_id: pmacs::buffer::BufferId) {
    let mut core = state.core.borrow_mut();
    let window_id = core.views.get(&frontend_id).expect("view present").active;
    let text_view = {
        let registry = core.registry.clone();
        let registry = registry.borrow();
        let buffer = registry.get(buffer_id).expect("buffer present");
        pmacs::text_view::TextView::new(buffer)
    };
    let window = core.windows.get_mut(&window_id).expect("window present");
    window.buffer_id = buffer_id;
    window.text_view = text_view;
    window.cursor = 0;
    window.selection = None;
}

fn open_terminal(
    state: &mut EditorState,
    args: &str,
    rows: u16,
    cols: u16,
) -> pmacs::buffer::BufferId {
    let mut spec = TerminalSpec::new("/bin/sh");
    spec.args = vec!["-c".into(), args.into()];
    spec.rows = rows;
    spec.cols = cols;
    state
        .terminal_manager
        .borrow_mut()
        .open(
            spec,
            &mut state.core.borrow_mut(),
            &mut state.process_supervisor.borrow_mut(),
        )
        .expect("open terminal")
}

fn terminal_frames(messages: &[InstanceMessage]) -> Vec<TerminalFrame> {
    messages
        .iter()
        .filter_map(|msg| match msg {
            InstanceMessage::TerminalFrame(frame) => Some(frame.clone()),
            _ => None,
        })
        .collect()
}

fn one_frame(messages: &[InstanceMessage]) -> TerminalFrame {
    let frames = terminal_frames(messages);
    assert_eq!(frames.len(), 1, "expected exactly one terminal frame");
    frames.into_iter().next().expect("checked length")
}

/// Acceptance 30: the first activation emits one authoritative complete
/// frame, an equal payload is silent, view-only and process-only changes
/// emit despite an unchanged screen generation, the document projection
/// is suppressed while terminal, and switching back resyncs the
/// document in full.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one producer-baseline lifecycle scenario"
)]
fn a30_first_frame_is_authoritative_then_only_real_changes_emit() {
    let mut state = EditorState::new();
    let frontend_id = FrontendId(31);
    let terminal_buffer = open_terminal(
        &mut state,
        "printf 'alpha\\nbeta\\n'; i=0; while [ $i -lt 20 ]; do printf 'row%02d\\n' \"$i\"; \
         i=$((i+1)); done; sleep 30",
        6,
        24,
    );
    tick_until(&mut state, Duration::from_secs(5), |state| {
        state
            .terminal_manager
            .borrow()
            .snapshot(terminal_buffer)
            .is_some_and(|snapshot| snapshot_text(&snapshot).contains("row19"))
    });
    attach_view(&state, frontend_id, terminal_buffer);

    let mut producer = SemanticRenderState::for_peer(frontend_id, 19);
    let size = CellSize::new(6, 24);
    producer.set_terminal_viewport(terminal_buffer, size);

    // First activation: exactly one complete frame, and no document
    // family at all.
    let first = producer.render_frame(&state);
    let frame = one_frame(&first);
    assert_eq!(frame.buffer_id, terminal_buffer);
    assert_eq!(frame.size, size);
    assert_eq!(frame.cells.len(), (size.rows * size.cols) as usize);
    assert_eq!(frame.validate(), Ok(()));
    assert!(frame_text(&frame).contains("row19"));
    assert!(
        producer.in_terminal_mode(),
        "a projected terminal puts the producer in terminal mode"
    );
    for msg in &first {
        assert!(
            !matches!(
                msg,
                InstanceMessage::StyleSpans { .. }
                    | InstanceMessage::Decorations { .. }
                    | InstanceMessage::InlineAdornments { .. }
                    | InstanceMessage::FileStyleSummary { .. }
                    | InstanceMessage::LineNumbers { .. }
                    | InstanceMessage::SearchPrompt { .. }
                    | InstanceMessage::CompletionPopup { .. }
            ),
            "terminal mode must suppress the document projection, saw {msg:?}"
        );
    }

    // Nothing changed: a completely equal payload is silent.
    let second = producer.render_frame(&state);
    assert!(
        terminal_frames(&second).is_empty(),
        "an unchanged terminal payload must not re-send"
    );

    // A view-only change — scrolling this frontend's own view — leaves
    // `screen_generation` alone but must still reach the frontend.
    let key = {
        let core = state.core.borrow();
        let window_id = core.views.get(&frontend_id).expect("view").active;
        TerminalViewKey::new(frontend_id, window_id, terminal_buffer)
    };
    let generation_before = frame.screen_generation;
    assert!(
        state
            .terminal_manager
            .borrow_mut()
            .scroll_view(key, size, 3),
        "scrolling the view back into history"
    );
    let scrolled = one_frame(&producer.render_frame(&state));
    assert_eq!(
        scrolled.screen_generation, generation_before,
        "scrolling does not advance the screen generation"
    );
    assert_eq!(scrolled.scroll_offset, 3);
    assert!(!scrolled.at_bottom);

    // A selection is also view-only.
    assert!(
        state
            .terminal_manager
            .borrow_mut()
            .begin_selection(key, size, CellCoord::new(0, 0))
    );
    assert!(
        state
            .terminal_manager
            .borrow_mut()
            .finish_selection(key, size, CellCoord::new(0, 4))
    );
    let selected = one_frame(&producer.render_frame(&state));
    assert!(
        !selected.selection.is_empty(),
        "a selection change must reach the frontend"
    );
    assert_eq!(selected.screen_generation, generation_before);

    // A process-only change: the child exits. The frame carries the new
    // outcome even though the frontend has not scrolled or typed.
    state
        .terminal_manager
        .borrow_mut()
        .terminate(terminal_buffer, &mut state.process_supervisor.borrow_mut())
        .expect("terminate child");
    let mut exited = None;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        state.tick_processes();
        for frame in terminal_frames(&producer.render_frame(&state)) {
            if !matches!(frame.process, TerminalProcessState::Running) {
                exited = Some(frame);
                break;
            }
        }
        if exited.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let exited = exited.expect("a process-state change must produce a frame");
    assert_ne!(exited.process, TerminalProcessState::Running);

    // Switching back to a document: the snapshot reset drops the
    // terminal declaration and baseline, and the producer leaves
    // terminal mode so the document resync can happen.
    let document = state.core.borrow().active_buffer_id();
    switch_view(&state, frontend_id, document);
    producer.on_buffer_snapshot_sent(document);
    assert!(
        producer.terminal_viewport().is_none(),
        "a snapshot clears the terminal declaration"
    );
    producer.set_viewport(document, pmacs::protocol::ByteRange { start: 0, end: 0 }, 0);
    let back = producer.render_frame(&state);
    assert!(
        terminal_frames(&back).is_empty(),
        "a document buffer produces no terminal frames"
    );
    assert!(
        !producer.in_terminal_mode(),
        "the producer leaves terminal mode when the window switches away"
    );
}

/// Acceptance 31: two semantic frontends over one terminal keep
/// independent sizes, scroll, selection, and baselines while sharing one
/// process and screen; a passive declaration produces its own
/// clipped/padded frame but only the durable controller resizes the PTY.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one shared-session two-frontend scenario"
)]
fn a31_two_semantic_frontends_share_one_session_with_independent_views() {
    let mut state = EditorState::new();
    let first_id = FrontendId(41);
    let second_id = FrontendId(42);
    let terminal_buffer = open_terminal(
        &mut state,
        "i=0; while [ $i -lt 30 ]; do printf 'row%02d\\n' \"$i\"; i=$((i+1)); done; sleep 30",
        8,
        40,
    );
    tick_until(&mut state, Duration::from_secs(5), |state| {
        state
            .terminal_manager
            .borrow()
            .snapshot(terminal_buffer)
            .is_some_and(|snapshot| snapshot_text(&snapshot).contains("row29"))
    });
    let first_window = attach_view(&state, first_id, terminal_buffer);
    let second_window = attach_view(&state, second_id, terminal_buffer);

    let first_size = CellSize::new(6, 30);
    let second_size = CellSize::new(4, 20);
    let mut first = SemanticRenderState::for_peer(first_id, 19);
    let mut second = SemanticRenderState::for_peer(second_id, 19);

    // The first frontend declares and becomes the controller by acting
    // on its view; the second declares only.
    assert!(state.semantic_terminal_declaration_is_active(first_id, terminal_buffer));
    first.set_terminal_viewport(terminal_buffer, first_size);
    second.set_terminal_viewport(terminal_buffer, second_size);

    let first_key = TerminalViewKey::new(first_id, first_window, terminal_buffer);
    state.terminal_manager.borrow_mut().register_view(first_key);
    assert!(
        state
            .terminal_manager
            .borrow_mut()
            .claim_controller(first_key)
    );

    let screen_before = state
        .terminal_manager
        .borrow()
        .snapshot(terminal_buffer)
        .expect("snapshot")
        .size;

    // A PASSIVE declaration records geometry and produces the passive
    // frontend's own projection, but never touches the shared screen.
    assert!(!state.sync_semantic_terminal_layout(second_id, terminal_buffer, second_size));
    assert_eq!(
        state
            .terminal_manager
            .borrow()
            .snapshot(terminal_buffer)
            .expect("snapshot")
            .size,
        screen_before,
        "a passive declaration must not resize the shared PTY"
    );

    // The CONTROLLER's declaration does resize it.
    assert!(state.sync_semantic_terminal_layout(first_id, terminal_buffer, first_size));
    assert_eq!(
        state
            .terminal_manager
            .borrow()
            .snapshot(terminal_buffer)
            .expect("snapshot")
            .size,
        first_size,
        "the durable controller owns the shared geometry"
    );

    let first_frame = one_frame(&first.render_frame(&state));
    let second_frame = one_frame(&second.render_frame(&state));
    assert_eq!(first_frame.size, first_size);
    assert_eq!(second_frame.size, second_size);
    assert_eq!(
        first_frame.pid, second_frame.pid,
        "both views project one process"
    );
    assert_eq!(first_frame.process, second_frame.process);
    assert_eq!(
        first_frame.screen_generation,
        second_frame.screen_generation
    );

    // Scroll and select on the first view only.
    assert!(
        state
            .terminal_manager
            .borrow_mut()
            .scroll_view(first_key, first_size, 4)
    );
    let second_key = TerminalViewKey::new(second_id, second_window, terminal_buffer);
    assert!(state.terminal_manager.borrow_mut().begin_selection(
        first_key,
        first_size,
        CellCoord::new(0, 0)
    ));
    assert!(state.terminal_manager.borrow_mut().finish_selection(
        first_key,
        first_size,
        CellCoord::new(0, 5)
    ));

    let first_after = one_frame(&first.render_frame(&state));
    assert_eq!(first_after.scroll_offset, 4);
    assert!(!first_after.at_bottom);
    assert!(!first_after.selection.is_empty());

    // The second frontend's baseline is its own: its projection is
    // unchanged, so it emits nothing — and when it does emit, it carries
    // its own scroll and no selection.
    let second_after = second.render_frame(&state);
    for frame in terminal_frames(&second_after) {
        assert_eq!(frame.scroll_offset, 0, "scroll is per view");
        assert!(frame.selection.is_empty(), "selection is per view");
    }
    assert!(
        state
            .terminal_manager
            .borrow()
            .view_state(second_key)
            .is_some(),
        "the passive view remains registered"
    );

    state
        .terminal_manager
        .borrow_mut()
        .terminate(terminal_buffer, &mut state.process_supervisor.borrow_mut())
        .expect("terminate child");
}

/// Acceptance 32: forged frontend/buffer identities, stale buffers,
/// undeclared viewports, and out-of-bounds coordinates cannot affect
/// another view, the controller, terminal selection, the menu, the PTY
/// size, or child input.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one case per rejected identity or bound"
)]
fn a32_forged_stale_and_out_of_bounds_terminal_events_change_nothing() {
    let mut state = EditorState::new();
    let owner = FrontendId(51);
    let attacker = FrontendId(52);
    let terminal_buffer = open_terminal(&mut state, "sleep 30", 6, 20);
    let other_terminal = open_terminal(&mut state, "sleep 30", 6, 20);
    tick_until(&mut state, Duration::from_secs(5), |state| {
        state
            .terminal_manager
            .borrow()
            .snapshot(terminal_buffer)
            .is_some()
    });
    let owner_window = attach_view(&state, owner, terminal_buffer);
    let document = state.core.borrow().active_buffer_id();
    attach_view(&state, attacker, document);

    // Declare a size that differs from the spec's 6x20, so the
    // controller's resize is observable rather than suppressed as
    // unchanged.
    let size = CellSize::new(5, 18);
    let owner_key = TerminalViewKey::new(owner, owner_window, terminal_buffer);
    state.terminal_manager.borrow_mut().register_view(owner_key);
    assert!(
        state
            .terminal_manager
            .borrow_mut()
            .claim_controller(owner_key)
    );
    assert!(state.sync_semantic_terminal_layout(owner, terminal_buffer, size));
    let geometry_before = state
        .terminal_manager
        .borrow()
        .snapshot(terminal_buffer)
        .expect("snapshot")
        .size;
    assert_eq!(
        geometry_before, size,
        "the controller's declaration applied"
    );

    // The attacker's window shows a DOCUMENT, so naming the owner's
    // terminal buffer is a forgery: the declaration is refused outright.
    assert!(!state.semantic_terminal_declaration_is_active(attacker, terminal_buffer));
    assert!(!state.sync_semantic_terminal_layout(attacker, terminal_buffer, CellSize::new(2, 4)));
    assert_eq!(
        state
            .terminal_manager
            .borrow()
            .snapshot(terminal_buffer)
            .expect("snapshot")
            .size,
        geometry_before,
        "a forged declaration must not resize another frontend's session"
    );
    assert_eq!(
        state.terminal_manager.borrow().controller(terminal_buffer),
        Some(pmacs::terminal::TerminalController::from_view(owner_key)),
        "a forged declaration must not steal control"
    );

    // A pointer naming a buffer the sender is not displaying, and one
    // naming a terminal that exists but is not its active window, are
    // both dropped before any view mutation.
    assert!(!state.dispatch_semantic_terminal_pointer(
        attacker,
        terminal_buffer,
        CellCoord::new(0, 0),
        MouseKind::Down(MouseButton::Left),
        Modifiers::NONE,
    ));
    assert!(!state.dispatch_semantic_terminal_pointer(
        owner,
        other_terminal,
        CellCoord::new(0, 0),
        MouseKind::Down(MouseButton::Left),
        Modifiers::NONE,
    ));

    // An out-of-bounds coordinate against a DECLARED viewport is
    // dropped too — a cell the sender never saw is not a hit.
    assert!(!state.dispatch_semantic_terminal_pointer(
        owner,
        terminal_buffer,
        CellCoord::new(size.rows, 0),
        MouseKind::Down(MouseButton::Left),
        Modifiers::NONE,
    ));
    assert!(!state.dispatch_semantic_terminal_pointer(
        owner,
        terminal_buffer,
        CellCoord::new(0, size.cols),
        MouseKind::Down(MouseButton::Left),
        Modifiers::NONE,
    ));
    assert!(
        state
            .terminal_manager
            .borrow()
            .view_state(owner_key)
            .expect("owner view")
            .selection
            .is_none(),
        "a rejected pointer must not begin a selection"
    );
    assert!(
        !state.core.borrow().menu_is_open(),
        "a rejected pointer must not open the context menu"
    );

    // A frontend with no declared viewport cannot hit-test at all: the
    // coordinate has no geometry to be relative to.
    let fresh = FrontendId(53);
    attach_view(&state, fresh, terminal_buffer);
    assert!(!state.dispatch_semantic_terminal_pointer(
        fresh,
        terminal_buffer,
        CellCoord::new(0, 0),
        MouseKind::Down(MouseButton::Left),
        Modifiers::NONE,
    ));

    // The owner's own in-bounds gesture is accepted, proving the
    // rejections above are about identity and bounds, not a dead path.
    assert!(state.dispatch_semantic_terminal_pointer(
        owner,
        terminal_buffer,
        CellCoord::new(0, 0),
        MouseKind::Down(MouseButton::Left),
        Modifiers::NONE,
    ));

    for buffer in [terminal_buffer, other_terminal] {
        state
            .terminal_manager
            .borrow_mut()
            .terminate(buffer, &mut state.process_supervisor.borrow_mut())
            .expect("terminate child");
    }
}

/// The daemon config the Stage 3 real-path acceptance runs against: a
/// command that opens a controlled terminal child, bound to a key the
/// probe can press.
#[cfg(feature = "crdt")]
const PROBE_INIT_LUA: &str = r#"
pmacs.command.define {
  name = "vterm-probe.open",
  description = "Open the Stage 3 acceptance terminal child.",
  fn = function()
    return pmacs.terminal.open {
      command = "/bin/sh",
      args = { "-c",
        "i=0; while [ $i -lt 400 ]; do printf 'VTERMROW%02d\n' \"$i\"; i=$((i+1)); sleep 0.05; done" },
    }
  end,
}
-- C-M-t is deliberately an unbound chord: `bind` is strict and
-- refuses to shadow an existing binding, so a bound one (C-t is
-- transpose-chars) would fail init and leave the probe on a scratch.
pmacs.keymap.bind { scope = "global", sequence = "C-M-t", command = "vterm-probe.open" }
"#;

/// Acceptance 37: one path through a real daemon, a real PTY child, and
/// real headless wgpu rendering.
///
/// The GPU is a separate binary that deliberately depends only on
/// `pmacs-protocol`, so this drives it as a process: the daemon opens a
/// `/bin/sh` terminal from its own `init.lua`, and `pmacs-gpu
/// --headless-probe` attaches through the REAL attach client, applies
/// real `TerminalFrame`s, composites real pixels offscreen, sends real
/// input and a real geometry change, and reports named observations.
/// A decoded-message fixture is deliberately not a substitute — it would
/// prove nothing about the three fitting together.
#[cfg(feature = "crdt")]
#[test]
fn a37_real_daemon_real_pty_and_headless_gpu_render_one_terminal_session() {
    use std::path::{Path, PathBuf};

    /// The `pmacs-gpu` binary sits beside the `pmacs` one in the same
    /// target directory. Cargo exposes `CARGO_BIN_EXE_*` only for the
    /// package under test, so the sibling is derived rather than named.
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
            "PMACS_REQUIRE_GPU is set but {} is not built; run the workspace build first",
            binary.display()
        );
        eprintln!(
            "skipping a37: {} is not built (build the workspace to include it)",
            binary.display()
        );
        return;
    }

    let daemon = common::daemon::TestDaemon::spawn_with_env_and_init(
        &[
            ("PMACS_INSTANCE_SEMANTIC_RENDER", "1"),
            ("PMACS_INSTANCE_MULTI_FRONTEND", "1"),
        ],
        // The terminal is opened BY THE FRONTEND, through a real key
        // binding: `terminal.open` targets the invoking frontend's
        // window, so this is what actually puts the GPU's own window on
        // a terminal buffer. Opening it from init.lua instead would
        // switch the daemon's local window and leave the attached
        // frontend on a scratch — which is a real behavior, just not
        // the one this acceptance is about.
        //
        // The child keeps writing so the rendered frame carries live
        // cursor-addressed content and outlives the resize.
        PROBE_INIT_LUA,
    );

    let report = daemon
        .socket_path()
        .parent()
        .expect("socket parent")
        .join("gpu-probe.txt");
    let output = std::process::Command::new(&binary)
        .arg("--headless-probe")
        .arg(daemon.socket_path())
        .arg(&report)
        // The chord the probe presses to run `vterm-probe.open`.
        .env("PMACS_GPU_PROBE_OPEN_KEY", "t")
        .output()
        .expect("run the headless GPU probe");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Exit 3 is "no wgpu adapter" — the same skip contract the
        // in-crate headless render tests use.
        let no_adapter = output.status.code() == Some(3);
        assert!(
            no_adapter && !required,
            "headless GPU probe failed (status {:?}):\n{stderr}",
            output.status.code()
        );
        eprintln!("skipping a37: no wgpu adapter available");
        return;
    }

    let text = std::fs::read_to_string(&report).expect("probe report");
    let facts: std::collections::HashMap<&str, &str> = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();

    assert_eq!(
        facts.get("server_protocol_version").copied(),
        Some("20"),
        "the real daemon negotiated v20 with the real client: {text}"
    );
    assert_eq!(
        facts.get("entered_terminal_mode").copied(),
        Some("true"),
        "the GPU entered terminal mode from a real frame: {text}"
    );
    let frames: u32 = facts
        .get("frames")
        .and_then(|v| v.parse().ok())
        .unwrap_or_default();
    assert!(
        frames >= 2,
        "expected live terminal frames, got {frames}: {text}"
    );
    let rendered: u32 = facts
        .get("rendered_nonuniform_frames")
        .and_then(|v| v.parse().ok())
        .unwrap_or_default();
    assert!(
        rendered >= 2,
        "expected real composited frames, got {rendered}: {text}"
    );
    assert!(
        facts
            .get("last_frame_text")
            .is_some_and(|t| t.contains("VTERMROW")),
        "the child's cursor-addressed output must reach the rendered frame: {text}"
    );
    let declarations: u32 = facts
        .get("declarations")
        .and_then(|v| v.parse().ok())
        .unwrap_or_default();
    assert!(
        declarations >= 2,
        "expected a bootstrap declaration and a resize declaration, got \
         {declarations}: {text}"
    );
    assert_eq!(
        facts.get("observed_resized_frame").copied(),
        Some("true"),
        "the controller's resize must reach the shared PTY and come back \
         as a frame at the new width: {text}"
    );
    assert!(
        facts
            .get("disconnect")
            .copied()
            .unwrap_or_default()
            .is_empty(),
        "the probe must finish without a transport failure: {text}"
    );
}

/// Review round 1, finding 1: a frontend that enters terminal mode must
/// still tell its peers it left the document.
///
/// **This is a regression guard, not a bite-verified fix.** The review
/// predicted that skipping the presence sweep in terminal mode freezes
/// `last_broadcast` at the abandoned document position. It does not, and
/// this test passes against the pre-fix tree: the buffer-follow clears
/// the terminal declaration when it ships the snapshot, so
/// `terminal_active` is false on the tick a window first shows a
/// terminal, and the declaration cannot arrive until a later tick — one
/// truthful sweep always lands first. The skip was load-bearing on that
/// ordering and bought nothing, so it is gone; this test pins the
/// resulting invariant against a future reordering that would make the
/// predicted freeze real.
///
/// Real daemon, real wire, two real frontends: presence delivery is a
/// property of the dispatcher loop, not of any function it calls.
#[cfg(feature = "crdt")]
#[test]
fn terminal_mode_keeps_reporting_presence_so_peers_drop_the_stale_caret() {
    use pmacs::protocol::{
        AttachRequest, FrontendCapabilities, Hello, Key, KeyEvent, PROTOCOL_VERSION,
        SessionBootstrapRequest, read_message, write_message,
    };
    use std::os::unix::net::UnixStream;

    fn attach(daemon: &common::daemon::TestDaemon, semantic: bool) -> (Hello, UnixStream) {
        let mut stream = daemon.connect();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("read timeout");
        let hello: Hello = read_message(&mut stream).expect("read Hello");
        let req = AttachRequest {
            protocol_version: hello.protocol_version,
            frontend_capabilities: FrontendCapabilities {
                synchronized_output: false,
                unicode_smp: true,
                true_color: true,
                mouse: false,
                bracketed_paste: false,
                terminal_kind: Some("acceptance".into()),
                multi_frontend: true,
                crdt_replica: true,
                semantic_render: semantic,
            },
            initial_size: CellSize::new(24, 80),
        };
        write_message(&mut stream, &req).expect("write AttachRequest");
        if semantic {
            write_message(&mut stream, &SessionBootstrapRequest::default())
                .expect("write semantic bootstrap");
        }
        (hello, stream)
    }

    /// Pump one stream until `want` returns a value, or time out.
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
                Err(_) => continue,
            }
        }
        panic!("timed out waiting for {what}");
    }

    assert_eq!(PROTOCOL_VERSION, 20);
    let daemon = common::daemon::TestDaemon::spawn_with_env_and_init(
        &[
            ("PMACS_INSTANCE_SEMANTIC_RENDER", "1"),
            ("PMACS_INSTANCE_MULTI_FRONTEND", "1"),
        ],
        PROBE_INIT_LUA,
    );

    // B attaches FIRST. The presence sweep is diff-keyed, so A's very
    // first snapshot is its only guaranteed broadcast — if B were not
    // yet a registered recipient, that broadcast would reach nobody and
    // A's presence would sit unchanged (and unsent) forever after.
    let (_hello_b, mut b) = attach(&daemon, false);
    // A is the semantic frontend that will open a terminal; B is the
    // peer whose document view would keep painting A's stale caret.
    let (hello_a, mut a) = attach(&daemon, true);
    let a_id = hello_a.assigned_frontend_id;

    // A declares a byte viewport so it is a live semantic session.
    let document = pump(&mut a, "A's first BufferSnapshot", |msg| match msg {
        InstanceMessage::BufferSnapshot { buffer_id, .. } => Some(*buffer_id),
        _ => None,
    });
    write_message(
        &mut a,
        &pmacs::protocol::FrontendEvent::Viewport {
            frontend_id: a_id,
            buffer_id: document,
            visible: pmacs::protocol::ByteRange { start: 0, end: 0 },
            generation: 0,
        },
    )
    .expect("A declares a viewport");

    // B sees A in the document. Without this the later assertion could
    // pass vacuously against a peer that never had presence at all.
    let seen_in_document = pump(&mut b, "A's presence in the document", |msg| match msg {
        InstanceMessage::PresenceUpdate {
            frontend_id,
            buffer_id,
            ..
        } if *frontend_id == a_id => Some(*buffer_id),
        _ => None,
    });
    assert_eq!(seen_in_document, document);

    // A opens a terminal through the bound chord and declares its cells.
    write_message(
        &mut a,
        &pmacs::protocol::FrontendEvent::Key(KeyEvent {
            frontend_id: a_id,
            key: Key::Char('t'),
            mods: Modifiers::CTRL | Modifiers::ALT,
            timestamp_ns: 0,
        }),
    )
    .expect("A opens a terminal");
    let terminal = pump(&mut a, "A's terminal BufferSnapshot", |msg| match msg {
        InstanceMessage::BufferSnapshot { buffer_id, .. } if *buffer_id != document => {
            Some(*buffer_id)
        }
        _ => None,
    });
    write_message(
        &mut a,
        &pmacs::protocol::FrontendEvent::TerminalResize {
            frontend_id: a_id,
            buffer_id: terminal,
            size: CellSize::new(12, 40),
        },
    )
    .expect("A declares terminal cells");
    // A really is in terminal mode once a frame arrives.
    let framed = pump(&mut a, "A's first TerminalFrame", |msg| match msg {
        InstanceMessage::TerminalFrame(frame) => Some(frame.buffer_id),
        _ => None,
    });
    assert_eq!(framed, terminal);

    // The finding: B must learn that A left the document.
    let seen_after = pump(
        &mut b,
        "A's presence leaving the document",
        |msg| match msg {
            InstanceMessage::PresenceUpdate {
                frontend_id,
                buffer_id,
                ..
            } if *frontend_id == a_id && *buffer_id != document => Some(*buffer_id),
            _ => None,
        },
    );
    assert_eq!(
        seen_after, terminal,
        "A's presence must move into the terminal identity buffer, not freeze \
         at the document position it abandoned"
    );
}

/// Review round 1, finding 2: hover must not claim durable control.
///
/// A semantic frontend reports motion at pixel rate, so if bare `Move`
/// claimed the controller, sweeping the mouse across a PASSIVE split's
/// terminal would take it — and the next layout sync would resize the
/// shared PTY to that background view's geometry. Every deliberate
/// gesture still claims; only motion does not.
#[test]
fn hover_does_not_steal_terminal_control_from_the_active_frontend() {
    let mut state = EditorState::new();
    let owner = FrontendId(71);
    let bystander = FrontendId(72);
    let terminal_buffer = open_terminal(&mut state, "sleep 30", 6, 20);
    tick_until(&mut state, Duration::from_secs(5), |state| {
        state
            .terminal_manager
            .borrow()
            .snapshot(terminal_buffer)
            .is_some()
    });
    let owner_window = attach_view(&state, owner, terminal_buffer);
    let bystander_window = attach_view(&state, bystander, terminal_buffer);

    let owner_size = CellSize::new(6, 20);
    let bystander_size = CellSize::new(4, 12);
    let owner_key = TerminalViewKey::new(owner, owner_window, terminal_buffer);
    let bystander_key = TerminalViewKey::new(bystander, bystander_window, terminal_buffer);
    {
        let mut manager = state.terminal_manager.borrow_mut();
        assert!(manager.record_view_size(owner_key, owner_size));
        assert!(manager.record_view_size(bystander_key, bystander_size));
    }

    // The owner takes control with a real press.
    assert!(state.dispatch_semantic_terminal_pointer(
        owner,
        terminal_buffer,
        CellCoord::new(0, 0),
        MouseKind::Down(MouseButton::Left),
        Modifiers::NONE,
    ));
    assert_eq!(
        state.terminal_manager.borrow().controller(terminal_buffer),
        Some(pmacs::terminal::TerminalController::from_view(owner_key)),
        "a press claims control"
    );

    // The bystander merely hovers, repeatedly. Control must not move.
    for col in 0..4 {
        assert!(state.dispatch_semantic_terminal_pointer(
            bystander,
            terminal_buffer,
            CellCoord::new(0, col),
            MouseKind::Move,
            Modifiers::NONE,
        ));
    }
    assert_eq!(
        state.terminal_manager.borrow().controller(terminal_buffer),
        Some(pmacs::terminal::TerminalController::from_view(owner_key)),
        "hovering a passive view must not take durable control"
    );

    // And the shared PTY keeps the controller's geometry: a stolen
    // controller would resize it to the bystander's smaller view.
    assert!(!state.sync_semantic_terminal_layout(bystander, terminal_buffer, bystander_size));
    assert_eq!(
        state
            .terminal_manager
            .borrow()
            .snapshot(terminal_buffer)
            .expect("snapshot")
            .size,
        owner_size,
        "a hovered-over passive view must not resize the shared screen"
    );

    // A deliberate gesture from the bystander still claims, so the
    // hover exemption is narrow rather than a dead controller path.
    assert!(state.dispatch_semantic_terminal_pointer(
        bystander,
        terminal_buffer,
        CellCoord::new(0, 1),
        MouseKind::Down(MouseButton::Left),
        Modifiers::NONE,
    ));
    assert_eq!(
        state.terminal_manager.borrow().controller(terminal_buffer),
        Some(pmacs::terminal::TerminalController::from_view(
            bystander_key
        )),
        "a press still claims control"
    );

    state
        .terminal_manager
        .borrow_mut()
        .terminate(terminal_buffer, &mut state.process_supervisor.borrow_mut())
        .expect("terminate child");
}

/// Acceptance 30 (v18 half) and 28: a peer that negotiated v18 receives
/// no terminal message at all and keeps the ordinary document path over
/// the empty identity buffer.
#[test]
fn a28_a30_a_v18_semantic_peer_has_no_terminal_surface() {
    let mut state = EditorState::new();
    let frontend_id = FrontendId(61);
    let terminal_buffer = open_terminal(&mut state, "sleep 30", 4, 20);
    tick_until(&mut state, Duration::from_secs(5), |state| {
        state
            .terminal_manager
            .borrow()
            .snapshot(terminal_buffer)
            .is_some()
    });
    attach_view(&state, frontend_id, terminal_buffer);

    let mut producer = SemanticRenderState::for_peer(frontend_id, 18);
    producer.set_terminal_viewport(terminal_buffer, CellSize::new(4, 20));
    producer.set_viewport(
        terminal_buffer,
        pmacs::protocol::ByteRange { start: 0, end: 0 },
        0,
    );
    let messages = producer.render_frame(&state);
    assert!(
        terminal_frames(&messages).is_empty(),
        "a v18 peer must never receive a terminal frame"
    );
    assert!(
        !producer.in_terminal_mode(),
        "a v18 peer stays on the document path"
    );

    state
        .terminal_manager
        .borrow_mut()
        .terminate(terminal_buffer, &mut state.process_supervisor.borrow_mut())
        .expect("terminate child");
}
