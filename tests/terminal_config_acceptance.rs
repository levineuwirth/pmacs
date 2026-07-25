//! Terminal configuration acceptance (Stage 1 of
//! `docs/terminal-config-and-copy-mode-framing.md`, criteria 1-12).
//!
//! Deliberately NOT `#[cfg(feature = "crdt")]`: CI never enables that
//! feature, so a gated suite is written and then never run.

use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mlua::Value;
use pmacs::cell::{CellSize, Glyph};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;
use pmacs::terminal::TerminalViewKey;
use pmacs::window::WindowId;

fn exec(state: &EditorState, src: &str) {
    state
        .lua_host
        .lua()
        .load(src)
        .exec()
        .unwrap_or_else(|e| panic!("lua failed: {src}\n{e}"));
}

fn eval_err(state: &EditorState, src: &str) -> String {
    let result: mlua::Result<Value> = state.lua_host.lua().load(src).eval();
    match result {
        Ok(_) => panic!("expected an error from: {src}"),
        Err(e) => e.to_string(),
    }
}

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

/// Give LOCAL a window on `buffer` and register/claim its terminal view,
/// which is what makes `dispatch_key`'s terminal arm reachable.
fn focus_terminal(state: &EditorState, buffer: pmacs::buffer::BufferId) -> WindowId {
    state.core.borrow_mut().switch_active_buffer(buffer).ok();
    let window = state.core.borrow().active_window_id();
    let key = TerminalViewKey::new(FrontendId::LOCAL, window, buffer);
    let mut manager = state.terminal_manager.borrow_mut();
    manager.register_view(key);
    manager.claim_controller(key);
    let _ = manager.snapshot_for_view(key, CellSize::new(10, 40));
    window
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

/// Open a terminal from Lua and return the identity buffer it created.
///
/// The id is derived by diffing the manager's terminal set rather than
/// returned through Lua: `BufferIdLua` exposes no id accessor, and
/// diffing also asserts in passing that exactly one terminal appeared.
fn open_cat_terminal(state: &EditorState, lua_spec: &str) -> pmacs::buffer::BufferId {
    let before = terminal_buffers(state);
    exec(
        state,
        &format!("TERM_BUF = pmacs.terminal.open {{ {lua_spec} }}"),
    );
    let after = terminal_buffers(state);
    let mut fresh: Vec<_> = after
        .into_iter()
        .filter(|id| !before.contains(id))
        .collect();
    assert_eq!(fresh.len(), 1, "exactly one terminal must have opened");
    fresh.remove(0)
}

/// `cat -v` is the echo instrument, deliberately: the terminal screen
/// rejects C0/C1 controls before they enter cells (Vterm Stage 1
/// criterion 2), so a raw echoed `Ctrl-X` would be invisible and a test
/// probing for it could never pass. `-v` renders it as the printable
/// two-character `^X`, which is what makes "the configured chord reached
/// the child" observable at all.
const CAT_PROFILE: &str = r#"
pmacs.terminal.profiles.echo = {
  command = "/bin/sh",
  args = { "-c", "printf 'READY\r\n'; exec cat -v" },
}
"#;

/// Did the last key ARM the terminal escape?
///
/// Observed behaviorally rather than through an accessor: while the
/// escape is armed the next key goes to ordinary dispatch, so it never
/// reaches the child. `cat` echoes anything that does reach it, which
/// makes "the probe character did not appear" the exact observable for
/// "that chord was consumed as the escape".
fn escape_was_armed(state: &mut EditorState, buffer: pmacs::buffer::BufferId, probe: char) -> bool {
    // Count occurrences rather than testing for presence: the screen
    // already holds the child's own output, and a single-character probe
    // like 'R' collides with the "READY" banner. Only an INCREASE proves
    // this keystroke reached the child.
    let before = screen_text(state, buffer).matches(probe).count();
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Char(probe), KeyModifiers::NONE),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        state.tick_processes();
        if screen_text(state, buffer).matches(probe).count() > before {
            return false;
        }
        if Instant::now() >= deadline {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Acceptance 1: a profile spec is strict, and rejects before anything spawns.
#[test]
fn acc1_profile_specs_are_strict_and_reject_before_spawning() {
    let state = EditorState::new();
    let before = state.core.borrow().registry.borrow().ids().len();

    exec(
        &state,
        r#"pmacs.terminal.profiles.bad = { command = "/bin/sh", nonsense = true }"#,
    );
    let err = eval_err(&state, r#"return pmacs.terminal.open { profile = "bad" }"#);
    assert!(
        err.contains("unknown field") && err.contains("nonsense"),
        "the error must name the offending field: {err}"
    );

    exec(&state, "pmacs.terminal.profiles.wrong = { command = 42 }");
    let err = eval_err(
        &state,
        r#"return pmacs.terminal.open { profile = "wrong" }"#,
    );
    assert!(err.contains("must be a string"), "typed field error: {err}");

    assert_eq!(
        state.core.borrow().registry.borrow().ids().len(),
        before,
        "a rejected profile must create no buffer"
    );
    assert_eq!(state.terminal_manager.borrow().len(), 0);
}

/// Acceptance 2: an unknown profile names the known ones and creates nothing.
#[test]
fn acc2_unknown_profile_lists_known_names_and_creates_nothing() {
    let state = EditorState::new();
    exec(&state, CAT_PROFILE);
    exec(
        &state,
        r#"pmacs.terminal.profiles.other = { command = "/bin/sh" }"#,
    );
    let before = state.core.borrow().registry.borrow().ids().len();

    // Via the default setting.
    exec(
        &state,
        r#"pmacs.config.set("terminal.default-profile", "ghost")"#,
    );
    let err = eval_err(&state, "return pmacs.terminal.open {}");
    assert!(err.contains("ghost"), "names the missing profile: {err}");
    assert!(
        err.contains("echo") && err.contains("other"),
        "must LIST the known profiles: {err}"
    );

    // An explicit bad profile fails even though the default is now valid —
    // a typo must not silently fall back (Q#TC3a).
    exec(
        &state,
        r#"pmacs.config.set("terminal.default-profile", "echo")"#,
    );
    let err = eval_err(&state, r#"return pmacs.terminal.open { profile = "typo" }"#);
    assert!(err.contains("typo"), "explicit bad profile errors: {err}");

    assert_eq!(
        state.core.borrow().registry.borrow().ids().len(),
        before,
        "no buffer, session, or process is created"
    );
    assert_eq!(state.terminal_manager.borrow().len(), 0);
}

/// Acceptance 3: explicit beats profile beats setting beats `$SHELL`, and
/// `env` MERGES rather than replacing.
#[test]
fn acc3_field_resolution_order_and_env_merge() {
    let mut state = EditorState::new();
    exec(
        &state,
        r#"
        pmacs.terminal.profiles.merged = {
          command = "/bin/sh",
          args = { "-c", "printf 'PROFILE:%s:%s\r\n' \"$FROM_PROFILE\" \"$SHARED\"; exec cat" },
          env = { FROM_PROFILE = "p", SHARED = "profile" },
        }
        "#,
    );
    let buffer = open_cat_terminal(
        &state,
        r#"profile = "merged", env = { SHARED = "explicit" }"#,
    );
    assert!(
        tick_until(&mut state, "PROFILE:p:explicit", buffer),
        "profile env survives and explicit env overrides the same key: {:?}",
        screen_text(&state, buffer)
    );
    state.process_supervisor.borrow_mut().shutdown();
}

/// Acceptance 3 (explicit command wins) and 4 (`""` means no profile).
#[test]
fn acc3_acc4_explicit_command_wins_and_empty_default_means_no_profile() {
    let mut state = EditorState::new();
    exec(&state, CAT_PROFILE);
    exec(
        &state,
        r#"pmacs.config.set("terminal.default-profile", "echo")"#,
    );

    // Explicit command beats the profile's.
    let explicit = open_cat_terminal(
        &state,
        r#"command = "/bin/sh", args = { "-c", "printf 'EXPLICIT\r\n'; exec cat" }"#,
    );
    assert!(tick_until(&mut state, "EXPLICIT", explicit));

    // `""` is the no-profile sentinel: falls through to $SHELL.
    exec(
        &state,
        r#"pmacs.config.set("terminal.default-profile", "")"#,
    );
    let bare = open_cat_terminal(&state, "");
    let spec_ok = state.terminal_manager.borrow().is_terminal(bare);
    assert!(spec_ok, "an empty default must open a $SHELL terminal");
    assert!(
        !screen_text(&state, bare).contains("READY"),
        "the echo profile must NOT have been applied"
    );
    state.process_supervisor.borrow_mut().shutdown();
}

/// Acceptance 5: scrollback resolves from the setting, is overridden by an
/// explicit value, and `0` is legal.
#[test]
fn acc5_scrollback_setting_override_and_bounds() {
    let state = EditorState::new();
    exec(&state, r#"pmacs.config.set("terminal.scrollback-rows", 0)"#);
    assert_eq!(
        state
            .lua_host
            .lua()
            .load(r#"return pmacs.config.get("terminal.scrollback-rows")"#)
            .eval::<i64>()
            .unwrap(),
        0,
        "0 is a legal scrollback value meaning 'retain no history'"
    );

    let err = eval_err(
        &state,
        r#"return pmacs.config.set("terminal.scrollback-rows", -1)"#,
    );
    assert!(
        err.contains("-1") || err.contains("min"),
        "below range: {err}"
    );
    let err = eval_err(
        &state,
        r#"return pmacs.config.set("terminal.scrollback-rows", 4000001)"#,
    );
    assert!(
        err.contains("4000001") || err.contains("max"),
        "above range: {err}"
    );
}

/// Acceptance 6 and 9: the configured chord escapes, repeating it sends
/// THAT chord to the child, and an ordinary `C-c` still reaches the child.
#[test]
fn acc6_acc9_configured_escape_chord_and_literal_repeat() {
    let mut state = EditorState::new();
    exec(&state, CAT_PROFILE);
    let buffer = open_cat_terminal(&state, r#"profile = "echo""#);
    assert!(tick_until(&mut state, "READY", buffer));
    focus_terminal(&state, buffer);

    exec(&state, r#"pmacs.config.set("terminal.escape-key", "C-x")"#);

    // `C-x C-x` must send Ctrl-X (0x18), which `cat` echoes back. Against
    // the pre-change hardcoded `&[0x03]` this sends Ctrl-C instead.
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
    );
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
    );
    assert!(
        tick_until(&mut state, "^X", buffer),
        "C-x C-x must send literal Ctrl-X: {:?}",
        screen_text(&state, buffer)
    );

    // With the escape moved, an ordinary C-c is just another key.
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    assert!(
        tick_until(&mut state, "^C", buffer),
        "plain C-c must reach the child once the escape moved: {:?}",
        screen_text(&state, buffer)
    );
    state.process_supervisor.borrow_mut().shutdown();
}

/// Acceptance 7, 8 and 8a: per-terminal escape resolution, an A→B→A parse
/// count that does not grow, and a cache that dies with its terminal.
#[test]
fn acc7_acc8_acc8a_per_terminal_escape_cache_identity_and_lifecycle() {
    let mut state = EditorState::new();
    exec(&state, CAT_PROFILE);
    let a = open_cat_terminal(&state, r#"profile = "echo""#);
    exec(&state, "TERM_A = TERM_BUF");
    let b = open_cat_terminal(&state, r#"profile = "echo""#);
    exec(&state, "TERM_B = TERM_BUF");
    assert!(tick_until(&mut state, "READY", a));
    assert!(tick_until(&mut state, "READY", b));

    // Different buffer-local escapes, then NO further writes.
    exec(
        &state,
        r#"pmacs.config.set_local(TERM_A, "terminal.escape-key", "C-x")"#,
    );
    exec(
        &state,
        r#"pmacs.config.set_local(TERM_B, "terminal.escape-key", "C-b")"#,
    );

    // Prime both caches. Each priming press ARMS the escape, so it is
    // consumed with a probe — otherwise the next chord would be read as
    // the escape repeat rather than a fresh escape.
    focus_terminal(&state, a);
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
    );
    assert!(escape_was_armed(&mut state, a, 'M'), "A primes on its C-x");
    focus_terminal(&state, b);
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
    );
    assert!(escape_was_armed(&mut state, b, 'N'), "B primes on its C-b");
    let primed = state.terminal_manager.borrow().escape_parses();

    // Acceptance 7 — BOTH directions. Asserting only that A still works
    // after A->B->A is not enough: an epoch-only cache hands whichever
    // entry it finds to every terminal, so A keeps working by accident
    // while B silently inherits A's chord. The discriminating assertion
    // is that EACH terminal honors its OWN chord and NOT the other's.
    focus_terminal(&state, b);
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
    );
    assert!(
        escape_was_armed(&mut state, b, 'R'),
        "terminal B must escape on its own C-b"
    );
    // ...and A's chord must be ordinary input in B, not an escape.
    focus_terminal(&state, b);
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
    );
    assert!(
        !escape_was_armed(&mut state, b, 'S'),
        "terminal A's C-x must NOT escape terminal B"
    );

    focus_terminal(&state, a);
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
    );
    assert!(
        escape_was_armed(&mut state, a, 'Q'),
        "terminal A must still escape on its own C-x after A->B->A"
    );

    // Acceptance 8: that round trip parsed nothing new. A single
    // last-entry cache would have reparsed twice.
    assert_eq!(
        state.terminal_manager.borrow().escape_parses(),
        primed,
        "A->B->A with no setting written must not reparse"
    );

    // Acceptance 8a: the cache dies with its terminal.
    let sessions_before = state.terminal_manager.borrow().len();
    exec(&state, "pmacs.terminal.terminate(TERM_A)");
    exec(&state, "pmacs.buffer.kill(TERM_A)");
    // Pruning is tick-driven (the manager reaps on the process tick), so
    // the session outlives the kill call by design.
    let deadline = Instant::now() + Duration::from_secs(5);
    while state.terminal_manager.borrow().len() >= sessions_before {
        state.tick_processes();
        assert!(
            Instant::now() < deadline,
            "killing the terminal must remove its session, and with it the cache"
        );
        thread::sleep(Duration::from_millis(20));
    }
    state.process_supervisor.borrow_mut().shutdown();
}

/// Acceptance 10 and 10a: an unparseable value falls back, reports through
/// the status line, and reports once per terminal per effective bad value.
#[test]
fn acc10_acc10a_invalid_escape_falls_back_and_reports_once() {
    let mut state = EditorState::new();
    exec(&state, CAT_PROFILE);
    let buffer = open_cat_terminal(&state, r#"profile = "echo""#);
    assert!(tick_until(&mut state, "READY", buffer));
    focus_terminal(&state, buffer);

    exec(
        &state,
        r#"pmacs.config.set("terminal.escape-key", "not-a-chord")"#,
    );
    state.core.borrow_mut().status.clear();

    // Acceptance 10: falls back to C-c, so the terminal stays escapable.
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    // Read the report BEFORE probing: `status` is a single slot, and the
    // probe key's own rejected self-insert would overwrite it.
    let reported = state.core.borrow().status.clone();
    assert!(
        reported.contains("terminal.escape-key") && reported.contains("not-a-chord"),
        "the report must name the setting and the bad value: {reported:?}"
    );
    assert!(
        escape_was_armed(&mut state, buffer, 'Q'),
        "an invalid escape-key must fall back to C-c, not leave the \
         terminal unescapable"
    );

    // Acceptance 10a: the same bad value does not report again.
    state.core.borrow_mut().status.clear();
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    assert!(
        state.core.borrow().status.is_empty(),
        "an unchanged invalid value must not re-report: {:?}",
        state.core.borrow().status
    );
    let _ = escape_was_armed(&mut state, buffer, 'W');

    // A DIFFERENT bad value is new information, so it reports again.
    exec(
        &state,
        r#"pmacs.config.set("terminal.escape-key", "also-bad")"#,
    );
    state.core.borrow_mut().status.clear();
    state.dispatch_key(
        FrontendId::LOCAL,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    assert!(
        state.core.borrow().status.contains("also-bad"),
        "a different invalid value must report: {:?}",
        state.core.borrow().status
    );
    state.process_supervisor.borrow_mut().shutdown();
}

/// Acceptance 11: the opening binding exists, resolves to the command, and
/// shadowed nothing (`keymap.bind` is strict, so loading the runtime at all
/// proves the second half).
#[test]
fn acc11_terminal_opening_binding_is_bound_and_shadowed_nothing() {
    let state = EditorState::new();
    let command: Option<String> = state
        .lua_host
        .lua()
        .load(r#"local d = pmacs.describe.key("C-c t"); return d and d.command"#)
        .eval()
        .expect("describe.key");
    assert_eq!(
        command.as_deref(),
        Some("terminal"),
        "C-c t must open a terminal"
    );
}

/// Acceptance 12: with no settings written and no profiles registered, the
/// defaults reproduce the pre-arc behavior.
#[test]
fn acc12_defaults_reproduce_prior_behavior() {
    let state = EditorState::new();
    let lua = state.lua_host.lua();
    assert_eq!(
        lua.load(r#"return pmacs.config.get("terminal.default-profile")"#)
            .eval::<String>()
            .unwrap(),
        ""
    );
    assert_eq!(
        lua.load(r#"return pmacs.config.get("terminal.scrollback-rows")"#)
            .eval::<i64>()
            .unwrap(),
        10_000
    );
    assert_eq!(
        lua.load(r#"return pmacs.config.get("terminal.escape-key")"#)
            .eval::<String>()
            .unwrap(),
        "C-c"
    );
    assert!(
        lua.load("return next(pmacs.terminal.profiles) == nil")
            .eval::<bool>()
            .unwrap(),
        "no profiles are registered by default"
    );
}
