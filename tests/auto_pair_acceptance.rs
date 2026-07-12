//! Auto-pairing acceptance (Arc 2, docs/auto-pairing-framing.md).
//!
//! Dispatch-driven: pair chars round-trip through `dispatch_key`
//! (Q#AP1 removed them from both optimistic classifiers, so this IS
//! the production path for both frontends). Scratch-buffer tests cover
//! the default pair set; per-language tests visit file-backed buffers
//! with an emptied `pmacs.lsp.config` (language DETECTION must work,
//! server SPAWNING must not); the Q#AP7/Q#AP8 ordering tests drive the
//! real fake-LSP `sighelp` mode and replay the exact document-sync
//! sequence the server received via `PMACS_FAKE_LSP_CHANGE_SINK`.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

fn fresh_state_dir() -> PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pmacs-autopair-{}-{}",
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
    // Language DETECTION must work (filetypes/grammars); server
    // SPAWNING must not (rust/python have default configs).
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

fn ctrl(s: &mut EditorState, c: char) {
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char(c), KeyModifiers::CONTROL),
    );
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
}

/// `C-x u` — the always-dispatched undo (daemon-peer history).
fn undo(s: &mut EditorState) {
    ctrl(s, 'x');
    press(s, KeyCode::Char('u'));
}

/// `C-x r` — redo.
fn redo(s: &mut EditorState) {
    ctrl(s, 'x');
    press(s, KeyCode::Char('r'));
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

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

/// Fresh scratch-buffer editor whose buffer holds `body`, cursor at 0.
/// No state dir / no files: scratch pairing uses the `default` set.
fn editor_with(body: &str) -> EditorState {
    let s = EditorState::new();
    if !body.is_empty() {
        exec(&s, &format!("pmacs.window.buffer():insert(0, {body:?})"));
    }
    exec(&s, "pmacs.editor.goto_byte(0)");
    s
}

/// Fresh editor visiting `name` (created in a private tempdir) with
/// `body` on disk, cursor at 0, `pmacs.lsp.config` emptied.
fn editor_visiting(name: &str, body: &str) -> EditorState {
    let dir = fresh_state_dir();
    let s = editor(&dir);
    let f = write_file(&dir, name, body);
    exec(&s, &format!("pmacs.buffer.find_or_open({f:?})"));
    exec(&s, "pmacs.editor.goto_byte(0)");
    s
}

// ---------------------------------------------------------------------------
// Insert-pair semantics (Q#AP3): the conservative predicate
// ---------------------------------------------------------------------------

#[test]
fn opener_at_end_of_buffer_pairs_with_cursor_between() {
    let mut s = editor_with("");
    type_str(&mut s, "(");
    assert_eq!(buffer_text(&s), "()");
    assert_eq!(cursor(&s), 1, "cursor sits between the pair");
}

#[test]
fn shifted_opener_pairs_too() {
    // Real keyboards produce `(` as Shift+9: the chord arrives as
    // `Char('(')` with SHIFT set and must still self-insert + pair.
    let mut s = editor_with("");
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('('), KeyModifiers::SHIFT),
    );
    assert_eq!(buffer_text(&s), "()");
    assert_eq!(cursor(&s), 1);
}

#[test]
fn opener_at_end_of_line_pairs() {
    let mut s = editor_with("x\ny");
    exec(&s, "pmacs.editor.goto_byte(1)");
    type_str(&mut s, "(");
    assert_eq!(buffer_text(&s), "x()\ny");
    assert_eq!(cursor(&s), 2);
}

#[test]
fn opener_before_whitespace_pairs() {
    let mut s = editor_with("foo bar");
    exec(&s, "pmacs.editor.goto_byte(3)");
    type_str(&mut s, "(");
    assert_eq!(buffer_text(&s), "foo() bar");
    assert_eq!(cursor(&s), 4);
}

#[test]
fn opener_before_closing_bracket_pairs() {
    let mut s = editor_with("()");
    exec(&s, "pmacs.editor.goto_byte(1)");
    type_str(&mut s, "[");
    assert_eq!(buffer_text(&s), "([])");
    assert_eq!(cursor(&s), 2);
}

#[test]
fn opener_before_word_char_does_not_pair() {
    let mut s = editor_with("bar");
    type_str(&mut s, "(");
    assert_eq!(
        buffer_text(&s),
        "(bar",
        "`foo|bar` + `(` gives `(bar`, never `()bar`"
    );
    assert_eq!(cursor(&s), 1);
}

// ---------------------------------------------------------------------------
// Skip-over-close (Q#AP4)
// ---------------------------------------------------------------------------

#[test]
fn closer_skips_over_existing_closer() {
    let mut s = editor_with("");
    type_str(&mut s, "(");
    assert_eq!(buffer_text(&s), "()");
    type_str(&mut s, ")");
    assert_eq!(
        buffer_text(&s),
        "()",
        "the typed `)` steps over, not doubles"
    );
    assert_eq!(cursor(&s), 2, "cursor lands after the closer");
}

#[test]
fn nested_closers_skip_outward() {
    let mut s = editor_with("");
    type_str(&mut s, "((");
    assert_eq!(buffer_text(&s), "(())");
    assert_eq!(cursor(&s), 2);
    type_str(&mut s, "))");
    assert_eq!(buffer_text(&s), "(())", "both closers skip");
    assert_eq!(cursor(&s), 4);
}

