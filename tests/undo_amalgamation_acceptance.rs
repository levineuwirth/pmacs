// tests/undo_amalgamation_acceptance.rs --- E6.4: self-insert amalgamation.

//! Consecutive typed characters undo as one step, in groups of
//! `undo.amalgamate` (20, Emacs's number), cut by any other command;
//! and the v0.1 history is capped. Every row types through
//! `dispatch_key` and undoes through the bound `C-/`, because the claim
//! is about the keystrokes a user makes and the command boundary they
//! rotate --- a direct `Buffer` call would not rotate it and would pass
//! vacuously.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::buffer::{Buffer, BufferId, EditOp, UNDO_HISTORY_LIMIT};
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

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn fresh() -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    s
}

fn text(s: &EditorState) -> String {
    eval(
        s,
        "local b = pmacs.window.buffer() return b:slice(0, b:len())",
    )
}

/// The bound undo chord.
fn undo(s: &mut EditorState) {
    ctrl(s, '/');
}

/// E6.4's row: a typed word is one undo step.
#[test]
fn typing_a_word_undoes_as_one_step() {
    let mut s = fresh();
    type_str(&mut s, "hello");
    assert_eq!(text(&s), "hello");
    undo(&mut s);
    assert_eq!(text(&s), "", "five keystrokes, one step");
}

/// Groups are cut every twenty characters: twenty-five typed undo as
/// five, then twenty.
#[test]
fn groups_are_cut_at_twenty_characters() {
    let mut s = fresh();
    let twenty_five = "abcdefghijklmnopqrstuvwxy";
    type_str(&mut s, twenty_five);
    assert_eq!(text(&s), twenty_five);
    undo(&mut s);
    assert_eq!(text(&s), &twenty_five[..20], "the tail group of five");
    undo(&mut s);
    assert_eq!(text(&s), "", "then the group of twenty");
}

/// Any other command ends the run: a motion between two typed
/// characters makes the next character its own step.
#[test]
fn a_motion_between_keystrokes_breaks_the_run() {
    let mut s = fresh();
    type_str(&mut s, "ab");
    ctrl(&mut s, 'b');
    type_str(&mut s, "c");
    assert_eq!(text(&s), "acb");
    undo(&mut s);
    assert_eq!(
        text(&s),
        "ab",
        "the character after the motion undoes alone"
    );
    undo(&mut s);
    assert_eq!(text(&s), "", "then the two before it, as one");
}

/// Probe: a command that moves nothing --- End at the end of the line
/// --- still ends the run, so the characters after it are a new group
/// even though they are contiguous with the ones before.
#[test]
fn a_no_op_command_between_keystrokes_breaks_the_run() {
    let mut s = fresh();
    type_str(&mut s, "ab");
    ctrl(&mut s, 'e');
    type_str(&mut s, "cd");
    assert_eq!(text(&s), "abcd");
    undo(&mut s);
    assert_eq!(text(&s), "ab", "the two typed after End");
    undo(&mut s);
    assert_eq!(text(&s), "");
}

/// Probe: Backspace is not a self-insert, so it is its own step and the
/// characters after it form a new group.
#[test]
fn a_delete_is_its_own_step_and_starts_a_new_group() {
    let mut s = fresh();
    type_str(&mut s, "abc");
    press(&mut s, KeyCode::Backspace);
    type_str(&mut s, "de");
    assert_eq!(text(&s), "abde");
    undo(&mut s);
    assert_eq!(text(&s), "ab", "the two typed after the delete");
    undo(&mut s);
    assert_eq!(text(&s), "abc", "the delete");
    undo(&mut s);
    assert_eq!(text(&s), "", "the first three");
}

/// Probe: redo restores a whole group.
#[test]
fn redo_restores_the_whole_group() {
    let mut s = fresh();
    type_str(&mut s, "hello");
    undo(&mut s);
    assert_eq!(text(&s), "");
    exec(&s, "pmacs.command.invoke('buffer.redo')");
    assert_eq!(text(&s), "hello");
}

