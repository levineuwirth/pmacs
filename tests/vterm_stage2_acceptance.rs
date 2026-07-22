//! Stage 2 terminal/TUI integration and real-host acceptance.

mod common;

use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mlua::{AnyUserData, Table, Value};
use pmacs::cell::{CellCoord, CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::BufferIdLua;
use pmacs::protocol::FrontendId;
use pmacs::statusline::{
    StatuslineEvaluationOutcome, StatuslineEvaluationTarget, evaluate_statusline,
};
use pmacs::terminal::{TerminalProcessState, TerminalSpec, TerminalViewKey};
use pmacs::window::WindowId;

use common::pty::{PmacsPty, spawn_pmacs_in_pty};

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

fn lua_string(value: &str) -> String {
    format!("{value:?}")
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

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "cross-surface Lua transaction scenario"
)]
fn lua_surface_is_strict_fresh_transactional_and_context_safe() {
    let mut state = EditorState::new();
    let command_lua = lua_string("/bin/sh");

    let baseline_sessions = state.terminal_manager.borrow().len();
    let baseline_buffers = state.core.borrow().registry.borrow().ids().len();
    {
        let lua = state.lua_host.lua();
        let error = lua
            .load(format!(
                "return pmacs.terminal.open {{ command = {command_lua}, unknown = true }}"
            ))
            .eval::<Value>()
            .expect_err("unknown open field must fail");
        assert!(error.to_string().contains("unknown field `unknown`"));
    }
    assert_eq!(state.terminal_manager.borrow().len(), baseline_sessions);
    assert_eq!(
        state.core.borrow().registry.borrow().ids().len(),
        baseline_buffers
    );

    {
        let lua = state.lua_host.lua();
        let kind: String = lua
            .load(format!(
                r#"
                TERM_BUFFER = pmacs.terminal.open {{
                  command = {command_lua},
                  args = {{ "-c", "printf 'copy-me\\n'; sleep 30" }},
                  rows = 4,
                  cols = 30,
                }}
                local first = pmacs.terminal.state(TERM_BUFFER)
                first.process.kind = "poisoned"
                first.injected = true
                local second = pmacs.terminal.state(TERM_BUFFER)
                assert(second.injected == nil)
                assert(pmacs.terminal.resize == nil)
                return second.process.kind
                "#
            ))
            .eval()
            .expect("open terminal and read fresh state");
        assert_eq!(kind, "running");
    }

    let buffer_id = {
        let userdata: AnyUserData = state
            .lua_host
            .lua()
            .globals()
            .get("TERM_BUFFER")
            .expect("terminal buffer global");
        userdata
            .borrow::<BufferIdLua>()
            .expect("BufferId userdata")
            .0
    };
    let buffer_name = state
        .core
        .borrow()
        .registry
        .borrow()
        .get(buffer_id)
        .expect("terminal identity buffer")
        .name()
        .to_owned();
    assert_eq!(buffer_name, "*terminal:sh*");
    tick_until(&mut state, Duration::from_secs(5), |state| {
        state
            .terminal_manager
            .borrow()
            .snapshot(buffer_id)
            .is_some_and(|snapshot| snapshot_text(&snapshot).contains("copy-me"))
    });

    let frontend_id = FrontendId::LOCAL;
    let window_id = state.core.borrow().active_window_id();
    assert!(state.sync_terminal_layout(frontend_id, CellSize::new(8, 30)));
    let snapshots = state.prepare_terminal_views(frontend_id, CellSize::new(8, 30));
    assert!(snapshots.contains_key(&window_id));
    let modeline = evaluate_statusline(
        state.lua_host.lua(),
        &state.core,
        &state.statusline_registry,
        StatuslineEvaluationTarget::Grid { frontend_id },
    );
    let StatuslineEvaluationOutcome::Ready(windows) = modeline.outcome else {
        panic!("terminal statusline provider must evaluate successfully");
    };
    assert!(
        windows
            .iter()
            .flat_map(|window| &window.right)
            .any(|segment| segment.text.starts_with("TERM")),
        "built-in terminal statusline provider must report live process state"
    );
    let snapshot = snapshots.get(&window_id).expect("active terminal snapshot");
    let snapshot_text = snapshot_text(snapshot);
    let start = snapshot_text
        .find("copy-me")
        .unwrap_or_else(|| panic!("copy probe missing from projected snapshot: {snapshot_text:?}"));
    let cols = usize::try_from(snapshot.size.cols).expect("column count fits");
    let row = u32::try_from(start / cols).expect("row fits");
    let col = u32::try_from(start % cols).expect("column fits");
    let view_key = TerminalViewKey::new(frontend_id, window_id, buffer_id);
    {
        let mut manager = state.terminal_manager.borrow_mut();
        assert!(manager.begin_selection(view_key, snapshot.size, CellCoord::new(row, col)));
        assert!(manager.finish_selection(view_key, snapshot.size, CellCoord::new(row, col + 7)));
    }
    state.dispatch_key(
        frontend_id,
        KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT),
    );
    let (clipboard_frontend, clipboard) = state
        .core
        .borrow_mut()
        .take_pending_clipboard()
        .expect("terminal copy must queue host clipboard bytes");
    assert_eq!(clipboard_frontend, frontend_id);
    assert_eq!(clipboard, b"copy-me");
    {
        let lua = state.lua_host.lua();
        let status: Table = lua
            .load(format!(
                r"
                local first = pmacs.terminal.view_state {{
                  frontend = 1, window = {}, buffer = TERM_BUFFER, active = true
                }}
                first.injected = true
                local second = pmacs.terminal.view_state {{
                  frontend = 1, window = {}, buffer = TERM_BUFFER, active = true
                }}
                assert(second.injected == nil)
                return second
                ",
                window_id.raw(),
                window_id.raw()
            ))
            .eval()
            .expect("fresh exact view state");
        assert!(status.get::<bool>("at_bottom").expect("at_bottom"));

        let (ok, error): (bool, String) = lua
            .load(
                r"
                local ok, err = pcall(function() pmacs.terminal.scroll(1) end)
                return ok, tostring(err)
                ",
            )
            .eval()
            .expect("pcall implicit scroll");
        assert!(!ok);
        assert!(error.contains("interactive frontend context"));
    }

    state
        .terminal_manager
        .borrow_mut()
        .terminate(buffer_id, &mut state.process_supervisor.borrow_mut())
        .expect("terminate terminal child");
    tick_until(&mut state, Duration::from_secs(5), |state| {
        state
            .terminal_manager
            .borrow()
            .snapshot(buffer_id)
            .is_some_and(|snapshot| !matches!(snapshot.process, TerminalProcessState::Running))
    });
    state
        .core
        .borrow_mut()
        .kill_buffer(buffer_id)
        .expect("kill retained terminal buffer");
    state.tick_processes();
    assert_eq!(state.terminal_manager.borrow().len(), baseline_sessions);
    assert_eq!(
        state.core.borrow().registry.borrow().ids().len(),
        baseline_buffers
    );

    state.process_supervisor.borrow_mut().shutdown();
    let _error = state
        .lua_host
        .lua()
        .load(format!(
            "return pmacs.terminal.open {{ command = {command_lua} }}"
        ))
        .eval::<Value>()
        .expect_err("closed supervisor must make spawn fail transactionally");
    assert_eq!(state.terminal_manager.borrow().len(), baseline_sessions);
    assert_eq!(
        state.core.borrow().registry.borrow().ids().len(),
        baseline_buffers
    );
}