#[test]
fn quote_pairs_then_second_quote_exits() {
    let mut s = editor_with("");
    type_str(&mut s, "\"");
    assert_eq!(buffer_text(&s), "\"\"", "symmetric pair inserts");
    assert_eq!(cursor(&s), 1);
    type_str(&mut s, "\"");
    assert_eq!(
        buffer_text(&s),
        "\"\"",
        "second quote skips (exits the string)"
    );
    assert_eq!(cursor(&s), 2);
}

// ---------------------------------------------------------------------------
// Pair sets (Q#AP2): per-language, conservative default
// ---------------------------------------------------------------------------

#[test]
fn single_quote_pairs_in_python_but_not_rust() {
    let mut py = editor_visiting("a.py", "");
    type_str(&mut py, "'");
    assert_eq!(buffer_text(&py), "''", "python's set adds `''`");

    let mut rs = editor_visiting("a.rs", "");
    type_str(&mut rs, "'");
    assert_eq!(
        buffer_text(&rs),
        "'",
        "rust keeps the default set: `'` is a lifetime, not a pair"
    );
}

#[test]
fn malformed_pair_entries_are_skipped_not_partially_honored() {
    // PR #110 round 1, finding 3: an entry is exactly two codepoints.
    // "()x" must be ignored entirely — never "type `(`, get `)x`".
    let mut s = editor_with("");
    exec(&s, "pmacs.pair.sets.default = { \"()x\" }");
    type_str(&mut s, "(");
    assert_eq!(buffer_text(&s), "(", "a malformed entry pairs nothing");
    assert_eq!(cursor(&s), 1);

    // Overlong multibyte entry: same rule after a 2-byte opener.
    let mut s2 = editor_with("");
    exec(&s2, "pmacs.pair.sets.default = { \"\u{ab}\u{bb}x\" }"); // "«»x"
    type_str(&mut s2, "\u{ab}");
    assert_eq!(
        buffer_text(&s2),
        "\u{ab}",
        "an overlong entry pairs nothing"
    );
}

#[test]
fn malformed_utf8_pair_entries_are_rejected() {
    // PR #110 round 2, finding 1: byte-length-from-lead-byte alone is
    // not validation. Every ill-formed closer shape from Unicode
    // Table 3-7 must disqualify the entry — never land in the buffer.
    let cases = [
        // Truncated 2-byte sequence with a trailing ASCII byte: the
        // lead byte "promises" 2 bytes, so lead-length parsing counts
        // "\xC2x" as one codepoint.
        ("string.char(0xC2) .. \"x\"", "truncated sequence"),
        // Overlong encoding of `/` (C0 AF).
        ("string.char(0xC0, 0xAF)", "overlong encoding"),
        // UTF-16 surrogate D800 (ED A0 80).
        ("string.char(0xED, 0xA0, 0x80)", "surrogate encoding"),
        // Beyond U+10FFFF (F5 80 80 80).
        ("string.char(0xF5, 0x80, 0x80, 0x80)", "beyond U+10FFFF"),
    ];
    for (closer, what) in cases {
        let mut s = editor_with("");
        exec(
            &s,
            &format!("pmacs.pair.sets.default = {{ \"(\" .. {closer} }}"),
        );
        type_str(&mut s, "(");
        assert_eq!(buffer_text(&s), "(", "a {what} closer must pair nothing");
    }
}

#[test]
fn multibyte_pair_entries_pair_and_skip() {
    // Two-codepoint entries with multibyte members are valid: guillemets.
    let mut s = editor_with("");
    exec(&s, "pmacs.pair.sets.default = { \"\u{ab}\u{bb}\" }"); // "«»"
    type_str(&mut s, "\u{ab}");
    assert_eq!(buffer_text(&s), "\u{ab}\u{bb}");
    assert_eq!(cursor(&s), 2, "cursor between the pair (byte offset)");
    type_str(&mut s, "\u{bb}");
    assert_eq!(buffer_text(&s), "\u{ab}\u{bb}", "the closer skips");
    assert_eq!(cursor(&s), 4);
}

#[test]
fn non_table_default_set_fails_closed_without_erroring() {
    // PR #110 round 2, finding 3: a config typo assigning a STRING
    // where the set table belongs must behave as an empty set — not
    // throw from the after-edit callback on every keystroke.
    let mut s = editor_with("");
    exec(&s, "pmacs.pair.sets.default = \"()\"");
    type_str(&mut s, "(");
    assert_eq!(buffer_text(&s), "(", "a non-table set pairs nothing");
    let log = s.lua_host.errors_buffer_text();
    assert!(
        !log.contains("pair"),
        "the pairing callback must not error over a config typo; *errors*:\n{log}"
    );
}

#[test]
fn non_table_language_set_falls_back_to_default() {
    // The language entry being junk falls back to `default` (the
    // buffer still deserves pairing), and nothing throws.
    let mut s = editor_visiting("a.rs", "");
    exec(&s, "pmacs.pair.sets.rust = 42");
    type_str(&mut s, "(");
    assert_eq!(
        buffer_text(&s),
        "()",
        "a junk language entry falls back to the default set"
    );
    let log = s.lua_host.errors_buffer_text();
    assert!(
        !log.contains("pair"),
        "the pairing callback must not error over a config typo; *errors*:\n{log}"
    );
}

