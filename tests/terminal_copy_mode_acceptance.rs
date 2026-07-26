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

fn viewport() -> CellSize {
    CellSize::new(10, 40)
}

/// Give LOCAL a window on the terminal and register/claim its view, which
/// is what makes `dispatch_key`'s terminal transport arm reachable.
/// Returns the view key, so assertions can read the *projected* view
/// rather than the context-free live screen.
fn focus_terminal(state: &EditorState, buffer: pmacs::buffer::BufferId) -> TerminalViewKey {
    state.core.borrow_mut().switch_active_buffer(buffer).ok();
    let window = state.core.borrow().active_window_id();
    let key = TerminalViewKey::new(FrontendId::LOCAL, window, buffer);
    let mut manager = state.terminal_manager.borrow_mut();
    manager.register_view(key);
    manager.claim_controller(key);
    let _ = manager.snapshot_for_view(key, viewport());
    key
}

/// Make the child produce NEW output, so a refresh has something to find.
///
/// The child is `exec cat`, so typing into the focused terminal echoes
/// back. Without this, "refresh" tests compare a quiet terminal against
/// itself and pass with the render replaced by a no-op — the defect review
/// round 1 found in acceptance 18 and 19.
fn emit_into_child(state: &mut EditorState, terminal: pmacs::buffer::BufferId, marker: &str) {
    focus_terminal(state, terminal);
    for ch in marker.chars() {
        press(state, KeyCode::Char(ch), KeyModifiers::NONE);
    }
    assert!(
        tick_until(state, marker, terminal),
        "the child must echo {marker:?} back onto the live screen"
    );
}

