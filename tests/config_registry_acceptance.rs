//! Config-registry acceptance (docs/config-registry-framing.md).
//!
//! The registry's own semantics are unit-tested in
//! `src/config_registry.rs` (value/scope/epoch/listener behavior) and
//! `src/lua_bindings/config.rs` (the Lua boundary). This suite covers
//! only what those cannot reach: the three adopters wired into a real
//! `EditorState`, the owner-defines source-location contract observed
//! after actual chunk load, and `M-x describe-setting` rendering
//! through the real minibuffer.
//!
//! Framing acceptance items covered here: 9, 19, 26, 27, 28, 29, 30, 33.
//!
//! Pairing is exercised by DISPATCHING keys, never
//! `pmacs.command.invoke` — pair.lua reacts to `buffer.after-edit`
//! with a typed-edit record that only real dispatch produces, so an
//! invoke-driven test would pass vacuously against a broken gate.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;

// ---------------------------------------------------------------------------
// Harness (mirrors tests/auto_pair_acceptance.rs)
// ---------------------------------------------------------------------------

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn fresh_state_dir() -> PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pmacs-configreg-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn editor(state_dir: &std::path::Path) -> EditorState {
    let s = EditorState::new();
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host
        .lua()
        .set_app_data(StateDir(state_dir.to_path_buf()));
    // Language DETECTION must work; server SPAWNING must not (rust and
    // python carry default configs and the after-load hook would spawn
    // real servers).
    exec(&s, "pmacs.lsp.config = {}");
    s
}