#[test]
fn scratch_buffer_pairs_the_default_set() {
    let mut s = editor_with("");
    type_str(&mut s, "{");
    assert_eq!(
        buffer_text(&s),
        "{}",
        "language-less buffers pair the default set"
    );
    let mut s2 = editor_with("");
    type_str(&mut s2, "'");
    assert_eq!(
        buffer_text(&s2),
        "'",
        "no apostrophe pairing in the default set"
    );
    let mut s3 = editor_with("");
    type_str(&mut s3, "`");
    assert_eq!(
        buffer_text(&s3),
        "`",
        "no backtick pairing in the default set"
    );
}

// ---------------------------------------------------------------------------
// Non-typed provenance (Q#AP9): no record, no reaction — with the
// after-edit callback actually exercised in every case.
// ---------------------------------------------------------------------------

#[test]
fn paste_of_opener_does_not_pair() {
    let mut s = editor_with("");
    exec(&s, "pmacs.pair._capture_records = true");
    // A prior self-insert, so a heuristic keyed only on buffer text or
    // `char_before` would be primed to misfire. (It also proves the
    // capture seam live: the nil assertion below is a transition from
    // this keystroke's captured record, not an unset field.)
    type_str(&mut s, "a");
    let primed: bool = eval(&s, "return pmacs.pair._last_record ~= nil");
    assert!(primed, "the typed `a` captured a record");
    // The daemon's unified inbound-paste route, faithfully: break the
    // source's command chain, insert, fire the after-edit hook
    // (`handle_inbound_paste` + `with_after_edit_check`).
    s.core.borrow_mut().break_command_chain(FrontendId::LOCAL);
    s.core.borrow_mut().paste_inbound(b"(").unwrap();
    s.lua_host
        .run_hook("buffer.after-edit", mlua::MultiValue::new());
    assert_eq!(buffer_text(&s), "a(", "a pasted opener stays lone");
    let record_nil: bool = eval(&s, "return pmacs.pair._last_record == nil");
    assert!(record_nil, "paste must arm no typed-edit record");
}

#[test]
fn programmatic_insert_with_stale_this_command_does_not_pair() {
    let mut s = editor_with("");
    exec(&s, "pmacs.pair._capture_records = true");
    // Type 'a' so `this_command` is (and stays) "buffer.self-insert" —
    // the deliberately stale signal the provenance gate must ignore.
    type_str(&mut s, "a");
    let stale: String = eval(&s, "return pmacs.editor.this_command()");
    assert_eq!(stale, "buffer.self-insert");
    exec(&s, "pmacs.window.buffer():insert(1, \"(\")");
    exec(&s, "pmacs.hook.run(\"buffer.after-edit\")");
    assert_eq!(
        buffer_text(&s),
        "a(",
        "programmatic insert + manual hook run must not pair, even with \
         this_command still reading buffer.self-insert"
    );
    let record_nil: bool = eval(&s, "return pmacs.pair._last_record == nil");
    assert!(record_nil, "manual hook run must observe no record");
}

#[test]
fn command_invoke_self_insert_does_not_pair() {
    let s = editor_with("");
    exec(&s, "pmacs.pair._capture_records = true");
    // Plain `pmacs.command.invoke` is the programmatic API: it stamps
    // no boundary and arms no record.
    exec(&s, "pmacs.command.invoke(\"buffer.self-insert\", 40)"); // '('
    exec(&s, "pmacs.hook.run(\"buffer.after-edit\")");
    assert_eq!(buffer_text(&s), "(", "invoked self-insert stays lone");
    let record_nil: bool = eval(&s, "return pmacs.pair._last_record == nil");
    assert!(record_nil);
}

// ---------------------------------------------------------------------------
// Type-over composition (Q#AP6, dispatch route)
// ---------------------------------------------------------------------------

#[test]
fn opener_over_region_type_overs_then_pairs() {
    let mut s = editor_with("abc");
    // Shift+Right x3: region [0, 3).
    for _ in 0..3 {
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Right, KeyModifiers::SHIFT));
    }
    type_str(&mut s, "(");
    assert_eq!(
        buffer_text(&s),
        "()",
        "the region is consumed by type-over, then the predicate pairs at EOB"
    );
    assert_eq!(cursor(&s), 1);
    let region_nil: bool = eval(&s, "return pmacs.editor.region() == nil");
    assert!(region_nil, "selection cleared");
}

// ---------------------------------------------------------------------------
// Undo grain (Q#AP5, daemon history in the non-replica harness)
// ---------------------------------------------------------------------------

#[test]
fn pair_is_two_adjacent_undo_steps_and_two_redo_steps() {
    let mut s = editor_with("");
    type_str(&mut s, "(");
    assert_eq!(buffer_text(&s), "()");
    undo(&mut s);
    assert_eq!(
        buffer_text(&s),
        "(",
        "first undo removes the reaction closer"
    );
    undo(&mut s);
    assert_eq!(buffer_text(&s), "", "second undo removes the typed opener");
    redo(&mut s);
    assert_eq!(buffer_text(&s), "(", "first redo restores the opener");
    redo(&mut s);
    assert_eq!(buffer_text(&s), "()", "second redo restores the closer");
}

