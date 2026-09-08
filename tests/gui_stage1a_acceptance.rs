//! GUI arc Stage 1a acceptance — `TextInput` at protocol v24.
//!
//! Framing: the archived gui-stage1-input framing §5 (Q#S1-9 precedence)
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
}

/// §5 — a MULTI-scalar commit **breaks the command chain**, as a paste
/// does.
///
/// **The chain is PRIMED first, and that is what makes the row
/// discriminating.** Starting from a fresh editor the chain is already
/// empty, so an assertion that it is empty afterwards passes whether or
/// not `break_command_chain` is called — the first version of this row
/// did exactly that and would have survived deleting the call.
#[test]
fn multi_scalar_text_input_breaks_a_live_command_chain() {
    let mut s = editor_with("");

    // Prime it with 1a's OWN single-scalar path, which rotates to
    // `buffer.self-insert`. A programmatic
    // `pmacs.command.invoke('buffer.self-insert')` cannot prime it:
    // rotation belongs to the dispatcher, and invoking the command
    // directly deliberately never rotates or arms.
    s.dispatch_text_input(FID, "x");
    let primed: String = eval(&s, "return pmacs.editor.this_command() or ''");
    assert_eq!(
        primed, "buffer.self-insert",
        "precondition: the chain is live before the commit"
    );

    s.dispatch_text_input(FID, "e\u{301}");

    let after: Option<String> = eval(&s, "return pmacs.editor.this_command()");
    assert_eq!(
        after, None,
        "a multi-scalar commit is not a command and must clear the chain"
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

// ---------------------------------------------------------------------
// A8 — a terminal receives RAW UTF-8, never bracketed paste
// ---------------------------------------------------------------------

/// A8, delivered rather than merely routed: the child process receives
/// the exact UTF-8 bytes, **while bracketed-paste mode is ENABLED**, and
/// no `ESC[200~` / `ESC[201~` markers.
///
/// **The enabled mode is the whole precondition.** With bracketed paste
/// off, "no markers" is true of every code path including a paste, so
/// the assertion would pass against the behaviour it exists to forbid.
/// The row therefore waits for the child's own `ESC[?2004h` to be
/// parsed, asserts the mode really is on, and only then types.
///
/// The contrast at the end is what makes it a discriminator: through the
/// same terminal in the same mode, a PASTE does get the markers. One
/// path bracketed and the other not, observed at the PTY.
#[test]
fn a8_a_terminal_receives_raw_utf8_with_bracketed_paste_enabled() {
    use pmacs::terminal::TerminalSpec;
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().expect("tempdir");
    let sink = dir.path().join("received");
    let sink_disp = sink.display().to_string();

    let mut s = EditorState::new_with_roots(&crate::iso::roots());

    // The child turns bracketed paste ON, then copies its stdin to a
    // file so the test can read exactly what arrived on the PTY.
    let script = format!("printf '\\033[?2004h'; exec cat > {sink_disp}");
    let mut spec = TerminalSpec::new("/bin/sh");
    spec.args = vec!["-c".into(), script];
    spec.rows = 24;
    spec.cols = 80;
    let buffer_id = s
        .terminal_manager
        .borrow_mut()
        .open(
            spec,
            &mut s.core.borrow_mut(),
            &mut s.process_supervisor.borrow_mut(),
        )
        .expect("open terminal");

    // Point this frontend's view at the terminal buffer, the way a
    // daemon-side buffer switch does. Without it `active_terminal_key`
    // returns `None`, the terminal branch is never taken, and the row
    // fails for a setup reason rather than a behavioural one — which is
    // exactly how it first failed.
    let window_id = attach_terminal_view(&s, FID, buffer_id);
    let key = pmacs::terminal::TerminalViewKey::new(FID, window_id, buffer_id);
    // Precondition, asserted rather than assumed: this frontend's
    // ACTIVE window shows the terminal buffer. A row that silently
    // failed this would be testing the document path and reporting it
    // as a terminal result.
    {
        let core = s.core.borrow();
        let view = core.views.get(&FID).expect("view registered");
        let active = core.windows.get(&view.active).expect("active window");
        assert_eq!(active.buffer_id, buffer_id);
        assert!(
            s.terminal_manager.borrow().is_terminal(buffer_id),
            "and the manager agrees it is a terminal"
        );
    }

    // Wait for the child's mode-set to be parsed — a condition, not a
    // sleep, so a slow machine waits longer rather than failing.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        s.tick_processes();
        let on = s
            .terminal_manager
            .borrow()
            .modes_for_view(key)
            .is_some_and(|m| m.bracketed_paste);
        if on {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "child never enabled bracketed paste; the precondition this \
             row depends on was never established"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    // Multi-byte and multi-scalar, so a byte-level mistake shows up.
    let typed = "h\u{e9}llo\u{301}";
    s.dispatch_text_input(FID, typed);

    // Waits for AT LEAST the full payload, then asserts exact equality.
    // Sound against a split write for a reason the contrast below does
    // not share: the gate is a lower bound on length, so a partial
    // delivery keeps waiting rather than being mistaken for a wrong
    // answer — and the equality can still fail for the real reason,
    // which a wait-for-exact-content loop could not.
    let deadline = Instant::now() + Duration::from_secs(10);
    let got = loop {
        s.tick_processes();
        let got = std::fs::read(&sink).unwrap_or_default();
        if got.len() >= typed.len() {
            break got;
        }
        assert!(
            Instant::now() < deadline,
            "the child never received the typed text; got {got:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    };

    assert_eq!(
        String::from_utf8_lossy(&got),
        typed,
        "the PTY must receive the exact UTF-8 that was typed"
    );
    let text = String::from_utf8_lossy(&got).into_owned();
    assert!(
        !text.contains("\u{1b}[200~") && !text.contains("\u{1b}[201~"),
        "typed text must NOT be bracketed: {text:?}"
    );

    // The contrast, through the same terminal in the same mode: a paste
    // IS bracketed. Without this the row above could pass because the
    // mode was somehow inert rather than because the code is right.
    assert!(
        s.dispatch_paste(FID, b"pasted"),
        "the terminal claims the paste"
    );
    //
    // **Wait for the COMPLETE sequence, not the opening marker.** PTY
    // delivery and the child's writes can split anywhere, so breaking
    // as soon as `ESC[200~` appears and then requiring the payload and
    // the closer is a race that fails on correct code — the closer may
    // simply not have arrived yet. Polling for the whole string makes a
    // partial write indistinguishable from "not yet", which is what it
    // is. Same rule the vterm suite follows when it waits for `row19`
    // rather than for a prefix of it.
    let want = "\u{1b}[200~pasted\u{1b}[201~";
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        s.tick_processes();
        let all = String::from_utf8_lossy(&std::fs::read(&sink).unwrap_or_default()).into_owned();
        if all.contains(want) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "a paste through the same terminal must be bracketed on both \
             sides; waited for {want:?}, saw {all:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Register a frontend view whose active window shows `buffer_id`.
fn attach_terminal_view(
    state: &EditorState,
    frontend_id: FrontendId,
    buffer_id: pmacs::buffer::BufferId,
) -> pmacs::window::WindowId {
    use pmacs::window::{FrontendView, Layout, Window, WindowId};
    let mut core = state.core.borrow_mut();
    let text_view = {
        let registry = core.registry.clone();
        let registry = registry.borrow();
        let buffer = registry.get(buffer_id).expect("buffer present");
        pmacs::text_view::TextView::new(buffer)
    };
    let window_id = WindowId::next();
    core.windows
        .insert(window_id, Window::new(window_id, buffer_id, text_view));
    core.register_frontend_view(
        frontend_id,
        FrontendView {
            layout: Layout::single(window_id),
            active: window_id,
            fold_projection: true,
            panel_capable: true,
            frame_geometry: None,
            panel_hidden: false,
        },
    );
    window_id
}

#[path = "common/iso.rs"]
mod iso;
