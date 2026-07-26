//! Terminal copy-mode acceptance (Stage 2 of
//! `docs/terminal-config-and-copy-mode-framing.md`, criteria 13-21).
//!
//! **Deliberately NOT `#[cfg(feature = "crdt")]`.** CI never enables that
//! feature, so a gated suite is written and then never run — 264 tests are
//! dark workspace-wide for exactly that reason. Criterion 16, the
//! round-trip gate Q#TC6a's entire safety argument rests on, needs no CRDT
//! and must be caught by the default configuration.

use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mlua::Value;
use pmacs::cell::{CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;
use pmacs::terminal::TerminalViewKey;

const SNAPSHOT_NAME: &str = "*terminal-copy: terminal:sh*";

fn exec(state: &EditorState, src: &str) {
    state
        .lua_host
        .lua()
        .load(src)
        .exec()
        .unwrap_or_else(|e| panic!("lua failed: {src}\n{e}"));
}

fn eval<T: mlua::FromLuaMulti>(state: &EditorState, src: &str) -> T {
    state
        .lua_host
        .lua()
        .load(src)
        .eval()
        .unwrap_or_else(|e| panic!("lua eval failed: {src}\n{e}"))
}

fn eval_err(state: &EditorState, src: &str) -> String {
    let result: mlua::Result<Value> = state.lua_host.lua().load(src).eval();
    match result {
        Ok(_) => panic!("expected an error from: {src}"),
        Err(e) => e.to_string(),
    }
}

fn press(state: &mut EditorState, code: KeyCode, mods: KeyModifiers) {
    state.dispatch_key(FrontendId::LOCAL, KeyEvent::new(code, mods));
}

/// The live terminal screen's text, used only to wait for the child.
fn screen_text(state: &EditorState, buffer: pmacs::buffer::BufferId) -> String {
    let manager = state.terminal_manager.borrow();
    let Some(snapshot) = manager.snapshot(buffer) else {
        return String::new();
    };
    let mut text = String::new();
    for cell in &snapshot.cells {
        match &cell.glyph {
            Glyph::Char(c) => text.push(*c),
            Glyph::Cluster(b) => text.push_str(&String::from_utf8_lossy(b)),
            Glyph::Continuation => {}
        }
    }
    text
}

fn tick_until(state: &mut EditorState, needle: &str, buffer: pmacs::buffer::BufferId) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        state.tick_processes();
        if screen_text(state, buffer).contains(needle) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn terminal_buffers(state: &EditorState) -> Vec<pmacs::buffer::BufferId> {
    let manager = state.terminal_manager.borrow();
    state
        .core
        .borrow()
        .registry
        .borrow()
        .ids()
        .iter()
        .copied()
        .filter(|id| manager.is_terminal(*id))
        .collect()
}

/// A child that overflows the 24-row screen and then goes quiet, so its
/// early lines exist ONLY in scrollback — which is what makes criterion
/// 15's "content only in scrollback" claim meaningful.
const FILL_PROFILE: &str = r#"
pmacs.terminal.profiles.fill = {
  command = "/bin/sh",
  args = { "-c",
    "printf 'NEEDLE-IN-SCROLLBACK\r\n'; i=1; while [ $i -le 200 ]; do printf 'LINE%03d\r\n' $i; i=$((i+1)); done; printf 'DONE\r\n'; exec cat" },
}
"#;

/// Open the fill terminal, wait for the child to finish, and return its id.
fn open_fill_terminal(state: &mut EditorState) -> pmacs::buffer::BufferId {
    exec(state, FILL_PROFILE);
    let before = terminal_buffers(state);
    exec(
        state,
        r#"TERM_BUF = pmacs.terminal.open { profile = "fill" }"#,
    );
    let fresh: Vec<_> = terminal_buffers(state)
        .into_iter()
        .filter(|id| !before.contains(id))
        .collect();
    assert_eq!(fresh.len(), 1, "exactly one terminal must have opened");
    let buffer = fresh[0];
    assert!(tick_until(state, "DONE", buffer), "the child must finish");
    buffer
}

/// Give LOCAL a window on the terminal and register/claim its view, which
/// is what makes `dispatch_key`'s terminal transport arm reachable.
fn focus_terminal(state: &EditorState, buffer: pmacs::buffer::BufferId) {
    state.core.borrow_mut().switch_active_buffer(buffer).ok();
    let window = state.core.borrow().active_window_id();
    let key = TerminalViewKey::new(FrontendId::LOCAL, window, buffer);
    let mut manager = state.terminal_manager.borrow_mut();
    manager.register_view(key);
    manager.claim_controller(key);
    let _ = manager.snapshot_for_view(key, CellSize::new(10, 40));
}

fn buffer_text_by_name(state: &EditorState, name: &str) -> Option<String> {
    eval(
        state,
        &format!(
            r"
            for _, id in ipairs(pmacs.buffer.list()) do
              local ok, d = pcall(pmacs.describe.buffer, id)
              if ok and d and d.name == {name:?} then
                return id:slice(0, id:len())
              end
            end
            return nil
            "
        ),
    )
}

fn active_buffer_name(state: &EditorState) -> String {
    eval(
        state,
        r"local b = pmacs.window.buffer(); return (pmacs.describe.buffer(b)).name",
    )
}

fn buffer_count(state: &EditorState) -> usize {
    state.core.borrow().registry.borrow().ids().len()
}

/// Acceptance 13: the snapshot's text is exactly the whole retained range
/// as the existing copy path serializes it.
///
/// Compared against `_copy_retained` rather than a literal, so this cannot
/// pass by both sides drifting the same way; the exact-bytes fidelity
/// claims (criterion 14) are pinned at the unit level in
/// `src/terminal/view.rs`, against the same projection fixtures that pin
/// `copy_selection_bytes` itself.
#[test]
fn acc13_snapshot_is_the_whole_retained_range_through_the_shared_serializer() {
    let mut state = EditorState::new();
    let terminal = open_fill_terminal(&mut state);
    focus_terminal(&state, terminal);

    exec(&state, "SNAP = pmacs.terminal.copy_mode(TERM_BUF)");
    let snapshot_text = buffer_text_by_name(&state, SNAPSHOT_NAME).expect("snapshot buffer exists");
    let serialized: String = eval(
        &state,
        r"return pmacs.terminal._copy_retained(TERM_BUF) or ''",
    );

    assert_eq!(
        snapshot_text, serialized,
        "the snapshot must be byte-identical to the shared serializer's output"
    );
    assert!(
        snapshot_text.contains("NEEDLE-IN-SCROLLBACK") && snapshot_text.contains("LINE200"),
        "the range must span scrollback AND the visible screen"
    );
    state.process_supervisor.borrow_mut().shutdown();
}

/// Acceptance 14 (end-to-end half): the snapshot really is a rope-backed
/// document buffer and not a terminal, which is what makes every
/// buffer-shaped consumer work and what removes the transport arm.
#[test]
fn acc14_the_snapshot_is_an_ordinary_non_terminal_buffer() {
    let mut state = EditorState::new();
    let terminal = open_fill_terminal(&mut state);
    focus_terminal(&state, terminal);
    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");

    let is_terminal: bool = eval(
        &state,
        r"local b = pmacs.window.buffer(); return pmacs.terminal.is_terminal(b)",
    );
    assert!(
        !is_terminal,
        "the snapshot must NOT be a terminal — that is what structurally \
         removes the transport arm rather than guarding it"
    );
    assert_eq!(active_buffer_name(&state), SNAPSHOT_NAME);
    state.process_supervisor.borrow_mut().shutdown();
}

/// Acceptance 15: isearch finds content that exists ONLY in scrollback,
/// with no change to `src/search.rs` (B1).
#[test]
fn acc15_isearch_finds_content_only_in_scrollback() {
    let mut state = EditorState::new();
    let terminal = open_fill_terminal(&mut state);
    focus_terminal(&state, terminal);

    // The needle is off the visible screen: the live terminal cannot see it.
    assert!(
        !screen_text(&state, terminal).contains("NEEDLE-IN-SCROLLBACK"),
        "precondition: the needle must have scrolled off the live screen"
    );

    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");
    state.core.borrow_mut().set_cursor_byte(0);

    // Drive real isearch: C-s then the needle.
    press(&mut state, KeyCode::Char('s'), KeyModifiers::CONTROL);
    for ch in "NEEDLE-IN-SCROLLBACK".chars() {
        press(&mut state, KeyCode::Char(ch), KeyModifiers::NONE);
    }
    let cursor = state.core.borrow().cursor();
    press(&mut state, KeyCode::Enter, KeyModifiers::NONE);

    let text = buffer_text_by_name(&state, SNAPSHOT_NAME).expect("snapshot");
    let expected = text
        .find("NEEDLE-IN-SCROLLBACK")
        .expect("the needle is in the snapshot") as u64;
    assert_eq!(
        cursor,
        expected,
        "isearch must land on the scrollback-only match; text was {:?}",
        &text[..text.len().min(80)]
    );
    state.process_supervisor.borrow_mut().shutdown();
}

/// Acceptance 16 — the load-bearing pin, and the reason this suite is
/// ungated. `set_round_trip_input` is the ONLY thing standing between a
/// replica frontend and unauthorized mutation (Q#TC6a), so its regression
/// must be caught in the configuration CI actually compiles.
#[test]
fn acc16_dispatch_idle_is_false_while_the_snapshot_is_focused() {
    let mut state = EditorState::new();
    let terminal = open_fill_terminal(&mut state);
    focus_terminal(&state, terminal);
    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");

    assert!(
        !state.dispatch_idle(),
        "a focused snapshot must round-trip keys, so no replica applies \
         optimistically and none emits a CRDT op"
    );
    // ...and it is the SNAPSHOT that does it, not merely "some terminal
    // buffer is around": switching to an ordinary buffer restores idle.
    exec(
        &state,
        r#"pmacs.window.switch_buffer(pmacs.buffer.create("*plain*"))"#,
    );
    assert!(state.dispatch_idle(), "an ordinary buffer is idle again");
    state.process_supervisor.borrow_mut().shutdown();
}

/// Acceptance 16 (the other half): the intercept rejects ordinary edits,
/// and — the fact that makes round-trip load-bearing rather than defence
/// in depth — the buffer is **not** `read_only` at the rope boundary.
#[test]
fn acc16b_the_intercept_rejects_edits_but_is_not_rope_level_protection() {
    let mut state = EditorState::new();
    let terminal = open_fill_terminal(&mut state);
    focus_terminal(&state, terminal);
    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");

    let before = buffer_text_by_name(&state, SNAPSHOT_NAME).expect("snapshot");
    press(&mut state, KeyCode::Char('z'), KeyModifiers::NONE);
    let after = buffer_text_by_name(&state, SNAPSHOT_NAME).expect("snapshot");
    assert_eq!(before, after, "the read-only intercept rejects self-insert");

    // Q#TC6a, stated as a test so the next reader does not mistake the
    // intercept for real immutability: no Lua binding sets
    // `Buffer::read_only`, so this buffer accepts rope/CRDT mutation and
    // only the round-trip mark above keeps a replica from producing one.
    let core = state.core.borrow();
    let registry = core.registry.borrow();
    let ids = registry.ids();
    let snapshot = ids
        .iter()
        .copied()
        .find(|id| {
            registry
                .get(*id)
                .is_ok_and(|buf| buf.name() == SNAPSHOT_NAME)
        })
        .expect("snapshot buffer id");
    assert!(
        !registry
            .get(snapshot)
            .expect("snapshot buffer")
            .is_read_only(),
        "the Lua intercept does NOT set Buffer::read_only — this is why \
         set_round_trip_input is the guard and not hardening"
    );
    drop(registry);
    drop(core);
    state.process_supervisor.borrow_mut().shutdown();
}

/// Acceptance 18: re-invoking refreshes in place, and the lifecycle runs
/// both directions.
#[test]
fn acc18_reinvoke_refreshes_in_place_and_lifecycle_runs_both_ways() {
    let mut state = EditorState::new();
    let terminal = open_fill_terminal(&mut state);
    focus_terminal(&state, terminal);

    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");
    let count_after_first = buffer_count(&state);

    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");
    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");
    assert_eq!(
        buffer_count(&state),
        count_after_first,
        "re-invoking must refresh in place, not accumulate buffers"
    );

    // Killing the snapshot alone leaves the terminal running.
    exec(
        &state,
        &format!(
            r"
            for _, id in ipairs(pmacs.buffer.list()) do
              local ok, d = pcall(pmacs.describe.buffer, id)
              if ok and d and d.name == {SNAPSHOT_NAME:?} then pmacs.buffer.kill(id) end
            end
            "
        ),
    );
    assert!(
        state.terminal_manager.borrow().is_terminal(terminal),
        "killing the snapshot must leave the terminal untouched"
    );
    assert!(
        buffer_text_by_name(&state, SNAPSHOT_NAME).is_none(),
        "the snapshot buffer is gone"
    );

    // ...and it can be rebuilt afterwards.
    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");
    assert!(
        buffer_text_by_name(&state, SNAPSHOT_NAME).is_some(),
        "a later invoke rebuilds the snapshot"
    );

    // Killing the terminal takes its snapshot with it.
    exec(&state, "pmacs.terminal.terminate(TERM_BUF)");
    exec(&state, "pmacs.buffer.kill(TERM_BUF)");
    assert!(
        buffer_text_by_name(&state, SNAPSHOT_NAME).is_none(),
        "killing the terminal must remove its snapshot"
    );
    state.process_supervisor.borrow_mut().shutdown();
}

/// Acceptance 19: `C-t` in a terminal — physically `C-c C-t`, because every
/// unescaped key goes to the child — enters copy mode; `g` refreshes and
/// `q` returns to the source terminal.
#[test]
fn acc19_escape_c_t_enters_copy_mode_and_g_and_q_work() {
    let mut state = EditorState::new();
    let terminal = open_fill_terminal(&mut state);
    focus_terminal(&state, terminal);
    let terminal_name = active_buffer_name(&state);

    // The escape, then the terminal-local binding.
    press(&mut state, KeyCode::Char('c'), KeyModifiers::CONTROL);
    press(&mut state, KeyCode::Char('t'), KeyModifiers::CONTROL);
    assert_eq!(
        active_buffer_name(&state),
        SNAPSHOT_NAME,
        "C-c C-t must enter copy mode"
    );

    // `g` re-snapshots in place.
    let before = buffer_text_by_name(&state, SNAPSHOT_NAME).expect("snapshot");
    press(&mut state, KeyCode::Char('g'), KeyModifiers::NONE);
    let after = buffer_text_by_name(&state, SNAPSHOT_NAME).expect("snapshot");
    assert_eq!(before, after, "a quiet terminal re-snapshots identically");
    assert_eq!(
        active_buffer_name(&state),
        SNAPSHOT_NAME,
        "g must not move us"
    );

    // `q` returns to the source terminal.
    press(&mut state, KeyCode::Char('q'), KeyModifiers::NONE);
    assert_eq!(
        active_buffer_name(&state),
        terminal_name,
        "q must return to the terminal the snapshot was taken from"
    );
    state.process_supervisor.borrow_mut().shutdown();
}

/// Acceptance 20: copy mode is additive — the live terminal's own keys are
/// unchanged while a snapshot exists, and the terminal still follows its
/// tail.
#[test]
fn acc20_live_terminal_keys_are_unchanged_while_a_snapshot_exists() {
    let mut state = EditorState::new();
    let terminal = open_fill_terminal(&mut state);
    focus_terminal(&state, terminal);
    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");

    // Back to the terminal; its five live bindings must still resolve.
    exec(&state, "pmacs.window.switch_buffer(TERM_BUF)");
    for (sequence, command) in [
        ("M-w", "terminal.copy-selection"),
        ("M-v", "terminal.page-up"),
        ("C-v", "terminal.page-down"),
        ("M-<", "terminal.scroll-oldest"),
        ("M->", "terminal.scroll-bottom"),
    ] {
        let resolved: Option<String> = eval(
            &state,
            &format!(r"local d = pmacs.describe.key({sequence:?}); return d and d.command"),
        );
        assert_eq!(
            resolved.as_deref(),
            Some(command),
            "{sequence} must still be the live terminal binding"
        );
    }

    // The terminal is still following its tail: the child's last output is
    // visible without scrolling.
    assert!(
        screen_text(&state, terminal).contains("DONE"),
        "the live terminal keeps following its tail"
    );
    state.process_supervisor.borrow_mut().shutdown();
}

/// Acceptance 21: the dispatch-shadow count is unchanged at six, pinned by
/// the observable difference between a buffer-local keymap and a shadow —
/// `describe-key` telling the truth about `g` and `q` in the snapshot.
///
/// A seventh shadow would decode these keys before `KeymapStack::resolve`
/// ever ran, so introspection would report whatever the global binding is
/// (or nothing) while the keys behaved differently.
#[test]
fn acc21_describe_key_reports_the_truth_for_the_snapshot_bindings() {
    let mut state = EditorState::new();
    let terminal = open_fill_terminal(&mut state);
    focus_terminal(&state, terminal);
    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");

    for (sequence, command) in [("g", "terminal.copy-refresh"), ("q", "terminal.copy-quit")] {
        let resolved: Option<String> = eval(
            &state,
            &format!(r"local d = pmacs.describe.key({sequence:?}); return d and d.command"),
        );
        assert_eq!(
            resolved.as_deref(),
            Some(command),
            "describe-key must report the buffer-local {sequence} binding"
        );
    }

    // And the binding really is scoped: back in the terminal, `q` is not
    // the copy-mode command.
    exec(&state, "pmacs.window.switch_buffer(TERM_BUF)");
    let resolved: Option<String> = eval(
        &state,
        r#"local d = pmacs.describe.key("q"); return d and d.command"#,
    );
    assert_ne!(
        resolved.as_deref(),
        Some("terminal.copy-quit"),
        "the snapshot's q must not leak into the terminal buffer"
    );
    state.process_supervisor.borrow_mut().shutdown();
}

/// Copy mode refuses a non-terminal buffer rather than producing an empty
/// snapshot of nothing.
#[test]
fn copy_mode_refuses_a_non_terminal_buffer() {
    let state = EditorState::new();
    let err = eval_err(&state, "return pmacs.terminal.copy_mode()");
    assert!(
        err.contains("not a terminal"),
        "the refusal must say why: {err}"
    );
}