#[test]
fn skip_undo_restores_the_swallowed_duplicate() {
    let mut s = editor_with("");
    type_str(&mut s, "()");
    assert_eq!(buffer_text(&s), "()");
    undo(&mut s);
    assert_eq!(
        buffer_text(&s),
        "())",
        "undoing the skip's delete restores the typed duplicate"
    );
}

// ---------------------------------------------------------------------------
// Reaction intercept outcomes (Q#AP3/Q#AP4): rejected vs transformed
// ---------------------------------------------------------------------------

#[test]
fn rejected_closer_leaves_opener_alone_and_reports() {
    let mut s = editor_with("");
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "insert" and op.bytes == ")" then
            error("rejected by test intercept")
          end
          return nil
        end)
        "#,
    );
    type_str(&mut s, "(");
    assert_eq!(
        buffer_text(&s),
        "(",
        "nothing landed; the opener stands alone"
    );
    assert_eq!(cursor(&s), 1);
    assert!(
        status(&s).contains("auto-pair closer rejected"),
        "got: {:?}",
        status(&s)
    );
}

#[test]
fn relocated_closer_lands_where_the_intercept_put_it_cursor_translated() {
    let mut s = editor_with("ab");
    exec(&s, "pmacs.editor.goto_byte(2)");
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "insert" and op.bytes == ")" then
            return { kind = "insert", pos = 0, bytes = op.bytes }
          end
          return nil
        end)
        "#,
    );
    type_str(&mut s, "(");
    assert_eq!(
        buffer_text(&s),
        ")ab(",
        "the intercept's positional result stands"
    );
    assert!(
        status(&s).contains("auto-pair closer altered"),
        "got: {:?}",
        status(&s)
    );
    assert_eq!(
        cursor(&s),
        4,
        "pre-edit cursor 3 right-gravity-translated through the insert at 0 — \
         translated, not teleported to the relocated closer"
    );
}

#[test]
fn rejected_skip_delete_keeps_the_duplicate() {
    let mut s = editor_with("");
    type_str(&mut s, "(");
    assert_eq!(buffer_text(&s), "()");
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "delete" then error("rejected by test intercept") end
          return nil
        end)
        "#,
    );
    type_str(&mut s, ")");
    assert_eq!(buffer_text(&s), "())", "the typed duplicate stays");
    assert_eq!(cursor(&s), 2);
    assert!(
        status(&s).contains("auto-pair skip rejected"),
        "got: {:?}",
        status(&s)
    );
}

#[test]
fn expanded_skip_delete_lands_reported_and_cursor_clamped() {
    let mut s = editor_with("");
    type_str(&mut s, "(");
    exec(&s, "pmacs.window.buffer():insert(2, \"x\")"); // "()x"
    exec(&s, "pmacs.editor.goto_byte(1)");
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "delete" then
            return { kind = "delete", start = op.start, ["end"] = op["end"] + 1 }
          end
          return nil
        end)
        "#,
    );
    type_str(&mut s, ")");
    assert_eq!(
        buffer_text(&s),
        "()",
        "the expanded delete swallowed the duplicate AND the x — it stands"
    );
    assert!(
        status(&s).contains("auto-pair skip altered"),
        "got: {:?}",
        status(&s)
    );
    assert_eq!(cursor(&s), 2, "translate-and-clamp repair");
}

// ---------------------------------------------------------------------------
// Source self-insert intercepts (Q#AP9): the reaction fails closed
// ---------------------------------------------------------------------------

#[test]
fn post_insert_mutation_by_the_command_kills_the_record() {
    // PR #110 round 1, finding 1: a redefined `buffer.self-insert`
    // that inserts the char and then REPLACES it — leaving the cursor
    // untouched — must not pair off the stale record. The record pins
    // the buffer revision after the completing edit; any further edit
    // by the same command kills it before the fan-out.
    let mut s = editor_with("");
    exec(
        &s,
        r#"
        pmacs.command.unregister("buffer.self-insert")
        pmacs.command.define {
          name = "buffer.self-insert",
          description = "test override: insert, then replace the typed char",
          fn = function(cp)
            pmacs.editor.insert_char_over_region(cp)
            pmacs.window.buffer():replace(0, 1, "[")
          end,
        }
        "#,
    );
    type_str(&mut s, "(");
    assert_eq!(
        buffer_text(&s),
        "[",
        "the typed `(` no longer exists; a `)` reaction would produce `[)`"
    );
    assert_eq!(cursor(&s), 1);
    assert!(
        !status(&s).contains("auto-pair"),
        "a dead record is a silent non-event; got: {:?}",
        status(&s)
    );
}

#[test]
fn transformed_non_pair_char_stays_silent() {
    // PR #110 round 1, finding 2: pairing has no interest in `a`; an
    // intercept relocating it must not draw an auto-pair report.
    let mut s = editor_with("xy");
    exec(&s, "pmacs.editor.goto_byte(2)");
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "insert" and op.bytes == "a" then
            return { kind = "insert", pos = 0, bytes = op.bytes }
          end
          return nil
        end)
        "#,
    );
    type_str(&mut s, "a");
    assert_eq!(buffer_text(&s), "axy");
    assert!(
        !status(&s).contains("auto-pair"),
        "chars outside the active pair set must stay silent; got: {:?}",
        status(&s)
    );
}

