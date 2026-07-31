//! Lean 4 mode, Stage 1 acceptance (Arc 8, `docs/lean4-mode-framing.md`).
//!
//! Covers the framing's Stage 1 criteria that live above the Rust
//! substrate — major mode, modeline aliasing, comment toggle, the pair
//! set, and markdown fence injection. Criteria 1, 2, and the Q#LN4
//! retro-paint pins (7, 8) are unit tests in `src/syntax.rs` and
//! `src/highlight.rs`, where the theme table and grammar registry live.
//!
//! Dispatch-driven, following `comment_toggle_acceptance`: `M-;` and
//! typed characters go through `dispatch_key` so the real command
//! boundary and typed-edit provenance are exercised. Buffers are
//! file-backed (language detection needs a path); each editor gets a
//! private tempdir `StateDir` and an emptied `pmacs.lsp.config` so
//! nothing spawns a language server — Stage 1 has no LSP at all.
//!
//! Criterion 12 is the reason this suite touches no process: it must
//! pass on a machine with no `lean`, no `lake`, and no configured elan
//! toolchain. That is not hypothetical — the machine this arc was
//! scouted on has elan installed with no default toolchain, where
//! `lake --version` itself fails.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fresh_state_dir() -> PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pmacs-lean4-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn editor(state_dir: &std::path::Path) -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host
        .lua()
        .set_app_data(StateDir(state_dir.to_path_buf()));
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

