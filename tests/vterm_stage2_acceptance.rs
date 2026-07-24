//! Stage 2 terminal/TUI integration and real-host acceptance.

mod common;

use std::fmt::Write as _;
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
    let baseline_buffer_id = state.core.borrow().active_buffer_id();

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
    assert!(
        state
            .prepare_terminal_views(frontend_id, CellSize::new(1, 1))
            .is_empty(),
        "zero-area terminal placements paint no snapshot"
    );
    assert!(
        state
            .terminal_manager
            .borrow()
            .view_state(view_key)
            .is_some_and(|view| view.selection.is_some()),
        "a transient zero-area placement must retain existing view anchors"
    );
    assert!(
        state
            .prepare_terminal_views(frontend_id, CellSize::new(8, 30))
            .contains_key(&window_id)
    );
    state.dispatch_key(
        frontend_id,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
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

        let (ok, error): (bool, String) = lua
            .load(
                r"
                local ok, err = pcall(function()
                  pmacs.command.invoke_interactive('terminal.scroll-up')
                end)
                return ok, tostring(err)
                ",
            )
            .eval()
            .expect("pcall ambient interactive invoke");
        assert!(!ok);
        assert!(error.contains("active interactive frontend context"));

        for (field, source) in [
            (
                "frontend",
                "pmacs.terminal.view_state { window = 1, buffer = TERM_BUFFER, active = true }",
            ),
            (
                "window",
                "pmacs.terminal.view_state { frontend = 1, buffer = TERM_BUFFER, active = true }",
            ),
            (
                "buffer",
                "pmacs.terminal.view_state { frontend = 1, window = 1, active = true }",
            ),
        ] {
            let (_, error): (bool, String) = lua
                .load(format!(
                    "local ok, err = pcall(function() {source} end); return ok, tostring(err)"
                ))
                .eval()
                .expect("pcall incomplete explicit context");
            assert!(
                error.contains(&format!("missing field `{field}`")),
                "missing `{field}` surfaced as {error:?}"
            );
        }
    }

    state
        .lua_host
        .lua()
        .load(
            r#"
            pmacs.command.define {
              name = "test.terminal-context",
              description = "Exercise context-implicit terminal failure",
              fn = function() pmacs.terminal.scroll(1) end,
            }
            pmacs.keymap.bind {
              scope = "global", sequence = "<f12>", command = "test.terminal-context"
            }
            "#,
        )
        .exec()
        .expect("install context failure probe");
    state
        .core
        .borrow_mut()
        .switch_active_buffer_for(frontend_id, baseline_buffer_id)
        .expect("switch to document buffer");
    state.dispatch_key(
        frontend_id,
        KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE),
    );
    assert!(
        state
            .core
            .borrow()
            .status
            .contains("active window is not a terminal"),
        "context-implicit terminal command must raise a named Lua error"
    );
    state
        .core
        .borrow_mut()
        .switch_active_buffer_for(frontend_id, buffer_id)
        .expect("restore terminal buffer");

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
#[allow(clippy::too_many_lines, reason = "shared view and controller scenario")]
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
    let mut replacement_spec = TerminalSpec::new("/bin/sh");
    replacement_spec.args = vec!["-c".into(), "sleep 30".into()];
    replacement_spec.rows = 4;
    replacement_spec.cols = 24;
    let replacement_buffer = state
        .terminal_manager
        .borrow_mut()
        .open(
            replacement_spec,
            &mut state.core.borrow_mut(),
            &mut state.process_supervisor.borrow_mut(),
        )
        .expect("open replacement terminal");

    let size = CellSize::new(4, 24);
    let first = TerminalViewKey::new(FrontendId(11), WindowId::next(), buffer_id);
    let second = TerminalViewKey::new(FrontendId(22), WindowId::next(), buffer_id);
    let replacement = TerminalViewKey::new(FrontendId(11), WindowId::next(), replacement_buffer);
    {
        let mut manager = state.terminal_manager.borrow_mut();
        let first_tail = manager
            .snapshot_for_view(first, size)
            .expect("first snapshot");
        let second_tail = manager
            .snapshot_for_view(second, size)
            .expect("second snapshot");
        manager
            .snapshot_for_view(replacement, size)
            .expect("replacement snapshot");
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
        assert!(manager.claim_controller(replacement));
        assert_eq!(
            manager.controller_view_for_frontend(FrontendId(11)),
            Some(replacement),
            "claiming another session atomically replaces a frontend's controller"
        );
        assert!(manager.controller(buffer_id).is_none());
        assert!(manager.claim_controller(second));
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
    state
        .terminal_manager
        .borrow_mut()
        .terminate(
            replacement_buffer,
            &mut state.process_supervisor.borrow_mut(),
        )
        .expect("terminate replacement terminal child");
}