#[test]
fn relocated_opener_gets_no_pair_reaction() {
    let mut s = editor_with("ab");
    exec(&s, "pmacs.editor.goto_byte(2)");
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "insert" and op.bytes == "(" then
            return { kind = "insert", pos = 0, bytes = op.bytes }
          end
          return nil
        end)
        "#,
    );
    type_str(&mut s, "(");
    assert_eq!(
        buffer_text(&s),
        "(ab",
        "exactly the intercept's positional result — no closer anywhere"
    );
    assert!(
        status(&s).contains("auto-pair skipped: source self-insert transformed"),
        "got: {:?}",
        status(&s)
    );
}

#[test]
fn transformed_type_over_gets_no_pair_reaction() {
    let mut s = editor_with("abcd");
    for _ in 0..2 {
        s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Right, KeyModifiers::SHIFT));
    }
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "replace" then
            return { kind = "replace", start = op.start,
                     ["end"] = op["end"] + 1, bytes = op.bytes }
          end
          return nil
        end)
        "#,
    );
    type_str(&mut s, "(");
    assert_eq!(
        buffer_text(&s),
        "(d",
        "the expanded type-over stands as the intercept produced it"
    );
    assert!(
        status(&s).contains("auto-pair skipped: source self-insert transformed"),
        "got: {:?}",
        status(&s)
    );
}

#[test]
fn source_context_switch_fails_closed() {
    // A context-switching INTERCEPT cannot exist on the dispatch
    // self-insert path (the core borrow is held across it; the
    // three-phase borrow-released discipline is the Lua-mutator
    // path's). The legal producer of a context-switched source
    // self-insert is a user-redefined `buffer.self-insert` command
    // that switches after inserting — the record still completes for
    // the exact typed codepoint, and the hook then runs in the new
    // context, where pairing must fail closed.
    let dir = fresh_state_dir();
    let mut s = editor(&dir);
    let other = write_file(&dir, "other.txt", "z");
    exec(&s, "_G.scratch = pmacs.window.buffer()");
    exec(&s, &format!("pmacs.buffer.find_or_open({other:?})"));
    exec(&s, "_G.other = pmacs.window.buffer()");
    exec(&s, "pmacs.window.switch_buffer(_G.scratch)");
    // Bump the scratch revision past other.txt's so the active-buffer
    // revision compare still fires the hook after the switch (the
    // buffer-aware edit epoch is a named substrate deferral).
    type_str(&mut s, "ab");
    exec(
        &s,
        r#"
        pmacs.command.unregister("buffer.self-insert")
        pmacs.command.define {
          name = "buffer.self-insert",
          description = "test override: insert, then switch context",
          fn = function(cp)
            pmacs.editor.insert_char_over_region(cp)
            pmacs.window.switch_buffer(_G.other)
          end,
        }
        "#,
    );
    type_str(&mut s, "(");
    assert!(
        status(&s).contains("auto-pair skipped: source context changed"),
        "got: {:?}",
        status(&s)
    );
    let scratch_text: String = eval(&s, "return _G.scratch:slice(0, _G.scratch:len())");
    assert_eq!(
        scratch_text, "ab(",
        "the opener landed in scratch, no closer"
    );
    let other_text: String = eval(&s, "return _G.other:slice(0, _G.other:len())");
    assert_eq!(other_text, "z", "the switched-to buffer is untouched");
}

#[test]
fn context_switch_relevance_is_the_source_buffers_rust_to_python_is_silent() {
    // PR #110 round 2, finding 2: `'` typed in Rust is not a pair
    // char THERE — that the context-switching command lands in a
    // Python buffer (where `''` pairs) must not conjure an irrelevant
    // "source context changed" report. Relevance and reporting are
    // attributed to the buffer the record names.
    let dir = fresh_state_dir();
    let mut s = editor(&dir);
    let py = write_file(&dir, "b.py", "");
    let rs = write_file(&dir, "a.rs", "");
    exec(&s, &format!("pmacs.buffer.find_or_open({py:?})"));
    exec(&s, "_G.py = pmacs.window.buffer()");
    exec(&s, &format!("pmacs.buffer.find_or_open({rs:?})"));
    exec(&s, "_G.rs = pmacs.window.buffer()");
    // Revision skew so the fan-out runs after the switch (the
    // buffer-aware edit epoch is a named substrate deferral).
    type_str(&mut s, "xy");
    exec(
        &s,
        r#"
        pmacs.command.unregister("buffer.self-insert")
        pmacs.command.define {
          name = "buffer.self-insert",
          description = "test override: insert, then switch context",
          fn = function(cp)
            pmacs.editor.insert_char_over_region(cp)
            pmacs.window.switch_buffer(_G.py)
          end,
        }
        "#,
    );
    type_str(&mut s, "'");
    assert!(
        !status(&s).contains("auto-pair"),
        "`'` is outside the SOURCE (rust) set; got: {:?}",
        status(&s)
    );
    let rs_text: String = eval(&s, "return _G.rs:slice(0, _G.rs:len())");
    assert_eq!(
        rs_text, "xy'",
        "the quote landed in the rust buffer, no pair"
    );
    let py_text: String = eval(&s, "return _G.py:slice(0, _G.py:len())");
    assert_eq!(py_text, "", "the python buffer is untouched");
}

