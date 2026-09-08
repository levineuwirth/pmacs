//! Lean 4 Unicode input method acceptance (Arc 8 Stage 4b,
//! the archived lean4-mode framing Q#LN11/Q#LN21/Q#LN22, criteria 38–45i).
//!
//! Dispatch-driven throughout: `dispatch_key` is the producer that arms
//! the typed-edit record for a grid frontend. The optimistic CRDT
//! producer is criterion 45f and lives in a `--lib` test, where the gate
//! list's `--features crdt` run reaches it.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use pmacs::window::{FrontendView, Layout, Window, WindowId};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

fn fresh_dir() -> PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pmacs-leaninput-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn text(s: &EditorState) -> String {
    let b: mlua::String = eval(
        s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
}

fn cursor(s: &EditorState) -> i64 {
    eval(s, "return pmacs.editor.cursor()")
}

fn type_as(s: &mut EditorState, fid: FrontendId, chars: &str) {
    for ch in chars.chars() {
        s.dispatch_key(fid, key(KeyCode::Char(ch)));
    }
}

fn type_str(s: &mut EditorState, chars: &str) {
    type_as(s, FrontendId::LOCAL, chars);
}

/// An editor with an empty `.lean` file open and the point at 0.
/// `pmacs.lsp.config = {}` keeps the real user config from spawning a
/// server; the language still resolves from the extension.
fn lean_editor() -> (EditorState, PathBuf) {
    let dir = fresh_dir();
    let f = dir.join("a.lean");
    std::fs::write(&f, "").unwrap();
    let s = EditorState::new_with_roots(&crate::iso::roots());
    exec(&s, "pmacs.lsp.config = {}");
    let fd = f.display().to_string();
    exec(&s, &format!("pmacs.buffer.find_or_open({fd:?})"));
    exec(&s, "pmacs.editor.goto_byte(0)");
    assert_eq!(
        eval::<Option<String>>(
            &s,
            "return pmacs.lsp.buffer_language(pmacs.window.buffer())"
        )
        .as_deref(),
        Some("lean4"),
        "the fixture must actually be a lean4 buffer, or every \
         expansion assertion below is vacuous"
    );
    (s, f)
}

// ---------------------------------------------------------------------------
// 38 / 41 — the two expansion paths, and what an undo restores
// ---------------------------------------------------------------------------

#[test]
fn the_finish_path_retains_the_terminator_in_one_undo_step() {
    // `alp` is not a key; `alpha` is the shortest key extending it. The
    // space does not extend anything, so it lands first and the
    // expansion replaces the leader and the typed text — the span stops
    // BEFORE the terminator, so whatever auto-pairing did with it
    // survives. One undo restores the same text either way, because the
    // terminator was its own insert to begin with.
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\alp ");
    assert_eq!(text(&s), "α ", "terminator retained, not consumed");

    exec(&s, "pmacs.window.buffer():undo()");
    assert_eq!(
        text(&s),
        "\\alp ",
        "one undo restores the pre-expansion text WITH its terminator — \
         the expansion is a single edit"
    );
}

#[test]
fn the_eager_path_takes_no_terminator_and_undoes_separately() {
    // `alpha` has no longer key extending it, so it is one of the 1,550
    // eager keys: it expands the moment the final `a` lands, and a
    // following space is a SEPARATE edit. Rev 8 asserted the finish-path
    // undo text for this example, which is the trap (round 9).
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\alpha");
    assert_eq!(text(&s), "α", "eager expansion, no terminator typed");

    type_str(&mut s, " ");
    assert_eq!(text(&s), "α ");
    exec(&s, "pmacs.window.buffer():undo()");
    assert_eq!(text(&s), "α", "the first undo removes the separate space");
    exec(&s, "pmacs.window.buffer():undo()");
    assert_eq!(text(&s), "\\alpha", "the second undoes the expansion");
}

#[test]
fn to_is_not_eager_because_longer_keys_extend_it() {
    // The criterion rev 8 got wrong: `to` looks unique and is not.
    // `top`, `to0`, `toa` and others extend it, so it needs a
    // terminator. Bites against an eager rule that tests only "is this
    // a key" without asking whether anything extends it.
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\to");
    assert_eq!(text(&s), "\\to", "no expansion without a terminator");

    type_str(&mut s, " ");
    assert_eq!(text(&s), "→ ", "the finish path then resolves it");
}

// ---------------------------------------------------------------------------
// 39 — $CURSOR
// ---------------------------------------------------------------------------

#[test]
fn the_cursor_placeholder_places_the_point_between_the_symbols() {
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\<>");
    assert_eq!(text(&s), "⟨⟩");
    // The placeholder is a point position, not a literal: typing lands
    // between the brackets.
    type_str(&mut s, "x");
    assert_eq!(text(&s), "⟨x⟩", "$CURSOR left the point inside");
}

// ---------------------------------------------------------------------------
// 40 — the pair collision
// ---------------------------------------------------------------------------

#[test]
fn a_pending_abbreviation_is_never_corrupted_by_auto_pairing() {
    // 64 keys contain a `lean4` pair-set character. Two DISTINCT bugs
    // produce the same symptom here, so both are asserted: pairing
    // running first, and a consumer that claims only completed
    // expansions (which would hand each intermediate `[` to pairing).
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\[");
    assert_eq!(
        text(&s),
        "\\[",
        "the intermediate `[` was CLAIMED — pairing inserted no `]`, \
         which is what keeps `\\[[]]` reachable"
    );

    type_str(&mut s, "[]]");
    assert_eq!(text(&s), "⟦⟧", "the full key resolves");
}

#[test]
fn a_pair_character_that_terminates_an_abbreviation_still_pairs() {
    // The other half of the collision, and the one the first revision
    // of this file got wrong. `(` does not extend `alp`, so it
    // TERMINATES — and a terminator is an ordinary character that
    // pairing is entitled to react to.
    //
    // Claiming the terminator suppresses pairing entirely (`α(`).
    // Expanding before declining is no better: the replace makes
    // pairing's copy of the record stale, so pairing declines and the
    // closer is silently lost. Only deferring the expansion past the
    // chain gives both.
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\alp(");
    assert_eq!(
        text(&s),
        "α()",
        "the abbreviation expanded AND the terminator paired"
    );
    assert_eq!(
        cursor(&s),
        3,
        "and the point sits between the pair — after α (2 bytes) and \
         the opener"
    );
}

#[test]
fn a_nested_fan_out_between_the_expander_and_pairing_does_not_expand_early() {
    // `buffer.after-edit` fan-outs NEST — the typed-edit contract
    // explicitly supports a consumer calling `pmacs.hook.run`, and a
    // nested run re-enters every subscriber, including the deferred
    // expansion's. If the nested pass performed the expansion, the
    // OUTER chain would then resume and hand pairing a record the
    // replace had already invalidated: `α(` again, reached through the
    // chain's documented re-entrancy seam rather than through claiming.
    let (mut s, _f) = lean_editor();
    exec(
        &s,
        r#"
        _G.NESTED = 0
        pmacs.typed_edit.add_consumer {
          name = "nested-fan-out",
          priority = 75,   -- between the expander (50) and pairing (100)
          fn = function()
            if _G.NESTED == 0 then
              _G.NESTED = 1
              pmacs.hook.run("buffer.after-edit")
            end
            return false
          end,
        }
        "#,
    );

    type_str(&mut s, "\\alp(");
    let nested: i64 = eval(&s, "return _G.NESTED");
    assert_eq!(nested, 1, "the nested fan-out must actually have run");
    assert_eq!(
        text(&s),
        "α()",
        "the expansion waited for the OUTERMOST pass, so pairing still \
         held a valid record when the terminator reached it"
    );
}

#[test]
fn a_nested_fan_out_that_never_reaches_the_expander_still_does_not_expand_early() {
    // The chain's OTHER exit: a consumer may CLAIM and stop the chain
    // before the expander is reached, while the fan-out's
    // deferred-expansion subscriber still runs. Counting in the
    // expander itself therefore misses that pass — it would look like
    // the outermost one and expand early, and outer pairing would
    // resume with an invalidated record.
    //
    // The sequence, exactly: a consumer at 25 declines on the outer
    // pass (there is a record) and claims on the nested one (there is
    // not); a consumer at 75 runs one nested fan-out from between the
    // expander and pairing.
    let (mut s, _f) = lean_editor();
    exec(
        &s,
        r#"
        _G.NESTED, _G.CLAIMED = 0, 0
        pmacs.typed_edit.add_consumer {
          name = "claims-only-when-recordless",
          priority = 25,   -- ahead of the expander at 50
          fn = function(rec)
            if rec == nil then
              _G.CLAIMED = _G.CLAIMED + 1
              return true   -- stops the chain: the expander never runs
            end
            return false
          end,
        }
        pmacs.typed_edit.add_consumer {
          name = "nested-fan-out",
          priority = 75,   -- between the expander (50) and pairing (100)
          fn = function()
            if _G.NESTED == 0 then
              _G.NESTED = 1
              pmacs.hook.run("buffer.after-edit")
            end
            return false
          end,
        }
        "#,
    );

    type_str(&mut s, "\\alp(");
    let (nested, claimed): (i64, i64) = eval(&s, "return _G.NESTED, _G.CLAIMED");
    assert_eq!(nested, 1, "the nested fan-out must actually have run");
    assert!(
        claimed >= 1,
        "the nested pass must actually have been short-circuited before \
         the expander, or this pins the same thing as 45n"
    );
    assert_eq!(
        text(&s),
        "α()",
        "the nesting count comes from a point that runs before any \
         consumer can claim, so the nested pass was still recognised"
    );
}

#[test]
fn a_pair_character_outside_a_pending_abbreviation_still_pairs() {
    // The other direction: claiming extensions must not disable pairing
    // in Lean buffers generally.
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "[");
    assert_eq!(text(&s), "[]", "ordinary auto-pairing is untouched");
}

// ---------------------------------------------------------------------------
// 42 — a prefix that opens nothing
// ---------------------------------------------------------------------------

#[test]
fn a_prefix_that_opens_no_key_is_left_literal_with_no_edit() {
    // `W` is one of exactly six printable characters that begin no key
    // (`$ % , ; @ W`). Rev 8 used `\zzzz`, which expands — `ze`, `zeta`
    // and `zsqrtd` exist (round 9).
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\WWWW ");
    assert_eq!(text(&s), "\\WWWW ", "literal text, no expansion");
}

#[test]
fn a_prefix_with_no_complete_match_still_expands_its_best_prefix() {
    // The case rev 8 mistook for "no match": `z` DOES open a pending
    // abbreviation, and the second `z` finishes it.
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\zzzz ");
    assert_eq!(text(&s), "ζzzz ", "`z` resolved through `ze`");
}

// ---------------------------------------------------------------------------
// 43 — lazy abandonment
// ---------------------------------------------------------------------------

#[test]
fn moving_the_point_away_abandons_the_pending_abbreviation() {
    // There is no cursor-motion hook, so the pending record is
    // validated at the NEXT typed edit: the point must still be at the
    // end of the pending span.
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\alp");
    exec(&s, "pmacs.editor.goto_byte(0)");
    type_str(&mut s, "h");
    assert_eq!(text(&s), "h\\alp", "the `h` landed as plain text");

    // The keystroke that makes abandonment OBSERVABLE. Asserting only
    // the line above proves nothing: claiming an extension makes no
    // edit, so a record that wrongly survived would look identical
    // here. If `h` had extended the record to `alph`, this `a`
    // completes `alpha` and eagerly expands — over a span whose offsets
    // are now stale by one.
    type_str(&mut s, "a");
    assert_eq!(
        text(&s),
        "ha\\alp",
        "`\\alp` is still literal: the record was dropped when the \
         point left the end of its span, not carried along"
    );
}

#[test]
fn switching_buffers_clears_pending_state_eagerly() {
    let (mut s, f) = lean_editor();
    let dir = fresh_dir();
    let other = dir.join("b.lean");
    std::fs::write(&other, "").unwrap();
    let od = other.display().to_string();
    let fd = f.display().to_string();

    // Open the second buffer FIRST, then come back. `find_or_open`
    // fires `buffer.after-switch` only on the already-open branch — a
    // fresh load fires `buffer.after-load` instead, and its own insert
    // fires a record-less `buffer.after-edit`. Without this warm-up the
    // test passes through the nil-record path and pins nothing about
    // switching: deleting the after-switch subscriber leaves it green.
    exec(&s, &format!("pmacs.buffer.find_or_open({od:?})"));
    exec(&s, &format!("pmacs.buffer.find_or_open({fd:?})"));
    exec(&s, "pmacs.editor.goto_byte(0)");

    type_str(&mut s, "\\alph");
    exec(&s, &format!("pmacs.buffer.find_or_open({od:?})"));
    exec(&s, &format!("pmacs.buffer.find_or_open({fd:?})"));
    exec(&s, "pmacs.editor.goto_byte(pmacs.window.buffer():len())");

    type_str(&mut s, "a");
    assert_eq!(
        text(&s),
        "\\alpha",
        "without the switch this would have eagerly expanded to α; \
         `buffer.after-switch` cleared the record"
    );
}

#[test]
fn a_self_insert_that_moves_the_point_afterwards_does_not_expand() {
    // Buffer and window matching is not enough. A redefined
    // `buffer.self-insert` may insert the completing character and THEN
    // move the point; expanding over a span the user has left teleports
    // them back into it. Pairing makes the same three-part check
    // (`ed.cursor() ~= rec.post_cursor`) for the same reason.
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\alph");
    exec(
        &s,
        r#"
        pmacs.command.unregister("buffer.self-insert")
        pmacs.command.define {
          name = "buffer.self-insert",
          description = "test override: insert, then move the point away",
          fn = function(cp)
            pmacs.editor.insert_char_over_region(cp)
            pmacs.editor.goto_byte(0)
          end,
        }
        "#,
    );

    type_str(&mut s, "a");
    assert_eq!(
        text(&s),
        "\\alpha",
        "the record died with the point that left it — no expansion"
    );
    assert_eq!(cursor(&s), 0, "and the point stayed where it was moved to");
}

#[test]
fn an_intercept_that_switches_buffers_does_not_move_the_other_points() {
    // A buffer intercept may switch window or buffer while the replace
    // runs. An unguarded `goto_byte` afterwards moves the point of
    // whatever it switched TO — a buffer with nothing to do with this
    // expansion. Pairing's `repair_cursor` guards the same way.
    let (mut s, f) = lean_editor();
    let dir = fresh_dir();
    let other = dir.join("other.lean");
    std::fs::write(&other, "0123456789").unwrap();
    let od = other.display().to_string();
    let fd = f.display().to_string();

    exec(&s, &format!("pmacs.buffer.find_or_open({od:?})"));
    exec(&s, &format!("pmacs.buffer.find_or_open({fd:?})"));
    exec(&s, "pmacs.editor.goto_byte(0)");
    exec(
        &s,
        &format!(
            r#"
            _G.SWITCHED = false
            pmacs.buffer.add_intercept(pmacs.window.buffer(), function(op)
              if op.kind == "replace" and not _G.SWITCHED then
                _G.SWITCHED = true
                pmacs.buffer.find_or_open({od:?})
              end
              return nil
            end)
            "#
        ),
    );

    type_str(&mut s, "\\alpha");
    let switched: bool = eval(&s, "return _G.SWITCHED");
    assert!(switched, "the intercept must actually have fired");
    assert_eq!(
        text(&s),
        "0123456789",
        "we are now in the buffer the intercept switched to"
    );
    // Whatever point the switch left in that buffer, the expansion must
    // not have moved it. Unguarded, `goto_byte` runs against the
    // ambient buffer and translates the LEAN buffer's pre-edit point
    // (6) through the LEAN buffer's replace, landing at 2 here — a
    // number with no meaning in this buffer at all.
    assert_eq!(
        cursor(&s),
        0,
        "its point is untouched — the expansion's cursor placement is \
         guarded on the window and buffer still being the ones it \
         edited"
    );
}

// ---------------------------------------------------------------------------
// 44 / 45 — the setting and the language gate, both on the SOURCE buffer
// ---------------------------------------------------------------------------

#[test]
fn disabling_the_setting_stops_expansion() {
    let (mut s, _f) = lean_editor();
    exec(&s, "pmacs.config.set('lean.abbrev', false)");
    type_str(&mut s, "\\alpha");
    assert_eq!(text(&s), "\\alpha", "no expansion when disabled");

    exec(&s, "pmacs.config.set('lean.abbrev', true)");
    exec(&s, "pmacs.editor.goto_byte(pmacs.window.buffer():len())");
    type_str(&mut s, " \\alpha");
    assert_eq!(text(&s), "\\alpha α", "and it comes back live");
}

#[test]
fn the_setting_is_read_against_the_typed_edits_source_buffer() {
    // A buffer-local override must not follow the user to another
    // buffer of the same language — the `editing.auto-pair` precedent,
    // including its round-2 correction to resolve `rec.buffer` rather
    // than `pmacs.window.buffer()`.
    let (mut s, f) = lean_editor();
    let dir = fresh_dir();
    let other = dir.join("b.lean");
    std::fs::write(&other, "").unwrap();

    exec(
        &s,
        "pmacs.config.set_local(pmacs.window.buffer(), 'lean.abbrev', false)",
    );
    type_str(&mut s, "\\alpha");
    assert_eq!(text(&s), "\\alpha", "disabled in THIS buffer");

    let od = other.display().to_string();
    exec(&s, &format!("pmacs.buffer.find_or_open({od:?})"));
    exec(&s, "pmacs.editor.goto_byte(0)");
    type_str(&mut s, "\\alpha");
    assert_eq!(text(&s), "α", "a second lean buffer is unaffected");

    let fd = f.display().to_string();
    exec(&s, &format!("pmacs.buffer.find_or_open({fd:?})"));
    assert_eq!(text(&s), "\\alpha", "and the first is still disabled");
}

#[test]
fn no_abbreviation_state_is_opened_outside_a_lean_buffer() {
    let dir = fresh_dir();
    let f = dir.join("a.rs");
    std::fs::write(&f, "").unwrap();
    let s = EditorState::new_with_roots(&crate::iso::roots());
    exec(&s, "pmacs.lsp.config = {}");
    let fd = f.display().to_string();
    exec(&s, &format!("pmacs.buffer.find_or_open({fd:?})"));
    exec(&s, "pmacs.editor.goto_byte(0)");
    let mut s = s;

    type_str(&mut s, "\\alpha");
    assert_eq!(text(&s), "\\alpha", "no expansion in Rust");

    // And the leader opened nothing, so `[` still pairs normally.
    exec(&s, "pmacs.editor.goto_byte(pmacs.window.buffer():len())");
    type_str(&mut s, "\\[");
    assert_eq!(
        text(&s),
        "\\alpha\\[]",
        "`\\[` in a Rust buffer pairs — the input method never armed"
    );
}

// ---------------------------------------------------------------------------
// 45a / 45b / 45c / 45d / 45e — resolution rules
// ---------------------------------------------------------------------------

#[test]
fn the_shortest_key_wins_not_the_longest() {
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\alp ");
    assert_eq!(text(&s), "α ", "`alp` resolves through `alpha`");

    exec(&s, "pmacs.editor.goto_byte(pmacs.window.buffer():len())");
    type_str(&mut s, "\\al ");
    assert_eq!(
        text(&s),
        "α ∀ ",
        "`al` resolves through `all` (3) — NOT `alpha` (5). A \
         longest-match or unique-match-only rule passes the first \
         assertion and fails this one"
    );
}

#[test]
fn an_unmatchable_tail_is_appended_not_dropped() {
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\alp7 ");
    assert_eq!(
        text(&s),
        "α7 ",
        "`7` finished `alp`; it is kept, not swallowed, and the whole \
         abbreviation is not abandoned"
    );
}

#[test]
fn there_is_no_terminator_list() {
    // `'+ '` is a key — a trailing SPACE is part of it. Bites against
    // any hardcoded space/tab/RET terminator set.
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\+ ");
    assert_eq!(text(&s), "⊹", "the space EXTENDED rather than terminating");
}

#[test]
fn a_doubled_backslash_yields_one_literal_backslash() {
    // Not a terminator case: the pending text is empty, `\` is itself a
    // key, and it extends-and-eagerly-matches.
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\\\");
    assert_eq!(text(&s), "\\", "one literal backslash");

    // ...and no pending state was left open, so an ordinary letter is
    // an ordinary letter.
    type_str(&mut s, "n");
    assert_eq!(text(&s), "\\n", "two characters, not a newline");
}

#[test]
fn a_terminating_backslash_re_arms_as_a_new_leader() {
    // `al` is NOT eager, so its pending record is still open when the
    // second `\` arrives: the `\` terminates it, the expansion runs,
    // and the same `\` must then open a fresh abbreviation.
    //
    // The framing's own example — `\alpha\to` — does NOT exercise this
    // branch: `alpha` is eager, so the record is already closed and the
    // `\` is handled by the ordinary open-a-leader path. It passes with
    // the re-arm branch deleted, which is why the non-eager case is the
    // one asserted first.
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\al\\to ");
    assert_eq!(
        text(&s),
        "∀→ ",
        "the terminating `\\` expanded `al` AND opened a new \
         abbreviation at its own position"
    );

    // The criterion's example still holds, by the other route.
    exec(&s, "pmacs.editor.goto_byte(pmacs.window.buffer():len())");
    type_str(&mut s, "\\alpha\\to ");
    assert_eq!(text(&s), "∀→ α→ ");
}

#[test]
fn an_inserted_backslash_does_not_re_arm() {
    // `setminus` expands to a literal `\`. That backslash is a
    // programmatic replace, which arms no typed-edit record — so it
    // opens no pending abbreviation. Bites against a future consumer
    // that infers pending state from buffer text instead of provenance.
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\setminus");
    assert_eq!(text(&s), "\\", "expanded to a literal backslash");

    type_str(&mut s, "n");
    assert_eq!(
        text(&s),
        "\\n",
        "the letter after it is a plain letter — the INSERTED backslash \
         armed nothing"
    );
}

// ---------------------------------------------------------------------------
// 45h — the tie-break by source declaration order
// ---------------------------------------------------------------------------

#[test]
fn equal_length_candidates_break_by_source_declaration_order() {
    // `f<` and `f>` are both length 2. `f<` is declared first, so `\f`
    // resolves to `‹`. This is the criterion that bites a map-shaped
    // vendored table: with `pairs` iteration it passes or fails by hash
    // order.
    let (mut s, _f) = lean_editor();
    type_str(&mut s, "\\f ");
    assert_eq!(text(&s), "‹ ", "`f<` wins over `f>` by source order");

    exec(&s, "pmacs.editor.goto_byte(pmacs.window.buffer():len())");
    type_str(&mut s, "\\\" ");
    assert_eq!(
        text(&s),
        "‹ Ä ",
        "`\"A` is the first of eleven equal-length candidates"
    );
}

#[test]
fn reversing_the_vendored_sequence_reverses_the_tie() {
    // The falsification 45h requires: run the same resolution against a
    // deliberately reversed sequence and show it changes. If this did
    // NOT change, the tie-break would not be reading source order at
    // all and the assertion above would be passing by luck.
    let (s, _f) = lean_editor();
    let forward: String = eval(&s, "return pmacs.lean_input._resolve('f')");
    assert_eq!(forward, "‹");

    let reversed: String = eval(
        &s,
        "
        local seq = pmacs.lean_abbrev
        local rev = {}
        for i = #seq, 1, -1 do rev[#rev + 1] = seq[i] end
        -- Resolve `f` the way the module does, over the reversed order.
        local best = nil
        for i = 1, #rev do
          local k, v = rev[i][1], rev[i][2]
          if k:sub(1, 1) == 'f' then
            if best == nil or #k < best.len then best = { sym = v, len = #k } end
          end
        end
        return best.sym
        ",
    );
    assert_eq!(
        reversed, "›",
        "reversed source order picks `f>` — the tie really is decided \
         by position in the sequence"
    );
}

// ---------------------------------------------------------------------------
// 45g — table integrity, limited to what the suite can actually check
// ---------------------------------------------------------------------------

#[test]
fn the_vendored_table_is_self_consistent() {
    // `abbreviations.json` is not shipped, so the suite cannot diff
    // against it; the full source-fidelity check belongs to the
    // generator, which re-parses its own output from disk. What is
    // checkable here are the properties a corrupt emit breaks.
    let (s, _f) = lean_editor();

    let count: i64 = eval(&s, "return #pmacs.lean_abbrev");
    assert_eq!(
        count, 1855,
        "the declared entry count for the recorded upstream commit"
    );

    let (unique, cursor_ok, utf8_ok): (i64, bool, bool) = eval(
        &s,
        r#"
        local seen, n = {}, 0
        local cursor_ok, utf8_ok = true, true
        for i = 1, #pmacs.lean_abbrev do
          local k, v = pmacs.lean_abbrev[i][1], pmacs.lean_abbrev[i][2]
          if not seen[k] then seen[k] = true; n = n + 1 end
          local _, c = v:gsub("%$CURSOR", "")
          if c > 1 then cursor_ok = false end
          -- A Lua pattern cannot validate UTF-8; check the shape the
          -- emitter guarantees instead: no lone continuation byte at the
          -- start of a sequence and no truncated tail.
          if k:find("[\128-\191]") == 1 then utf8_ok = false end
        end
        return n, cursor_ok, utf8_ok
        "#,
    );
    assert_eq!(
        unique, 1855,
        "every key is unique — a collision would silently drop entries \
         from the derived lookup"
    );
    assert!(cursor_ok, "no symbol carries more than one $CURSOR");
    assert!(utf8_ok, "no key begins with a continuation byte");

    // The resolution spot-set named by 45g.
    for (input, want) in [
        ("alpha", "α"),
        ("to", "→"),
        ("<>", "⟨$CURSOR⟩"),
        ("+ ", "⊹"),
        ("\\\\", "\\"),
        ("n", "\\n"),
        ("setminus", "\\"),
        ("f", "‹"),
    ] {
        let got: String = eval(&s, &format!("return pmacs.lean_input._resolve('{input}')"));
        assert_eq!(got, want, "resolution of {input:?}");
    }

    // The eager set is the one the state machine branches on.
    let alpha_eager: bool = eval(&s, "return pmacs.lean_input._is_eager('alpha')");
    let to_eager: bool = eval(&s, "return pmacs.lean_input._is_eager('to')");
    assert!(alpha_eager, "`alpha` has no extension");
    assert!(!to_eager, "`to` is extended by `top`, `to0`, `toa`, …");
}

// ---------------------------------------------------------------------------
// Q#AP7 for the deferred expansion: it must land before lsp.lua flushes
// ---------------------------------------------------------------------------

#[test]
fn the_expansion_reaches_the_first_did_change() {
    // The expansion runs on its OWN `buffer.after-edit` subscriber,
    // after the typed-edit chain. That makes it a new instance of the
    // Q#AP7 obligation pairing already carries: lsp.lua's subscriber
    // flushes `didChange` SYNCHRONOUSLY on the signature-trigger path,
    // and `(` is a trigger. A server told about `\alp(` instead of
    // `α()` stays wrong until the next edit — diagnostics, semantic
    // tokens and inlay hints all frozen at stale byte positions.
    //
    // Falsified by loading lean_input.lua after lsp.lua in
    // `src/editor.rs`: the expansion would then arrive in the SECOND
    // didChange, or not at all.
    let dir = fresh_dir();
    let sink = dir.join("changes.jsonl");
    let sink_disp = sink.display().to_string();
    let fake = env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned();

    let f = dir.join("a.lean");
    std::fs::write(&f, "").unwrap();
    let mut s = EditorState::new_with_roots(&crate::iso::roots());
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.clone()));
    exec(&s, "pmacs.lsp.config = {}");
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.lean4 = {{
               command = '{fake}',
               env = {{
                 PMACS_FAKE_LSP_MODE = 'sighelp',
                 PMACS_FAKE_LSP_CHANGE_SINK = '{sink_disp}',
               }},
             }}"
        ),
    );

    let fd = f.display().to_string();
    exec(&s, &format!("pmacs.buffer.find_or_open({fd:?})"));
    exec(&s, "pmacs.editor.goto_byte(0)");
    let initialized = "(function() \
       for _,r in ipairs(pmacs.lsp.list()) do \
         if r.state and r.state.kind=='initialized' then return true end \
       end \
       return false \
     end)()";
    assert!(pump_lua_flag(&mut s, initialized, 5), "fake server init");

    type_str(&mut s, "\\alp(");
    assert_eq!(text(&s), "α()", "precondition: the expansion happened");

    // Wait for the flush that carries the `(` keystroke. Earlier
    // keystrokes have already produced their own didChanges, so
    // `changes[0]` is NOT the one under test — asserting on it compares
    // against `\al` and fails for the wrong reason.
    let deadline = Instant::now() + Duration::from_secs(5);
    let changes = loop {
        s.tick_processes();
        s.tick_lsp();
        s.tick_async();
        let c = did_change_texts(&sink);
        if c.iter().any(|t| t.contains('α')) {
            break c;
        }
        assert!(
            Instant::now() < deadline,
            "no didChange carrying the expansion reached the fake server;              got {:?}",
            did_change_texts(&sink)
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        !changes.iter().any(|t| t == "\\alp("),
        "no didChange may ever carry the UNEXPANDED text — one would          mean lsp.lua flushed before the deferred expansion ran (Q#AP7).          Got {changes:?}"
    );
    assert_eq!(
        changes.last().map(String::as_str),
        Some("α()"),
        "the flush that carries the terminator carries the expansion          and pairing's closer with it"
    );
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

// ---------------------------------------------------------------------------
// 45i — pending state is per frontend
// ---------------------------------------------------------------------------

/// Register a second frontend on the SAME buffer, with its own window.
fn attach_frontend(s: &EditorState, fid: FrontendId) -> WindowId {
    let mut core = s.core.borrow_mut();
    let buffer_id = core.active_buffer_id();
    let text_view = {
        let registry = core.registry.clone();
        let reg = registry.borrow();
        pmacs::text_view::TextView::new(reg.get(buffer_id).unwrap())
    };
    let win_id = WindowId::next();
    core.windows
        .insert(win_id, Window::new(win_id, buffer_id, text_view));
    core.register_frontend_view(
        fid,
        FrontendView {
            layout: Layout::single(win_id),
            active: win_id,
            fold_projection: true,
            panel_capable: true,
            frame_geometry: None,
            panel_hidden: false,
        },
    );
    win_id
}

const B: FrontendId = FrontendId(9);

#[test]
fn a_peer_edit_to_the_shared_buffer_abandons_the_pending_record() {
    let (mut s, _f) = lean_editor();
    let b_win = attach_frontend(&s, B);
    // B sits at the start of the buffer; A types at the end.
    s.core.borrow_mut().windows.get_mut(&b_win).unwrap().cursor = 0;

    type_as(&mut s, FrontendId::LOCAL, "\\al");
    type_as(&mut s, B, "p");
    assert!(
        text(&s).contains('p'),
        "B's keystroke landed as ordinary text rather than extending \
         A's abbreviation, got {:?}",
        text(&s)
    );

    type_as(&mut s, FrontendId::LOCAL, "l ");
    assert!(
        !text(&s).contains('∀'),
        "A's record was abandoned: `revision()` is buffer-global, so \
         B's edit invalidates it even though B edited elsewhere. Got {:?}",
        text(&s)
    );
}

#[test]
fn a_peer_buffer_switch_does_not_clear_another_frontends_record() {
    let (mut s, f) = lean_editor();
    let dir = fresh_dir();
    let other = dir.join("b.lean");
    std::fs::write(&other, "").unwrap();
    let od = other.display().to_string();
    let fd = f.display().to_string();
    // Warm up both buffers so B's switch takes `find_or_open`'s
    // already-open branch, which is the only one that fires
    // `buffer.after-switch`. A fresh load fires `buffer.after-load`
    // and a record-less edit instead — and that path clears pending
    // state for a different reason, which would make this test green
    // no matter whose entries the subscriber clears.
    exec(&s, &format!("pmacs.buffer.find_or_open({od:?})"));
    exec(&s, &format!("pmacs.buffer.find_or_open({fd:?})"));
    exec(&s, "pmacs.editor.goto_byte(0)");
    attach_frontend(&s, B);

    type_as(&mut s, FrontendId::LOCAL, "\\al");

    // B switches buffers WITHOUT editing the shared buffer.
    // Only B moves: the switch is scoped to B's own window, so A's
    // window still shows the shared buffer with A's point where it was.
    s.core.borrow_mut().active_frontend = B;
    exec(&s, &format!("pmacs.buffer.find_or_open({od:?})"));
    s.core.borrow_mut().active_frontend = FrontendId::LOCAL;

    type_as(&mut s, FrontendId::LOCAL, "l ");
    assert_eq!(
        text(&s),
        "∀ ",
        "`buffer.after-switch` clears only the ACTING frontend's \
         entries — a blanket clear would discard A's half-typed \
         abbreviation"
    );
}

#[test]
fn detaching_a_frontend_purges_only_its_own_pending_state() {
    let (mut s, _f) = lean_editor();
    attach_frontend(&s, B);

    type_as(&mut s, FrontendId::LOCAL, "\\al");
    exec(&s, &format!("pmacs.hook.run('frontend.detached', {})", B.0));

    type_as(&mut s, FrontendId::LOCAL, "l ");
    assert_eq!(
        text(&s),
        "∀ ",
        "B's detachment purged B's entries and left A's record valid"
    );
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