#[test]
fn shared_screen_keeps_view_scroll_selection_and_controller_independent() {
    let mut state = EditorState::new();
    let mut spec = TerminalSpec::new("/bin/sh");
    spec.args = vec![
        "-c".into(),
        "i=0; while [ $i -lt 30 ]; do printf 'row%02d\\n' \"$i\"; i=$((i+1)); done; sleep 30"
            .into(),
    ];
    spec.rows = 6;
    spec.cols = 24;
    let buffer_id = state
        .terminal_manager
        .borrow_mut()
        .open(
            spec,
            &mut state.core.borrow_mut(),
            &mut state.process_supervisor.borrow_mut(),
        )
        .expect("open terminal");
    tick_until(&mut state, Duration::from_secs(5), |state| {
        state
            .terminal_manager
            .borrow()
            .snapshot(buffer_id)
            .is_some_and(|snapshot| snapshot_text(&snapshot).contains("row29"))
    });

    let size = CellSize::new(4, 24);
    let first = TerminalViewKey::new(FrontendId(11), WindowId::next(), buffer_id);
    let second = TerminalViewKey::new(FrontendId(22), WindowId::next(), buffer_id);
    {
        let mut manager = state.terminal_manager.borrow_mut();
        let first_tail = manager
            .snapshot_for_view(first, size)
            .expect("first snapshot");
        let second_tail = manager
            .snapshot_for_view(second, size)
            .expect("second snapshot");
        assert_eq!(first_tail.title, second_tail.title);
        assert_eq!(first_tail.process, second_tail.process);

        assert!(manager.scroll_lines(first, 3));
        let first_status = manager.view_status(first).expect("first status");
        let second_status = manager.view_status(second).expect("second status");
        assert_eq!(first_status.scroll_offset, 3);
        assert_eq!(second_status.scroll_offset, 0);

        assert!(manager.begin_selection(first, size, CellCoord::new(0, 0)));
        assert!(manager.finish_selection(first, size, CellCoord::new(0, 4)));
        assert!(
            manager
                .view_status(first)
                .expect("selected status")
                .selection
        );
        assert!(
            !manager
                .view_status(second)
                .expect("passive status")
                .selection
        );
        assert!(
            !manager
                .copy_selection(first)
                .expect("copied selection")
                .is_empty()
        );
        assert!(manager.copy_selection(second).is_none());

        assert!(manager.claim_controller(first));
        assert!(manager.claim_controller(second));
        assert_eq!(
            manager.controller_view_for_frontend(FrontendId(11)),
            None,
            "one terminal session has one most-recent controller"
        );
        assert_eq!(
            manager.controller_view_for_frontend(FrontendId(22)),
            Some(second)
        );
        manager.detach_frontend(FrontendId(11));
        assert!(manager.view_status(first).is_none());
        assert!(manager.view_status(second).is_some());
        assert!(
            manager
                .controller_view_for_frontend(FrontendId(11))
                .is_none()
        );
        assert_eq!(
            manager.controller_view_for_frontend(FrontendId(22)),
            Some(second)
        );
    }

    state
        .terminal_manager
        .borrow_mut()
        .terminate(buffer_id, &mut state.process_supervisor.borrow_mut())
        .expect("terminate terminal child");
}