#[test]
fn context_switch_relevance_is_the_source_buffers_python_to_rust_reports() {
    // The inverse route: `'` typed in Python IS a pair char there, so
    // the context-change report must fire even though the destination
    // (rust) set would have suppressed it under active-buffer lookup.
    let dir = fresh_state_dir();
    let mut s = editor(&dir);
    let rs = write_file(&dir, "a.rs", "");
    let py = write_file(&dir, "b.py", "");
    exec(&s, &format!("pmacs.buffer.find_or_open({rs:?})"));
    exec(&s, "_G.rs = pmacs.window.buffer()");
    exec(&s, &format!("pmacs.buffer.find_or_open({py:?})"));
    exec(&s, "_G.py = pmacs.window.buffer()");
    type_str(&mut s, "xy");
    exec(
        &s,
        r#"
        pmacs.command.unregister("buffer.self-insert")
        pmacs.command.define {
          name = "buffer.self-insert",
          description = "test override: insert, then switch context",
          fn = function(cp)
            pmacs.editor.insert_char_over_region(cp)
            pmacs.window.switch_buffer(_G.rs)
          end,
        }
        "#,
    );
    type_str(&mut s, "'");
    assert!(
        status(&s).contains("auto-pair skipped: source context changed"),
        "`'` is in the SOURCE (python) set, so the report fires; got: {:?}",
        status(&s)
    );
    let py_text: String = eval(&s, "return _G.py:slice(0, _G.py:len())");
    assert_eq!(
        py_text, "xy'",
        "the quote landed in the python buffer, no pair"
    );
    let rs_text: String = eval(&s, "return _G.rs:slice(0, _G.rs:len())");
    assert_eq!(rs_text, "", "the rust buffer is untouched");
}

#[test]
fn source_context_switch_with_equal_revisions_fails_closed_silently() {
    // PR #110 round 1, finding 5: the twin of the test above WITHOUT
    // the revision bump. Dispatch's active-buffer revision compare
    // (pre = scratch@0, post = other@0) sees no delta, so the
    // after-edit fan-out never runs: no reaction anywhere, and no
    // context-change report either — the record dies un-armed. The
    // report is best-effort until the buffer-aware edit epoch lands
    // (named substrate deferral); failing closed is unconditional.
    let dir = fresh_state_dir();
    let mut s = editor(&dir);
    let other = write_file(&dir, "other.txt", "z");
    exec(&s, "_G.scratch = pmacs.window.buffer()");
    exec(&s, &format!("pmacs.buffer.find_or_open({other:?})"));
    exec(&s, "_G.other = pmacs.window.buffer()");
    exec(&s, "pmacs.window.switch_buffer(_G.scratch)");
    exec(
        &s,
        r#"
        pmacs.command.unregister("buffer.self-insert")
        pmacs.command.define {
          name = "buffer.self-insert",
          description = "test override: insert, then switch context",
          fn = function(cp)
            pmacs.editor.insert_char_over_region(cp)
            pmacs.window.switch_buffer(_G.other)
          end,
        }
        "#,
    );
    type_str(&mut s, "(");
    let scratch_text: String = eval(&s, "return _G.scratch:slice(0, _G.scratch:len())");
    assert_eq!(scratch_text, "(", "the opener landed in scratch, no closer");
    let other_text: String = eval(&s, "return _G.other:slice(0, _G.other:len())");
    assert_eq!(other_text, "z", "the switched-to buffer is untouched");
    assert!(
        !status(&s).contains("auto-pair"),
        "no fan-out ran, so no report is possible; got: {:?}",
        status(&s)
    );
    let take_nil: bool = eval(&s, "return pmacs.editor.take_typed_edit() == nil");
    assert!(take_nil, "the record was never armed");
}

// ---------------------------------------------------------------------------
// Context-switching REACTION intercept: repair skipped, deferral pinned
// ---------------------------------------------------------------------------

#[test]
fn context_switching_reaction_intercept_skips_repair_and_later_callbacks_observe_it() {
    let dir = fresh_state_dir();
    let mut s = editor(&dir);
    let other = write_file(&dir, "other.txt", "z");
    exec(&s, "_G.scratch = pmacs.window.buffer()");
    exec(&s, &format!("pmacs.buffer.find_or_open({other:?})"));
    exec(&s, "_G.other = pmacs.window.buffer()");
    exec(&s, "pmacs.window.switch_buffer(_G.scratch)");
    type_str(&mut s, "ab");
    // A probe registered AFTER pair.lua (and every builtin): it
    // observes whatever context the fan-out is in when it runs —
    // explicitly pinning, not concealing, the origin-context deferral.
    exec(
        &s,
        r#"
        _G.probe_buf = nil
        pmacs.hook.add("buffer.after-edit", function()
          _G.probe_buf = tostring(pmacs.window.buffer())
        end)
        pmacs.buffer.add_intercept(_G.scratch, function(op)
          if op.kind == "insert" and op.bytes == ")" then
            pmacs.window.switch_buffer(_G.other)
            return { kind = "insert", pos = 0, bytes = op.bytes }
          end
          return nil
        end)
        "#,
    );
    type_str(&mut s, "(");
    assert!(
        status(&s).contains("auto-pair closer altered"),
        "got: {:?}",
        status(&s)
    );
    let scratch_text: String = eval(&s, "return _G.scratch:slice(0, _G.scratch:len())");
    assert_eq!(
        scratch_text, ")ab(",
        "the relocated closer landed in the scratch buffer as the intercept wrote it"
    );
    let other_text: String = eval(&s, "return _G.other:slice(0, _G.other:len())");
    assert_eq!(
        other_text, "z",
        "pair.lua never touched the new context's text"
    );
    assert_eq!(
        cursor(&s),
        0,
        "no cursor repair in the switched-to context (switch_buffer's own \
         cursor reset stands untouched)"
    );
    let probe_saw_other: bool = eval(&s, "return _G.probe_buf == tostring(_G.other)");
    assert!(
        probe_saw_other,
        "a later callback observes the switched context — the origin-pinned \
         fan-out deferral, pinned"
    );
}