#[test]
fn terminal_escape_gates_local_bindings_and_double_escape_sends_interrupt() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let ready_path = temp.path().join("ready");
    let input_path = temp.path().join("input");
    let probe = format!(
        concat!(
            "import os, tty\n",
            "tty.setraw(0)\n",
            "open({:?}, 'wb').write(b'1')\n",
            "data = b''\n",
            "while len(data) < 5: data += os.read(0, 5 - len(data))\n",
            "open({:?}, 'wb').write(data)\n",
        ),
        ready_path.to_str().expect("UTF-8 ready path"),
        input_path.to_str().expect("UTF-8 input path")
    );
    let mut state = EditorState::new();
    state
        .lua_host
        .lua()
        .load(format!(
            r#"
            return pmacs.terminal.open {{
              command = "/usr/bin/python3",
              args = {{ "-c", {} }},
              rows = 4,
              cols = 20,
            }}
            "#,
            lua_string(&probe)
        ))
        .eval::<AnyUserData>()
        .expect("open raw input probe");
    assert_eq!(wait_for_file(&ready_path, Duration::from_secs(5)), b"1");

    let frontend_id = FrontendId::LOCAL;
    state.dispatch_key(
        frontend_id,
        KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT),
    );
    state.dispatch_key(
        frontend_id,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    state.dispatch_key(
        frontend_id,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    state.dispatch_key(
        frontend_id,
        KeyEvent::new(KeyCode::Char('w'), KeyModifiers::ALT),
    );

    assert_eq!(
        wait_for_file(&input_path, Duration::from_secs(5)),
        b"\x1bv\x03\x1bw",
        "unescaped local bindings reach the child; C-c C-c sends one literal interrupt"
    );
}

/// How far pmacs actually got before a wait timed out.
///
/// Distinguishes the failure modes that the raw host-output tail cannot:
/// pmacs died on startup, `init.lua` was never loaded (config resolution),
/// `pmacs.terminal.open` raised, or the child spawned but produced nothing.
/// `startup` is `(label, path)` breadcrumb pairs written by `init.lua`.
fn describe_startup(pty: &mut PmacsPty, startup: &[(&str, &Path)]) -> String {
    let mut out = match pty.wait_for_exit(Duration::from_millis(0)) {
        Some(status) => format!("pmacs ALREADY EXITED (status {status:?})"),
        None => "pmacs still running".to_string(),
    };
    for (label, path) in startup {
        let state = match fs::read(path) {
            Ok(bytes) => format!("{:?}", String::from_utf8_lossy(&bytes)),
            Err(_) => "MISSING".to_string(),
        };
        let _ = write!(out, "; {label}={state}");
    }
    out
}

/// Host bytes with ANSI escape sequences removed.
///
/// The TUI differ paints only cells that CHANGED and skips ones already
/// matching, so a run the emulator holds contiguously on one screen row can
/// still reach the host as `PREF<cursor-move>IX`. Assertions about child
/// output therefore fall back to matching over this stripped stream.
///
/// This cannot mask the failure that matters: text the child never wrote is
/// absent from the stripped bytes too. It only removes the false negative
/// where the differ split a run that did render.
fn strip_ansi(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != 0x1b {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        match bytes.get(i + 1) {
            // CSI: parameters/intermediates, then a final byte in 0x40..=0x7e.
            Some(b'[') => {
                i += 2;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                i += usize::from(i < bytes.len());
            }
            // OSC: terminated by BEL or ST (`ESC \`).
            Some(b']') => {
                i += 2;
                while i < bytes.len() && bytes[i] != 0x07 {
                    if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                i += usize::from(i < bytes.len());
            }
            // nF form: intermediates in 0x20..=0x2f then one final byte
            // (`ESC ( B` designates ASCII into G0 and is three bytes, not two).
            Some(0x20..=0x2f) => {
                i += 1;
                while i < bytes.len() && (0x20..=0x2f).contains(&bytes[i]) {
                    i += 1;
                }
                i += usize::from(i < bytes.len());
            }
            // Single-byte final (`ESC 7`, `ESC M`, …).
            Some(_) => i += 2,
            None => i += 1,
        }
    }
    out
}

/// Longest prefix of `needle` that appears anywhere in `haystack`.
///
/// Diagnostic only (see the failure arm of [`wait_for_output`]): it tells a
/// failed match whether the child's bytes reached the host at all.
fn longest_rendered_prefix(haystack: &[u8], needle: &[u8]) -> usize {
    (1..=needle.len())
        .rev()
        .find(|&n| haystack.windows(n).any(|window| window == &needle[..n]))
        .unwrap_or(0)
}

fn wait_for_output(
    pty: &mut PmacsPty,
    needle: &[u8],
    timeout: Duration,
    startup: &[(&str, &Path)],
) {
    let deadline = Instant::now() + timeout;
    // Matching is STRICT, deliberately. An earlier revision also tried an
    // escape-stripped match to tolerate a run the differ had split; that is
    // unsound for painted CELLS, because `cell::diff` drops an already-matching
    // cell entirely rather than merely interrupting the run, and no match
    // strategy recovers a byte that was never sent. Assertions here are
    // therefore limited to protocol escapes pmacs writes straight to the host
    // (clipboard, mode resets), which the differ never touches. Content that
    // must be seen on SCREEN is asserted in-process over `snapshot_text`.
    loop {
        let output = pty.output();
        if output.windows(needle.len()).any(|window| window == needle) {
            return;
        }
        if Instant::now() >= deadline {
            let diagnosis = describe_startup(pty, startup);
            let output = pty.output();
            let start = output.len().saturating_sub(4_000);
            // Report how much of the needle reached the host at all. The
            // printed tail cannot answer that on its own: a settled screen
            // emits empty diffs forever and pushes any real text out of the
            // window. A prefix strictly between 0 and the full length means
            // the bytes arrived mutilated rather than never — which for cell
            // content is the differ dropping an already-matching cell.
            let visible = strip_ansi(&output);
            let seen = longest_rendered_prefix(&visible, needle)
                .max(longest_rendered_prefix(&output, needle));
            let verdict = if seen == 0 {
                "no child text reached the host"
            } else {
                "child text rendered only partially"
            };
            panic!(
                "host output never contained {needle:?} after {timeout:?}\n  \
                 startup: {diagnosis}\n  \
                 rendered prefix: {seen}/{len} bytes ({prefix:?}) — {verdict}\n  \
                 tail: {tail}",
                needle = String::from_utf8_lossy(needle),
                len = needle.len(),
                prefix = String::from_utf8_lossy(&needle[..seen]),
                tail = output[start..].escape_ascii()
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// [`wait_for_file`] that reports startup breadcrumbs when it times out.
///
/// The plain helper panics with only the missing path, which is the least
/// useful thing to know: a readiness file that never appears is exactly when
/// "how far did startup get" decides where to look next.
fn wait_for_published_file(
    pty: &mut PmacsPty,
    path: &Path,
    timeout: Duration,
    startup: &[(&str, &Path)],
) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(bytes) = fs::read(path) {
            return bytes;
        }
        assert!(
            Instant::now() < deadline,
            "child never published {} within {timeout:?}\n  startup: {}",
            path.display(),
            describe_startup(pty, startup)
        );
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
    // Readiness is published as a FILE, not as host bytes. See the wait below.
    let alt_ready_path = temp.path().join("alt-ready");
    let input_path = temp.path().join("child-input");
    let size_path = temp.path().join("child-size");
    let init_path = temp.path().join("init-reached");
    let open_path = temp.path().join("terminal-open");
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
            // CRLF, not bare LF. The supervisor's PTY trampoline runs
            // `stty raw`, which clears OPOST, so a lone `\n` moves DOWN
            // without returning to column 1 and every line staircases five
            // columns right. Against this session's 40-column child that
            // walks the readiness marker into the right margin, where it
            // wraps mid-word and reaches the host as two pieces separated by
            // other repainted cells — unmatchable, and intermittent because
            // the column depends on whether pmacs has resized the PTY to the
            // window width yet. Explicit carriage returns keep every write
            // column-stable, so the markers below start at column 1.
            "os.write(1, b'\\x1b[?1049h\\x1b[2J')\n",
            "for i in range(20): os.write(1, b'alt%02d\\r\\n' % i)\n",
            "os.write(1, b'VTERM_ALT_READY')\n",
            "open({:?}, 'wb').write(b'1')\n",
            "read_until(b'ALT_GATE\\n')\n",
            "os.write(1, b'\\x1b[?1049l')\n",
            "for i in range(40): os.write(1, b'main%02d\\r\\n' % i)\n",
            "os.write(1, b'VTERM_MAIN_READY\\x07')\n",
            "data = read_exact(18)\n",
            "open({:?}, 'wb').write(data)\n",
            "size = os.get_terminal_size(0)\n",
            "open({:?}, 'w').write(f'{{size.lines}} {{size.columns}}\\n')\n"
        ),
        alt_ready_path.to_str().expect("UTF-8 alt-ready path"),
        input_path.to_str().expect("UTF-8 input path"),
        size_path.to_str().expect("UTF-8 size path")
    );
    // Breadcrumbs (see `describe_startup`). This test spawns the REAL pmacs
    // binary in a real PTY, so when the first wait below times out the only
    // evidence is host escape bytes — which cannot distinguish "init.lua never
    // loaded" from "terminal.open failed" from "the child produced nothing".
    // That ambiguity has outlived several investigations of this exact
    // failure, so the config records how far startup actually got.
    let init = format!(
        r#"
        local function breadcrumb(path, text)
          local f = io.open(path, "w")
          if f then f:write(text) f:close() end
        end
        breadcrumb({:?}, "1")
        local ok, terminal_buffer = pcall(pmacs.terminal.open, {{
          command = "/bin/sh",
          args = {{ "-c", "exec /usr/bin/python3 -c \"$1\"", "pmacs-vterm-probe", {} }},
          rows = 10,
          cols = 40,
          scrollback_rows = 200,
        }})
        breadcrumb({:?}, ok and "ok" or ("ERROR: " .. tostring(terminal_buffer)))
        assert(ok, terminal_buffer)
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
        init_path.to_str().expect("UTF-8 init breadcrumb path"),
        lua_string(&probe),
        open_path.to_str().expect("UTF-8 open breadcrumb path")
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
    let startup: &[(&str, &Path)] = &[
        ("init.lua reached", init_path.as_path()),
        ("terminal.open", open_path.as_path()),
    ];
    // Gate on a file the child publishes, not on its text appearing in the
    // host stream. Host bytes cannot carry this assertion: `cell::diff` splits
    // a run at any cell where `prev == next` and NEVER TRANSMITS that cell
    // (pinned by `cell::tests::diff_split_by_unchanged_cell_is_two_spans`), so
    // whenever a character of the marker already happens to sit at its
    // destination the host receives the marker with that byte missing. No
    // matching strategy can recover a byte that was never sent — which is what
    // the macOS failures were, reporting a stable `rendered prefix: 6/15`
    // across two different child layouts.
    //
    // This is a synchronisation gate, not the assertion. That the child's
    // output reaches the SCREEN is pinned in-process, at the layer that can
    // see it, by `lua_surface_is_strict_...` asserting over `snapshot_text`.
    // What this test uniquely owns is host lifecycle — the clipboard escape,
    // geometry propagation, and terminal restore asserted below — and those
    // are protocol escapes pmacs writes directly, never painted cells, so the
    // differ cannot split them.
    assert_eq!(
        wait_for_published_file(&mut pty, &alt_ready_path, Duration::from_secs(10), startup),
        b"1",
        "alt-screen readiness breadcrumb was published but malformed"
    );

    pty.resize(30, 90).expect("resize host PTY");
    thread::sleep(Duration::from_millis(150));
    pty.write_input(b"\x03\x1bv")
        .expect("escaped terminal page-up binding");
    // Inject editor-owned scrolling, selection, and copy gestures while the
    // real terminal child is active; focused tests pin their exact state.
    pty.write_input(b"\x1b[<4;2;2M\x1b[<36;8;2M\x1b[<4;8;2m")
        .expect("terminal selection drag");
    pty.write_input(b"\x03\x1bw")
        .expect("escaped copy selection binding");
    wait_for_output(&mut pty, b"\x1b]52;c;", Duration::from_secs(5), startup);
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

/// `strip_ansi` joins a run interrupted by escapes, WITHOUT inventing text.
///
/// It backs the failure diagnostic in [`wait_for_output`], not the match: it
/// separates "arrived, interrupted by cursor moves" from "never arrived".
/// It deliberately does NOT rescue a run the cell differ split, because that
/// path drops the matching cell rather than escaping around it — the case
/// pinned below and by `cell::tests::diff_split_by_unchanged_cell_is_two_spans`.
#[test]
fn strip_ansi_rejoins_a_split_run_but_never_invents_absent_text() {
    let needle = b"VTERM_ALT_READY";

    // Split by a cursor move mid-run: stripping rejoins it.
    let split = b"\x1b[9;30HVTERM_ALT_\x1b[10;1HREADY".to_vec();
    let visible = strip_ansi(&split);
    assert!(visible.windows(needle.len()).any(|w| w == needle));

    // A silent child stays silent: no amount of stripping conjures the text.
    let silent = b"\x1b[?2026h\x1b[22;42H\x1b[?25h\x1b[?2026l".repeat(4);
    let visible = strip_ansi(&silent);
    assert!(
        !visible.windows(needle.len()).any(|w| w == needle),
        "stripping must not manufacture text the child never wrote"
    );
    assert!(
        visible.is_empty(),
        "pure escapes strip to nothing: {visible:?}"
    );

    // OSC (clipboard) and two-byte escapes are consumed, payload text kept.
    assert_eq!(strip_ansi(b"a\x1b]52;c;Zm9v\x07b"), b"ab");
    assert_eq!(strip_ansi(b"x\x1b(By"), b"xy");

    // The case that defeated two attempted fixes, kept here so the wrong
    // remedy is not reached for again. `cell::diff` splits a run at an
    // already-matching cell and never transmits it, so the host sees the
    // marker with an interior byte MISSING, not merely escaped around.
    // Stripping is powerless; the longest prefix stops at the hole, which is
    // exactly the `6/15 ("VTERM_")` the macOS runs reported.
    let dropped = b"\x1b[9;1HVTERM_\x1b[9;8HLT_READY".to_vec();
    let visible = strip_ansi(&dropped);
    assert_eq!(visible, b"VTERM_LT_READY", "the 'A' was never sent");
    assert!(!visible.windows(needle.len()).any(|w| w == needle));
    assert_eq!(
        longest_rendered_prefix(&visible, needle),
        6,
        "a dropped interior cell caps the prefix at the hole"
    );
}

/// The flake diagnostic's discriminator (see [`wait_for_output`]).
///
/// The macOS `VTERM_ALT_READY` failure reports a settled screen whose tail is
/// pure cursor/sync escapes, which alone cannot say whether the child ever
/// wrote. These two cases are exactly what the failure arm must tell apart.
#[test]
fn longest_rendered_prefix_separates_absent_child_text_from_a_split_render() {
    let needle = b"VTERM_ALT_READY";

    // Nothing from the child: a settled screen of cursor moves only.
    let silent = b"\x1b[?2026h\x1b[22;42H\x1b[?25h\x1b[?2026l".repeat(4);
    assert_eq!(longest_rendered_prefix(&silent, needle), 0);

    // Rendered, but the emulator held it across two screen rows, so the host
    // stream carries a cursor move mid-marker and the contiguous match fails.
    let mut split = Vec::new();
    split.extend_from_slice(b"\x1b[9;30HVTERM_ALT_");
    split.extend_from_slice(b"\x1b[10;1HREADY");
    let seen = longest_rendered_prefix(&split, needle);
    assert_eq!(seen, 10, "must report the rendered prefix, not zero");
    assert_eq!(&needle[..seen], b"VTERM_ALT_");

    // Fully contiguous is the passing case and never reaches the failure arm.
    assert_eq!(
        longest_rendered_prefix(b"\x1b[9;1HVTERM_ALT_READY", needle),
        needle.len()
    );
}