fn wait_for_output(pty: &PmacsPty, needle: &[u8], timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if pty
            .output()
            .windows(needle.len())
            .any(|window| window == needle)
        {
            return;
        }
        if Instant::now() >= deadline {
            let output = pty.output();
            let start = output.len().saturating_sub(4_000);
            panic!(
                "host output never contained {:?}; tail: {}",
                String::from_utf8_lossy(needle),
                output[start..].escape_ascii()
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_file(path: &Path, timeout: Duration) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(bytes) = fs::read(path) {
            return bytes;
        }
        assert!(
            Instant::now() < deadline,
            "file was not published: {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[allow(clippy::too_many_lines, reason = "one real-host lifecycle scenario")]
fn real_tui_terminal_smoke_restores_host_after_output_input_resize_scroll_copy_and_bell() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let config_root = temp.path().join("config");
    let config_dir = config_root.join("pmacs");
    let state_root = temp.path().join("state");
    let input_path = temp.path().join("child-input");
    let size_path = temp.path().join("child-size");
    fs::create_dir_all(&config_dir).expect("config dir");

    let probe = format!(
        concat!(
            "import os\n",
            "def read_exact(count):\n",
            "    data = b''\n",
            "    while len(data) < count:\n",
            "        data += os.read(0, count - len(data))\n",
            "    return data\n",
            "def read_until(marker):\n",
            "    data = b''\n",
            "    while marker not in data:\n",
            "        data += os.read(0, 4096)\n",
            "os.write(1, b'\\x1b[?1049h\\x1b[2J')\n",
            "for i in range(20): os.write(1, b'alt%02d\\n' % i)\n",
            "os.write(1, b'VTERM_ALT_READY')\n",
            "read_until(b'ALT_GATE\\n')\n",
            "os.write(1, b'\\x1b[?1049l')\n",
            "for i in range(40): os.write(1, b'main%02d\\n' % i)\n",
            "os.write(1, b'VTERM_MAIN_READY\\x07')\n",
            "data = read_exact(18)\n",
            "open({:?}, 'wb').write(data)\n",
            "size = os.get_terminal_size(0)\n",
            "open({:?}, 'w').write(f'{{size.lines}} {{size.columns}}\\n')\n"
        ),
        input_path.to_str().expect("UTF-8 input path"),
        size_path.to_str().expect("UTF-8 size path")
    );
    let init = format!(
        r#"
        local terminal_buffer = pmacs.terminal.open {{
          command = "/bin/sh",
          args = {{ "-c", "exec /usr/bin/python3 -c \"$1\"", "pmacs-vterm-probe", {} }},
          rows = 10,
          cols = 40,
          scrollback_rows = 200,
        }}
        local ticks = 0
        pmacs.hook.add("process.after-tick", function()
          ticks = ticks + 1
          if ticks == 120 then
            pmacs.terminal.send(terminal_buffer, "ALT_GATE\n")
          elseif ticks == 180 then
            pmacs.terminal.send(terminal_buffer, "VTERM_INPUT_SMOKE\n")
          end
        end)
        "#,
        lua_string(&probe)
    );
    fs::write(config_dir.join("init.lua"), init).expect("write init.lua");

    let mut pty = spawn_pmacs_in_pty(
        &[],
        &[
            ("XDG_CONFIG_HOME", config_root.as_path()),
            ("PMACS_STATE_HOME", state_root.as_path()),
            ("TERM", Path::new("xterm-256color")),
        ],
        24,
        80,
    );
    wait_for_output(&pty, b"VTERM_ALT_READY", Duration::from_secs(10));

    pty.resize(30, 90).expect("resize host PTY");
    thread::sleep(Duration::from_millis(150));
    pty.write_input(b"\x1bv").expect("terminal page-up binding");
    // Inject editor-owned scrolling, selection, and copy gestures while the
    // real terminal child is active; focused tests pin their exact state.
    pty.write_input(b"\x1b[<4;2;2M\x1b[<36;8;2M\x1b[<4;8;2m")
        .expect("terminal selection drag");
    pty.write_input(b"\x1bw").expect("copy selection binding");
    wait_for_output(&pty, b"\x1b]52;c;", Duration::from_secs(5));
    let input = wait_for_file(&input_path, Duration::from_secs(5));
    assert_eq!(input, b"VTERM_INPUT_SMOKE\n");
    pty.write_input(b"\x1b[200~PASTE_AFTER_EXIT\x1b[201~")
        .expect("route a real host paste event");
    let size = String::from_utf8(wait_for_file(&size_path, Duration::from_secs(5)))
        .expect("UTF-8 stty size");
    assert_eq!(size.trim(), "28 90", "child PTY must receive cell geometry");
    thread::sleep(Duration::from_millis(200));
    pty.write_input(b"\x03\x18\x03").expect("quit pmacs");

    let status = pty
        .wait_for_exit(Duration::from_secs(5))
        .expect("pmacs should exit after terminal smoke");
    assert!(status.success(), "pmacs exit status: {status:?}");
    thread::sleep(Duration::from_millis(50));
    let output = pty.output();
    assert!(
        output.windows(8).any(|window| window == b"\x1b[?1049h"),
        "pmacs must enter its host alternate screen"
    );
    assert!(
        output.windows(8).any(|window| window == b"\x1b[?1049l"),
        "pmacs must restore the host main screen"
    );
    assert!(
        output.contains(&0x07),
        "one active-terminal BEL must reach the local host"
    );
    assert!(
        output.windows(8).any(|window| window == b"\x1b[?2004l"),
        "pmacs must disable host bracketed paste on exit"
    );
}