/// What the registered VIEW currently projects — which, unlike
/// `manager.snapshot(buffer)`, depends on where the view is anchored.
fn view_text(state: &EditorState, key: TerminalViewKey) -> String {
    let mut manager = state.terminal_manager.borrow_mut();
    let Some(snapshot) = manager.snapshot_for_view(key, viewport()) else {
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

fn view_at_bottom(state: &EditorState, key: TerminalViewKey) -> bool {
    state
        .terminal_manager
        .borrow_mut()
        .snapshot_for_view(key, viewport())
        .is_some_and(|snapshot| snapshot.at_bottom)
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
/// and the buffer is genuinely `read_only` at the rope boundary, so the
/// protection does not depend on which key or command was used.
#[test]
fn acc16b_the_snapshot_is_immutable_at_the_rope_not_merely_intercepted() {
    let mut state = EditorState::new();
    let terminal = open_fill_terminal(&mut state);
    focus_terminal(&state, terminal);
    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");

    let before = buffer_text_by_name(&state, SNAPSHOT_NAME).expect("snapshot");
    press(&mut state, KeyCode::Char('z'), KeyModifiers::NONE);
    let after = buffer_text_by_name(&state, SNAPSHOT_NAME).expect("snapshot");
    assert_eq!(before, after, "the read-only intercept rejects self-insert");

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
        registry
            .get(snapshot)
            .expect("snapshot buffer")
            .is_read_only(),
        "an intercept guards the dispatch path only; `Buffer::undo` reaches \
         the rope through `ensure_writable` without consulting it, so the \
         snapshot must be read-only at the rope"
    );
    drop(registry);
    drop(core);
    state.process_supervisor.borrow_mut().shutdown();
}

/// Acceptance 16c (review round 2, P1): **undo cannot empty the snapshot**,
/// through the chord *or* through the command.
///
/// The chord half alone would be a false pass. `M-x buffer.undo` and the
/// menu reach `Buffer::undo` without passing through any buffer-local
/// keymap, so rebinding `C-/` to a no-op — the existing `*compilation*`
/// idiom, which documents that "command/menu undo stays dispatchable" —
/// leaves the buffer emptiable. Only rope-level `read_only` closes both.
#[test]
fn acc16c_undo_cannot_empty_the_snapshot_by_chord_or_by_command() {
    let mut state = EditorState::new();
    let terminal = open_fill_terminal(&mut state);
    focus_terminal(&state, terminal);
    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");

    let rendered = buffer_text_by_name(&state, SNAPSHOT_NAME).expect("snapshot");
    assert!(
        rendered.contains("LINE200"),
        "precondition: the snapshot has content to lose"
    );

    // The command path — reachable regardless of any buffer-local binding.
    let _: Value = state
        .lua_host
        .lua()
        .load(r"return pcall(pmacs.command.invoke_interactive, 'buffer.undo')")
        .eval()
        .expect("invoke_interactive is callable");
    assert_eq!(
        buffer_text_by_name(&state, SNAPSHOT_NAME).as_deref(),
        Some(rendered.as_str()),
        "M-x buffer.undo must not empty the snapshot"
    );

    // The chord path.
    press(&mut state, KeyCode::Char('/'), KeyModifiers::CONTROL);
    assert_eq!(
        buffer_text_by_name(&state, SNAPSHOT_NAME).as_deref(),
        Some(rendered.as_str()),
        "C-/ must not empty the snapshot"
    );

    // Redo is the same door.
    let _: Value = state
        .lua_host
        .lua()
        .load(r"return pcall(pmacs.command.invoke_interactive, 'buffer.redo')")
        .eval()
        .expect("invoke_interactive is callable");
    assert_eq!(
        buffer_text_by_name(&state, SNAPSHOT_NAME).as_deref(),
        Some(rendered.as_str()),
        "buffer.redo must not alter the snapshot either"
    );

    // ...and the owner's own refresh still works, which is the whole
    // reason plain `read_only` was not enough on its own.
    emit_into_child(&mut state, terminal, "STILLREFRESHES");
    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");
    assert!(
        buffer_text_by_name(&state, SNAPSHOT_NAME)
            .expect("snapshot")
            .contains("STILLREFRESHES"),
        "the owner-authorized write path must survive immutability"
    );
    state.process_supervisor.borrow_mut().shutdown();
}

/// Snapshot buffer id, by name, from the Rust side.
#[cfg(feature = "crdt")]
fn snapshot_buffer_id(state: &EditorState) -> pmacs::buffer::BufferId {
    let core = state.core.borrow();
    let reg = core.registry.borrow();
    reg.ids()
        .iter()
        .copied()
        .find(|id| reg.get(*id).is_ok_and(|b| b.name() == SNAPSHOT_NAME))
        .expect("snapshot buffer exists")
}

/// Rendered cells of the active window (the `m4_acceptance` grid helper;
/// cross-crate test code can't import it).
fn render_active_window_to_grid(
    state: &mut EditorState,
    rows: u32,
    cols: u32,
) -> Vec<pmacs::cell::Cell> {
    use pmacs::cell::{Cell, CellGrid};
    use pmacs::view::{View, Viewport};
    use pmacs::window::Rect;

    let mut core = state.core.borrow_mut();
    let active = core.active_window_id();
    let registry = core.registry.clone();
    let win = core.windows.get_mut(&active).expect("active window");
    let rect = Rect::new(0, 0, rows, cols);
    let mut backing = vec![Cell::default(); (rows * cols) as usize];
    let reg = registry.borrow();
    let buf = reg.get(win.buffer_id).expect("buffer in registry");
    let viewport = Viewport {
        buffer_start: 0,
        buffer_end: buf.len(),
        cell_origin: rect.origin,
        cell_size: CellSize::new(rows, cols),
        gutter_w: 0,
        folds: None,
    };
    let mut grid = CellGrid {
        cells: &mut backing,
        stride: cols,
        size: CellSize::new(rows, cols),
    };
    win.text_view.render(buf, viewport, &mut grid);
    backing
}

fn grid_row(cells: &[pmacs::cell::Cell], row: u32, cols: u32) -> String {
    (0..cols)
        .map(|c| match cells[(row * cols + c) as usize].glyph {
            Glyph::Char(ch) => ch,
            _ => ' ',
        })
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// Review round 3, P1. A rope write is only half of an edit: the window
/// showing the buffer holds a `TextView` line index that only `on_edit`
/// maintains, so a write that reaches the rope without the notification
/// leaves the two disagreeing.
///
/// Pinned by PAINTING, because that is where the disagreement bites: with
/// the fan-out dropped, the next render indexes the new rope with the old
/// line offsets. A shrinking write is used deliberately — stale offsets
/// then point past the buffer end, which is the reported crash rather than
/// merely stale pixels.
///
/// Driven through `pmacs.buffer.set_generated_contents`, the seam copy
/// mode's refresh actually calls, so it also covers `*compilation*` and
/// any other owner that adopts the primitive later.
#[test]
fn acc16d_a_generated_write_notifies_the_window_that_displays_it() {
    let mut state = EditorState::new();
    exec(
        &state,
        r"
        GEN = pmacs.buffer.create('*generated-probe*')
        pmacs.buffer.set_generated_contents(GEN, 'alpha\nbeta\ngamma\ndelta\nepsilon\n')
        pmacs.window.switch_buffer(GEN)
        ",
    );
    let painted = render_active_window_to_grid(&mut state, 6, 20);
    assert_eq!(
        grid_row(&painted, 0, 20),
        "alpha",
        "precondition: the window paints the generated buffer"
    );

    exec(
        &state,
        r"pmacs.buffer.set_generated_contents(GEN, 'CHANGED\n')",
    );
    let painted = render_active_window_to_grid(&mut state, 6, 20);
    assert_eq!(
        grid_row(&painted, 0, 20),
        "CHANGED",
        "the window must paint the refreshed contents"
    );
    assert_eq!(
        grid_row(&painted, 1, 20),
        "",
        "and nothing of the longer contents it replaced"
    );
}

/// Review round 3, P1, CRDT half. The same dropped fan-out also skips
/// `queue_daemon_origin_crdt_op`, so replica mirrors never import the
/// owner's write and their optimistic edits are generated against content
/// the owner has already replaced.
///
/// Gated because `upgrade_to_crdt` is — and therefore dark in CI, which
/// never enables the feature. The default-configuration half above is the
/// one that actually runs there.
#[cfg(feature = "crdt")]
#[test]
fn acc16e_a_refresh_queues_the_owners_write_for_replica_mirrors() {
    let mut state = EditorState::new();
    let terminal = open_fill_terminal(&mut state);
    focus_terminal(&state, terminal);
    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");

    let snapshot = snapshot_buffer_id(&state);
    {
        let core = state.core.borrow();
        let mut reg = core.registry.borrow_mut();
        let buffer = reg.get_mut(snapshot).expect("snapshot buffer");
        // `read_only` refuses the upgrade's own bookkeeping path the same
        // way it refuses everything else, so lift it around the upgrade.
        buffer.set_read_only(false);
        buffer.upgrade_to_crdt(2).expect("upgrade");
        buffer.set_read_only(true);
    }
    state.core.borrow_mut().pending_crdt_ops.clear();

    emit_into_child(&mut state, terminal, "MIRRORME");
    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");

    let queued: Vec<_> = state
        .core
        .borrow()
        .pending_crdt_ops
        .iter()
        .map(|(_, id, _)| *id)
        .collect();
    assert!(
        queued.contains(&snapshot),
        "the owner's refresh must be queued for broadcast; queued: {queued:?}"
    );
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
    assert!(
        !buffer_text_by_name(&state, SNAPSHOT_NAME)
            .expect("snapshot")
            .contains("REINVOKE"),
        "precondition: the marker has not been emitted yet"
    );

    // Advance the world, then re-invoke. Counting buffers alone is
    // vacuous: it passes with the render replaced by a no-op, so the
    // refresh must be observed by CONTENT that only exists after the
    // first snapshot was taken.
    emit_into_child(&mut state, terminal, "REINVOKE");
    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");
    assert!(
        buffer_text_by_name(&state, SNAPSHOT_NAME)
            .expect("snapshot")
            .contains("REINVOKE"),
        "re-invoking must actually re-serialize, not just reuse the buffer"
    );
    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");
    assert_eq!(
        buffer_count(&state),
        count_after_first,
        "...and it must refresh IN PLACE, not accumulate buffers"
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

    // `q` returns to the source terminal.
    press(&mut state, KeyCode::Char('q'), KeyModifiers::NONE);
    assert_eq!(
        active_buffer_name(&state),
        terminal_name,
        "q must return to the terminal the snapshot was taken from"
    );

    // Now advance the world and come back WITHOUT re-invoking copy mode,
    // so the snapshot is genuinely stale. Comparing a quiet terminal's
    // snapshot against itself is vacuous — it passes with `render_snapshot`
    // replaced by a no-op.
    emit_into_child(&mut state, terminal, "AFTER-G");
    exec(
        &state,
        &format!(
            r"
            for _, id in ipairs(pmacs.buffer.list()) do
              local ok, d = pcall(pmacs.describe.buffer, id)
              if ok and d and d.name == {SNAPSHOT_NAME:?} then
                pmacs.window.switch_buffer(id)
              end
            end
            "
        ),
    );
    assert!(
        !buffer_text_by_name(&state, SNAPSHOT_NAME)
            .expect("snapshot")
            .contains("AFTER-G"),
        "the snapshot must still be stale before `g` — otherwise the next \
         assertion proves nothing"
    );

    press(&mut state, KeyCode::Char('g'), KeyModifiers::NONE);
    assert!(
        buffer_text_by_name(&state, SNAPSHOT_NAME)
            .expect("snapshot")
            .contains("AFTER-G"),
        "`g` must re-snapshot from the live terminal"
    );
    assert_eq!(
        active_buffer_name(&state),
        SNAPSHOT_NAME,
        "g must not move us"
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
    let key = focus_terminal(&state, terminal);
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

    // The terminal still FOLLOWS ITS TAIL while a snapshot exists.
    //
    // Read through the registered view, not `manager.snapshot(buffer)`:
    // that call is context-free and always returns the live screen, so it
    // reports "at the tail" even for a view forced to the oldest retained
    // row. The projected view is the only thing that can distinguish them.
    assert!(
        view_at_bottom(&state, key),
        "precondition: the view starts at the tail"
    );
    emit_into_child(&mut state, terminal, "TAILMARK");
    assert!(
        view_at_bottom(&state, key),
        "new child output must not knock the view off the tail"
    );
    assert!(
        view_text(&state, key).contains("TAILMARK"),
        "the freshest output must be visible in the PROJECTED view: {:?}",
        view_text(&state, key)
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

/// Acceptance 18a (review round 1, P1): a foreign buffer that happens to
/// carry the snapshot's name is **never adopted**.
///
/// `pmacs.buffer.create` takes any caller-chosen name, and snapshot writes
/// use `bypass_intercept`, so found-by-name adoption clobbers a user's
/// data outright. Ownership means "in copy mode's own handle table"
/// (dired's F7 rule); a taken name gets a `<2>` variant instead.
#[test]
fn acc18a_a_foreign_same_named_buffer_is_never_adopted_or_clobbered() {
    let mut state = EditorState::new();
    let terminal = open_fill_terminal(&mut state);
    focus_terminal(&state, terminal);

    // A user's buffer, sitting exactly where the snapshot wants to go.
    exec(
        &state,
        &format!(
            r"
            FOREIGN = pmacs.buffer.create({SNAPSHOT_NAME:?})
            FOREIGN:insert(0, 'do not clobber')
            "
        ),
    );

    exec(&state, "pmacs.terminal.copy_mode(TERM_BUF)");

    let foreign_text: String = eval(&state, r"return FOREIGN:slice(0, FOREIGN:len())");
    assert_eq!(
        foreign_text, "do not clobber",
        "the foreign buffer must be untouched"
    );
    assert_ne!(
        active_buffer_name(&state),
        SNAPSHOT_NAME,
        "copy mode must not display the foreign buffer"
    );
    assert_eq!(
        active_buffer_name(&state),
        format!("{SNAPSHOT_NAME}<2>"),
        "a taken name must yield a unique variant"
    );
    assert!(
        buffer_text_by_name(&state, &format!("{SNAPSHOT_NAME}<2>"))
            .expect("variant snapshot")
            .contains("LINE200"),
        "the variant is the real snapshot"
    );
    state.process_supervisor.borrow_mut().shutdown();
}

/// Acceptance 18b (review round 1, P1): snapshot identity is the terminal
/// BUFFER, not its name.
///
/// `TerminalManager::open` uniquifies only the *derived* name — an
/// explicit `name = ...` is inserted verbatim — so two valid terminals can
/// share a name. Keying snapshots by name gives them one buffer between
/// them: the second invocation retargets it, `q` returns to the wrong
/// terminal, and killing either one removes the shared snapshot.
#[test]
fn acc18b_two_same_named_terminals_get_two_independent_snapshots() {
    let mut state = EditorState::new();
    exec(&state, FILL_PROFILE);

    let before = terminal_buffers(&state);
    exec(
        &state,
        r#"TERM_A = pmacs.terminal.open { profile = "fill", name = "*same*" }"#,
    );
    exec(
        &state,
        r#"TERM_B = pmacs.terminal.open { profile = "fill", name = "*same*" }"#,
    );
    let fresh: Vec<_> = terminal_buffers(&state)
        .into_iter()
        .filter(|id| !before.contains(id))
        .collect();
    assert_eq!(fresh.len(), 2, "two terminals opened under one name");

    // Distinguish them by content, since their names are identical.
    emit_into_child(&mut state, fresh[0], "AAAA");
    emit_into_child(&mut state, fresh[1], "BBBB");

    focus_terminal(&state, fresh[0]);
    let snap_a: String = eval(
        &state,
        r"local b = pmacs.terminal.copy_mode(TERM_A); return (pmacs.describe.buffer(b)).name",
    );
    focus_terminal(&state, fresh[1]);
    let snap_b: String = eval(
        &state,
        r"local b = pmacs.terminal.copy_mode(TERM_B); return (pmacs.describe.buffer(b)).name",
    );

    assert_ne!(
        snap_a, snap_b,
        "two terminals must not share one snapshot buffer"
    );
    let text_a = buffer_text_by_name(&state, &snap_a).expect("snapshot A");
    let text_b = buffer_text_by_name(&state, &snap_b).expect("snapshot B");
    assert!(
        text_a.contains("AAAA") && !text_a.contains("BBBB"),
        "snapshot A must hold only A's output: {:?}",
        &text_a[text_a.len().saturating_sub(60)..]
    );
    assert!(
        text_b.contains("BBBB") && !text_b.contains("AAAA"),
        "snapshot B must hold only B's output"
    );

    // `q` from each snapshot returns to ITS OWN terminal, which is only
    // observable through the buffer id — the two names are the same.
    exec(
        &state,
        &format!(
            r"
            for _, id in ipairs(pmacs.buffer.list()) do
              local ok, d = pcall(pmacs.describe.buffer, id)
              if ok and d and d.name == {snap_b:?} then pmacs.window.switch_buffer(id) end
            end
            "
        ),
    );
    press(&mut state, KeyCode::Char('q'), KeyModifiers::NONE);
    let returned_is_b: bool = eval(&state, r"return pmacs.window.buffer() == TERM_B");
    assert!(
        returned_is_b,
        "q from B's snapshot must return to terminal B"
    );

    // Killing terminal A removes only A's snapshot.
    exec(&state, "pmacs.terminal.terminate(TERM_A)");
    exec(&state, "pmacs.buffer.kill(TERM_A)");
    assert!(
        buffer_text_by_name(&state, &snap_a).is_none(),
        "A's snapshot dies with A"
    );
    assert!(
        buffer_text_by_name(&state, &snap_b).is_some(),
        "B's snapshot must SURVIVE — a shared buffer would have gone too"
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