// ---------------------------------------------------------------------------
// Hook fan-out: one fire per keystroke, the reaction edit doesn't re-fire
// ---------------------------------------------------------------------------

#[test]
fn after_edit_fires_once_per_pairing_keystroke() {
    let mut s = editor_with("");
    exec(
        &s,
        r#"
        _G.fires = 0
        pmacs.hook.add("buffer.after-edit", function() _G.fires = _G.fires + 1 end)
        "#,
    );
    type_str(&mut s, "(");
    assert_eq!(buffer_text(&s), "()");
    let fires: i64 = eval(&s, "return _G.fires");
    assert_eq!(
        fires, 1,
        "the closer edit must not re-fire buffer.after-edit"
    );
}

// ---------------------------------------------------------------------------
// Typed-edit record lifecycle (Q#AP9)
// ---------------------------------------------------------------------------

#[test]
fn typed_edit_record_is_exact_and_one_shot() {
    let mut s = editor_with("");
    exec(
        &s,
        r#"
        pmacs.pair._capture_records = true
        _G.second_take = "unset"
        pmacs.hook.add("buffer.after-edit", function()
          _G.second_take = pmacs.editor.take_typed_edit()
        end)
        "#,
    );
    type_str(&mut s, "(");
    // pair.lua (first registrant) consumed the record and published it
    // on the test seam: exact codepoint + effective triple.
    let (cp, ch, clean, es, ee, il, pc): (i64, String, bool, i64, i64, i64, i64) = eval(
        &s,
        "
        local r = pmacs.pair._last_record
        return r.codepoint, r.char, r.clean, r.effective_start,
               r.effective_end, r.inserted_len, r.post_cursor
        ",
    );
    assert_eq!(cp, 40, "exact codepoint for '('");
    assert_eq!(ch, "(");
    assert!(clean);
    assert_eq!(
        (es, ee, il),
        (0, 0, 1),
        "effective triple of the opener insert"
    );
    assert_eq!(pc, 1);
    // A second take — from a later callback in the SAME fan-out — is nil.
    let second_nil: bool = eval(&s, "return _G.second_take == nil");
    assert!(second_nil, "the record is consumable exactly once");
    // Outside any fan-out the slot is empty.
    let outside_nil: bool = eval(&s, "return pmacs.editor.take_typed_edit() == nil");
    assert!(outside_nil, "no record outside the after-edit fan-out");
}

#[test]
fn nested_manual_after_edit_run_sees_no_record() {
    let mut s = editor_with("");
    exec(
        &s,
        r#"
        pmacs.pair._capture_records = true
        _G.outer = nil
        _G.ran_nested = false
        pmacs.hook.add("buffer.after-edit", function()
          if _G.ran_nested then return end -- the nested run reaches this callback too
          _G.ran_nested = true
          _G.outer = pmacs.pair._last_record
          pmacs.hook.run("buffer.after-edit")
        end)
        "#,
    );
    type_str(&mut s, "(");
    let outer_seen: bool = eval(&s, "return _G.outer ~= nil");
    assert!(outer_seen, "the outer fan-out carried a record");
    // The nested run re-entered pair.lua, which took nil and published
    // nil on the seam — proving the nested run observed no record.
    let nested_nil: bool = eval(&s, "return pmacs.pair._last_record == nil");
    assert!(nested_nil, "a nested manual re-run must see nil");
    assert_eq!(buffer_text(&s), "()", "and must insert no second closer");
}

#[test]
fn record_capture_is_off_by_default() {
    // PR #110 round 1, finding 4: without the explicit test facility,
    // no consumed record is retained anywhere — the one-shot take API
    // is the only access, and it is empty after the fan-out.
    let mut s = editor_with("");
    type_str(&mut s, "(");
    assert_eq!(buffer_text(&s), "()");
    let leaked: bool = eval(&s, "return pmacs.pair._last_record ~= nil");
    assert!(!leaked, "production keystrokes must retain no record");
}

#[test]
fn rejected_self_insert_leaves_no_record() {
    let mut s = editor_with("");
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(_op)
          error("rejected by test intercept")
        end)
        "#,
    );
    type_str(&mut s, "(");
    assert_eq!(buffer_text(&s), "", "nothing landed");
    let take_nil: bool = eval(&s, "return pmacs.editor.take_typed_edit() == nil");
    assert!(take_nil, "a rejecting edit must arm no record");
}

