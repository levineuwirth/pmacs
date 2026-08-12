//! GUI arc Stage 1a acceptance — `TextInput` at protocol v24.
//!
//! Framing: `docs/gui-stage1-input-framing.md` §5 (Q#S1-9 precedence)
//! and §6's A1–A9.
//!
//! **These rows drive the real dispatch, not the classifier.** 1a's
//! first review found A7 and A8 unreachable from the production
//! producer while `text_input_payload` was perfectly correct: the
//! intercept branch returned before classification, and a modal prompt
//! or a focused terminal is exactly what makes intercept true. A test
//! that exercises the pure function would have stayed green through
//! that, so the rows here go through `dispatch_text_input` and, for the
//! producer-side ones, through the real classifier at the real call
//! site.

use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn editor_with(body: &str) -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    if !body.is_empty() {
        exec(&s, &format!("pmacs.window.buffer():insert(0, {body:?})"));
    }
    exec(&s, "pmacs.editor.goto_byte(0)");
    s
}

fn buffer_text(s: &EditorState) -> String {
    let b: mlua::String = eval(
        s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
}

const FID: FrontendId = FrontendId::LOCAL;

// ---------------------------------------------------------------------
// A6 — one commit is one edit, one undo unit, one hook fan-out
// ---------------------------------------------------------------------

/// A6 — a multi-scalar commit is **one** edit and **one** undo unit.
///
/// This is the failure 1a exists to fix: as separate keypresses the same
/// grapheme is two edits, so one undo leaves half of it behind.
#[test]
fn a6_a_multi_scalar_commit_is_one_edit_and_one_undo_unit() {
    let mut s = editor_with("");
    exec(
        &s,
        "_G.edits = 0
         pmacs.hook.add('buffer.after-edit', function() _G.edits = _G.edits + 1 end)",
    );

    // A composed grapheme: base plus combining acute. Two scalars, one
    // thing the user meant to type.
    s.dispatch_text_input(FID, "e\u{301}");
    assert_eq!(buffer_text(&s), "e\u{301}");

    let edits: i64 = eval(&s, "return _G.edits");
    assert_eq!(edits, 1, "one commit must fire ONE buffer.after-edit");

    exec(&s, "pmacs.command.invoke('buffer.undo')");
    assert_eq!(
        buffer_text(&s),
        "",
        "one undo must remove the whole commit, not its last scalar"
    );
}

// ---------------------------------------------------------------------
// §5 provenance — the single/multi split
// ---------------------------------------------------------------------

/// §5 — a SINGLE-scalar commit is indistinguishable from a keypress, so
/// it must produce a real, consumable `TypedEditRecord`.
///
/// **Asserting `this_command` is not enough**, and that is the whole
/// point of this row: review round 2 found the code rotating the command
/// correctly while never completing the record, because arming and
/// completing are different steps and only the insert primitives
/// complete. `this_command` looked right and auto-pairing was broken.
/// So this consumes the record through the same seam `pair.lua` uses.
#[test]
fn single_scalar_text_input_produces_a_consumable_typed_edit_record() {
    let mut s = editor_with("");
    exec(&s, "pmacs.pair._capture_records = true");

    s.dispatch_text_input(FID, "(");

    let (cp, ch, clean, il): (i64, String, bool, i64) = eval(
        &s,
        "local r = pmacs.pair._last_record
         return r.codepoint, r.char, r.clean, r.inserted_len",
    );
    assert_eq!(cp, 40, "exact codepoint for '('");
    assert_eq!(ch, "(");
    assert!(clean, "no intercept ran, so the effective triple is clean");
    assert_eq!(il, 1);

    let this_command: String = eval(&s, "return pmacs.editor.this_command() or ''");
    assert_eq!(
        this_command, "buffer.self-insert",
        "and the command rotates, which is the half that already worked"
    );
}

/// §5 — a MULTI-scalar commit is **not** a keystroke: it creates no
/// typed provenance and breaks the command chain.
#[test]
fn multi_scalar_text_input_creates_no_typed_provenance() {
    let mut s = editor_with("");
    exec(&s, "pmacs.pair._capture_records = true");

    s.dispatch_text_input(FID, "e\u{301}");

    let no_record: bool = eval(&s, "return pmacs.pair._last_record == nil");
    assert!(
        no_record,
        "a multi-scalar commit must not forge a typed-edit record"
    );
    let this_command: String = eval(&s, "return pmacs.editor.this_command() or ''");
    assert_ne!(
        this_command, "buffer.self-insert",
        "a multi-scalar commit is not a self-insert"
    );
}

/// The pairing consumer, end to end: a single-scalar `(` must auto-pair
/// exactly as a typed `(` does. This is the behaviour the missing record
/// silently disabled, stated in the terms a user would notice.
#[test]
fn single_scalar_text_input_auto_pairs_like_a_keypress() {
    let mut s = editor_with("");
    s.dispatch_text_input(FID, "(");
    assert_eq!(
        buffer_text(&s),
        "()",
        "auto-pairing consumes the typed-edit record; without one the \
         closer is never inserted"
    );
}

// ---------------------------------------------------------------------
// A7 — prompts consume scalars IN ORDER
// ---------------------------------------------------------------------

/// A7 — a prompt accumulates the scalars in order.
///
/// Order is the contract: a reversed or set-wise delivery would still
/// "consume" the text and would produce a different query.
#[test]
fn a7_a_prompt_consumes_scalars_in_order() {
    let mut s = editor_with("");
    exec(
        &s,
        "pmacs.minibuffer.read({ prompt = 'x: ', on_accept = function() end })",
    );
    assert!(s.core.borrow().minibuffer.is_active(), "prompt is up");

    s.dispatch_text_input(FID, "abc");

    let content: String = eval(&s, "return pmacs.minibuffer.contents() or ''");
    assert_eq!(content, "abc", "in order, not reversed or reordered");
    assert_eq!(
        buffer_text(&s),
        "",
        "and the buffer underneath is untouched"
    );
}

// ---------------------------------------------------------------------
// A9 — the cap rejects rather than truncates
// ---------------------------------------------------------------------

/// A9 — a payload at the cap is accepted whole. The complement of the
/// rejection row: a cap that refused its own boundary value would be
/// off by one in the direction nobody notices until a long IME commit
/// vanishes.
///
/// **The rejection half is witnessed where it is enforced** — at the
/// daemon boundary (`daemon.rs`, gated before any insert) and at the
/// producer (`AttachClient::send_text_input`, whose unit test lives
/// beside it in `pmacs-gpu`). Neither is reachable from an
/// `EditorState`, so asserting the constant here instead would be a row
/// that cannot fail for the right reason.
#[test]
fn a9_a_payload_at_the_cap_is_inserted_whole() {
    let mut s = editor_with("");
    let at_cap = "a".repeat(pmacs_protocol::TEXT_INPUT_MAX_BYTES);
    s.dispatch_text_input(FID, &at_cap);
    assert_eq!(
        buffer_text(&s).len(),
        pmacs_protocol::TEXT_INPUT_MAX_BYTES,
        "the boundary value is legal and must land intact"
    );
}

#[path = "common/iso.rs"]
mod iso;
