// tests/e6_review1_undo_probes.rs --- E6 review 1: E6.4 probed as an
// experience, in both histories.

//! Three user stories, each run once in a plain (v0.1 stack) buffer and
//! once in a buffer upgraded to CRDT (loro's `UndoManager`), and the
//! text after one `C-/` compared across the two: the two histories must
//! agree on what one undo removes, and the record says what that is.
//!
//! Every keystroke goes through `dispatch_key` and the undo through the
//! bound chord, as `undo_amalgamation_acceptance` does; the CRDT rows
//! upgrade the scratch buffer in place as that suite's own CRDT rows do.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

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

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn text(s: &EditorState) -> String {
    eval(
        s,
        "local b = pmacs.window.buffer() return b:slice(0, b:len())",
    )
}

fn undo(s: &mut EditorState) {
    ctrl(s, '/');
}

/// A fresh editor; with `crdt` the scratch buffer is upgraded in place
/// so its history is loro's from the first keystroke.
fn fresh(crdt: bool) -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    if crdt {
        #[cfg(feature = "crdt")]
        {
            let core = s.core.borrow();
            let id = core.active_buffer_id();
            let mut reg = core.registry.borrow_mut();
            reg.get_mut(id)
                .expect("scratch")
                .upgrade_to_crdt(1)
                .expect("upgrade");
        }
        #[cfg(not(feature = "crdt"))]
        panic!("a CRDT row needs the crdt feature");
    }
    s.core.borrow_mut().pending_crdt_ops.clear();
    s
}

/// Run `story` in a plain buffer and in a CRDT one; return the texts
/// after the story and after each of `undos` undos, per mode.
fn both(story: impl Fn(&mut EditorState), undos: usize) -> (Vec<String>, Vec<String>) {
    let run = |crdt: bool| {
        let mut s = fresh(crdt);
        story(&mut s);
        let mut out = vec![text(&s)];
        for _ in 0..undos {
            undo(&mut s);
            out.push(text(&s));
        }
        out
    };
    let plain = run(false);
    let crdt = if cfg!(feature = "crdt") {
        run(true)
    } else {
        plain.clone()
    };
    eprintln!("plain: {plain:?}\ncrdt:  {crdt:?}");
    (plain, crdt)
}

/// `abc(` with auto-pair on (the default): the closer lands inside the
/// keystroke. One undo removes the closer alone, in both histories; the
/// second removes `abc(` as one step, in both.
#[test]
fn review_abc_paren_with_auto_pair_one_undo_agrees_across_histories() {
    let (plain, crdt) = both(
        |s| {
            assert!(
                eval::<bool>(s, "return pmacs.config.get('editing.auto-pair')"),
                "auto-pair is on by default"
            );
            type_str(s, "abc(");
        },
        2,
    );
    assert_eq!(plain[0], "abc()", "auto-pair closed the paren");
    assert_eq!(plain, crdt, "the two histories must agree step for step");
    assert_eq!(plain[1], "abc(", "one undo: the closer alone");
    assert_eq!(plain[2], "", "two undos: `abc(` as one step");
}

/// Twenty-five characters: one undo removes the tail five in both
/// histories, the second the twenty.
#[test]
fn review_twenty_five_characters_one_undo_agrees_across_histories() {
    let twenty_five = "abcdefghijklmnopqrstuvwxy";
    let (plain, crdt) = both(|s| type_str(s, twenty_five), 2);
    assert_eq!(plain[0], twenty_five);
    assert_eq!(plain, crdt, "the two histories must agree step for step");
    assert_eq!(plain[1], &twenty_five[..20], "one undo: the tail five");
    assert_eq!(plain[2], "", "two undos: the twenty");
}

/// Ten, a caret motion (`<left>`), ten more: one undo removes the ten
/// typed after the motion in both histories, leaving the first ten.
#[test]
fn review_ten_then_left_then_ten_one_undo_agrees_across_histories() {
    let (plain, crdt) = both(
        |s| {
            type_str(s, "0123456789");
            press(s, KeyCode::Left);
            type_str(s, "abcdefghij");
        },
        2,
    );
    assert_eq!(plain[0], "012345678abcdefghij9");
    assert_eq!(plain, crdt, "the two histories must agree step for step");
    assert_eq!(plain[1], "0123456789", "one undo: the ten after the motion");
    assert_eq!(plain[2], "", "two undos: the first ten");
}

/// Probe beyond the charge: typing through a closer. `(` then `)` skips
/// over the auto-inserted `)`, so the text is `()`; one undo restores
/// the transient `())` in both histories --- a pre-E6 grain (each hook
/// edit was its own step already), recorded so the two agree.
#[test]
fn review_typing_through_a_closer_one_undo_agrees_across_histories() {
    let (plain, crdt) = both(|s| type_str(s, "()"), 1);
    assert_eq!(plain[0], "()", "the typed `)` skipped over the closer");
    assert_eq!(plain, crdt, "the two histories must agree step for step");
}

/// Probe beyond the charge: the knob is read per keystroke, so lowering
/// it mid-run cuts the next group at the new size in both histories.
#[test]
fn review_lowering_the_knob_mid_run_agrees_across_histories() {
    let (plain, crdt) = both(
        |s| {
            type_str(s, "abcde");
            s.lua_host
                .lua()
                .load("pmacs.config.set('undo.amalgamate', 2)")
                .exec()
                .unwrap();
            type_str(s, "fgh");
        },
        3,
    );
    assert_eq!(plain[0], "abcdefgh");
    assert_eq!(plain, crdt, "the two histories must agree step for step");
}

#[path = "common/iso.rs"]
mod iso;