#[test]
fn frontends_cannot_consume_each_others_slot() {
    let s = editor_with("");
    let (buffer, window) = {
        let core = s.core.borrow();
        (core.active_buffer_id(), core.active_window_id())
    };
    let record = pmacs::editor_core::TypedEditRecord {
        buffer,
        window,
        codepoint: '(',
        requested_start: 0,
        requested_end: 0,
        effective_start: 0,
        effective_end: 0,
        inserted_len: 1,
        post_cursor: 1,
        clean: true,
        revision: 0,
    };
    let a = FrontendId::LOCAL;
    let b = FrontendId(a.0 + 1);
    let mut core = s.core.borrow_mut();
    core.typed_edit_set_armed(a, record);
    core.active_frontend = b;
    assert!(
        core.take_typed_edit().is_none(),
        "frontend B must not see frontend A's record"
    );
    core.active_frontend = a;
    assert!(
        core.take_typed_edit().is_some(),
        "the slot survives a foreign take attempt for its owner"
    );
    assert!(
        core.take_typed_edit().is_none(),
        "one-shot for the owner too"
    );
}

// ---------------------------------------------------------------------------
// Signature help + first-didChange ordering (Q#AP7 / Q#AP8)
// ---------------------------------------------------------------------------

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

fn pump_lua_flag(state: &mut EditorState, flag: &str, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        state.tick_processes();
        state.tick_lsp();
        state.tick_async();
        let done: bool = state
            .lua_host
            .lua()
            .load(format!("return ({flag}) == true"))
            .eval()
            .unwrap_or(false);
        if done {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Editor visiting a `.rs` file attached to the fake LSP in `sighelp`
/// mode, with the document-sync sink at `sink`. Returns after the
/// server initialized.
fn sighelp_editor(dir: &std::path::Path, sink: &std::path::Path, body: &str) -> EditorState {
    let mut s = editor(dir);
    let fake = fake_lsp_path();
    let sink_disp = sink.display().to_string();
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust = {{
               command = '{fake}',
               env = {{
                 PMACS_FAKE_LSP_MODE = 'sighelp',
                 PMACS_FAKE_LSP_CHANGE_SINK = '{sink_disp}',
               }},
             }}"
        ),
    );
    let f = write_file(dir, "a.rs", body);
    exec(&s, &format!("pmacs.buffer.find_or_open({f:?})"));
    exec(&s, "pmacs.editor.goto_byte(0)");
    let initialized = "(function() \
       for _,r in ipairs(pmacs.lsp.list()) do \
         if r.state and r.state.kind=='initialized' then return true end \
       end \
       return false \
     end)()";
    assert!(pump_lua_flag(&mut s, initialized, 5), "fake server init");
    s
}

/// The `text` of every `textDocument/didChange` line in the sink, in
/// arrival order.
fn did_change_texts(sink: &std::path::Path) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(sink) else {
        return Vec::new();
    };
    raw.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v.get("method").and_then(|m| m.as_str()) == Some("textDocument/didChange"))
        .filter_map(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_owned))
        .collect()
}

#[test]
fn first_did_change_after_opener_carries_the_pair() {
    let dir = fresh_state_dir();
    let sink = dir.join("changes.jsonl");
    let mut s = sighelp_editor(&dir, &sink, "\n");

    type_str(&mut s, "(");
    assert_eq!(
        buffer_text(&s),
        "()\n",
        "pairing is active in the attached buffer"
    );

    // The signature auto-trigger's synchronous flush sends the first
    // didChange from inside the SAME fan-out; pump until the fake
    // server has written it to the sink.
    let deadline = Instant::now() + Duration::from_secs(5);
    let changes = loop {
        s.tick_processes();
        s.tick_lsp();
        s.tick_async();
        let c = did_change_texts(&sink);
        if !c.is_empty() {
            break c;
        }
        assert!(
            Instant::now() < deadline,
            "no didChange reached the fake server"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        changes[0], "()\n",
        "the FIRST didChange after `(` carries the closer — pair.lua ran \
         before lsp.lua's synchronous flush (Q#AP7 ordering observable)"
    );

    // And the auto-trigger itself still fired with pairing active
    // (Q#AP8): the fake's signature label reaches the status line.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw = false;
    while Instant::now() < deadline {
        s.tick_processes();
        s.tick_lsp();
        s.tick_async();
        if status(&s).contains("fn echo(") {
            saw = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        saw,
        "signature help must still auto-trigger with pairing active"
    );
}

#[test]
fn relocated_closer_first_did_change_carries_the_complete_effective_text() {
    let dir = fresh_state_dir();
    let sink = dir.join("changes.jsonl");
    let mut s = sighelp_editor(&dir, &sink, "\n");
    // Context-preserving position transform: the closer lands at 0.
    exec(
        &s,
        r#"
        pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
          if op.kind == "insert" and op.bytes == ")" then
            return { kind = "insert", pos = 0, bytes = op.bytes }
          end
          return nil
        end)
        "#,
    );
    type_str(&mut s, "(");
    assert_eq!(buffer_text(&s), ")(\n");

    let deadline = Instant::now() + Duration::from_secs(5);
    let changes = loop {
        s.tick_processes();
        s.tick_lsp();
        s.tick_async();
        let c = did_change_texts(&sink);
        if !c.is_empty() {
            break c;
        }
        assert!(
            Instant::now() < deadline,
            "no didChange reached the fake server"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        changes[0], ")(\n",
        "a position-only closer transform still sends the complete effective \
         text in the first didChange — never an opener-only intermediate"
    );
}