/// The knob: at 0 every keystroke is its own step; at 3 the groups are
/// three; and it is live.
#[test]
fn the_knob_bounds_the_group_and_zero_disables_it() {
    let mut s = fresh();
    assert_eq!(
        eval::<i64>(&s, "return pmacs.config.get('undo.amalgamate')"),
        20,
        "the registered default is Emacs's twenty"
    );
    exec(&s, "pmacs.config.set('undo.amalgamate', 0)");
    type_str(&mut s, "abc");
    undo(&mut s);
    assert_eq!(text(&s), "ab", "0: one character per step");

    exec(&s, "pmacs.config.set('undo.amalgamate', 3)");
    ctrl(&mut s, 'e');
    type_str(&mut s, "defgh");
    assert_eq!(text(&s), "abdefgh");
    undo(&mut s);
    assert_eq!(text(&s), "abdef", "3: the tail group of two");
    undo(&mut s);
    assert_eq!(text(&s), "ab", "then the group of three");
}

/// Probe: an insert a script makes between two keystrokes --- with no
/// command boundary of its own, so the run is still self-insert's ---
/// is not typed and is never swallowed into the group, even though it
/// is contiguous with the keystroke after it.
#[test]
fn a_programmatic_insert_between_keystrokes_is_not_amalgamated() {
    let mut s = fresh();
    type_str(&mut s, "ab");
    exec(&s, "local b = pmacs.window.buffer() b:insert(2, 'X')");
    exec(&s, "pmacs.editor.goto_byte(3)");
    type_str(&mut s, "cd");
    assert_eq!(text(&s), "abXcd");
    undo(&mut s);
    assert_eq!(text(&s), "abX", "the two typed after the script's insert");
    undo(&mut s);
    assert_eq!(text(&s), "ab", "the script's insert alone");
    undo(&mut s);
    assert_eq!(text(&s), "");
}

/// Probe: typing over a selection is a replace, and the characters
/// after it join it into one group --- the first keystroke of a run
/// may be a replace, the rest are inserts at its end.
#[test]
fn typing_over_a_region_starts_a_group_the_rest_join() {
    let mut s = fresh();
    type_str(&mut s, "abc");
    ctrl(&mut s, 'a');
    // Select `ab` with the mark and two motions.
    ctrl(&mut s, ' ');
    ctrl(&mut s, 'f');
    ctrl(&mut s, 'f');
    type_str(&mut s, "XY");
    assert_eq!(text(&s), "XYc");
    undo(&mut s);
    assert_eq!(
        text(&s),
        "abc",
        "the replace and the character after it, as one"
    );
}

/// CRDT mode (E6.4): once a buffer is CRDT-backed the history is loro's,
/// and the same run undoes as one step through an undo group.
#[cfg(feature = "crdt")]
#[test]
fn typing_a_word_undoes_as_one_step_in_crdt_mode() {
    let mut s = fresh();
    {
        let core = s.core.borrow();
        let id = core.active_buffer_id();
        let mut reg = core.registry.borrow_mut();
        reg.get_mut(id)
            .expect("scratch")
            .upgrade_to_crdt(1)
            .expect("upgrade");
    }
    s.core.borrow_mut().pending_crdt_ops.clear();
    type_str(&mut s, "hello");
    ctrl(&mut s, 'b');
    type_str(&mut s, "XY");
    assert_eq!(text(&s), "hellXYo");
    undo(&mut s);
    assert_eq!(text(&s), "hello", "the run after the motion, as one");
    undo(&mut s);
    assert_eq!(text(&s), "", "the word, as one");
}

/// The cap: the v0.1 history keeps `UNDO_HISTORY_LIMIT` steps and
/// forgets the oldest past it.
#[test]
fn the_history_is_capped_and_forgets_the_oldest() {
    let mut buf = Buffer::new(BufferId::next(), "cap");
    let extra = 5;
    for _ in 0..(UNDO_HISTORY_LIMIT + extra) {
        let pos = buf.len();
        buf.apply_edit(EditOp::Insert { pos, bytes: b"x" })
            .expect("insert");
    }
    assert_eq!(buf.undo_depth(), Some(UNDO_HISTORY_LIMIT), "capped");
    let mut undone = 0;
    while buf.undo().is_ok() {
        undone += 1;
    }
    assert_eq!(undone, UNDO_HISTORY_LIMIT);
    assert_eq!(
        buf.len(),
        extra as u64,
        "the oldest inserts are beyond the history and stay"
    );
}

#[path = "common/iso.rs"]
mod iso;