fn alt(s: &mut EditorState, c: char) {
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::ALT));
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn buffer_text(s: &EditorState) -> String {
    let b: mlua::String = eval(
        s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
}

fn cursor(s: &EditorState) -> i64 {
    eval(s, "return pmacs.editor.cursor()")
}

/// Fresh editor visiting `name` (created in the state tempdir) with
/// `body` on disk, cursor at 0.
fn editor_visiting(name: &str, body: &str) -> EditorState {
    let dir = fresh_state_dir();
    let s = editor(&dir);
    let f = write_file(&dir, name, body);
    exec(&s, &format!("pmacs.buffer.find_or_open({f:?})"));
    exec(&s, "pmacs.editor.goto_byte(0)");
    s
}

fn major_mode(s: &EditorState) -> Option<String> {
    eval(s, "return pmacs.buffer.major_mode(pmacs.window.buffer())")
}

// ---------------------------------------------------------------------------
// Criterion 3 — major mode
// ---------------------------------------------------------------------------

#[test]
fn acc3_opening_a_lean_file_sets_the_lean4_major_mode() {
    let s = editor_visiting("Basic.lean", "def x : Nat := 1\n");
    assert_eq!(
        major_mode(&s).as_deref(),
        Some("lean4"),
        "a .lean file carries the lean4 major mode"
    );
}

// ---------------------------------------------------------------------------
// Criterion 4 — modeline aliasing (Q#LN2)
// ---------------------------------------------------------------------------

#[test]
fn acc4_emacs_and_vim_modelines_spelling_lean_resolve_to_lean4() {
    // The grammar entry is `lean4`, but `-*- mode: lean -*-` and `ft=lean`
    // are what people write. Both must land on the same mode, or a file
    // with an explicit modeline is stranded with no grammar.
    //
    // Deliberately on a `.txt` path: if the fixture were `.lean`, the
    // extension alone would produce `lean4` and the assertion would pass
    // with the alias table empty — the vacuous shape.
    for body in [
        "-- -*- mode: lean -*-\ndef x : Nat := 1\n",
        "-- vim: ft=lean\ndef x : Nat := 1\n",
    ] {
        let s = editor_visiting("modeline.txt", body);
        assert_eq!(
            major_mode(&s).as_deref(),
            Some("lean4"),
            "modeline {body:?} resolves through the alias to lean4"
        );
    }
}

#[test]
fn acc4b_the_lean_alias_is_load_bearing() {
    // Non-vacuity guard for acc4: with the alias removed, the same
    // fixture resolves to the raw `lean` name instead. If this ever
    // reports `lean4`, acc4 is proving nothing.
    let s = editor_visiting("modeline.txt", "x\n");
    exec(&s, "pmacs.parse.modeline_aliases.lean = nil");
    let dir = fresh_state_dir();
    let f = write_file(&dir, "other.txt", "-- -*- mode: lean -*-\ndef x := 1\n");
    exec(&s, &format!("pmacs.buffer.find_or_open({f:?})"));
    assert_eq!(
        major_mode(&s).as_deref(),
        Some("lean"),
        "without the alias the modeline name is not normalized"
    );
}

// ---------------------------------------------------------------------------
// Criterion 9 — comment toggle (Q#LN5)
// ---------------------------------------------------------------------------

#[test]
fn acc9_comment_toggle_round_trips_with_the_dash_dash_prefix() {
    let mut s = editor_visiting("Basic.lean", "def x : Nat := 1\ndef y : Nat := 2\n");
    exec(&s, "pmacs.editor.goto_byte(0)");
    alt(&mut s, ';');
    assert_eq!(
        buffer_text(&s),
        "-- def x : Nat := 1\ndef y : Nat := 2\n",
        "M-; comments a Lean line with `-- `"
    );
    // Round trip, including the padding space.
    exec(&s, "pmacs.editor.goto_byte(0)");
    alt(&mut s, ';');
    assert_eq!(
        buffer_text(&s),
        "def x : Nat := 1\ndef y : Nat := 2\n",
        "M-; uncomments it exactly"
    );
}

// ---------------------------------------------------------------------------
// Criterion 10 — pairs (Q#LN6)
// ---------------------------------------------------------------------------

#[test]
fn acc10_lean_bracket_pairs_close_and_the_prime_does_not() {
    // The three Unicode brackets are the reason this decision exists: all
    // are outside the nine built-in pair chars, so they exercise the
    // user-extended pair path rather than the frontends' optimistic
    // classifier.
    for (opener, expected) in [("⟨", "⟨⟩"), ("⦃", "⦃⦄"), ("⟮", "⟮⟯")] {
        let mut s = editor_visiting("Basic.lean", "");
        exec(&s, "pmacs.editor.goto_byte(0)");
        type_str(&mut s, opener);
        assert_eq!(
            buffer_text(&s),
            expected,
            "typing {opener} inserts the closing half"
        );
        assert_eq!(
            cursor(&s),
            i64::try_from(opener.len()).expect("opener length fits"),
            "the point sits between the pair"
        );
    }
}

#[test]
fn acc10b_the_prime_suffix_does_not_pair_in_lean() {
    // Lean uses `'` as a primed-identifier suffix (`h'`, `foo'`), so
    // pairing it would fight the user on nearly every proof.
    let mut s = editor_visiting("Basic.lean", "");
    exec(&s, "pmacs.editor.goto_byte(0)");
    type_str(&mut s, "h'");
    assert_eq!(
        buffer_text(&s),
        "h'",
        "the prime is a suffix in Lean, not an opener"
    );
}

// ---------------------------------------------------------------------------
// Criterion 11 — markdown fences (Q#LN17)
// ---------------------------------------------------------------------------

/// Parse `src` as markdown and return the child layer language names.
///
/// Goes through the real `_parse_now` injection path rather than reading
/// the alias table: `pmacs.parse.injection_aliases` is a documented
/// WRITE-ONLY proxy (the canonical map lives Rust-side), so an
/// alias-table read would prove nothing about what the parser does.
fn markdown_layer_languages(src: &[u8]) -> Vec<String> {
    let state = EditorState::new_with_roots(&crate::iso::roots());
    let buf_id = state
        .lua_host
        .registry()
        .borrow_mut()
        .create_from_bytes("doc.md".to_owned(), src);
    state
        .lua_host
        .lua()
        .globals()
        .set("BUF", pmacs::lua_bindings::BufferIdLua(buf_id))
        .expect("bind BUF");
    state
        .lua_host
        .lua()
        .load("pmacs.parse._parse_now(BUF, 'markdown')")
        .exec()
        .expect("synchronous parse");
    let bundle = state
        .syntax_registry
        .view(buf_id)
        .and_then(|h| h.current())
        .expect("installed bundle");
    bundle
        .layers
        .iter()
        .map(|l| l.language_name.clone())
        .collect()
}

#[test]
fn acc11_lean_and_lean4_markdown_fences_both_inject_the_lean_grammar() {
    // Both spellings must resolve to the same grammar: `lean4` is the entry
    // name and `lean` goes through the injection alias. A ```lean fence is
    // overwhelmingly Lean 4 in practice, which is why the Lean 3 spelling
    // is mapped forward rather than left unresolved (Q#LN17).
    for fence in ["lean", "lean4"] {
        let src = format!("# Doc\n\n```{fence}\ndef x : Nat := 1\n```\n");
        let langs = markdown_layer_languages(src.as_bytes());
        assert!(
            langs.iter().any(|l| l == "lean4"),
            "```{fence} injects a lean4 child layer; got {langs:?}"
        );
    }
}

#[test]
fn acc11b_an_unknown_fence_name_still_injects_nothing() {
    // Non-vacuity guard for acc11: the alias must be what resolves `lean`,
    // not some catch-all that would light up any fence name.
    let langs = markdown_layer_languages(b"# Doc\n\n```leen\ndef x := 1\n```\n");
    assert!(
        !langs.iter().any(|l| l == "lean4"),
        "a misspelled fence must not reach the lean4 grammar; got {langs:?}"
    );
}

// ---------------------------------------------------------------------------
// Criterion 12 — no toolchain required
// ---------------------------------------------------------------------------

#[test]
fn acc12_opening_lean_spawns_no_process_without_a_server_config() {
    // **Superseded in half by Stage 3b.** This criterion originally also
    // asserted `pmacs.lsp.config.lean4 == nil`, guarding against a Stage-3
    // front-run. Stage 3b *is* Stage 3: `builtin/runtime/lean.lua` now ships
    // that config deliberately, and its shape is pinned by
    // `tests/lean4_server_acceptance.rs`. Asserting the absence here would
    // now pin the opposite of the intended behavior, so it is gone rather
    // than weakened.
    //
    // What survives is the half that was always about *restraint*, and it
    // matters more now than it did in Stage 1 — it is what holds Q#LN7's
    // "not at init" promise. `pmacs.lsp.config` is a declarative table, and
    // spawning a process at startup for every user, Lean-using or not, is
    // the cost rev 1 refused. Both the `lake serve` spawn and the
    // `lake --version` probe are gated on a real Lean attachment.

    // Constructing an editor touches no process, even though the Lean
    // config now exists and names `lake`.
    let pristine = EditorState::new_with_roots(&crate::iso::roots());
    let at_init: i64 = eval(&pristine, "return #pmacs.process.list()");
    assert_eq!(
        at_init, 0,
        "constructing an editor must not probe or spawn for Lean"
    );
    // Non-vacuity for the assertion above: the config really is present and
    // really does name a command, so "nothing spawned" is restraint rather
    // than an empty table having nothing to act on.
    let names_lake: bool = eval(
        &pristine,
        "return pmacs.lsp.config.lean4 ~= nil and pmacs.lsp.config.lean4.command == \"lake\"",
    );
    assert!(
        names_lake,
        "Stage 3b ships a lean4 config naming `lake`, so the no-spawn \
         assertion above is meaningful"
    );

    // And opening a Lean buffer with no server configured spawns nothing —
    // the `editor()` helper wipes `pmacs.lsp.config`, so this catches a
    // probe that fires off the mode rather than off an attachment.
    let s = editor_visiting("Basic.lean", "def x : Nat := 1\n");
    let procs: i64 = eval(&s, "return #pmacs.process.list()");
    assert_eq!(
        procs, 0,
        "with no server configured, opening a Lean buffer spawns nothing"
    );
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