fn write_file(dir: &std::path::Path, name: &str, body: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p.display().to_string()
}

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn active_text(s: &EditorState) -> String {
    let b: mlua::String = eval(
        s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
}

/// `show_help_text` ends by switching the window to `*help*`, so after
/// a describe command the active buffer IS the help buffer. Asserting
/// through the active buffer also proves the switch happened, which a
/// direct by-name lookup would silently tolerate skipping.
fn help_text(s: &EditorState) -> String {
    let name: String = eval(s, "return pmacs.window.buffer():name()");
    assert_eq!(name, "*help*", "the describe command must display *help*");
    active_text(s)
}

// ---------------------------------------------------------------------------
// Item 26 / 9 — owner-defines, observed after real chunk load
// ---------------------------------------------------------------------------

#[test]
fn builtin_defines_succeed_at_chunk_load_and_report_their_owning_module() {
    let s = editor(&fresh_state_dir());

    // Item 26: the define calls at the top of pair.lua / editops.lua /
    // autosave.lua ran during EditorState::new, which is only possible
    // if pmacs.config was populated before the first runtime chunk.
    for name in [
        "editing.auto-pair",
        "editing.trim-on-save",
        "autosave.interval-ms",
    ] {
        let known: bool = eval(
            &s,
            &format!("return pmacs.config.describe({name:?}) ~= nil"),
        );
        assert!(known, "{name} must be defined by its owning module");
    }

    // Item 9: each definition's SourceLocation points at the module
    // that owns the setting, not at a shared helper. This is exactly
    // what a centralized define table in a config.lua would have
    // broken (framing Q#CR14).
    let pair_src: String = eval(
        &s,
        "return pmacs.config.describe('editing.auto-pair').source",
    );
    assert!(
        pair_src.contains("pair.lua"),
        "editing.auto-pair must report pair.lua as its source, got {pair_src:?}"
    );
    let trim_src: String = eval(
        &s,
        "return pmacs.config.describe('editing.trim-on-save').source",
    );
    assert!(
        trim_src.contains("editops.lua"),
        "editing.trim-on-save must report editops.lua, got {trim_src:?}"
    );
    let auto_src: String = eval(
        &s,
        "return pmacs.config.describe('autosave.interval-ms').source",
    );
    assert!(
        auto_src.contains("autosave.lua"),
        "autosave.interval-ms must report autosave.lua, got {auto_src:?}"
    );
}

// ---------------------------------------------------------------------------
// Item 29 — the flagship: per-buffer auto-pair, the feature this arc exists for
// ---------------------------------------------------------------------------

#[test]
fn auto_pair_off_buffer_locally_suppresses_only_that_buffer() {
    let dir = fresh_state_dir();
    let mut s = editor(&dir);
    let a = write_file(&dir, "a.rs", "");
    let b = write_file(&dir, "b.rs", "");

    // Two buffers of the SAME language — so a language-keyed
    // implementation could not pass this test. The handle is stashed in
    // a Lua global because buffer handles are userdata, not ids.
    exec(&s, &format!("BUF_A = pmacs.buffer.find_or_open({a:?})"));
    exec(&s, &format!("pmacs.buffer.find_or_open({b:?})"));

    // Turn pairing off in buffer A only.
    exec(
        &s,
        "pmacs.config.set_local(BUF_A, 'editing.auto-pair', false)",
    );

    // B (untouched) still pairs.
    exec(&s, "pmacs.editor.goto_byte(0)");
    type_str(&mut s, "(");
    assert_eq!(
        active_text(&s),
        "()",
        "a buffer with no local override still pairs"
    );

    // A (overridden) does not.
    exec(&s, &format!("pmacs.buffer.find_or_open({a:?})"));
    exec(&s, "pmacs.editor.goto_byte(0)");
    type_str(&mut s, "(");
    assert_eq!(
        active_text(&s),
        "(",
        "the buffer-local override must suppress pairing in THIS buffer"
    );
}

#[test]
fn auto_pair_off_globally_suppresses_everywhere() {
    let dir = fresh_state_dir();
    let mut s = editor(&dir);
    let a = write_file(&dir, "a.rs", "");
    exec(&s, &format!("pmacs.buffer.find_or_open({a:?})"));
    exec(&s, "pmacs.config.set('editing.auto-pair', false)");
    exec(&s, "pmacs.editor.goto_byte(0)");
    type_str(&mut s, "(");
    assert_eq!(active_text(&s), "(", "a global false suppresses pairing");
}

#[test]
fn auto_pair_defaults_on_so_the_migration_changed_no_default() {
    let dir = fresh_state_dir();
    let mut s = editor(&dir);
    let a = write_file(&dir, "a.rs", "");
    exec(&s, &format!("pmacs.buffer.find_or_open({a:?})"));
    exec(&s, "pmacs.editor.goto_byte(0)");
    type_str(&mut s, "(");
    assert_eq!(active_text(&s), "()", "pairing is on by default");
}

// ---------------------------------------------------------------------------
// Items 27 / 28 — the migration wrappers keep their legacy coercion (F4)
// ---------------------------------------------------------------------------

#[test]
fn trim_on_save_wrapper_and_registry_are_interchangeable_both_ways() {
    let s = editor(&fresh_state_dir());

    // Wrapper write observed by the registry.
    exec(&s, "pmacs.editops.trim_on_save(true)");
    let via_registry: bool = eval(&s, "return pmacs.config.get('editing.trim-on-save')");
    assert!(via_registry, "the wrapper's write must reach the registry");

    // Registry write observed by the wrapper.
    exec(&s, "pmacs.config.set('editing.trim-on-save', false)");
    let via_wrapper: bool = eval(&s, "return pmacs.editops.trim_on_save()");
    assert!(
        !via_wrapper,
        "the registry's write must be visible through the wrapper"
    );
}

#[test]
fn trim_on_save_keeps_its_lenient_truthiness() {
    // F4: the registry is strict (a real boolean or nothing), but this
    // legacy setter has always accepted anything that is not literally
    // `false`. A thin wrapper over a strict `set` would raise here.
    let s = editor(&fresh_state_dir());
    exec(&s, "pmacs.editops.trim_on_save('yes')");
    let on: bool = eval(&s, "return pmacs.config.get('editing.trim-on-save')");
    assert!(on, "a non-false argument must still turn trimming on");

    exec(&s, "pmacs.editops.trim_on_save(false)");
    let off: bool = eval(&s, "return pmacs.config.get('editing.trim-on-save')");
    assert!(!off, "a literal false must still turn it off");
}

#[test]
fn interval_ms_keeps_flooring_a_fractional_argument() {
    // F4 again: `integer` demands exactness, so the wrapper must floor
    // BEFORE handing the value over. Pre-migration this returned 1500.
    let s = editor(&fresh_state_dir());
    let got: i64 = eval(&s, "return pmacs.autosave.interval_ms(1500.7)");
    assert_eq!(got, 1500, "a fractional interval floors, it does not raise");
    let stored: i64 = eval(&s, "return pmacs.config.get('autosave.interval-ms')");
    assert_eq!(stored, 1500, "and the floored value is what was stored");
}

#[test]
fn interval_ms_still_raises_below_the_floor() {
    let s = editor(&fresh_state_dir());
    let raised: bool = eval(
        &s,
        "local ok = pcall(pmacs.autosave.interval_ms, 500); return not ok",
    );
    assert!(raised, "sub-floor intervals must still raise");
    let unchanged: i64 = eval(&s, "return pmacs.autosave.interval_ms()");
    assert_eq!(unchanged, 30000, "a rejected set leaves the value alone");
}

// ---------------------------------------------------------------------------
// Item 30 — a direct registry write is what the tick will read
// ---------------------------------------------------------------------------

#[test]
fn interval_change_through_the_registry_is_visible_to_the_cadence_reader() {
    // The tick re-reads `pmacs.config.get("autosave.interval-ms")` every
    // frame rather than a module-local, so a mid-session change through
    // EITHER path applies without a restart. Asserting through the
    // wrapper's getter proves the module-local is really gone: a stale
    // upvalue would still report 30000 here.
    let s = editor(&fresh_state_dir());
    exec(&s, "pmacs.config.set('autosave.interval-ms', 5000)");
    let seen: i64 = eval(&s, "return pmacs.autosave.interval_ms()");
    assert_eq!(
        seen, 5000,
        "a direct registry write must be what the cadence reads"
    );
}

// ---------------------------------------------------------------------------
// Item 19 — user config runs after builtins define, so a set in init.lua lands
// ---------------------------------------------------------------------------

#[test]
fn a_set_in_user_config_position_is_observed_by_the_consumer() {
    // EditorState::new does not load user config in test builds, so
    // this drives the same ORDER explicitly: every builtin has defined,
    // and a user-config-shaped `set` now runs against those names and
    // is observed by the adopter that owns each one.
    let dir = fresh_state_dir();
    let mut s = editor(&dir);
    exec(
        &s,
        r#"
        pmacs.config.set("editing.auto-pair", false)
        pmacs.config.set("editing.trim-on-save", true)
        pmacs.config.set("autosave.interval-ms", 9000)
        "#,
    );
    assert!(
        eval::<bool>(&s, "return pmacs.editops.trim_on_save()"),
        "editops observes the user-config write"
    );
    assert_eq!(
        eval::<i64>(&s, "return pmacs.autosave.interval_ms()"),
        9000,
        "autosave observes the user-config write"
    );
    let a = write_file(&dir, "a.rs", "");
    exec(&s, &format!("pmacs.buffer.find_or_open({a:?})"));
    exec(&s, "pmacs.editor.goto_byte(0)");
    type_str(&mut s, "(");
    assert_eq!(
        active_text(&s),
        "(",
        "pair.lua observes the user-config write"
    );
}

// ---------------------------------------------------------------------------
// Item 33 — M-x describe-setting renders into *help*
// ---------------------------------------------------------------------------

#[test]
fn describe_setting_renders_into_help_with_the_source_location() {
    let s0 = editor(&fresh_state_dir());
    let mut s = s0;
    exec(&s, "pmacs.command.invoke('editor.describe-setting')");
    type_str(&mut s, "editing.auto-pair");
    press(&mut s, KeyCode::Enter);

    let text = help_text(&s);
    assert!(
        text.contains("Setting: editing.auto-pair"),
        "*help* must carry the setting header, got {text:?}"
    );
    assert!(
        text.contains("pair.lua"),
        "the rendered source location must name the owning module, got {text:?}"
    );
    assert!(
        text.contains("Type:") && text.contains("boolean"),
        "the type must be rendered, got {text:?}"
    );
    assert!(
        text.contains("Mutability:") && text.contains("live"),
        "mutability must be rendered, got {text:?}"
    );
}

#[test]
fn describe_setting_reports_an_unknown_name_in_the_status_line() {
    // `describe` RAISES NotFound for an undefined name rather than
    // returning nil (define-before-set, Q#CR10), so the command must
    // pcall — without it this dispatch would surface a Lua traceback.
    let s0 = editor(&fresh_state_dir());
    let mut s = s0;
    exec(&s, "pmacs.command.invoke('editor.describe-setting')");
    type_str(&mut s, "editing.no-such-setting");
    press(&mut s, KeyCode::Enter);

    let status = s.core.borrow().status.clone();
    assert!(
        format!("{status:?}").contains("no such setting"),
        "an unknown name must report cleanly, got {status:?}"
    );
}

#[test]
fn describe_setting_shows_a_buffer_local_override_when_one_exists() {
    let dir = fresh_state_dir();
    let mut s = editor(&dir);
    let a = write_file(&dir, "a.rs", "");
    exec(&s, &format!("pmacs.buffer.find_or_open({a:?})"));
    exec(
        &s,
        "pmacs.config.set_local(pmacs.window.buffer(), 'editing.auto-pair', false)",
    );
    exec(&s, "pmacs.command.invoke('editor.describe-setting')");
    type_str(&mut s, "editing.auto-pair");
    press(&mut s, KeyCode::Enter);

    let text = help_text(&s);
    assert!(
        text.contains("Buffer-local override:"),
        "an existing buffer-local override must be reported, got {text:?}"
    );
}
